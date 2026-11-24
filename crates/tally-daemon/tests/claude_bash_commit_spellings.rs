//! Agent git commits must be detected whatever the spelling (path, .exe, wrappers, shell keywords).
use tally_daemon::claude_code::session::{bash_kind, BashKind};

#[test]
fn git_commit_spellings_are_detected() {
    let cases = [
        "/usr/bin/git commit -m x",
        "git.exe commit -m x",
        r#"& 'C:\Program Files\Git\cmd\git.exe' commit -m x"#,
        r#""C:\Program Files\Git\cmd\git.exe" commit -m x"#,
        "/bin/bash -c \"git commit -m x\"",
        "bash -x -c \"git commit -m x\"",
        "cmd /c git commit -m x",
        "cmd.exe /c \"git commit -m x\"",
        "if true; then git commit -m x; fi",
        "for i in 1; do git commit -m x; done",
        "! git commit -m x",
        "nohup git commit -m x",
        "timeout 30 git commit -m x",
        "exec git commit -m x",
        "env -i git commit -m x",
        "eval \"git commit -m x\"",
        "Invoke-Expression 'git commit -m x'",
        "git cherry-pick abc",
        "git merge feature",
        "git revert HEAD",
        "git rebase main",
        r"\git commit -m x",
        "GIT commit -m x",
    ];
    let missed: Vec<_> = cases
        .iter()
        .filter(|c| bash_kind(c) != Some(BashKind::Commit))
        .collect();
    assert!(missed.is_empty(), "missed: {missed:?}");
    assert_eq!(bash_kind("git status"), None);
}
