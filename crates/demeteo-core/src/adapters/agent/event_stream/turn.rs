use std::sync::Arc;

use crate::domain::agent_event::AgentEvent;
use crate::domain::models::AgentTimeouts;
use crate::domain::usage::UsageAccumulator;
use crate::ports::agent_runtime::AgentSession;
use crate::ports::execution::ExecutionPort;
use crate::ports::pricing::PricingTable;
use tokio::sync::watch;
use tokio_stream::StreamExt;

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
    /// The agent itself reported an error (CLI error event). Retrying the
    /// work with feedback may help.
    Failed(String),
    /// The orchestrator killed or lost the turn for environmental reasons
    /// — silence timeout, wall-clock cap, spawn failure, process crash.
    /// The implementation is not at fault; callers must not route this
    /// into an on_failure re-implementation loop.
    Environmental(String),
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
    timeouts: AgentTimeouts,
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
    let mut run_failed: Option<TurnResult> = None;
    let mut run_cancelled = false;
    // Tool calls the agent has issued but not yet resolved. While any are
    // in flight the wire is legitimately silent — a `cargo build` or a big
    // test suite produces no stdout events between `tool_use` and
    // `tool_result`, and typically nothing on stderr either. Firing the
    // fast/normal silence timeouts in that window kills healthy turns and
    // masquerades as an implementation failure downstream. Only the wall
    // cap bounds an in-flight tool call.
    let mut pending_tool_calls: std::collections::HashSet<String> =
        std::collections::HashSet::new();

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

                acc.ingest_event(&event);

                match &event {
                    AgentEvent::Text { delta } => {
                        let is_tool_breadcrumb = delta.starts_with("[tool ") || delta.starts_with("[tool:");
                        if !is_tool_breadcrumb {
                            text_buffer.push_str(delta);
                        }
                    }
                    AgentEvent::ToolCall { tool_call_id, .. } => {
                        pending_tool_calls.insert(tool_call_id.clone());
                    }
                    AgentEvent::ToolCallUpdate {
                        tool_call_id,
                        status,
                        ..
                    } => {
                        if matches!(
                            status,
                            crate::domain::agent_event::ToolCallStatus::Completed
                                | crate::domain::agent_event::ToolCallStatus::Failed { .. }
                        ) {
                            pending_tool_calls.remove(tool_call_id);
                        }
                    }
                    AgentEvent::ArtifactProduced { artifact } => {
                        produced_artifacts.push(artifact.clone());
                    }
                    AgentEvent::TurnComplete { .. } => break,
                    AgentEvent::Error { message, code, .. } => {
                        let descriptive = crate::adapters::step_executor::steps::agent::format_agent_error_message(message, machine_str, exec).await;
                        // A process that couldn't spawn, died with a
                        // non-zero exit, or whose output stream we lost
                        // mid-turn is an environment problem, not
                        // something re-implementing the code can fix. A
                        // `cli_error` is the agent's own reported failure
                        // — that one is feedback-worthy.
                        let environmental = code == "spawn_failed"
                            || code == "agent_exit_nonzero"
                            || code == "agent_stream_lost";
                        run_failed = Some(if environmental {
                            TurnResult::Environmental(descriptive)
                        } else {
                            TurnResult::Failed(descriptive)
                        });
                        break;
                    }
                    _ => {}
                }
            }
            _ = &mut fast_sleep => {
                if !first_event_seen || !pending_tool_calls.is_empty() {
                    // Startup, or a tool call is in flight — silence is
                    // expected; the wall cap is the only bound here.
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.fast_timeout_s),
                    );
                    continue;
                }
                if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > timeouts.fast_timeout_s * 1000) {
                    let msg = format!("Agent blocked: no output for {}s (stdout and stderr both silent)", timeouts.fast_timeout_s);
                    let descriptive = crate::adapters::step_executor::steps::agent::format_agent_error_message(&msg, machine_str, exec).await;
                    run_failed = Some(TurnResult::Environmental(descriptive));
                    break;
                }
                fast_sleep.as_mut().reset(
                    tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.fast_timeout_s),
                );
            }
            _ = &mut normal_sleep => {
                if !pending_tool_calls.is_empty() {
                    normal_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeouts.normal_timeout_s),
                    );
                    continue;
                }
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
                run_failed = Some(TurnResult::Environmental(descriptive));
                break;
            }
            _ = &mut wall_sleep => {
                let elapsed = start_instant.elapsed().as_secs();
                run_failed = Some(TurnResult::Environmental(format!(
                    "Agent step exceeded wall clock cap ({}s / {}s elapsed)",
                    timeouts.wall_cap_s, elapsed,
                )));
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
        return err;
    }

    // Resolve cost: prefer agent-supplied cost_usd; fall back to pricing
    // table when the model is known. Idempotent.
    acc.finalize_arc(&pricing);

    // Strip extended-thinking tags before the text reaches artifact storage
    // or the memory agent. Models that use thinking mode emit <think>…</think>
    // as raw text deltas; they are internal reasoning, not user-facing output.
    let text = crate::domain::text::strip_think_tags(&text_buffer);

    // Cache profile for this turn. The hit ratio is the share of the prompt
    // served from the vendor's prompt cache (~10% of base price); a resumed
    // turn where `cache_creation` dominates instead means the gap since the
    // previous turn outlived the cache TTL and the whole transcript was
    // re-billed at cache-*write* price. Watching this ratio across steps is
    // the evidence for (or against) keeping agent processes alive between
    // turns rather than respawning with `--resume`.
    let cache_read = acc.cache_read_input_tokens();
    let cache_creation = acc.cache_creation_input_tokens();
    let uncached_input = acc.input_tokens();
    let prompt_total = uncached_input + cache_read + cache_creation;
    if prompt_total > 0 {
        tracing::info!(
            session_id = session.session_id(),
            uncached_input,
            cache_read,
            cache_creation,
            cache_hit_ratio = (cache_read as f64 / prompt_total as f64 * 100.0).round() / 100.0,
            "turn cache profile"
        );
    }

    TurnResult::Success(TurnOutcome {
        text,
        produced_artifacts,
        cost_usd: acc.cost_usd(),
        tokens: acc.tokens(),
        cache_read_input_tokens: acc.cache_read_input_tokens(),
        cache_creation_input_tokens: acc.cache_creation_input_tokens(),
    })
}
