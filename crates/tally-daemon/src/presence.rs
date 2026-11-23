//! Idle-span and coverage tracking from periodic "ms since last input" samples. Pure logic.
use crate::source::SourceState;
use crate::store::Interval;
use tally_scoring::ruleset::IDLE_SAMPLE_MIN_MS;

/// Two samples further apart than this are a gap (sleep, daemon stopped, stalled poll).
pub const COVERAGE_MAX_GAP_MS: i64 = 60_000;
/// Tolerance for timer granularity when deciding that input resumed.
const RESUME_SLACK_MS: i64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleSpan {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Default)]
pub struct Step {
    pub closed: Vec<IdleSpan>,
    pub coverage: Vec<Interval>,
    pub watermark: i64,
}

#[derive(Default)]
pub struct Tracker {
    last_sample: Option<i64>,
    open: Option<i64>,
    wm: i64,
}

impl Tracker {
    pub fn load(state: &dyn SourceState) -> Tracker {
        let n = |k: &str| state.get(k).and_then(|v| v.parse::<i64>().ok());
        Tracker {
            last_sample: n("last_sample"),
            open: n("open_start"),
            wm: n("wm").unwrap_or(0),
        }
    }

    pub fn save(&self, state: &mut dyn SourceState) {
        let put = |state: &mut dyn SourceState, k: &str, v: Option<i64>| {
            // empty string = cleared (SourceState has no remove); load() parses it as None
            state.set(k, &v.map(|v| v.to_string()).unwrap_or_default());
        };
        put(state, "last_sample", self.last_sample);
        put(state, "open_start", self.open);
        state.set("wm", &self.wm.to_string());
    }

