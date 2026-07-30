// Tests extracted from `src/adapters/step_executor/baseline.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// [`measure_gates`] is a free function over one port precisely so it is
// reachable here without an `ExecutionDriver` and its twenty-odd unread ports
// (AGENTS.md §3). What the tests below protect is a single asymmetry: an
// *absent* baseline degrades to today's behaviour, while a *fabricated* one
// inverts HB2c's decision table and excuses a real regression. So every
// ambiguous input must record nothing rather than something plausible.
//
// The end-to-end wiring — which producer writes, at which sha, and that the
// worktree is torn down — is in `tests/conformance/harness_baseline.rs`, which
// runs a real shell.

use super::*;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// The shared strict double, keyed on the command a test *authored* rather than
/// the `( … ) 2>&1` shape the adapter is handed.
fn scripted(answers: &[(&str, Result<&str, &str>)]) -> ScriptedExec {
    ScriptedExec::new(answers).map_keys(|c| wrapped(c))
}

/// The `( … ) 2>&1` shape every baseline command is wrapped in.
fn wrapped(cmd: &str) -> String {
    crate::domain::harness_outcome::merge_stderr_into_stdout(cmd)
}

/// A [`BaselineTriage`] double that answers from a script and **records every
/// gate it was asked about**.
///
/// The call log is half the point: "a green baseline costs no agent call" and
/// "the classification is read from the record rather than re-asked at validate"
/// are both claims about a call that must *not* happen, and a double that only
/// returns values cannot witness them.
///
/// Like [`ScriptedExec`], it refuses anything it was not told to answer — but
/// its refusal has to be a `TriageVerdict`, so it returns `Regression` and
/// records the miss for the assertion to name. That is also the honest model of
/// production: `triage_harness_failure` cannot fail, it can only fall back.
struct ScriptedTriage {
    answers: HashMap<String, TriageVerdict>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedTriage {
    /// Keyed by harness *name*, since that is what identifies a gate.
    fn new(answers: &[(&str, TriageVerdict)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Classifies nothing — the shape every "no call should happen" assertion
    /// uses, and the one a malfunctioning classifier degrades to.
    fn none() -> Self {
        Self::new(&[])
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

fn environmental(reason: &str, remediation: &str) -> TriageVerdict {
    TriageVerdict::Environment {
        reason: reason.to_string(),
        remediation: remediation.to_string(),
    }
}

/// A [`FailingTestExtractor`] double, same shape and same reason as
/// [`ScriptedTriage`]: rung 3's cost claims — "a green gate is never asked",
/// "each red gate is asked exactly once" — are about calls that must or must not
/// happen, and only a double that records can witness them.
struct ScriptedExtractor {
    answers: HashMap<String, Vec<String>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedExtractor {
    /// Keyed by the gate's *command*, which is what the extractor is handed —
    /// deliberately not by name, since the reading is of one command's output and
    /// nothing about a gate's name reaches it.
    fn new(answers: &[(&str, &[&str])]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Reads nothing, ever — the malfunctioning extractor, and the shape every
    /// "no call should happen" assertion uses.
    fn none() -> Self {
        Self::new(&[])
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::adapters::step_executor::failing_tests::FailingTestExtractor for ScriptedExtractor {
    async fn extract(&self, cmd: &str, _output: &str) -> Vec<String> {
        self.asked.lock().unwrap().push(cmd.to_string());
        self.answers.get(cmd).cloned().unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl BaselineTriage for ScriptedTriage {
    async fn classify(&self, harness: &ResolvedHarness, _output: &str) -> TriageVerdict {
        self.asked.lock().unwrap().push(harness.name.clone());
        self.answers
            .get(&harness.name)
            .cloned()
            .unwrap_or(TriageVerdict::Regression)
    }
}

/// The worktree every test measures in. Its path is what the fingerprint is
/// normalized against, so it has to be the same one the assertions use.
fn site(producer: BaselineProducer) -> BaselineSite<'static> {
    BaselineSite {
        machine: "local",
        wt_path: "/repo_wt_baseline",
        step_id: "s-validate",
        base_sha: "abc123",
        producer,
    }
}

fn gate(name: &str, command: &str, deadline_s: u64) -> ResolvedHarness {
    ResolvedHarness {
        name: name.to_string(),
        command: command.to_string(),
        deadline_s,
    }
}

/// Run `measure_gates` against a scripted port with the defaults every test
/// shares, and a classifier that answers `regression` for everything — i.e. the
/// pre-HB2c-fix behaviour, which is what most of these tests hold fixed.
async fn measure(
    exec: &ScriptedExec,
    prepare: Option<&str>,
    harnesses: &[ResolvedHarness],
) -> Vec<MeasuredGate> {
    measure_with(exec, &ScriptedTriage::none(), prepare, harnesses).await
}

async fn measure_with(
    exec: &ScriptedExec,
    triage: &ScriptedTriage,
    prepare: Option<&str>,
    harnesses: &[ResolvedHarness],
) -> Vec<MeasuredGate> {
    measure_extracting(exec, triage, &ScriptedExtractor::none(), prepare, harnesses).await
}

async fn measure_extracting(
    exec: &ScriptedExec,
    triage: &ScriptedTriage,
    extractor: &ScriptedExtractor,
    prepare: Option<&str>,
    harnesses: &[ResolvedHarness],
) -> Vec<MeasuredGate> {
    measure_gates(
        &MeasurementPorts {
            exec,
            triage,
            extractor,
        },
        &site(BaselineProducer::Node),
        prepare,
        harnesses,
        ShellOptions::login_interactive(),
        1_700_000_000,
    )
    .await
}
mod classification;
mod gates;
mod omissions;
