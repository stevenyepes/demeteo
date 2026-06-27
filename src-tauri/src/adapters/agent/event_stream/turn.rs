use std::sync::Arc;

use crate::domain::agent_event::AgentEvent;
use crate::domain::usage::UsageAccumulator;
use crate::ports::agent_runtime::AgentSession;
use crate::ports::execution::ExecutionPort;
use crate::ports::pricing::PricingTable;
use tokio::sync::watch;
use tokio_stream::StreamExt;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub fast_timeout_s: u64,
    pub normal_timeout_s: u64,
    pub wall_cap_s: u64,
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub text: String,
    pub produced_artifacts: Vec<crate::domain::artifact::Artifact>,
    pub cost_usd: f64,
    pub tokens: i64,
    /// Tokens served from prompt cache (priced at ~10% of base). Surfaced
    /// to the UI as a separate field; not aggregated into `tokens`.
    pub cache_read_input_tokens: u64,
    /// Tokens written to prompt cache (priced above base). Surfaced
    /// separately; not aggregated into `tokens`.
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum TurnResult {
    Success(TurnOutcome),
    Interrupted,
    Failed(String),
}

/// Drive a single agent turn: stream events, accumulate usage, time out.
///
/// `model` and `pricing` are used by the [`UsageAccumulator`] to compute
/// a fallback USD cost when the agent's wire format omits `cost_usd`.
/// Both are `None`/default when the call site doesn't have them — the
/// accumulator then leaves `cost_usd` at `0.0`.
#[allow(clippy::too_many_arguments)]
pub async fn stream_agent_turn<F>(
    session: &dyn AgentSession,
    prompt: &str,
    timeouts: Timeouts,
    mut cancel_watch: Option<watch::Receiver<bool>>,
    machine_str: &str,
    exec: &dyn ExecutionPort,
    model: Option<String>,
    pricing: Arc<dyn PricingTable>,
    mut on_event: F,
) -> TurnResult
where
    F: FnMut(&AgentEvent),
{
    let hb = session.stderr_heartbeat();
    let mut stream = session.prompt(prompt);
    let mut first_event_seen = false;
    let mut text_buffer = String::new();
    let mut produced_artifacts = Vec::new();
    let mut acc = UsageAccumulator::new(model);
    let mut run_failed = None;
    let mut run_cancelled = false;

    let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(timeouts.fast_timeout_s));
    let normal_sleep =
        tokio::time::sleep(std::time::Duration::from_secs(timeouts.normal_timeout_s));
    let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(timeouts.wall_cap_s));
    tokio::pin!(fast_sleep);
    tokio::pin!(normal_sleep);
    tokio::pin!(wall_sleep);

    let start_instant = std::time::Instant::now();

    loop {
        tokio::select! {
            event_opt = stream.next() => {
                let event = match event_opt {
                    Some(ev) => ev,
                    None => break,
                };
                first_event_seen = true;

                let now = tokio::time::Instant::now();
                let next_fast = now + std::time::Duration::from_secs(timeouts.fast_timeout_s);
                let next_normal = now + std::time::Duration::from_secs(timeouts.normal_timeout_s);
                fast_sleep.as_mut().reset(next_fast);
                normal_sleep.as_mut().reset(next_normal);

                on_event(&event);

                match &event {
                    AgentEvent::Text { delta } => {
                        let is_tool_breadcrumb = delta.starts_with("[tool ") || delta.starts_with("[tool:");
                        if !is_tool_breadcrumb {
                            text_buffer.push_str(delta);
                        }
                    }
                    AgentEvent::ArtifactProduced { artifact } => {
                        produced_artifacts.push(artifact.clone());
                    }
                    AgentEvent::TurnComplete { .. } => break,
                    AgentEvent::Error { message, .. } => {
                        let descriptive = crate::adapters::step_executor::steps::agent::format_agent_error_message(message, machine_str, exec).await;
                        run_failed = Some(descriptive);
                        break;
                    }
                    _ => {}
                }

                acc.ingest_event(&event);
            }
            _ = &mut fast_sleep => {
                if !first_event_seen {
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.fast_timeout_s),
                    );
                    continue;
                }
                if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > timeouts.fast_timeout_s * 1000) {
                    let msg = format!("Agent blocked: no output for {}s (stdout and stderr both silent)", timeouts.fast_timeout_s);
                    let descriptive = crate::adapters::step_executor::steps::agent::format_agent_error_message(&msg, machine_str, exec).await;
                    run_failed = Some(descriptive);
                    break;
                }
                fast_sleep.as_mut().reset(
                    tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.fast_timeout_s),
                );
            }
            _ = &mut normal_sleep => {
                if let Some(ref h) = hb {
                    if h.last_activity_ago_ms() < timeouts.normal_timeout_s * 1000 {
                        normal_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.normal_timeout_s),
                        );
                        continue;
                    }
                }
                let msg = format!("Agent response timed out (no output for {}s)", timeouts.normal_timeout_s);
                let descriptive = crate::adapters::step_executor::steps::agent::format_agent_error_message(&msg, machine_str, exec).await;
                run_failed = Some(descriptive);
                break;
            }
            _ = &mut wall_sleep => {
                let elapsed = start_instant.elapsed().as_secs();
                run_failed = Some(format!(
                    "Agent step exceeded wall clock cap ({}s / {}s elapsed)",
                    timeouts.wall_cap_s, elapsed,
                ));
                break;
            }
            _ = async {
                if let Some(ref mut cw) = cancel_watch {
                    let _ = cw.changed().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                if let Some(ref cw) = cancel_watch {
                    if *cw.borrow() {
                        let _ = session.cancel();
                        run_cancelled = true;
                        break;
                    }
                }
            }
        }
    }

    if run_cancelled {
        return TurnResult::Interrupted;
    }
    if let Some(err) = run_failed {
        return TurnResult::Failed(err);
    }

    // Resolve cost: prefer agent-supplied cost_usd; fall back to pricing
    // table when the model is known. Idempotent.
    acc.finalize_arc(&pricing);

    TurnResult::Success(TurnOutcome {
        text: text_buffer,
        produced_artifacts,
        cost_usd: acc.cost_usd(),
        tokens: acc.tokens(),
        cache_read_input_tokens: acc.cache_read_input_tokens(),
        cache_creation_input_tokens: acc.cache_creation_input_tokens(),
    })
}
