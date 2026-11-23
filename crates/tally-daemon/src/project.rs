use sha2::{Digest, Sha256};
use crate::store::{Store, StoreError};
use std::path::{Path, PathBuf};

#[derive(thiserror::Error, Debug)]
pub enum ProjectError {
    #[error("not inside a git repository")]
    NotGit,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Registers the git repository containing `path` (relative paths resolve against the cwd);
/// stores the canonical, normalized root and returns its local project id.
/// `public_name = None` keeps an existing name.
pub fn register_git_project(store: &mut Store, path: &Path, public_name: Option<&str>) -> Result<String, ProjectError> {
    Ok(register_git_project_matched(store, path, public_name)?.0)
}

/// `(adopted name, "common dir" or "remote")`.
pub type Adopted = Option<(String, &'static str)>;

/// Like `register_git_project`; also returns `(adopted name, "common dir" | "remote")` when an unnamed
/// project took the public name of a sibling. Adoption happens only here, never for claims already ledgered.
pub fn register_git_project_matched(
    store: &mut Store,
    path: &Path,
    public_name: Option<&str>,
) -> Result<(String, Adopted), ProjectError> {
    std::fs::canonicalize(path)?; // a missing path is an Io error, not NotGit
    let (root, id) = project_for_dir(path).ok_or(ProjectError::NotGit)?;
    store.register_project(&id, &normalize_path(&root.to_string_lossy()), public_name)?;
    store.set_project_identity(&id, &repo_identity(&root))?;
    let rows = store.projects()?;
    let Some(me) = rows.iter().find(|p| p.local_id == id) else { return Ok((id, None)) };
    if me.public_name.is_some() {
        return Ok((id, None));
    }
    let sibling = |field: fn(&crate::store::ProjectRow) -> &Option<String>| {
        let key = field(me).as_ref()?;
        rows.iter().find(|p| p.local_id != id && field(p).as_ref() == Some(key) && p.public_name.is_some()).and_then(|p| p.public_name.clone())
    };
    let adopted = sibling(|p| &p.common_dir)
        .map(|n| (n, "common dir"))
        .or_else(|| sibling(|p| &p.remote).map(|n| (n, "remote")));
    if let Some((name, _)) = &adopted {
        store.register_project(&id, &me.root, Some(name))?;
    }
    Ok((id, adopted))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub common_dir: Option<String>,
    pub remote: Option<String>,
}

/// What ties linked worktrees and clones of one repository together. Any git failure (including git
/// missing) gives `None` for that field. Both values stay in the local `projects` table.
pub fn repo_identity(root: &Path) -> RepoIdentity {
    let git = |args: &[&str]| {
        let out = crate::git::run_git("git", root, args).ok()?;
        Some(out.trim().to_string()).filter(|s| !s.is_empty())
    };
    RepoIdentity {
        common_dir: git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]).map(|d| normalize_path(&d)),
        remote: git(&["config", "--get", "remote.origin.url"]).map(|u| normalize_remote(&u)),
    }
}

/// `https://user:tok@GitHub.com/O/r.git` and `git@github.com:o/r` both give `github.com/o/r`.
pub fn normalize_remote(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let (scheme, rest) = match lower.split_once("://") {
        Some((_, r)) => (true, r),
        None => (false, lower.as_str()),
    };
    let auth_end = rest.find('/').unwrap_or(rest.len());
    let rest = rest[..auth_end].rfind('@').map_or(rest, |i| &rest[i + 1..]);
    let mut out = rest.to_string();
    if !scheme {
        // scp-like `host:owner/repo`
        if let Some(i) = out.find(':').filter(|&i| out.find('/').is_none_or(|sl| i < sl)) {
            out.replace_range(i..=i, "/");
        }
    }
    let t = out.trim_end_matches('/');
    t.strip_suffix(".git").unwrap_or(t).trim_end_matches('/').to_string()
}

