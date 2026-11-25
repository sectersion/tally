//! Evidence tooling behind the pitch's "Evidence before we ask". Calls `tally_scoring::score`; never reimplements a rule.
//! The Hackatime-style comparator is the only other time computation and is an approximation (labelled as such).
pub mod build;
pub mod hackatime;
pub mod redteam;
pub mod replay;
pub mod scenarios;
