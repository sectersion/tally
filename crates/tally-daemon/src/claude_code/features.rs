//! Prompt features for the Claude Code adapter: project vocabulary and context tail.
use std::io::{Error, Read};
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;
use tally_scoring::ruleset::CONTEXT_TAIL_BYTES;
use tally_scoring::text::{prompt_features, Vocab};

const MAX_FILE_BYTES: u64 = 1 << 20;
const BINARY_SNIFF_BYTES: u64 = 8 * 1024;

/// Tokens of git-tracked text files under `root` (`git -C <root> ls-files -z`), most recently
/// modified first, until the vocabulary is full. Skips files over 1 MiB, files with a NUL in their
/// first 8 KiB, and non-UTF-8 files. `git` missing or failing is an `Err`.
pub fn project_vocab(root: &Path) -> std::io::Result<Vocab> {
    let out = Command::new("git").arg("-C").arg(root).args(["ls-files", "-z"]).output()?;
    if !out.status.success() {
        return Err(Error::other(format!("git ls-files failed: {}", out.status)));
    }
    let mut files: Vec<(SystemTime, std::path::PathBuf)> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .filter_map(|p| std::str::from_utf8(p).ok())
        .filter_map(|p| {
            let path = root.join(p);
            let md = std::fs::metadata(&path).ok()?;
            (md.is_file() && md.len() <= MAX_FILE_BYTES).then_some((md.modified().ok()?, path))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut vocab = Vocab::default();
    for (_, path) in files {
        if vocab.is_full() {
            break;
        }
        let mut bytes = Vec::new();
        let Ok(f) = std::fs::File::open(&path) else { continue };
        if f.take(MAX_FILE_BYTES).read_to_end(&mut bytes).is_err() {
            continue;
        }
        if bytes[..bytes.len().min(BINARY_SNIFF_BYTES as usize)].contains(&0) {
            continue;
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            vocab.extend(text);
        }
    }
    Ok(vocab)
}

/// Last `CONTEXT_TAIL_BYTES` of conversation text, char-boundary safe. Memory only.
#[derive(Default)]
pub struct ContextTail(String);

impl ContextTail {
    pub fn push(&mut self, text: &str) {
        let mut start = text.len().saturating_sub(CONTEXT_TAIL_BYTES);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        self.0.push('\n');
        self.0.push_str(&text[start..]);
        let mut cut = self.0.len().saturating_sub(CONTEXT_TAIL_BYTES);
        while !self.0.is_char_boundary(cut) {
            cut += 1;
        }
        self.0.drain(..cut);
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `(chars, relevance_pm, novelty_bytes)`. Vocabulary = `project` + `last_output` + `last_error`.
pub fn prompt_numbers(
    text: &str,
    project: &Vocab,
    last_output: &str,
    last_error: &str,
    tail: &ContextTail,
) -> (u32, u16, u32) {
    let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    let mut vocab = project.clone();
    vocab.extend(last_output);
    vocab.extend(last_error);
    let (rel, nov) = prompt_features(text, &vocab, tail.as_str());
    (chars, rel, nov)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_scoring::ruleset::CONTEXT_TAIL_BYTES;
    use tally_scoring::text::{relevance_pm, Vocab};

    #[test]
    fn context_tail_keeps_the_last_8_kib_on_char_boundaries() {
        let mut t = ContextTail::default();
        t.push(&"é".repeat(10_000));
        t.push("end");
        assert!(t.as_str().len() <= CONTEXT_TAIL_BYTES && t.as_str().ends_with("end"));
    }

    #[test]
    fn last_error_words_make_a_prompt_relevant() {
        let empty = Vocab::default();
        let tail = ContextTail::default();
        let p = "why does borrow_checker reject moved_value here";
        let (_, without, _) = prompt_numbers(p, &empty, "", "", &tail);
        let (chars, with, nov) =
            prompt_numbers(p, &empty, "", "error: borrow_checker moved_value", &tail);
        assert_eq!(chars, p.chars().count() as u32);
        assert!(with > without && nov >= 24, "{with} {without} {nov}");
    }

    #[test]
    fn vocab_comes_from_tracked_files_only() {
        use std::process::Command;
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipping: git not available");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git").arg("-C").arg(d.path()).args(args).status().unwrap();
            assert!(st.success());
        };
        git(&["init", "-q"]);
        std::fs::write(d.path().join("tracked.md"), "zebra_word").unwrap();
        std::fs::write(d.path().join("untracked.md"), "yak_word").unwrap();
        std::fs::write(d.path().join("bin.dat"), b"binary_word\0\xff").unwrap();
        git(&["add", "tracked.md", "bin.dat"]);
        let v = project_vocab(d.path()).unwrap();
        assert_eq!(relevance_pm("zebra_word", &v), 1000);
        assert_eq!(relevance_pm("yak_word", &v), 0);
        assert_eq!(relevance_pm("binary_word", &v), 0);
    }

    #[test]
    fn a_non_repository_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(project_vocab(d.path()).is_err());
    }

    #[test]
    fn a_huge_prompt_is_fine() {
        let p = "word ".repeat(1_000_000);
        let (chars, _, _) = prompt_numbers(&p, &Vocab::default(), "", "", &ContextTail::default());
        assert_eq!(chars, 5_000_000);
    }
}