/// Local project id of the git repository containing `dir`, without registering it. `None` outside git
/// or when `dir` does not exist. The root is canonical (not normalized).
pub fn project_for_dir(dir: &Path) -> Option<(PathBuf, String)> {
    let root = find_git_root(&std::fs::canonicalize(dir).ok()?)?;
    let id = project_id_from_normalized(&normalize_path(&root.to_string_lossy()));
    Some((root, id))
}

/// Pure string normalization, identical on every OS (no `std::path`).
pub fn normalize_path(raw: &str) -> String {
    // Verbatim UNC (`\\?\UNC\srv\share`, or with one leading backslash) is `//srv/share`.
    let unc_form;
    let raw = match raw.strip_prefix(r"\\?\UNC\").or_else(|| raw.strip_prefix(r"\?\UNC\")) {
        Some(rest) => {
            unc_form = format!(r"\\{rest}");
            unc_form.as_str()
        }
        None => raw,
    };
    // Windows verbatim prefix: `\\?\` (real) or `\?\`.
    let s = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\?\"))
        .unwrap_or(raw)
        .replace('\\', "/");
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        return if s.is_empty() { s } else { "/".into() };
    }
    // Drive root like `C:` keeps its slash.
    if t.len() == 2 && t.ends_with(':') {
        return format!("{t}/");
    }
    t.to_string()
}

