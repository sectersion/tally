//! In-process fake of the v1 ledger API (contract C7), modelled on server/src (plan 06).
#![allow(dead_code)]
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tally_scoring::{verify_chain, LedgerEntry, GENESIS_HASH, RULESET_VERSION};

pub const R: &str = RULESET_VERSION;

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub struct Project {
    pub name: String,
    pub token: String,
    pub revoked: bool,
}

pub struct State {
    pub projects: HashMap<String, Project>,
    pub chains: HashMap<(String, String), Vec<LedgerEntry>>,
    pub commits: HashMap<String, Vec<(String, i64)>>,
    pub requests: Vec<Recorded>,
    pub fail: (usize, u16),
    pub down: bool,
    pub drop_upload: bool,
    pub max_batch: usize,
    pub ruleset: String,
    /// Runs once, just before the next `POST entries` is processed (a concurrent writer).
    pub before_upload: Option<Hook>,
}

pub type Hook = Box<dyn FnOnce(&mut State) + Send>;

pub struct FakeServer {
    pub url: String,
    st: Arc<Mutex<State>>,
}

impl FakeServer {
    pub fn start(token: &str) -> FakeServer {
        let st = Arc::new(Mutex::new(State {
            projects: HashMap::new(),
            chains: HashMap::new(),
            commits: HashMap::new(),
            requests: vec![],
            fail: (0, 0),
            down: false,
            drop_upload: false,
            max_batch: 1000,
            ruleset: R.into(),
            before_upload: None,
        }));
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let s2 = st.clone();
        // detached: ends with the test process
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                serve(c, &s2);
            }
        });
        let f = FakeServer { url, st };
        f.add_project("srv_1", "owner/repo", token);
        f
    }
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.st.lock().unwrap())
    }
    pub fn add_project(&self, id: &str, public_name: &str, token: &str) {
        let p = Project { name: public_name.into(), token: token.into(), revoked: false };
        self.with(|s| s.projects.insert(id.into(), p));
    }
    pub fn requests(&self) -> Vec<Recorded> {
        self.with(|s| s.requests.clone())
    }
    pub fn chain(&self, project: &str, ruleset: &str) -> Vec<LedgerEntry> {
        self.with(|s| s.chains.get(&(project.into(), ruleset.into())).cloned().unwrap_or_default())
    }
    pub fn set_chain(&self, project: &str, ruleset: &str, v: Vec<LedgerEntry>) {
        self.with(|s| s.chains.insert((project.into(), ruleset.into()), v));
    }
    pub fn commits(&self, project: &str) -> Vec<(String, i64)> {
        self.with(|s| s.commits.get(project).cloned().unwrap_or_default())
    }
    pub fn fail_next(&self, n: usize, status: u16) {
        self.with(|s| s.fail = (n, status));
    }
    pub fn down(&self, yes: bool) {
        self.with(|s| s.down = yes);
    }
    pub fn store_then_drop_next_upload(&self) {
        self.with(|s| s.drop_upload = true);
    }
    pub fn revoke(&self, server_id: &str) {
        self.with(|s| s.projects.get_mut(server_id).unwrap().revoked = true);
    }
    pub fn max_batch(&self, n: usize) {
        self.with(|s| s.max_batch = n);
    }
    pub fn set_current_ruleset(&self, r: &str) {
        self.with(|s| s.ruleset = r.into());
    }
    pub fn on_next_upload(&self, f: impl FnOnce(&mut State) + Send + 'static) {
        self.with(|s| s.before_upload = Some(Box::new(f)));
    }
}

pub fn raw_request(url: &str, text: &str) -> String {
    let mut c = TcpStream::connect(url.trim_start_matches("http://")).unwrap();
    c.write_all(text.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out); // a reset after the full response is fine
    String::from_utf8_lossy(&out).into_owned()
}

