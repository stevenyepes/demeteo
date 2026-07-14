# Adding a coding agent to Demeteo

This guide walks through adding support for a new coding-agent CLI. You do not
need deep knowledge of the orchestrator — every agent plugs into the **same
capability contract**, and nothing downstream special-cases a particular agent.
If you fill in the contract, the step executor, model probe, settings UI, and
wizards pick your agent up automatically.

> **Why a contract instead of scraping?** Demeteo once shipped an `antigravity`
> adapter that scraped an undocumented headless surface. Google killed Gemini
> CLI and replaced it (with Antigravity) inside one release cycle; the scraped
> parser broke silently and the adapter was eventually removed. The lesson,
> baked into this contract: we integrate agents through a **stable, structured
> CLI surface** (`--format json` / `--output-format stream-json`) and a
> declared capability descriptor — never by guessing at output shape. If a CLI
> has no structured/stream output, it is not yet a good fit.

## The big picture

An agent is one implementation of the `AgentRuntime` port
(`crates/demeteo-core/src/ports/agent_runtime.rs`). Every current agent is a
**one-shot CLI process**: Demeteo spawns `<binary> <args>` per prompt, reads
newline-delimited JSON from stdout, maps each line to an internal `AgentEvent`,
and tears the process down when the turn completes. That shared machinery lives
in `UnifiedCliRuntime` (`crates/demeteo-core/src/adapters/agent/cli_runtime.rs`),
so a new CLI agent is **four function pointers + one descriptor**, not a new
runtime:

| Piece | Type | What it does |
|-------|------|--------------|
| `parse_event` | `fn(&str) -> Option<AgentEvent>` | Map one JSON line → internal event |
| `build_args` | `fn(&AgentContext, Option<&str>, &str) -> Vec<String>` | Build the argv for one turn |
| `perm_env` | `fn(&PermissionProfile) -> HashMap<String,String>` | Translate the permission policy to native env |
| `effort_env` | `fn(Option<EffortLevel>) -> HashMap<String,String>` | Translate the resolved effort to native env |
| `AgentCapabilities` | struct | Declared `display_label`, `lists_models`, `default_model`, `effort_levels` |

Look at `adapters/agent/claude_code/mod.rs` (a mature CLI with structured
stream output) as the closest template, or `opencode`/`hermes` for the
`OPENCODE_PERMISSION` family.

## Step 1 — Register the kind

`AgentKind` (`crates/demeteo-core/src/domain/models/agent_config.rs`) is the
canonical enum. Add your variant, its kebab string, and include it in `ALL`:

```rust
pub enum AgentKind { Opencode, Hermes, ClaudeCode, /* + */ Yourname }

// as_str():   AgentKind::Yourname => "yourname",
// parse():    "yourname" => Some(AgentKind::Yourname),
// ALL:        add AgentKind::Yourname
```

The kebab string is the single identifier used on the wire, in the DB, and as
the runtime `kind()` key — `kind()` must equal `AgentKind::as_str()`. Validation
everywhere (`create_project.rs`, DB config parsing) routes through
`AgentKind::parse` / `is_supported`, so registering the variant is all that's
needed to make your kind "supported."

## Step 2 — Write the adapter

Create `crates/demeteo-core/src/adapters/agent/yourname/mod.rs`.

### `parse_event`

Parse exactly one stdout line into an `Option<AgentEvent>` (return `None` to
drop a line). The event vocabulary is in `domain/agent_event.rs`:

- `Text { delta }` — assistant prose / tool chatter.
- `ToolCall { tool_call_id, intercept_id, action, target, preview }` — a tool
  invocation, mapped to an `ActionKind` so the policy layer can gate it.
- `ToolCallUpdate { tool_call_id, status, preview }` — tool result.
- `Usage(Usage)` — token/cost snapshot (input/output/cache_read/cache_creation/cost_usd).
- `TurnComplete { stop_reason, usage }` — terminal event; closes the stream.
- `Error { code, message, recoverable }` — terminal error.

If your CLI emits a session id on its first event, `UnifiedCliRuntime`'s read
loop captures it from the raw JSON (see how claude-code's `system`/`init` line
is handled) and threads it back into `build_args` for cross-step continuity.

### `build_args`

Return the argv for one turn. The third parameter is the prompt — place it in
the slot your CLI expects (all current agents take it as a **trailing
positional**; passing it via stdin races the CLI's own init phase, so avoid
that). Model selection is a `--model` flag built from `AgentContext.model`:

```rust
if let Some(ref m) = ctx.model { args.push("--model".into()); args.push(m.clone()); }
```

There is no config-file/env model path — every supported agent is a CLI runtime
that takes `--model`. If a captured session id is present, add your CLI's
resume flag (opencode `--session <id>`, claude-code/hermes `--resume <id>`).

Effort is a **peer of the model**, not a property of it, and rides the same way.
`ctx.effort` is already resolved (the 5-tier chain, terminating at
`EffortLevel::DEFAULT` = `high`), but **clamp it to your own kind before you
emit it** — the clamp is what makes an unsupported level unemittable even if a
buggy caller asks for one:

```rust
if let Some(level) = ctx.effort.and_then(|e| EffortLevel::clamp_for(AgentKind::YourName, e)) {
    args.push("--effort".into());
    args.push(level.as_str().to_string());   // "low" | "medium" | "high" | "xhigh" | "max"
}
```

