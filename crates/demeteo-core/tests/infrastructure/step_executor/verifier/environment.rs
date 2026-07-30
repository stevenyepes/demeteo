// Tests for `src/adapters/step_executor/driver/verifier/environment.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// HB9's decision is a *pair*: the notification row is what survives a refresh,
// the domain event is what raises the toast. A test that checked only the event
// would pass while the bell stayed empty, which is the shape of the bug the
// persistence path was added to fix — so both are asserted, every time.
//
// Three doubles, all of which refuse anything they were not told to answer.

use super::*;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::models::{Feature, Notification, NotificationKind};
use std::sync::Mutex;

const F_ID: &str = "f-env";
const P_ID: &str = "p-env";

// ── doubles ─────────────────────────────────────────────────────────────────

/// A `FeatureRepository` that knows about **one** feature and refuses every
/// other question. `get` is the only method this stage may call; anything else
/// panics rather than answering plausibly.
struct FeaturesDouble {
    known: Option<Feature>,
}

impl FeaturesDouble {
    fn with_feature() -> Self {
        Self {
            known: Some(feature()),
        }
    }

    fn empty() -> Self {
        Self { known: None }
    }
}

fn feature() -> Feature {
    Feature {
        id: FeatureId::from(F_ID),
        project_id: ProjectId::from(P_ID),
        workflow_id: Some(WorkflowId::from("w-1")),
        workflow_version_id: None,
        title: "env feature".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 1_700_000_000,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: None,
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
    }
}

fn step_exec() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-env"),
        feature_id: FeatureId::from(F_ID),
        step_id: StepId::from("s-validate"),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "running".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
    }
}

impl FeatureRepository for FeaturesDouble {
    fn get(&self, id: &FeatureId) -> Result<Option<Feature>, String> {
        Ok(self.known.clone().filter(|f| &f.id == id))
    }
    fn get_active(&self, _p: &ProjectId) -> Result<Vec<Feature>, String> {
        panic!("unscripted get_active")
    }
    fn add(&self, _f: Feature) -> Result<(), String> {
        panic!("unscripted add")
    }
    fn update(
        &self,
        _id: &FeatureId,
        _patch: &crate::ports::db::FeaturePatch,
    ) -> Result<(), String> {
        panic!("unscripted update")
    }
    fn update_workflow_id(&self, _id: &FeatureId, _w: &WorkflowId) -> Result<(), String> {
        panic!("unscripted update_workflow_id")
    }
    fn merge_harness_baseline(
        &self,
        _id: &FeatureId,
        _b: &crate::domain::harness_baseline::HarnessBaseline,
    ) -> Result<(), String> {
        panic!("unscripted merge_harness_baseline")
    }
    fn pin_workflow_version(
        &self,
        _id: &FeatureId,
        _v: &crate::domain::ids::WorkflowVersionId,
    ) -> Result<(), String> {
        panic!("unscripted pin_workflow_version")
    }
    fn list_with_open_mr(&self) -> Result<Vec<Feature>, String> {
        panic!("unscripted list_with_open_mr")
    }
    fn step_create(&self, _s: StepExecution) -> Result<(), String> {
        panic!("unscripted step_create")
    }
    fn step_get(&self, _id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        panic!("unscripted step_get")
    }
    fn step_update(
        &self,
        _id: &StepExecutionId,
        _p: &crate::ports::db::StepExecutionPatch,
    ) -> Result<(), String> {
        panic!("unscripted step_update")
    }
    fn steps_for_feature(&self, _id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        panic!("unscripted steps_for_feature")
    }
    fn attempt_open(
        &self,
        _id: &StepExecutionId,
        _now: i64,
        _fp: Option<&str>,
    ) -> Result<u32, String> {
        panic!("unscripted attempt_open")
    }
    #[allow(clippy::too_many_arguments)]
    fn attempt_close(
        &self,
        _id: &StepExecutionId,
        _no: u32,
        _status: &str,
        _cost: f64,
        _tokens: i64,
        _wall_ms: u64,
        _class: Option<&str>,
        _fp: Option<&str>,
        _rule: Option<&str>,
        _now: i64,
    ) -> Result<(), String> {
        panic!("unscripted attempt_close")
    }
    fn attempts_for_step(
        &self,
        _id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::StepAttempt>, String> {
        panic!("unscripted attempts_for_step")
    }
    fn subtask_runs_for_step(
        &self,
        _id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::SubtaskRunRow>, String> {
        panic!("unscripted subtask_runs_for_step")
    }
}

/// Records every row written; refuses every read.
#[derive(Default)]
struct NotificationsDouble {
    rows: Mutex<Vec<Notification>>,
}

impl NotificationRepository for NotificationsDouble {
    fn add(&self, n: Notification) -> Result<(), String> {
        self.rows.lock().expect("not poisoned").push(n);
        Ok(())
    }
    fn list(&self, _p: Option<&ProjectId>, _limit: u32) -> Result<Vec<Notification>, String> {
        panic!("unscripted list")
    }
    fn mark_read(&self, _id: &str) -> Result<u32, String> {
        panic!("unscripted mark_read")
    }
    fn unread_count(&self) -> Result<u32, String> {
        panic!("unscripted unread_count")
    }
}

#[derive(Default)]
struct NotifDouble {
    events: Mutex<Vec<String>>,
}

impl NotificationPort for NotifDouble {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        if let DomainEvent::EnvironmentNotReady {
            feature_id,
            step_id,
            reason,
        } = event
        {
            self.events
                .lock()
                .expect("not poisoned")
                .push(format!("{}|{}|{}", feature_id.0, step_id, reason));
        } else {
            panic!("this stage emits exactly one kind of event");
        }
        Ok(())
    }
}

struct Harness {
    features: FeaturesDouble,
    notifications: NotificationsDouble,
    notif: NotifDouble,
    feature_id: FeatureId,
}

impl Harness {
    fn new(features: FeaturesDouble) -> Self {
        Self {
            features,
            notifications: NotificationsDouble::default(),
            notif: NotifDouble::default(),
            feature_id: FeatureId::from(F_ID),
        }
    }