fn serve(mut c: TcpStream, st: &Mutex<State>) {
    if st.lock().unwrap().down {
        return;
    }
    let mut r = BufReader::new(c.try_clone().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let mut it = line.split_whitespace();
    let (method, path) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string());
    let mut headers = vec![];
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap() == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_string()));
        }
    }
    let len = headers.iter().find(|h| h.0 == "content-length").map_or(0, |h| h.1.parse().unwrap());
    let mut body = vec![0; len];
    r.read_exact(&mut body).unwrap();
    let rec = Recorded { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() };
    let reply = {
        let mut s = st.lock().unwrap();
        s.requests.push(rec.clone());
        handle(&mut s, &rec)
    };
    if let Some((status, body)) = reply {
        let msg = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = c.write_all(msg.as_bytes());
    }
}

/// `None` means close without answering.
fn handle(s: &mut State, rec: &Recorded) -> Option<(u16, String)> {
    let out = |code: u16, v: Value| Some((code, v.to_string()));
    if s.fail.0 > 0 {
        s.fail.0 -= 1;
        return out(s.fail.1, json!({}));
    }
    let (path, query) = rec.path.split_once('?').unwrap_or((&rec.path, ""));
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let ["v1", "ledger", p, action] = parts[..] else { return out(404, json!({})) };
    let bearer = rec.headers.iter().find(|h| h.0 == "authorization").and_then(|h| h.1.strip_prefix("Bearer "));
    let owner = s.projects.iter().find(|(_, x)| Some(x.token.as_str()) == bearer);
    match owner {
        Some((_, x)) if !x.revoked => {}
        _ => return out(401, json!({})),
    }
    if owner.unwrap().0 != p {
        return out(403, json!({}));
    }
    let name = s.projects[p].name.clone();
    match (rec.method.as_str(), action) {
        ("GET", "head") => {
            let r = query.split('&').find_map(|q| q.strip_prefix("ruleset=")).unwrap_or(R);
            let c = s.chains.get(&(p.into(), r.into()));
            out(200, head(c.map_or(&[][..], |v| v)))
        }
        ("POST", "entries") => {
            if let Some(f) = s.before_upload.take() {
                f(s);
            }
            let drop_after = std::mem::take(&mut s.drop_upload);
            let b: Value = serde_json::from_str(&rec.body).unwrap();
            let prev = b["prev_hash"].as_str().unwrap().to_string();
            let es: Vec<LedgerEntry> = serde_json::from_value(b["entries"].clone()).unwrap();
            if es.len() > s.max_batch {
                return out(413, json!({"error": "batch_too_large", "max": s.max_batch}));
            }
            let bad = |index: usize, reason: &str| {
                out(422, json!({"error": "broken_chain", "index": index, "reason": reason}))
            };
            let r = es[0].ruleset.clone();
            let current = s.ruleset.clone();
            let chain = s.chains.entry((p.into(), r.clone())).or_default();
            if r != current && chain.is_empty() {
                return bad(0, "unknown_ruleset");
            }
            if let Some(i) = es.iter().position(|e| e.project.as_deref() != Some(name.as_str())) {
                return bad(i, "project_mismatch");
            }
            if let Err(e) = verify_chain(&es, &prev) {
                return bad(e.index, e.reason);
            }
            let stale = out(
                409,
                json!({"error": "stale_prev_hash", "head_hash": head_hash(chain), "count": chain.len()}),
            );
            let start = if prev == head_hash(chain) {
                chain.len()
            } else if prev == GENESIS_HASH {
                0
            } else if let Some(i) = chain.iter().position(|e| e.hash == prev) {
                i + 1
            } else {
                return stale;
            };
            let overlap = chain.len().min(start + es.len()) - start;
            if (0..overlap).any(|k| chain[start + k].hash != es[k].hash) {
                return stale;
            }
            chain.extend_from_slice(&es[overlap..]);
            if drop_after {
                return None;
            }
            out(200, head(chain))
        }
        ("POST", "commit") => {
            let b: Value = serde_json::from_str(&rec.body).unwrap();
            let c = (b["hash"].as_str().unwrap().to_string(), b["ts"].as_i64().unwrap());
            s.commits.entry(p.into()).or_default().push(c);
            out(200, json!({}))
        }
        _ => out(404, json!({})),
    }
}

fn head_hash(c: &[LedgerEntry]) -> String {
    c.last().map_or(GENESIS_HASH.to_string(), |e| e.hash.clone())
}
fn head(c: &[LedgerEntry]) -> Value {
    json!({"head_hash": head_hash(c), "count": c.len()})
}
