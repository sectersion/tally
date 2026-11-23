pub mod adapters;
pub mod api;
pub mod claude_code;
pub mod config;
pub mod credentials;
pub mod finalize;
pub mod git;
pub mod hosting;
pub mod publish;
pub mod ingest;
pub mod presence;
#[cfg(windows)]
mod presence_win;
pub mod project;
pub mod published;
pub mod repair;
pub mod run;
pub mod source;
pub mod status;
pub mod store;
pub mod sync;
pub mod testrun;
#[cfg(test)]
pub(crate) mod test_support;
pub use project::{find_git_root, normalize_path, project_id, project_id_from_normalized};
