//! Local text features (relevance, novelty). Integer math only.
use crate::ruleset::{CONTEXT_TAIL_BYTES, NOVELTY_MIN, VOCAB_MAX};
use std::collections::BTreeSet;

pub const STOPWORDS: &[&str] = &[
    "the", "and", "for", "that", "this", "with", "you", "are", "not", "but", "have", "was", "can",
    "all", "any", "its", "from", "they", "will", "would", "there", "their", "what", "about",
    "which", "when", "make", "like", "just", "into", "than", "then", "them", "these", "some",
    "could", "other", "only", "also", "been",
];

/// Lowercase, then find runs matching `[a-z_][a-z0-9_]{2,}`; drop stopwords.
/// Non-ASCII chars are separators.
pub fn tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        // Leading digits cannot start a token (regex semantics: "12abc" -> "abc").
        .map(|run| run.trim_start_matches(|c: char| c.is_ascii_digit()))
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(t))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct Vocab(BTreeSet<String>);

impl Vocab {
    /// Stops adding after `VOCAB_MAX` distinct tokens; pass most recently modified first.
    pub fn from_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vocab {
        let mut v = Vocab::default();
        for t in texts {
            v.extend(t);
        }
        v
    }

    pub fn extend(&mut self, text: &str) {
        for t in tokens(text) {
            if self.0.len() >= VOCAB_MAX {
                return;
            }
            self.0.insert(t);
        }
    }
}

pub fn relevance_pm(prompt: &str, vocab: &Vocab) -> u16 {
    let distinct: BTreeSet<String> = tokens(prompt).into_iter().collect();
    let hit = distinct.iter().filter(|t| vocab.0.contains(*t)).count();
    (1000 * hit / distinct.len().max(1)) as u16
}

fn zlen(data: &[u8]) -> usize {
    zstd::bulk::compress(data, 3)
        .expect("zstd compression of in-memory data cannot fail")
        .len()
}

pub fn novelty_bytes(context_tail: &str, prompt: &str) -> u32 {
    let mut start = context_tail.len().saturating_sub(CONTEXT_TAIL_BYTES);
    while !context_tail.is_char_boundary(start) {
        start += 1;
    }
    let tail = &context_tail[start..];
    let joined = [tail, prompt].concat();
    zlen(joined.as_bytes()).saturating_sub(zlen(tail.as_bytes())) as u32
}

/// `(relevance_pm, novelty_bytes)`; relevance is 0 when novelty < `NOVELTY_MIN`.
pub fn prompt_features(prompt: &str, vocab: &Vocab, context_tail: &str) -> (u16, u32) {
    let nov = novelty_bytes(context_tail, prompt);
    let rel = if nov < NOVELTY_MIN { 0 } else { relevance_pm(prompt, vocab) };
    (rel, nov)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_lowercase_and_drop_stopwords_and_short() {
        assert_eq!(tokens("The Jump_Height is 2x OK"), vec!["jump_height"]);
    }
    #[test]
    fn plain_language_prompt_matches_prose_vocab() {
        let v = Vocab::from_texts(["// jump gravity tuning", "# A tiny game"]);
        assert_eq!(relevance_pm("the jump feels floaty", &v), 333);
    }
    #[test]
    fn lorem_ipsum_is_irrelevant() {
        let v = Vocab::from_texts(["fn player_jump(gravity: f32) {}"]);
        assert_eq!(relevance_pm("lorem ipsum dolor sit amet consectetur", &v), 0);
    }
    #[test]
    fn repeating_the_context_has_no_novelty() {
        let tail = "error[E0382]: borrow of moved value: `player` at src/main.rs:42".repeat(20);
        let (rel, nov) = prompt_features(&tail[..200], &Vocab::from_texts([tail.as_str()]), &tail);
        assert!(nov < NOVELTY_MIN);
        assert_eq!(rel, 0);
    }
    #[test]
    fn non_ascii_and_digit_prefixes_do_not_panic() {
        assert_eq!(tokens("日本語 12abc x2y é_ab"), vec!["abc", "x2y", "_ab"]);
        let tail = "é".repeat(5000);
        let _ = novelty_bytes(&tail, "héllo wörld");
    }
}
