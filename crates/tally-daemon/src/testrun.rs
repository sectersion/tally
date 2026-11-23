//! User test runs recorded by `tally hook test-run`. Only a hash of the command is stored.
use sha2::{Digest, Sha256};
use tally_scoring::{Actor, Event, Kind};

/// Lowercase hex SHA-256 of the arguments joined with `\u{1f}`.
pub fn cmd_hash(args: &[String]) -> String {
    hex::encode(Sha256::digest(args.join("\u{1f}").as_bytes()))
}

/// A user test run stamped at completion time.
pub fn test_run_event(args: &[String], exit_code: i32, now_ms: i64, project: Option<String>, pid: u32) -> Event {
    Event {
        v: 1,
        id: format!("shell:{now_ms}:{pid}"),
        ts: now_ms,
        source: "shell".into(),
        project,
        session: None,
        kind: Kind::TestRun { cmd_hash: cmd_hash(args), exit_code, initiator: Actor::User },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_hash_is_stable_and_argument_sensitive() {
        let a = |s: &[&str]| cmd_hash(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["cargo", "test"]), a(&["cargo", "test"]));
        assert_ne!(a(&["cargo", "test"]), a(&["cargo", "test", "-p", "x"]));
        assert_ne!(a(&["ab", "c"]), a(&["a", "bc"]));
        assert_eq!(a(&["cargo", "test"]).len(), 64);
    }

    #[test]
    fn event_is_a_user_test_run_stamped_at_completion() {
        let args = vec!["cargo".to_string(), "test".to_string()];
        let e = test_run_event(&args, 101, 5000, Some("p".into()), 42);
        assert_eq!(e.id, "shell:5000:42");
        assert_eq!(e.source, "shell");
        assert_eq!(e.ts, 5000);
        assert_eq!(e.project.as_deref(), Some("p"));
        assert_eq!(e.kind, Kind::TestRun { cmd_hash: cmd_hash(&args), exit_code: 101, initiator: Actor::User });
        assert!(e.is_human_action());
    }
}
