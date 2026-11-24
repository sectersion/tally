//! Per-file Claude Code session state: entries in, events out. Text fields are memory only;
//! events carry numbers and hashes, never prompt, output or command text.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use tally_scoring::text::Vocab;
use tally_scoring::{Actor, Event, Kind};

use super::features::{prompt_numbers, ContextTail};
use super::line::{Entry, Meta};

pub const OPEN_SPAN_MAX_MS: i64 = 6 * 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashKind {
    Test,
    Commit,
}

/// Per-file state. Text fields are memory only; `open_turn` is persisted by the caller as "<uuid> <ts>".
#[derive(Default)]
pub struct Session {
    pub open_turn: Option<(String, i64)>, // (prompt uuid, prompt ts)
    pub last_seen_ts: i64,
    pending: BTreeMap<String, (Meta, String, BashKind)>, // Bash tool_use id -> (meta, cmd_hash, kind)
    tail: ContextTail,
    last_output: String,
    last_error: String,
    /// (tool_use ts, file_path) of agent Edit/Write/MultiEdit/NotebookEdit calls since the last drain. Memory only.
    /// Plan 08 drains it into `Source::take_agent_writes` for its heartbeat agent-write filter (README ruling 7);
    /// until then `ClaudeCode::poll` clears it after each poll. Shape `(ts, raw path)` is fixed here.
    pub agent_writes: Vec<(i64, String)>,
}

pub struct Ctx<'a> {
    pub project: Option<String>,
    pub vocab: &'a Vocab,
}

fn event(session: &str, uuid: &str, suffix: &str, ts: i64, project: &Option<String>, kind: Kind) -> Event {
    Event {
        v: 1,
        // ponytail: the session id is part of the id (Decision 1), so a forked/resumed session that copies
        // history under a new session id would double count entries newer than finalized_through; ids from
        // (uuid, kind) alone would dedup them. Maintainer to decide.
        id: format!("claude:{session}:{uuid}:{suffix}"),
        ts,
        source: "claude-code".into(),
        project: project.clone(),
        session: Some(session.to_owned()),
        kind,
    }
}

impl Session {
    /// Turn close rule (FORMAT.md deviation 1): the open turn closes at the first of an interrupt, a
    /// `turn_duration`, or the next non-queued human prompt. A `turn_duration` of a task-notification turn that
    /// arrives while a human turn is open closes that turn early; accepted (README D4).
    pub fn feed(&mut self, e: Entry, ctx: &Ctx, out: &mut Vec<Event>) {
        let prev_seen = self.last_seen_ts;
        let m = e.meta();
        self.last_seen_ts = self.last_seen_ts.max(m.ts);
        let ev = |uuid: &str, suffix: &str, ts: i64, kind: Kind| event(&m.session, uuid, suffix, ts, &ctx.project, kind);
        match &e {
            Entry::HumanPrompt { meta, text, queued } => {
                if !queued {
                    self.close(prev_seen, &meta.session, &ctx.project, out);
                }
                let (chars, relevance_pm, novelty_bytes) =
                    prompt_numbers(text, ctx.vocab, &self.last_output, &self.last_error, &self.tail);
                out.push(ev(&meta.uuid, "prompt", meta.ts, Kind::Prompt { chars, relevance_pm, novelty_bytes }));
                self.tail.push(text);
                self.open_turn.get_or_insert_with(|| (meta.uuid.clone(), meta.ts));
            }
            Entry::Interrupt { meta } => {
                out.push(ev(&meta.uuid, "interrupt", meta.ts, Kind::Interrupt));
                self.close(meta.ts, &meta.session, &ctx.project, out);
            }
            Entry::ToolDecision { meta, decision, n } => {
                out.push(ev(&meta.uuid, &format!("tool_decision:{n}"), meta.ts, Kind::ToolDecision { decision: *decision }));
            }
            Entry::AssistantText { meta, text, n } => {
                let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
                out.push(ev(&meta.uuid, &format!("agent_output:{n}"), meta.ts, Kind::AgentOutput { chars }));
                self.tail.push(text);
                self.last_output.clone_from(text);
            }
            Entry::ToolUse { meta, id, name, command, file_path } => match (name.as_str(), command, file_path) {
                // Same `command` key; on Windows Claude Code runs commands through PowerShell (FORMAT.md).
                ("Bash" | "PowerShell", Some(c), _) => {
                    if let Some(k) = bash_kind(c) {
                        self.pending.insert(id.clone(), (meta.clone(), cmd_hash(c), k));
                    }
                }
                ("Edit" | "Write" | "MultiEdit" | "NotebookEdit", _, Some(p)) => self.agent_writes.push((meta.ts, p.clone())),
                _ => {}
            },
            Entry::ToolResult { tool_use_id, is_error, text, .. } => {
                if let Some((tm, hash, k)) = self.pending.remove(tool_use_id) {
                    let kind = match k {
                        BashKind::Test => Some(("test_run", Kind::TestRun {
                            cmd_hash: hash,
                            exit_code: *is_error as i32,
                            initiator: Actor::Agent,
                        })),
                        // Marked even when the chain exits non-zero (`commit && push` failing): the commit may exist, and
                        // a false positive only makes a commit agent-owned (the safe direction).
                        BashKind::Commit => Some(("commit", Kind::Commit {
                            sha: format!("claude:{tool_use_id}"),
                            by: Actor::Agent,
                            lines_changed: 0,
                        })),
                    };
                    // Id has no block index: two Bash blocks in one entry would collide (FORMAT.md: one block per entry).
                    if let Some((suffix, kind)) = kind {
                        out.push(ev(&tm.uuid, suffix, tm.ts, kind));
                    }
                }
                if *is_error {
                    self.last_error.clone_from(text);
                }
            }
            Entry::TurnEnd { meta } => self.close(meta.ts, &meta.session, &ctx.project, out),
        }
    }

