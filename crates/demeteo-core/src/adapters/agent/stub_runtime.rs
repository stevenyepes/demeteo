//! Deterministic, no-LLM agent runtime for the topology-equivalence gate
//! (C5, `docs/EXECUTION_PARITY.md`).
//!
//! The three-transport conformance test (local / desktop-over-SSH /
//! headless runner) needs a workflow to run to a terminal state *without*
//! a real, authed coding-agent CLI — a container has no `claude`/`opencode`
//! binary and no credentials, and even locally we want a byte-deterministic
//! result to compare `RunView`s against. `StubRuntime` is that agent: it
//! reads directives embedded in the step prompt and produces exactly the
//! artifacts they name, then ends the turn.
//!
//! ## Why this is safe to compile into the release binary
//!
//! Unlike the `#[cfg(test)]` stubs in `super::test_stubs` (not linkable: rustdoc
//! builds without `--cfg test`, so the module is invisible to it), this runtime
//! must exist in the **shipped `demeteo-runner` binary** so the CI runner
//! container can execute a `RunSpec` with `agent_kind: "stub"`. It is
//! therefore a normal (non-test) module, but it is **only ever registered
//! when `DEMETEO_STUB_AGENT` is set** (see `composition::build_core_context`).
//! With the env var unset — every production path — the registry never
//! contains a `"stub"` runtime, so no workflow can select it.
//!
//! ## Prompt directive protocol
//!
//! The stub scans the rendered prompt for lines of the form:
//!
//! ```text
//! @stub-write <worktree-relative-path>
//! ```
//!
//! For each, it writes a deterministic body to `<cwd>/<path>` through the
//! [`ExecutionPort`](crate::ports::execution::ExecutionPort) (so the file
//! lands on whichever transport is in play — local disk or a remote
//! worktree over SFTP) and emits an
//! [`AgentEvent::ArtifactProduced`] whose
//! [`ArtifactSource::ToolWrite`] `path` equals the directive path verbatim.
//! That is exactly what the declared-artifact resolver
//! (`artifacts::declared::resolve_declared_artifacts`) matches a
//! `LastWriteTo { path }` declaration against, so a workflow that declares
//! `report.md` and whose prompt says `@stub-write report.md` gets a real,
//! materialized deliverable — identically on every transport.
//!
//! A directive path whose file name is `task-list.json` gets a valid
//! two-task `TaskPlan` JSON body instead of the markdown stub (see
//! [`stub_body`]), so a `sequence` step consuming the artifact via
//! `task_list_from` resolves a real, deterministic plan.
//!
//! A trailing `@stub-verdict <key>` directive makes the stub end its reply
//! with `{"<key>":"pass"}` so single-turn validate steps (which parse a
//! verdict object out of the agent text) pass deterministically.
//!
//! A `@stub-tests <a,b,c>` directive makes the stub end its reply with the
//! failing-test-identifier object rung 3's extractor asks for
//! (`{"failing_tests":["a","b","c"]}`), which
//! [`parse_test_ids_text`](crate::adapters::step_executor::failing_tests::parse_test_ids_text)
//! lifts out. Put it in a *gate's own output* and it reaches the extractor
//! through the harness block the prompt inlines, so the base's reading and the
//! tip's can differ without a second commit — which is how the "only the new
//! failures are reported" leg is driven deterministically. An empty list is
//! spelled by omitting the directive, i.e. by the extractor reading nothing.
//!
//! A `@stub-triage <category>` directive makes the stub end its reply with a
//! harness-triage verdict object (`{"category":"<category>", …}`) that
//! [`parse_triage_text`](crate::domain::harness_triage::parse_triage_text)
//! lifts out. This is what lets the C6 harness-failure triage classifier be
//! driven deterministically end-to-end: a failing harness command whose output
//! carries `@stub-triage environment` reaches the triage agent (which runs on
//! the feature's own `agent_kind`, i.e. the stub) via `build_triage_prompt`, so
//! the classifier returns `environment` without a real model. `<category>` is
//! echoed verbatim; anything other than `environment` parses back as
//! `regression` (the classifier's fail-safe default).

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::domain::agent_event::{AgentEvent, StopReason, Usage};
use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartFuture};

/// The `agent_kind` a `RunSpec`/feature selects to route to this runtime.
pub const STUB_AGENT_KIND: &str = "stub";

/// Env var that gates registration of the stub runtime in
/// `build_core_context`. Unset in production ⇒ the runtime is never
/// registered and `agent_kind: "stub"` resolves to
/// `AgentStartError::NotFound`.
pub const STUB_AGENT_ENV: &str = "DEMETEO_STUB_AGENT";

/// `true` when the stub runtime should be registered (env gate set to a
/// non-empty, non-`0` value).
pub fn stub_agent_enabled() -> bool {
    matches!(std::env::var(STUB_AGENT_ENV), Ok(v) if !v.is_empty() && v != "0")
}