pub fn project_id_from_normalized(normalized: &str) -> String {
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

pub fn project_id(root: &Path) -> std::io::Result<String> {
    let canon = std::fs::canonicalize(root)?;
    Ok(project_id_from_normalized(&normalize_path(
        &canon.to_string_lossy(),
    )))
}

pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_handles_windows_and_trailing_separators() {
        assert_eq!(normalize_path(r"\?\C:\Users\o\tally\"), "C:/Users/o/tally");
        assert_eq!(normalize_path(r"\\?\C:\Users\o\tally\"), "C:/Users/o/tally");
        assert_eq!(normalize_path(r"C:\Users\o\tally\"), "C:/Users/o/tally");
        assert_eq!(normalize_path(r"C:\Users\o\tally"), "C:/Users/o/tally");
        assert_eq!(normalize_path("/home/o/tally/"), "/home/o/tally");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path(r"C:\"), "C:/");
    }
    #[test]
    fn project_id_is_sha256_of_normalized_path() {
        assert_eq!(
            project_id_from_normalized("C:/Users/o/tally"),
            "e72dd4b784b9b9080795c60969a3bfeaa51b8f6efe4b78cd6667db6625f36965"
        );
    }
    #[test]
    fn git_root_found_from_subdirectory_and_for_worktree_file() {
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir(t.path().join(".git")).unwrap();
        assert_eq!(find_git_root(&sub).unwrap(), t.path());
        let w = tempfile::tempdir().unwrap();
        std::fs::write(w.path().join(".git"), "gitdir: elsewhere").unwrap();
        assert_eq!(find_git_root(w.path()).unwrap(), w.path());
    }
    /// Guards the process-wide cwd for tests that change it.
    static CWD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn registering_dot_from_a_subdirectory_stores_the_canonical_repo_root() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        let sub = repo.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let mut s = Store::open_in_memory().unwrap();

        let r = {
            let _g = CWD.lock().unwrap_or_else(|e| e.into_inner());
            let old = std::env::current_dir().unwrap();
            std::env::set_current_dir(&sub).unwrap();
            let r = register_git_project(&mut s, Path::new("."), None);
            std::env::set_current_dir(old).unwrap();
            r
        };
        let id = r.unwrap();
        assert_eq!(id, project_id(&repo).unwrap());
        let root: String = s.conn.query_row("SELECT root FROM projects WHERE local_id = ?1", [&id], |r| r.get(0)).unwrap();
        assert_eq!(root, normalize_path(&std::fs::canonicalize(&repo).unwrap().to_string_lossy()));
        assert!(Path::new(&root).is_absolute(), "{root}");
    }
    #[test]
    fn project_for_dir_matches_project_id_and_needs_an_existing_dir() {
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("a");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir(t.path().join(".git")).unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        assert_eq!(project_for_dir(&sub), Some((root, project_id(t.path()).unwrap())));
        assert_eq!(project_for_dir(&t.path().join("missing")), None);
    }
    #[test]
    fn verbatim_unc_normalizes_like_plain_unc() {
        assert_eq!(normalize_path(r"\\?\UNC\srv\share\d\"), "//srv/share/d");
        assert_eq!(normalize_path(r"\?\UNC\srv\share\d\"), "//srv/share/d");
        assert_eq!(normalize_path(r"\\srv\share\d"), "//srv/share/d");
        assert_eq!(normalize_path(r"\\?\C:\x"), "C:/x");
        assert_eq!(normalize_path(r"\?\C:\x"), "C:/x");
    }
    #[test]
    fn remotes_normalize_to_host_owner_repo() {
        assert_eq!(normalize_remote("https://github.com/Sectersion/tally.git"), "github.com/sectersion/tally");
        assert_eq!(normalize_remote("git@github.com:sectersion/tally"), "github.com/sectersion/tally");
        assert_eq!(normalize_remote("https://user:tok@github.com/o/r/"), "github.com/o/r");
    }

    fn git_available() -> bool {
        std::process::Command::new("git").arg("--version").output().is_ok()
    }
    /// Hermetic git: no system or global config, fixed identity.
    fn git(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Me", "-c", "user.email=me@x.org"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", dir.join("no-global-config"))
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }
    fn init(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
    }
    fn worktree(main: &Path, wt: &Path, branch: &str) {
        git(main, &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", branch]);
    }

    #[test]
    fn worktree_and_main_checkout_share_a_common_dir() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt) = (t.path().join("main"), t.path().join("wt"));
        init(&main);
        worktree(&main, &wt, "f");
        let (a, b) = (repo_identity(&main), repo_identity(&wt));
        assert!(a.common_dir.is_some());
        assert_eq!(a.common_dir, b.common_dir);
        assert_ne!(project_id(&main).unwrap(), project_id(&wt).unwrap());
    }
    #[test]
    fn two_clones_of_one_remote_share_the_remote() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        init(&a);
        init(&b);
        git(&a, &["remote", "add", "origin", "https://github.com/Sectersion/tally.git"]);
        git(&b, &["remote", "add", "origin", "git@github.com:sectersion/tally"]);
        let (ia, ib) = (repo_identity(&a), repo_identity(&b));
        assert_eq!(ia.remote.as_deref(), Some("github.com/sectersion/tally"));
        assert_eq!(ia.remote, ib.remote);
        assert_ne!(ia.common_dir, ib.common_dir);
    }
    #[test]
    fn registering_a_worktree_adopts_the_name_of_its_main_checkout() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt) = (t.path().join("main"), t.path().join("wt"));
        init(&main);
        worktree(&main, &wt, "f");
        let mut s = Store::open_in_memory().unwrap();
        let (m, none) = register_git_project_matched(&mut s, &main, Some("o/r")).unwrap();
        assert_eq!(none, None);
        let (w, adopted) = register_git_project_matched(&mut s, &wt, None).unwrap();
        assert_eq!(adopted, Some(("o/r".to_string(), "common dir")));
        let names = s.project_names().unwrap();
        assert_eq!((names[&m].as_str(), names[&w].as_str()), ("o/r", "o/r"));
    }
    #[test]
    fn a_clone_adopts_the_name_via_the_remote() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        for d in [&a, &b] {
            init(d);
            git(d, &["remote", "add", "origin", "https://github.com/o/r.git"]);
        }
        let mut s = Store::open_in_memory().unwrap();
        register_git_project(&mut s, &a, Some("o/r")).unwrap();
        let (_, adopted) = register_git_project_matched(&mut s, &b, None).unwrap();
        assert_eq!(adopted, Some(("o/r".to_string(), "remote")));
    }
    #[test]
    fn an_explicit_name_wins_and_unrelated_repos_stay_unnamed() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt, other) = (t.path().join("main"), t.path().join("wt"), t.path().join("other"));
        init(&main);
        init(&other);
        worktree(&main, &wt, "f");
        let mut s = Store::open_in_memory().unwrap();
        register_git_project(&mut s, &main, Some("o/r")).unwrap();
        let (w, adopted) = register_git_project_matched(&mut s, &wt, Some("o/other")).unwrap();
        assert_eq!(adopted, None);
        let (o, adopted) = register_git_project_matched(&mut s, &other, None).unwrap();
        assert_eq!(adopted, None);
        let names = s.project_names().unwrap();
        assert_eq!(names[&w], "o/other");
        assert!(!names.contains_key(&o));
    }
    #[test]
    fn identity_without_git_is_none_not_an_error() {
        let t = tempfile::tempdir().unwrap(); // a plain directory, no repository
        let plain = repo_identity(&t.path().join("missing"));
        assert_eq!(plain, RepoIdentity { common_dir: None, remote: None });
    }
    #[test]
    fn repos_to_poll_returns_one_repo_per_common_dir() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt, other) = (t.path().join("main"), t.path().join("wt"), t.path().join("other"));
        init(&main);
        init(&other);
        worktree(&main, &wt, "f");
        let mut s = Store::open_in_memory().unwrap();
        let m = register_git_project(&mut s, &main, None).unwrap();
        register_git_project(&mut s, &wt, None).unwrap();
        let o = register_git_project(&mut s, &other, None).unwrap();
        let ids: Vec<String> = s.repos_to_poll().unwrap().into_iter().map(|r| r.0).collect();
        assert_eq!(ids, vec![m, o]);
    }
    #[test]
    fn a_removed_root_is_skipped_in_favour_of_a_live_one_in_its_group() {
        if !git_available() { return; }
        let t = tempfile::tempdir().unwrap();
        let (main, wt) = (t.path().join("main"), t.path().join("wt"));
        init(&main);
        worktree(&main, &wt, "f");
        let mut s = Store::open_in_memory().unwrap();
        register_git_project(&mut s, &main, None).unwrap();
        let w = register_git_project(&mut s, &wt, None).unwrap();
        // the main checkout vanishes (its common dir is gone, the worktree row remains)
        s.conn.execute("UPDATE projects SET root = ?1 WHERE local_id != ?2", [t.path().join("gone").to_str().unwrap(), &w]).unwrap();
        let repos = s.repos_to_poll().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].0, w);
        // when none exist, the earliest registered is kept
        s.conn.execute("UPDATE projects SET root = ?1", [t.path().join("gone").to_str().unwrap()]).unwrap();
        let first = s.projects().unwrap()[0].local_id.clone();
        assert_eq!(s.repos_to_poll().unwrap()[0].0, first);
    }
    #[test]
    fn identity_columns_are_added_to_an_old_database() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("tally.db");
        {
            let c = rusqlite::Connection::open(&p).unwrap();
            c.execute_batch("CREATE TABLE projects(local_id TEXT PRIMARY KEY, root TEXT NOT NULL, public_name TEXT); INSERT INTO projects VALUES('a','/r','o/r');").unwrap();
        }
        let mut s = Store::open(&p).unwrap();
        let rows = s.projects().unwrap();
        assert_eq!((rows[0].public_name.as_deref(), rows[0].common_dir.clone()), (Some("o/r"), None));
        s.set_project_identity("a", &RepoIdentity { common_dir: Some("/r/.git".into()), remote: Some("h/o/r".into()) }).unwrap();
        assert_eq!(s.projects().unwrap()[0].remote.as_deref(), Some("h/o/r"));
        Store::open(&p).unwrap(); // reopening is a no-op
    }
    #[test]
    fn no_git_root_in_plain_directory() {
        let t = tempfile::tempdir().unwrap();
        assert!(find_git_root(t.path()).is_none_or(|r| !r.starts_with(t.path())));
    }
}
