use std::collections::BTreeMap;
use tally_daemon::heartbeat::{map_heartbeats, server::Received, Heartbeat};
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

/// Editor plugin says the whole change was AI-generated (ai_line_changes>0, human_line_changes=0, category "ai coding"):
/// it must not become a human user_edit.
#[test]
fn ai_attributed_heartbeat_is_not_a_human_edit() {
    let d = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(d.path()).unwrap().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let f = format!("{}/a.rs", normalize_path(&repo.to_string_lossy()));
    let mut r = hb(&f, T);
    r.hb.ai_line_changes = Some(500);
    r.hb.human_line_changes = Some(0);
    r.hb.category = Some("ai coding".into());
    let m = map_heartbeats(
        &[r],
        &project_for_dir,
        &|_, _, _| false,
        0,
        &mut BTreeMap::new(),
    );
    println!("events={} stats={:?}", m.events.len(), m.stats);
    assert_eq!(
        m.events.len(),
        0,
        "AI-attributed heartbeat credited as human user_edit"
    );
}
