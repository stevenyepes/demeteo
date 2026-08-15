// Tests for `src/adapters/step_executor/driver/failure.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// These are the writes a retry decision makes on its way out — and the one
// case where it must make none. `begin_redirect`'s doc has always claimed
// that a dangling `on_failure` target produces `None` with **no writes at
// all**, matching v1; nothing checked it, and the check is only possible
// because these are free functions over the two ports they use rather than
// methods on an eighteen-port driver.
//
// Every double refuses whatever it was not told to answer.

use super::*;
use crate::adapters::step_executor::step_status::CacheTokens;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::models::Feature;
use crate::domain::permission::StepCapability;
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};
use crate::ports::notification::NotificationPort;
use std::sync::Mutex;
use std::time::Instant;

const F_ID: &str = "f-retry";
const P_ID: &str = "p-retry";

// ── doubles ─────────────────────────────────────────────────────────────────

/// Answers `step_update` and `get`, records both, and panics on anything
/// else. `calls` is the whole point of the dangling-target test: it must
/// still be empty afterwards.
#[derive(Default)]
struct FeaturesDouble {
    calls: Mutex<Vec<String>>,
    patches: Mutex<Vec<StepExecutionPatch>>,
}

impl FeaturesDouble {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("not poisoned").clone()
    }
    fn patches(&self) -> Vec<StepExecutionPatch> {
        self.patches.lock().expect("not poisoned").clone()
    }
}

impl FeatureRepository for FeaturesDouble {
    fn step_update(&self, _id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String> {
        self.calls
            .lock()
            .expect("not poisoned")
            .push("step_update".to_string());
        self.patches
            .lock()
            .expect("not poisoned")
            .push(patch.clone());
        Ok(())
    }
    fn get(&self, _id: &FeatureId) -> Result<Option<Feature>, String> {
        self.calls
            .lock()
            .expect("not poisoned")
            .push("get".to_string());
        Ok(Some(feature()))
    }
    fn get_active(&self, _p: &ProjectId) -> Result<Vec<Feature>, String> {
        panic!("unscripted get_active")
    }
    fn add(&self, _f: Feature) -> Result<(), String> {
        panic!("unscripted add")
    }
    fn update(&self, _id: &FeatureId, _patch: &FeaturePatch) -> Result<(), String> {
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

/// Records the status of every `StepProgress` and the reason of every
/// `RetryBudgetExhausted`; any other event is a test bug.
#[derive(Default)]
struct NotifDouble {
    progress: Mutex<Vec<String>>,
    exhausted: Mutex<Vec<String>>,
}

impl NotificationPort for NotifDouble {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        match event {
            DomainEvent::StepProgress { status, .. } => self
                .progress
                .lock()
                .expect("not poisoned")
                .push(status.clone()),
            DomainEvent::RetryBudgetExhausted {
                attempt,
                max,
                reason,
                ..
            } => self
                .exhausted
                .lock()
                .expect("not poisoned")
                .push(format!("{attempt}/{max}: {reason}")),
            other => panic!("a retry write emits neither {other:?}"),
        }
        Ok(())
    }
}

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

struct Harness {
    features: FeaturesDouble,
    notif: NotifDouble,
    notifications: NotificationsDouble,
    f_id: FeatureId,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            features: FeaturesDouble::default(),
            notif: NotifDouble::default(),
            notifications: NotificationsDouble::default(),
            f_id: FeatureId::from(F_ID),
        }
    }
}

impl Harness {
    fn writers(&self) -> StatusWriters<'_> {
        StatusWriters {
            features: &self.features,
            notif: &self.notif,
            f_id: &self.f_id,
        }
    }
    fn progress(&self) -> Vec<String> {
        self.notif.progress.lock().expect("not poisoned").clone()
    }
    fn exhausted(&self) -> Vec<String> {
        self.notif.exhausted.lock().expect("not poisoned").clone()
    }
    fn rows(&self) -> Vec<Notification> {
        self.notifications
            .rows
            .lock()
            .expect("not poisoned")
            .clone()
    }
}

