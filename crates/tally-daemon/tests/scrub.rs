use tally_daemon::submit::scrub::scrub;

fn lines(name: &str) -> Vec<String> {
    let p = format!("{}/tests/fixtures/secrets/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.replace("~~", ""))
        .collect()
}

#[test]
fn every_positive_is_redacted() {
    let ls = lines("positives.txt");
    assert!(ls.len() >= 33);
    for l in ls {
        let mut it = l.splitn(3, '|');
        let (kind, secret, line) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        assert!(line.contains(secret), "fixture secret not in line: {l}");
        let out = scrub(line);
        let chars: Vec<char> = secret.chars().collect();
        for w in chars.windows(8) {
            let sub: String = w.iter().collect();
            assert!(!out.text.contains(&sub), "{kind}: leaked {sub} in {}", out.text);
        }
        assert!(
            out.counts.get(kind).copied().unwrap_or(0) >= 1,
            "{kind} not counted: {:?} / {}",
            out.counts,
            out.text
        );
    }
}

#[test]
fn negatives_are_untouched() {
    for l in lines("negatives.txt") {
        assert_eq!(scrub(&l).text, l);
    }
}

#[test]
fn redaction_is_idempotent() {
    for l in lines("positives.txt").iter().chain(lines("negatives.txt").iter()) {
        let once = scrub(l).text;
        assert_eq!(scrub(&once).text, once);
    }
}

#[test]
fn multiline_private_key_is_one_redaction() {
    let t = "before\n-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIAbcdefghijkl\nmnopqrstuvwxyz0123456\n-----END EC PRIVATE KEY-----\nafter";
    let s = scrub(t);
    assert_eq!(s.text, "before\n[REDACTED:private_key]\nafter");
    assert_eq!(s.counts["private_key"], 1);
}
