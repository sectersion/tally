pub mod claude_code;
pub mod config;
pub mod finalize;
pub mod ingest;
pub mod presence;
pub mod project;
pub mod run;
pub mod source;
pub mod status;
pub mod store;
#[cfg(test)]
pub(crate) mod test_support;
pub use project::{find_git_root, normalize_path, project_id, project_id_from_normalized};
