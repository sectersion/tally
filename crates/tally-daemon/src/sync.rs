//! Uploads each published chain from the server's head. Never uploads anything unverified:
//! `sync_all` checks both ledgers first.
use crate::api::{Api, ApiError};
use crate::finalize::{self, FinalizeError};
use crate::published::{self, PublishError};
use crate::store::{Store, StoreError};
use std::collections::BTreeMap;
use tally_scoring::{LedgerEntry, GENESIS_HASH};

pub const BATCH_MAX: usize = 500;
pub const MAX_409_RETRIES: usize = 3;

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
    let (mut uploaded, mut conflicts, mut batch_max) = (0, 0, BATCH_MAX);
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
                Err(ApiError::TooLarge) if batch_max > 1 => batch_max /= 2,
                Err(e) => return Err(e.into()),
            }
        }
        return Ok(ChainReport { project: project.into(), ruleset: ruleset.into(), uploaded, server_count: count });
    }
}

/// `tokens` is keyed by server id (see `credentials::load_tokens`).
pub fn sync_all(store: &mut Store, server: &str, tokens: &BTreeMap<String, String>) -> Vec<Result<ChainReport, SyncError>> {
    let links = match prepare(store) {
        Ok(l) => l,
        Err(e) => return vec![Err(e)],
    };
    let mut out = Vec::new();
    for (name, ids) in links {
        sync_project(store, server, tokens, &name, &ids, &mut out);
    }
    out
}

fn prepare(store: &mut Store) -> Result<BTreeMap<String, Vec<String>>, SyncError> {
    finalize::check_derivation(store)?;
    published::sync(store, true)?;
    Ok(store.server_ids()?)
}

fn sync_project(
    store: &Store,
    server: &str,
    tokens: &BTreeMap<String, String>,
    name: &str,
    ids: &[String],
    out: &mut Vec<Result<ChainReport, SyncError>>,
) {
    let [id] = ids else {
        out.push(Err(SyncError::ConflictingLink { project: name.into() }));
        return;
    };
    let rows = match store.published_rows(name) {
        Ok(r) => r,
        Err(e) => return out.push(Err(e.into())),
    };
    if rows.is_empty() {
        return;
    }
    let Some(token) = tokens.get(id) else {
        return out.push(Err(SyncError::NotLoggedIn { project: name.into() }));
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
        let r = sync_chain(&api, id, name, &ruleset, &entries);
        let stop = matches!(r, Err(SyncError::Api(ApiError::Unauthorized)));
        out.push(r);
        if stop {
            break;
        }
    }
}
