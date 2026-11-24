//! Classify one Claude Code session-log line (see tests/fixtures/claude_code/FORMAT.md).
//! Text fields live only in memory; `LineError` never carries line content.

use serde_json::Value;
use tally_scoring::Decision;

#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub uuid: String,
    pub session: String,
    pub ts: i64,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    HumanPrompt { meta: Meta, text: String, queued: bool },
    Interrupt { meta: Meta },
    ToolDecision { meta: Meta, decision: Decision, n: usize },
    ToolResult { meta: Meta, tool_use_id: String, is_error: bool, text: String },
    AssistantText { meta: Meta, text: String, n: usize },
    ToolUse { meta: Meta, id: String, name: String, command: Option<String>, file_path: Option<String> },
    TurnEnd { meta: Meta },
}

impl Entry {
    pub fn meta(&self) -> &Meta {
        match self {
            Entry::HumanPrompt { meta, .. }
            | Entry::Interrupt { meta }
            | Entry::ToolDecision { meta, .. }
            | Entry::ToolResult { meta, .. }
            | Entry::AssistantText { meta, .. }
            | Entry::ToolUse { meta, .. }
            | Entry::TurnEnd { meta } => meta,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum LineError {
    Utf8,
    Json,
    Shape,
}

const INTERRUPT_PREFIX: &str = "[Request interrupted by user";

pub fn classify(line: &[u8]) -> Result<Vec<Entry>, LineError> {
    let s = std::str::from_utf8(line).map_err(|_| LineError::Utf8)?;
    let v: Value = serde_json::from_str(s).map_err(|_| LineError::Json)?;
    if !v.is_object() {
        return Err(LineError::Json);
    }
    let ty = v["type"].as_str().unwrap_or("");
    if !matches!(ty, "user" | "assistant" | "system") || v["isSidechain"].as_bool() == Some(true) {
        return Ok(vec![]);
    }
    let meta = Meta {
        uuid: v["uuid"].as_str().ok_or(LineError::Shape)?.to_owned(),
        session: v["sessionId"].as_str().ok_or(LineError::Shape)?.to_owned(),
        ts: v["timestamp"].as_str().and_then(parse_rfc3339_ms).ok_or(LineError::Shape)?,
        cwd: v["cwd"].as_str().map(str::to_owned),
    };
    Ok(match ty {
        "user" => user(&v, meta),
        "assistant" => assistant(&v, meta),
        _ if v["subtype"].as_str() == Some("turn_duration") => vec![Entry::TurnEnd { meta }],
        _ => vec![],
    })
}

/// Text of a content value: a string, or the concatenated `text` blocks of an array.
fn text_of(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|i| i["text"].as_str()).collect(),
        _ => String::new(),
    }
}

fn user(v: &Value, meta: Meta) -> Vec<Entry> {
    let content = &v["message"]["content"];
    let items = content.as_array();
    if v.get("toolUseResult").is_none()
        && items.is_some_and(|a| {
            a.iter().any(|i| {
                i["type"] == "text" && i["text"].as_str().is_some_and(|t| t.starts_with(INTERRUPT_PREFIX))
            })
        })
    {
        return vec![Entry::Interrupt { meta }];
    }
    let human = match v.get("origin") {
        Some(o) => o["kind"].as_str() == Some("human"),
        None => v["turnOrigin"].as_str() == Some("human"),
    };
    if human {
        return match content {
            Value::String(_) | Value::Array(_) => vec![Entry::HumanPrompt {
                meta,
                text: text_of(content),
                queued: v["promptSource"].as_str() == Some("queued"),
            }],
            _ => vec![],
        };
    }
    let mut out = Vec::new();
    for i in items.into_iter().flatten().filter(|i| i["type"] == "tool_result") {
        out.push(Entry::ToolResult {
            meta: meta.clone(),
            tool_use_id: i["tool_use_id"].as_str().unwrap_or("").to_owned(),
            is_error: i["is_error"].as_bool().unwrap_or(false),
            text: text_of(&i["content"]),
        });
    }
    // One permissionDecision per entry; it applies to the entry as a whole (n = 0).
    let pd = &v["permissionDecision"];
    if !out.is_empty() && pd["source"].as_str().is_some_and(|s| s != "config") {
        let decision = match pd["decision"].as_str() {
            Some("accept") => Some(Decision::Approve),
            Some("reject") => Some(Decision::Deny),
            _ => None,
        };
        if let Some(decision) = decision {
            out.push(Entry::ToolDecision { meta, decision, n: 0 });
        }
    }
    out
}

fn assistant(v: &Value, meta: Meta) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut n = 0;
    for i in v["message"]["content"].as_array().into_iter().flatten() {
        match i["type"].as_str() {
            Some("text") => {
                out.push(Entry::AssistantText {
                    meta: meta.clone(),
                    text: i["text"].as_str().unwrap_or("").to_owned(),
                    n,
                });
                n += 1;
            }
            Some("tool_use") => {
                let name = i["name"].as_str().unwrap_or("").to_owned();
                let inp = &i["input"];
                let key = match name.as_str() {
                    "Edit" | "Write" | "MultiEdit" => Some("file_path"),
                    "NotebookEdit" => Some(if inp["file_path"].is_string() { "file_path" } else { "notebook_path" }),
                    _ => None,
                };
                out.push(Entry::ToolUse {
                    meta: meta.clone(),
                    id: i["id"].as_str().unwrap_or("").to_owned(),
                    command: inp["command"].as_str().map(str::to_owned),
                    file_path: key.and_then(|k| inp[k].as_str()).map(str::to_owned),
                    name,
                });
            }
            _ => {}
        }
    }
    out
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z` only; fraction truncated to ms.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.strip_suffix('Z')?.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let d = &b[r];
        d.iter().all(u8::is_ascii_digit).then(|| d.iter().fold(0, |a, c| a * 10 + (c - b'0') as i64))
    };
    let (y, m, d, hh, mm, ss) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut ms = 0;
    if b.len() > 19 {
        if b[19] != b'.' || b.len() == 20 || !b[20..].iter().all(u8::is_ascii_digit) {
            return None;
        }
        for k in 0..3 {
            ms = ms * 10 + b.get(20 + k).map_or(0, |c| (c - b'0') as i64);
        }
    }
    let dim = |y: i64, m: i64| match m {
        2 => 28 + (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) as i64,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&m) || d < 1 || d > dim(y, m) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days-from-civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ty: &str, extra: &str, message: &str) -> String {
        format!(
            r#"{{"type":"{ty}","uuid":"u","sessionId":"s","timestamp":"2026-11-22T00:00:00Z","cwd":"/c"{extra},"message":{message}}}"#
        )
    }
    fn human(extra: &str) -> String {
        line("user", extra, r#"{"role":"user","content":"hi"}"#)
    }
    fn cls(l: &str) -> Vec<Entry> {
        classify(l.as_bytes()).unwrap()
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56.789Z"), Some(1_795_350_896_789));
        assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56.7891Z"), Some(1_795_350_896_789));
        assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56+01:00"), None);
        assert_eq!(parse_rfc3339_ms("garbage"), None);
        assert_eq!(parse_rfc3339_ms("2026-02-31T10:00:00Z"), None);
        assert_eq!(parse_rfc3339_ms("2026-04-31T10:00:00Z"), None);
        assert_eq!(parse_rfc3339_ms("2026-02-29T10:00:00Z"), None);
        assert!(parse_rfc3339_ms("2028-02-29T10:00:00Z").is_some());
    }

    #[test]
    fn bad_bytes_and_bad_json_are_errors_not_panics() {
        assert_eq!(classify(b"{\"type\":\"user\"\xff}").err(), Some(LineError::Utf8));
        assert_eq!(classify(b"{\"type\":\"user\",\"message\":").err(), Some(LineError::Json));
        assert_eq!(classify(b"[1,2]").err(), Some(LineError::Json));
        assert_eq!(classify(br#"{"type":"user","uuid":"u"}"#).err(), Some(LineError::Shape));
    }

    #[test]
    fn human_prompt_vs_task_notification_vs_sidechain() {
        let h = human(r#","origin":{"kind":"human"}"#);
        assert!(matches!(&cls(&h)[..], [Entry::HumanPrompt { text, queued: false, .. }] if text == "hi"));
        assert!(cls(&human(r#","origin":{"kind":"task-notification"}"#)).is_empty());
        assert!(cls(&human(r#","origin":{"kind":"human"},"isSidechain":true"#)).is_empty());
    }

    #[test]
    fn human_without_origin_needs_turn_origin_human() {
        assert!(matches!(&cls(&human(r#","turnOrigin":"human""#))[..], [Entry::HumanPrompt { .. }]));
        assert!(cls(&human(r#","turnOrigin":"sdk","promptSource":"sdk""#)).is_empty());
        assert!(cls(&human(r#","turnOrigin":"human","origin":{"kind":"coordinator"}"#)).is_empty());
        assert!(cls(&human("")).is_empty());
    }

    #[test]
    fn queued_flag_and_text_block_content() {
        let q = human(r#","origin":{"kind":"human"},"promptSource":"queued""#);
        assert!(matches!(&cls(&q)[..], [Entry::HumanPrompt { queued: true, .. }]));
        let t = human(r#","origin":{"kind":"human"},"promptSource":"typed""#);
        assert!(matches!(&cls(&t)[..], [Entry::HumanPrompt { queued: false, .. }]));
        let arr = line("user", r#","origin":{"kind":"human"}"#, r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#);
        assert!(matches!(&cls(&arr)[..], [Entry::HumanPrompt { text, .. }] if text == "ab"));
    }

    #[test]
    fn interrupt_is_recognised() {
        for t in ["[Request interrupted by user]", "[Request interrupted by user for tool use]"] {
            let l = line("user", "", &format!(r#"{{"content":[{{"type":"text","text":"{t}"}}]}}"#));
            assert!(matches!(&cls(&l)[..], [Entry::Interrupt { .. }]));
        }
        let with_result = line("user", r#","toolUseResult":{}"#, r#"{"content":[{"type":"text","text":"[Request interrupted by user]"}]}"#);
        assert!(cls(&with_result).is_empty());
    }

    #[test]
    fn only_user_permission_decisions_count() {
        let l = |pd: &str| {
            line(
                "user",
                &format!(r#","toolUseResult":{{}},"permissionDecision":{pd}"#),
                r#"{"content":[{"type":"tool_result","tool_use_id":"t1","content":"out","is_error":true}]}"#,
            )
        };
        let a = cls(&l(r#"{"decision":"accept","source":"user_temporary"}"#));
        assert!(matches!(&a[..], [Entry::ToolResult { tool_use_id, is_error: true, text, .. }, Entry::ToolDecision { decision: Decision::Approve, n: 0, .. }] if tool_use_id == "t1" && text == "out"));
        let d = cls(&l(r#"{"decision":"reject","source":"user_temporary"}"#));
        assert!(matches!(&d[..], [Entry::ToolResult { .. }, Entry::ToolDecision { decision: Decision::Deny, .. }]));
        assert!(matches!(&cls(&l(r#"{"decision":"accept","source":"config"}"#))[..], [Entry::ToolResult { .. }]));
        assert!(matches!(&cls(&l(r#"{"decision":"maybe","source":"user_temporary"}"#))[..], [Entry::ToolResult { .. }]));
    }

    #[test]
    fn assistant_items_and_turn_end() {
        let a = line(
            "assistant",
            "",
            r#"{"content":[{"type":"text","text":"one"},{"type":"thinking","thinking":"x"},{"type":"text","text":"two"},{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}},{"type":"tool_use","id":"t2","name":"Write","input":{"__unparsedToolInput":"x"}},{"type":"tool_use","id":"t3","name":"NotebookEdit","input":{"notebook_path":"n.ipynb"}}]}"#,
        );
        let e = cls(&a);
        assert!(matches!(&e[..], [
            Entry::AssistantText { n: 0, .. },
            Entry::AssistantText { n: 1, .. },
            Entry::ToolUse { command: Some(c), file_path: None, .. },
            Entry::ToolUse { command: None, file_path: None, .. },
            Entry::ToolUse { file_path: Some(p), .. },
        ] if c == "ls" && p == "n.ipynb"));
        let sys = |st: &str| format!(r#"{{"type":"system","subtype":"{st}","uuid":"u","sessionId":"s","timestamp":"2026-11-22T00:00:00Z"}}"#);
        assert!(matches!(&cls(&sys("turn_duration"))[..], [Entry::TurnEnd { .. }]));
        assert!(cls(&sys("stop_hook_summary")).is_empty());
        assert!(cls(r#"{"type":"attachment"}"#).is_empty());
    }

    #[test]
    fn huge_line_is_fine() {
        let text = "x".repeat(5_000_000);
        let line = format!(r#"{{"type":"user","uuid":"u","sessionId":"s","timestamp":"2026-11-22T00:00:00Z","origin":{{"kind":"human"}},"message":{{"role":"user","content":"{text}"}}}}"#);
        assert!(matches!(&classify(line.as_bytes()).unwrap()[..], [Entry::HumanPrompt { text, .. }] if text.len() == 5_000_000));
    }

    #[test]
    fn fixture_session_1_classifies_as_documented() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude_code/projects/__PROJECT__/aaaaaaaa-0000-4000-8000-000000000001.jsonl");
        let bytes = std::fs::read(p).unwrap();
        let mut pieces: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
        pieces.pop(); // unterminated (or empty) last piece
        let res: Vec<_> = pieces.iter().map(|l| classify(l)).collect();
        assert_eq!(res.len(), 20);
        assert_eq!(res[17].as_ref().err(), Some(&LineError::Json));
        assert_eq!(res[18].as_ref().err(), Some(&LineError::Utf8));
        assert_eq!(res.iter().filter(|r| r.is_err()).count(), 2);
        let prompts = |i: usize| matches!(&res[i].as_ref().unwrap()[..], [Entry::HumanPrompt { .. }]);
        assert!(prompts(2) && prompts(13) && prompts(19)); // P1, P2, P3
        assert!(res[10].as_ref().unwrap().is_empty()); // task-notification
        assert!(matches!(&res[15].as_ref().unwrap()[..], [Entry::Interrupt { .. }]));
        let n_prompts = res.iter().flatten().flatten().filter(|e| matches!(e, Entry::HumanPrompt { .. })).count();
        assert_eq!(n_prompts, 3);
        // sidechain file yields nothing
        let sp = p.replace(".jsonl", "/subagents/agent-a000000000000001.jsonl");
        let side = std::fs::read(sp).unwrap();
        for l in side.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            assert!(classify(l).unwrap().is_empty());
        }
    }
}
