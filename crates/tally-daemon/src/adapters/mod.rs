//! Shared types for harness adapters: a pure `parse` per harness plus the poll wrapper in `common`.
pub mod common;
pub mod copilot;

use crate::claude_code::features::{self, ContextTail};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tally_scoring::text::Vocab;

pub struct Parsed {
    pub events: Vec<tally_scoring::Event>,
    /// Agent file writes as `(file_hash, ts)`; never events.
    pub agent_write_hashes: Vec<(String, i64)>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ParseReport {
    /// Detected version label, "unknown" when not recognized.
    pub format: String,
    /// Records examined.
    pub records: usize,
    /// Recognized as not relevant, or unknown record types.
    pub skipped: usize,
    /// Malformed: bad JSON, missing timestamp, unparsable.
    pub errors: usize,
    /// Newest timestamp among emitted events.
    pub last_ts: Option<i64>,
}

pub trait Adapter {
    const ID: &'static str;
    /// Pure: whole text of one log (or the unread tail), no I/O, no clock.
    fn parse(text: &str, ctx: &dyn ParseCtx) -> (Parsed, ParseReport);
}

pub trait ParseCtx {
    fn project_for_dir(&self, cwd: &str) -> Option<(PathBuf, String)>;
    /// `(chars, relevance_pm, novelty_bytes)`. `last_output` = preceding assistant text,
    /// `last_error` = preceding failed tool result text (spec section 5).
    fn prompt_features(
        &self,
        root: Option<&Path>,
        text: &str,
        last_output: &str,
        last_error: &str,
        tail: &ContextTail,
    ) -> (u32, u16, u32);
}

/// Production context: real git lookups, vocab cached per root, project lookups cached per cwd.
#[derive(Default)]
pub struct RealCtx {
    dirs: RefCell<HashMap<String, Option<(PathBuf, String)>>>,
    vocabs: RefCell<HashMap<PathBuf, Vocab>>,
}

impl ParseCtx for RealCtx {
    fn project_for_dir(&self, cwd: &str) -> Option<(PathBuf, String)> {
        self.dirs
            .borrow_mut()
            .entry(cwd.to_string())
            .or_insert_with(|| crate::project::project_for_dir(Path::new(cwd)))
            .clone()
    }

    fn prompt_features(
        &self,
        root: Option<&Path>,
        text: &str,
        last_output: &str,
        last_error: &str,
        tail: &ContextTail,
    ) -> (u32, u16, u32) {
        let mut vocabs = self.vocabs.borrow_mut();
        let empty = Vocab::default();
        let vocab = match root {
            Some(r) => vocabs
                .entry(r.to_path_buf())
                .or_insert_with(|| features::project_vocab(r).unwrap_or_default()),
            None => &empty,
        };
        features::prompt_numbers(text, vocab, last_output, last_error, tail)
    }
}
