//! `<home>/device_tokens`: one `<server_id> <token>` line per project, sorted by id.
//! Tokens are secrets: nothing here logs or returns them in an error message.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn token_path(home: &Path) -> PathBuf {
    home.join("device_tokens")
}

pub fn load_tokens(home: &Path) -> io::Result<BTreeMap<String, String>> {
    let text = match std::fs::read_to_string(token_path(home)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    Ok(text.lines().filter_map(|l| l.split_once(' ')).map(|(i, t)| (i.to_string(), t.to_string())).collect())
}

pub fn save_token(home: &Path, server_id: &str, token: &str) -> io::Result<()> {
    let token = token.trim();
    if token.is_empty() || !token.bytes().all(|b| (b'!'..=b'~').contains(&b)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "token must be non-empty visible ASCII"));
    }
    let _lock = lock(home)?;
    let mut all = load_tokens(home)?;
    all.insert(server_id.to_string(), token.to_string());
    write_all(home, &all)
}

/// True if a line was removed.
pub fn delete_token(home: &Path, server_id: &str) -> io::Result<bool> {
    let _lock = lock(home)?;
    let mut all = load_tokens(home)?;
    let removed = all.remove(server_id).is_some();
    if removed {
        write_all(home, &all)?;
    }
    Ok(removed)
}

/// Exclusive advisory lock (threads and processes) held for the read-modify-write; released on drop.
fn lock(home: &Path) -> io::Result<std::fs::File> {
    let f = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(home.join("device_tokens.lock"))?;
    f.lock()?;
    Ok(f)
}

fn write_all(home: &Path, all: &BTreeMap<String, String>) -> io::Result<()> {
    let tmp = home.join("device_tokens.tmp");
    // A stale file would keep its old mode; mode() only applies on creation.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    for (i, t) in all {
        writeln!(f, "{i} {t}")?;
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, token_path(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrips_trimmed_and_logout_removes_it() {
        let d = tempfile::tempdir().unwrap();
        assert!(load_tokens(d.path()).unwrap().is_empty());
        save_token(d.path(), "srv_1", "  tok_abc123\n").unwrap();
        save_token(d.path(), "srv_2", "tok_two").unwrap();
        save_token(d.path(), "srv_1", "tok_new").unwrap(); // replaces, does not add
        assert_eq!(
            load_tokens(d.path()).unwrap(),
            BTreeMap::from([("srv_1".into(), "tok_new".into()), ("srv_2".into(), "tok_two".into())])
        );
        assert!(delete_token(d.path(), "srv_1").unwrap());
        assert!(!delete_token(d.path(), "srv_1").unwrap());
        assert_eq!(load_tokens(d.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_token_that_cannot_go_in_a_header_is_refused() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["", "   ", "a b", "tok\u{e9}", "a\r\nX-Evil: 1"] {
            assert_eq!(save_token(d.path(), "srv_1", bad).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
        assert!(!token_path(d.path()).exists());
    }

    #[test]
    fn concurrent_saves_lose_nothing() {
        let d = tempfile::tempdir().unwrap();
        let hs: Vec<_> = (0..8)
            .map(|k| {
                let p = d.path().to_path_buf();
                std::thread::spawn(move || (0..40).for_each(|i| save_token(&p, &format!("s{k}_{i}"), "tok").unwrap()))
            })
            .collect();
        hs.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(load_tokens(d.path()).unwrap().len(), 320);
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        save_token(d.path(), "srv_1", "tok").unwrap();
        assert_eq!(std::fs::metadata(token_path(d.path())).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
