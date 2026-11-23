//! Pure request router for the local dashboard: `(method, path, query, Host)` to a JSON response, no sockets.
//! Named `dash_api` because `api.rs` is the server ledger client. Read-only; see `docs/superpowers/plans/2026-11-23-11-local-dashboard.md`.
use crate::analytics::{self, Hours};
use crate::replay;
use crate::snapshot::Snapshot;
use crate::source::now_ms;
use crate::store::{Store, StoreError};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const DAY_MS: i64 = 86_400_000;
const MAX_WINDOW_MS: i64 = 366 * DAY_MS;
const DEFAULT_WINDOW_MS: i64 = 28 * DAY_MS;

pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub host: Option<&'a str>,
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub no_store: bool,
    pub allow: Option<&'static str>,
}

impl Response {
    pub fn json(status: u16, v: &impl Serialize) -> Response {
        Response { status, content_type: "application/json", body: serde_json::to_vec(v).unwrap_or_default(), no_store: true, allow: None }
    }
    fn err(status: u16, msg: &str) -> Response {
        Response::json(status, &json!({ "error": msg }))
    }
    /// Every header the server adds. Never any `Access-Control-*`.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut h = vec![
            ("Content-Type", self.content_type.to_string()),
            ("Content-Security-Policy", CSP.to_string()),
            ("X-Content-Type-Options", "nosniff".to_string()),
            ("Referrer-Policy", "no-referrer".to_string()),
        ];
        if self.no_store {
            h.push(("Cache-Control", "no-store".to_string()));
        }
        if let Some(a) = self.allow {
            h.push(("Allow", a.to_string()));
        }
        h
    }
}

pub struct DashApi {
    db: PathBuf,
    port: u16,
}

#[derive(Serialize)]
struct AsOf {
    horizon: Option<i64>,
    finalized_through: Option<i64>,
}

/// A local id or public name to the local ids it stands for (a name may map to several).
pub fn project_key_to_ids(names: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    let mut ids: Vec<String> = names.iter().filter(|(_, n)| n.as_str() == key).map(|(id, _)| id.clone()).collect();
    if ids.is_empty() {
        ids.push(key.to_string());
    }
    ids
}

/// Public name, or the first 8 characters of a 64-hex local id.
fn label(names: &BTreeMap<String, String>, p: &str) -> String {
    match names.get(p) {
        Some(n) => n.clone(),
        None if p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()) => p[..8].to_string(),
        None => p.to_string(),
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Query<'a>(Vec<(&'a str, &'a str)>);

impl<'a> Query<'a> {
    fn new(q: &'a str) -> Self {
        Query(q.split('&').filter(|s| !s.is_empty()).map(|kv| kv.split_once('=').unwrap_or((kv, ""))).collect())
    }
    fn get(&self, k: &str) -> Option<&'a str> {
        self.0.iter().find(|(key, _)| *key == k).map(|(_, v)| *v)
    }
    fn int(&self, k: &str) -> Result<Option<i64>, String> {
        self.get(k).map(|v| v.parse::<i64>().map_err(|_| format!("{k} must be an integer"))).transpose()
    }
    fn window(&self) -> Result<(i64, i64), String> {
        let to = self.int("to")?.unwrap_or_else(now_ms);
        let from = self.int("from")?.unwrap_or(to - DEFAULT_WINDOW_MS);
        if from >= to {
            return Err("from must be less than to".into());
        }
        if to - from > MAX_WINDOW_MS {
            return Err("window (from, to) is longer than 366 days".into());
        }
        Ok((from, to))
    }
    fn edges(&self) -> Result<Vec<i64>, String> {
        let raw = self.get("edges").ok_or("missing edges")?;
        let e: Vec<i64> = raw.split(',').map(|x| x.parse::<i64>().map_err(|_| "edges must be comma separated integers".to_string())).collect::<Result<_, _>>()?;
        if !(2..=367).contains(&e.len()) {
            return Err("edges must have 2 to 367 entries".into());
        }
        if e.windows(2).any(|w| w[0] >= w[1]) {
            return Err("edges must be strictly ascending".into());
        }
        if e[e.len() - 1] - e[0] > MAX_WINDOW_MS + 3_600_000 {
            return Err("edges span more than 366 days".into());
        }
        Ok(e)
    }
}

impl DashApi {
    pub fn new(db: PathBuf, port: u16) -> DashApi {
        DashApi { db, port }
    }

    fn host_ok(&self, host: Option<&str>) -> bool {
        let Some(h) = host else { return false };
        h == format!("127.0.0.1:{}", self.port) || h.eq_ignore_ascii_case(&format!("localhost:{}", self.port))
    }

