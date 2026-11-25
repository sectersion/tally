//! The only place a bundle leaves the machine. The URL carries the server id, never the public name.
use super::{Bundle, SubmitError};
use crate::api::{Api, ApiError};
use std::io::Write;

/// Returns the submission id.
pub fn upload(base: &str, server_id: &str, token: &str, b: &Bundle) -> Result<String, SubmitError> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&serde_json::to_vec(b)?)?;
    let v = Api::new(base, token).post_gzip(&format!("/v1/projects/{server_id}/submissions"), &gz.finish()?)?;
    v["id"].as_str().map(String::from).ok_or_else(|| ApiError::Network("BadBody".into()).into())
}

/// `to_ms` of the project's last completed submission. Best effort: any failure or unexpected shape is `None`.
/// (Server Task 8 defines GET /v1/projects/{id}/submissions; accepted shapes: a list, or `{"submissions": [...]}`,
/// of objects with `to_ms` and `status`; only `status == "completed"` counts.)
pub fn last_submitted_to(base: &str, server_id: &str, token: &str) -> Option<i64> {
    let v = Api::new(base, token).get_json(&format!("/v1/projects/{server_id}/submissions")).ok()?;
    let list = v.as_array().or_else(|| v["submissions"].as_array())?;
    list.iter().filter(|s| s["status"] == "completed").filter_map(|s| s["to_ms"].as_i64()).max()
}
