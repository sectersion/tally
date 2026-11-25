use tally_evidence::build::*;
use tally_evidence::replay::*;

const MIN: i64 = 60_000;

fn agent_away() -> Vec<tally_scoring::Event> {
    vec![
        prompt("p1", 0, 200, 400, 100),
        busy("b", 0, 60 * MIN),
        idle("i", 5 * MIN, 55 * MIN),
        prompt("p2", 58 * MIN, 200, 400, 100),
    ]
}

fn steady() -> Vec<tally_scoring::Event> {
    (0..30).map(|i| if i % 2 == 0 { prompt(&format!("p{i}"), i * MIN, 200, 700, 100) } else { edit(&format!("e{i}"), i * MIN, 5) }).collect()
}

#[test]
fn agent_runs_while_user_is_away() {
    let r = compare(&agent_away(), None);
    assert!(r.hackatime_s >= 3500, "{}", r.hackatime_s);
    assert!(r.tally_verified_s < r.hackatime_s / 4, "{} vs {}", r.tally_verified_s, r.hackatime_s);
    assert!(!r.removed.is_empty());
    assert!(r.removed.iter().any(|m| m.reason == "away"));
}

#[test]
fn steady_human_work_loses_almost_nothing() {
    let r = compare(&steady(), None);
    assert!(r.hackatime_s >= 1700, "Hackatime must count the session or this test proves nothing: {}", r.hackatime_s);
    assert!(r.removed.len() <= 2, "{:?}", r.removed);
    assert!(r.tally_verified_s > 0);
}

#[test]
fn removed_plus_common_minutes_match_the_hackatime_total() {
    for ev in [agent_away(), steady()] {
        let r = compare(&ev, None);
        assert!(r.hackatime_s > 0);
        let counted = (r.removed.len() + r.common_minutes) as i64 * 60;
        assert!((counted - r.hackatime_s).abs() <= 60, "{counted} vs {}", r.hackatime_s);
    }
}

#[test]
fn real_heartbeats_replace_the_synthetic_ones_and_are_labelled() {
    let r = compare(&steady(), Some(vec![0, 60_000]));
    assert_eq!(r.hackatime_s, 60);
    assert_eq!(r.source_of_hackatime, "real heartbeat export");
}

#[test]
fn markdown_and_json_render_the_numbers() {
    let r = compare(&agent_away(), None);
    let md = render_markdown(&r, "Agent away");
    assert!(md.starts_with("# Agent away") && md.contains("`away`") && md.contains("approximation"));
    assert!(render_json(&r).contains("\"hackatime_s\""));
}
