//! pi coding agent (`@earendil-works/pi-coding-agent`), in `--mode json`.
//!
//! Wire format, verified against pi 0.83.0; the captured transcripts are in
//! `crates/demeteo-core/tests/fixtures/agent_transcripts/pi/0.83.0/`:
//!
//! ```json
//! {"type":"session","version":3,"id":"019fbb89-…","timestamp":"…","cwd":"/path"}
//! {"type":"agent_start"}
//! {"type":"turn_start"}
//! {"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"pong","partial":{…}},"message":{…}}
//! {"type":"tool_execution_start","toolCallId":"call_…","toolName":"ls","args":{"path":"."}}
//! {"type":"tool_execution_end","toolCallId":"call_…","toolName":"ls","result":{"content":[{"type":"text","text":"…"}]},"isError":false}
//! {"type":"turn_end","message":{…,"usage":{…},"stopReason":"toolUse"},"toolResults":[…]}
//! {"type":"agent_end","messages":[…],"willRetry":false}
//! {"type":"agent_settled"}
//! ```
//!
//! Two shapes differ from every other adapter here and both are load-bearing:
//!
//! - **`turn_end.message.usage` counts one model request, not the turn.** pi
//!   emits a `turn_end` per request and resets the counters, so it maps to
//!   [`AgentEvent::UsageDelta`] and never [`AgentEvent::Usage`].
//! - **`agent_end` is the terminal event, not `turn_end`**, and only when
//!   `willRetry == false`: pi retries API errors itself and emits another
//!   `agent_end` after the retry. `agent_settled` trails it and is never
//!   terminal.
//!
//! The session id is the bare `id` on the `{"type":"session",…}` header, read
//! by the shared `cli_runtime::session_id_from_line` (type-guarded there) and
//! threaded back into `build_pi_args` as `--session`.
//!
//! A fatal error (bad model, bad auth) exits 1 with the reason on **stderr**
//! and an empty stdout, so nothing reaches this parser; the failure surfaces
//! through the exit-code path instead.

use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus, Usage};
use crate::domain::models::{AgentKind, EffortLevel, WindowsAgentShell};
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::{AgentContext, ModelListing};

/// `--ignore-scripts` because the package's install script is not needed for
/// the CLI and refusing it keeps a fleet-wide `npm install -g` inert.
const PI_INSTALL: &str = "npm install -g --ignore-scripts @earendil-works/pi-coding-agent";

fn parse_pi_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match v.get("type")?.as_str()? {
        "message_update" => parse_pi_text_delta(&v),
        "tool_execution_start" => parse_pi_tool_start(&v),
        "tool_execution_end" => parse_pi_tool_end(&v),
        "turn_end" => Some(AgentEvent::UsageDelta(parse_pi_usage(
            v.get("message")?.get("usage")?,
        ))),
        "agent_end" => parse_pi_agent_end(&v),
        // pi goes silent while it retries an API error or compacts the
        // context. Surfacing a breadcrumb keeps the silence watchdog in
        // `stream_agent_turn` from reading that pause as a stuck agent.
        "auto_retry_start" | "compaction_start" => Some(AgentEvent::Text {
            delta: pi_breadcrumb(&v),
        }),
        _ => None,
    }
}

/// `message_update` carries the whole assistant-message lifecycle
/// (`text_start` / `text_delta` / `text_end`, and the `toolcall_*` family).
/// Only `text_delta` adds prose: `text_end` repeats the full accumulated
/// `content`, and every event embeds `partial`, so anything wider double-counts
/// the stream. Tool calls arrive again as `tool_execution_start`, which carries
/// the parsed args.
fn parse_pi_text_delta(v: &serde_json::Value) -> Option<AgentEvent> {
    let evt = v.get("assistantMessageEvent")?;
    if evt.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    let delta = evt.get("delta")?.as_str()?;
    if delta.is_empty() {
        return None;
    }
    Some(AgentEvent::Text {
        delta: delta.to_string(),
    })
}