    fn signal(&self) -> EnvironmentSignal<'_> {
        EnvironmentSignal {
            features: &self.features,
            notifications: &self.notifications,
            notif: &self.notif,
            feature_id: &self.feature_id,
        }
    }

    fn rows(&self) -> Vec<Notification> {
        self.notifications
            .rows
            .lock()
            .expect("not poisoned")
            .clone()
    }

    fn events(&self) -> Vec<String> {
        self.notif.events.lock().expect("not poisoned").clone()
    }
}

fn red(cmd: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: "default".to_string(),
        cmd: cmd.to_string(),
        output: output.to_string(),
    }
}

// ── HB9: the row and the event are one decision ─────────────────────────────

/// A gate that cannot run is the same news whether the engine noticed it at the
/// head of the graph or at validate, and it must arrive through a channel that
/// survives a refresh. The row is that channel; the event is the toast. Both,
/// or the bell is empty the moment the user reloads.
#[test]
fn the_signal_persists_a_row_and_emits_an_event() {
    let h = Harness::new(FeaturesDouble::with_feature());

    notify_environment_not_ready(&h.signal(), &step_exec(), "gdk-3.0 is missing");

    let rows = h.rows();
    assert_eq!(rows.len(), 1, "the bell has to survive a refresh");
    assert!(matches!(
        rows[0].kind,
        NotificationKind::EnvironmentNotReady
    ));
    assert_eq!(rows[0].message, "gdk-3.0 is missing");
    assert_eq!(rows[0].feature_id, F_ID);
    assert_eq!(rows[0].project_id, P_ID);
    assert_eq!(
        rows[0].feature_url.as_deref(),
        Some("/projects/p-env/features/f-env"),
        "a notification the user cannot click through is half a notification"
    );
    assert!(!rows[0].read);

    assert_eq!(
        h.events(),
        vec![format!("{F_ID}|s-validate|gdk-3.0 is missing")],
        "and the live toast carries the same reason"
    );
}

