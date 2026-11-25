use std::path::PathBuf;
use std::time::Duration;
use tally_evidence::load::LoadPlan;
use tally_evidence::resources::*;

fn machine() -> MachineInfo {
    MachineInfo { description: "test box".into(), os: "os".into(), cpu_model: "cpu".into(), logical_cores: 2, total_ram_bytes: 4_000_000_000 }
}

fn plan() -> LoadPlan {
    LoadPlan::default_for(PathBuf::from("."))
}

#[test]
fn summary_percentiles_and_means() {
    let s: Vec<Sample> = (1..=100).map(|i| Sample { t_s: i, rss_bytes: i * 1_000_000, cpu_pct_x100: (i % 4) as u32 * 100 }).collect();
    let r = summarize(&s);
    assert_eq!((r.rss_p50, r.rss_p95, r.rss_max), (50_000_000, 95_000_000, 100_000_000));
    assert_eq!(r.cpu_max_x100, 300);
    assert_eq!(r.cpu_mean_x100, 150);
    assert_eq!(summarize(&[]), Summary::default());
}

#[test]
fn numbers_file_states_pass_or_fail_against_the_targets() {
    let sum = Summary { n: 10, rss_p50: 55_000_000, rss_p95: 60_000_000, rss_max: 61_000_000, cpu_mean_x100: 20, cpu_p95_x100: 40, cpu_max_x100: 90 };
    let md = render_numbers(&sum, &sum, &machine(), &plan(), "tally 0.1.0", Some(true));
    assert!(md.contains("RSS p95"), "{md}");
    assert!(md.contains("60.0 MB: **FAIL**"), "{md}");
    assert!(md.contains("0.40%: **PASS**"), "{md}");
    assert!(md.contains("test box") && md.contains("plugged in"));
}

#[test]
fn accelerated_runs_are_labelled_as_not_real_time() {
    let mut p = plan();
    p.speedup = 600;
    let md = render_numbers(&Summary::default(), &Summary::default(), &machine(), &p, "v", None);
    assert!(md.contains("accelerated 600x (NOT a real-time run)"));
}

#[test]
fn missing_machine_description_is_refused_and_no_file_is_written() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("r.md");
    let o = Options { tally: PathBuf::from("does-not-matter"), machine: "  ".into(), duration: Duration::from_secs(1), speedup: 1, out: out.clone(), no_presence: true, plugged: None };
    let e = run(&o).unwrap_err().to_string();
    assert!(e.contains("--machine"), "{e}");
    assert!(!out.exists());
}