/// Map a `tool_execution_start` to a [`AgentEvent::ToolCall`] the policy layer
/// can gate. pi's built-in set is `read bash edit write grep find ls`; the
/// argument holding the target differs per tool (`path`, `command`, `pattern`).
fn parse_pi_tool_start(v: &serde_json::Value) -> Option<AgentEvent> {
    let tool_call_id = v.get("toolCallId")?.as_str()?.to_string();
    let tool_name = v.get("toolName").and_then(|s| s.as_str()).unwrap_or("");
    let args = v.get("args").cloned().unwrap_or(serde_json::Value::Null);

    let (action, target_key) = match tool_name {
        "read" | "ls" => (ActionKind::Read, "path"),
        "grep" | "find" => (ActionKind::Read, "pattern"),
        "edit" => (ActionKind::Edit, "path"),
        "write" => (ActionKind::Write, "path"),
        "bash" => (ActionKind::RunBash, "command"),
        // An extension tool, or a built-in added after 0.83.0. Model the worst
        // case so it still goes through the policy layer.
        _ => (ActionKind::RunBash, "command"),
    };
    let target = args
        .get(target_key)
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    Some(AgentEvent::ToolCall {
        tool_call_id: tool_call_id.clone(),
        intercept_id: format!("pi-{tool_call_id}"),
        action,
        target,
        preview: Some(serde_json::to_string(&args).unwrap_or_default()),
    })
}

fn parse_pi_tool_end(v: &serde_json::Value) -> Option<AgentEvent> {
    let tool_call_id = v.get("toolCallId")?.as_str()?.to_string();
    let is_error = v.get("isError").and_then(|b| b.as_bool()).unwrap_or(false);
    let text = pi_result_text(v.get("result"));

    let status = if is_error {
        let reason = if text.is_empty() {
            "tool failed".to_string()
        } else {
            text.clone()
        };
        ToolCallStatus::Failed { reason }
    } else {
        ToolCallStatus::Completed
    };

    Some(AgentEvent::ToolCallUpdate {
        tool_call_id,
        status,
        preview: (!text.is_empty()).then_some(text),
    })
}

fn pi_result_text(result: Option<&serde_json::Value>) -> String {
    let Some(content) = result
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
    else {
        return String::new();
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_pi_usage(u: &serde_json::Value) -> Usage {
    Usage {
        input_tokens: u.get("input").and_then(|t| t.as_u64()).unwrap_or(0),
        output_tokens: u.get("output").and_then(|t| t.as_u64()).unwrap_or(0),
        cost_usd: u
            .get("cost")
            .and_then(|c| c.get("total"))
            .and_then(|c| c.as_f64()),
        cache_read_input_tokens: u.get("cacheRead").and_then(|t| t.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: u.get("cacheWrite").and_then(|t| t.as_u64()).unwrap_or(0),
    }
}

/// `agent_end` closes the stream — but a `willRetry: true` one is pi announcing
/// its own retry, after which a second `agent_end` follows. Treating the first
/// as terminal would tear the session down mid-retry.
///
/// `usage` stays `None`: the per-request figures already went out as
/// [`AgentEvent::UsageDelta`], and the accumulator applies a `TurnComplete`
/// snapshot with last-write-wins on cost — attaching one here would replace the
/// summed total with the final request's cost.
fn parse_pi_agent_end(v: &serde_json::Value) -> Option<AgentEvent> {
    if v.get("willRetry")
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let stop_reason = v
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|m| m.last())
        .and_then(|m| m.get("stopReason"))
        .and_then(|s| s.as_str())
        .map_or(StopReason::EndOfTurn, pi_stop_reason);
    Some(AgentEvent::TurnComplete {
        stop_reason,
        usage: None,
    })
}

/// pi's `StopReason` union is `pending | stop | length | toolUse | error |
/// aborted`. `toolUse` and `pending` are mid-turn states that only reach here
/// on a run that ended while one was outstanding, so they fold into
/// `EndOfTurn` with everything unrecognised.
fn pi_stop_reason(s: &str) -> StopReason {
    match s {
        "aborted" => StopReason::Cancelled,
        // `length` is pi's spelling of the token ceiling; `maxTokens` is the
        // provider-side name the same condition travels under.
        "length" | "maxTokens" => StopReason::MaxTokens,
        "error" => StopReason::Error,
        _ => StopReason::EndOfTurn,
    }
}

/// The `[tool: ` prefix is load-bearing, not decoration: `stream_agent_turn`
/// keeps a delta out of the turn's answer text only when it starts with
/// `[tool ` or `[tool: `. Without it these land in the string the verdict,
/// task-plan, and PR-body parsers read.
fn pi_breadcrumb(v: &serde_json::Value) -> String {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("auto_retry_start") => {
            let attempt = v.get("attempt").and_then(|a| a.as_u64()).unwrap_or(0);
            let max = v.get("maxAttempts").and_then(|a| a.as_u64()).unwrap_or(0);
            let err = v
                .get("errorMessage")
                .and_then(|s| s.as_str())
                .unwrap_or("no reason given");
            format!("[tool: pi-retry {attempt}/{max}] {err}\n")
        }
        _ => {
            let reason = v
                .get("reason")
                .and_then(|s| s.as_str())
                .unwrap_or("threshold");
            format!("[tool: pi-compaction] {reason}\n")
        }
    }
}

/// Map an abstract [`PermissionProfile`] to the pi tools this step must not
/// have. Emitted as `-xt` (a denylist) rather than `-t` (an allowlist) so a
/// user's own extension tools survive a step that only means to drop shell or
/// write access.
///
/// `network` has no row: pi ships no webfetch or websearch tool, so
/// `network: Deny` is unenforceable beyond removing `bash`. Nothing here
/// pretends otherwise. The artifacts-vs-source path shape is enforced for
/// every agent by the chmod fence in `adapters/worktree/git_ops/scope.rs`.
pub(crate) fn excluded_tools_for(p: &PermissionProfile) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !p.read_fs.is_allow() {
        out.extend_from_slice(&["read", "grep", "find", "ls"]);
    }
    if !p.write_fs.is_allow() {
        out.extend_from_slice(&["edit", "write"]);
    }
    if !p.execute.is_allow() {
        out.push("bash");
    }
    out
}

