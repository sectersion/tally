//! `tally login --browser`: the OAuth device-code flow against the Tally server (`/v1/device/code`, `/v1/device/token`).
//! The token is returned to the caller, which stores it exactly as the stdin login does; it is never printed.
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum PollStep {
    Wait(u64),
    Done(String),
    Fail(String),
}

/// What to do after one poll of `/v1/device/token`. Pure.
pub fn next_step(status: u16, body: &str, interval: u64) -> PollStep {
    #[derive(Deserialize)]
    struct B {
        access_token: Option<String>,
        error: Option<String>,
    }
    let b: B = serde_json::from_str(body).unwrap_or(B { access_token: None, error: None });
    match (status, b.access_token, b.error.as_deref()) {
        (200, Some(t), _) if !t.is_empty() => PollStep::Done(t),
        (_, _, Some("authorization_pending")) => PollStep::Wait(interval),
        (_, _, Some("slow_down")) => PollStep::Wait(interval + 5),
        (_, _, Some("access_denied")) => PollStep::Fail("denied in the browser".into()),
        (_, _, Some("expired_token")) => PollStep::Fail("the code expired; run the command again".into()),
        (s, _, e) => PollStep::Fail(format!("unexpected answer from the server (status {s}{})", e.map(|e| format!(", {e}")).unwrap_or_default())),
    }
}

#[derive(Deserialize)]
struct Code {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

fn post(agent: &ureq::Agent, url: &str, body: serde_json::Value) -> Result<(u16, String)> {
    match agent.post(url).set("content-type", "application/json").send_string(&body.to_string()) {
        Ok(r) => Ok((r.status(), r.into_string().unwrap_or_default())),
        Err(ureq::Error::Status(c, r)) => Ok((c, r.into_string().unwrap_or_default())),
        Err(e) => bail!("cannot reach {url}: {e}"),
    }
}

/// Runs the flow and returns the device token. `say` gets the lines meant for the person (never the token);
/// `open` tries to open the browser (failure ignored by the caller); `sleep` is injectable for tests.
pub fn fetch_token(server: &str, device_name: &str, say: &mut dyn FnMut(&str), open: &dyn Fn(&str), sleep: &mut dyn FnMut(u64)) -> Result<String> {
    let base = server.trim_end_matches('/');
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let name: String = device_name.chars().take(64).collect();
    let (status, body) = post(&agent, &format!("{base}/v1/device/code"), serde_json::json!({ "device_name": name }))?;
    if status != 200 {
        bail!("the server refused to start a login (status {status})");
    }
    let c: Code = serde_json::from_str(&body).context("unexpected answer from /v1/device/code")?;
    say(&format!("open {}", c.verification_uri_complete));
    say(&format!("code: {}", c.user_code));
    open(&c.verification_uri_complete);
    let mut interval = c.interval.max(1);
    let mut waited = 0;
    while waited < c.expires_in {
        let (status, body) = post(&agent, &format!("{base}/v1/device/token"), serde_json::json!({ "device_code": c.device_code }))?;
        match next_step(status, &body, interval) {
            PollStep::Done(t) => return Ok(t),
            PollStep::Fail(why) => return Err(anyhow!(why)),
            PollStep::Wait(n) => {
                interval = n;
                sleep(n);
                waited += n;
            }
        }
    }
    bail!("the code expired; run the command again")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_step_covers_the_five_answers() {
        assert_eq!(next_step(200, r#"{"access_token":"tok","token_type":"Bearer"}"#, 5), PollStep::Done("tok".into()));
        assert_eq!(next_step(400, r#"{"error":"authorization_pending"}"#, 5), PollStep::Wait(5));
        assert_eq!(next_step(400, r#"{"error":"slow_down"}"#, 5), PollStep::Wait(10));
        assert!(matches!(next_step(400, r#"{"error":"access_denied"}"#, 5), PollStep::Fail(m) if m.contains("denied")));
        assert!(matches!(next_step(400, r#"{"error":"expired_token"}"#, 5), PollStep::Fail(m) if m.contains("expired")));
        assert!(matches!(next_step(500, "oops", 5), PollStep::Fail(_)));
        // a 200 without a token is not a success
        assert!(matches!(next_step(200, "{}", 5), PollStep::Fail(_)));
    }

    fn serve(answers: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<()>) {
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", srv.server_addr().to_ip().unwrap());
        let h = std::thread::spawn(move || {
            let mut polls = answers.into_iter();
            for mut req in srv.incoming_requests().take(1 + 3) {
                let mut body = String::new();
                std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
                let (code, text) = if req.url() == "/v1/device/code" {
                    assert!(body.contains("device_name"), "{body}");
                    (200, r#"{"device_code":"dc","user_code":"BCDF-GHJK","verification_uri":"http://x/device","verification_uri_complete":"http://x/device?code=BCDF-GHJK","expires_in":600,"interval":5}"#)
                } else {
                    assert!(body.contains("dc"), "{body}");
                    polls.next().unwrap()
                };
                req.respond(tiny_http::Response::from_string(text).with_status_code(code)).unwrap();
            }
        });
        (url, h)
    }

    #[test]
    fn polls_with_the_servers_interval_then_slows_down_and_never_prints_the_token() {
        let (url, h) = serve(vec![
            (400, r#"{"error":"authorization_pending"}"#),
            (400, r#"{"error":"slow_down"}"#),
            (200, r#"{"access_token":"SECRET-TOKEN","token_type":"Bearer"}"#),
        ]);
        let (mut said, mut waits) = (vec![], vec![]);
        let tok = fetch_token(&url, "my laptop", &mut |l| said.push(l.to_string()), &|_| {}, &mut |n| waits.push(n)).unwrap();
        h.join().unwrap();
        assert_eq!(tok, "SECRET-TOKEN");
        assert_eq!(waits, [5, 10]);
        assert!(said.iter().any(|l| l.contains("BCDF-GHJK")) && said.iter().any(|l| l.starts_with("open http://x/device?code=")));
        assert!(!said.join("\n").contains("SECRET-TOKEN"));
    }
}
