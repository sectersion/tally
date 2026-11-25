//! Session text for a submission, re-read from the harness's own logs (the store keeps no text).
//! Text lives in memory only, for the sessions the given prompt events name; tool output is reduced to stubs.
use crate::claude_code::line::{classify, Entry};
use std::collections::HashMap;
use std::path::Path;
use tally_scoring::Event;

pub const REPLY_MAX_CHARS: usize = 4000;
pub const PROMPT_MAX_CHARS: usize = 20_000;
const HEAD: usize = 3000;
const TAIL: usize = 1000;
const CMD_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub action_id: String,
    pub ts: i64,
    pub session: String,
    pub prompt: Option<String>,
    pub reply: Option<String>,
    /// "log_missing" | "no_text_reader"
    pub missing: Option<&'static str>,
}

pub fn read_turns(logs_dir: &Path, prompts: &[Event]) -> Result<Vec<Turn>, std::io::Error> {
    // session -> its classified entries; None when the file is gone. Only sessions named by `prompts` are opened.
    let mut logs: HashMap<&str, Option<Vec<Entry>>> = HashMap::new();
    let mut turns = Vec::with_capacity(prompts.len());
    for ev in prompts {
        let session = ev.session.clone().unwrap_or_default();
        let mut turn = Turn { action_id: ev.id.clone(), ts: ev.ts, session, prompt: None, reply: None, missing: None };
        if ev.source != "claude-code" {
            turn.missing = Some("no_text_reader");
        } else {
            let key = ev.session.as_deref().unwrap_or("");
            if !logs.contains_key(key) {
                logs.insert(key, load(logs_dir, key)?);
            }
            match logs[key].as_deref().and_then(|e| find_turn(e, &ev.id)) {
                Some((prompt, rest)) => {
                    turn.prompt = Some(cap_prompt(prompt));
                    turn.reply = Some(trim_reply(rest));
                }
                None => turn.missing = Some("log_missing"),
            }
        }
        turns.push(turn);
    }
    Ok(turns)
}

