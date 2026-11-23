//! WakaTime-compatible heartbeat protocol types, auth and file hash.
//! Wire format is UNVERIFIED, see tests/fixtures/heartbeat/NOTES.md.
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Heartbeat {
    pub entity: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Float seconds since epoch.
    pub time: f64,
    pub category: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub language: Option<String>,
    pub is_write: Option<bool>,
    pub lines: Option<u32>,
    pub lineno: Option<u32>,
    pub cursorpos: Option<u32>,
    pub line_additions: Option<u32>,
    pub line_deletions: Option<u32>,
    pub ai_line_changes: Option<u32>,
    pub human_line_changes: Option<u32>,
    pub user_agent: Option<String>,
    pub plugin: Option<String>,
}

impl Heartbeat {
    pub fn time_ms(&self) -> i64 {
        (self.time * 1000.0).round() as i64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("malformed json")]
    Json,
    #[error("body is neither object nor array")]
    Shape,
}

/// A bulk array yields one result per element; an object yields one.
pub fn parse_body(bytes: &[u8]) -> Result<Vec<Result<Heartbeat, String>>, BodyError> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| BodyError::Json)?;
    let one = |v: serde_json::Value| serde_json::from_value::<Heartbeat>(v).map_err(|e| e.to_string());
    match v {
        serde_json::Value::Array(a) => Ok(a.into_iter().map(one).collect()),
        o @ serde_json::Value::Object(_) => Ok(vec![one(o)]),
        _ => Err(BodyError::Shape),
    }
}

/// Compares over the longer length with no early return.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut d = (a.len() ^ b.len()) as u64;
    for i in 0..n {
        d |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u64;
    }
    d == 0
}

/// Accepts `Basic base64(key)`, `Bearer key`, or `?api_key=key`.
pub fn check_auth(header: Option<&str>, query_key: Option<&str>, token: &str) -> bool {
    let mut ok = false;
    if let Some(h) = header {
        let h = h.trim();
        if let Some(b) = h.strip_prefix("Basic ") {
            if let Ok(k) = base64::engine::general_purpose::STANDARD.decode(b.trim()) {
                ok |= ct_eq(&k, token.as_bytes());
            }
        } else if let Some(k) = h.strip_prefix("Bearer ") {
            ok |= ct_eq(k.trim().as_bytes(), token.as_bytes());
        }
    }
    if let Some(k) = query_key {
        ok |= ct_eq(k.as_bytes(), token.as_bytes());
    }
    ok
}

/// Never stores or returns the path. Hash of "<project_id>\0<path relative to the git root, '/' separators>".
pub fn file_hash(project_id: &str, rel_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(project_id.as_bytes());
    h.update([0]);
    h.update(rel_path.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SINGLE: &str = include_str!("../../tests/fixtures/heartbeat/single.json");
    const BULK: &str = include_str!("../../tests/fixtures/heartbeat/bulk.json");

    #[test]
    fn parses_single_and_bulk_fixtures() {
        let s = parse_body(SINGLE.as_bytes()).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].as_ref().unwrap().kind.as_deref(), Some("file"));
        assert_eq!(parse_body(BULK.as_bytes()).unwrap().len(), 3);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let r = parse_body(SINGLE.as_bytes()).unwrap();
        assert!(r[0].is_ok()); // fixture carries future_field
    }

    #[test]
    fn missing_entity_or_time_is_an_error_for_that_heartbeat_only() {
        let r = parse_body(BULK.as_bytes()).unwrap();
        assert!(r[0].is_ok() && r[1].is_err() && r[2].is_ok());
        let r = parse_body(br#"[{"entity":"a"},{"entity":"a","time":1}]"#).unwrap();
        assert!(r[0].is_err() && r[1].is_ok());
        assert!(parse_body(b"{nope").is_err());
        assert!(parse_body(b"3").is_err());
    }

    #[test]
    fn time_float_seconds_to_ms() {
        let r = parse_body(SINGLE.as_bytes()).unwrap();
        assert_eq!(r[0].as_ref().unwrap().time_ms(), 1763800000123);
    }

    #[test]
    fn auth_accepts_basic_base64_key_and_query_key_and_bearer() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("k");
        assert!(check_auth(Some(&format!("Basic {b64}")), None, "k"));
        assert!(check_auth(None, Some("k"), "k"));
        assert!(check_auth(Some("Bearer k"), None, "k"));
    }

    #[test]
    fn auth_rejects_wrong_key_with_401_and_constant_time_compare() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("kk");
        assert!(!check_auth(Some(&format!("Basic {b64}")), None, "k"));
        assert!(!check_auth(Some("Bearer x"), Some("y"), "k"));
        assert!(!check_auth(None, None, "k"));
        assert!(!check_auth(Some("Basic !!!"), None, "k"));
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abcd") && !ct_eq(b"abc", b"abd") && !ct_eq(b"", b"\0"));
    }

    #[test]
    fn file_hash_is_stable_and_path_free() {
        let a = file_hash("p1", "src/main.rs");
        assert_eq!(a, file_hash("p1", "src/main.rs"));
        assert_ne!(a, file_hash("p2", "src/main.rs"));
        assert_ne!(a, file_hash("p1", "src/lib.rs"));
        assert_eq!(a.len(), 64);
        assert!(!a.contains("main"));
    }
}