    pub fn step(&mut self, now: i64, idle_ms: u64) -> Step {
        let idle = idle_ms.min(i64::MAX as u64) as i64;
        let mut input = now.saturating_sub(idle);
        let reset = match self.last_sample {
            None => true,
            Some(l) => now < l || now - l > COVERAGE_MAX_GAP_MS,
        };
        let mut out = Step::default();
        if reset {
            if let (Some(l), Some(s)) = (self.last_sample, self.open.take()) {
                if l > s {
                    out.closed.push(IdleSpan { start: s, end: l });
                }
            }
            self.open = None;
            input = now;
        } else if let Some(l) = self.last_sample {
            out.coverage.push((l, now));
        }
        if let Some(s) = self.open {
            if input > s + RESUME_SLACK_MS {
                out.closed.push(IdleSpan {
                    start: s,
                    end: input,
                });
                self.open = None;
            }
        }
        if self.open.is_none() && idle >= IDLE_SAMPLE_MIN_MS {
            self.open = Some(input.max(self.wm + 1));
        }
        let wm = match self.open {
            Some(s) => s - 1,
            None => now - IDLE_SAMPLE_MIN_MS,
        };
        self.wm = self.wm.max(wm);
        out.watermark = self.wm;
        self.last_sample = Some(now);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // The default watermark is 0, so tests start at a realistic base time (1000 s) instead of 0.
    const B: i64 = 1_000_000;
    fn s(secs: i64) -> i64 {
        B + secs * 1000
    }
    fn run(steps: &[(i64, u64)]) -> (Vec<IdleSpan>, Vec<Interval>, i64) {
        let mut t = Tracker::default();
        let (mut c, mut cov, mut wm) = (vec![], vec![], 0);
        for &(secs, idle_s) in steps {
            let st = t.step(s(secs), idle_s * 1000);
            c.extend(st.closed);
            cov.extend(st.coverage);
            wm = st.watermark;
        }
        (c, cov, wm)
    }

    #[test]
    fn idle_span_starts_at_last_input_and_ends_when_input_resumes() {
        let mut steps: Vec<(i64, u64)> = (0..=7).map(|i| (i * 5, (i * 5) as u64)).collect();
        steps.push((95, 95));
        let (closed, _, _) = run(&steps);
        assert!(closed.is_empty());
        steps.push((101, 1));
        let (closed, _, _) = run(&steps);
        assert_eq!(
            closed,
            vec![IdleSpan {
                start: s(0),
                end: s(100)
            }]
        );
    }

    #[test]
    fn open_span_holds_the_watermark_below_its_start() {
        let mut t = Tracker::default();
        let mut wm = 0;
        for i in 0..=12 {
            let secs = i * 5;
            wm = t.step(s(secs), (secs * 1000) as u64).watermark;
            if secs >= 40 {
                assert_eq!(wm, s(0) - 1, "at {secs}s");
            }
        }
        // 60 s -> 119 s is within the gap limit; input resumed 1 s before
        let st = t.step(s(119), 1000);
        assert_eq!(
            st.closed,
            vec![IdleSpan {
                start: s(0),
                end: s(118)
            }]
        );
        assert!(st.watermark >= st.closed[0].end - 30_000);
        assert!(st.watermark > wm);
    }

    #[test]
    fn below_threshold_idle_emits_nothing() {
        let mut steps: Vec<(i64, u64)> = (0..10).map(|i| (i * 5, 29)).collect();
        steps.push((50, 0));
        let (closed, _, wm) = run(&steps);
        assert!(closed.is_empty());
        assert_eq!(wm, s(50) - 30_000);
    }

    #[test]
    fn coverage_extends_across_normal_samples_and_not_across_a_gap() {
        let (_, cov, _) = run(&[(0, 0), (5, 0), (10, 0), (5000, 0), (5005, 0)]);
        // per-step pieces; the store merges touching intervals
        assert_eq!(cov, vec![(s(0), s(5)), (s(5), s(10)), (s(5000), s(5005))]);
    }

    #[test]
    fn resume_after_sleep_does_not_emit_a_span_reaching_into_the_sleep() {
        let (closed, cov, wm) = run(&[(0, 0), (40, 40), (5000, 5000)]);
        assert_eq!(
            closed,
            vec![IdleSpan {
                start: s(0),
                end: s(40)
            }]
        );
        assert_eq!(cov, vec![(s(0), s(40))]);
        // the new span starts at the first post-gap sample, not earlier
        assert_eq!(wm, s(5000) - 1);
    }

    #[test]
    fn backward_clock_jump_resets_coverage_and_never_goes_below_the_watermark() {
        let mut t = Tracker::default();
        t.step(s(100), 0);
        let prev = t.step(s(105), 0);
        assert_eq!(prev.watermark, s(75));
        let st = t.step(s(50), 40_000);
        assert!(st.coverage.is_empty());
        assert!(st.closed.is_empty());
        // span opens at wm + 1, so the watermark stays put
        assert_eq!(st.watermark, s(75));
        // input resumes: the span closes above the old watermark
        let st = t.step(s(80), 0);
        assert_eq!(
            st.closed,
            vec![IdleSpan {
                start: s(75) + 1,
                end: s(80)
            }]
        );
        assert!(st.closed[0].start > prev.watermark);
        assert!(st.watermark >= prev.watermark);
    }

    #[test]
    fn forward_clock_jump_over_the_gap_limit_is_a_gap() {
        let (_, cov, _) = run(&[(0, 0), (61, 0)]);
        assert!(cov.is_empty());
        let (_, cov, _) = run(&[(0, 0), (60, 0)]);
        assert_eq!(cov, vec![(s(0), s(60))]);
    }

    #[test]
    fn state_roundtrip_continues_an_open_span_after_restart() {
        let mut t = Tracker::default();
        t.step(s(0), 0);
        let st = t.step(s(35), 35_000);
        assert_eq!(st.watermark, s(0) - 1);
        let mut m: BTreeMap<String, String> = BTreeMap::new();
        t.save(&mut m);
        assert_eq!(m.get("open_start"), Some(&s(0).to_string()));

        let mut t2 = Tracker::load(&m);
        let st = t2.step(s(40), 40_000);
        assert!(st.closed.is_empty());
        assert_eq!(st.coverage, vec![(s(35), s(40))]);
        assert_eq!(st.watermark, s(0) - 1);

        // restart after a long gap: the span is closed at the saved last_sample
        let mut t3 = Tracker::load(&m);
        let st = t3.step(s(35 + 61), 96_000);
        assert_eq!(
            st.closed,
            vec![IdleSpan {
                start: s(0),
                end: s(35)
            }]
        );
        assert!(st.coverage.is_empty());
    }
}
