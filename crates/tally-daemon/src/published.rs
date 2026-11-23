//! Per-project public chains derived from the final main ledger. Never reads events or scores.
use crate::status::{verify_ledger, VerifyError};
use crate::store::{PublishedRow, Store, StoreError};
use std::collections::{BTreeMap, HashMap, HashSet};
use tally_scoring::ledger::entry_hash;
use tally_scoring::{verify_chain, LedgerEntry, GENESIS_HASH};

#[derive(thiserror::Error, Debug)]
pub enum PublishError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    MainLedger(#[from] VerifyError),
    #[error("invalid public project name {0:?}")]
    BadName(String),
    #[error("published ledger for {project} is inconsistent with main entry {main_seq}: {what}")]
    Inconsistent { project: String, main_seq: i64, what: &'static str },
    #[error("published chain {project} ({ruleset}) broken at entry {index}: {reason}")]
    PublishedChain { project: String, ruleset: String, index: usize, reason: &'static str },
}

/// `owner/repo`: two non-empty segments of `[A-Za-z0-9._-]`, neither `.` nor `..`.
pub fn valid_public_name(name: &str) -> bool {
    let seg = |s: &str| {
        !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    matches!(name.split_once('/'), Some((a, b)) if seg(a) && seg(b))
}

/// Public name for a main-ledger project: via the registry, or already a public name.
fn resolve<'a>(names: &'a BTreeMap<String, String>, p: &str) -> Option<&'a str> {
    names.get(p).map(String::as_str).or_else(|| names.values().find(|n| *n == p).map(String::as_str))
}

fn valid_names(store: &Store) -> Result<BTreeMap<String, String>, PublishError> {
    let names = store.project_names()?;
    match names.values().find(|n| !valid_public_name(n)) {
        Some(bad) => Err(PublishError::BadName(bad.clone())),
        None => Ok(names),
    }
}

/// Rows to append; writes nothing.
pub fn pending(store: &Store) -> Result<Vec<PublishedRow>, PublishError> {
    let names = valid_names(store)?;
    let mut have: HashMap<String, HashSet<i64>> = HashMap::new();
    let mut last: HashMap<(String, String), String> = HashMap::new();
    let mut out = Vec::new();
    for (seq, e) in store.ledger_rows()? {
        let Some(name) = e.project.as_deref().and_then(|p| resolve(&names, p)) else { continue };
        if !have.contains_key(name) {
            let stored = store.published_rows(name)?;
            for r in &stored {
                last.insert((name.into(), r.entry.ruleset.clone()), r.entry.hash.clone());
            }
            have.insert(name.into(), stored.iter().map(|r| r.main_seq).collect());
        }
        if have[name].contains(&seq) {
            continue;
        }
        let key = (name.to_string(), e.ruleset.clone());
        let mut copy = e;
        copy.project = Some(name.into());
        copy.prev_hash = last.get(&key).cloned().unwrap_or_else(|| GENESIS_HASH.into());
        copy.hash = entry_hash(&copy);
        last.insert(key, copy.hash.clone());
        out.push(PublishedRow { main_seq: seq, entry: copy });
    }
    Ok(out)
}

fn same_claim(a: &LedgerEntry, b: &LedgerEntry) -> bool {
    (&a.action_id, a.ts, &a.kind, a.verified_s, a.unverified_s, &a.reasons, &a.features, &a.ruleset)
        == (&b.action_id, b.ts, &b.kind, b.verified_s, b.unverified_s, &b.reasons, &b.features, &b.ruleset)
}

/// Main chain verifies; every published row matches its main row and maps to its project; each chain verifies alone.
pub fn check(store: &Store) -> Result<(), PublishError> {
    verify_ledger(store)?;
    let names = store.project_names()?;
    let main: HashMap<i64, LedgerEntry> = store.ledger_rows()?.into_iter().collect();
    for project in store.published_projects()? {
        let rows = store.published_rows(&project)?;
        let mut chains: BTreeMap<&str, Vec<LedgerEntry>> = BTreeMap::new();
        for r in &rows {
            let bad = |what| PublishError::Inconsistent { project: project.clone(), main_seq: r.main_seq, what };
            let m = main.get(&r.main_seq).ok_or_else(|| bad("main row missing"))?;
            if !same_claim(m, &r.entry) {
                return Err(bad("main row differs"));
            }
            let mapped = m.project.as_deref().and_then(|p| resolve(&names, p));
            if !valid_public_name(&project)
                || r.entry.project.as_deref() != Some(project.as_str())
                || mapped != Some(project.as_str())
            {
                return Err(bad("project mapping"));
            }
            chains.entry(&r.entry.ruleset).or_default().push(r.entry.clone());
        }
        for (ruleset, entries) in chains {
            verify_chain(&entries, GENESIS_HASH).map_err(|e| PublishError::PublishedChain {
                project: project.clone(),
                ruleset: ruleset.into(),
                index: e.index,
                reason: e.reason,
            })?;
        }
    }
    Ok(())
}

/// Rows appended (or, with `apply = false`, that would be).
pub fn sync(store: &mut Store, apply: bool) -> Result<Vec<PublishedRow>, PublishError> {
    for attempt in 1.. {
        check(store)?;
        let rows = pending(store)?;
        if !apply || rows.is_empty() {
            return Ok(rows);
        }
        match store.append_published(&rows) {
            Ok(()) => return Ok(rows),
            // another process synced first
            Err(StoreError::ChainBroken | StoreError::Sqlite(rusqlite::Error::SqliteFailure(..))) if attempt < 3 => {}
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}

pub fn verified_seconds(rows: &[PublishedRow]) -> u64 {
    rows.iter().map(|r| u64::from(r.entry.verified_s)).sum()
}

pub fn hours_message(verified_s: u64) -> String {
    let tenths = verified_s / 360;
    format!("{}.{} h", tenths / 10, tenths % 10)
}

pub fn badge_json(verified_s: u64) -> String {
    let v = serde_json::json!({
        "schemaVersion": 1, "label": "tracked with Tally", "message": hours_message(verified_s), "color": "blue",
    });
    format!("{}\n", serde_json::to_string(&v).expect("json"))
}

#[cfg(test)]
pub(crate) fn store_with_ledger(name: Option<&str>) -> Store {
    use crate::finalize::finalize;
    use crate::test_support::{edit, prompt};
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
    if let Some(n) = name {
        s.register_project("p1", "C:/r", Some(n)).unwrap();
    }
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::test_support::{edit, prompt};

    fn drop_triggers(s: &Store) {
        let names: Vec<String> = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger'").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for n in names { s.conn.execute(&format!("DROP TRIGGER {n}"), []).unwrap(); }
    }

    #[test] fn unnamed_projects_publish_nothing() {
        let mut s = store_with_ledger(None);
        assert!(sync(&mut s, true).unwrap().is_empty());
        assert!(s.published_projects().unwrap().is_empty());
    }
    #[test] fn a_named_project_gets_its_own_chain_that_verifies_alone() {
        let mut s = store_with_ledger(Some("owner/repo"));
        assert_eq!(sync(&mut s, true).unwrap().len(), 2);
        let entries: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
        assert!(entries.iter().all(|e| e.project.as_deref() == Some("owner/repo")));
        assert_eq!(entries[0].prev_hash, GENESIS_HASH);
        assert!(tally_scoring::verify_chain(&entries, GENESIS_HASH).is_ok());
        assert!(sync(&mut s, true).unwrap().is_empty());             // idempotent
    }
    #[test] fn other_projects_in_the_main_ledger_never_appear() {
        let mut s = Store::open_in_memory().unwrap();
        let mut other = edit("b", 100_000); other.project = Some("p2".into());
        s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
        s.register_project("p2", "C:/q", None).unwrap();
        s.append_events(&[prompt("a", 0, 600), other]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        sync(&mut s, true).unwrap();
        let all: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
        assert_eq!(all.len(), 1);
        assert!(!serde_json::to_string(&all).unwrap().contains("p2"));
    }
    #[test] fn a_name_registered_after_finalizing_still_publishes_the_older_claims() {
        let mut s = store_with_ledger(None);
        s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
        let rows = sync(&mut s, true).unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].entry.hash, s.ledger().unwrap()[0].hash);   // rechained, not copied
        let entries: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
        assert!(tally_scoring::verify_chain(&entries, GENESIS_HASH).is_ok());
        assert_eq!(rows[1].entry.prev_hash, rows[0].entry.hash);
    }
    #[test] fn a_second_local_id_with_the_same_name_joins_the_same_chain() {
        let mut s = store_with_ledger(Some("owner/repo"));
        sync(&mut s, true).unwrap();
        let mut e = edit("z", 2_000_000); e.project = Some("p9".into());
        s.register_project("p9", "C:/w", Some("owner/repo")).unwrap();
        s.append_events(&[e]).unwrap();
        s.set_watermark("src", 9_000_000).unwrap();
        finalize(&mut s).unwrap();
        assert_eq!(sync(&mut s, true).unwrap().len(), 1);
        let entries: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
        assert_eq!(entries.len(), 3);
        assert!(tally_scoring::verify_chain(&entries, GENESIS_HASH).is_ok());
    }
    #[test] fn dry_run_writes_nothing() {
        let mut s = store_with_ledger(Some("owner/repo"));
        assert_eq!(sync(&mut s, false).unwrap().len(), 2);
        assert!(s.published_projects().unwrap().is_empty());
    }
    #[test] fn a_new_ruleset_starts_a_new_published_chain() {
        let mut s = Store::open_in_memory().unwrap();
        s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
        let mut prev = GENESIS_HASH.to_string();
        let mut entries = Vec::new();
        for (i, rs) in ["r1", "r2"].into_iter().enumerate() {
            let mut e = LedgerEntry {
                v: 1, ruleset: rs.into(), action_id: format!("a{i}"), ts: i as i64, project: Some("owner/repo".into()),
                kind: "prompt".into(), verified_s: 10, unverified_s: 0, reasons: vec![], features: Default::default(),
                prev_hash: prev.clone(), hash: String::new(),
            };
            e.hash = entry_hash(&e);
            prev = e.hash.clone();
            entries.push(e);
        }
        s.append_ledger(&entries, 0, 0, &[]).unwrap();
        let rows = sync(&mut s, true).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.entry.prev_hash == GENESIS_HASH));
        for rs in ["r1", "r2"] {
            let chain: Vec<_> = rows.iter().filter(|r| r.entry.ruleset == rs).map(|r| r.entry.clone()).collect();
            assert_eq!(chain.len(), 1);
            assert!(tally_scoring::verify_chain(&chain, GENESIS_HASH).is_ok());
        }
        assert_eq!(s.published_rows("owner/repo").unwrap().len(), 2);
    }
    #[test] fn a_tampered_main_or_published_ledger_stops_everything() {
        let mut s = store_with_ledger(Some("owner/repo"));
        drop_triggers(&s);
        s.conn.execute("UPDATE ledger SET entry_json = replace(entry_json, '\"verified_s\":100', '\"verified_s\":900') WHERE seq = 2", []).unwrap();
        assert!(matches!(sync(&mut s, true), Err(PublishError::MainLedger(_))));
        assert!(s.published_rows("owner/repo").unwrap().is_empty());

        let mut s = store_with_ledger(Some("owner/repo"));
        sync(&mut s, true).unwrap();
        let before = s.published_rows("owner/repo").unwrap();
        drop_triggers(&s);
        let n = s.conn.execute("UPDATE published_ledger SET entry_json = replace(entry_json, '\"verified_s\":100', '\"verified_s\":900') WHERE main_seq = 2", []).unwrap();
        assert_eq!(n, 1);
        assert!(matches!(sync(&mut s, true),
            Err(PublishError::PublishedChain { reason: "bad_hash", .. } | PublishError::Inconsistent { .. })));
        let after = s.published_rows("owner/repo").unwrap();
        assert_eq!(after.len(), before.len());   // nothing appended
        assert_eq!(after[0], before[0]);
    }
    #[test] fn bad_public_names_are_rejected() {
        for bad in ["", "owner", "a/b/c", "../x", "o/..", "o/r r", "o/r?x"] { assert!(!valid_public_name(bad), "{bad}"); }
        assert!(valid_public_name("sectersion/tally"));
        let mut s = store_with_ledger(None);
        s.register_project("p1", "C:/r", Some("o/r r")).unwrap();
        assert!(matches!(sync(&mut s, true), Err(PublishError::BadName(n)) if n == "o/r r"));
        assert!(s.published_projects().unwrap().is_empty());
    }
    #[test] fn badge_hours_floor_to_one_decimal() {
        assert_eq!(hours_message(0), "0.0 h");
        assert_eq!(hours_message(359), "0.0 h");
        assert_eq!(hours_message(360), "0.1 h");
        assert_eq!(hours_message(719), "0.1 h");
        assert_eq!(hours_message(720), "0.2 h");
        assert_eq!(hours_message(3600), "1.0 h");
        assert_eq!(hours_message(45_000), "12.5 h");
        assert_eq!(badge_json(660),
            "{\"color\":\"blue\",\"label\":\"tracked with Tally\",\"message\":\"0.1 h\",\"schemaVersion\":1}\n");
    }
}
