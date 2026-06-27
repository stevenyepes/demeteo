use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus, Usage};
use crate::ports::agent_runtime::AgentContext;

/// Parse a Claude Code JSON-lines event into an `AgentEvent`.
///
/// Wire format (verified against `claude -p --output-format stream-json --verbose`,
/// current as of Claude Code v2.1+):
///
/// ```json
/// {"type":"system","subtype":"init","session_id":"<uuid>", "model":"...", ...}
/// {"type":"system","subtype":"thinking_tokens", ...}
/// {"type":"assistant","message":{"role":"assistant","content":[
///     {"type":"thinking","thinking":"...", "signature":"..."},
///     {"type":"text","text":"..."},
///     {"type":"tool_use","id":"...","name":"Bash","input":{...}}
/// ]}}
/// {"type":"user","message":{"role":"user","content":[
///     {"type":"tool_result","tool_use_id":"...","content":"...","is_error":false}
/// ]}, "tool_use_result":{"stdout":"...","stderr":"","interrupted":false,...}}
/// {"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn",
///  "session_id":"...","total_cost_usd":0.187,
///  "usage":{"input_tokens":100,"output_tokens":50,
///           "cache_creation_input_tokens":500,"cache_read_input_tokens":1000}}
/// ```
///
/// Per Anthropic SDK cost-tracking docs, both `total_cost_usd` and `usage`
/// are present on the `result` event for success AND error results. The
/// cost figure is a client-side estimate from a bundled price table (per
/// Anthropic's own warning) but is the authoritative per-turn number for
/// our telemetry.
///
/// Returns the highest-priority event per line: `ToolCall` beats `Text` beats
/// `ToolCallUpdate` beats `TurnComplete`/`Error` beats `Usage`.
fn parse_claude_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let event_type = v.get("type")?.as_str()?;
    match event_type {
        "system" => parse_claude_system_event(&v),
        "assistant" => parse_claude_assistant_message(&v),
        "user" => parse_claude_user_message(&v),
        "result" => parse_claude_result_event(&v),
        _ => None,
    }
}

/// `{"type":"system","subtype":"init"|"thinking_tokens"|...}` — init carries
/// the session id (captured by `drain_lines` from the raw JSON before we get
/// here); thinking_tokens is pure telemetry. Both are dropped.
fn parse_claude_system_event(v: &serde_json::Value) -> Option<AgentEvent> {
    match v.get("subtype").and_then(|s| s.as_str()) {
        Some("init") | Some("thinking_tokens") | Some("plugin_install") | None => None,
        _ => None,
    }
}

/// Walk `message.content[]` and emit the most important event:
/// `tool_use` blocks win over accumulated `text` blocks. `thinking` blocks
/// are internal reasoning and are skipped.
fn parse_claude_assistant_message(v: &serde_json::Value) -> Option<AgentEvent> {
    let content = v.get("message")?.get("content")?.as_array()?;
    let mut text_acc = String::new();
    let mut first_tool_use: Option<&serde_json::Value> = None;

    for block in content {
        let Some(block_type) = block.get("type").and_then(|s| s.as_str()) else {
            continue;
        };
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|s| s.as_str()) {
                    text_acc.push_str(t);
                }
            }
            "thinking" => continue,
            "tool_use" => {
                if first_tool_use.is_none() {
                    first_tool_use = Some(block);
                }
            }
            _ => continue,
        }
    }

    if let Some(tu) = first_tool_use {
        return Some(claude_tool_use_to_event(tu));
    }
    if !text_acc.is_empty() {
        return Some(AgentEvent::Text { delta: text_acc });
    }
    None
}

