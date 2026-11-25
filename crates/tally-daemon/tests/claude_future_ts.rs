//! A far-future timestamp must not pin an open turn: expiry runs on entries no newer than the poll clock.
use std::collections::BTreeMap;
use tally_daemon::claude_code::{line::parse_rfc3339_ms, ClaudeCode};
use tally_daemon::source::Source;

#[test]
fn future_timestamp_does_not_pin_the_open_turn() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("p")).unwrap();
    let l = |u: &str, ty: &str, t: &str, extra: &str, msg: &str| {
        format!(
            r#"{{"type":"{ty}","uuid":"{u}","sessionId":"s","timestamp":"{t}"{extra},"message":{msg}}}"#
        )
    };
    let body = [
        l(
            "p1",
            "user",
            "2026-11-22T10:00:00.000Z",
            r#","origin":{"kind":"human"}"#,
            r#"{"content":"go"}"#,
        ),
        l(
            "a1",
            "assistant",
            "2099-01-01T00:00:00.000Z",
            "",
            r#"{"content":[{"type":"text","text":"hi"}]}"#,
        ),
    ]
    .join("\n")
        + "\n";
    std::fs::write(dir.path().join("p/s.jsonl"), body).unwrap();
    let t0 = parse_rfc3339_ms("2026-11-22T10:00:00.000Z").unwrap();
    let mut src = ClaudeCode::new(dir.path().to_path_buf());
    let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
    let p = src.poll(t0 + 30 * 86_400_000, &mut st).unwrap();
    assert!(
        p.watermark.is_some_and(|w| w > t0),
        "watermark pinned at {:?} by a future-dated entry",
        p.watermark
    );
}