/// Test-only record of what the driver actually resolved for each spawned
/// session: `(AgentContext::title, AgentContext::effort)`, in spawn order.
///
/// The stub is the only agent an in-crate e2e can drive end-to-end, and the
/// resolved effort is otherwise invisible from outside the runtime (it lands
/// on argv/env inside a real CLI). Compiled out of the shipped binary.
#[cfg(test)]
pub(crate) static SPAWN_LOG: std::sync::Mutex<
    Vec<(Option<String>, Option<crate::domain::models::EffortLevel>)>,
> = std::sync::Mutex::new(Vec::new());

/// Test-only record of every rendered prompt the stub was handed, in order.
///
/// A prompt is the one artefact of a step that is otherwise write-only: the
/// engine assembles it from a template, the harness evidence, the artifact
/// contract and the operating boundary, hands it to a runtime, and nothing
/// keeps a copy. So "did the excluded harness block reach the agent?" and "was
/// `{{harness_baseline}}` bound?" are unanswerable from outside without this —
/// and both are load-bearing claims (HB2c: a subtraction the reader cannot
/// audit will not be trusted).
///
/// Shared across concurrently-running tests, exactly like [`SPAWN_LOG`], so a
/// reader must filter by a marker unique to its own fixture rather than assume
/// it owns the log. Compiled out of the shipped binary.
#[cfg(test)]
pub(crate) static PROMPT_LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub struct StubRuntime;

#[async_trait]
impl AgentRuntime for StubRuntime {
    fn kind(&self) -> &'static str {
        STUB_AGENT_KIND
    }

    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: "Stub Agent",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: crate::ports::agent_runtime::PersonalizationSupport::Native,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }

    async fn availability(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> crate::domain::models::Availability {
        crate::domain::models::Availability::Installed
    }

    fn install_command(&self) -> &'static str {
        "true"
    }

    fn default_model(&self) -> Option<String> {
        // Intentionally `None`: the driver would otherwise treat this as an
        // explicit model override and, for a non-CLI agent, verify it was
        // applied by reading it back out of `session_info` — which this
        // deterministic stub does not model. No real model is involved, so
        // there is nothing to apply.
        None
    }

    fn start(&self, ctx: AgentContext) -> AgentStartFuture<'_> {
        #[cfg(test)]
        if let Ok(mut log) = SPAWN_LOG.lock() {
            log.push((ctx.title.clone(), ctx.effort));
        }
        Box::pin(async move { Ok(Arc::new(StubSession { ctx }) as Arc<dyn AgentSession>) })
    }
}

struct StubSession {
    ctx: AgentContext,
}

/// One parsed `@stub-write` directive.
struct WriteDirective {
    /// Worktree-relative path — matched verbatim against a declared
    /// `LastWriteTo { path }` and used as the `ToolWrite` source path.
    path: String,
}

struct StubDirectives {
    writes: Vec<WriteDirective>,
    verdict_key: Option<String>,
    /// Harness-triage category to echo back (`environment` / `regression`).
    /// Drives the C6 classifier deterministically.
    triage_category: Option<String>,
    /// Failing test identifiers to echo back, driving rung 3's extractor.
    failing_tests: Option<Vec<String>>,
}

