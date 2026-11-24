//! Ruleset constants (spec section 7). Durations are in milliseconds.

pub const RULESET_VERSION: &str = "2026.11-1";

pub const MAX_INTERVAL_MS: i64 = 3_600_000;
pub const FIRST_ACTION_MS: i64 = 120_000;
pub const IDLE_SAMPLE_MIN_MS: i64 = 30_000;
pub const IDLE_GRACE_MS: i64 = 180_000;
pub const THINK_MAX_MS: i64 = 300_000;
pub const THINK_RELEVANCE_PM: u16 = 500;
pub const AGENT_IDLE_MS: i64 = 45_000;
pub const LOOP_WINDOW_MS: i64 = 3_600_000;
pub const NOVELTY_MIN: u32 = 24;
pub const VOCAB_MAX: usize = 50_000;
pub const CONTEXT_TAIL_BYTES: usize = 8192;
pub const COMMIT_INTERVAL_MS: i64 = 300_000;

pub const TRIVIAL_CHARS: u32 = 20;
pub const TRIVIAL_RELEVANCE_PM: u16 = 150;
pub const INTERRUPT_FOLLOW_MS: i64 = 120_000;
