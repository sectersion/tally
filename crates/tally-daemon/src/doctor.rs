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
}

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
    }
}

/// One row per stored health row, plus a `NoLogs` row for each id in `configured` without one.
pub fn report(store: &Store, configured: &[&str]) -> Result<Vec<HarnessHealth>, StoreError> {
    let mut v: Vec<_> = store.get_health()?.iter().map(|(s, h, _)| from_health(s, h)).collect();
    for c in configured {
        if !v.iter().any(|h| h.source == *c) {
            v.push(from_health(c, &Health::default()));
        }
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
    let format = if h.format.is_empty() { "-" } else { &h.format };
    let counters: String = h.counters.iter().map(|(k, v)| format!("  {k}={v}")).collect();
    format!("{}  {}  files={}  last={}  format={}  errors={}/{}{}", h.source, status, h.files_found, last, format, h.errors, h.records, counters)
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
        let r = report(&s, &["claude-code", "codex"]).unwrap();
        assert_eq!(
            r.iter().map(|x| (x.source.as_str(), x.status)).collect::<Vec<_>>(),
            [("claude-code", Status::NoLogs), ("codex", Status::Ok)]
        );
        assert_eq!(line(&r[1]), "codex  ok  files=3  last=never  format=rollout-v1  errors=0/412");
    }
}
