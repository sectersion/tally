//! Cost sheet from MEASURED ledger sizes. Prices come only from `cost-inputs.toml`, supplied by the maintainer with a
//! source; the generator never invents a price, and a price without a source prints `UNSOURCED`.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tally_scoring::LedgerEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerStats {
    pub entries: usize,
    pub verified_s: u64,
    pub bytes_total: u64,
    pub bytes_per_entry_mean: u64,
    pub bytes_per_verified_hour: u64,
}

/// Bytes are the `serde_json::to_string` length of each entry (what the daemon uploads and the server stores).
pub fn ledger_stats(entries_json: &str) -> Result<LedgerStats> {
    let es: Vec<LedgerEntry> = serde_json::from_str(entries_json).context("the ledger file is not a JSON array of ledger entries")?;
    if es.is_empty() {
        bail!("the ledger is empty: there is nothing to measure");
    }
    let verified_s: u64 = es.iter().map(|e| u64::from(e.verified_s)).sum();
    if verified_s == 0 {
        bail!("the ledger has no verified time: bytes per verified hour is undefined");
    }
    let bytes_total: u64 = es.iter().map(|e| serde_json::to_string(e).map_or(0, |s| s.len() as u64)).sum();
    Ok(LedgerStats {
        entries: es.len(),
        verified_s,
        bytes_total,
        bytes_per_entry_mean: bytes_total / es.len() as u64,
        bytes_per_verified_hour: bytes_total * 3600 / verified_s,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub entries_monthly: u64,
    pub upload_bytes_monthly: u64,
    pub storage_bytes_yearly: u64,
}

pub fn project(stats: &LedgerStats, hours_monthly: u64) -> Projection {
    let upload = hours_monthly * stats.bytes_per_verified_hour;
    Projection { entries_monthly: hours_monthly * stats.entries as u64 * 3600 / stats.verified_s.max(1), upload_bytes_monthly: upload, storage_bytes_yearly: upload * 12 }
}

#[derive(Debug, Deserialize)]
pub struct Price {
    pub item: String,
    pub low_usd: f64,
    pub high_usd: f64,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Deserialize)]
pub struct Inputs {
    pub users_monthly: u64,
    pub hours_monthly: u64,
    #[serde(default)]
    pub prices: Vec<Price>,
}

pub fn parse_inputs(text: &str) -> Result<Inputs> {
    toml::from_str(text).context("cost-inputs.toml")
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

pub fn render(stats: &LedgerStats, p: &Projection, inputs: &Inputs, ledger_name: &str) -> String {
    let mut s = format!(
        "# Cost sheet (generated)\n\n## Measured\n\nFrom `{ledger_name}`: {} entries, {:.2} verified hours.\n\n- Bytes per entry (mean): **{}**\n- Bytes per verified hour: **{}**\n- Total: {} bytes\n\n",
        stats.entries,
        stats.verified_s as f64 / 3600.0,
        stats.bytes_per_entry_mean,
        stats.bytes_per_verified_hour,
        stats.bytes_total
    );
    if stats.bytes_per_entry_mean > 100 {
        s.push_str("The PITCH sentence \"tens of bytes per action record\" is wrong by this measurement (mean above 100 bytes): replace it with the measured figure.\n\n");
    }
    s.push_str(&format!(
        "## Projected at PITCH scale ({} users, {} hours per month)\n\n- Entries per month: {}\n- Upload per month: {}\n- Stored after 12 months: {}\n\n## Line items\n\n| item | low USD/month | high USD/month | source |\n|---|---|---|---|\n",
        inputs.users_monthly,
        inputs.hours_monthly,
        p.entries_monthly,
        mb(p.upload_bytes_monthly),
        mb(p.storage_bytes_yearly)
    ));
    let (mut lo, mut hi) = (0.0, 0.0);
    for pr in &inputs.prices {
        lo += pr.low_usd;
        hi += pr.high_usd;
        let src = if pr.source.trim().is_empty() { "**UNSOURCED**".to_string() } else { pr.source.clone() };
        s.push_str(&format!("| {} | {:.2} | {:.2} | {} |\n", pr.item, pr.low_usd, pr.high_usd, src));
    }
    s.push_str(&format!("| **Total of the items above** | **{lo:.2}** | **{hi:.2}** | only the numbers given in `cost-inputs.toml` |\n"));
    if inputs.prices.iter().any(|p| p.source.trim().is_empty()) {
        s.push_str("\nSome prices are UNSOURCED: they are placeholders the maintainer must replace or source before this sheet is quoted.\n");
    }
    s
}
