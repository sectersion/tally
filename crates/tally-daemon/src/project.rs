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
    std::fs::canonicalize(path)?; // a missing path is an Io error, not NotGit
    let (root, id) = project_for_dir(path).ok_or(ProjectError::NotGit)?;
    store.register_project(&id, &normalize_path(&root.to_string_lossy()), public_name)?;
    Ok(id)
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
    fn no_git_root_in_plain_directory() {
        let t = tempfile::tempdir().unwrap();
        assert!(find_git_root(t.path()).is_none_or(|r| !r.starts_with(t.path())));
    }
}
