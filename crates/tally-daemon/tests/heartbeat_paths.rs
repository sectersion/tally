use std::collections::BTreeMap;
use tally_daemon::heartbeat::{file_hash, map_heartbeats, server::Received, Heartbeat};
use tally_daemon::project::{normalize_path, project_for_dir};

const T: i64 = 1_763_800_000_000;

fn hb(path: &str, ms: i64) -> Received {
    let h = Heartbeat {
        entity: path.into(),
        kind: Some("file".into()),
        time: ms as f64 / 1000.0,
        category: None,
        project: None,
        branch: None,
        language: None,
        is_write: Some(true),
        lines: None,
        lineno: None,
        cursorpos: None,
        line_additions: None,
        line_deletions: None,
        ai_line_changes: None,
        human_line_changes: None,
        user_agent: None,
        plugin: None,
    };
    Received {
        hb: h,
        received_ms: ms,
    }
}

/// Agent wrote src/a.rs at T. Editor-reaction heartbeats for the SAME file spelled differently must be dropped as agent edits.
#[test]
fn agent_filter_survives_path_spelling_variants() {
    let d = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(d.path()).unwrap().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let (_, pid) = project_for_dir(&repo).unwrap();
    let base = normalize_path(&repo.to_string_lossy());
    let agent = [(file_hash(&pid, "src/a.rs"), T)];
    let af = |h: &str, a: i64, b: i64| agent.iter().any(|(x, t)| x == h && *t >= a && *t <= b);
    let mut leaked = vec![];
    for (name, p) in [
        ("exact", format!("{base}/src/a.rs")),
        ("dotdot", format!("{base}/sub/../src/a.rs")),
        ("dot", format!("{base}/src/./a.rs")),
        ("doubleslash", format!("{base}/src//a.rs")),
        (
            "upper-case dir (win/mac FS case-insensitive)",
            format!("{base}/SRC/a.rs"),
        ),
        ("upper-case file", format!("{base}/src/A.rs")),
        ("backslashes", p_back(&base)),
    ] {
        let m = map_heartbeats(
            &[hb(&p, T + 1_000)],
            &project_for_dir,
            &af,
            0,
            &mut BTreeMap::new(),
        );
        println!(
            "{name:45} -> accepted={} dropped_agent={} outside={}",
            m.stats.accepted, m.stats.dropped_agent, m.stats.outside_project
        );
        if m.stats.accepted == 1 {
            leaked.push(name);
        }
    }
    assert!(
        leaked.is_empty(),
        "agent write credited as human edit via: {leaked:?}"
    );
}

fn p_back(base: &str) -> String {
    format!("{base}/src/a.rs").replace('/', "\\")
}
