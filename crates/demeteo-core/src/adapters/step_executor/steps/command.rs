//! The `command` node type — a deterministic shell command, run through
//! the existing [`ExecutionPort`], at zero token cost (task P3.5, PRD §5.2;
//! un-defers Decision 8 / `OPEN_QUESTIONS` §8).
//!
//! This is also the **extensibility proof** the PRD asks for (§9): the
//! whole node type is this file plus one registration line in
//! [`registry`](super::super::registry) — the scheduler, the dispatch
//! path, the run loop, and the entire frontend are untouched, and the
//! builder palette + config panel light up on their own because both
//! derive from [`NodeHandler`](super::super::registry::NodeHandler).
//!
//! # What it does
//!
//! 1. Provisions a fresh subtask worktree off the feature branch (the
//!    same isolation every agent step gets), so the command sees the
//!    feature's code and cannot disturb the shared checkout.
//! 2. Runs `command` there under an **interactive login shell** — the
//!    same shell the harness runs under
//!    ([`harness_shell_options`](crate::adapters::step_executor::harness_shell::harness_shell_options)),
//!    for the same reason: user-authored commands (`cargo test`, `npm run
//!    build`) resolve binaries off the user's `PATH`, which only a login
//!    shell establishes and only an interactive one activates `mise` /
//!    `asdf` / `nvm` shims for.
//! 3. Captures the command's output — stdout **and** stderr, merged, since
//!    the harnesses this node exists to run report on the latter — as the
//!    [`STDOUT_ARTIFACT`] artifact, and reads back every declared
//!    `last_write_to` artifact from the worktree.
//! 4. Tears the worktree down and reports.
//!
//! # What it deliberately does not do
//!
//! **It does not merge its worktree back into the feature branch.** A
//! command node is a *check*: harness, build, script, lint — the two
//! shapes PRD §7 migrates the starters to (`baseline-harness(command)`,
//! `baseline(command)`). Keeping it write-free is also what lets it run
//! concurrently with a research node under the §5.6 write-scope exclusion
//! lint, which is the whole payoff of Phase 4. A command that must change
//! tracked files is an `implement`-capability concern and needs the
//! merge-back + conflict-resolution path an agent step carries; that is
//! not in this task and the config schema says so.
//!
//! # Failure classification
//!
//! Mapped onto the P1.10 taxonomy so the declarative retry policy answers
//! a command failure the same way it answers an agent's:
//!
//! | Situation | Outcome | Class |
//! |---|---|---|
//! | Exit 0 | `Completed` | — |
//! | Non-zero exit | `VerdictFailed` | `verdict` — the harness spoke; redirect-to-implement with the output as feedback is exactly right |
//! | Missing declared artifact | `VerdictFailed` | `verdict` — the step ran but didn't produce its deliverable |
//! | Transport failure (`transport: …`) | `Environmental` | `environment` — the machine, not the code |
//! | Timeout | `Environmental` | `environment` — it hung |
//! | Malformed config | `NonRetryable` | `non_retryable` — no retry fixes a bad definition |

use std::collections::BTreeMap;
use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::registry::{
    CancelBehavior, NodeCtx, NodeDisplay, NodeHandler, NodePorts, ResumePolicy,
};
use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactSource};
use crate::domain::command_step::outcome::{
    classify_run, feedback_tail, CommandRun, FEEDBACK_TAIL_BYTES,
};
use crate::domain::command_step::spec::{parse_spec, validate_relative_cwd, CommandSpec};
use crate::domain::ids::FeatureId;
use crate::domain::models::workflow_v2::{NodeConfig, PortType};
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::verifier::VerdictFailure;
use crate::domain::workflow_graph::{LintFinding, WorkflowGraph};
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::StepExecutionPatch;
use crate::ports::execution::{ExecutionPort, ScriptRequest};
use crate::ports::notification::DomainEvent;

use super::StepOutcome;

/// Registry key.
pub(crate) const KIND: &str = "command";

