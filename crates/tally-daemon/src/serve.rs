//! Synchronous localhost HTTP server for the dashboard. Binds 127.0.0.1 only (no way to change that), two workers.
//! Read-only: requests never read a body. ponytail: tiny_http has no socket read timeout; since no handler reads a
//! body, a stalled client holds nothing but its own connection (the worker answers the headers it has).
use crate::assets;
use crate::dash_api::{DashApi, Request, Response};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tiny_http::{Header, Method, Response as HttpResponse, Server};

pub struct DashboardServer {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl DashboardServer {
    /// `port` 0 picks a free port; `addr.port()` is the real one (the Host allowlist uses it).
    pub fn start(db: PathBuf, port: u16) -> io::Result<DashboardServer> {
        let server = Arc::new(Server::http(("127.0.0.1", port)).map_err(|e| io::Error::new(io::ErrorKind::AddrInUse, e.to_string()))?);
        let addr = server.server_addr().to_ip().ok_or_else(|| io::Error::other("not an ip socket"))?;
        let api = Arc::new(DashApi::new(db, addr.port()));
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..2)
            .map(|_| {
                let (server, api, stop) = (server.clone(), api.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        if let Ok(Some(req)) = server.recv_timeout(Duration::from_millis(100)) {
                            respond(req, &api);
                        }
                    }
                })
            })
            .collect();
        Ok(DashboardServer { addr, stop, threads })
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads {
            let _ = t.join();
        }
    }
}

fn respond(req: tiny_http::Request, api: &DashApi) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let host = req.headers().iter().find(|h| h.field.equiv("Host")).map(|h| h.value.as_str().to_string());
    let method = match req.method() {
        Method::Get => "GET",
        Method::Head => "HEAD",
        _ => "OTHER",
    };
    let r = Request { method, path, query, host: host.as_deref() };
    // Everything but a plain page request goes through the router (host check, 405, API).
    let (resp, extra) = if path.starts_with("/api/") || method == "OTHER" || !api.host_ok(host.as_deref()) {
        (api.handle(&r), None)
    } else {
        match assets::get(path) {
            Some(a) => (Response { status: 200, content_type: a.content_type, body: a.body.to_vec(), no_store: false, allow: None }, Some(("Cache-Control", "no-cache".to_string()))),
            None => (Response { status: 404, content_type: "text/plain; charset=utf-8", body: b"not found".to_vec(), no_store: false, allow: None }, None),
        }
    };
    let mut out = HttpResponse::from_data(resp.body.clone()).with_status_code(resp.status);
    for (k, v) in resp.headers().into_iter().chain(extra) {
        if let Ok(h) = Header::from_bytes(k, v) {
            out.add_header(h);
        }
    }
    let _ = req.respond(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn raw(addr: SocketAddr, request: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }
    fn get(addr: SocketAddr, path: &str, host: &str) -> String {
        raw(addr, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"))
    }

    #[test]
    fn server_binds_loopback_only_and_answers_with_security_headers() {
        let d = tempfile::tempdir().unwrap();
        let s = DashboardServer::start(d.path().join("none.db"), 0).unwrap();
        assert!(s.addr.ip().is_loopback());
        let r = get(s.addr, "/", &format!("127.0.0.1:{}", s.addr.port()));
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
        let lower = r.to_lowercase();
        assert!(lower.contains("content-security-policy: default-src 'self'") && lower.contains("x-content-type-options: nosniff"));
        assert!(!lower.contains("access-control-"));
        let api = get(s.addr, "/api/status", &format!("localhost:{}", s.addr.port()));
        assert!(api.starts_with("HTTP/1.1 503") && api.to_lowercase().contains("cache-control: no-store"), "{api}");
        s.stop();
    }
    #[test]
    fn rebinding_host_is_refused_over_the_socket() {
        let d = tempfile::tempdir().unwrap();
        let s = DashboardServer::start(d.path().join("none.db"), 0).unwrap();
        for p in ["/", "/api/status", "/main.js"] {
            assert!(get(s.addr, p, &format!("evil.example:{}", s.addr.port())).starts_with("HTTP/1.1 403"), "{p}");
        }
        s.stop();
    }
    #[test]
    fn stop_releases_the_port() {
        let d = tempfile::tempdir().unwrap();
        let s = DashboardServer::start(d.path().join("none.db"), 0).unwrap();
        let port = s.addr.port();
        s.stop();
        DashboardServer::start(d.path().join("none.db"), port).unwrap().stop();
    }
    #[test]
    fn a_post_with_a_body_is_405() {
        let d = tempfile::tempdir().unwrap();
        let s = DashboardServer::start(d.path().join("none.db"), 0).unwrap();
        let r = raw(s.addr, &format!("POST /api/status HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello", s.addr.port()));
        assert!(r.starts_with("HTTP/1.1 405") && r.contains("Allow: GET, HEAD"), "{r}");
        s.stop();
    }
}
