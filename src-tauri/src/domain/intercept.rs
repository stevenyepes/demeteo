use serde::{Deserialize, Serialize};

use super::action::{ActionKind, AgentAction};
use super::ids::{InterceptId, MachineId, ThreadId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptPayload {
    pub intercept_id: InterceptId,
    pub thread_id: ThreadId,
    pub machine_id: MachineId,
    pub action: ActionKind,
    pub target: String,
    pub preview: Option<String>,
    pub created_at: String,
    pub tool_call_id: Option<String>, // NEW: Some(...) for agent-originated; None for hand-rolled
}

impl InterceptPayload {
    pub fn from_action(
        intercept_id: InterceptId,
        thread_id: ThreadId,
        machine_id: MachineId,
        action: &AgentAction,
    ) -> Self {
        let (preview, target) = match action {
            AgentAction::Read { path } => (None, path.clone()),
            AgentAction::Edit { path, content } => {
                let preview = preview_content(content, 12);
                (Some(preview), path.clone())
            }
            AgentAction::Write { path, content } => {
                let preview = preview_content(content, 12);
                (Some(preview), path.clone())
            }
            AgentAction::RunBash { cmd } => (Some(cmd.clone()), cmd.clone()),
        };
        Self {
            intercept_id,
            thread_id,
            machine_id,
            action: action.kind(),
            target,
            preview,
            created_at: current_iso8601(),
            tool_call_id: None,
        }
    }

    /// Construct an intercept for an agent-originated tool call. The agent's
    /// own id is preserved so the runtime can correlate a later resolution
    /// with the in-flight `tool_call/update` notification.
    pub fn from_agent_tool_call(
        intercept_id: InterceptId,
        thread_id: ThreadId,
        machine_id: MachineId,
        tool_call_id: String,
        action: &AgentAction,
    ) -> Self {
        let mut p = Self::from_action(intercept_id, thread_id, machine_id, action);
        p.tool_call_id = Some(tool_call_id);
        p
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionResult {
    Bash { output: String },
    FileChanged { path: String, lines_added: usize, lines_removed: usize },
    FileRead { path: String, content_preview: String },
}

#[derive(Debug, Clone)]
pub enum Resolution {
    Approve,
    Reject { feedback: String },
    /// The action originated from an agent tool call; signal failure as a
    /// tool-call-shaped result (so the agent runtime can return it as a
    /// `tool_call/update` with status: Failed) rather than a synthetic bash output.
    RejectAsToolFailure { feedback: String },
}

fn preview_content(content: &str, max_lines: usize) -> String {
    let mut s = String::new();
    for (i, line) in content.lines().enumerate() {
        if i >= max_lines {
            s.push_str("...\n");
            break;
        }
        if i > 0 {
            s.push('\n');
        }
        s.push('+');
        s.push(' ');
        s.push_str(line);
    }
    s
}

fn current_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}Z", secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::action::AgentAction;

    #[test]
    fn preview_truncates_long_content() {
        let big = (0..50).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let p = InterceptPayload::from_action(
            "i1".into(),
            "t1".into(),
            "m1".into(),
            &AgentAction::Edit { path: "/x".into(), content: big },
        );
        let preview = p.preview.unwrap();
        assert!(preview.contains("..."));
        assert!(preview.lines().count() <= 13);
    }

    #[test]
    fn bash_target_is_full_command() {
        let p = InterceptPayload::from_action(
            "i1".into(),
            "t1".into(),
            "m1".into(),
            &AgentAction::RunBash { cmd: "cargo build --release".into() },
        );
        assert_eq!(p.target, "cargo build --release");
        assert_eq!(p.preview.as_deref(), Some("cargo build --release"));
        assert!(p.tool_call_id.is_none());
    }

    #[test]
    fn from_agent_tool_call_records_tool_call_id() {
        let p = InterceptPayload::from_agent_tool_call(
            "i1".into(),
            "t1".into(),
            "m1".into(),
            "tc-99".into(),
            &AgentAction::Read { path: "/x".into() },
        );
        assert_eq!(p.tool_call_id.as_deref(), Some("tc-99"));
    }
}
