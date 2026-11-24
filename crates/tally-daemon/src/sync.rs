//! Uploads each published chain from the server's head. Never uploads anything unverified:
//! `sync_all` checks both ledgers first.
use crate::api::{Api, ApiError};
use crate::finalize::{self, FinalizeError};
use crate::published::{self, PublishError};
use crate::store::{PublishedRow, Store, StoreError};
use crate::credentials::load_tokens;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tally_scoring::ruleset::COMMIT_INTERVAL_MS;
use tally_scoring::{LedgerEntry, GENESIS_HASH, RULESET_VERSION};

pub const BATCH_MAX: usize = 500;
pub const MAX_409_RETRIES: usize = 3;
const MAX_429_RETRIES: usize = 10;

#[derive(thiserror::Error, Debug)]
pub enum SyncError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("the server's chain for {project} has a head this device never wrote; another device may be syncing it")]
    ForeignHead { project: String, ruleset: String, server_head: String, server_count: u64 },
    #[error("two local projects named {project} are linked to different server ids")]
    ConflictingLink { project: String },
    #[error("the server's head for {project} kept changing; will retry next run")]
    TooManyConflicts { project: String },
    #[error("no device token for {project}; run tally login --server-id <id>")]
    NotLoggedIn { project: String },
    #[error(transparent)]
    Derivation(#[from] FinalizeError),
    #[error(transparent)]
    Published(#[from] PublishError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainReport {
    pub project: String,
    pub ruleset: String,
    pub uploaded: usize,
    pub server_count: u64,
}

pub fn sync_chain(api: &Api, server_id: &str, project: &str, ruleset: &str, local: &[LedgerEntry]) -> Result<ChainReport, SyncError> {
    chain(api, server_id, project, ruleset, local, false)
}

/// `dry`: stop after the head check and report how many entries are left to upload.
fn chain(api: &Api, server_id: &str, project: &str, ruleset: &str, local: &[LedgerEntry], dry: bool) -> Result<ChainReport, SyncError> {
    let (mut uploaded, mut conflicts, mut limited, mut batch_max) = (0, 0, 0, BATCH_MAX);
    'refetch: loop {
        let head = api.head(server_id, ruleset)?;
        let foreign = |hash: &str, count: u64| SyncError::ForeignHead {
            project: project.into(),
            ruleset: ruleset.into(),
            server_head: hash.into(),
            server_count: count,
        };
        let mut pos = if head.head_hash == GENESIS_HASH {
            0
        } else {
            match local.iter().position(|e| e.hash == head.head_hash) {
                Some(i) => i + 1,
                None => return Err(foreign(&head.head_hash, head.count)),
            }
        };
        if pos as u64 != head.count {
            return Err(foreign(&head.head_hash, head.count));
        }
        if dry {
            let left = local.len() - pos;
            return Ok(ChainReport { project: project.into(), ruleset: ruleset.into(), uploaded: left, server_count: head.count });
        }
        let mut count = head.count;
        while pos < local.len() {
            let batch = &local[pos..local.len().min(pos + batch_max)];
            let prev = if pos == 0 { GENESIS_HASH } else { local[pos - 1].hash.as_str() };
            match api.upload(server_id, prev, batch) {
                Ok(h) => {
                    if h.head_hash != batch[batch.len() - 1].hash {
                        return Err(foreign(&h.head_hash, h.count));
                    }
                    pos += batch.len();
                    uploaded += batch.len();
                    count = h.count;
                }
                Err(ApiError::Conflict { .. }) => {
                    conflicts += 1;
                    if conflicts > MAX_409_RETRIES {
                        return Err(SyncError::TooManyConflicts { project: project.into() });
                    }
                    continue 'refetch;
                }
                Err(ApiError::RateLimited(secs)) if limited < MAX_429_RETRIES => {
                    limited += 1;
                    std::thread::sleep(std::time::Duration::from_secs(secs.min(30)));
                }
                Err(ApiError::TooLarge) if batch_max > 1 => batch_max /= 2,
                Err(e) => return Err(e.into()),
            }
        }
        return Ok(ChainReport { project: project.into(), ruleset: ruleset.into(), uploaded, server_count: count });
    }
}

/// A result labelled with its public project name (`"ledger"` for a whole-store failure).
pub type Labeled<T> = (String, Result<T, SyncError>);

/// `tokens` is keyed by server id (see `credentials::load_tokens`).
pub fn sync_all(store: &mut Store, server: &str, tokens: &BTreeMap<String, String>) -> Vec<Labeled<ChainReport>> {
    sync_once(store, server, tokens, &mut BTreeMap::new(), None).1
}