/// Translate one [`AgentContext::tool_allowlist`] entry into the pi tool it
/// names, or `None` when pi has no analogue.
///
/// The field carries **claude-code's** tool vocabulary (`Read`, `Grep`,
/// `Glob`), as its port doc pins. pi's built-ins are the lowercase `read bash
/// edit write grep find ls` and there is no `glob` at all, so forwarding those
/// strings verbatim resolves the allowlist to zero tools and runs the turn
/// blind.
fn pi_tool_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "read" => Some("read"),
        "grep" => Some("grep"),
        "glob" | "find" => Some("find"),
        "ls" => Some("ls"),
        "edit" => Some("edit"),
        "write" => Some("write"),
        "bash" => Some("bash"),
        _ => None,
    }
}

/// Build the argv for one pi turn.
///
/// ```text
/// --mode json           the JSONL stream this adapter parses. Non-interactive
///                       on its own, so no `-p` alongside it.
/// -na                   ignore `~/.pi/agent/trust.json`. A fleet run must not
///                       depend on which projects the user happened to trust
///                       interactively, and Demeteo never writes that file.
/// --session <id>        continue the captured session so the static prefix
///                       (system prompt + tool defs) hits the provider prompt
///                       cache instead of starting cold.
/// --model <m>           accepts `provider/id`; that qualified form is what
///                       Demeteo stores and hands back.
/// --thinking <level>    pi's reasoning ladder.
/// --name <title>        session display name.
/// -t <a,b>              allowlist of tool *definitions*, mapped into pi's
///                       vocabulary by `pi_tool_name` — `-t ""` for
///                       `Some(vec![])` strips them all. Orthogonal to `-xt`
///                       below: the allowlist shrinks the prompt for
///                       single-purpose role turns, the denylist enforces the
///                       step's permission profile.
/// -xt <denied>          compiled from the profile by `excluded_tools_for`.
/// --no-extensions --no-skills --no-prompt-templates --no-themes
///                       bare mode only: a byte-identical static prefix across
///                       worktrees, for prompt-cache reuse.
/// <prompt>              trailing positional — stdin races pi's own init.
/// ```
///
/// Deliberately **not** passed: `-nc` (`--no-context-files`). AGENTS.md and
/// CLAUDE.md are identical across a feature's worktrees, so keeping them costs
/// no cache reuse and carries the project constitution to the agent.
///
/// `ctx.max_turns` and `ctx.max_budget_usd` have no pi equivalent and are
/// ignored rather than approximated.
fn build_pi_args(
    ctx: &AgentContext,
    captured_session_id: Option<&str>,
    prompt: &str,
) -> Vec<String> {
    let mut args = vec!["--mode".to_string(), "json".to_string(), "-na".to_string()];

    if let Some(sid) = captured_session_id {
        args.push("--session".to_string());
        args.push(sid.to_string());
    }
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    // Clamped even though pi's ladder is a superset of ours: the clamp, not the
    // CLI, is what makes an unsupported level unemittable.
    if let Some(effort) = ctx
        .effort
        .and_then(|e| EffortLevel::clamp_for(AgentKind::Pi, e))
    {
        args.push("--thinking".to_string());
        args.push(effort.as_str().to_string());
    }
    if let Some(ref title) = ctx.title {
        args.push("--name".to_string());
        args.push(title.clone());
    }
    if let Some(ref allowlist) = ctx.tool_allowlist {
        let mapped: Vec<&str> = allowlist
            .iter()
            .filter_map(|t| pi_tool_name(t.as_str()))
            .collect();
        // An ask naming only tools pi lacks drops the flag rather than sending
        // `-t ""`: the allowlist is a prompt-size optimisation and `-xt` still
        // holds the profile, so pi's full set is the honest degradation.
        // `Some(vec![])` keeps meaning no tools at all.
        if allowlist.is_empty() || !mapped.is_empty() {
            args.push("-t".to_string());
            args.push(mapped.join(","));
        }
    }
    let excluded = excluded_tools_for(&ctx.permissions);
    if !excluded.is_empty() {
        args.push("-xt".to_string());
        args.push(excluded.join(","));
    }
    if ctx.bare_mode {
        args.push("--no-extensions".to_string());
        args.push("--no-skills".to_string());
        args.push("--no-prompt-templates".to_string());
        args.push("--no-themes".to_string());
    }
    if !prompt.is_empty() {
        args.push(prompt.to_string());
    }
    args
}

