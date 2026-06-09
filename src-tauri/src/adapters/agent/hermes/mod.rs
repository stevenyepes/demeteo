//! Hermes agent from Nous Research. Speaks ACP via `hermes acp` over stdio.

use crate::adapters::agent::acp::runtime::AcpRuntime;

pub const HERMES_INSTALL: &str = "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash";

pub fn runtime() -> AcpRuntime {
    AcpRuntime::new("hermes", HERMES_INSTALL)
}
