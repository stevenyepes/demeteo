use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::{empty, Stream};

use crate::domain::agent_event::AgentEvent;
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

/// Default runtime used during Phase 7a so the wiring compiles and
/// `agent_start` returns a structured `AgentStartError::NotFound("noop")`
/// rather than crashing. The real `AcpRuntime` and the `opencode` /
/// `hermes` adapters land in Phase 7b. Retained in Phase 7b for tests
/// and as a fallback when the user hasn't enabled either agent.
pub struct NoopRuntime;

impl AgentRuntime for NoopRuntime {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn is_available(&self, _machine_id: &str) -> bool {
        false
    }

    fn install_command(&self) -> &'static str {
        "echo 'NoopRuntime: real agent adapters land in Phase 7b'"
    }

    fn start(
        &self,
        _ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>
    {
        Box::pin(async {
            Err(AgentStartError::NotFound(self.kind().to_string()))
        })
    }
}

pub struct NoopSession;

impl AgentSession for NoopSession {
    fn session_id(&self) -> &str {
        "noop-session"
    }

    fn prompt(&self, _text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        Box::pin(empty())
    }

    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
}
