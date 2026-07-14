//! OpenAI Codex CLI adapter.
//!
//! Wire format: `codex exec "<prompt>" --json` emits an nd-JSON event stream on
//! stdout. Verified against `codex-cli 0.142.3`. Unlike the opencode/hermes/
//! claude-code families, Codex wraps everything in a small
//! `thread` / `turn` / `item` envelope:
//!
//! ```json
//! {"type":"thread.started","thread_id":"019f4ce2-2d91-7442-be57-b599da8e827b"}
//! {"type":"turn.started"}
//! {"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"..."}}
//! {"type":"item.started","item":{"id":"item_2","type":"command_execution","command":"...","aggregated_output":"","exit_code":null,"status":"in_progress"}}
//! {"type":"item.completed","item":{"id":"item_2","type":"command_execution","command":"...","aggregated_output":"...\n","exit_code":0,"status":"completed"}}
//! {"type":"item.completed","item":{"id":"item_3","type":"file_change","changes":[{"path":"hello.txt","kind":"update"}],"status":"completed"}}
//! {"type":"item.completed","item":{"id":"item_0","type":"error","message":"Model metadata ... not found. ..."}}
//! {"type":"turn.completed","usage":{"input_tokens":22039,"cached_input_tokens":11008,"output_tokens":75,"reasoning_output_tokens":0}}
//! ```
//!
//! The `thread_id` on the first line is the session identifier; the shared read
//! loop (`cli_runtime::drain_lines`) captures it from the raw JSON and threads
//! it back into [`build_codex_args`] as `codex exec resume <id>` for cross-step
//! continuity (decision 36 / `AGENT_INTEGRATION.md` §5.3). Codex has no session
//! flag on the *initial* turn — persistence is automatic (rollout files under
//! `~/.codex/sessions`), and `resume` replays the thread.

use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus, Usage};
use crate::domain::models::{AgentKind, EffortLevel};
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::AgentContext;

/// `npm i -g @openai/codex` per market research §1 and decision 34's install
/// table. The standalone binary is `codex`.
const CODEX_INSTALL: &str = "npm install -g @openai/codex";

/// Parse one Codex JSONL line into an [`AgentEvent`].
///
/// Returns the highest-priority event per line. `thread.started` /
/// `turn.started` and internal `reasoning` items are dropped (`None`); the
/// `thread_id` is captured upstream by the read loop, not here. Unknown
/// top-level or item types are dropped so a future CLI version that adds an
/// event kind doesn't break the stream.
fn parse_codex_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match v.get("type")?.as_str()? {
        // Session lifecycle — thread_id captured by `drain_lines`, nothing to emit.
        "thread.started" | "turn.started" => None,
        "item.started" => parse_codex_item(v.get("item")?, true),
        "item.completed" | "item.updated" => parse_codex_item(v.get("item")?, false),
        "turn.completed" => Some(AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: parse_codex_turn_usage(&v),
        }),
        // A turn-level failure is terminal for the step.
        "turn.failed" | "thread.error" | "error" => Some(AgentEvent::Error {
            code: "cli_error".to_string(),
            message: codex_error_message(&v),
            recoverable: false,
            usage: None,
        }),
        _ => None,
    }
}

/// Map one `item` object to an event. `started` distinguishes an
/// `item.started` (tool-call *begin*) from an `item.completed` (result).
fn parse_codex_item(item: &serde_json::Value, started: bool) -> Option<AgentEvent> {
    let item_type = item.get("type")?.as_str()?;
    let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("");
    match item_type {
        // Assistant prose. Only ever arrives as a completed item.
        "agent_message" => {
            let text = item.get("text").and_then(|s| s.as_str()).unwrap_or("");
            if text.is_empty() {
                None
            } else {
                Some(AgentEvent::Text {
                    delta: text.to_string(),
                })
            }
        }
        // Internal chain-of-thought — never surfaced (mirrors claude `thinking`).
        "reasoning" => None,
        // Shell command: `started` → ToolCall (begin), `completed` → ToolCallUpdate (result).
        "command_execution" => Some(parse_codex_command(item, id, started)),
        // A patch the agent applied. The write already happened by `completed`;
        // model it as an `edit` ToolCall so the policy layer / artifact
        // collector still sees the mutated path.
        "file_change" if !started => parse_codex_file_change(item, id),
        // A per-item `error`. Two very different things arrive on this channel:
        //
        //   1. The "Model metadata for `<slug>` not found. Defaulting to
        //      fallback metadata..." notice codex emits for *any* model absent
        //      from its bundled catalog — routine the moment a user points
        //      Codex at a custom provider/model (e.g. a MiniMax slug in
        //      ~/.codex/config.toml). It is not an error: codex proceeds with
        //      conservative defaults. Surfacing it as `AgentEvent::Error` made
        //      every custom-model turn show a scary error and could trip
        //      error-reactive consumers (the verifier triage reads any
        //      `Error` as `Regression`). Drop it. A user who needs the real
        //      metadata (so a large-context model isn't compacted early) sets
        //      `model_context_window` / `model_catalog_json` in their codex
        //      config — see `is_metadata_fallback_notice`.
        //
        //   2. A genuine per-item error — surfaced as a recoverable `Error` so
        //      the StepExecutor keeps running rather than aborting the turn.
        "error" => {
            let message = item
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("codex reported an error");
            if is_metadata_fallback_notice(message) {
                None
            } else {
                Some(AgentEvent::Error {
                    code: "item_error".to_string(),
                    message: message.to_string(),
                    recoverable: true,
                    usage: None,
                })
            }
        }
        _ => None,
    }
}

