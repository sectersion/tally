//! Submission of a project's claims for a time range.
pub mod scrub;
pub mod timeline;
pub mod transcript;
pub mod upload;

use crate::api::ApiError;
use crate::published::PublishError;
use crate::store::{Store, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use tally_scoring::{Event, Kind};

pub const BUNDLE_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const BUNDLE_FORMAT: &str = "tally-bundle/1";
pub const WARNING: &str = "Secrets the scrubber does not recognise are sent to Tally's own judge and deleted after review. Read the turns below.";

#[derive(thiserror::Error, Debug)]
pub enum SubmitError {
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Published(#[from] PublishError),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("reading session logs: {0}")]
    Io(#[from] std::io::Error),
    #[error("bundle is {bytes} bytes; submit a shorter range with --from/--to")]
    TooLarge { bytes: usize },
    #[error("bundle json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub ruleset: String,
    pub project: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub created_ms: i64,
    pub client: String,
    pub timeline: Vec<Event>,
    pub excluded_action_ids: Vec<String>,
    pub turns: Vec<BundleTurn>,
    pub redactions: BTreeMap<String, usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BundleTurn {
    pub action_id: String,
    pub ts: i64,
    pub prompt: Option<String>,
    pub reply: Option<String>,
    pub missing: Option<String>,
}

pub fn build_bundle(store: &mut Store, logs_dir: &Path, project: &str, from_ms: i64, to_ms: i64, now_ms: i64) -> Result<Bundle, SubmitError> {
    build_bundle_with_limit(store, logs_dir, project, from_ms, to_ms, now_ms, BUNDLE_MAX_BYTES)
}

pub fn build_bundle_with_limit(
    store: &mut Store,
    logs_dir: &Path,
    project: &str,
    from_ms: i64,
    to_ms: i64,
    now_ms: i64,
    limit: usize,
) -> Result<Bundle, SubmitError> {
    // select_timeline compares against the published rows, so bring them up to date first
    crate::published::sync(store, true)?;
    let t = timeline::select_timeline(store, project, from_ms, to_ms)?;
    // only the project's own prompts inside the range have text to read
    let prompts: Vec<Event> = t
        .events
        .iter()
        .filter(|e| matches!(e.kind, Kind::Prompt { .. }) && e.project.as_deref() == Some(project) && (from_ms..=to_ms).contains(&e.ts))
        .cloned()
        .collect();
    let mut redactions: BTreeMap<String, usize> = BTreeMap::new();
    let mut clean = |s: String| {
        let r = scrub::scrub(&s);
        for (k, n) in r.counts {
            *redactions.entry(k.to_string()).or_default() += n;
        }
        r.text
    };
    let turns = transcript::read_turns(logs_dir, &prompts)?
        .into_iter()
        .map(|t| BundleTurn {
            action_id: t.action_id,
            ts: t.ts,
            prompt: t.prompt.map(&mut clean),
            reply: t.reply.map(&mut clean),
            missing: t.missing.map(String::from),
        })
        .collect();
    let b = Bundle {
        format: BUNDLE_FORMAT.into(),
        ruleset: tally_scoring::RULESET_VERSION.into(),
        project: project.into(),
        from_ms,
        to_ms,
        created_ms: now_ms,
        client: concat!("tally/", env!("CARGO_PKG_VERSION")).into(),
        timeline: t.events,
        excluded_action_ids: t.excluded_action_ids,
        turns,
        redactions,
    };
    let bytes = serde_json::to_vec(&b)?.len();
    if bytes > limit {
        return Err(SubmitError::TooLarge { bytes });
    }
    Ok(b)
}

fn utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(|| ms.to_string(), |d| d.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// The summary block alone (what the CLI prints before asking).
pub fn summary_text(b: &Bundle) -> String {
    let mut s = String::new();
    let day = |ms| utc(ms)[..10].to_string();
    let _ = writeln!(s, "project: {}", b.project);
    let _ = writeln!(s, "range: {} to {} (UTC)", day(b.from_ms), day(b.to_ms));
    let _ = writeln!(s, "turns: {}", b.turns.len());
    let mut missing: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &b.turns {
        if let Some(m) = &t.missing {
            *missing.entry(m).or_default() += 1;
        }
    }
    let list = |m: Vec<String>| if m.is_empty() { "none".to_string() } else { m.join(", ") };
    let _ = writeln!(s, "turns without text: {}", list(missing.iter().map(|(k, n)| format!("{k} {n}")).collect()));
    let _ = writeln!(s, "redactions: {}", list(b.redactions.iter().map(|(k, n)| format!("{k} {n}")).collect()));
    let other = b.timeline.iter().filter(|e| e.project.is_none()).count();
    let _ = writeln!(s, "events from other projects: {other} (timing only, no content)");
    let _ = writeln!(s, "{WARNING}");
    s
}

pub fn preview_text(b: &Bundle) -> String {
    let mut s = summary_text(b);
    for t in &b.turns {
        let _ = writeln!(s, "\n--- {} {}", utc(t.ts), t.action_id);
        match (&t.prompt, &t.reply) {
            (None, None) => {
                let _ = writeln!(s, "(no text: {})", t.missing.as_deref().unwrap_or("unknown"));
            }
            (p, r) => {
                let _ = writeln!(s, "> {}", p.as_deref().unwrap_or("").replace('\n', "\n> "));
                let _ = writeln!(s, "< {}", r.as_deref().unwrap_or("").replace('\n', "\n< "));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::test_support::prompt;

    const FROM: i64 = 10_000_000;
    const TO: i64 = 12_000_000;
    const KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn fixture() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("p")).unwrap();
        let line = |u: &str, t: &str| {
            format!(r#"{{"type":"user","uuid":"{u}","sessionId":"s1","timestamp":"2026-11-22T00:00:00.000Z","origin":{{"kind":"human"}},"message":{{"role":"user","content":"{t}"}}}}"#)
        };
        let text = format!("{}\n{}\n", line("u1", &format!("use key {KEY} please")), line("u2", "second"));
        std::fs::write(dir.path().join("p/s1.jsonl"), text).unwrap();
        let mut s = Store::open_in_memory().unwrap();
        s.register_project("p1", "C:/r", Some("o/one")).unwrap();
        let mut evs = vec![];
        for (k, u) in ["u1", "u2"].iter().enumerate() {
            let mut e = prompt(&format!("claude:s1:{u}:prompt"), FROM + 100_000 + k as i64 * 400_000, 600);
            e.source = "claude-code".into();
            e.session = Some("s1".into());
            evs.push(e);
        }
        s.append_events(&evs).unwrap();
        s.set_watermark("src", 100_000_000).unwrap();
        finalize(&mut s).unwrap();
        (s, dir)
    }

    #[test]
    fn bundle_round_trips_and_scrubs() {
        let (mut s, dir) = fixture();
        let b = build_bundle(&mut s, dir.path(), "o/one", FROM, TO, 5).unwrap();
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains("[REDACTED:aws_key]") && !json.contains(KEY));
        assert_eq!(b.redactions["aws_key"], 1);
        assert_eq!(b.turns.len(), 2);
        assert_eq!(serde_json::from_str::<Bundle>(&json).unwrap(), b);
    }

    #[test]
    fn too_large_is_refused() {
        let (mut s, dir) = fixture();
        let e = build_bundle_with_limit(&mut s, dir.path(), "o/one", FROM, TO, 5, 100).unwrap_err();
        assert!(matches!(e, SubmitError::TooLarge { .. }));
        assert!(e.to_string().contains("submit a shorter range with --from/--to"));
    }

    #[test]
    fn preview_lists_every_turn_and_the_warning() {
        let (mut s, dir) = fixture();
        let b = build_bundle(&mut s, dir.path(), "o/one", FROM, TO, 5).unwrap();
        let p = preview_text(&b);
        assert!(p.contains(WARNING) && p.contains("redactions: aws_key 1"));
        for t in &b.turns {
            assert!(p.contains(&format!(" {}\n", t.action_id)));
        }
        assert!(p.contains("> use key [REDACTED:aws_key] please") && p.contains("> second"));
    }
}
