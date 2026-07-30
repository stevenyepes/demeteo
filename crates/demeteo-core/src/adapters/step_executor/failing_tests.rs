//! Rung 3 of the granularity ladder: reading the *test identifiers* out of a
//! gate's output, and the comparison that spends an agent call only where they
//! can change something (`docs/HARNESS_BASELINE.md` HB2c, decision 44).
//!
//! # What this is allowed to do, and what it is not
//!
//! Rungs 1–2 compare an exit status and a normalized fingerprint, which the
//! engine owns outright. They answer "is this gate's failure the same one it had
//! at the base?" and nothing finer, so a gate that is red both sides but
//! *differently* can only be reported as "everything about this gate is
//! suspect" — and the rework cycle that follows re-derives the whole gate.
//!
//! Naming which failures are new is comprehension of output the engine already
//! captured, so it is a job an agent can do. Decision 44 forbids agent-produced
//! **evidence** — the thing being judged must not control whether it passed —
//! and this never touches an exit code, never runs a command, and cannot supply
//! one. Structurally:
//!
//! * the engine decides pass/fail from the exit status, always;
//! * an extraction that returns nothing, times out, fails to spawn, or answers
//!   unparseable text yields an empty list, which
//!   [`compare_gate`](crate::domain::harness_delta::compare_gate) reads as
//!   *unscoped* — byte-for-byte the behaviour before this module existed;
//! * the verdict reason still carries every failing gate's output tail whatever
//!   happens here, so a wrong reading narrows advice and can never remove
//!   evidence.
//!
//! # What it costs
//!
//! One cheap-model, tool-less, two-turn call per red gate, in exactly two
//! places:
//!
//! 1. **at baseline measurement**, for each gate that came back red — cached on
//!    [`HarnessBaselineRun::failing_tests`](crate::domain::harness_baseline::HarnessBaselineRun::failing_tests)
//!    and therefore paid once per measurement, never per attempt;
//! 2. **at validate**, only for a gate rungs 1–2 called `NewFailures` *and*
//!    whose record already carries names to diff against
//!    ([`GateComparison::extraction_would_scope`]). A green suite, a first-sight
//!    regression, a subtracted pre-existing failure and a run with no baseline
//!    at all each cost nothing.
//!
//! That second gate is why [`compare_gates_with_extraction`] compares twice: the
//! comparison is pure and free, and running it first is how the expensive rung
//! finds out whether it is worth paying for. Escalating only when the coarser
//! level is ambiguous is the ladder's whole rule.

use crate::adapters::step_executor::driver::verifier::{harness_block, HarnessRun};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::harness_baseline::HarnessBaseline;
use crate::domain::harness_delta::{compare_gate, GateComparison, ObservedFailure};
use crate::domain::harness_fingerprint::normalize_failure_fingerprint;
use crate::ports::agent_runtime::AgentContext;
use tokio_stream::StreamExt;

/// Reads the failing test identifiers a harness run named, out of its own
/// output.
///
/// A trait for the same reason [`BaselineTriage`](super::baseline::BaselineTriage)
/// is one: the extraction is the only thing the functions below need beyond
/// values the caller already holds, and taking an `ExecutionDriver` for it would
/// drag twenty-odd unread ports into every test (AGENTS.md §3 names constructing
/// one in a test as the shape to avoid). The production implementation is
/// [`DriverExtractor`].
#[async_trait::async_trait]
pub(crate) trait FailingTestExtractor: Send + Sync {
    /// The identifiers `output` names as failing, verbatim, in the order the
    /// runner printed them.
    ///
    /// **Must fail safe.** Every spawn, timeout, cancellation and parse failure
    /// owes an empty `Vec`, because empty is what leaves the comparison exactly
    /// as it was before this rung existed.
    async fn extract(&self, cmd: &str, output: &str) -> Vec<String>;
}

/// The production [`FailingTestExtractor`]: a cheap classifier turn through the
/// driver, asked in the worktree the gate ran in.
///
/// Borrows rather than owns for the same reason `DriverTriage` does — the
/// measurement it serves is a single `await`, and an extractor that outlived its
/// site could be asked about a worktree that no longer exists (the fallback
/// producer tears its own down as soon as the measurement returns).
pub(crate) struct DriverExtractor<'a> {
    pub driver: &'a ExecutionDriver,
    pub machine: &'a str,
    pub wt_path: &'a str,
}

#[async_trait::async_trait]
impl FailingTestExtractor for DriverExtractor<'_> {
    async fn extract(&self, cmd: &str, output: &str) -> Vec<String> {
        self.driver
            .extract_failing_tests(self.machine, self.wt_path, cmd, output)
            .await
    }
}