/// Is this the benign "Model metadata for `<slug>` not found. Defaulting to
/// fallback metadata; this can degrade performance and cause issues." notice?
///
/// Codex emits it as an `error` item whenever the active model has no row in
/// its bundled metadata catalog — i.e. for essentially every custom
/// provider/model. It is informational, not a turn failure. Matching on the
/// distinctive `"fallback metadata"` phrase keeps it robust to the exact slug
/// and to minor wording changes across codex versions.
fn is_metadata_fallback_notice(message: &str) -> bool {
    message.contains("fallback metadata")
}

/// `command_execution` item → `ToolCall` (begin) or `ToolCallUpdate` (result).
fn parse_codex_command(item: &serde_json::Value, id: &str, started: bool) -> AgentEvent {
    let command = item
        .get("command")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    if started {
        return AgentEvent::ToolCall {
            tool_call_id: id.to_string(),
            intercept_id: format!("codex-{id}"),
            action: ActionKind::RunBash,
            target: command.clone(),
            preview: Some(command),
        };
    }

    let status_str = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
    let exit_code = item.get("exit_code").and_then(|c| c.as_i64());
    let output = item
        .get("aggregated_output")
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let failed = status_str == "failed" || exit_code.is_some_and(|c| c != 0);
    let status = if failed {
        let reason = if !output.is_empty() {
            output.to_string()
        } else {
            format!("command exited with code {}", exit_code.unwrap_or(-1))
        };
        ToolCallStatus::Failed { reason }
    } else {
        ToolCallStatus::Completed
    };

    AgentEvent::ToolCallUpdate {
        tool_call_id: id.to_string(),
        status,
        preview: (!output.is_empty()).then(|| output.to_string()),
    }
}

/// `file_change` item → an `edit`/`write` `ToolCall` targeting the first
/// changed path. `kind` is one of `add | update | delete`.
fn parse_codex_file_change(item: &serde_json::Value, id: &str) -> Option<AgentEvent> {
    let changes = item.get("changes").and_then(|c| c.as_array())?;
    let first = changes.first()?;
    let path = first
        .get("path")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let action = match first.get("kind").and_then(|s| s.as_str()) {
        Some("add") => ActionKind::Write,
        _ => ActionKind::Edit, // update / delete / unknown
    };
    Some(AgentEvent::ToolCall {
        tool_call_id: id.to_string(),
        intercept_id: format!("codex-{id}"),
        action,
        target: path,
        preview: Some(serde_json::to_string(changes).unwrap_or_default()),
    })
}

/// Extract the cumulative token snapshot from a `turn.completed` line.
///
/// Codex reports `input_tokens` (total, cache-inclusive), `cached_input_tokens`
/// (the cached subset), and `output_tokens`. It emits no per-turn cost, so
/// `cost_usd` is `None` and the [`UsageAccumulator`](crate::domain::usage::UsageAccumulator)
/// falls back to the pricing table.
fn parse_codex_turn_usage(v: &serde_json::Value) -> Option<Usage> {
    let u = v.get("usage")?;
    Some(Usage {
        input_tokens: u.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
        cost_usd: None,
        cache_read_input_tokens: u
            .get("cached_input_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
    })
}

/// Pull a human-readable message out of a `turn.failed` / error envelope,
/// tolerating both `{"error":{"message":...}}` and `{"message":...}`.
fn codex_error_message(v: &serde_json::Value) -> String {
    v.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|s| s.as_str())
        .or_else(|| v.get("message").and_then(|s| s.as_str()))
        .unwrap_or("codex turn failed")
        .to_string()
}

