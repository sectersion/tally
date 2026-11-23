//! Publishes per-project chains and badges to the data branch.
use crate::hosting::{Hosting, HostingError};
use crate::published::{self, badge_json, PublishError};
use crate::store::{PublishedRow, Store};
use std::collections::BTreeMap;
use std::path::Path;
use tally_scoring::LedgerEntry;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct PublishConfig {
    pub enabled: bool,
    pub remote: Option<String>,
    pub branch: String,
}

impl Default for PublishConfig {
    fn default() -> Self {
        PublishConfig { enabled: false, remote: None, branch: "tally-data".into() }
    }
}

impl PublishConfig {
    fn remote_for(&self, name: &str) -> String {
        self.remote.clone().unwrap_or_else(|| format!("https://github.com/{name}.git"))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum PublishFail {
    #[error(transparent)]
    Published(#[from] PublishError),
    #[error(transparent)]
    Hosting(#[from] HostingError),
    #[error("remote chain for {project} ({file}) is not a prefix of the local chain")]
    RemoteDiverged { project: String, file: String },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<crate::store::StoreError> for PublishFail {
    fn from(e: crate::store::StoreError) -> Self {
        PublishError::Store(e).into()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectReport {
    pub name: String,
    pub new_entries: usize,
    pub total_entries: usize,
    pub verified_s: u64,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishReport {
    pub projects: Vec<ProjectReport>,
    pub pushed: bool,
}

impl PublishReport {
    /// One block per project, then the outcome line. `dry_run` also shows the badge bytes.
    pub fn render_text(&self, dry_run: bool) -> String {
        if self.projects.is_empty() {
            return "no named projects; nothing to publish\n".into();
        }
        let verb = if dry_run { "would write" } else { "wrote" };
        let mut s = String::new();
        for p in &self.projects {
            s += &format!("project {}: {} new entries, {} total, {}\n", p.name, p.new_entries, p.total_entries,
                published::hours_message(p.verified_s));
            for f in &p.files {
                s += &format!("  {verb} {f}\n");
            }
            if dry_run {
                s += &format!("  badge: {}\n", badge_json(p.verified_s).trim_end());
            }
        }
        s += if dry_run { "dry run: nothing written, nothing pushed\n" } else if self.pushed { "pushed\n" } else { "nothing to push\n" };
        s
    }
}

/// Files for one project: stored rows plus `extra`, one ledger per ruleset and the badge.
pub fn render(store: &Store, name: &str, extra: &[PublishedRow]) -> Result<Vec<(String, String)>, PublishFail> {
    let mut rows = store.published_rows(name)?;
    rows.extend(extra.iter().filter(|r| r.entry.project.as_deref() == Some(name)).cloned());
    let mut by_rs: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for r in &rows {
        by_rs.entry(&r.entry.ruleset).or_default().push(serde_json::to_string(&r.entry)?);
    }
    let mut files: Vec<_> = by_rs
        .into_iter()
        .map(|(rs, lines)| (format!("{name}/ledger-{rs}.json"), format!("[\n{}\n]\n", lines.join(",\n"))))
        .collect();
    files.push((format!("{name}/badge.json"), badge_json(published::verified_seconds(&rows))));
    Ok(files)
}

pub fn publish_once(
    store: &mut Store,
    cfg: &PublishConfig,
    home: &Path,
    dry_run: bool,
) -> Result<PublishReport, PublishFail> {
    let new = published::sync(store, !dry_run)?;
    // A dry run has stored nothing, so its rows ride along as `extra`.
    let extra: &[PublishedRow] = if dry_run { &new } else { &[] };
    let mut names = store.published_projects()?;
    for r in &new {
        names.extend(r.entry.project.clone());
    }
    names.sort();
    names.dedup();

    let mut projects = Vec::new();
    let mut rendered = Vec::new();
    for name in names {
        let files = render(store, &name, extra)?;
        let new_entries = new.iter().filter(|r| r.entry.project.as_deref() == Some(&name)).count();
        let mut all = store.published_rows(&name)?;
        all.extend(extra.iter().filter(|r| r.entry.project.as_deref() == Some(&name)).cloned());
        projects.push(ProjectReport {
            name: name.clone(),
            new_entries,
            total_entries: all.len(),
            verified_s: published::verified_seconds(&all),
            files: files.iter().map(|(p, _)| p.clone()).collect(),
        });
        rendered.push((name, files));
    }
    if dry_run {
        return Ok(PublishReport { projects, pushed: false });
    }

    type Files = Vec<(String, String)>;
    let mut by_remote: BTreeMap<String, Vec<(String, Files)>> = BTreeMap::new();
    for (name, files) in rendered {
        by_remote.entry(cfg.remote_for(&name)).or_default().push((name, files));
    }
    let mut pushed = false;
    for (remote, group) in by_remote {
        let host = Hosting::new(home, &remote, &cfg.branch);
        host.sync_down()?;
        let mut count = 0;
        let mut files = Vec::new();
        // A remote ledger we have no local chain for is someone else's history.
        for (name, pfiles) in &group {
            for f in host.ledger_files(name)? {
                if !pfiles.iter().any(|(p, _)| *p == f) {
                    return Err(PublishFail::RemoteDiverged { project: name.clone(), file: f });
                }
            }
        }
        for (name, pfiles) in group {
            for (path, text) in pfiles {
                if path.ends_with("/badge.json") {
                    files.push((path, text));
                    continue;
                }
                let local: Vec<LedgerEntry> = serde_json::from_str(&text)?;
                let remote_chain: Vec<LedgerEntry> = match host.read(&path)? {
                    Some(t) => serde_json::from_str(&t)?,
                    None => Vec::new(),
                };
                if remote_chain.len() > local.len() || remote_chain.iter().zip(&local).any(|(a, b)| a.hash != b.hash) {
                    return Err(PublishFail::RemoteDiverged { project: name, file: path });
                }
                count += local.len() - remote_chain.len();
                files.push((path, text));
            }
        }
        pushed |= host.commit_and_push(&files, &format!("tally: publish {count} new entries"))?;
    }
    Ok(PublishReport { projects, pushed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::published::{store_with_ledger, verify_published_json};
    use crate::test_support::{edit, prompt};
    use std::process::Command;

    fn skip_without_git() -> bool {
        let missing = Command::new("git").arg("--version").output().is_err();
        if missing {
            eprintln!("git not found; skipping");
        }
        missing
    }
    fn git(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    }
    fn bare_remote() -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["init", "--bare", "-b", "tally-data"]);
        let url = d.path().to_string_lossy().into_owned();
        (d, url)
    }
    fn cfg(url: &str) -> PublishConfig {
        PublishConfig { enabled: true, remote: Some(url.into()), branch: "tally-data".into() }
    }
    fn named_store() -> Store {
        store_with_ledger(Some("owner/repo"))
    }
    fn commits(bare: &Path) -> usize {
        git(bare, &["rev-list", "--count", "tally-data"]).trim().parse().unwrap()
    }
    fn clone(url: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["clone", "-q", "-b", "tally-data", url, "c"]);
        d
    }
    fn ledger_name(s: &Store) -> String {
        format!("owner/repo/ledger-{}.json", s.published_rows("owner/repo").unwrap()[0].entry.ruleset)
    }
    fn drop_triggers(s: &Store) {
        let names: Vec<String> = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger'").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for n in names { s.conn.execute(&format!("DROP TRIGGER {n}"), []).unwrap(); }
    }

    #[test] fn first_publish_creates_the_branch_with_badge_and_a_verifiable_chain() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        let r = publish_once(&mut s, &cfg(&url), home.path(), false).unwrap();
        assert!(r.pushed);
        assert_eq!(r.projects[0].total_entries, 2);
        let c = clone(&url);
        let root = c.path().join("c");
        let text = std::fs::read_to_string(root.join(ledger_name(&s))).unwrap();
        let v = verify_published_json(&text).unwrap();
        assert_eq!((v.entries, v.project.as_deref()), (2, Some("owner/repo")));
        // read from the repo, not the checkout: autocrlf may rewrite line endings on Windows
        assert_eq!(git(bare.path(), &["show", "tally-data:owner/repo/badge.json"]), badge_json(220));
        let mut top: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        top.sort();
        assert_eq!(top, [".git", "owner"]);
        let log = git(bare.path(), &["log", "--format=%an%n%s"]);
        assert_eq!(log.lines().collect::<Vec<_>>(), ["tally", "tally: publish 2 new entries"]);
        assert!(!log.contains("p1") && !log.contains("C:/r"));
    }
    #[test] fn second_publish_with_nothing_new_makes_no_commit() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        assert!(publish_once(&mut s, &cfg(&url), home.path(), false).unwrap().pushed);
        assert!(!publish_once(&mut s, &cfg(&url), home.path(), false).unwrap().pushed);
        assert_eq!(commits(bare.path()), 1);
    }
    #[test] fn new_claims_extend_the_branch_by_one_commit() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        publish_once(&mut s, &cfg(&url), home.path(), false).unwrap();
        let name = ledger_name(&s);
        let first = std::fs::read_to_string(clone(&url).path().join("c").join(&name)).unwrap();
        s.append_events(&[edit("z", 2_000_000)]).unwrap();
        s.set_watermark("src", 9_000_000).unwrap();
        finalize(&mut s).unwrap();
        assert!(publish_once(&mut s, &cfg(&url), home.path(), false).unwrap().pushed);
        assert_eq!(commits(bare.path()), 2);
        let second = std::fs::read_to_string(clone(&url).path().join("c").join(&name)).unwrap();
        let (a, b): (Vec<_>, Vec<_>) = (first.lines().collect(), second.lines().collect());
        assert_eq!(b.len(), a.len() + 1);
        // the old last entry gains a trailing comma; every earlier line is byte-identical
        assert_eq!(&a[..a.len() - 2], &b[..a.len() - 2]);
        assert_eq!(a[a.len() - 2].to_string() + ",", b[a.len() - 2]);
        assert_eq!(verify_published_json(&second).unwrap().entries, 3);
    }
    #[test] fn a_failed_push_leaves_nothing_half_published_and_the_next_run_recovers() {
        if skip_without_git() { return; }
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        let missing = home.path().join("nope").to_string_lossy().into_owned();
        assert!(matches!(
            publish_once(&mut s, &cfg(&missing), home.path(), false),
            Err(PublishFail::Hosting(HostingError::Git { .. }))
        ));
        assert_eq!(s.published_rows("owner/repo").unwrap().len(), 2);
        let (bare, url) = bare_remote();
        assert!(publish_once(&mut s, &cfg(&url), home.path(), false).unwrap().pushed);
        let (bare2, url2) = bare_remote();
        let home2 = tempfile::tempdir().unwrap();
        publish_once(&mut named_store(), &cfg(&url2), home2.path(), false).unwrap();
        // names and blob hashes
        assert_eq!(git(bare.path(), &["ls-tree", "-r", "tally-data"]), git(bare2.path(), &["ls-tree", "-r", "tally-data"]));
        assert_eq!(commits(bare.path()), 1);
    }
    #[test] fn a_push_rejected_after_the_commit_recovers_on_the_next_run() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        let hook = bare.path().join("hooks/pre-receive");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(matches!(
            publish_once(&mut s, &cfg(&url), home.path(), false),
            Err(PublishFail::Hosting(HostingError::Git { .. }))
        ));
        std::fs::remove_file(&hook).unwrap();
        assert!(publish_once(&mut s, &cfg(&url), home.path(), false).unwrap().pushed);
        assert_eq!(commits(bare.path()), 1);
    }
    #[test] fn a_remote_chain_from_another_machine_is_refused() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let (home_a, home_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        publish_once(&mut named_store(), &cfg(&url), home_a.path(), false).unwrap();
        let head = git(bare.path(), &["rev-parse", "tally-data"]);
        let mut b = Store::open_in_memory().unwrap();
        b.append_events(&[prompt("x", 0, 600), edit("y", 100_000)]).unwrap();
        b.register_project("p1", "C:/other", Some("owner/repo")).unwrap();
        b.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut b).unwrap();
        assert!(matches!(
            publish_once(&mut b, &cfg(&url), home_b.path(), false),
            Err(PublishFail::RemoteDiverged { .. })
        ));
        assert_eq!(git(bare.path(), &["rev-parse", "tally-data"]), head);
    }
    #[test] fn a_remote_ledger_file_with_no_local_chain_is_refused() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        publish_once(&mut s, &cfg(&url), home.path(), false).unwrap();
        // someone else pushes a chain for a ruleset we have no rows for
        let c = clone(&url);
        let root = c.path().join("c");
        std::fs::copy(root.join(ledger_name(&s)), root.join("owner/repo/ledger-other.json")).unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["-c", "user.name=x", "-c", "user.email=x@x", "commit", "-q", "-m", "other"]);
        git(&root, &["push", "-q", "origin", "tally-data"]);
        let head = git(bare.path(), &["rev-parse", "tally-data"]);
        assert!(matches!(
            publish_once(&mut s, &cfg(&url), home.path(), false),
            Err(PublishFail::RemoteDiverged { file, .. }) if file == "owner/repo/ledger-other.json"
        ));
        assert_eq!(git(bare.path(), &["rev-parse", "tally-data"]), head);
    }
    #[test] fn a_remote_chain_longer_than_the_local_one_is_refused() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let (home_a, home_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut a = named_store();
        a.append_events(&[edit("z", 2_000_000)]).unwrap();
        a.set_watermark("src", 9_000_000).unwrap();
        finalize(&mut a).unwrap();
        publish_once(&mut a, &cfg(&url), home_a.path(), false).unwrap();
        let head = git(bare.path(), &["rev-parse", "tally-data"]);
        // same first two claims, so the hashes agree but the remote has one more
        assert!(matches!(
            publish_once(&mut named_store(), &cfg(&url), home_b.path(), false),
            Err(PublishFail::RemoteDiverged { .. })
        ));
        assert_eq!(git(bare.path(), &["rev-parse", "tally-data"]), head);
    }
    #[test] fn a_ledger_that_does_not_verify_publishes_nothing() {
        if skip_without_git() { return; }
        let (bare, url) = bare_remote();
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        drop_triggers(&s);
        s.conn.execute("UPDATE ledger SET entry_json = replace(entry_json, '\"verified_s\":100', '\"verified_s\":900') WHERE seq = 2", []).unwrap();
        assert!(matches!(
            publish_once(&mut s, &cfg(&url), home.path(), false),
            Err(PublishFail::Published(PublishError::MainLedger(_)))
        ));
        let o = Command::new("git").current_dir(bare.path()).args(["rev-parse", "--verify", "tally-data"]).output().unwrap();
        assert!(!o.status.success());
    }
    #[test] fn dry_run_touches_neither_the_store_nor_git() {
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        let missing = home.path().join("nope").to_string_lossy().into_owned();
        let r = publish_once(&mut s, &cfg(&missing), home.path(), true).unwrap();
        assert_eq!((r.projects[0].name.as_str(), r.projects[0].new_entries), ("owner/repo", 2));
        assert!(!r.pushed);
        assert!(s.published_projects().unwrap().is_empty());
        assert!(!home.path().join("publish").exists());
    }
    #[test] fn git_errors_do_not_echo_the_remote_url() {
        if skip_without_git() { return; }
        let home = tempfile::tempdir().unwrap();
        let mut s = named_store();
        let c = cfg("https://user:secret@example.invalid/x.git");
        let e = publish_once(&mut s, &c, home.path(), false).unwrap_err();
        assert!(!e.to_string().contains("secret") && !format!("{e:?}").contains("secret"), "{e}");
    }
}
