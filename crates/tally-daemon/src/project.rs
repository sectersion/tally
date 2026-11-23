use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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
    #[test]
    fn no_git_root_in_plain_directory() {
        let t = tempfile::tempdir().unwrap();
        assert!(find_git_root(t.path()).is_none_or(|r| !r.starts_with(t.path())));
    }
}