/// Map the abstract [`PermissionProfile`] to a Codex `--sandbox` mode.
///
/// Codex sandbox modes are `read-only | workspace-write | danger-full-access`.
/// We never select `danger-full-access` (it removes the sandbox entirely). The
/// artifacts-vs-source path distinction is still enforced by the OS-level chmod
/// fence in `adapters/worktree/git_ops/scope.rs`, uniformly across every agent.
fn codex_sandbox_mode(p: &PermissionProfile) -> &'static str {
    if p.write_fs.is_allow() {
        "workspace-write"
    } else {
        "read-only"
    }
}

/// Build `codex exec` args for one turn.
///
/// Layout:
///   exec [resume <sid>]        resume replays the captured thread for cross-step
///                              continuity; on the first turn there is no id yet.
///   --json                     nd-JSON wire format we parse.
///   --skip-git-repo-check      worktrees are git repos, but this keeps a
///                              non-git project host from hard-failing at spawn.
///   -c sandbox_mode=<mode>     compiled from the PermissionProfile. Set via `-c`
///                              (not `--sandbox`) because `codex exec resume`
///                              accepts `-c` but not `--sandbox`, so both the
///                              initial and resumed turns share one code path.
///   -c approval_policy=never   the autonomous-pipeline guarantee — a sandbox
///                              denial returns to the model instantly instead of
///                              parking on an approval prompt no human will answer.
///   -c sandbox_workspace_write.network_access=true
///                              only when network is allowed and we're in
///                              workspace-write (workspace-write blocks network
///                              by default).
///   --model <model>            when the step pins one.
///   <prompt>                   trailing positional (passing it via stdin races
///                              Codex's init phase — see build_opencode_args).
fn build_codex_args(
    ctx: &AgentContext,
    captured_session_id: Option<&str>,
    prompt: &str,
) -> Vec<String> {
    let mut args = vec!["exec".to_string()];

    if let Some(sid) = captured_session_id {
        // Cross-step / cross-turn continuation: replay the recorded thread so
        // the static prefix (system prompt + tool defs) hits the vendor
        // prompt-cache instead of starting cold.
        args.push("resume".to_string());
        args.push(sid.to_string());
    }

    args.push("--json".to_string());
    args.push("--skip-git-repo-check".to_string());

    let mode = codex_sandbox_mode(&ctx.permissions);
    args.push("-c".to_string());
    args.push(format!("sandbox_mode={mode}"));
    args.push("-c".to_string());
    args.push("approval_policy=never".to_string());
    if ctx.permissions.network.is_allow() && mode == "workspace-write" {
        args.push("-c".to_string());
        args.push("sandbox_workspace_write.network_access=true".to_string());
    }

    // Effort, via the same `-c` channel (accepted by both `exec` and
    // `exec resume`). Clamped here because codex does *not* validate:
    // an unknown value becomes a `Custom(String)` and goes on the wire.
    if let Some(effort) = ctx
        .effort
        .and_then(|e| EffortLevel::clamp_for(AgentKind::Codex, e))
    {
        args.push("-c".to_string());
        args.push(format!("model_reasoning_effort={}", effort.as_str()));
    }

    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    if !prompt.is_empty() {
        args.push(prompt.to_string());
    }
    args
}

/// Reusable typed-output arg fragment: `--output-schema <schema> -o <out>`.
///
/// Epic B1's BrainPort needs Codex to emit a JSON object matching a schema
/// (structured output over scraping). It is exposed as a standalone builder,
/// per Story A1.1, so B1 can append it to [`build_codex_args`]'s output once
/// `AgentContext` carries an output-schema field — rather than re-deriving the
/// flag spelling. `-o`/`--output-last-message` writes the final message to
/// `out_path`; `--output-schema` constrains its shape.
pub fn codex_output_schema_args(schema_path: &str, out_path: &str) -> Vec<String> {
    vec![
        "--output-schema".to_string(),
        schema_path.to_string(),
        "-o".to_string(),
        out_path.to_string(),
    ]
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "codex",
        binary: "codex",
        install_cmd: CODEX_INSTALL,
        parse_event: parse_codex_event as EventParser,
        build_args: build_codex_args,
        // Codex enforces via the `--sandbox`/`-c sandbox_mode` policy in
        // `build_codex_args`, not an env var (flag-based, like claude-code).
        perm_env: crate::ports::agent_runtime::no_permission_env,
        // Effort rides on argv (`-c model_reasoning_effort=…`), not env.
        effort_env: crate::adapters::agent::cli_runtime::no_effort_env,
        display_label: "Codex",
        // `codex` has no `models` list subcommand; aliases come from the static
        // fallback in `application::agent_probe`.
        lists_models: false,
        // Codex's default model is user-configurable in ~/.codex/config.toml;
        // don't seed a cost fallback that could misprice an overridden model.
        default_model: None,
        effort_levels: EffortLevel::supported_for(AgentKind::Codex),
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/agent/codex.rs"]
mod tests;
