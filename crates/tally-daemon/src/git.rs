//! Git commit source: polls local repositories for the user's own commits.
//! Events carry only the sha, who made the commit and a line count.
use crate::source::{Poll, Source, SourceError, SourceState};
use crate::store::Store;
use std::path::PathBuf;
use std::process::Command;
use tally_scoring::event::Actor;
use tally_scoring::{Event, Kind};

pub const AGENT_COMMIT_WINDOW_MS: i64 = 120_000;
pub const AGENT_COMMIT_AFTER_MS: i64 = 5_000;
/// Fallback lag, used only when the claude-code watermark is unavailable.
pub const GIT_LAG_MS: i64 = 60_000;

pub struct RepoRef {
    pub local_id: String,
    pub root: PathBuf,
}

pub trait AgentCommitLookup {
    fn is_agent_commit(&self, repo: &RepoRef, commit_ts: i64) -> Result<bool, SourceError>;
    fn claude_watermark(&self) -> Result<Option<i64>, SourceError>;
}

/// Reads the store through its own connection; never writes.
pub struct StoreLookup {
    pub store: Store,
    pub claude_enabled: bool,
}

impl AgentCommitLookup for StoreLookup {
    fn is_agent_commit(&self, repo: &RepoRef, commit_ts: i64) -> Result<bool, SourceError> {
        // Markers carry the id of the directory Claude ran in: a linked worktree of this repo,
        // registered or not.
        let err = |e: crate::store::StoreError| SourceError::Other(e.to_string());
        let mut ids = self.store.project_group(&repo.local_id).map_err(err)?;
        ids.extend(crate::project::worktree_ids(&repo.root));
        // An agent can commit into a sibling registered repo (`git -C ../b commit`): the marker then sits
        // under the project Claude ran in, so any registered project's marker in the window counts.
        ids.extend(self.store.projects().map_err(err)?.into_iter().map(|p| p.local_id));
        for p in ids {
            if self.store.has_agent_commit_marker(&p, commit_ts - AGENT_COMMIT_WINDOW_MS, commit_ts + AGENT_COMMIT_AFTER_MS).map_err(err)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn claude_watermark(&self) -> Result<Option<i64>, SourceError> {
        if !self.claude_enabled {
            return Ok(None);
        }
        self.store.watermark("claude-code").map_err(|e| SourceError::Other(e.to_string()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawCommit {
    pub sha: String,
    pub ts_ms: i64,
    pub lines: u32,
}

/// Parses `--format=@%H %ct %ce --numstat` output, keeping commits by `email` (case-insensitive).
pub fn parse_log(out: &str, email: &str) -> Vec<RawCommit> {
    let mut res: Vec<RawCommit> = Vec::new();
    let mut keep = false;
    for line in out.lines() {
        if let Some(h) = line.strip_prefix('@') {
            let mut p = h.split(' ');
            let (sha, ts, ce) = (p.next(), p.next().and_then(|t| t.parse::<i64>().ok()), p.next());
            keep = false;
            if let (Some(sha), Some(ts), Some(ce)) = (sha, ts, ce) {
                if ce.eq_ignore_ascii_case(email) {
                    keep = true;
                    res.push(RawCommit { sha: sha.into(), ts_ms: ts * 1000, lines: 0 });
                }
            }
        } else if keep {
            let mut p = line.split('\t');
            let n = |s: &str| s.parse::<u32>().unwrap_or(0);
            if let (Some(a), Some(d), Some(c)) = (p.next(), p.next(), res.last_mut()) {
                c.lines = c.lines.saturating_add(n(a)).saturating_add(n(d));
            }
        }
    }
    res
}

pub struct GitSource {
    repos: Vec<RepoRef>,
    lookup: Box<dyn AgentCommitLookup>,
    pub(crate) program: String,
    /// Repos whose missing root was already logged.
    warned: std::collections::HashSet<String>,
}

impl GitSource {
    pub fn new(repos: Vec<RepoRef>, lookup: Box<dyn AgentCommitLookup>) -> GitSource {
        GitSource { repos, lookup, program: "git".into(), warned: Default::default() }
    }
}

/// How long a failing (but existing) repo may hold the git watermark back before it is skipped.
const FAILING_REPO_GRACE_MS: i64 = 24 * 3_600_000;

pub(crate) enum GitFail {
    Missing,
    Failed,
}

pub(crate) fn run_git(program: &str, root: &std::path::Path, args: &[&str]) -> Result<String, GitFail> {
    let out = Command::new(program)
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { GitFail::Missing } else { GitFail::Failed })?;
    if !out.status.success() {
        return Err(GitFail::Failed);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `Ok(None)`: no committer email configured, so none of the commits can be the user's.
fn fetch(program: &str, repo: &RepoRef, wm: i64) -> Result<Option<Vec<RawCommit>>, GitFail> {
    let email = match run_git(program, &repo.root, &["config", "user.email"]) {
        Ok(e) => e.trim().to_string(),
        Err(GitFail::Missing) => return Err(GitFail::Missing),
        // exit 1 means the key is unset; a broken or missing repo fails here too
        Err(GitFail::Failed) => {
            run_git(program, &repo.root, &["rev-parse", "--git-dir"])?;
            String::new()
        }
    };
    if email.is_empty() {
        return Ok(None);
    }
    let since = format!("--since={}", (wm / 1000 - 1).max(0));
    let out = run_git(
        program,
        &repo.root,
        &["log", "--branches", "--no-merges", &since, "--format=@%H %ct %ce", "--numstat"],
    )?;
    Ok(Some(parse_log(&out, &email)))
}

impl Source for GitSource {
    fn id(&self) -> &str {
        "git"
    }

    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError> {
        // A commit at T is emitted only once claude-code's watermark is >= T + 5 s, so every agent
        // marker that could match it is already stored.
        let cutoff = match self.lookup.claude_watermark()? {
            Some(cw) => now_ms.min(cw - AGENT_COMMIT_AFTER_MS),
            None => now_ms - GIT_LAG_MS,
        };
        let mut events = Vec::new();
        let mut min_wm: Option<i64> = None;
        for repo in &self.repos {
            let key = format!("{}:wm", repo.local_id);
            let new_wm = match state.get(&key).and_then(|v| v.parse::<i64>().ok()) {
                None => cutoff, // first sight: live capture only
                Some(wm) => match fetch(&self.program, repo, wm) {
                    Ok(commits) => {
                        for c in commits.unwrap_or_default().into_iter().filter(|c| c.ts_ms > wm && c.ts_ms <= cutoff) {
                            let by = if self.lookup.is_agent_commit(repo, c.ts_ms)? { Actor::Agent } else { Actor::User };
                            events.push(Event {
                                v: 1,
                                id: format!("git:{}:commit", c.sha),
                                ts: c.ts_ms,
                                source: "git".into(),
                                project: Some(repo.local_id.clone()),
                                session: None,
                                kind: Kind::Commit { sha: c.sha, by, lines_changed: c.lines },
                            });
                        }
                        wm.max(cutoff)
                    }
                    Err(GitFail::Missing) => return Err(SourceError::Other("git not found on PATH".into())),
                    // A deleted root has no new commits: do not let it freeze every source's finalization.
                    Err(GitFail::Failed) if !repo.root.exists() => {
                        if self.warned.insert(repo.local_id.clone()) {
                            eprintln!("git: repository root {} no longer exists; not polling it", repo.root.display());
                        }
                        wm.max(cutoff)
                    }
                    // Retried next poll, holding the source watermark back, but only for a day: a repo that
                    // stays broken (corrupt HEAD, dubious ownership) must not cap finalization forever.
                    Err(GitFail::Failed) if cutoff - wm <= FAILING_REPO_GRACE_MS => wm,
                    Err(GitFail::Failed) => {
                        if self.warned.insert(repo.local_id.clone()) {
                            eprintln!("git: repository {} keeps failing; skipping commits until it works again", repo.root.display());
                        }
                        wm.max(cutoff)
                    }
                },
            };
            state.set(&key, &new_wm.to_string());
            min_wm = Some(min_wm.map_or(new_wm, |m| m.min(new_wm)));
        }
        events.sort_by_key(|e| e.ts); // git log is newest first
        Ok(Poll { events, watermark: min_wm })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn parse_log_sums_numstat_counts_binary_as_zero_and_filters_the_committer() {
        let out = "@aaa 1700000010 me@x.org\n3\t1\tsrc/a.rs\n-\t-\timg.png\n\n@bbb 1700000020 other@x.org\n9\t9\tz\n";
        assert_eq!(
            parse_log(out, "ME@x.org"),
            vec![RawCommit { sha: "aaa".into(), ts_ms: 1_700_000_010_000, lines: 4 }]
        );
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    fn git(dir: &std::path::Path, secs: i64, extra_env: &[(&str, &str)], args: &[&str]) {
        let d = format!("{secs} +0000");
        let mut c = Command::new("git");
        c.arg("-C").arg(dir).args(args);
        c.env("GIT_COMMITTER_DATE", &d).env("GIT_AUTHOR_DATE", &d);
        c.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", dir.join("no-global-config"));
        for (k, v) in extra_env {
            c.env(k, v);
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn init(dir: &std::path::Path) {
        git(dir, 1_600_000_000, &[], &["init", "-q", "-b", "main"]);
        git(dir, 1_600_000_000, &[], &["config", "user.email", "me@x.org"]);
        git(dir, 1_600_000_000, &[], &["config", "user.name", "Me"]);
        git(dir, 1_600_000_000, &[], &["commit", "-q", "--allow-empty", "-m", "init"]);
    }

    fn commit(dir: &std::path::Path, secs: i64) {
        git(dir, secs, &[], &["commit", "-q", "--allow-empty", "-m", "c"]);
    }

    fn head(dir: &std::path::Path) -> String {
        let o = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    }

    #[derive(Default)]
    struct Fake {
        agent: Vec<(String, i64)>,
        claude_wm: Option<i64>,
        fail: bool,
    }
    #[derive(Clone, Default)]
    struct FakeLookup(Arc<Mutex<Fake>>);
    impl AgentCommitLookup for FakeLookup {
        fn is_agent_commit(&self, repo: &RepoRef, ts: i64) -> Result<bool, SourceError> {
            let project = repo.local_id.as_str();
            let f = self.0.lock().unwrap();
            if f.fail {
                return Err(SourceError::Other("lookup down".into()));
            }
            Ok(f.agent.iter().any(|(p, t)| p == project && *t >= ts - AGENT_COMMIT_WINDOW_MS && *t <= ts + AGENT_COMMIT_AFTER_MS))
        }
        fn claude_watermark(&self) -> Result<Option<i64>, SourceError> {
            let f = self.0.lock().unwrap();
            if f.fail {
                return Err(SourceError::Other("lookup down".into()));
            }
            Ok(f.claude_wm)
        }
    }

    const T: i64 = 1_700_000_000_000;

    fn setup() -> (tempfile::TempDir, GitSource, FakeLookup, BTreeMap<String, String>) {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        let lk = FakeLookup::default();
        let src = GitSource::new(vec![RepoRef { local_id: "r1".into(), root: dir.path().into() }], Box::new(lk.clone()));
        (dir, src, lk, BTreeMap::new())
    }

    #[test]
    fn first_poll_emits_nothing_then_a_new_commit_is_emitted_once_it_is_older_than_the_lag() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        let p = src.poll(T - 100_000, &mut st).unwrap();
        assert!(p.events.is_empty());
        assert_eq!(p.watermark, Some(T - 160_000));
        commit(dir.path(), T / 1000);
        let p = src.poll(T + 61_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        let e = &p.events[0];
        assert_eq!(e.id, format!("git:{}:commit", head(dir.path())));
        assert_eq!((e.ts, e.project.as_deref(), e.source.as_str()), (T, Some("r1"), "git"));
        assert!(matches!(e.kind, Kind::Commit { by: Actor::User, lines_changed: 0, .. }));
        assert!(src.poll(T + 120_000, &mut st).unwrap().events.is_empty());
    }

    #[test]
    fn a_commit_inside_the_lag_window_waits_and_the_watermark_is_now_minus_lag() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        src.poll(T - 100_000, &mut st).unwrap();
        commit(dir.path(), T / 1000);
        let p = src.poll(T + 30_000, &mut st).unwrap();
        assert!(p.events.is_empty());
        assert_eq!(p.watermark, Some(T + 30_000 - GIT_LAG_MS));
        assert_eq!(src.poll(T + 61_000, &mut st).unwrap().events.len(), 1);
    }

    #[test]
    fn commit_is_held_until_the_claude_watermark_passes_it_by_five_seconds() {
        if !git_available() { return; }
        let (dir, mut src, lk, mut st) = setup();
        lk.0.lock().unwrap().claude_wm = Some(T - 200_000);
        src.poll(T - 190_000, &mut st).unwrap();
        commit(dir.path(), T / 1000);
        lk.0.lock().unwrap().claude_wm = Some(T + 4_999);
        let p = src.poll(T + 10_000, &mut st).unwrap();
        assert!(p.events.is_empty());
        assert!(p.watermark.unwrap() <= T + 4_999 - 5_000);
        lk.0.lock().unwrap().claude_wm = Some(T + 5_000);
        let p = src.poll(T + 10_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1); // now - T is 10 s, far below GIT_LAG_MS: the lag is not used
    }

    #[test]
    fn late_marker_within_the_window_still_makes_the_commit_by_agent() {
        if !git_available() { return; }
        let (dir, mut src, lk, mut st) = setup();
        lk.0.lock().unwrap().claude_wm = Some(T - 200_000);
        src.poll(T - 190_000, &mut st).unwrap();
        commit(dir.path(), T / 1000);
        lk.0.lock().unwrap().claude_wm = Some(T - 1_000);
        assert!(src.poll(T + 10_000, &mut st).unwrap().events.is_empty());
        {
            let mut f = lk.0.lock().unwrap();
            f.agent.push(("r1".into(), T + 3_000));
            f.claude_wm = Some(T + 5_000);
        }
        let p = src.poll(T + 10_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert!(matches!(p.events[0].kind, Kind::Commit { by: Actor::Agent, .. }));
    }

    #[test]
    fn agent_marker_makes_the_commit_by_agent_and_no_marker_means_user() {
        if !git_available() { return; }
        let (dir, mut src, lk, mut st) = setup();
        src.poll(T - 100_000, &mut st).unwrap();
        commit(dir.path(), T / 1000);
        commit(dir.path(), T / 1000 + 600);
        lk.0.lock().unwrap().agent.push(("r1".into(), T - 10_000));
        let p = src.poll(T + 700_000, &mut st).unwrap();
        let by: Vec<Actor> = p
            .events
            .iter()
            .map(|e| match &e.kind {
                Kind::Commit { by, .. } => *by,
                _ => panic!(),
            })
            .collect();
        assert_eq!(by, vec![Actor::Agent, Actor::User]);
    }

    fn rr(id: &str) -> RepoRef {
        RepoRef { local_id: id.into(), root: "/no/such/dir".into() }
    }

    #[test]
    fn store_lookup_matches_markers_by_agent_and_ignores_the_sha() {
        let mut store = Store::open_in_memory().unwrap();
        let mk = |id: &str, sha: &str, by| Event {
            v: 1,
            id: id.into(),
            ts: T,
            source: "claude-code".into(),
            project: Some("r1".into()),
            session: None,
            kind: Kind::Commit { sha: sha.into(), by, lines_changed: 0 },
        };
        store.append_events(&[mk("a", "claude:toolu_1", Actor::Agent)]).unwrap();
        let on = StoreLookup { store, claude_enabled: true };
        assert!(on.is_agent_commit(&rr("r1"), T + 100_000).unwrap());
        assert!(!on.is_agent_commit(&rr("r1"), T + 130_000).unwrap());
        assert!(!on.is_agent_commit(&rr("r2"), T).unwrap());
        assert_eq!(on.claude_watermark().unwrap(), None);
        let mut s2 = Store::open_in_memory().unwrap();
        s2.append_events(&[mk("b", "deadbeef", Actor::User)]).unwrap();
        s2.set_watermark("claude-code", 42).unwrap();
        let l2 = StoreLookup { store: s2, claude_enabled: true };
        assert!(!l2.is_agent_commit(&rr("r1"), T).unwrap());
        assert_eq!(l2.claude_watermark().unwrap(), Some(42));
        let l3 = StoreLookup { store: Store::open_in_memory().unwrap(), claude_enabled: false };
        assert_eq!(l3.claude_watermark().unwrap(), None);
    }

    #[test]
    fn agent_marker_stored_under_a_linked_worktree_marks_a_commit_polled_for_the_group_representative() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt) = (t.path().join("main"), t.path().join("wt"));
        std::fs::create_dir_all(&main).unwrap();
        init(&main);
        git(&main, T / 1000, &[], &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feature"]);
        let db = t.path().join("tally.db");
        let mut store = Store::open(&db).unwrap();
        let m = crate::project::register_git_project(&mut store, &main, None).unwrap();
        let w = crate::project::register_git_project(&mut store, &wt, None).unwrap();
        // plan 03 stamps the marker with the id of the directory Claude ran in: the worktree
        let marker = Event {
            v: 1,
            id: "mk".into(),
            ts: T - 10_000,
            source: "claude-code".into(),
            project: Some(w),
            session: None,
            kind: Kind::Commit { sha: "claude:toolu_1".into(), by: Actor::Agent, lines_changed: 0 },
        };
        store.append_events(&[marker]).unwrap();
        let repos: Vec<RepoRef> =
            store.repos_to_poll().unwrap().into_iter().map(|(local_id, root)| RepoRef { local_id, root: root.into() }).collect();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].local_id, m);
        let mut src = GitSource::new(repos, Box::new(StoreLookup { store: Store::open(&db).unwrap(), claude_enabled: false }));
        let mut st = BTreeMap::new();
        src.poll(T - 100_000, &mut st).unwrap();
        commit(&wt, T / 1000);
        let p = src.poll(T + 61_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].project.as_deref(), Some(m.as_str()));
        assert!(matches!(p.events[0].kind, Kind::Commit { by: Actor::Agent, .. }));
    }

    #[test]
    fn agent_marker_from_an_unregistered_worktree_still_marks_the_commit() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt) = (t.path().join("main"), t.path().join("wt"));
        std::fs::create_dir_all(&main).unwrap();
        init(&main);
        git(&main, T / 1000, &[], &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feature"]);
        let db = t.path().join("tally.db");
        let mut store = Store::open(&db).unwrap();
        let m = crate::project::register_git_project(&mut store, &main, None).unwrap(); // wt is NOT registered
        let w = crate::project::project_for_dir(&wt).unwrap().1;
        assert_ne!(m, w);
        let marker = Event {
            v: 1, id: "mk".into(), ts: T - 10_000, source: "claude-code".into(), project: Some(w), session: None,
            kind: Kind::Commit { sha: "claude:toolu_1".into(), by: Actor::Agent, lines_changed: 0 },
        };
        store.append_events(&[marker]).unwrap();
        let repos = vec![RepoRef { local_id: m, root: main }];
        let mut src = GitSource::new(repos, Box::new(StoreLookup { store: Store::open(&db).unwrap(), claude_enabled: false }));
        let mut st = BTreeMap::new();
        src.poll(T - 100_000, &mut st).unwrap();
        commit(&wt, T / 1000);
        let p = src.poll(T + 61_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert!(matches!(p.events[0].kind, Kind::Commit { by: Actor::Agent, .. }));
    }

    #[test]
    fn agent_marker_stamped_under_a_sibling_registered_repo_marks_the_commit() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).unwrap();
            init(d);
        }
        let db = t.path().join("tally.db");
        let mut store = Store::open(&db).unwrap();
        let ida = crate::project::register_git_project(&mut store, &a, None).unwrap();
        let idb = crate::project::register_git_project(&mut store, &b, None).unwrap();
        // Claude ran in a and committed into b with `git -C ../b commit`
        let marker = Event {
            v: 1, id: "mk".into(), ts: T - 10_000, source: "claude-code".into(), project: Some(ida), session: None,
            kind: Kind::Commit { sha: "claude:toolu_1".into(), by: Actor::Agent, lines_changed: 0 },
        };
        store.append_events(&[marker]).unwrap();
        let lk = StoreLookup { store: Store::open(&db).unwrap(), claude_enabled: false };
        let mut src = GitSource::new(vec![RepoRef { local_id: idb, root: b.clone() }], Box::new(lk));
        let mut st = BTreeMap::new();
        src.poll(T - 100_000, &mut st).unwrap();
        commit(&b, T / 1000);
        let p = src.poll(T + 61_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert!(matches!(p.events[0].kind, Kind::Commit { by: Actor::Agent, .. }));
    }

    #[test]
    fn commit_in_a_linked_worktree_is_seen_from_the_main_checkout_exactly_once() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        let wt = tempfile::tempdir().unwrap();
        let wtp = wt.path().join("wt");
        git(dir.path(), T / 1000, &[], &["worktree", "add", "-q", wtp.to_str().unwrap(), "-b", "feature"]);
        src.poll(T - 100_000, &mut st).unwrap();
        commit(&wtp, T / 1000);
        let sha = head(&wtp);
        let p = src.poll(T + 61_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].id, format!("git:{sha}:commit"));
        assert!(src.poll(T + 200_000, &mut st).unwrap().events.is_empty());
    }

    #[test]
    fn other_committers_and_merge_commits_are_ignored() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        let d = dir.path();
        src.poll(T - 100_000, &mut st).unwrap();
        let other = [("GIT_COMMITTER_EMAIL", "other@x.org"), ("GIT_AUTHOR_EMAIL", "other@x.org")];
        git(d, T / 1000, &other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
        git(d, T / 1000 + 1, &[], &["checkout", "-q", "-b", "side"]);
        commit(d, T / 1000 + 2);
        let side = head(d);
        git(d, T / 1000 + 3, &[], &["checkout", "-q", "main"]);
        commit(d, T / 1000 + 4);
        let main = head(d);
        git(d, T / 1000 + 5, &[], &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
        let p = src.poll(T + 100_000, &mut st).unwrap();
        let mut ids: Vec<String> = p.events.iter().map(|e| e.id.clone()).collect();
        ids.sort();
        let mut want = vec![format!("git:{side}:commit"), format!("git:{main}:commit")];
        want.sort();
        assert_eq!(ids, want);
    }

    #[test]
    fn lookup_errors_drop_the_tick_and_a_later_poll_still_emits_by_the_right_actor() {
        if !git_available() { return; }
        let (dir, mut src, lk, mut st) = setup();
        lk.0.lock().unwrap().claude_wm = Some(T - 200_000);
        src.poll(T - 190_000, &mut st).unwrap();
        commit(dir.path(), T / 1000);
        {
            let mut f = lk.0.lock().unwrap();
            f.claude_wm = Some(T + 5_000);
            f.agent.push(("r1".into(), T - 1_000));
        }
        // the runner discards state on Err; model that by restoring it
        let saved = st.clone();
        lk.0.lock().unwrap().fail = true;
        assert!(src.poll(T + 10_000, &mut st).is_err());
        st = saved;
        lk.0.lock().unwrap().fail = false;
        let p = src.poll(T + 10_000, &mut st).unwrap();
        assert_eq!(p.events.len(), 1);
        assert!(matches!(p.events[0].kind, Kind::Commit { by: Actor::Agent, .. }));
    }

    #[test]
    fn a_commit_only_on_a_remote_tracking_ref_is_not_emitted() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        let d = dir.path();
        src.poll(T - 100_000, &mut st).unwrap();
        git(d, T / 1000, &[], &["checkout", "-q", "-b", "side"]);
        commit(d, T / 1000);
        let sha = head(d);
        git(d, T / 1000, &[], &["checkout", "-q", "main"]);
        git(d, T / 1000, &[], &["update-ref", "refs/remotes/origin/side", &sha]);
        git(d, T / 1000, &[], &["branch", "-q", "-D", "side"]);
        assert!(src.poll(T + 100_000, &mut st).unwrap().events.is_empty());
    }

    #[test]
    fn missing_git_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = GitSource::new(
            vec![RepoRef { local_id: "r1".into(), root: dir.path().into() }],
            Box::new(FakeLookup::default()),
        );
        src.program = "tally-no-such-git".into();
        let mut st = BTreeMap::new();
        st.insert("r1:wm".to_string(), "1".to_string());
        let err = src.poll(T, &mut st).err().unwrap();
        assert!(err.to_string().contains("git not found"), "{err}");
    }

    #[test]
    fn a_failing_repo_whose_root_exists_holds_the_watermark() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        let p = src.poll(T, &mut st).unwrap();
        assert_eq!(p.watermark, Some(T - GIT_LAG_MS));
        std::fs::write(dir.path().join(".git").join("HEAD"), "garbage\n").unwrap();
        let p = src.poll(T + 500_000, &mut st).unwrap();
        assert!(p.events.is_empty());
        assert_eq!(p.watermark, Some(T - GIT_LAG_MS));
    }

    #[test]
    fn a_repo_failing_for_over_a_day_stops_holding_the_watermark() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        src.poll(T, &mut st).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "garbage
").unwrap();
        let later = T + 30 * 86_400_000;
        let p = src.poll(later, &mut st).unwrap();
        assert_eq!(p.watermark, Some(later - GIT_LAG_MS));
        assert_eq!(src.warned.len(), 1);
    }

    #[test]
    fn a_deleted_repo_root_does_not_hold_the_watermark_and_is_logged_once() {
        if !git_available() { return; }
        let (dir, mut src, _lk, mut st) = setup();
        src.poll(T, &mut st).unwrap();
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
        let p = src.poll(T + 500_000, &mut st).unwrap();
        assert!(p.events.is_empty());
        assert_eq!(p.watermark, Some(T + 500_000 - GIT_LAG_MS));
        assert_eq!(src.warned.len(), 1);
        src.poll(T + 600_000, &mut st).unwrap();
        assert_eq!(src.warned.len(), 1);
    }
}
