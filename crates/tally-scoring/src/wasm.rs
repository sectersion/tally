//! JS bindings. Logic lives in `*_impl` (plain Rust, natively testable); exports just map errors.
use crate::{event::Event, ledger, ruleset::RULESET_VERSION, score};
use wasm_bindgen::prelude::*;

pub fn score_json_impl(events_json: &str) -> Result<String, String> {
    let events: Vec<Event> = serde_json::from_str(events_json).map_err(|e| e.to_string())?;
    serde_json::to_string(&score(&events)).map_err(|e| e.to_string())
}

/// -1 if valid, else the failing index.
pub fn verify_chain_json_impl(entries_json: &str, prev_hash: &str) -> Result<i32, String> {
    let entries: Vec<ledger::LedgerEntry> = serde_json::from_str(entries_json).map_err(|e| e.to_string())?;
    Ok(match ledger::verify_chain(&entries, prev_hash) {
        Ok(()) => -1,
        Err(e) => e.index as i32,
    })
}

#[wasm_bindgen(js_name = scoreJson)]
pub fn score_json(events_json: &str) -> Result<String, JsError> {
    score_json_impl(events_json).map_err(|m| JsError::new(&m))
}

#[wasm_bindgen(js_name = verifyChainJson)]
pub fn verify_chain_json(entries_json: &str, prev_hash: &str) -> Result<i32, JsError> {
    verify_chain_json_impl(entries_json, prev_hash).map_err(|m| JsError::new(&m))
}

#[wasm_bindgen(js_name = rulesetVersion)]
pub fn ruleset_version() -> String {
    RULESET_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const PROMPT: &str = r#""kind":"prompt","chars":200,"relevance_pm":600,"novelty_bytes":90"#;

    fn ev(id: &str, ts: i64, kind: &str) -> String {
        format!(r#"{{"v":1,"id":"{id}","ts":{},"source":"t","project":"p","session":null,{kind}}}"#, ts * 1000)
    }

    #[test]
    fn lunch_scenario() {
        let idle = r#""kind":"idle","end_ts":1860000"#;
        let json = format!("[{},{},{}]", ev("1", 0, PROMPT), ev("2", 60, idle), ev("3", 1900, PROMPT));
        let claims: serde_json::Value = serde_json::from_str(&score_json_impl(&json).unwrap()).unwrap();
        let v: Vec<i64> = claims.as_array().unwrap().iter().map(|c| c["verified_s"].as_i64().unwrap()).collect();
        assert_eq!(v, [120, 540]);
    }

    #[test]
    fn bad_input_errs() {
        assert!(score_json_impl("not json").is_err());
        assert!(verify_chain_json_impl("not json", GENESIS).is_err());
    }

    #[test]
    fn chain_verify() {
        assert_eq!(verify_chain_json_impl("[]", GENESIS), Ok(-1));
        let events: Vec<crate::event::Event> =
            serde_json::from_str(&format!("[{},{}]", ev("1", 0, PROMPT), ev("2", 600, PROMPT))).unwrap();
        let mut entries = crate::ledger::chain(&crate::score(&events), GENESIS);
        assert_eq!(verify_chain_json_impl(&serde_json::to_string(&entries).unwrap(), GENESIS), Ok(-1));
        entries[1].verified_s += 1;
        assert_eq!(verify_chain_json_impl(&serde_json::to_string(&entries).unwrap(), GENESIS), Ok(1));
    }
}