/// `<logs_dir>/*/<session>.jsonl`, classified line by line; unreadable lines are skipped. None if no such file.
fn load(logs_dir: &Path, session: &str) -> Result<Option<Vec<Entry>>, std::io::Error> {
    if session.is_empty() || session.contains(['/', '\\']) || session == ".." || session == "." {
        return Ok(None);
    }
    let dirs = match std::fs::read_dir(logs_dir) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    for d in dirs {
        let f = d?.path().join(format!("{session}.jsonl"));
        match std::fs::read(&f) {
            Ok(bytes) => {
                let lines = bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty());
                return Ok(Some(lines.filter_map(|l| classify(l).ok()).flatten().collect()));
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// The prompt text of the `HumanPrompt` with this event id, and the entries after it up to the next `HumanPrompt`.
fn find_turn<'a>(entries: &'a [Entry], action_id: &str) -> Option<(&'a str, &'a [Entry])> {
    let at = entries.iter().position(|e| {
        matches!(e, Entry::HumanPrompt { meta, .. } if format!("claude:{}:{}:prompt", meta.session, meta.uuid) == action_id)
    })?;
    let Entry::HumanPrompt { text, .. } = &entries[at] else { return None };
    let rest = &entries[at + 1..];
    let end = rest.iter().position(|e| matches!(e, Entry::HumanPrompt { .. })).unwrap_or(rest.len());
    Some((text, &rest[..end]))
}

fn cap_prompt(p: &str) -> String {
    match p.char_indices().nth(PROMPT_MAX_CHARS) {
        None => p.to_owned(),
        Some((i, _)) => format!("{}\n[... prompt capped at {PROMPT_MAX_CHARS} chars ...]", &p[..i]),
    }
}

pub fn trim_reply(entries: &[Entry]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::AssistantText { text, .. } => Some(text.clone()),
            Entry::ToolUse { name, command, .. } => Some(match (name.as_str(), command) {
                ("Bash" | "PowerShell", Some(c)) => {
                    format!("[tool: {name}] {}", c.chars().take(CMD_MAX_CHARS).collect::<String>())
                }
                _ => format!("[tool: {name}]"),
            }),
            Entry::ToolResult { is_error, text, .. } => Some(format!("[result: {} chars, error: {is_error}]", text.chars().count())),
            _ => None,
        })
        .collect();
    let all = parts.join("\n");
    let n = all.chars().count();
    if n <= REPLY_MAX_CHARS {
        return all;
    }
    let head: String = all.chars().take(HEAD).collect();
    let tail: String = all.chars().skip(n - TAIL).collect();
    format!("{head}[... {} chars trimmed ...]{tail}", n - HEAD - TAIL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_code::line::Meta;
    use crate::claude_code::session::{Ctx, Session};
    use tally_scoring::{text::Vocab, Kind};

    const SESSION: &str = "aaaaaaaa-0000-4000-8000-000000000010";
    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude_code/submit-session.jsonl");

    /// A logs dir holding the fixture, and the prompt events plan 03's adapter derives from it.
    fn setup() -> (tempfile::TempDir, Vec<Event>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("proj")).unwrap();
        let bytes = std::fs::read(FIXTURE).unwrap();
        std::fs::write(dir.path().join("proj").join(format!("{SESSION}.jsonl")), &bytes).unwrap();
        let (mut s, v, mut out) = (Session::default(), Vocab::default(), vec![]);
        let ctx = Ctx { project: None, vocab: &v };
        for l in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            for e in classify(l).unwrap() {
                s.feed(e, &ctx, &mut out);
            }
        }
        out.retain(|e| matches!(e.kind, Kind::Prompt { .. }));
        (dir, out)
    }

    fn meta() -> Meta {
        Meta { uuid: "u".into(), session: "s".into(), ts: 0, cwd: None }
    }
    fn text(t: &str) -> Entry {
        Entry::AssistantText { meta: meta(), text: t.into(), n: 0 }
    }

    #[test]
    fn turns_match_prompt_event_ids() {
        let (dir, events) = setup();
        assert_eq!(events.len(), 3);
        let turns = read_turns(dir.path(), &events).unwrap();
        let ids: Vec<_> = turns.iter().map(|t| t.action_id.as_str()).collect();
        assert_eq!(ids, events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>());
        assert!(turns.iter().all(|t| t.missing.is_none() && t.session == SESSION));
        assert!(turns[0].prompt.as_deref().unwrap().starts_with("Please summarise"));
        assert!(turns[0].reply.as_deref().unwrap().contains("falls back to defaults"));
        assert!(!turns[0].reply.as_deref().unwrap().contains("private reasoning"));
        assert!(turns[2].prompt.as_deref().unwrap().starts_with("Thanks"));
        assert!(!turns[1].reply.as_deref().unwrap().contains("falls back to defaults"));
    }

    #[test]
    fn tool_output_is_stubbed() {
        let (dir, events) = setup();
        let turns = read_turns(dir.path(), &events).unwrap();
        let (r0, r1) = (turns[0].reply.as_deref().unwrap(), turns[1].reply.as_deref().unwrap());
        assert!(r0.contains("[result: 6750 chars, error: false]"), "{r0}");
        assert!(!r0.contains("Invented long output"));
        assert!(r0.contains("[tool: Read]") && r0.contains("[tool: Bash] echo invented-argument"));
        assert!(r1.contains("[result: 43 chars, error: true]") && !r1.contains("check three did not pass"), "{r1}");
        assert!(r1.contains("[tool: Bash] sample-check --all") && r1.contains("[tool: Edit]") && !r1.contains("/sample/values.txt"));
    }

    #[test]
    fn reply_is_capped_head_and_tail() {
        let long: String = (0..6000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let r = trim_reply(&[text(&long)]);
        assert!(r.starts_with(&long[..3000]) && r.ends_with(&long[5000..]));
        assert!(r.contains("[... 2000 chars trimmed ...]"));
        let short = "x".repeat(4000);
        assert_eq!(trim_reply(&[text(&short)]), short);
        let cmd = Entry::ToolUse { meta: meta(), id: "t".into(), name: "Bash".into(), command: Some("y".repeat(500)), file_path: None };
        assert_eq!(trim_reply(&[cmd]), format!("[tool: Bash] {}", "y".repeat(200)));
    }

    #[test]
    fn long_prompt_is_capped_and_noted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("p")).unwrap();
        let line = format!(
            r#"{{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-11-22T00:00:00.000Z","origin":{{"kind":"human"}},"message":{{"role":"user","content":"{}"}}}}"#,
            "w".repeat(25_000)
        );
        std::fs::write(dir.path().join("p/s1.jsonl"), line + "\n").unwrap();
        let ev = Event { v: 1, id: "claude:s1:u1:prompt".into(), ts: 0, source: "claude-code".into(), project: None, session: Some("s1".into()), kind: Kind::Prompt { chars: 0, relevance_pm: 0, novelty_bytes: 0 } };
        let p = read_turns(dir.path(), &[ev]).unwrap().remove(0).prompt.unwrap();
        assert!(p.starts_with(&"w".repeat(20_000)) && p.contains("capped at 20000 chars") && p.len() < 20_100);
    }

    #[test]
    fn missing_log_is_marked_not_fatal() {
        let (dir, mut events) = setup();
        events[1].session = Some("gone".into());
        events[1].id = "claude:gone:u:prompt".into();
        let turns = read_turns(dir.path(), &events).unwrap();
        assert_eq!(turns[1].missing, Some("log_missing"));
        assert!(turns[1].prompt.is_none() && turns[1].reply.is_none());
        assert_eq!(turns[0].missing, None);
        // record gone from an existing file
        events[2].id = format!("claude:{SESSION}:nope:prompt");
        assert_eq!(read_turns(dir.path(), &events).unwrap()[2].missing, Some("log_missing"));
        // logs dir gone entirely
        assert_eq!(read_turns(&dir.path().join("none"), &events).unwrap()[0].missing, Some("log_missing"));
    }

    #[test]
    fn other_sources_have_no_text() {
        let (dir, mut events) = setup();
        events[0].source = "codex".into();
        let t = &read_turns(dir.path(), &events).unwrap()[0];
        assert_eq!(t.missing, Some("no_text_reader"));
        assert!(t.prompt.is_none() && t.reply.is_none());
    }
}
