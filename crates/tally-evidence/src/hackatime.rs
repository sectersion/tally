//! A Hackatime/WakaTime-style hours estimate: heartbeats closer than a timeout join one span.
//! An APPROXIMATION of what those tools compute, built from Tally events unless a real heartbeat export is given.
use anyhow::{bail, Result};
use tally_scoring::{Event, Kind};

/// Hackatime/WakaTime merge heartbeats closer than the timeout. **The maintainer must confirm the timeout
/// Hackatime really uses and replace this constant** (2 minutes is the WakaTime default as far as is known).
pub const HACKATIME_TIMEOUT_MS: i64 = 120_000;
const AGENT_HEARTBEAT_EVERY_MS: i64 = 30_000;

/// Every event except `Idle` is a heartbeat at its `ts`; an `AgentBusy` span also yields one every 30 s across
/// `[ts, end_ts]` (an agent writing files produces editor heartbeats; PITCH: "Hackatime counts how active your AI agent is").
pub fn heartbeats_from_events(ev: &[Event]) -> Vec<i64> {
    let mut hb = vec![];
    for e in ev {
        match e.kind {
            Kind::Idle { .. } => {}
            Kind::AgentBusy { end_ts } => {
                let mut t = e.ts;
                while t <= end_ts {
                    hb.push(t);
                    t += AGENT_HEARTBEAT_EVERY_MS;
                }
            }
            _ => hb.push(e.ts),
        }
    }
    hb
}

/// Sorted, deduplicated heartbeats; consecutive ones closer than `timeout_ms` join one span.
pub fn hackatime_spans(hb: &[i64], timeout_ms: i64) -> Vec<(i64, i64)> {
    let mut v = hb.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut out: Vec<(i64, i64)> = vec![];
    for t in v {
        match out.last_mut() {
            Some(last) if t - last.1 < timeout_ms => last.1 = t,
            _ => out.push((t, t)),
        }
    }
    out
}

/// NDJSON lines `{"ts": <ms>}` from a real Hackatime export (preferred over the synthetic derivation).
pub fn from_heartbeat_file(text: &str) -> Result<Vec<i64>> {
    #[derive(serde::Deserialize)]
    struct H {
        ts: i64,
    }
    let mut out = vec![];
    for (i, l) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<H>(l) {
            Ok(h) => out.push(h.ts),
            Err(e) => bail!("heartbeat file line {}: {e}", i + 1),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::*;

    #[test]
    fn gaps_under_the_timeout_join_and_longer_gaps_split() {
        let s = hackatime_spans(&[0, 60_000, 150_000, 400_000], 120_000);
        assert_eq!(s, vec![(0, 150_000), (400_000, 400_000)]);
    }
    #[test]
    fn agent_busy_span_generates_heartbeats_every_30s() {
        let hb = heartbeats_from_events(&[busy("a", 0, 3_600_000)]);
        assert_eq!(hb.len(), 121);
        assert_eq!(hackatime_spans(&hb, HACKATIME_TIMEOUT_MS), vec![(0, 3_600_000)]);
    }
    #[test]
    fn idle_events_are_not_heartbeats() {
        assert!(heartbeats_from_events(&[idle("i", 0, 600_000)]).is_empty());
    }
    #[test]
    fn heartbeat_file_parses_and_names_the_bad_line() {
        assert_eq!(from_heartbeat_file("{\"ts\":5}\n\n{\"ts\":9}\n").unwrap(), [5, 9]);
        assert!(from_heartbeat_file("{\"ts\":5}\nnope\n").unwrap_err().to_string().contains("line 2"));
    }
}