/// Logical name of the artifact holding the command's merged stdout+stderr.
/// Always written (even on failure, even on a cancel that raced the exit,
/// and even when empty) so the node panel's Output tab has something to show
/// for every attempt — a command step whose output vanished on failure would
/// be the opposite of the visibility this phase is for.
const STDOUT_ARTIFACT: &str = "command-output";

// ── Execution ────────────────────────────────────────────────────────────────

impl ExecutionDriver {
    /// Handle a `kind == "command"` step. See the module docs for the
    /// lifecycle and the failure-class table.
    pub(crate) async fn handle_command_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        step_start: Instant,
    ) -> StepOutcome {
        self.emit_command_status(step_exec, "running", 0, None, None);

        let mut spec = match parse_spec(step_conf) {
            Ok(spec) => spec,
            Err(msg) => {
                return self.finish_command(
                    step_exec,
                    step_start,
                    &[],
                    StepOutcome::NonRetryable(msg),
                )
            }
        };

        // Resolve `{{project_setting}}` tokens against the same bindings the
        // agent prompts render from. This is what lets a *reusable* workflow
        // own a command node at all: a starter cannot know that this project
        // builds with `npm run build` and that one with `cargo build`, so
        // without resolution the only command node a starter can carry is the
        // baseline node, whose commands come from the project by construction.
        //
        // `render_executable`, not `render`: the prose renderer collapses an
        // unset token to `""`, and an empty command is not a command that did
        // nothing — it is a gate that reports success without running. Its
        // `Err` names the token, hence the setting, and lands on
        // `NonRetryable` because no agent turn can add a project setting; a
        // rework loop over one spends the whole budget and closes nothing
        // (S13, decision 43).
        if !spec.measure_baseline {
            let render = |script: &Option<String>| {
                script
                    .as_deref()
                    .map(|body| self.base_ctx.render_executable(body))
                    .transpose()
            };
            match (
                render(&spec.command.posix),
                render(&spec.command.powershell),
            ) {
                (Ok(posix), Ok(powershell)) => {
                    spec.command.posix = posix;
                    spec.command.powershell = powershell;
                }
                (Err(msg), _) | (_, Err(msg)) => {
                    return self.finish_command(
                        step_exec,
                        step_start,
                        &[],
                        StepOutcome::NonRetryable(format!(
                            "command node '{}': {msg}",
                            step_exec.step_id.0
                        )),
                    )
                }
            }
        }

        let machine_str = self.machine_id().to_string();
        let subtask_id = format!("{}-step-{}", self.f_id_str, step_exec.step_id.0);
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return self.finish_command(
                    step_exec,
                    step_start,
                    &[],
                    StepOutcome::Environmental(format!(
                        "command step worktree provision failed ({subtask_id}): {e}"
                    )),
                )
            }
        };

        // Two shapes share every line above and below this: the same
        // disposable worktree, the same login shell, the same teardown. Only
        // *what is run in it* differs — an authored command, or the project's
        // own gates measured into the baseline record (HB2b / P4.2a).
        let outcome = if spec.measure_baseline {
            self.run_baseline_node(step_exec, step_conf, &machine_str, &wt_path)
                .await
        } else {
            self.run_command_in_worktree(step_exec, step_conf, &spec, &machine_str, &wt_path)
                .await
        };

        // The worktree is disposable by design (no merge-back), so tear it
        // down on every path — including the failure paths, whose evidence
        // has already been read back into the artifact store above.
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await;

        let (outcome, artifact_refs) = outcome;
        self.finish_command(step_exec, step_start, &artifact_refs, outcome)
    }

    /// Run the command and collect its evidence. Returns the outcome plus
    /// the artifact references to persist — the caller owns worktree
    /// teardown, so this function never early-returns past it.
    async fn run_command_in_worktree(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        spec: &CommandSpec,
        machine_str: &str,
        wt_path: &str,
    ) -> (StepOutcome, Vec<String>) {
        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, Vec::new());
        }

        let cwd = match &spec.cwd {
            Some(dir) => format!("{}/{}", wt_path.trim_end_matches('/'), dir),
            None => wt_path.to_string(),
        };
        // The deadline is the adapter's to enforce, not ours to wrap: a
        // `tokio::time::timeout` here would only stop *waiting*, leaving the
        // command running inside a worktree we are about to delete. See
        // `ShellOptions::timeout`.
        let request = ScriptRequest {
            variants: spec.command.clone(),
            cwd: Some(cwd),
            env: resolve_env(&spec.env_allowlist),
            timeout: spec.timeout,
        };

        // Race the command against cancellation. Dropping the run future is
        // what stops the work — the local adapter kills the command's process
        // group on drop — so this is also the mechanism behind
        // `CancelBehavior::Immediate`.
        let mut cancel_watch = self.cancel_watch.clone();
        let cancelled = async move {
            // `wait_for` also resolves — as `Err` — when the sender is
            // dropped. That is "nobody can cancel this any more", not "this
            // was cancelled", so park forever and let the command decide the
            // outcome; treating it as a cancel would kill a healthy step
            // during feature teardown.
            if cancel_watch.wait_for(|c| *c).await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancelled => return (StepOutcome::Cancelled, Vec::new()),
            r = self.exec.run_script(machine_str, request) => r,
        };

        let evidence = self.command_evidence();
        let (output, exit_ok) = match classify_run(result) {
            CommandRun::Succeeded(out) => (out, true),
            CommandRun::Transport(err) => {
                return (
                    StepOutcome::Environmental(format!(
                        "command '{}' could not run: {err}",
                        step_exec.step_id.0
                    )),
                    Vec::new(),
                )
            }
            CommandRun::TimedOut(err) => {
                return (
                    StepOutcome::Environmental(format!(
                        "command '{}' timed out: {err}",
                        step_exec.step_id.0
                    )),
                    Vec::new(),
                )
            }
            CommandRun::Failed(err) => (err, false),
        };

        // Evidence first: stdout is persisted before any verdict, so a
        // failing command's output is on the node panel even though the
        // step is about to fail.
        let mut refs = Vec::new();
        if let Some(r) = store_command_artifact(
            &evidence,
            step_exec,
            Artifact {
                name: STDOUT_ARTIFACT.to_string(),
                mime: "text/plain".to_string(),
                content: output.clone(),
                source: ArtifactSource::AgentText,
            },
        ) {
            refs.push(r);
        }

        // Checked *after* the artifact is stored: a command that finished
        // just as the run was cancelled still produced evidence, and throwing
        // it away is the opposite of what the Output tab is for.
        if *self.cancel_watch.borrow() {
            return (StepOutcome::Cancelled, refs);
        }

        if !exit_ok {
            return (
                StepOutcome::VerdictFailed(VerdictFailure::from_reason(format!(
                    "Command '{}' failed.\nOutput:\n```\n{}\n```",
                    step_exec.step_id.0,
                    feedback_tail(&output, FEEDBACK_TAIL_BYTES)
                ))),
                refs,
            );
        }

        // Declared `last_write_to` deliverables are read back off the
        // worktree — this is what gives the node's `file` output port
        // meaning. A missing one is a verdict failure for the same reason
        // it is for an agent step: the step's whole point is its output.
        let (produced, missing) =
            collect_declared_files(&evidence, step_exec, step_conf, machine_str, wt_path).await;
        refs.extend(produced);
        if !missing.is_empty() {
            return (
                StepOutcome::VerdictFailed(VerdictFailure::from_reason(format!(
                    "Command '{}' exited 0 but produced no {}",
                    step_exec.step_id.0,
                    missing.join(", ")
                ))),
                refs,
            );
        }

        (StepOutcome::Completed, refs)
    }

    /// The two ports a command node's evidence passes through, borrowed
    /// together so the reading and the storing are one call each.
    fn command_evidence(&self) -> CommandEvidence<'_> {
        CommandEvidence {
            exec: &*self.exec,
            artifacts: &*self.artifacts,
            f_id: &self.f_id,
        }
    }

    /// Write the step's terminal row + progress event and hand the
    /// outcome straight back, so every exit path above is a one-liner.
    fn finish_command(
        &self,
        step_exec: &StepExecution,
        step_start: Instant,
        artifact_refs: &[String],
        outcome: StepOutcome,
    ) -> StepOutcome {
        let wall = step_start.elapsed().as_secs();
        let (status, error) = match &outcome {
            StepOutcome::Completed => ("completed", None),
            StepOutcome::Cancelled => ("cancelled", None),
            StepOutcome::VerdictFailed(vf) => ("failed", Some(vf.to_feedback())),
            StepOutcome::Failed(msg)
            | StepOutcome::Environmental(msg)
            | StepOutcome::NonRetryable(msg) => ("failed", Some(msg.clone())),
            StepOutcome::RedirectTo(_) => ("completed", None),
        };
        self.emit_command_status(step_exec, status, wall, error, Some(artifact_refs));
        outcome
    }

    /// One durable write + one live event, in that order (the P1.13
    /// invariant: the row is the truth, the event is its push).
    fn emit_command_status(
        &self,
        step_exec: &StepExecution,
        status: &str,
        wall: u64,
        error: Option<String>,
        artifact_refs: Option<&[String]>,
    ) {
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                status: Some(status.to_string()),
                // A command node spends no tokens — that is the point of
                // it (PRD §5.2) — so cost/tokens stay at zero rather than
                // being left untouched and inheriting a stale value.
                cost_usd: Some(Some(0.0)),
                tokens: Some(Some(0)),
                wall_clock_secs: Some(Some(wall)),
                artifact_paths: artifact_refs.map(|r| r.to_vec()),
                error_message: Some(error),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: status.into(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }
}

