//! JSON Schema for the `agent` node's `config` payload.
//!
//! Split out of `mod.rs` because it is a 90-line data literal that shares
//! nothing with the step's execution logic and changes on a different
//! cadence — a schema edit should not land in the same diff as a change to
//! how a verdict is read.
//!
//! Kept as `serde_json::json!` rather than an `include_str!`-ed `.json`
//! file on purpose: the macro is syntax-checked at **compile** time, and a
//! standalone file would trade that for a runtime parse whose only
//! failure mode — a malformed schema shipped in a release — is exactly
//! what the compile-time check already prevents.

/// JSON Schema for the `agent` node's `config` payload — the residual
/// [`StepConfig`] fields the v1→v2 migration leaves in `config` after
/// lifting id/kind/title/on_failure/task_list_from into first-class
/// structure (see `workflow_migrate.rs::LIFTED_FIELDS`).
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
pub(super) static AGENT_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for an `agent` node: one agent turn \
                against the feature worktree, producing declared artifacts \
                and optionally verified by a harness/verifier turn.",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Per-step agent runtime override (e.g. \
                        `claude-code`). Unset inherits the run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Per-step model override. Resolves below the \
                        run-time per-step override, above the project default."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Per-step reasoning-effort override. \
                        Unset inherits."
                },
                "prompt_template": {
                    "type": ["string", "null"],
                    "description": "The step's prompt template. Supports the \
                        `{{...}}` placeholders documented in PROMPT_CONTEXT."
                },
                "rework_prompt_template": {
                    "type": ["string", "null"],
                    "description": "Prompt rendered instead of \
                        `prompt_template` when a verdict from behind this \
                        step's task-list consumer sends the run back here \
                        — the previous cycle's code is already on the \
                        branch, so the step emits a delta rather than a \
                        whole decomposition. Unset falls back to \
                        `prompt_template`."
                },
                "max_iterations": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "description": "v1 legacy retry budget. In v2 the retry \
                        block owns budgets; migration lifts this when an \
                        on_failure existed, and keeps it here only as inert \
                        author intent."
                },
                "artifacts": {
                    "type": ["array", "null"],
                    "description": "Declared artifact captures \
                        (name/path/capture strategy) committed or stored after \
                        the turn.",
                    "items": { "type": "object" }
                },
                "verifier": {
                    "type": ["object", "null"],
                    "description": "Optional harness/verifier turn run after \
                        the agent turn; a FAIL verdict feeds the retry policy."
                },
                "capability": {
                    "type": ["string", "null"],
                    "enum": ["read_only", "artifacts", "verify", "implement", null],
                    "description": "Write-scope capability class (ReadOnly / \
                        Artifacts / Implement). Unset infers the safe default."
                },
                "allow_network": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt this step into web search / fetch."
                },
                "allow_shell": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt a non-shell capability into the shell."
                }
            },
            "additionalProperties": true
        })
    });
