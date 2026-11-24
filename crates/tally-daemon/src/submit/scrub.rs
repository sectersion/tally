//! Local secret scrubber: every match becomes `[REDACTED:<kind>]` before text leaves the machine.
use regex::{Captures, Regex};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub struct Scrubbed {
    pub text: String,
    pub counts: BTreeMap<&'static str, usize>,
}

/// kind, pattern, capture group whose span is replaced (0 = whole match).
const RULES: &[(&str, &str, usize)] = &[
    (
        "private_key",
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----|PuTTY-User-Key-File-\d+:.*?Private-MAC: [0-9a-f]+",
        0,
    ),
    ("aws_key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0),
    ("github_token", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{60,}\b", 0),
    ("slack_token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}", 0),
    ("llm_key", r"\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}|\bhf_[A-Za-z0-9]{30,}\b", 0),
    ("stripe_key", r"\b[rs]k_(?:live|test)_[A-Za-z0-9]{16,}\b", 0),
    ("google_key", r"\bAIza[0-9A-Za-z_-]{35}\b", 0),
    ("jwt", r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}", 0),
    // only `user:pass` is replaced; scheme and `@` stay
    ("url_credentials", r"[a-z][a-z0-9+.-]*://([^/\s:@]+:[^/\s@]+)@", 1),
    // `["']?` before the separator also covers JSON keys like "client_secret": "..."
    (
        "env_secret",
        r#"(?i)\b[A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|API_?KEY|PRIVATE_KEY|ACCESS_KEY|AUTH)[A-Z0-9_]*["']?\s*[:=]\s*["']?([^\s"']{6,})"#,
        1,
    ),
    ("high_entropy", r"[A-Za-z0-9+/=_-]{32,}", 0),
];

fn regexes() -> &'static Vec<Regex> {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| RULES.iter().map(|(_, p, _)| Regex::new(p).unwrap()).collect())
}

fn entropy(s: &str) -> f64 {
    let mut freq = [0usize; 256];
    for b in s.bytes() {
        freq[b as usize] += 1;
    }
    let n = s.len() as f64;
    freq.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f64 / n;
        -p * p.log2()
    }).sum()
}

/// Should this kind's match be redacted? Only high_entropy and env_secret need a say.
fn wanted(kind: &str, c: &Captures, text: &str) -> bool {
    let m = c.get(RULES.iter().find(|r| r.0 == kind).unwrap().2).unwrap();
    let v = m.as_str();
    match kind {
        "env_secret" => !v.starts_with("[REDACTED:"),
        "high_entropy" => {
            let hex = v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            if hex && v.len() >= 40 {
                // ponytail: a hex secret written right after "commit"/"sha"/"hash" passes; accepted so linked commits stay readable
                let before: String = text[..m.start()].chars().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
                let before = before.to_lowercase();
                let sha_len = v.len() == 40 || v.len() == 64;
                if sha_len && ["commit", "sha", "hash"].iter().any(|w| before.contains(w)) {
                    return false;
                }
                entropy(v) >= 3.0
            } else {
                // needs all three classes so paths and identifiers (usually lowercase) survive
                let up = v.bytes().any(|b| b.is_ascii_uppercase());
                let lo = v.bytes().any(|b| b.is_ascii_lowercase());
                let di = v.bytes().any(|b| b.is_ascii_digit());
                // `/` mixed with `-` or `_` is a path, not one base64 alphabet
                let path = v.contains('/') && v.contains(['-', '_']);
                up && lo && di && !path && entropy(v) >= 4.0
            }
        }
        _ => true,
    }
}

pub fn scrub(text: &str) -> Scrubbed {
    let mut text = text.to_string();
    let mut counts = BTreeMap::new();
    for ((kind, _, group), re) in RULES.iter().zip(regexes()) {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut n = 0;
        for c in re.captures_iter(&text) {
            if !wanted(kind, &c, &text) {
                continue;
            }
            let m = c.get(*group).unwrap();
            out.push_str(&text[last..m.start()]);
            out.push_str("[REDACTED:");
            out.push_str(kind);
            out.push(']');
            last = m.end();
            n += 1;
        }
        out.push_str(&text[last..]);
        if n > 0 {
            counts.insert(*kind, n);
        }
        text = out;
    }
    Scrubbed { text, counts }
}
