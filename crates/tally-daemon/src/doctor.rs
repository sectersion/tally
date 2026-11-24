//! `tally doctor`: per-harness health read from the `adapter_health` table (never parses logs itself).
use crate::source::Health;
use crate::store::{Store, StoreError};
use serde::Serialize;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    NoLogs,
    /// error rate >= 5%
    Degraded,
    FormatUnknown,
    Unsupported,
    NotInstalled,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct HarnessHealth {
    pub source: String,
    pub files_found: usize,
    pub last_event_ts: Option<i64>,
    pub records: usize,
    pub errors: usize,
    pub skipped: usize,
    pub format: String,
    pub status: Status,
    /// Source-specific counters (heartbeat: accepted, dropped_agent, ...).
    pub counters: Vec<(String, u64)>,
    /// Seconds since `last_event_ts` when the report was made (None if never seen).
    pub last_seen_ago_s: Option<i64>,
    /// Signals this adapter never emits (copilot).
    pub unsupported_signals: Vec<String>,
    /// Why a row has no stats (harnesses without an adapter).
    pub note: Option<String>,
}

/// Harnesses with a docs page but no adapter yet (docs/adapters/README.md).
pub const UNSUPPORTED_HARNESSES: [&str; 4] = ["aider", "codex", "cursor", "gemini-cli"];
const COPILOT_GAPS: [&str; 3] = ["interrupt", "tool_decision", "agent_write"];

/// 1000 * errors / max(records, 1)
pub fn error_rate_pm(h: &HarnessHealth) -> u32 {
    (1000 * h.errors / h.records.max(1)) as u32
}

fn status_of(h: &Health) -> Status {
    let pm = 1000 * h.errors / h.records.max(1);
    if h.files_found == 0 {
        Status::NoLogs
    } else if h.format == "unknown" {
        Status::FormatUnknown
    } else if pm >= 50 {
        Status::Degraded
    } else {
        Status::Ok
    }
}

/// `3 min ago`, `2 h ago`, ... (a clock set back reads as `0 s ago`).
pub fn age_text(s: i64) -> String {
    let s = s.max(0);
    match s {
        0..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86400),
    }
}

fn from_health(source: &str, h: &Health) -> HarnessHealth {
    HarnessHealth {
        source: source.to_string(),
        files_found: h.files_found,
        last_event_ts: h.last_event_ts,
        records: h.records,
        errors: h.errors,
        skipped: h.skipped,
        format: h.format.clone(),
        status: status_of(h),
        counters: h.counters.clone(),
        last_seen_ago_s: None,
        unsupported_signals: if source == "copilot" { COPILOT_GAPS.map(String::from).to_vec() } else { vec![] },
        note: None,
    }
}

/// One row per stored health row, plus a `NoLogs` row for each id in `configured` without one.
/// Also one `Unsupported` row per harness without an adapter, and each last-seen as an age against `now_ms`.
pub fn report(store: &Store, configured: &[&str], now_ms: i64) -> Result<Vec<HarnessHealth>, StoreError> {
    let mut v: Vec<_> = store.get_health()?.iter().map(|(s, h, _)| from_health(s, h)).collect();
    for c in configured {
        if !v.iter().any(|h| h.source == *c) {
            v.push(from_health(c, &Health::default()));
        }
    }
    for u in UNSUPPORTED_HARNESSES.iter().filter(|u| !v.iter().any(|h| h.source == **u)).copied().collect::<Vec<_>>() {
        let mut row = from_health(u, &Health::default());
        row.status = Status::Unsupported;
        row.note = Some(format!("no adapter built; see docs/adapters/{u}.md"));
        v.push(row);
    }
    for h in &mut v {
        h.last_seen_ago_s = h.last_event_ts.map(|t| (now_ms - t) / 1000);
    }
    v.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(v)
}