Do **not** trust the CLI to reject a level it doesn't understand: codex wraps an
unknown effort as `Custom(String)` and puts it on the wire, and opencode treats
an unsupported `--variant` as a silent no-op. Demeteo owns the validation.

### `effort_env`

If your CLI reads effort from the environment as well as (or instead of) argv,
return it here; otherwise return the shared `no_effort_env` helper, exactly as
`no_permission_env` works for permissions. Only claude-code needs a non-empty
map today — `CLAUDE_CODE_EFFORT_LEVEL` **outranks** its own `--effort` flag, and
the child inherits the host environment (`sanitize_child_env` strips only
`LD_LIBRARY_PATH` / `LD_PRELOAD`), so a developer with that variable exported
would silently override every Demeteo run unless the adapter sets it explicitly
on each spawn. If your CLI has the same shape, set the variable — belt *and*
braces is the correct posture here.

### `perm_env`

Translate the abstract `PermissionProfile` (`read_fs | write_fs | execute |
network`, each Allow/Deny) into your CLI's native enforcement. Two precedents:

- **env-based** (opencode, hermes): return `OPENCODE_PERMISSION` JSON via the
  shared `opencode_permission_env` helper.
- **flag-based** (claude-code): return `no_permission_env` and enforce inside
  `build_args` (e.g. `--disallowedTools`).

The policy is always *complete* (`allow`/`deny`, never `ask`) so headless runs
never block on a prompt. The OS-level chmod fence in
`adapters/worktree/git_ops/scope.rs` enforces the artifacts-vs-source path shape
uniformly regardless of what your CLI supports.

### `runtime()` + capabilities

```rust
pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "yourname",
        binary: "yourname",                 // the executable on $PATH
        install_cmd: "npm install -g @vendor/yourname-cli",
        parse_event: parse_yourname_event,
        build_args: build_yourname_args,
        perm_env: crate::ports::agent_runtime::opencode_permission_env, // or no_permission_env
        effort_env: crate::adapters::agent::cli_runtime::no_effort_env,  // or your own
        display_label: "Your Name",         // shown in every picker
        lists_models: true,                 // does `<binary> models` list models?
        default_model: None,                // Some("...") to seed cost fallback
        effort_levels: EffortLevel::supported_for(AgentKind::YourName), // may be &[]
    }
}
```

`AgentCapabilities` is the whole reason no downstream site special-cases your
kind: `display_label` feeds the UI, `lists_models` drives dynamic model
discovery in `application/agent_probe.rs`, `default_model` seeds the
`UsageAccumulator` cost fallback, and `effort_levels` populates every effort
picker in the frontend.

Declare your effort mapping in one place — the per-agent table in
`domain/models/effort.rs` (`supported_for` / `clamp_for`), which the exhaustive
unit test there holds you to. Declare **only** the levels the CLI really takes:
an agent with no per-invocation effort control declares `&[]` (hermes does), and
the frontend then disables its effort control with a tooltip instead of
pretending a level applied. Honest degradation beats a silently ignored flag —
the user can see the difference; a dropped `--variant` they cannot.

## Step 3 — Register in composition

Add `pub mod yourname;` to `adapters/agent/mod.rs`, then push one line into the
runtimes vec in `crates/demeteo-core/src/composition/mod.rs`:

```rust
Arc::new(adapters::agent::yourname::runtime()) as Arc<dyn AgentRuntime>,
```

That is the only wiring. There is no enum-of-adapters to update elsewhere.

## Step 4 — The frontend is automatic

The UI has **no hardcoded agent list**. The `list_agents` Tauri command
(`src-tauri/src/commands/agent_config.rs`) enumerates the registry and returns
each agent's kind + capabilities; the wizards, settings, and machine views all
derive from it via `src/lib/agentCatalog.ts`. Once your `runtime()` is in the
composition vec and `AgentKind` knows the variant, your agent appears in every
picker with its `display_label`. Add a price row for its models in the pricing
table (`adapters/pricing.rs`) if you want accurate cost.

## Step 5 — Golden-transcript fixture (conformance)

Record one representative run's raw stdout and the expected `AgentEvent`
sequence as a fixture (format defined by the conformance harness, Epic A3.1)
and add a replay test that feeds the bytes through your `parse_event`. This is
what turns an upstream CLI wire-format change into a red CI check instead of a
silent parse failure weeks later — the whole reason the contract exists.

## Checklist

- [ ] `AgentKind` variant + `as_str` / `parse` / `ALL`
- [ ] `adapters/agent/yourname/mod.rs` with `parse_event` / `build_args` / `perm_env` / `effort_env`
- [ ] `runtime()` returning a `UnifiedCliRuntime` incl. `AgentCapabilities`
- [ ] Effort mapping in `domain/models/effort.rs` (`supported_for`) — `&[]` if the CLI has no per-invocation control
- [ ] `build_args` clamps `ctx.effort` via `EffortLevel::clamp_for` before emitting it
- [ ] `pub mod yourname;` + one line in `composition/mod.rs`
- [ ] Pricing rows in `adapters/pricing.rs` (optional but recommended)
- [ ] Golden-transcript fixture + replay test
- [ ] README "Supported agents" table row
- [ ] `cargo clippy --all-targets -D warnings` + `cargo test` + `npx tsc --noEmit` clean

See [`AGENT_INTEGRATION.md`](../../AGENT_INTEGRATION.md) for the full runtime
spec and [`docs/DDD_MODEL.md`](../DDD_MODEL.md) §6 for the domain model.
