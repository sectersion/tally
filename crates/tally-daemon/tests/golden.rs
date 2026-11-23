use tally_daemon::adapters::{Adapter, ParseCtx, ParseReport, Parsed};
use tally_daemon::test_kit::{assert_no_text_leak, golden, golden_at, normalize, StubCtx};
use tally_scoring::{score, Event, Kind};

/// Self-test adapter: each line is `<ts_ms>,<prompt|edit>`.
struct Mini;
impl Adapter for Mini {
    const ID: &'static str = "mini";
    fn parse(text: &str, _: &dyn ParseCtx) -> (Parsed, ParseReport) {
        let mut p = Parsed { events: vec![], agent_write_hashes: vec![] };
        let mut r = ParseReport { format: "v1".into(), ..Default::default() };
        for (i, line) in text.lines().enumerate() {
            r.records += 1;
            let Some((ts, kind)) = line.split_once(',').and_then(|(t, k)| Some((t.parse::<i64>().ok()?, k))) else {
                r.errors += 1;
                continue;
            };
            let kind_v = match kind {
                "prompt" => Kind::Prompt { chars: 10, relevance_pm: 500, novelty_bytes: 10 },
                "edit" => Kind::UserEdit { file_hash: "f".into(), lines_changed: 1 },
                _ => {
                    r.skipped += 1;
                    continue;
                }
            };
            p.events.push(Event {
                v: 1,
                id: format!("mini:{i}:{kind}"),
                ts,
                source: "mini".into(),
                project: Some("proj".into()),
                session: None,
                kind: kind_v,
            });
            r.last_ts = Some(ts);
        }
        (p, r)
    }
}

fn mini(text: &str) -> Vec<Event> {
    Mini::parse(text, &StubCtx).0.events
}

#[test]
fn normalize_replaces_ids_and_rebases_time() {
    let v = normalize(&mini("1000,prompt\n2500,edit\n"));
    assert_eq!(v[0]["id"], "e1");
    assert_eq!(v[0]["t"], 0);
    assert_eq!(v[1]["id"], "e2");
    assert_eq!(v[1]["t"], 1500);
    assert!(v[0].get("ts").is_none());
}

#[test]
fn golden_passes_on_matching_expected() {
    let d = tempfile::tempdir().unwrap();
    std::fs::copy("tests/fixtures/_selftest/v1/log.txt", d.path().join("log.txt")).unwrap();
    std::fs::write(
        d.path().join("expected.json"),
        serde_json::to_string(&normalize(&mini("1000,prompt\n2500,edit\n4000,prompt\n"))).unwrap(),
    )
    .unwrap();
    golden_at::<Mini>(d.path());
}

#[test]
fn committed_selftest_fixture_matches() {
    golden::<Mini>("_selftest/v1");
}

#[test]
fn bless_writes_expected() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("log.txt"), "1000,prompt\n").unwrap();
    // The env var is process-wide; set and clear inside this one test only.
    std::env::set_var("TALLY_BLESS", "1");
    golden_at::<Mini>(d.path());
    std::env::remove_var("TALLY_BLESS");
    let written = std::fs::read_to_string(d.path().join("expected.json")).unwrap();
    assert!(written.contains("\"e1\""));
    golden_at::<Mini>(d.path());
}

#[test]
#[should_panic(expected = "leak")]
fn no_text_leak_catches_a_substring() {
    assert_no_text_leak(&mini("1000,prompt\n"), &["mini"]);
}

#[test]
fn no_text_leak_passes_when_clean() {
    assert_no_text_leak(&mini("1000,prompt\n"), &["secret_word"]);
}

#[test]
fn two_harnesses_one_project_do_not_double_credit() {
    let secs = |evs: &[Event]| score(evs).iter().map(|c| c.verified_s + c.unverified_s).sum::<u32>();
    let rename = |mut evs: Vec<Event>, src: &str| {
        for e in &mut evs {
            e.source = src.into();
            e.id = format!("{src}:{}", e.id);
        }
        evs
    };
    let a = rename(mini("0,prompt\n60000,prompt\n"), "a");
    let b = rename(mini("5000,prompt\n65000,prompt\n"), "b");
    let merged: Vec<Event> = a.iter().chain(&b).cloned().collect();
    assert!(secs(&a) > 0);
    assert!(secs(&merged) < secs(&a) + secs(&b), "{} {} {}", secs(&merged), secs(&a), secs(&b));
}

mod copilot {
    use super::*;
    use tally_daemon::adapters::copilot::Copilot;

    const DIR: &str = "copilot/cli-1.0.111";
    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{DIR}/{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn copilot_cli_v1_golden() {
        golden::<Copilot>(DIR);
    }

    #[test]
    fn copilot_fixture_has_the_two_typed_prompts_and_session_project() {
        let (p, r) = Copilot::parse(&fixture("events.jsonl"), &StubCtx);
        let prompts = p.events.iter().filter(|e| matches!(e.kind, Kind::Prompt { .. })).count();
        assert_eq!((prompts, r.format.as_str(), r.records), (2, "cli-events-v1", 233));
        assert!(p.events.iter().all(|e| e.project.as_deref() == Some("proj") && e.source == "copilot"));
    }

    #[test]
    fn copilot_no_text_leak() {
        let (p, _) = Copilot::parse(&fixture("events.jsonl"), &StubCtx);
        assert_no_text_leak(&p.events, &["\"text\"", "\"cmd\"", "\"path\"", "\"content\"", "xxxx", "tallyproj"]);
    }

    #[test]
    fn copilot_format_drift() {
        let (clean, _) = Copilot::parse(&fixture("events.jsonl"), &StubCtx);
        let (p, r) = Copilot::parse(&fixture("drift/events.jsonl"), &StubCtx);
        assert_eq!(r.errors, 2); // one line cut mid-JSON, one without a timestamp
        assert!(r.skipped >= 1);
        // a renamed record type loses only that record's events (its prompt and busy span); nothing is invented
        assert!(p.events.iter().all(|e| clean.events.iter().any(|c| c.id == e.id)));
        let lost = clean.events.len() - p.events.len();
        assert!((1..=3).contains(&lost), "lost {lost}");
    }

    #[test]
    fn copilot_reread_gives_same_ids() {
        let ids = || -> Vec<String> { Copilot::parse(&fixture("events.jsonl"), &StubCtx).0.events.into_iter().map(|e| e.id).collect() };
        assert_eq!(ids(), ids());
        assert!(!ids().is_empty());
    }
}
