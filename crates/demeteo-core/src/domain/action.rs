use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ActionKind {
    Read,
    Edit,
    Write,
    RunBash,
}

impl ActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionKind::Read => "read",
            ActionKind::Edit => "edit",
            ActionKind::Write => "write",
            ActionKind::RunBash => "run_bash",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "read" => Some(ActionKind::Read),
            "edit" => Some(ActionKind::Edit),
            "write" => Some(ActionKind::Write),
            "run_bash" | "bash" | "runbash" => Some(ActionKind::RunBash),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentAction {
    Read { path: String },
    Edit { path: String, content: String },
    Write { path: String, content: String },
    RunBash { cmd: String },
}

impl AgentAction {
    pub fn kind(&self) -> ActionKind {
        match self {
            AgentAction::Read { .. } => ActionKind::Read,
            AgentAction::Edit { .. } => ActionKind::Edit,
            AgentAction::Write { .. } => ActionKind::Write,
            AgentAction::RunBash { .. } => ActionKind::RunBash,
        }
    }

    pub fn target(&self) -> &str {
        match self {
            AgentAction::Read { path } => path,
            AgentAction::Edit { path, .. } => path,
            AgentAction::Write { path, .. } => path,
            AgentAction::RunBash { cmd } => cmd,
        }
    }
}