/// The row needs a project id, which only the feature row has. When the lookup
/// comes back empty the event still fires: a signal the user can act on beats
/// silence, and the alternative would be losing the terminal reason entirely.
#[test]
fn an_unreadable_feature_still_emits_the_event() {
    let h = Harness::new(FeaturesDouble::empty());

    notify_environment_not_ready(&h.signal(), &step_exec(), "gdk-3.0 is missing");

    assert!(h.rows().is_empty());
    assert_eq!(h.events().len(), 1);
}

// ── the never-ran fast paths, and their order ───────────────────────────────

/// Exit 127 means the shell never found the binary, so the code never ran and a
/// `Verdict` would send an agent to fix something that was never tested.
#[test]
fn a_missing_binary_terminates_with_a_path_remediation_and_raises_the_signal() {
    let h = Harness::new(FeaturesDouble::with_feature());

    let err = command_never_ran_error(
        &h.signal(),
        &step_exec(),
        "gpu-box",
        "/home/u/wt/feat",
        &[red("cargo test", "bash: line 1: cargo: command not found")],
    )
    .expect("a 127 is terminal");

    let msg = match err {
        crate::domain::verifier::VerifierError::Environment(m) => m,
        other => panic!("a never-ran command is never a verdict; got {other:?}"),
    };
    assert!(msg.contains("exit 127"), "got:\n{msg}");
    assert!(
        msg.contains("bash -l -i -c 'command -v cargo'"),
        "got:\n{msg}"
    );
    assert!(msg.contains("ssh gpu-box"), "got:\n{msg}");

    assert_eq!(h.rows().len(), 1, "and it reaches the bell");
    assert_eq!(h.events().len(), 1);
}

/// The 127 check goes **first** because it is the stronger claim: if the binary
/// itself is absent, "your project's script list is wrong" misdiagnoses a
/// machine that cannot run the tool at all. This output is both shapes at once.
#[test]
fn a_missing_binary_is_diagnosed_ahead_of_a_missing_script() {
    let h = Harness::new(FeaturesDouble::with_feature());

    let err = command_never_ran_error(
        &h.signal(),
        &step_exec(),
        "local",
        "/wt",
        &[
            red(
                "npm run checks",
                "npm error Missing script: \"checks\"\nnpm error\n",
            ),
            red("cargo test", "bash: line 1: cargo: command not found"),
        ],
    )
    .expect("terminal");

    let msg = match err {
        crate::domain::verifier::VerifierError::Environment(m) => m,
        other => panic!("got {other:?}"),
    };
    assert!(
        msg.contains("exit 127"),
        "the missing binary must win; got:\n{msg}"
    );
    assert_eq!(
        h.rows().len(),
        1,
        "one terminal answer, one notification — the reproduce line names one command"
    );
}

/// A runner that *did* run but was asked for a script this worktree does not
/// define is the same category with different remediation: sending the user
/// after a package that was never missing is the misdiagnosis to avoid.
#[test]
fn a_missing_script_terminates_without_the_path_remediation() {
    let h = Harness::new(FeaturesDouble::with_feature());

    let err = command_never_ran_error(
        &h.signal(),
        &step_exec(),
        "local",
        "/wt",
        &[red(
            "npm run checks",
            "npm error Missing script: \"checks\"\nnpm error\n",
        )],
    )
    .expect("terminal");

    let msg = match err {
        crate::domain::verifier::VerifierError::Environment(m) => m,
        other => panic!("got {other:?}"),
    };
    assert!(
        !msg.contains("exit 127"),
        "the binary was found; got:\n{msg}"
    );
    assert!(msg.contains("checks"), "got:\n{msg}");
    assert_eq!(h.rows().len(), 1);
}

/// An ordinary red gate is a verdict, not an environment fault — and nothing
/// must reach the bell for it, or every failing test suite would raise a
/// terminal notification.
#[test]
fn an_ordinary_failure_is_not_a_never_ran_and_signals_nothing() {
    let h = Harness::new(FeaturesDouble::with_feature());

    let err = command_never_ran_error(
        &h.signal(),
        &step_exec(),
        "local",
        "/wt",
        &[red("cargo test", "test result: FAILED. 1 failed")],
    );

    assert!(err.is_none());
    assert!(h.rows().is_empty(), "no bell for a red suite");
    assert!(h.events().is_empty());
}