/// Compare every red gate against the baseline, escalating to rung 3 only where
/// the cheap rungs left something a reading could settle.
///
/// A free function over the one port it needs, so the whole escalation policy —
/// including *which gates get an agent call* — is assertable against a single
/// double rather than an `ExecutionDriver` (AGENTS.md §3).
///
/// The two passes are not a re-computation cost worth avoiding:
/// [`compare_gate`] is pure and synchronous, and the first pass is what makes
/// the second one cheap. Only the gates it flags reach the extractor.
pub(crate) async fn compare_gates_with_extraction(
    extractor: &dyn FailingTestExtractor,
    baseline: Option<&HarnessBaseline>,
    base_sha: &str,
    wt_path: &str,
    failed: &[HarnessRun],
) -> Vec<GateComparison> {
    // Fingerprinted over the same labelled block `measure_gates` records, so the
    // two sides of the comparison are strings built the same way rather than two
    // shapes that merely look similar.
    let fingerprints: Vec<String> = failed
        .iter()
        .map(|f| normalize_failure_fingerprint(&harness_block(&f.name, &f.cmd, &f.output), wt_path))
        .collect();
    fn observed<'a>(
        failed: &'a [HarnessRun],
        fingerprints: &'a [String],
        i: usize,
        tests: Option<&'a [String]>,
    ) -> ObservedFailure<'a> {
        ObservedFailure {
            name: &failed[i].name,
            command: &failed[i].cmd,
            fingerprint: &fingerprints[i],
            failing_tests: tests,
        }
    }

    let mut comparisons: Vec<GateComparison> = (0..failed.len())
        .map(|i| {
            compare_gate(
                baseline,
                base_sha,
                &observed(failed, &fingerprints, i, None),
            )
        })
        .collect();

    for i in 0..failed.len() {
        if !comparisons[i].extraction_would_scope() {
            continue;
        }
        let live = extractor.extract(&failed[i].cmd, &failed[i].output).await;
        if live.is_empty() {
            // Nothing readable. Leave the unscoped comparison exactly as it is
            // rather than re-running it to the same answer.
            continue;
        }
        comparisons[i] = compare_gate(
            baseline,
            base_sha,
            &observed(failed, &fingerprints, i, Some(&live)),
        );
    }

    comparisons
}

impl ExecutionDriver {
    /// Spawn a small agent to read the failing test identifiers out of one
    /// gate's output. Reuses the verifier's cheap-model plumbing, exactly as
    /// `triage_harness_failure` does — same registry, same pinned-low effort,
    /// same empty tool allowlist, same budget fraction, same cancellation race.
    ///
    /// Fails safe in one direction only: **every** way this can go wrong returns
    /// an empty `Vec`, and empty means "no scope", which is the behaviour with no
    /// extraction at all. There is no answer it can produce that changes whether
    /// a gate passed.
    pub(crate) async fn extract_failing_tests(
        &self,
        machine_str: &str,
        wt_path: &str,
        cmd: &str,
        output: &str,
    ) -> Vec<String> {
        let agent_kind = self
            .feature_agent_kind
            .clone()
            .or_else(|| self.default_agent_kind.clone())
            .unwrap_or_else(|| "claude-code".to_string());
        let model = self
            .feature_model
            .clone()
            .or_else(|| self.default_model.clone());

        let prompt = build_test_ids_prompt(cmd, &tail_chars(output, 6000));

        let agent_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;

        let thread_id = format!("{}-testids", self.f_id_str);
        let binary = self
            .registry
            .runtime_for(&agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.clone());
        let ctx = AgentContext {
            thread_id: thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env: agent_env,
            cwd: wt_path.to_string(),
            model,
            // Enumeration, not reasoning work — pinned low rather than
            // inheriting the run's effort (which may be `max`).
            effort: Some(crate::domain::models::EffortLevel::TRIAGE),
            title: Some("Read failing test ids".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
            // The entire input is inlined in the prompt and the entire output is
            // one JSON array — no tool definitions in context, no agentic loop.
            // It also means the extractor *cannot* run a command, which is what
            // keeps it on the reading side of decision 44's line.
            tool_allowlist: Some(vec![]),
            max_turns: Some(2),
            max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_TRIAGE),
        };

        let spawn_fut = self.registry.get_or_spawn(&thread_id, &agent_kind, ctx);
        let mut cancel_spawn = self.cancel_watch.clone();
        let session = match tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_spawn.changed() => None,
        } {
            Some(Ok(session)) => session,
            _ => return Vec::new(),
        };

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let idle_s = timeouts.normal_timeout_s;
        let wall_s = timeouts.wall_cap_s;
        let idle_sleep = tokio::time::sleep(std::time::Duration::from_secs(idle_s));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(wall_s));
        tokio::pin!(idle_sleep);
        tokio::pin!(wall_sleep);

        let mut text = String::new();
        let mut stream = session.prompt(&prompt);
        let mut cancel_watch = self.cancel_watch.clone();

        let ids = loop {
            tokio::select! {
                ev = stream.next() => {
                    idle_sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + std::time::Duration::from_secs(idle_s));
                    match ev {
                        Some(AgentEvent::Text { delta }) => text.push_str(&delta),
                        Some(AgentEvent::TurnComplete { .. }) | None => break parse_test_ids_text(&text),
                        Some(AgentEvent::Error { .. }) => break Vec::new(),
                        Some(_) => {}
                    }
                }
                _ = &mut idle_sleep => break Vec::new(),
                _ = &mut wall_sleep => break Vec::new(),
                _ = cancel_watch.changed() => {
                    if *cancel_watch.borrow() {
                        let _ = session.cancel();
                        break Vec::new();
                    }
                }
            }
        };

        let _ = self.registry.kill(&thread_id).await;
        ids
    }
}