/// Verifies and publishes once, then (with `commit_at`) `commit_heads`, then uploads every linked
/// project. The commit goes first: it is small and anchors the head even if the upload then fails.
/// A verification failure stops both, labelled `"ledger"`, in the upload results.
pub fn sync_once(
    store: &mut Store,
    server: &str,
    tokens: &BTreeMap<String, String>,
    committed: &mut BTreeMap<String, String>,
    commit_at: Option<i64>,
) -> (Vec<Labeled<()>>, Vec<Labeled<ChainReport>>) {
    let links = match prepare(store) {
        Ok(l) => l,
        Err(e) => return (vec![], vec![("ledger".into(), Err(e))]),
    };
    let commits = commit_at.map_or_else(Vec::new, |now| commit_heads(store, server, tokens, committed, now));
    let mut out = Vec::new();
    // A server that is down fails every project the same way, slowly: stop at the first such error.
    if !commits.iter().any(|(_, r)| is_down(r)) {
        for (name, ids) in links {
            sync_project(store, server, tokens, &name, &ids, (&[], false), &mut out);
            if out.iter().any(|(_, r)| is_down(r)) {
                break;
            }
        }
    }
    (commits, out)
}

/// A network or 5xx failure: the server is unreachable, so trying more projects only costs timeouts.
fn is_down<T>(r: &Result<T, SyncError>) -> bool {
    matches!(r, Err(SyncError::Api(a)) if a.retryable())
}

/// What `sync_all` would upload, from one `GET /head` per chain; stores and uploads nothing.
/// `ChainReport::uploaded` is the count still to upload.
pub fn dry_run(store: &mut Store, server: &str, tokens: &BTreeMap<String, String>) -> Vec<Labeled<ChainReport>> {
    let prepared = ruleset_ok(finalize::check_derivation(store))
        .and_then(|_| Ok(published::sync(store, false)?))
        .and_then(|pending| Ok((pending, store.server_ids()?)));
    let (pending, links) = match prepared {
        Ok(p) => p,
        Err(e) => return vec![("ledger".into(), Err(e))],
    };
    let mut out = Vec::new();
    for (name, ids) in links {
        sync_project(store, server, tokens, &name, &ids, (&pending, true), &mut out);
    }
    out
}

/// A ruleset bump is not a divergence: the old chain's tail must still upload (as in `repair::diagnose`).
fn ruleset_ok(r: Result<(), finalize::FinalizeError>) -> Result<(), SyncError> {
    match r {
        Err(finalize::FinalizeError::RulesetChanged { .. }) => Ok(()),
        r => Ok(r?),
    }
}

fn prepare(store: &mut Store) -> Result<BTreeMap<String, Vec<String>>, SyncError> {
    ruleset_ok(finalize::check_derivation(store))?;
    published::sync(store, true)?;
    Ok(store.server_ids()?)
}

fn sync_project(
    store: &Store,
    server: &str,
    tokens: &BTreeMap<String, String>,
    name: &str,
    ids: &[String],
    (extra, dry): (&[PublishedRow], bool),
    out: &mut Vec<Labeled<ChainReport>>,
) {
    let mut push = |r: Result<ChainReport, SyncError>| out.push((name.to_string(), r));
    let [id] = ids else {
        return push(Err(SyncError::ConflictingLink { project: name.into() }));
    };
    let mut rows = match store.published_rows(name) {
        Ok(r) => r,
        Err(e) => return push(Err(e.into())),
    };
    rows.extend(extra.iter().filter(|r| r.entry.project.as_deref() == Some(name)).cloned());
    if rows.is_empty() {
        return;
    }
    let Some(token) = tokens.get(id) else {
        return push(Err(SyncError::NotLoggedIn { project: name.into() }));
    };
    let api = Api::new(server, token);
    // Rulesets in order of first appearance: oldest first.
    let mut chains: Vec<(String, Vec<LedgerEntry>)> = Vec::new();
    for r in rows {
        match chains.iter_mut().find(|c| c.0 == r.entry.ruleset) {
            Some(c) => c.1.push(r.entry),
            None => chains.push((r.entry.ruleset.clone(), vec![r.entry])),
        }
    }
    for (ruleset, entries) in chains {
        let r = chain(&api, id, name, &ruleset, &entries, dry);
        let stop = matches!(r, Err(SyncError::Api(ApiError::Unauthorized))) || is_down(&r);
        push(r);
        if stop {
            break;
        }
    }
}

pub const BACKOFF_BASE_MS: i64 = 30_000;
pub const BACKOFF_MAX_MS: i64 = 3_600_000;

/// In memory only: a restart tries at once, which is what a user who just fixed their network expects.
#[derive(Debug)]
pub struct Backoff {
    failures: u32,
    next_ms: i64,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff { failures: 0, next_ms: i64::MIN }
    }
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }
    /// Earliest time the next attempt may run.
    pub fn next_ms(&self) -> i64 {
        self.next_ms
    }
    pub fn ready(&self, now_ms: i64) -> bool {
        now_ms >= self.next_ms
    }
    pub fn failed(&mut self, now_ms: i64) {
        self.failures = self.failures.saturating_add(1);
        let wait = BACKOFF_BASE_MS.saturating_mul(2i64.saturating_pow(self.failures - 1)).min(BACKOFF_MAX_MS);
        self.next_ms = now_ms.saturating_add(wait);
    }
    pub fn unauthorized(&mut self, now_ms: i64) {
        self.next_ms = now_ms.saturating_add(BACKOFF_MAX_MS);
    }
    pub fn ok(&mut self) {
        *self = Self::new();
    }
}