/// Extract `@stub-write` / `@stub-verdict` / `@stub-triage` / `@stub-tests`
/// directives from a
/// rendered prompt. Whitespace-tolerant; a directive must be the first token
/// on its (trimmed) line, so a prompt merely *mentioning* one in prose — e.g.
/// the failing-command echo embedded mid-line in a triage prompt — does not
/// match. Ignores everything else.
fn parse_directives(text: &str) -> StubDirectives {
    let mut writes = Vec::new();
    let mut verdict_key = None;
    let mut triage_category = None;
    let mut failing_tests = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("@stub-write") {
            let path = rest.trim();
            if !path.is_empty() {
                writes.push(WriteDirective {
                    path: path.to_string(),
                });
            }
        } else if let Some(rest) = line.strip_prefix("@stub-verdict") {
            let key = rest.trim();
            if !key.is_empty() {
                verdict_key = Some(key.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("@stub-triage") {
            let category = rest.trim();
            if !category.is_empty() {
                triage_category = Some(category.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("@stub-tests") {
            let ids: Vec<String> = rest
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !ids.is_empty() {
                failing_tests = Some(ids);
            }
        }
    }
    StubDirectives {
        writes,
        verdict_key,
        triage_category,
        failing_tests,
    }
}

/// A deterministic failing-test reading, shaped exactly like the JSON rung 3's
/// extraction prompt asks for so `parse_test_ids_text` reads it back.
fn failing_tests_json(ids: &[String]) -> String {
    serde_json::json!({ "failing_tests": ids }).to_string()
}

/// A deterministic harness-triage verdict object for the given category,
/// shaped exactly like the JSON the C6 classifier prompt asks for so
/// `parse_triage_text` reads it back. Only `environment` carries a non-empty
/// remediation (regression's is empty by contract).
fn triage_json(category: &str) -> String {
    let remediation = if category == "environment" {
        "install the missing system dependency on the target machine"
    } else {
        ""
    };
    format!(
        "{{\"category\":\"{category}\",\"reason\":\"stub triage: classified as {category}\",\
         \"remediation\":\"{remediation}\"}}"
    )
}

/// Deterministic body written for a given directive path. Kept stable
/// across transports so the materialized artifact bytes are identical
/// whether produced locally, over SSH, or by the runner.
///
/// One path is content-aware: a directive whose file name is
/// `task-list.json` gets a valid two-task [`TaskPlan`] body instead of the
/// markdown stub, because a `sequence` step's `task_list_from` consumer
/// parses the artifact through `extract_task_plan` — the markdown body can
/// never satisfy it, and the starter-baseline harness (P0.2) needs the
/// bundled sequence-bearing starters to run past plan resolution.
fn stub_body(path: &str) -> String {
    if Path::new(path).file_name().and_then(|n| n.to_str()) == Some("task-list.json") {
        return concat!(
            "{\"tasks\":[",
            "{\"id\":\"stub-task-1\",\"title\":\"Stub task one\",",
            "\"description\":\"Deterministic stub task one.\",\"files\":[],",
            "\"acceptance\":[\"stub task one ran\"],\"blocked_by\":[]},",
            "{\"id\":\"stub-task-2\",\"title\":\"Stub task two\",",
            "\"description\":\"Deterministic stub task two.\",\"files\":[],",
            "\"acceptance\":[\"stub task two ran\"],\"blocked_by\":[\"stub-task-1\"]}",
            "]}\n"
        )
        .to_string();
    }
    format!("# stub artifact\n\npath: {path}\ngenerated-by: demeteo stub agent\n")
}

/// Coarse IANA type from the file extension — mirrors what a real agent's
/// `ToolWrite` event would carry so `RunView` renders identically.
fn mime_for(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "txt" | "" => "text/plain",
        "diff" | "patch" => "text/x-diff",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// The logical artifact name: the file stem (declared `ByName` matches on
/// the stem, `LastWriteTo` on the full path — carrying the stem satisfies
/// both).
fn artifact_name(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Join a worktree-relative directive path onto the session cwd. Absolute
/// directive paths are passed through unchanged (an escape hatch, though
/// workflows should use relative paths).
fn resolve_abs(cwd: &str, rel: &str) -> String {
    let p = Path::new(rel);
    if p.is_absolute() {
        return rel.to_string();
    }
    Path::new(cwd).join(rel).to_string_lossy().into_owned()
}

impl AgentSession for StubSession {
    fn session_id(&self) -> &str {
        "stub-session"
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        #[cfg(test)]
        if let Ok(mut log) = PROMPT_LOG.lock() {
            log.push(text.to_string());
        }
        let directives = parse_directives(text);
        let exec = self.ctx.exec.clone();
        let machine = self.ctx.machine_id.clone();
        let cwd = self.ctx.cwd.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        tokio::spawn(async move {
            for d in &directives.writes {
                let abs = resolve_abs(&cwd, &d.path);
                let body = stub_body(&d.path);
                // Land the file on the target worktree so git snapshot /
                // AllWrites captures also see it. A write failure is
                // surfaced as an Error event (not a silent skip) so the
                // step fails loudly (D3) rather than "green with no
                // artifact".
                if let Err(e) = exec.write_file(&machine, &abs, &body).await {
                    let _ = tx
                        .send(AgentEvent::Error {
                            code: "stub_write_failed".to_string(),
                            message: format!("stub failed to write {}: {}", d.path, e),
                            recoverable: false,
                            usage: None,
                        })
                        .await;
                    continue;
                }
                let _ = tx
                    .send(AgentEvent::ArtifactProduced {
                        artifact: Artifact {
                            name: artifact_name(&d.path),
                            mime: mime_for(&d.path),
                            content: body,
                            source: ArtifactSource::ToolWrite {
                                path: d.path.clone(),
                            },
                        },
                    })
                    .await;
            }

            let answered_otherwise =
                directives.verdict_key.is_some() || directives.triage_category.is_some();

            if let Some(key) = directives.verdict_key {
                let _ = tx
                    .send(AgentEvent::Text {
                        delta: format!("{{\"{key}\":\"pass\"}}"),
                    })
                    .await;
            }

            // Only when nothing else was asked for. A gate's output carries this
            // directive so the *extractor* sees it, and that same output is
            // inlined into the validate turn's evidence — where answering with a
            // test list instead of a verdict would make the fixture's own
            // observation channel change the step's outcome.
            if let Some(ids) = directives.failing_tests.filter(|_| !answered_otherwise) {
                let _ = tx
                    .send(AgentEvent::Text {
                        delta: failing_tests_json(&ids),
                    })
                    .await;
            }

            if let Some(category) = directives.triage_category {
                let _ = tx
                    .send(AgentEvent::Text {
                        delta: triage_json(&category),
                    })
                    .await;
            }

            let _ = tx
                .send(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                    usage: Some(Usage {
                        input_tokens: 1,
                        output_tokens: 1,
                        cost_usd: Some(0.0),
                        ..Default::default()
                    }),
                })
                .await;
        });

        Box::pin(ReceiverStream::new(rx))
    }

    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }

    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }
}