    pub fn handle(&self, req: &Request) -> Response {
        if !self.host_ok(req.host) {
            return Response::err(403, "forbidden host");
        }
        if req.method != "GET" && req.method != "HEAD" {
            return Response { allow: Some("GET, HEAD"), ..Response::err(405, "method not allowed") };
        }
        let Some(route) = req.path.strip_prefix("/api/") else { return Response::err(404, "not found") };
        if !["status", "bugs", "prompts", "hours", "sessions", "replay"].contains(&route) {
            return Response::err(404, "not found");
        }
        let q = Query::new(req.query);
        match self.route(route, &q) {
            Ok(r) => r,
            Err(Fail::Bad(m)) => Response::err(400, &m),
            Err(Fail::Store(StoreError::Sqlite(rusqlite::Error::SqliteFailure(e, _)))) if e.code == rusqlite::ErrorCode::CannotOpen => {
                Response::err(503, "no database yet")
            }
            Err(Fail::Store(e)) => {
                eprintln!("dashboard: {e}");
                Response::err(500, "internal")
            }
        }
    }

    fn route(&self, route: &str, q: &Query) -> Result<Response, Fail> {
        // validate before touching the database
        let window = match route {
            "bugs" | "prompts" | "sessions" | "replay" => Some(q.window()?),
            _ => None,
        };
        let edges = if route == "hours" { Some(q.edges()?) } else { None };
        let limit = q.int("limit")?;
        let mut store = Store::open_read_only(&self.db)?;
        let snap = store.snapshot()?;
        let names = snap.project_names()?;
        let lab = |p: &mut Option<String>| *p = p.as_deref().map(|x| label(&names, x));
        let (w_from, w_to) = window.unwrap_or((0, 0));
        let data = match route {
            "status" => status_data(&snap, &names)?,
            "bugs" => {
                let mut b = analytics::bugs(&snap, w_from, w_to)?;
                b.fixes.iter_mut().for_each(|f| lab(&mut f.project));
                json!(b)
            }
            "prompts" => json!(analytics::prompts_after_failure_for(&snap, w_from, w_to)?),
            "hours" => {
                let edges = edges.unwrap_or_default();
                let mut h: Hours = analytics::hours_for(&snap, &edges)?;
                h.days.iter_mut().for_each(|d| lab(&mut d.project));
                let mut v = json!(h);
                v["edges"] = json!(edges);
                v
            }
            "sessions" => {
                let mut s = replay::sessions_for(&snap, w_from, w_to, limit.map_or(100, |l| l.max(1) as usize))?;
                s.iter_mut().for_each(|x| lab(&mut x.project));
                json!(s)
            }
            _ => {
                let projects: Vec<String> = q.get("project").map(|k| project_key_to_ids(&names, &pct_decode(k))).unwrap_or_default();
                let after = match q.get("after").map(pct_decode) {
                    None => None,
                    Some(a) => {
                        let (ts, id) = a.split_once(':').ok_or_else(|| Fail::Bad("after must be <ts>:<id>".into()))?;
                        Some((ts.parse::<i64>().map_err(|_| Fail::Bad("after must be <ts>:<id>".into()))?, id.to_string()))
                    }
                };
                let mut p = replay::replay_page(&snap, &projects, w_from, w_to, after, limit.map_or(500, |l| l.max(1) as usize))?;
                p.items.iter_mut().for_each(|i| lab(&mut i.project));
                json!(p)
            }
        };
        let fin = snap.finalized_through()?;
        let as_of = AsOf { horizon: snap.horizon()?, finalized_through: Some(fin).filter(|&t| t != i64::MIN) };
        Ok(Response::json(200, &json!({ "as_of": as_of, "data": data })))
    }
}

fn status_data(snap: &Snapshot, names: &BTreeMap<String, String>) -> Result<serde_json::Value, StoreError> {
    let (events, final_claims, verified_s, unverified_s, skipped, provisional) = snap.counts()?;
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for id in snap.project_ids()? {
        keys.insert(names.get(&id).cloned().unwrap_or_else(|| id.clone()), label(names, &id));
    }
    for p in snap.ledger_projects()? {
        let l = label(names, &p);
        keys.entry(p).or_insert(l);
    }
    Ok(json!({
        "events": events, "final_claims": final_claims, "provisional_claims": provisional,
        "verified_s": verified_s, "unverified_s": unverified_s, "skipped_claims": skipped,
        "sources": snap.watermarks()?.into_iter().map(|(source, watermark)| json!({"source": source, "watermark": watermark})).collect::<Vec<_>>(),
        "projects": keys.into_iter().map(|(key, label)| json!({"key": key, "label": label})).collect::<Vec<_>>(),
    }))
}

enum Fail {
    Bad(String),
    Store(StoreError),
}
impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Bad(s)
    }
}
impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Fail::Bad(s.into())
    }
}
impl From<StoreError> for Fail {
    fn from(e: StoreError) -> Self {
        Fail::Store(e)
    }
}

#[cfg(test)]
#[path = "dash_api_tests.rs"]
mod tests;