/// Resolve an env allowlist against the host process environment.
/// Unset names are skipped: a workflow that names an optional variable
/// should run on a machine that doesn't define it.
fn resolve_env(allowlist: &[String]) -> BTreeMap<String, String> {
    allowlist
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|v| (name.clone(), v)))
        .collect()
}

// ── Evidence ─────────────────────────────────────────────────────────────────

/// Where a command node's evidence is read from and written to. The two ports
/// travel together — every read is followed by a store — and neither is
/// addressable without the feature the artifact is filed under.
pub(crate) struct CommandEvidence<'a> {
    pub exec: &'a dyn ExecutionPort,
    pub artifacts: &'a dyn ArtifactStore,
    pub f_id: &'a FeatureId,
}

/// Read each declared `last_write_to` artifact out of the worktree.
/// Returns `(stored refs, missing descriptions)`. Capture shapes that
/// depend on agent tool-call events (`by_name`, `all_writes`) have no
/// meaning here — a shell command emits no events — and are skipped
/// rather than reported missing.
async fn collect_declared_files(
    evidence: &CommandEvidence<'_>,
    step_exec: &StepExecution,
    step_conf: &StepConfig,
    machine_str: &str,
    wt_path: &str,
) -> (Vec<String>, Vec<String>) {
    let mut refs = Vec::new();
    let mut missing = Vec::new();
    for decl in step_conf.artifacts.as_deref().unwrap_or(&[]) {
        let ArtifactCapture::LastWriteTo { path } = &decl.capture else {
            continue;
        };
        let abs = format!(
            "{}/{}",
            wt_path.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        match evidence.exec.read_file(machine_str, &abs).await {
            Ok(body) => {
                if let Some(r) = store_command_artifact(
                    evidence,
                    step_exec,
                    Artifact::tool_write(&decl.name, path.clone(), body),
                ) {
                    refs.push(r);
                }
            }
            Err(_) => missing.push(format!("declared artifact '{}' at {path}", decl.name)),
        }
    }
    (refs, missing)
}

/// Persist one artifact, degrading a store failure to a warning: a
/// missing evidence file must not turn a green command red.
fn store_command_artifact(
    evidence: &CommandEvidence<'_>,
    step_exec: &StepExecution,
    artifact: Artifact,
) -> Option<String> {
    match evidence
        .artifacts
        .put(evidence.f_id.as_str(), &step_exec.step_id.0, &artifact)
    {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(
                feature_id = %evidence.f_id,
                step_id = %step_exec.step_id.0,
                artifact = %artifact.name,
                error = %e,
                "failed to store command artifact"
            );
            None
        }
    }
}