/// Turn `pi --list-models` output into the identifiers handed back to
/// `--model`.
///
/// The command prints a padded six-column table under a
/// `provider model context max-out thinking images` header. The value is
/// `provider/model`, not the bare `model` column: `--model` takes a *pattern*,
/// and an id two authenticated providers both expose would resolve by pi's own
/// precedence rather than the row the user picked.
///
/// The column count doubles as the guard. With no provider authenticated pi
/// prints prose on this channel ("No models available. Use /login …" plus two
/// doc paths); none of those lines is six fields wide, so none survives.
fn parse_pi_model_table(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|cols| cols.len() == 6 && !(cols[0] == "provider" && cols[1] == "model"))
        .map(|cols| format!("{}/{}", cols[0], cols[1]))
        .collect()
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "pi",
        binary: "pi",
        install_cmd: PI_INSTALL,
        parse_event: parse_pi_event as EventParser,
        build_args: build_pi_args,
        // pi enforces through argv (`-xt`), not env — the claude-code shape.
        perm_env: crate::ports::agent_runtime::no_permission_env,
        // `--thinking` is argv, and pi reads no effort variable out of the
        // environment, so there is nothing to defend against here.
        effort_env: crate::adapters::agent::cli_runtime::no_effort_env,
        display_label: "Pi",
        model_listing: Some(ModelListing {
            args: "--list-models",
            parse: parse_pi_model_table,
        }),
        // pi follows the user's own `defaultProvider` / `defaultModel`, so
        // there is no statically-knowable model to seed the cost fallback.
        default_model: None,
        effort_levels: EffortLevel::supported_for(AgentKind::Pi),
        // `build_pi_args` answers `bare_mode` with `--no-skills
        // --no-extensions --no-prompt-templates --no-themes`, so a
        // capability-scoped step runs on nothing the user taught this harness.
        personalization: crate::ports::agent_runtime::PersonalizationSupport::Suppressed,
        // Headless hygiene, plus one behavioural pin: `long` retention buys the
        // extended provider prompt cache (1h Anthropic, 24h OpenAI), which is
        // what makes cross-step `--session` continuation actually pay off.
        static_env: &[
            ("PI_SKIP_VERSION_CHECK", "1"),
            ("PI_TELEMETRY", "0"),
            ("PI_CACHE_RETENTION", "long"),
        ],
        // pi resolves Git Bash and nothing else on Windows — the two Program
        // Files roots, then `bash.exe` on PATH — and raises rather than falling
        // back to a native shell when it finds none.
        windows_agent_shell: WindowsAgentShell::GitBash,
        windows_shell_env: &[],
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/agent/pi.rs"]
mod tests;
