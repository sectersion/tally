//! Shared poll wrapper pieces for file-reading adapters.
use crate::claude_code::{line::parse_rfc3339_ms, WRITE_SLACK_MS};
use crate::project::normalize_path;
use crate::source::SourceState;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const PREFIX_BYTES: u64 = 4096;

/// New complete lines of one file plus the cursor to store once they are appended.
#[derive(Debug, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub end: u64,
    pub prefix: String,
}

fn key(path: &Path) -> String {
    format!("cursor:{}", normalize_path(&path.to_string_lossy()))
}

fn prefix_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))[..16].to_string()
}

/// Text from the stored offset up to the last complete line, or the whole file (up to its last complete
/// line) when the stored prefix hash no longer matches or the file shrank. `None` = nothing new.
/// Changes no state; call `commit_cursor` after the events are safely appended.
pub fn read_new(path: &Path, state: &dyn SourceState) -> std::io::Result<Option<Chunk>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let stored = state.get(&key(path)).and_then(|v| {
        let (o, h) = v.split_once(':')?;
        Some((o.parse::<u64>().ok()?, h.to_string()))
    });
    let mut start = 0;
    if let Some((off, hash)) = stored {
        if off <= len {
            let mut head = vec![0; off.min(PREFIX_BYTES) as usize];
            f.read_exact(&mut head)?;
            if prefix_hash(&head) == hash {
                start = off;
            }
        }
    }
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return Ok(None);
    };
    buf.truncate(last_nl + 1);
    let end = start + buf.len() as u64;
    let mut head = vec![0; end.min(PREFIX_BYTES) as usize];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut head)?;
    Ok(Some(Chunk { text: String::from_utf8_lossy(&buf).into_owned(), end, prefix: prefix_hash(&head) }))
}

pub fn commit_cursor(state: &mut dyn SourceState, path: &Path, chunk: &Chunk) {
    state.set(&key(path), &format!("{}:{}", chunk.end, chunk.prefix));
}

/// Log-supplied ids go into event ids, the public ledger and the server, so only UUID-like tokens
/// (alphanumerics and `-`, up to 64) pass through; anything else is replaced by a truncated sha256.
pub fn safe_id(s: &str) -> String {
    if !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return s.to_string();
    }
    hex::encode(Sha256::digest(s.as_bytes()))[..16].to_string()
}

pub fn stable_id(source: &str, native: &str, kind: &str) -> String {
    format!("{source}:{native}:{kind}")
}

pub fn iso_to_ms(s: &str) -> Option<i64> {
    if s.ends_with('Z') {
        return parse_rfc3339_ms(s);
    }
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

/// The only watermark a file adapter returns: below the write slack and below every open span start.
pub fn file_watermark(now_ms: i64, open_starts: impl IntoIterator<Item = i64>) -> i64 {
    open_starts.into_iter().fold(now_ms - WRITE_SLACK_MS, |w, s| w.min(s - 1))
}

#[cfg(test)]
mod tests {
    #[test]
    fn safe_id_keeps_uuids_and_hashes_text() {
        assert_eq!(super::safe_id("0b1c2d3e-4f50-6789-abcd-ef0123456789"), "0b1c2d3e-4f50-6789-abcd-ef0123456789");
        let h = super::safe_id("my secret prompt text");
        assert_eq!(h.len(), 16);
        assert!(!h.contains("secret"));
        assert_eq!(super::safe_id("QACANARY_UUID_1").len(), 16);
    }

    use super::*;
    use std::collections::BTreeMap;

    fn read_commit(p: &Path, st: &mut BTreeMap<String, String>) -> Option<String> {
        let c = read_new(p, st).unwrap()?;
        commit_cursor(st, p, &c);
        Some(c.text)
    }

    #[test]
    fn read_new_returns_only_complete_lines() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l");
        let mut st = BTreeMap::new();
        std::fs::write(&p, "a\nb\npart").unwrap();
        assert_eq!(read_commit(&p, &mut st).unwrap(), "a\nb\n");
        assert_eq!(st.values().next().unwrap().split(':').next(), Some("4"));
        assert_eq!(read_commit(&p, &mut st), None);
        std::fs::write(&p, "a\nb\npartial\n").unwrap();
        assert_eq!(read_commit(&p, &mut st).unwrap(), "partial\n");
    }

    #[test]
    fn rewritten_file_is_reread() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l");
        let mut st = BTreeMap::new();
        std::fs::write(&p, "a\nb\n").unwrap();
        read_commit(&p, &mut st).unwrap();
        std::fs::write(&p, "x\nb\nc\n").unwrap();
        assert_eq!(read_commit(&p, &mut st).unwrap(), "x\nb\nc\n");
    }

    #[test]
    fn truncated_file_is_reread() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l");
        let mut st = BTreeMap::new();
        std::fs::write(&p, "aaaa\nbbbb\n").unwrap();
        read_commit(&p, &mut st).unwrap();
        std::fs::write(&p, "aa\n").unwrap();
        assert_eq!(read_commit(&p, &mut st).unwrap(), "aa\n");
    }

    #[test]
    fn iso_to_ms_handles_offset_and_z() {
        let z = iso_to_ms("2026-11-23T10:00:00Z").unwrap();
        assert_eq!(iso_to_ms("2026-11-23T12:00:00+02:00"), Some(z));
        assert_eq!(iso_to_ms("garbage"), None);
    }

    #[test]
    fn file_watermark_keeps_write_slack() {
        assert_eq!(file_watermark(1_000_000, []), 880_000);
        assert_eq!(file_watermark(1_000_000, [500_000]), 499_999);
    }

    #[test]
    fn stable_id_format() {
        assert_eq!(stable_id("codex", "abc", "prompt"), "codex:abc:prompt");
    }
}
