use tally_daemon::service::*;

fn spec() -> ServiceSpec {
    ServiceSpec {
        exe: "/usr/local/bin/tally".into(),
        home: "/home/o/.local/share/tally".into(),
        label: LABEL,
        unit_dir: "/home/o/.config/systemd/user".into(),
        uid: 501,
        user: r"WORKGROUP\o".into(),
    }
}

/// `UPDATE_GOLDEN=1 cargo test --test service_golden` rewrites the goldens; review the diff against the OS tool.
fn golden(name: &str, actual: String) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
    }
    // normalise CRLF from autocrlf checkouts
    let want = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    assert_eq!(actual, want, "{name}");
}

#[test]
fn systemd_unit_matches_golden() {
    golden("tally.service", render_systemd_unit(&spec()));
}
#[test]
fn plist_matches_golden() {
    golden("dev.tally.agent.plist", render_launchd_plist(&spec()));
}
#[test]
fn task_xml_matches_golden() {
    golden("task.xml", render_task_xml(&spec()));
}

#[test]
fn paths_with_spaces_and_xml_characters_are_escaped() {
    let mut s = spec();
    s.exe = r"C:\Program Files\Tally & Co\tally.exe".into();
    assert!(render_task_xml(&s).contains("Tally &amp; Co"));
    assert!(render_launchd_plist(&s).contains("Tally &amp; Co"));
    let mut t = spec();
    t.exe = "/opt/my tally/tally".into();
    assert!(render_systemd_unit(&t).contains("ExecStart=\"/opt/my tally/tally\" run"));
}

#[test]
fn uninstall_never_touches_the_data_directory() {
    let s = spec();
    for os in ["windows", "macos", "linux"] {
        for step in plan(os, Action::Uninstall, &s) {
            match &step {
                Step::RemoveFile(p) => assert!(!p.starts_with(&s.home), "{p:?}"),
                Step::Run(c) | Step::TryRun(c) => assert!(!c.args.iter().any(|a| a.contains(s.home.to_str().unwrap())), "{c:?}"),
                Step::WriteFile { .. } => panic!("uninstall writes nothing: {step:?}"),
            }
        }
    }
}

#[test]
fn install_twice_plans_the_same_idempotent_steps() {
    for os in ["windows", "macos", "linux"] {
        assert_eq!(plan(os, Action::Install, &spec()), plan(os, Action::Install, &spec()));
    }
}

#[test]
fn every_action_plans_something_on_every_os() {
    for os in ["windows", "macos", "linux"] {
        for a in [Action::Install, Action::Start, Action::Stop, Action::Status, Action::Uninstall] {
            assert!(!plan(os, a, &spec()).is_empty(), "{os} {a:?}");
        }
    }
}
