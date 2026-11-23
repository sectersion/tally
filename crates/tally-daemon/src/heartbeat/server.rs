//! Blocking localhost-only WakaTime-compatible heartbeat server. Pushes parsed heartbeats
//! into a shared sink and does nothing else (no file I/O, no store access).
use super::{check_auth, parse_body, Heartbeat};
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const WORKERS: usize = 8;

/// Shared between the server threads and the source.
#[derive(Debug, Default)]
pub struct Counters {
    pub malformed: AtomicUsize,
    pub overflow: AtomicUsize,
}

pub struct ServerConfig {
    /// 0 = ephemeral, for tests.
    pub port: u16,
    pub token: String,
    pub max_body: usize,
    pub max_bulk: usize,
    pub read_timeout: Duration,
    pub counters: Arc<Counters>,
}

impl ServerConfig {
    pub fn new(port: u16, token: String) -> Self {
        Self { port, token, max_body: 262_144, max_bulk: 25, read_timeout: Duration::from_secs(10), counters: Arc::default() }
    }
}

#[derive(Debug, Clone)]
pub struct Received {
    pub hb: Heartbeat,
    pub received_ms: i64,
}

pub struct Running {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// 127.0.0.1, localhost, [::1], each with an optional `:port`.
pub fn host_ok(host: Option<&str>) -> bool {
    let Some(h) = host else { return false };
    let h = h.trim().to_ascii_lowercase();
    let name = if let Some(rest) = h.strip_prefix('[') {
        match rest.split_once(']') {
            Some((a, tail)) if tail.is_empty() || tail.starts_with(':') => format!("[{a}]"),
            _ => return false,
        }
    } else {
        h.split(':').next().unwrap_or("").to_string()
    };
    matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/// Reads the key from `token_file`, creating a random 32-byte hex one (0600 on unix) on first use.
pub fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if let Ok(t) = std::fs::read_to_string(path) {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| std::io::Error::other(e.to_string()))?;
    let t = hex::encode(b);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(path)?, t.as_bytes())?;
    Ok(t)
}

pub fn start(cfg: ServerConfig, sink: Arc<Mutex<Vec<Received>>>) -> std::io::Result<Running> {
    let server = Server::http(("127.0.0.1", cfg.port)).map_err(|e| std::io::Error::other(e.to_string()))?;
    let addr = server.server_addr().to_ip().ok_or_else(|| std::io::Error::other("not an ip listener"))?;
    let (server, cfg, stop) = (Arc::new(server), Arc::new(cfg), Arc::new(AtomicBool::new(false)));
    let threads = (0..WORKERS)
        .map(|_| {
            let (server, cfg, sink, stop) = (server.clone(), cfg.clone(), sink.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    if let Ok(Some(req)) = server.recv_timeout(Duration::from_millis(100)) {
                        handle(req, &cfg, &sink);
                    }
                }
            })
        })
        .collect();
    Ok(Running { addr, stop, threads })
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn reply(req: Request, code: u16, body: Option<serde_json::Value>) {
    let _ = match body {
        Some(v) => {
            let h = Header::from_bytes("Content-Type", "application/json").unwrap();
            req.respond(Response::from_string(v.to_string()).with_status_code(code).with_header(h))
        }
        None => req.respond(Response::empty(code)),
    };
}

fn ack(hb: &Heartbeat, received_ms: i64) -> serde_json::Value {
    serde_json::json!({"data": {"id": format!("{received_ms}"), "entity": hb.entity, "type": hb.kind, "time": hb.time}})
}

fn header(req: &Request, name: &'static str) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string())
}

fn handle(req: Request, cfg: &ServerConfig, sink: &Mutex<Vec<Received>>) {
    if !host_ok(header(&req, "Host").as_deref()) {
        return reply(req, 403, None);
    }
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let route = path.strip_prefix("/api/hackatime").or_else(|| path.strip_prefix("/api")).unwrap_or(path);
    let bulk = match route {
        "/v1/users/current/heartbeats" => false,
        "/v1/users/current/heartbeats.bulk" => true,
        _ => return reply(req, 404, None),
    };
    if *req.method() != Method::Post {
        return reply(req, 405, None);
    }
    let qkey = query.split('&').find_map(|kv| kv.strip_prefix("api_key="));
    if !check_auth(header(&req, "Authorization").as_deref(), qkey, &cfg.token) {
        return reply(req, 401, None);
    }
    if req.body_length().is_some_and(|n| n > cfg.max_body) {
        return reply(req, 413, None);
    }
    // tiny_http has no socket timeout: read on a helper thread and give up on a stalled client.
    // ponytail: a stalled reader thread lingers until the client disconnects; fine for localhost.
    let (tx, rx) = std::sync::mpsc::channel();
    let max = cfg.max_body as u64 + 1;
    let mut req = req;
    std::thread::spawn(move || {
        let mut body = Vec::new();
        let r = req.as_reader().take(max).read_to_end(&mut body).map(|_| body);
        let _ = tx.send((req, r));
    });
    let (req, body) = match rx.recv_timeout(cfg.read_timeout) {
        Ok((req, Ok(body))) => (req, body),
        Ok((req, Err(_))) => return reply(req, 400, None),
        Err(_) => return, // timed out: the helper owns the request and drops it
    };
    if body.len() > cfg.max_body {
        return reply(req, 413, None);
    }
    let bad = || cfg.counters.malformed.fetch_add(1, Ordering::Relaxed);
    let Ok(items) = parse_body(&body) else {
        bad();
        return reply(req, 400, None);
    };
    if items.len() > cfg.max_bulk || (!bulk && items.len() != 1) {
        bad();
        return reply(req, 400, None);
    }
    let received_ms = now_ms();
    let mut pairs = Vec::new();
    for it in items {
        match it {
            Ok(hb) => {
                pairs.push(serde_json::json!([ack(&hb, received_ms), 201]));
                let mut g = sink.lock().unwrap_or_else(|e| e.into_inner());
                if g.len() >= super::SINK_CAP {
                    cfg.counters.overflow.fetch_add(1, Ordering::Relaxed);
                } else {
                    g.push(Received { hb, received_ms });
                }
            }
            Err(e) => {
                bad();
                pairs.push(serde_json::json!([{"error": e}, 400]));
            }
        }
    }
    if bulk {
        reply(req, 201, Some(serde_json::json!({"responses": pairs})));
    } else {
        let (v, code) = (pairs[0][0].clone(), pairs[0][1].as_u64().unwrap_or(400) as u16);
        reply(req, code, Some(v));
    }
}
