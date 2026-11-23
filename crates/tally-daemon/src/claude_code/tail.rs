//! Complete-line reader with a byte cursor for append-only JSONL logs.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

pub struct Chunk {
    pub lines: Vec<Vec<u8>>,
    pub start: u64,
    pub end: u64,
    pub rotated: bool,
    /// Stopped at the budget: more complete lines may follow.
    pub budget_hit: bool,
}

/// Reads whole lines from `offset`. A last line without `\n` is not returned and `end` stays at its start.
/// If the file is shorter than `offset` it was truncated or replaced: reading restarts at 0 and `rotated` is true.
/// Stops after the first complete line that takes the total past `budget` bytes, so one poll's memory is
/// bounded by `budget` plus one line. A trailing `\r` is stripped. Empty lines are skipped.
pub fn read_lines(path: &Path, offset: u64, budget: usize) -> std::io::Result<Chunk> {
    let file = std::fs::File::open(path)?;
    let rotated = file.metadata()?.len() < offset;
    let start = if rotated { 0 } else { offset };
    let mut r = BufReader::new(file);
    r.seek(SeekFrom::Start(start))?;
    let (mut lines, mut end, mut total, mut buf) = (Vec::new(), start, 0usize, Vec::new());
    while total <= budget {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            break; // EOF, or a line still being written
        }
        end += n as u64;
        total += n;
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        if !buf.is_empty() {
            lines.push(buf.clone());
        }
    }
    Ok(Chunk { lines, start, end, rotated, budget_hit: total > budget })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn partial_last_line_waits_for_its_newline() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(&p, b"one\ntw").unwrap();
        let c = read_lines(&p, 0, 1 << 20).unwrap();
        assert_eq!((c.lines, c.end), (vec![b"one".to_vec()], 4));
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"o\r\nthree\n").unwrap();
        let c = read_lines(&p, 4, 1 << 20).unwrap();
        assert_eq!((c.lines, c.end), (vec![b"two".to_vec(), b"three".to_vec()], 15));
        assert_eq!(read_lines(&p, 15, 1 << 20).unwrap().lines.len(), 0);
    }

    #[test]
    fn budget_bounds_a_poll_but_never_splits_a_line() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        let big = "x".repeat(3_000_000);
        std::fs::write(&p, format!("{big}\na\nb\n")).unwrap();
        let c = read_lines(&p, 0, 1024).unwrap();
        assert_eq!((c.lines.len(), c.lines[0].len(), c.budget_hit), (1, 3_000_000, true));
        let c = read_lines(&p, c.end, 1024).unwrap();
        assert_eq!((c.lines, c.budget_hit), (vec![b"a".to_vec(), b"b".to_vec()], false));
    }

    #[test]
    fn a_shorter_file_is_reread_from_the_start() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(&p, b"new\n").unwrap();
        let c = read_lines(&p, 500, 1 << 20).unwrap();
        assert!(c.rotated);
        assert_eq!((c.start, c.lines.len()), (0, 1));
    }

    #[test]
    fn non_utf8_bytes_pass_through_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(&p, b"\xff\xfe\nok\n").unwrap();
        let c = read_lines(&p, 0, 1 << 20).unwrap();
        assert_eq!(c.lines, vec![vec![0xff, 0xfe], b"ok".to_vec()]);
    }

    #[test]
    fn a_read_ending_on_a_newline_reaches_the_file_length() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(&p, b"a\n\n\r\nb\n").unwrap();
        let c = read_lines(&p, 0, 1 << 20).unwrap();
        // empty lines are skipped but consumed
        assert_eq!((c.lines.len(), c.end), (2, 7));
    }

    #[test]
    fn an_offset_equal_to_the_file_length_is_not_a_rotation() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(&p, b"a\nb\n").unwrap();
        let c = read_lines(&p, 4, 1 << 20).unwrap();
        assert_eq!((c.lines.len(), c.rotated, c.start, c.end), (0, false, 4, 4));
    }
}
