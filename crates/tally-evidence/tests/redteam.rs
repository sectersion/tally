use tally_evidence::redteam::*;
use tally_evidence::scenarios::*;

fn out(n: &str) -> Outcome {
    run(&by_name(n).unwrap())
}

#[test]
fn print_measurements() {
    // `cargo test -p tally-evidence --test redteam print_measurements -- --nocapture` shows every number.
    for s in all().iter().chain(std::iter::once(&control())) {
        let o = run(s);
        println!("{:40} verified {:5} s unverified {:5} s events {:4} {:?}", o.name, o.verified_s, o.unverified_s, s.events.len(), o.reasons);
    }
}

#[test]
fn control_earns_real_credit_so_a_zero_scorer_cannot_pass() {
    let r = out("honest_control");
    assert!(r.verified_s >= 1800, "honest hour credited {} s", r.verified_s);
}

#[test]
fn attacks_with_no_human_events_earn_nothing() {
    for n in ["scripted_claude_p_loop", "parallel_agents"] {
        assert_eq!(out(n).verified_s, 0, "{n}");
    }
}

#[test]
fn every_scenario_names_its_assumption_and_pitch_row() {
    for s in all() {
        assert!(!s.assumption.is_empty() && !s.pitch_row.is_empty(), "{}", s.name);
    }
}

#[test]
fn report_lists_gaps_and_what_it_does_not_model() {
    let md = render("abc1234");
    assert!(md.contains("abc1234") && md.contains("## Gaps") && md.contains("hourly_bot_push") && md.contains("prompt_injection_in_transcript"));
}

// ---- measured gaps ----
// The scorer caps each ACTION (spec section 4), not each hour: an action is credited up to its cap for the time since the
// previous human action. Cheap actions spaced closer than their cap therefore earn nearly the whole hour. These are findings,
// not intended behaviour: the pitch's threat-model table says each of these is "neutralized by its own rule". The ceilings
// below are the values measured on 2026-11-24 (ruleset 2026.11-1). When a ruleset change closes a gap the test fails on
// purpose: lower the ceiling to the new measurement and delete the GAP note in `redteam.rs` output expectations.

#[test]
fn gap_esc_spam_earns_almost_the_whole_hour() {
    // Interrupt cap is 30 s but an Interrupt every 5 s makes each interval 5 s: all of it is credited. Measured 3625 s.
    let v = out("esc_spam").verified_s;
    assert!(v > 3000 && v <= 3700, "{v}");
}

#[test]
fn gap_templated_loop_attributed_to_the_human_earns_the_whole_hour_at_the_scorer() {
    // Only the judge (plan 10) stands between this and full credit. Measured 3630 s.
    let v = out("scripted_loop_misattributed_as_human").verified_s;
    assert!(v > 3000 && v <= 3700, "{v}");
}

#[test]
fn gap_planted_failing_test_earns_the_whole_hour() {
    // Spec section 8 defers the "failing test existed before the loop" rule. Measured 3600 s (12 loops of 5 min).
    let v = out("planted_failing_test").verified_s;
    assert!(v > 3000 && v <= 3700, "{v}");
}

#[test]
fn gap_trivial_commits_are_capped_per_commit_not_per_hour() {
    // One-line commits are capped at 60 s each; one every 90 s earns 60/90 of the hour. Measured 2400 s.
    let v = out("trivial_commits").verified_s;
    assert!(v > 2000 && v <= 2500, "{v}");
}

#[test]
fn padded_prompts_are_capped_per_claim() {
    // Zero-relevance prompts are capped at 90 s each (spec section 4). 30 prompts: measured 2700 s, exactly the cap.
    let s = by_name("padded_random_prompts").unwrap();
    let prompts = s.events.len() as i64;
    assert!(run(&s).verified_s <= prompts * 90);
}

#[test]
fn youtube_while_agent_runs_credits_only_the_edges() {
    // Prompt 1: first action, 120 s. Prompt 2 at 59 min: the idle span (3 to 58 min, grace 3 min) and agent-idle leave about 285 s.
    // Measured 405 s = 120 + 285.
    assert!(out("idle_youtube_while_agent_runs").verified_s <= 405);
}