    /// Emits `agent_busy` for the open turn ending at `end_ts` (never before its start), if one is open.
    fn close(&mut self, end_ts: i64, session: &str, project: &Option<String>, out: &mut Vec<Event>) {
        if let Some((uuid, ts)) = self.open_turn.take() {
            out.push(event(session, &uuid, "agent_busy", ts, project, Kind::AgentBusy { end_ts: end_ts.max(ts) }));
        }
    }

    /// Closes a turn whose last entry is older than OPEN_SPAN_MAX_MS, at `last_seen_ts`, and drops (no event)
    /// pending Bash tool uses older than that which never got a result (Claude Code killed mid-tool).
    pub fn expire(&mut self, now_ms: i64, session: &str, project: Option<String>, out: &mut Vec<Event>) {
        if self.open_turn.is_some() && now_ms - self.last_seen_ts > OPEN_SPAN_MAX_MS {
            self.close(self.last_seen_ts, session, &project, out);
        }
        self.pending.retain(|_, p| p.0.ts >= now_ms - OPEN_SPAN_MAX_MS);
    }

    /// Highest watermark this session allows: min(open turn ts, oldest pending tool_use ts) - 1, or i64::MAX.
    pub fn limit(&self) -> i64 {
        let turn = self.open_turn.iter().map(|t| t.1);
        let pending = self.pending.values().map(|p| p.0.ts);
        turn.chain(pending).min().map_or(i64::MAX, |t| t - 1)
    }
}

const TEST_PREFIXES: [&str; 12] = [
    "cargo test", "cargo nextest", "npm test", "npm run test", "pnpm test", "yarn test", "bun test", "pytest",
    "python -m pytest", "go test", "dotnet test", "make test",
];

pub fn bash_kind(command: &str) -> Option<BashKind> {
    let c = command.trim();
    // Splitting on single `&`/`|` also covers `&&`/`||`; extra splits (e.g. `2>&1`) can only add false
    // positives, the safe direction. A `bash -c '<a && b>'` is split too, and each half is stripped below.
    // `{`/`}` split PowerShell 5.1 guards: `git add .; if ($?) { git commit -m x }`.
    if c.split(['&', '|', ';', '\n', '{', '}']).any(is_git_commit) {
        Some(BashKind::Commit)
    } else if TEST_PREFIXES.iter().any(|p| c.starts_with(p)) {
        Some(BashKind::Test)
    } else {
        None
    }
}