/// Posts the head of each linked project's current-ruleset chain when it differs from `last[server_id]`
/// (an unchanged head is not re-posted). Projects without a token or with conflicting links are
/// skipped here; `sync_all` reports them.
pub fn commit_heads(
    store: &Store,
    server: &str,
    tokens: &BTreeMap<String, String>,
    last: &mut BTreeMap<String, String>,
    now_ms: i64,
) -> Vec<Labeled<()>> {
    let links = match store.server_ids() {
        Ok(l) => l,
        Err(e) => return vec![("ledger".into(), Err(e.into()))],
    };
    let mut out = Vec::new();
    for (name, ids) in links {
        let ([id], Some(token)) = (&ids[..], ids.first().and_then(|id| tokens.get(id))) else { continue };
        let head = match store.published_rows(&name) {
            Ok(rows) => rows.into_iter().rev().find(|r| r.entry.ruleset == RULESET_VERSION).map(|r| r.entry.hash),
            Err(e) => {
                out.push((name, Err(e.into())));
                continue;
            }
        };
        let Some(head) = head.filter(|h| last.get(id) != Some(h)) else { continue };
        let r = Api::new(server, token).commit(id, &head, now_ms);
        if r.is_ok() {
            last.insert(id.clone(), head);
        }
        let r = r.map_err(SyncError::from);
        let down = is_down(&r);
        out.push((name, r));
        if down {
            break;
        }
    }
    out
}

/// The daemon's sync: every `COMMIT_INTERVAL_MS`, unless backing off.
#[derive(Default)]
pub struct SyncTicker {
    backoff: Backoff,
    last_tick_ms: Option<i64>,
    committed: BTreeMap<String, String>,
}

impl SyncTicker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn backoff(&self) -> &Backoff {
        &self.backoff
    }

    /// Returns whether the tick was due (and so ran). Errors go to stderr; never fails.
    pub fn tick(&mut self, store: &mut Store, server: &str, home: &Path, now_ms: i64) -> bool {
        let interval_due = match self.last_tick_ms {
            None => true,
            Some(t) => now_ms.saturating_sub(t) >= COMMIT_INTERVAL_MS,
        };
        if !interval_due || !self.backoff.ready(now_ms) {
            return false;
        }
        self.last_tick_ms = Some(now_ms);
        // Read each tick: picks up a `tally login` without a restart.
        let tokens = match load_tokens(home) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("sync: reading device tokens: {e}");
                return true;
            }
        };
        let (mut results, uploads) = sync_once(store, server, &tokens, &mut self.committed, Some(now_ms));
        results.extend(uploads.into_iter().map(|(p, r)| (p, r.map(|_| ()))));
        let (mut retry, mut unauth, mut lines) = (false, BTreeSet::new(), BTreeSet::new());
        for (p, r) in results {
            match r {
                Ok(()) => {}
                Err(SyncError::Api(ApiError::Unauthorized)) => {
                    unauth.insert(p);
                }
                Err(e) => {
                    retry |= matches!(&e, SyncError::Api(a) if a.retryable());
                    lines.insert(format!("sync: {p}: {e}"));
                }
            }
        }
        let ids = store.server_ids().unwrap_or_default();
        for p in &unauth {
            let id = ids.get(p).and_then(|v| v.first()).map_or("<id>", String::as_str);
            eprintln!("sync: {p}: device token rejected; run tally login --server-id {id}");
        }
        for l in &lines {
            eprintln!("{l}");
        }
        if !unauth.is_empty() {
            self.backoff.unauthorized(now_ms);
        } else if retry {
            self.backoff.failed(now_ms);
        } else {
            // permanent errors (no token, foreign head, ...) do not count as an outage
            self.backoff.ok();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_caps_at_an_hour_and_resets() {
        let mut b = Backoff::new();
        assert!(b.ready(0));
        let mut waits = vec![];
        let mut now = 0;
        for _ in 0..10 {
            b.failed(now);
            waits.push(b.next_ms - now);
            now = b.next_ms;
        }
        assert_eq!(&waits[..4], [30_000, 60_000, 120_000, 240_000]);
        assert_eq!(*waits.last().unwrap(), BACKOFF_MAX_MS);
        for _ in 0..100 {
            b.failed(now); // no overflow after days offline
        }
        b.ok();
        assert!(b.ready(now));
        b.unauthorized(now);
        assert_eq!(b.next_ms, now + BACKOFF_MAX_MS);
    }
}
