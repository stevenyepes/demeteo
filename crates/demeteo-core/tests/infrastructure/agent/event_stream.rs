use super::*;

use crate::adapters::agent::test_stubs::StubExec;
use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::{AgentTimeouts, SessionInfo};
use crate::ports::agent_runtime::AgentSession;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;

/// An [`AgentSession`] that replays a fixed script and then closes the
/// stream — the wire, minus the process.
struct ScriptedSession(Vec<AgentEvent>);

impl AgentSession for ScriptedSession {
    fn session_id(&self) -> &str {
        "scripted"
    }
    fn prompt(&self, _: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        Box::pin(tokio_stream::iter(self.0.clone()))
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn set_mode(&self, _: &str) -> Result<(), String> {
        Err("scripted session has no modes".to_string())
    }
    fn set_config_option(&self, _: &str, _: &str) -> Result<(), String> {
        Err("scripted session has no config options".to_string())
    }
    fn session_info(&self) -> SessionInfo {
        SessionInfo {
            modes: None,
            config_options: None,
            raw: None,
        }
    }
}

/// Unpriced: this suite asserts control flow, never cost. Returning `None`
/// (rather than a fabricated price) keeps a cost assertion from silently
/// passing against a made-up number.
struct NoPricing;

impl crate::ports::pricing::PricingTable for NoPricing {
    fn price_for(&self, _: &str) -> Option<crate::ports::pricing::ModelPrice> {
        None
    }
    fn context_window(&self, _: &str) -> Option<u64> {
        None
    }
    fn known_models(&self) -> Vec<String> {
        vec![]
    }
}

fn run(script: Vec<AgentEvent>) -> (TurnResult, Vec<AgentEvent>) {
    let session = ScriptedSession(script);
    let exec = StubExec;
    let seen = std::sync::Mutex::new(Vec::new());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(stream_agent_turn(
        &session,
        "resolve the conflicts",
        AgentTimeouts::default(),
        None,
        crate::domain::ids::LOCAL_MACHINE,
        &exec,
        None,
        Arc::new(NoPricing),
        |e| seen.lock().unwrap().push(e.clone()),
    ));
    (result, seen.into_inner().unwrap())
}

fn text(s: &str) -> AgentEvent {
    AgentEvent::Text { delta: s.into() }
}

/// Regression: codex has no warning channel — `exec` routes
/// `process_warning` onto the same non-fatal error item as a real per-item
/// failure. Its own backpressure notice therefore arrives mid-turn as an
/// `Error`, and treating every `Error` as terminal killed the turn while the
/// agent was still working. The sync-conflict resolver surfaced it verbatim:
/// "Resolver failed. in-process app-server event stream lagged; dropped 13
/// events", leaving both conflicted files unmerged.
#[test]
fn a_recoverable_error_mid_turn_does_not_fail_the_turn() {
    let (result, seen) = run(vec![
        text("resolving "),
        AgentEvent::Error {
            code: "item_error".into(),
            message: "in-process app-server event stream lagged; dropped 13 events".into(),
            recoverable: true,
            usage: None,
        },
        text("NewTerminalMenu.tsx"),
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: None,
        },
    ]);

    match result {
        TurnResult::Success(outcome) => assert_eq!(outcome.text, "resolving NewTerminalMenu.tsx"),
        other => panic!("expected Success, got {other:?}"),
    }
    assert!(
        seen.iter()
            .any(|e| matches!(e, AgentEvent::Error { recoverable, .. } if *recoverable)),
        "the warning must still reach the caller's on_event, not be swallowed: {seen:?}"
    );
}

#[test]
fn a_non_recoverable_agent_error_still_fails_the_turn() {
    let (result, _) = run(vec![
        text("working"),
        AgentEvent::Error {
            code: "cli_error".into(),
            message: "context window exceeded".into(),
            recoverable: false,
            usage: None,
        },
    ]);

    match result {
        TurnResult::Failed { reason, .. } => {
            assert!(reason.contains("context window exceeded"), "got: {reason}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// The three codes `drain_lines` mints for a transport that died are
/// environmental — the driver must not route them into a re-implementation
/// loop.
#[test]
fn a_lost_stream_is_environmental_not_an_implementation_failure() {
    let (result, _) = run(vec![AgentEvent::Error {
        code: "agent_stream_lost".into(),
        message: "lost the agent's output stream while the agent was still running".into(),
        recoverable: false,
        usage: None,
    }]);

    assert!(
        matches!(result, TurnResult::Environmental { .. }),
        "got: {result:?}"
    );
}

/// A turn that failed still bought what it read.
///
/// The harnesses go out of their way to put usage on an error result — the
/// claude adapter's `parse_claude_result_event` says so in as many words —
/// and the loop then returned the reason alone and dropped the accumulator.
/// So the turns whose spend is hardest to notice, the ones that ended without
/// reporting back, were exactly the ones billed at zero.
#[test]
fn a_failed_turn_carries_what_it_spent() {
    let (result, _) = run(vec![
        AgentEvent::Usage(crate::domain::agent_event::Usage {
            input_tokens: 400,
            output_tokens: 600,
            cache_read_input_tokens: 70,
            cache_creation_input_tokens: 30,
            cost_usd: Some(0.8),
        }),
        AgentEvent::Error {
            code: "cli_error".into(),
            message: "the agent stopped at its turn cap (--max-turns)".into(),
            recoverable: false,
            usage: None,
        },
    ]);

    match result {
        TurnResult::Failed { spent, .. } => {
            assert_eq!(
                spent.cost_usd, 0.8,
                "the dollars the turn spent are not free"
            );
            assert_eq!(spent.tokens, 1000);
            assert_eq!(spent.cache_read_input_tokens, 70);
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}
