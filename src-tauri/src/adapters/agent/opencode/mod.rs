//! `anomalyco/opencode` agent — the open-source coding agent. Demeteo's
//! integration targets this project; we are not affiliated with it.
//!
//! Wire format: `opencode acp` speaks ACP over stdio.

use crate::adapters::agent::acp::runtime::AcpRuntime;

pub const OPENCODE_INSTALL: &str = "curl -fsSL https://opencode.ai/install | bash";

pub fn runtime() -> AcpRuntime {
    AcpRuntime::new("opencode", OPENCODE_INSTALL)
}