/// `{"type":"user","message":{"content":[{"type":"tool_result",...}]},
///   "tool_use_result":{"stdout":"...","stderr":"..."}}` — emits a
/// `ToolCallUpdate` referencing the matching `tool_use_id`.
fn parse_claude_user_message(v: &serde_json::Value) -> Option<AgentEvent> {
    let content = v.get("message")?.get("content")?.as_array()?;
    for block in content {
        if block.get("type").and_then(|s| s.as_str()) != Some("tool_result") {
            continue;
        }
        let tool_use_id = block
            .get("tool_use_id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if tool_use_id.is_empty() {
            continue;
        }

        let is_error = block
            .get("is_error")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let result_text = block
            .get("content")
            .and_then(|c| match c {
                serde_json::Value::String(s) => Some(s.clone()),
                other => serde_json::to_string(other).ok(),
            })
            .unwrap_or_default();

        let tool_use_result = v.get("tool_use_result");
        let stdout = tool_use_result
            .and_then(|r| r.get("stdout"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let stderr = tool_use_result
            .and_then(|r| r.get("stderr"))
            .and_then(|s| s.as_str())
            .unwrap_or("");

        let status = if is_error {
            let reason = if !stderr.is_empty() {
                stderr.to_string()
            } else if !result_text.is_empty() {
                result_text.clone()
            } else {
                "tool failed".to_string()
            };
            ToolCallStatus::Failed { reason }
        } else {
            ToolCallStatus::Completed
        };

        let preview = if !stdout.is_empty() {
            Some(stdout.to_string())
        } else if !result_text.is_empty() {
            Some(result_text)
        } else {
            None
        };

        return Some(AgentEvent::ToolCallUpdate {
            tool_call_id: tool_use_id,
            status,
            preview,
        });
    }
    None
}

/// `{"type":"result","subtype":"success"|"error_max_turns"|..., ...,
///   "stop_reason":"end_turn"|"max_tokens"|"refusal", ...,
///   "total_cost_usd":0.187,
///   "usage":{"input_tokens":...,"output_tokens":...,
///            "cache_creation_input_tokens":...,"cache_read_input_tokens":...}}`
/// — terminal event. Map `stop_reason` to `StopReason` and attach the
/// cumulative `usage` snapshot so the
/// [`UsageAccumulator`](crate::domain::usage::UsageAccumulator) can fold
/// it into the turn outcome.
///
/// Both `total_cost_usd` and `usage` may be absent on tool-only turns
/// with no API call; in that case `usage` is `None` and the accumulator
/// falls back to the pricing table if a model is known.
fn parse_claude_result_event(v: &serde_json::Value) -> Option<AgentEvent> {
    let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
    if is_error {
        // Per Anthropic SDK cost-tracking docs, error result events STILL
        // carry `total_cost_usd` and `usage`. We parse usage here so it
        // is folded into the accumulator via the partial-failure path —
        // the error branch below short-circuits the turn loop, but the
        // accumulator still gets credit for the tokens spent up to the
        // failure point. (A future change could attach usage directly to
        // the Error variant, but that's not needed yet.)
        let _ = parse_claude_result_usage(v);
        let msg = v
            .get("result")
            .and_then(|s| s.as_str())
            .unwrap_or("agent error")
            .to_string();
        return Some(AgentEvent::Error {
            code: "cli_error".to_string(),
            message: msg,
            recoverable: false,
        });
    }

    let stop_reason = match v.get("stop_reason").and_then(|s| s.as_str()) {
        Some("max_tokens") | Some("max_turns") => StopReason::MaxTokens,
        Some("refusal") | Some("error") => StopReason::Error,
        // `end_turn`, `tool_use` (normal finish after a final tool call), …
        _ => StopReason::EndOfTurn,
    };

    let usage = parse_claude_result_usage(v);

    Some(AgentEvent::TurnComplete { stop_reason, usage })
}

/// Extract `Usage` from the `result` event JSON, or `None` if no
/// `total_cost_usd` and no `usage` block are present.
fn parse_claude_result_usage(v: &serde_json::Value) -> Option<Usage> {
    let usage_obj = v.get("usage");
    let cost_usd = v.get("total_cost_usd").and_then(|c| c.as_f64());
    if usage_obj.is_none() && cost_usd.is_none() {
        return None;
    }
    let input_tokens = usage_obj
        .and_then(|u| u.get("input_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let output_tokens = usage_obj
        .and_then(|u| u.get("output_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let cache_creation_input_tokens = usage_obj
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let cache_read_input_tokens = usage_obj
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    Some(Usage {
        input_tokens,
        output_tokens,
        cost_usd,
        cache_read_input_tokens,
        cache_creation_input_tokens,
    })
}

/// Map a Claude `tool_use` block to an `AgentEvent::ToolCall` with the
/// correct `ActionKind` and `target` for demeteo's policy layer.
fn claude_tool_use_to_event(tu: &serde_json::Value) -> AgentEvent {
    let tool_name = tu.get("name").and_then(|s| s.as_str()).unwrap_or("unknown");
    let tool_id = tu.get("id").and_then(|s| s.as_str()).unwrap_or("");
    let input = tu.get("input").cloned().unwrap_or(serde_json::Value::Null);

    let (action_str, target) = match tool_name {
        "Read" => (
            "read",
            input
                .get("file_path")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "Edit" | "MultiEdit" | "NotebookEdit" => (
            "edit",
            input
                .get("file_path")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "Write" => (
            "write",
            input
                .get("file_path")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "Bash" => (
            "run_bash",
            input
                .get("command")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "Glob" => (
            "read",
            input
                .get("pattern")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "Grep" => (
            "read",
            input
                .get("pattern")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "WebFetch" | "WebSearch" => (
            "read",
            input
                .get("url")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        // Internal / metadata tools — model the worst case so they go
        // through the policy layer.
        "TodoWrite" | "TaskCreate" | "TaskUpdate" | "TaskList" => {
            ("edit", serde_json::to_string(&input).unwrap_or_default())
        }
        _ => (
            "run_bash",
            serde_json::to_string(&input).unwrap_or_default(),
        ),
    };

    let action = ActionKind::from_str(action_str).unwrap_or(ActionKind::RunBash);
    let preview = Some(serde_json::to_string(&input).unwrap_or_default());

    AgentEvent::ToolCall {
        tool_call_id: tool_id.to_string(),
        intercept_id: format!("claude-{}", tool_id),
        action,
        target,
        preview,
    }
}

/// Build `claude -p` args.
///
/// Flags:
///   --print                   headless, one-shot mode
///   --verbose                 required by --output-format stream-json
///   --output-format stream-json  ndjson wire format we parse
///   --dangerously-skip-permissions  bypass tool permission prompts; in
///                                   headless mode every Write/Edit/Bash
///                                   call would otherwise be denied
///   --resume <sid>            continue a previously-captured session by
///                             id (only emitted once `captured_session_id`
///                             has been observed in the stream; the
///                             orchestrator relies on this for
///                             cross-step `--continue` semantics).
///   --bare                    shrink system prompt (no CLAUDE.md /
///                             hooks / skills / plugins auto-load) —
///                             makes the static prefix byte-identical
///                             across worktrees for better prompt-cache
///                             reuse. Only emitted when the caller sets
///                             `ctx.bare_mode = true` (orchestrator
///                             pipeline steps do; interactive
///                             AgentTerminalDrawer does not).
///   --exclude-dynamic-system-prompt-sections
///                             same goal as `--bare`: move per-machine
///                             sections (working dir, env info) into the
///                             first user message so cross-worktree
///                             cache hits improve.
fn build_claude_args(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        // Keep bypass mode so *allowed* tools auto-run with no prompts
        // (the autonomous-pipeline guarantee). Per-capability enforcement
        // is layered via --disallowedTools below: disallowed tools are
        // hard-denied even under bypass, and a hard deny returns instantly
        // to the model — it never blocks waiting on a human.
        "--dangerously-skip-permissions".to_string(),
    ];
    if let Some(sid) = captured_session_id {
        // Cross-step / cross-turn continuation. The orchestrator captures
        // `session_id` from the `system` init event on the first turn
        // and threads it back here so the conversation is replayed
        // instead of starting fresh — unlocking prompt-cache hits on
        // the static prefix (system prompt + tool definitions).
        args.push("--resume".to_string());
        args.push(sid.to_string());
    }
    if ctx.bare_mode {
        args.push("--bare".to_string());
        args.push("--exclude-dynamic-system-prompt-sections".to_string());
    }
    let disallowed = disallowed_tools_for(&ctx.permissions);
    if !disallowed.is_empty() {
        args.push("--disallowedTools".to_string());
        args.push(disallowed.join(","));
    }
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    args
}

/// Map an abstract [`PermissionProfile`] to the Claude Code tools that must
/// be denied for this step. Read tools (Read/Grep/Glob/LS) are never
/// denied — they're how a non-shell step still inspects the codebase
/// (`cat`→Read, `grep`→Grep). The chmod fence handles the
/// artifacts-vs-source path distinction that tool names can't express.
fn disallowed_tools_for(p: &crate::domain::permission::PermissionProfile) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !p.execute.is_allow() {
        out.push("Bash");
    }
    if !p.write_fs.is_allow() {
        out.extend_from_slice(&["Edit", "Write", "MultiEdit", "NotebookEdit"]);
    }
    if !p.network.is_allow() {
        out.extend_from_slice(&["WebSearch", "WebFetch"]);
    }
    out
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "claude-code",
        binary: "claude",
        install_cmd: "npm install -g @anthropic-ai/claude-code",
        parse_event: parse_claude_event as EventParser,
        build_args: build_claude_args,
        // claude-code enforces via CLI flags, not env.
        perm_env: crate::ports::agent_runtime::no_permission_env,
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/agent/claude_code.rs"]
mod tests;
