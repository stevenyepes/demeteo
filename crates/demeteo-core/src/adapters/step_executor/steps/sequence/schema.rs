//! JSON Schema for the `sequence` node's `config` payload.
//!
//! Split out of `mod.rs` because it is a 70-line data literal that shares
//! nothing with the step's execution logic and changes on a different
//! cadence — a schema edit should not land in the same diff as a change to
//! how a checkpoint resumes.
//!
//! Kept as `serde_json::json!` rather than an `include_str!`-ed `.json`
//! file on purpose: the macro is syntax-checked at **compile** time, and a
//! standalone file would trade that for a runtime parse whose only
//! failure mode — a malformed schema shipped in a release — is exactly
//! what the compile-time check already prevents.

/// JSON Schema for the `sequence` node's `config` payload — the
/// residual [`StepConfig`] fields after migration lifts
/// `task_list_from` into a typed `task_list` edge.
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
pub(super) static SEQUENCE_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `sequence` node: run an \
                ordered task list, one fresh agent session per task, in a \
                single worktree, merging once at the end. The task list \
                arrives on a typed `task_list` edge (v1: `task_list_from`); \
                without one, the node plans its own decomposition.",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Per-step agent runtime override for the \
                        task agents. Unset inherits the run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Per-step model override for the task \
                        agents."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Per-step reasoning-effort override. \
                        Unset inherits."
                },
                "prompt_template": {
                    "type": ["string", "null"],
                    "description": "Prompt template each task agent renders, \
                        with the task's own goal injected."
                },
                "max_iterations": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "description": "v1 legacy retry budget; see the agent \
                        node's field of the same name."
                },
                "artifacts": {
                    "type": ["array", "null"],
                    "description": "Declared artifact captures committed or \
                        stored after the step.",
                    "items": { "type": "object" }
                },
                "verifier": {
                    "type": ["object", "null"],
                    "description": "Optional harness/verifier turn run after \
                        the list lands; a FAIL verdict feeds the retry \
                        policy targeted at the tasks owning the implicated \
                        files."
                },
                "capability": {
                    "type": ["string", "null"],
                    "enum": ["read_only", "artifacts", "verify", "implement", null],
                    "description": "Write-scope capability class. Sequence \
                        steps default to Implement (they legitimately write \
                        across the source tree)."
                },
                "allow_network": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt the task agents into web search / \
                        fetch."
                },
                "allow_shell": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt a non-shell capability into the \
                        shell."
                }
            },
            "additionalProperties": true
        })
    });
