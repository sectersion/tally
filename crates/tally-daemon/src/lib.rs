pub mod adapters;
pub mod analytics;
pub mod api;
pub mod assets;
pub mod claude_code;
pub mod config;
pub mod credentials;
pub mod dash_api;
pub mod doctor;
pub mod ensure;
pub mod finalize;
pub mod git;
pub mod heartbeat;
pub mod hosting;
pub mod publish;
pub mod ingest;
pub mod presence;
pub mod presence_select;
mod presence_macos;
#[cfg(windows)]
mod presence_win;
pub mod project;
pub mod published;
pub mod repair;
pub mod replay;
pub mod run;
pub mod serve;
pub mod setup;
pub mod snapshot;
pub mod source;
pub mod status;
pub mod store;
pub mod sync;
pub mod testrun;
#[cfg(any(test, feature = "test-kit"))]
pub mod test_kit;
#[cfg(test)]
pub(crate) mod test_support;
pub use project::{find_git_root, normalize_path, project_id, project_id_from_normalized};
