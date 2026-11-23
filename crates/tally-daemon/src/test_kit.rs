//! Golden-test kit shared by every adapter test. Compiled only for tests or `--features test-kit`.
use crate::adapters::{Adapter, ParseCtx};
use crate::claude_code::features::ContextTail;
use std::path::{Path, PathBuf};
use tally_scoring::Event;

/// Deterministic context: any cwd containing "tallyproj" is project `proj`; features are `(chars, 500, chars)`.
pub struct StubCtx;

impl ParseCtx for StubCtx {
    fn project_for_dir(&self, cwd: &str) -> Option<(PathBuf, String)> {
        cwd.contains("tallyproj").then(|| (PathBuf::from("C:/tallyproj"), "proj".to_string()))
    }
    fn prompt_features(&self, _: Option<&Path>, text: &str, _: &str, _: &str, _: &ContextTail) -> (u32, u16, u32) {
        let chars = text.chars().count() as u32;
        (chars, 500, chars)
    }
}

/// Events as JSON with ids replaced by `e1, e2, ...` in `(ts, id)` order and times rebased to the first event
/// (`ts` becomes `t`, `end_ts` becomes ms since the first event), so goldens do not depend on clocks or id spelling.
pub fn normalize(events: &[Event]) -> serde_json::Value {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)));
    let base = sorted.first().map_or(0, |e| e.ts);
    let out = sorted
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut v = serde_json::to_value(e).unwrap();
            let o = v.as_object_mut().unwrap();
            o.insert("id".into(), format!("e{}", i + 1).into());
            o.remove("ts");
            o.insert("t".into(), (e.ts - base).into());
            if let Some(end) = o.get("end_ts").and_then(|x| x.as_i64()) {
                o.insert("end_ts".into(), (end - base).into());
            }
            v
        })
        .collect();
    serde_json::Value::Array(out)
}

/// Parses the first log file in `<crate>/tests/fixtures/<fixture_dir>` and compares with its `expected.json`.
pub fn golden<A: Adapter>(fixture_dir: &str) {
    golden_at::<A>(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture_dir));
}

pub fn golden_at<A: Adapter>(dir: &Path) {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("fixture dir {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            name != "expected.json"
                && matches!(p.extension().and_then(|x| x.to_str()), Some("jsonl" | "json" | "md" | "ndjson" | "txt"))
        })
        .collect();
    logs.sort();
    let log = logs.first().unwrap_or_else(|| panic!("no log file in {}", dir.display()));
    let text = std::fs::read_to_string(log).unwrap();
    let (parsed, report) = A::parse(&text, &StubCtx);
    let allowed: usize = std::fs::read_to_string(dir.join("allow_errors")).map_or(0, |s| s.trim().parse().unwrap());
    assert_eq!(report.errors, allowed, "{}: parse errors", dir.display());
    let actual = normalize(&parsed.events);
    let expected_path = dir.join("expected.json");
    if std::env::var_os("TALLY_BLESS").is_some() {
        std::fs::write(&expected_path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&expected_path).unwrap()).unwrap();
    assert_eq!(actual, expected, "{} differs from expected.json (TALLY_BLESS=1 to regenerate)", dir.display());
}

/// Panics when the serialized events contain any forbidden substring (prompt words, a path).
pub fn assert_no_text_leak(events: &[Event], forbidden: &[&str]) {
    let json = serde_json::to_string(events).unwrap();
    for f in forbidden {
        assert!(!json.contains(f), "events leak {f:?}");
    }
}
