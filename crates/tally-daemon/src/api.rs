//! Blocking client for the v1 ledger API (contract C7). Only this module talks HTTP.
use serde::Deserialize;
use std::time::Duration;
use tally_scoring::LedgerEntry;

pub struct Api {
    agent: ureq::Agent,
    base: String,
    token: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Head {
    pub head_hash: String,
    pub count: u64,
}

#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("device token rejected; run tally login --server-id <id>")]
    Unauthorized,
    #[error("batch too large")]
    TooLarge,
    #[error("server head differs: {head_hash}")]
    Conflict { head_hash: String },
    #[error("entry {index} rejected: {reason}")]
    Rejected { index: usize, reason: String },
    #[error("server has no such project; check tally project link")]
    NotFound,
    #[error("server error {0}")]
    Server(u16),
    #[error("network error: {0}")]
    Network(String),
}

impl ApiError {
    pub fn retryable(&self) -> bool {
        matches!(self, ApiError::Server(_) | ApiError::Network(_))
    }
}

fn classify(e: ureq::Error) -> ApiError {
    match e {
        ureq::Error::Status(code, resp) => match code {
            401 | 403 => ApiError::Unauthorized,
            404 => ApiError::NotFound,
            413 => ApiError::TooLarge,
            409 | 422 => {
                let v: serde_json::Value = serde_json::from_reader(resp.into_reader()).unwrap_or_default();
                if code == 409 {
                    match v["head_hash"].as_str() {
                        Some(h) => ApiError::Conflict { head_hash: h.into() },
                        None => ApiError::Server(409),
                    }
                } else {
                    ApiError::Rejected {
                        index: v["index"].as_u64().unwrap_or(0) as usize,
                        reason: v["reason"].as_str().unwrap_or("").into(),
                    }
                }
            }
            c => ApiError::Server(c),
        },
        // Kind only: the Display of a transport error can embed the URL.
        ureq::Error::Transport(t) => ApiError::Network(t.kind().to_string()),
    }
}

impl Api {
    pub fn new(base: &str, token: &str) -> Api {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("tally/", env!("CARGO_PKG_VERSION")))
            .redirects(0) // a redirect could carry the bearer token elsewhere
            .build();
        Api { agent, base: base.trim_end_matches('/').into(), token: token.into() }
    }

    fn send(&self, req: ureq::Request, body: Option<serde_json::Value>) -> Result<ureq::Response, ApiError> {
        let req = req.set("Authorization", &format!("Bearer {}", self.token));
        let r = match body {
            Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
            None => req.call(),
        }
        .map_err(classify)?;
        // With redirects(0) ureq may hand back a 3xx as Ok.
        match r.status() {
            200..=299 => Ok(r),
            c => Err(ApiError::Server(c)),
        }
    }

    fn parse_head(r: ureq::Response) -> Result<Head, ApiError> {
        serde_json::from_reader(r.into_reader()).map_err(|_| ApiError::Network("BadBody".into()))
    }

    pub fn head(&self, server_id: &str, ruleset: &str) -> Result<Head, ApiError> {
        assert!(ruleset.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'));
        let url = format!("{}/v1/ledger/{server_id}/head?ruleset={ruleset}", self.base);
        Self::parse_head(self.send(self.agent.get(&url), None)?)
    }

    pub fn upload(&self, server_id: &str, prev_hash: &str, entries: &[LedgerEntry]) -> Result<Head, ApiError> {
        let url = format!("{}/v1/ledger/{server_id}/entries", self.base);
        let body = serde_json::json!({"prev_hash": prev_hash, "entries": entries});
        Self::parse_head(self.send(self.agent.post(&url), Some(body))?)
    }

    pub fn commit(&self, server_id: &str, hash: &str, ts: i64) -> Result<(), ApiError> {
        let url = format!("{}/v1/ledger/{server_id}/commit", self.base);
        self.send(self.agent.post(&url), Some(serde_json::json!({"hash": hash, "ts": ts}))).map(|_| ())
    }
}