fn feature() -> Feature {
    Feature {
        id: FeatureId::from(F_ID),
        project_id: ProjectId::from(P_ID),
        workflow_id: Some(WorkflowId::from("w-1")),
        workflow_version_id: None,
        title: "retry feature".to_string(),
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
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

fn step_exec() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-retry"),
        feature_id: FeatureId::from(F_ID),
        step_id: StepId::from("s-validate"),
        step_index: 1,
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

fn steps() -> Vec<StepConfig> {
    ["s-implement", "s-validate"]
        .iter()
        .map(|id| StepConfig {
            id: StepId::from(*id),
            kind: "agent".into(),
            title: (*id).into(),
            capability: Some(StepCapability::Implement),
            ..Default::default()
        })
        .collect()
}

fn spend() -> StepSpend {
    StepSpend {
        cost: 1.25,
        tokens: 4_000,
        cache: CacheTokens::default(),
        start: Instant::now(),
    }
}

// ── the dangling on_failure target ──────────────────────────────────────────

/// v1 failed the feature when `on_failure` named a step the run does not
/// have, and it got there without touching the row first. Writing a
/// "retrying: will jump to 's-nowhere'" status and *then* discovering there
/// is nowhere to jump would leave that sentence on the user's screen as the
/// last thing the step ever said.
#[test]
fn a_dangling_redirect_target_writes_nothing() {
    let h = Harness::default();

    let idx = begin_redirect(
        h.writers(),
        &steps(),
        &step_exec(),
        &StepId::from("s-nowhere"),
        "the tests are red",
        RetryBudget { attempt: 2, max: 3 },
        spend(),
    );

    assert_eq!(idx, None);
    assert!(
        h.features.calls().is_empty(),
        "the target is resolved before anything is written"
    );
    assert!(h.progress().is_empty());
}

/// The other side of the same decision: a target that does exist yields its
/// index, writes the failed row with the "will jump" wording, and bumps
/// `iteration_count` to the attempt now starting — which is what the next
/// retry-policy evaluation counts from.
#[test]
fn a_resolvable_redirect_target_writes_the_status_and_bumps_the_iteration() {
    let h = Harness::default();

    let idx = begin_redirect(
        h.writers(),
        &steps(),
        &step_exec(),
        &StepId::from("s-implement"),
        "the tests are red",
        RetryBudget { attempt: 2, max: 3 },
        spend(),
    );

    assert_eq!(idx, Some(0));
    assert_eq!(h.progress(), vec!["failed".to_string()]);

    let patches = h.features.patches();
    assert_eq!(patches.len(), 2, "the status write, then the bump");
    assert_eq!(
        patches[0].error_message,
        Some(Some(
            "the tests are red (retrying: will jump to 's-implement' on attempt 2 of 3)"
                .to_string()
        ))
    );
    assert_eq!(patches[1].iteration_count, Some(2));
}

// ── an exhausted budget ─────────────────────────────────────────────────────

/// The bell row is what survives a refresh and the event is what raises the
/// toast; the failed row is what the ready-set derivation reads next. All
/// three, or the run ends somewhere the user cannot see.
#[test]
fn an_exhausted_budget_writes_the_row_the_bell_and_the_event() {
    let h = Harness::default();

    record_retry_exhausted(
        h.writers(),
        &h.notifications,
        &step_exec(),
        &StepId::from("s-implement"),
        "the tests are red",
        RetryBudget { attempt: 3, max: 3 },
        spend(),
    );

    assert_eq!(h.progress(), vec!["failed".to_string()]);
    assert_eq!(
        h.features.patches()[0].error_message,
        Some(Some(
            "the tests are red (retry budget exhausted: 3 of 3 attempts on 's-implement')"
                .to_string()
        ))
    );

    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0].kind,
        NotificationKind::RetryBudgetExhausted
    ));
    assert_eq!(rows[0].feature_id, F_ID);
    assert_eq!(
        rows[0].feature_url.as_deref(),
        Some("/projects/p-retry/features/f-retry")
    );
    assert!(rows[0].message.contains("after 3 attempt(s)"));

    assert_eq!(
        h.exhausted(),
        vec![
            "3/3: the tests are red (retry budget exhausted: 3 of 3 attempts on 's-implement')"
                .to_string()
        ]
    );
}