/// The most identifiers one reading may name.
///
/// A suite that reports 400 failures has told us "the build is broken", not
/// "these 400 tests regressed", and threading that list into a rework prompt
/// would bury the reason it is there. The cap is generous enough that the
/// scoping case this rung exists for — a handful of new failures on top of a
/// pre-existing one — is never truncated.
const MAX_TEST_IDS: usize = 50;

/// Build the extraction prompt. It asks for exactly one JSON array so
/// [`parse_test_ids_text`] can lift it out of any surrounding prose, and it is
/// worded to *refuse* rather than guess: an empty array costs the caller
/// nothing (rungs 1–2 stand), while an invented identifier is the one output
/// that could mis-scope a retry.
fn build_test_ids_prompt(cmd: &str, output_tail: &str) -> String {
    format!(
        "You are reading the output of a test/build command that FAILED, and \
         extracting the identifiers of the individual checks that failed. You are \
         NOT judging anything: whether the command failed is already known from its \
         exit status, and nothing you say can change it.\n\n\
         The command was:\n{}\n\n\
         The tail of its combined stdout and stderr was:\n```\n{}\n```\n\n\
         List every failing test/check identifier **verbatim**, exactly as the \
         runner printed it — the string somebody would paste back to re-run that \
         one test (e.g. `tests::auth::login_rejects_expired_token`, \
         `src/app.test.ts > renders the header`, `test_parse_empty_input`).\n\n\
         Rules:\n\
         - Copy identifiers character for character. Do not tidy, shorten, \
           translate or invent them.\n\
         - If the output names no individual test — a compile error, a lint pass, a \
           crashed runner, a truncated log — return an empty array. An empty array \
           is a correct and useful answer; a guessed identifier is not.\n\
         - Do not include passing tests, file paths that are not test identifiers, \
           or summary lines like \"3 failed\".\n\
         - At most {} identifiers; if there are more, the failure is not \
           test-shaped, so return an empty array.\n\n\
         Respond with ONLY a JSON object and no other text:\n\
         {{ \"failing_tests\": [\"<identifier>\", ...] }}",
        cmd, output_tail, MAX_TEST_IDS,
    )
}

/// Lift the identifier array out of an extraction turn's text.
///
/// Every failure to find a usable array — no JSON, the wrong key, a non-array
/// value, a list of non-strings, more names than [`MAX_TEST_IDS`] — yields an
/// empty `Vec`, which the comparison reads as "no scope". Pure, so all of that
/// is assertable without an agent.
pub(crate) fn parse_test_ids_text(raw_text: &str) -> Vec<String> {
    let Some(val) = crate::domain::text::find_json_object_with_key(raw_text, "failing_tests")
    else {
        return Vec::new();
    };
    let Some(arr) = val.get("failing_tests").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let ids: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Over the cap the reading is not a scope, it is a second copy of the log.
    // The prompt asks for an empty array in that case; enforcing it here means a
    // model that ignores the instruction cannot flood a rework prompt either.
    if ids.len() > MAX_TEST_IDS {
        return Vec::new();
    }
    ids
}

/// Tail-truncate for the prompt, same reasoning as the triage classifier's: a
/// failing run's useful signal is at the bottom of its log.
fn tail_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    s.chars().skip(total - max).collect()
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/failing_tests_tests.rs"]
mod failing_tests_tests;
