//! ACP (Agent Client Protocol) runtime. This is Demeteo's implementation of
//! the v1 client side of the protocol — Demeteo is the ACP *client*, the
//! agent (opencode, Hermes) is the *server*. JSON-RPC 2.0 over newline-
//! delimited JSON, one message per line on stdin/stdout of the agent
//! subprocess (or over a long-lived SSH channel for remote agents).
//!
//! Module layout:
//! - `runtime`         — `AcpRuntime` (lifecycle: initialize, session/new,
//!                        prompt, cancel) and `AcpSession` (the per-turn
//!                        `AgentEvent` stream).
//! - `jsonrpc`         — the line-buffered JSON-RPC 2.0 client. Newline-
//!                        delimited, hand-rolled (no `agent-client-protocol`
//!                        crate dep so we stay light and testable).
//! - `transport_local` — `LocalSubprocessTransport` (wraps
//!                        `std::process::Child`).
//! - `transport_ssh`   — `RemoteSshTransport` (wraps `ssh2::Channel` over
//!                        the existing `SshClientAdapter::spawn_interactive`).
//! - `event_mapper`    — ACP `session/update` / `tool_call/update` /
//!                        `session/usage_update` → `AgentEvent`.
//! - `tool_bridge`     — agent-initiated `fs/read_text_file` etc. →
//!                        `PolicyEnforcedExecutionPort::submit_agent` with
//!                        the `tool_call_id` recorded, so a rejection
//!                        surfaces as `tool_call/update { status: Failed }`.
//! - `install`         — `run_official_install` (the consent-driven
//!                        installer over local subprocess or SSH).
//! - `mock_agent`      — the in-tree mock ACP agent used by integration
//!                        tests. Lives in the source tree so we can verify
//!                        the full lifecycle without an external binary.

pub mod event_mapper;
pub mod install;
#[cfg(test)]
mod integration_tests;
pub mod jsonrpc;
pub mod mock_agent;
pub mod runtime;
pub mod tool_bridge;
pub mod transport_local;
pub mod transport_ssh;
