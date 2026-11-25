//! A session-log writer for the resource harness. Line shapes are copied from the plan-03 fixture
//! (`crates/tally-daemon/tests/fixtures/claude_code/FORMAT.md`), not invented: human prompt, assistant text,
//! assistant `tool_use` + user `tool_result` pairs, and a `turn_duration` system entry.
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub trait Clock {
    fn now_ms(&self) -> i64;
    fn sleep_ms(&mut self, ms: u64);
}

pub struct RealClock;
impl Clock for RealClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
    }
    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

#[derive(Clone, Debug)]
pub struct LoadPlan {
    pub hours: u32,
    pub prompts_per_hour: u32,
    pub tool_events_per_prompt: u32,
    pub seed: u64,
    /// 1 = real time. N = the same cadence N times faster (timestamps stay real wall-clock times, so nothing is in the future).
    pub speedup: u64,
    /// The `cwd` recorded in every entry; point it at a git repository so the adapter's project lookup works.
    pub cwd: PathBuf,
}

impl LoadPlan {
    pub fn default_for(cwd: PathBuf) -> Self {
        Self { hours: 8, prompts_per_hour: 40, tool_events_per_prompt: 6, seed: 1, speedup: 1, cwd }
    }
}

struct Writer<'a> {
    f: std::fs::File,
    session: String,
    cwd: String,
    n: u64,
    last: Option<String>,
    clock: &'a dyn Clock,
}

impl Writer<'_> {
    fn uuid(&mut self) -> String {
        self.n += 1;
        format!("00000000-0000-4000-8000-{:012}", self.n)
    }
    fn ts(&self) -> String {
        let ms = self.clock.now_ms();
        let secs = ms.div_euclid(1000);
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        // civil-from-days (Howard Hinnant's algorithm)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, ms.rem_euclid(1000))
    }
    fn put(&mut self, kind: &str, extra: Value) -> std::io::Result<()> {
        let uuid = self.uuid();
        let mut v = json!({
            "parentUuid": self.last, "isSidechain": false, "userType": "external", "entrypoint": "cli", "cwd": self.cwd,
            "sessionId": self.session, "version": "2.1.0", "gitBranch": "main", "type": kind, "uuid": uuid, "timestamp": self.ts(),
        });
        v.as_object_mut().expect("object").extend(extra.as_object().expect("object").clone());
        self.last = Some(uuid);
        writeln!(self.f, "{v}")?;
        self.f.flush()
    }
}

/// Appends one session log under `<dir>/-tally-load/`. Returns the file written.
pub fn write_session_log(dir: &Path, plan: &LoadPlan, clock: &mut dyn Clock) -> std::io::Result<PathBuf> {
    let proj = dir.join("-tally-load");
    std::fs::create_dir_all(&proj)?;
    let session = format!("aaaaaaaa-0000-4000-8000-{:012}", plan.seed);
    let path = proj.join(format!("{session}.jsonl"));
    let f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let total = u64::from(plan.hours) * u64::from(plan.prompts_per_hour);
    let gap_ms = 3_600_000 / u64::from(plan.prompts_per_hour.max(1));
    let mut w_n = plan.seed * 1_000_000;
    for i in 0..total {
        let mut w = Writer { f: f.try_clone()?, session: session.clone(), cwd: plan.cwd.display().to_string(), n: w_n, last: None, clock: &*clock };
        w.put(
            "user",
            json!({
                "message": {"role": "user", "content": format!("Placeholder prompt {i}: please look at the parser and explain why the totals look wrong after the second import.")},
                "promptId": format!("prompt-load-{i}"), "origin": {"kind": "human"}, "turnOrigin": "human", "promptSource": "typed",
            }),
        )?;
        for k in 0..plan.tool_events_per_prompt {
            let tid = format!("toolu_load_{i}_{k}");
            w.put(
                "assistant",
                json!({"message": {"id": format!("msg_load_{i}_{k}"), "type": "message", "role": "assistant", "model": "model-load",
                    "content": [{"type": "tool_use", "id": tid, "name": "Bash", "input": {"command": "cargo test -p demo", "description": "placeholder"}}],
                    "stop_reason": "tool_use", "stop_sequence": null, "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            )?;
            w.put(
                "user",
                json!({"message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tid, "content": "ok"}]},
                    "toolUseResult": {"stdout": "ok"}, "sourceToolAssistantUUID": w.last, "permissionDecision": {"decision": "accept", "source": "config"}}),
            )?;
        }
        w.put(
            "assistant",
            json!({"message": {"id": format!("msg_load_{i}_end"), "type": "message", "role": "assistant", "model": "model-load",
                "content": [{"type": "text", "text": "Placeholder assistant prose."}], "stop_reason": "end_turn", "stop_sequence": null,
                "usage": {"input_tokens": 1, "output_tokens": 1}}}),
        )?;
        w.put("system", json!({"subtype": "turn_duration", "durationMs": 30_000, "messageCount": 2, "isMeta": false}))?;
        w_n = w.n;
        clock.sleep_ms(gap_ms / plan.speedup.max(1));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(i64);
    impl Clock for Fake {
        fn now_ms(&self) -> i64 {
            self.0
        }
        fn sleep_ms(&mut self, ms: u64) {
            self.0 += ms as i64;
        }
    }

    #[test]
    fn writes_one_human_prompt_per_prompt_and_valid_json_lines() {
        let d = tempfile::tempdir().unwrap();
        let plan = LoadPlan { hours: 1, prompts_per_hour: 40, tool_events_per_prompt: 6, seed: 1, speedup: 1000, cwd: d.path().into() };
        let p = write_session_log(d.path(), &plan, &mut Fake(1_700_000_000_000)).unwrap();
        let lines: Vec<Value> = std::fs::read_to_string(p).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let humans = lines.iter().filter(|v| v["type"] == "user" && v["origin"]["kind"] == "human").count();
        assert_eq!(humans, 40);
        assert_eq!(lines.len(), 40 * (1 + 6 * 2 + 2));
        assert_eq!(lines[0]["timestamp"], "2023-11-14T22:13:20.000Z");
        let mut ids: Vec<_> = lines.iter().map(|v| v["uuid"].as_str().unwrap().to_string()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), lines.len(), "uuids must be unique");
    }
}
