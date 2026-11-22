use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub id: String,
    pub ts: i64,
    pub source: String,
    pub project: Option<String>,
    pub session: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    Prompt { chars: u32, relevance_pm: u16, novelty_bytes: u32 },
    Interrupt,
    ToolDecision { decision: Decision },
    UserEdit { file_hash: String, lines_changed: u32 },
    TestRun { cmd_hash: String, exit_code: i32, initiator: Actor },
    Commit { sha: String, by: Actor, lines_changed: u32 },
    AgentBusy { end_ts: i64 },
    Idle { end_ts: i64 },
    AgentOutput { chars: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Agent,
}

impl Event {
    pub fn is_human_action(&self) -> bool {
        match &self.kind {
            Kind::Prompt { .. }
            | Kind::Interrupt
            | Kind::ToolDecision { .. }
            | Kind::UserEdit { .. } => true,
            Kind::TestRun { initiator, .. } => *initiator == Actor::User,
            Kind::Commit { by, .. } => *by == Actor::User,
            Kind::AgentBusy { .. } | Kind::Idle { .. } | Kind::AgentOutput { .. } => false,
        }
    }
}

#[cfg(test)]
pub(crate) fn ev(kind: Kind) -> Event {
    Event { v: 1, id: "x".into(), ts: 0, source: "test".into(), project: None, session: None, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_roundtrips_with_flat_kind() {
        let json = r#"{"v":1,"id":"01A","ts":1000,"source":"claude-code","project":"p","session":"s","kind":"prompt","chars":40,"relevance_pm":600,"novelty_bytes":90}"#;
        let e: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(e.kind, Kind::Prompt { relevance_pm: 600, .. }));
        assert_eq!(serde_json::to_string(&e).unwrap(), json);
    }

    #[test]
    fn human_action_classification() {
        assert!(!ev(Kind::TestRun { cmd_hash: "x".into(), exit_code: 1, initiator: Actor::Agent }).is_human_action());
        assert!(ev(Kind::TestRun { cmd_hash: "x".into(), exit_code: 1, initiator: Actor::User }).is_human_action());
        assert!(!ev(Kind::Commit { sha: "a".into(), by: Actor::Agent, lines_changed: 5 }).is_human_action());
        assert!(!ev(Kind::Idle { end_ts: 5 }).is_human_action());
    }
}