fn is_env_assign(tok: &str) -> bool {
    tok.split_once('=').is_some_and(|(k, _)| {
        k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Segment starts (after wrappers) with `git` and any later token is exactly `commit`. Matching any later token
/// survives `git -C "my dir" commit` and option arguments; `git log --grep commit` is a false positive (safe).
fn is_git_commit(seg: &str) -> bool {
    let toks: Vec<&str> = seg
        .split_whitespace()
        .map(|t| t.trim_start_matches(['(', '"', '\'']).trim_end_matches([')', '"', '\'']))
        .filter(|t| !t.is_empty())
        .collect();
    let mut t = &toks[..];
    loop {
        match t {
            [w, f, rest @ ..] if matches!(*w, "bash" | "sh" | "zsh") && f.starts_with('-') && f.contains('c') => t = rest,
            [w, rest @ ..] if matches!(*w, "env" | "sudo" | "time" | "command") || is_env_assign(w) => t = rest,
            // `pwsh [-flags [values]] -Command "..."`: skip to after `-c`/`-Command`; without one, skip leading flags.
            [w, rest @ ..] if matches!(w.to_ascii_lowercase().as_str(), "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe") => {
                let cmd = rest.iter().position(|f| f.eq_ignore_ascii_case("-c") || f.eq_ignore_ascii_case("-command"));
                t = match cmd {
                    Some(i) => &rest[i + 1..],
                    None => &rest[rest.iter().take_while(|f| f.starts_with('-')).count()..],
                };
            }
            _ => break,
        }
    }
    matches!(t, ["git", rest @ ..] if rest.contains(&"commit"))
}

/// Lowercase hex SHA-256 of the command string.
pub fn cmd_hash(command: &str) -> String {
    hex::encode(Sha256::digest(command.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(uuid: &str, ts: i64) -> Meta {
        Meta { uuid: uuid.into(), session: "s".into(), ts, cwd: None }
    }
    fn prompt(uuid: &str, ts: i64, text: &str) -> Entry {
        Entry::HumanPrompt { meta: meta(uuid, ts), text: text.into(), queued: false }
    }
    fn text(uuid: &str, ts: i64, t: &str) -> Entry {
        Entry::AssistantText { meta: meta(uuid, ts), text: t.into(), n: 0 }
    }
    fn tool(uuid: &str, ts: i64, id: &str, name: &str, command: Option<&str>, file_path: Option<&str>) -> Entry {
        Entry::ToolUse {
            meta: meta(uuid, ts),
            id: id.into(),
            name: name.into(),
            command: command.map(Into::into),
            file_path: file_path.map(Into::into),
        }
    }
    fn result(uuid: &str, ts: i64, id: &str, is_error: bool, t: &str) -> Entry {
        Entry::ToolResult { meta: meta(uuid, ts), tool_use_id: id.into(), is_error, text: t.into() }
    }
    fn busy(out: &[Event]) -> Vec<(String, i64, i64)> {
        out.iter()
            .filter_map(|e| match e.kind {
                Kind::AgentBusy { end_ts } => Some((e.id.clone(), e.ts, end_ts)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_turn_holds_the_limit_below_its_start_until_it_closes() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: Some("p".into()), vocab: &v };
        s.feed(prompt("u1", 1_000, "fix the parser"), &ctx, &mut out);
        assert_eq!(s.limit(), 999);
        s.feed(text("u2", 5_000, "done"), &ctx, &mut out);
        assert_eq!(s.limit(), 999);
        s.feed(Entry::TurnEnd { meta: meta("u3", 9_000) }, &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        let busy = out.iter().find(|e| matches!(e.kind, Kind::AgentBusy { .. })).unwrap();
        assert_eq!(
            (busy.id.as_str(), busy.ts, &busy.kind),
            ("claude:s:u1:agent_busy", 1_000, &Kind::AgentBusy { end_ts: 9_000 })
        );
        let p = out.iter().find(|e| matches!(e.kind, Kind::Prompt { .. })).unwrap();
        assert_eq!((p.id.as_str(), p.ts, p.source.as_str()), ("claude:s:u1:prompt", 1_000, "claude-code"));
        assert_eq!((p.project.as_deref(), p.session.as_deref()), (Some("p"), Some("s")));
        assert!(matches!(p.kind, Kind::Prompt { chars: 14, .. }));
        let o = out.iter().find(|e| matches!(e.kind, Kind::AgentOutput { .. })).unwrap();
        assert_eq!((o.id.as_str(), &o.kind), ("claude:s:u2:agent_output:0", &Kind::AgentOutput { chars: 4 }));
    }

    #[test]
    fn turn_end_without_a_human_turn_emits_nothing() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        s.feed(Entry::TurnEnd { meta: meta("u1", 1_000) }, &Ctx { project: None, vocab: &v }, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_stale_open_turn_expires_at_its_last_entry() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(prompt("u1", 0, "go"), &ctx, &mut out);
        s.feed(text("u2", 60_000, "ok"), &ctx, &mut out);
        out.clear();
        s.expire(60_000 + OPEN_SPAN_MAX_MS, "s", None, &mut out);
        assert!(out.is_empty());
        assert_eq!(s.limit(), -1);
        s.expire(60_000 + OPEN_SPAN_MAX_MS + 1, "s", None, &mut out);
        assert_eq!(busy(&out), [("claude:s:u1:agent_busy".to_owned(), 0, 60_000)]);
        assert_eq!(s.limit(), i64::MAX);
    }

    #[test]
    fn agent_test_run_and_commit_come_from_bash_results() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(tool("a1", 2_000, "t1", "Bash", Some("cargo test -p x"), None), &ctx, &mut out);
        assert_eq!(s.limit(), 1_999);
        s.feed(result("r1", 2_500, "t1", true, "failed"), &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        let t = out.last().unwrap();
        assert_eq!((t.id.as_str(), t.ts), ("claude:s:a1:test_run", 2_000));
        assert_eq!(
            t.kind,
            Kind::TestRun { cmd_hash: cmd_hash("cargo test -p x"), exit_code: 1, initiator: Actor::Agent }
        );
        s.feed(tool("a2", 3_000, "t2", "Bash", Some("git commit -m x"), None), &ctx, &mut out);
        assert_eq!(s.limit(), 2_999);
        s.feed(result("r2", 3_100, "t2", false, "ok"), &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        let c = out.last().unwrap();
        assert_eq!((c.id.as_str(), c.ts), ("claude:s:a2:commit", 3_000));
        assert_eq!(c.kind, Kind::Commit { sha: "claude:t2".into(), by: Actor::Agent, lines_changed: 0 });
        // a commit chain that exits non-zero (e.g. failing push) is still marked (GIT-1)
        let n = out.len();
        s.feed(tool("a3", 4_000, "t3", "Bash", Some("git commit -m y"), None), &ctx, &mut out);
        s.feed(result("r3", 4_100, "t3", true, "push failed"), &ctx, &mut out);
        assert_eq!(out.len(), n + 1);
        assert!(matches!(out.last().unwrap().kind, Kind::Commit { by: Actor::Agent, .. }));
        s.feed(tool("a4", 5_000, "t4", "Bash", Some("ls"), None), &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        assert!(!serde_json::to_string(&out).unwrap().contains("cargo"));
        assert_eq!(
            cmd_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_pending_tool_use_without_a_result_expires() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(tool("a1", 2_000, "t1", "Bash", Some("git commit -m x"), None), &ctx, &mut out);
        s.expire(2_000 + OPEN_SPAN_MAX_MS, "s", None, &mut out);
        assert_eq!(s.limit(), 1_999);
        s.expire(2_000 + OPEN_SPAN_MAX_MS + 1, "s", None, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        s.feed(result("r1", 2_000 + OPEN_SPAN_MAX_MS + 2, "t1", false, "ok"), &ctx, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn bash_kind_cases() {
        for c in [
            "(git commit)",
            "bash -c \"git commit\"",
            "cd x && (git commit -m x)",
            "git add . && git commit -m x",
            "git -C dir commit -m x",
            "GIT_AUTHOR_DATE=x git commit",
            "bash -c \"git commit -m x\"",
            "(git commit -m x)",
            "sh -c 'cd x && git commit -m y'",
            "  git -c user.name=a commit  ",
            "bash  -c \"git commit\"",
            "bash -lc 'git commit -m x'",
            "sh -ec \"git add . && git commit -m x\"",
            "env X=1 git commit",
            "sudo git commit",
            "git -C \"my dir\" commit -m x",
            "git --git-dir x commit",
        ] {
            assert_eq!(bash_kind(c), Some(BashKind::Commit), "{c}");
        }
        for c in ["echo \"git commit\"", "git status", "git log --oneline", "git commit-tree x", "ls", "cd x && cargo test"] {
            assert_eq!(bash_kind(c), None, "{c}");
        }
        for c in ["cargo test -p x", "  pytest -q", "python -m pytest", "npm run test", "go test ./..."] {
            assert_eq!(bash_kind(c), Some(BashKind::Test), "{c}");
        }
    }

    #[test]
    fn powershell_commands_classify_like_bash() {
        for c in [
            "git commit -m x",
            "git add . ; git commit -m x",
            "git add . && git commit -m x",
            "pwsh -Command \"git commit -m x\"",
            "powershell -Command \"git add .; git commit -m x\"",
            "powershell.exe -NoProfile -Command 'git commit -m x'",
            "pwsh -c \"git commit\"",
            "git add .; if ($?) { git commit -m x }",
            "git add . ; if ($LASTEXITCODE -eq 0) { git commit -m x }",
            "pwsh -ExecutionPolicy Bypass -Command \"git commit -m x\"",
            "powershell -NoProfile -Command \"git add . ; git commit -m x\"",
        ] {
            assert_eq!(bash_kind(c), Some(BashKind::Commit), "{c}");
        }
        for c in [
            "pwsh -Command \"git status\"",
            "powershell -NoProfile",
            "pwsh",
            "echo \"git commit\"",
            "git commit-tree x",
            "if ($?) { echo \"git commit\" }",
        ] {
            assert_eq!(bash_kind(c), None, "{c}");
        }
    }

    #[test]
    fn powershell_tool_uses_give_agent_test_runs_and_commits() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(tool("a1", 2_000, "t1", "PowerShell", Some("cargo test -p x"), None), &ctx, &mut out);
        assert_eq!(s.limit(), 1_999);
        s.feed(result("r1", 2_500, "t1", true, "failed"), &ctx, &mut out);
        let t = out.last().unwrap();
        assert_eq!(t.id, "claude:s:a1:test_run");
        assert_eq!(t.kind, Kind::TestRun { cmd_hash: cmd_hash("cargo test -p x"), exit_code: 1, initiator: Actor::Agent });
        s.feed(tool("a2", 3_000, "t2", "PowerShell", Some("git commit -m x"), None), &ctx, &mut out);
        assert_eq!(s.limit(), 2_999);
        s.feed(result("r2", 3_100, "t2", false, "ok"), &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        let c = out.last().unwrap();
        assert_eq!(c.id, "claude:s:a2:commit");
        assert_eq!(c.kind, Kind::Commit { sha: "claude:t2".into(), by: Actor::Agent, lines_changed: 0 });
    }

    #[test]
    fn agent_file_edits_are_recorded_for_plan_08() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(tool("a1", 4_000, "t1", "Edit", None, Some("src/a.rs")), &ctx, &mut out);
        s.feed(tool("a2", 4_100, "t2", "Read", None, Some("src/b.rs")), &ctx, &mut out);
        assert_eq!(s.agent_writes, [(4_000, "src/a.rs".to_owned())]);
        assert!(out.is_empty());
    }

    #[test]
    fn prompt_features_use_the_last_error() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(result("r1", 1_000, "none", true, "error: zebra_word failed"), &ctx, &mut out);
        // Long enough to pass NOVELTY_MIN (a shorter prompt gets relevance 0 regardless of vocab).
        s.feed(prompt("u1", 2_000, "why does zebra_word fail when the loader runs here"), &ctx, &mut out);
        assert!(matches!(out.last().unwrap().kind, Kind::Prompt { relevance_pm, .. } if relevance_pm > 0));
    }

    #[test]
    fn interrupt_closes_the_turn_and_a_later_turn_end_adds_nothing() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(prompt("u1", 1_000, "go"), &ctx, &mut out);
        s.feed(Entry::Interrupt { meta: meta("u2", 4_000) }, &ctx, &mut out);
        assert_eq!(s.limit(), i64::MAX);
        s.feed(Entry::TurnEnd { meta: meta("u3", 5_000) }, &ctx, &mut out);
        assert_eq!(busy(&out), [("claude:s:u1:agent_busy".to_owned(), 1_000, 4_000)]);
        assert!(out.iter().any(|e| e.id == "claude:s:u2:interrupt" && e.kind == Kind::Interrupt));
    }

    #[test]
    fn a_typed_second_prompt_closes_the_first_turn_at_the_last_seen_ts() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(prompt("u1", 1_000, "go"), &ctx, &mut out);
        s.feed(text("u2", 3_000, "working"), &ctx, &mut out);
        s.feed(prompt("u3", 8_000, "next"), &ctx, &mut out);
        assert_eq!(busy(&out), [("claude:s:u1:agent_busy".to_owned(), 1_000, 3_000)]);
        assert_eq!(s.open_turn, Some(("u3".to_owned(), 8_000)));
        assert_eq!(s.limit(), 7_999);
    }

    #[test]
    fn a_queued_second_prompt_joins_the_open_turn() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        s.feed(prompt("u1", 1_000, "go"), &ctx, &mut out);
        s.feed(Entry::HumanPrompt { meta: meta("u2", 2_000), text: "also".into(), queued: true }, &ctx, &mut out);
        assert!(busy(&out).is_empty());
        assert_eq!(s.open_turn, Some(("u1".to_owned(), 1_000)));
        assert_eq!(out.iter().filter(|e| matches!(e.kind, Kind::Prompt { .. })).count(), 2);
    }

    #[test]
    fn fixture_shaped_interrupt_then_turn_end_gives_one_busy_span() {
        let (mut s, v) = (Session::default(), Vocab::default());
        let mut out = vec![];
        let ctx = Ctx { project: None, vocab: &v };
        for e in [
            prompt("p2", 180_000, "widget"),
            text("a", 185_000, "looking"),
            Entry::Interrupt { meta: meta("i", 195_000) },
            Entry::TurnEnd { meta: meta("d", 201_000) },
        ] {
            s.feed(e, &ctx, &mut out);
        }
        assert_eq!(busy(&out), [("claude:s:p2:agent_busy".to_owned(), 180_000, 195_000)]);
    }

    #[test]
    fn ids_are_stable_across_two_identical_feeds() {
        let entries = || {
            vec![
                prompt("u1", 1_000, "fix the parser"),
                text("u2", 2_000, "ok"),
                tool("u3", 3_000, "t1", "Bash", Some("cargo test"), None),
                result("u4", 4_000, "t1", false, "passed"),
                Entry::ToolDecision { meta: meta("u5", 4_500), decision: tally_scoring::Decision::Approve, n: 0 },
                Entry::TurnEnd { meta: meta("u6", 5_000) },
            ]
        };
        let v = Vocab::default();
        let run = || {
            let (mut s, mut out) = (Session::default(), vec![]);
            for e in entries() {
                s.feed(e, &Ctx { project: Some("p".into()), vocab: &v }, &mut out);
            }
            out
        };
        let a = run();
        assert_eq!(a.len(), 5);
        assert!(a.iter().any(|e| e.id == "claude:s:u5:tool_decision:0"));
        assert_eq!(a, run());
    }
}