/// `codex  ok  files=3  last=2026-11-23T10:00:00Z  format=rollout-v1  errors=0/412`
pub fn line(h: &HarnessHealth) -> String {
    let status = serde_json::to_value(h.status).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let last = h
        .last_event_ts
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or("never".to_string(), |t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    if let Some(note) = &h.note {
        return format!("{}  {}  {}", h.source, status, note);
    }
    let last = match h.last_seen_ago_s {
        Some(a) if h.last_event_ts.is_some() => format!("{last} ({})", age_text(a)),
        _ => last,
    };
    let format = if h.format.is_empty() { "-" } else { &h.format };
    let counters: String = h.counters.iter().map(|(k, v)| format!("  {k}={v}")).collect();
    let gaps = if h.unsupported_signals.is_empty() { String::new() } else { format!("  unsupported_signals={}", h.unsupported_signals.join(",")) };
    format!("{}  {}  files={}  last={}  format={}  errors={}/{}{}{}", h.source, status, h.files_found, last, format, h.errors, h.records, counters, gaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(files: usize, records: usize, errors: usize, format: &str) -> Health {
        Health { files_found: files, records, errors, format: format.into(), ..Default::default() }
    }

    #[test]
    fn error_rate_is_per_mille() {
        assert_eq!(error_rate_pm(&from_health("x", &h(1, 200, 5, "v1"))), 25);
        assert_eq!(error_rate_pm(&from_health("x", &h(1, 0, 0, "v1"))), 0);
    }
    #[test]
    fn status_degraded_at_five_percent() {
        assert_eq!(status_of(&h(1, 1000, 49, "v1")), Status::Ok);
        assert_eq!(status_of(&h(1, 1000, 50, "v1")), Status::Degraded);
    }
    #[test]
    fn no_files_means_no_logs() {
        assert_eq!(status_of(&h(0, 0, 0, "v1")), Status::NoLogs);
    }
    #[test]
    fn unknown_format_beats_ok() {
        assert_eq!(status_of(&h(2, 10, 0, "unknown")), Status::FormatUnknown);
    }
    #[test]
    fn report_lists_every_configured_source_even_without_a_row() {
        let mut s = Store::open_in_memory().unwrap();
        s.put_health("codex", &h(3, 412, 0, "rollout-v1"), 1).unwrap();
        let r = report(&s, &["claude-code", "codex"], 0).unwrap();
        assert_eq!(
            r.iter().map(|x| (x.source.as_str(), x.status)).collect::<Vec<_>>(),
            [
                ("aider", Status::Unsupported),
                ("claude-code", Status::NoLogs),
                ("codex", Status::Ok),
                ("cursor", Status::Unsupported),
                ("gemini-cli", Status::Unsupported)
            ]
        );
        assert_eq!(line(&r[2]), "codex  ok  files=3  last=never  format=rollout-v1  errors=0/412");
    }
    #[test]
    fn unsupported_row_points_at_its_docs_page() {
        let r = report(&Store::open_in_memory().unwrap(), &[], 0).unwrap();
        assert_eq!(line(&r[0]), "aider  unsupported  no adapter built; see docs/adapters/aider.md");
    }
    #[test]
    fn last_seen_is_shown_as_an_age_and_copilot_lists_its_gaps() {
        let mut s = Store::open_in_memory().unwrap();
        let mut hh = h(1, 4, 0, "events-v1");
        hh.last_event_ts = Some(1_000_000);
        s.put_health("copilot", &hh, 1).unwrap();
        let r = report(&s, &[], 1_000_000 + 180_000).unwrap();
        let c = r.iter().find(|x| x.source == "copilot").unwrap();
        assert_eq!(c.last_seen_ago_s, Some(180));
        assert_eq!(
            line(c),
            "copilot  ok  files=1  last=1970-01-01T00:16:40Z (3 min ago)  format=events-v1  errors=0/4  unsupported_signals=interrupt,tool_decision,agent_write"
        );
    }
    #[test]
    fn age_text_picks_a_unit() {
        assert_eq!(age_text(-5), "0 s ago");
        assert_eq!(age_text(59), "59 s ago");
        assert_eq!(age_text(7200), "2 h ago");
        assert_eq!(age_text(3 * 86400), "3 d ago");
    }
}
