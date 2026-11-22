use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ruleset::RULESET_VERSION;
use crate::score::Claim;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    pub v: u32,
    pub ruleset: String,
    pub action_id: String,
    pub ts: i64,
    pub project: Option<String>,
    pub kind: String,
    pub verified_s: u32,
    pub unverified_s: u32,
    pub reasons: Vec<String>,
    pub features: BTreeMap<String, i64>,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, PartialEq)]
pub struct ChainError {
    pub index: usize,
    pub reason: &'static str,
}

/// The entry without `hash`, keys sorted, no whitespace. Relies on serde_json's sorted maps.
pub fn canonical_json(e: &LedgerEntry) -> String {
    let mut v = serde_json::to_value(e).expect("entry serializes");
    v.as_object_mut().expect("entry is an object").remove("hash");
    v.to_string()
}

pub fn entry_hash(e: &LedgerEntry) -> String {
    hex::encode(Sha256::digest(canonical_json(e).as_bytes()))
}

pub fn chain(claims: &[Claim], prev_hash: &str) -> Vec<LedgerEntry> {
    let mut out: Vec<LedgerEntry> = Vec::with_capacity(claims.len());
    for c in claims {
        let prev = out.last().map_or(prev_hash, |e| e.hash.as_str()).to_string();
        let mut e = LedgerEntry {
            v: 1,
            ruleset: RULESET_VERSION.to_string(),
            action_id: c.action_id.clone(),
            ts: c.ts,
            project: c.project.clone(),
            kind: c.kind.clone(),
            verified_s: c.verified_s,
            unverified_s: c.unverified_s,
            reasons: c.reasons.clone(),
            features: c.features.clone(),
            prev_hash: prev,
            hash: String::new(),
        };
        e.hash = entry_hash(&e);
        out.push(e);
    }
    out
}

/// Returns the first failing index. A tampered entry fails `bad_hash` before its successor is checked.
pub fn verify_chain(entries: &[LedgerEntry], prev_hash: &str) -> Result<(), ChainError> {
    let mut prev = prev_hash;
    for (index, e) in entries.iter().enumerate() {
        if e.prev_hash != prev {
            return Err(ChainError { index, reason: "bad_prev_hash" });
        }
        if e.hash != entry_hash(e) {
            return Err(ChainError { index, reason: "bad_hash" });
        }
        prev = &e.hash;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::Claim;
    use std::collections::BTreeMap;

    fn claim(id: &str) -> Claim {
        Claim {
            action_id: id.into(),
            ts: 1000,
            project: Some("p".into()),
            kind: "prompt".into(),
            verified_s: 60,
            unverified_s: 0,
            reasons: vec!["capped".into()],
            features: BTreeMap::from([("relevance_pm".to_string(), 600)]),
        }
    }

    fn sample_entry() -> LedgerEntry {
        let mut e = chain(&[claim("a1")], GENESIS_HASH).remove(0);
        e.hash = String::new();
        e
    }

    #[test]
    fn canonical_json_is_sorted_and_compact() {
        let e = sample_entry();
        assert_eq!(canonical_json(&e), format!(
            r#"{{"action_id":"a1","features":{{"relevance_pm":600}},"kind":"prompt","prev_hash":"{}","project":"p","reasons":["capped"],"ruleset":"2026.11-0","ts":1000,"unverified_s":0,"v":1,"verified_s":60}}"#,
            GENESIS_HASH));
    }

    #[test]
    fn null_project_is_kept() {
        let mut c = claim("a1");
        c.project = None;
        let e = chain(&[c], GENESIS_HASH).remove(0);
        assert!(canonical_json(&e).contains(r#""project":null"#));
    }

    #[test]
    fn chain_links_and_verifies() {
        let entries = chain(&[claim("a1"), claim("a2")], GENESIS_HASH);
        assert_eq!(entries[1].prev_hash, entries[0].hash);
        assert!(verify_chain(&entries, GENESIS_HASH).is_ok());
    }

    #[test]
    fn tampering_is_detected_at_the_right_index() {
        let mut entries = chain(&[claim("a1"), claim("a2"), claim("a3")], GENESIS_HASH);
        entries[1].verified_s += 60;
        let err = verify_chain(&entries, GENESIS_HASH).unwrap_err();
        assert_eq!((err.index, err.reason), (1, "bad_hash"));
    }

    #[test]
    fn broken_link_is_reported() {
        let mut entries = chain(&[claim("a1"), claim("a2")], GENESIS_HASH);
        entries[1].prev_hash = GENESIS_HASH.into();
        let err = verify_chain(&entries, GENESIS_HASH).unwrap_err();
        assert_eq!((err.index, err.reason), (1, "bad_prev_hash"));
        let err = verify_chain(&entries[..1], "x").unwrap_err();
        assert_eq!((err.index, err.reason), (0, "bad_prev_hash"));
    }
}
