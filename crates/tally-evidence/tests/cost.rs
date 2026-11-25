use tally_evidence::build::*;
use tally_evidence::cost::*;
use tally_scoring::{chain, score, GENESIS_HASH};

fn ledger_json() -> (String, Vec<tally_scoring::LedgerEntry>) {
    // two prompts: the first is a first action (120 s cap), the second is 300 s later with a thinking-time allowance
    let claims = score(&[prompt("a", 120_000, 200, 700, 100), prompt("b", 420_000, 200, 700, 100)]);
    let es = chain(&claims, GENESIS_HASH);
    (serde_json::to_string(&es).unwrap(), es)
}

#[test]
fn stats_over_two_known_entries() {
    let (json, es) = ledger_json();
    let st = ledger_stats(&json).unwrap();
    let want: u64 = es.iter().map(|e| u64::from(e.verified_s)).sum();
    assert_eq!((st.entries, st.verified_s), (2, want));
    assert!(want > 0);
    assert_eq!(st.bytes_total, es.iter().map(|e| serde_json::to_string(e).unwrap().len() as u64).sum::<u64>());
    assert_eq!(st.bytes_per_entry_mean, st.bytes_total / 2);
}

#[test]
fn projection_scales_by_verified_hours() {
    let st = LedgerStats { entries: 10, verified_s: 3600, bytes_total: 2000, bytes_per_entry_mean: 200, bytes_per_verified_hour: 2000 };
    let p = project(&st, 109_759);
    assert_eq!(p.upload_bytes_monthly, 109_759 * 2000);
    assert_eq!(p.entries_monthly, 109_759 * 10);
    assert_eq!(p.storage_bytes_yearly, p.upload_bytes_monthly * 12);
}

#[test]
fn unsourced_price_is_flagged_and_totals_use_only_given_numbers() {
    let inputs = parse_inputs(
        "users_monthly = 100\nhours_monthly = 1000\n[[prices]]\nitem = \"host\"\nlow_usd = 10.0\nhigh_usd = 20.0\nsource = \"vendor page, checked today\"\n[[prices]]\nitem = \"gpu\"\nlow_usd = 5.5\nhigh_usd = 7.0\n",
    )
    .unwrap();
    let st = LedgerStats { entries: 10, verified_s: 3600, bytes_total: 2000, bytes_per_entry_mean: 200, bytes_per_verified_hour: 2000 };
    let md = render(&st, &project(&st, inputs.hours_monthly), &inputs, "x.json");
    assert!(md.contains("UNSOURCED"));
    assert!(md.contains("**15.50** | **27.00**"), "{md}");
    assert!(md.contains("wrong by this measurement"), "mean 200 bytes exceeds the pitch's 'tens of bytes'");
}

#[test]
fn empty_ledger_is_an_error_not_a_divide_by_zero() {
    assert!(ledger_stats("[]").is_err());
    assert!(ledger_stats("not json").is_err());
}