// ── NodeHandler registration ─────────────────────────────────────────────────

/// The `command` node type. Registered with a single line in
/// [`NodeTypeRegistry::global`](super::super::registry::NodeTypeRegistry::global)
/// — no scheduler, driver, or frontend edit (PRD §9).
pub(crate) struct CommandNodeHandler;

static COMMAND_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `command` node: run a \
                deterministic shell command in a disposable worktree off \
                the feature branch, at zero token cost. Its stdout is \
                captured as an artifact and a non-zero exit fails the step \
                as a verdict. The worktree is NOT merged back — a command \
                that must change tracked files needs an agent step.",
            "properties": {
                "command": {
                    "type": "object",
                    "description": "Explicit POSIX and PowerShell script variants. \
                        The target selects its native variant; script text is never \
                        translated. Required unless `measure_baseline` is set.",
                    "properties": {
                        "posix": { "type": ["string", "null"] },
                        "powershell": { "type": ["string", "null"] }
                    },
                    "additionalProperties": false
                },
                "measure_baseline": {
                    "type": ["boolean", "null"],
                    "description": "Measure the harness baseline instead of \
                        running `command`: the node runs this PROJECT's \
                        prepare command plus every harness that gates \
                        validation, and records what each said at the commit \
                        it measured. Only valid at the head of the graph, \
                        where the feature branch still points at the base \
                        commit. A red gate is recorded, not judged — \
                        subtracting it from validate's verdict is a separate \
                        decision."
                },
                "cwd": {
                    "type": ["string", "null"],
                    "description": "Worktree-relative working directory. \
                        Unset runs at the worktree root; absolute paths and \
                        `..` segments are refused."
                },
                "env_allowlist": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Names of environment variables forwarded \
                        from Demeteo's process into the command. Nothing \
                        crosses unless named here; unset names are skipped."
                },
                "timeout_secs": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "description": "Wall-clock ceiling. Expiry is classified \
                        as an environment failure. Unset means no ceiling."
                },
                "idempotent": {
                    "type": ["boolean", "null"],
                    "description": "True when re-running this command after a \
                        crash is safe (a build, a test run). Leave false for \
                        anything with outside side effects (deploy, publish, \
                        migration): an interrupted attempt then always parks \
                        at the synthetic gate instead of re-running."
                },
                "capability": {
                    "type": ["string", "null"],
                    "enum": ["read_only", "artifacts", "verify", "implement", null],
                    "description": "Write-scope classification. Only used by \
                        the parallel-scheduling lint (PRD §5.6) — a command \
                        node enforces no fence of its own."
                },
                "artifacts": {
                    "type": ["array", "null"],
                    "items": { "type": "object" },
                    "description": "Declared deliverables. Only `last_write_to` \
                        captures apply: each path is read back off the worktree \
                        after the command exits, and a missing one fails the \
                        step."
                }
            },
            "required": ["command"],
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl NodeHandler for CommandNodeHandler {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &COMMAND_CONFIG_SCHEMA
    }

    fn display(&self) -> NodeDisplay {
        NodeDisplay {
            label: "Command",
            summary: "Run a deterministic shell command (harness, build, \
                      script) at zero token cost.",
        }
    }

    fn ports(&self) -> NodePorts {
        NodePorts {
            inputs: &[PortType::Any],
            // stdout is always produced; a declared `last_write_to`
            // deliverable makes it a file producer too.
            outputs: &[PortType::Text, PortType::File],
        }
    }

    /// The rules a bad command node breaks *before* it costs a run:
    /// a missing command is unrunnable, an escaping cwd is refused at
    /// dispatch, and an undeclared `idempotent` silently opts into the
    /// cautious always-gate-on-interrupt behavior — worth saying out loud,
    /// but not worth blocking a save over (PRD §6.3: errors block, warnings
    /// don't).
    fn lint(&self, node: &NodeConfig, _graph: &WorkflowGraph) -> Vec<LintFinding> {
        let mut findings = Vec::new();
        let cfg = node.config.as_object();
        let str_field = |key: &str| -> Option<&str> {
            cfg.and_then(|o| o.get(key))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };

        let bool_field =
            |key: &str| -> Option<bool> { cfg.and_then(|o| o.get(key)).and_then(|v| v.as_bool()) };
        let measures_baseline = bool_field("measure_baseline") == Some(true);
        let has_command = cfg
            .and_then(|o| o.get("command"))
            .and_then(|value| value.as_object())
            .is_some_and(|variants| {
                ["posix", "powershell"].into_iter().any(|key| {
                    variants
                        .get(key)
                        .and_then(|value| value.as_str())
                        .is_some_and(|body| !body.trim().is_empty())
                })
            });

        if !has_command && !measures_baseline {
            findings.push(LintFinding::node_error(
                "command-missing",
                &node.id,
                "Command node has no `command` to run.".to_string(),
            ));
        }
        if let Some(dir) = str_field("cwd") {
            if let Err(msg) = validate_relative_cwd(dir) {
                findings.push(LintFinding::node_error(
                    "command-cwd-escapes",
                    &node.id,
                    msg,
                ));
            }
        }
        // A baseline node is a measurement of a commit that has not changed:
        // re-running it after an interrupt is free of consequence by
        // construction, so the "are you sure this is safe to repeat" warning
        // would be noise the author cannot act on.
        if bool_field("idempotent") != Some(true) && !measures_baseline {
            findings.push(LintFinding::node_warning(
                "command-not-idempotent",
                &node.id,
                "Command is treated as non-idempotent: if a run is \
                 interrupted mid-command, it will wait for you to approve \
                 re-running it. Set `idempotent` when the command is safe \
                 to repeat."
                    .to_string(),
            ));
        }
        findings
    }

    /// A shell command holds no session to say goodbye to. Locally the kill
    /// is real: `run_command_in_worktree` races the run against the cancel
    /// watch and the local adapter kills the command's process group when the
    /// abandoned future drops. Over SSH only demeteo's wait ends — ssh2 gives
    /// us no way to signal the remote process — so the step reports cancelled
    /// while the remote command finishes on its own.
    fn cancel_grace(&self) -> CancelBehavior {
        CancelBehavior::Immediate
    }

    /// The idempotency rule of PRD §5.4: a non-idempotent command that was
    /// interrupted must never be re-run on a hunch. The workspace
    /// fingerprint the P1.14 guard compares describes the *worktree*, and a
    /// deploy or a publish leaves no trace there — so for these nodes the
    /// guard must ask regardless of what the fingerprint says.
    fn resume_policy(&self, step_conf: &StepConfig) -> ResumePolicy {
        match parse_spec(step_conf) {
            // A baseline node only reads: it measures a commit and writes a
            // record. Re-running it after an interrupt cannot do anything
            // twice, so it never needs a human to approve the retry.
            Ok(spec) if spec.idempotent || spec.measure_baseline => ResumePolicy::WhenUnchanged,
            // An unparseable config resolves to "ask": it is the safe
            // reading, and the step is about to fail as NonRetryable anyway.
            _ => ResumePolicy::AlwaysAsk,
        }
    }

    async fn execute(&self, ctx: NodeCtx<'_>) -> StepOutcome {
        ctx.driver
            .handle_command_step(ctx.step_exec, ctx.step_conf, ctx.step_start)
            .await
    }
}

#[cfg(test)]
#[path = "../../../../tests/adapters/step_executor/command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/steps/command_evidence.rs"]
mod command_evidence_tests;
