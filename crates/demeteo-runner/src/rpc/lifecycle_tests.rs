// Tests extracted from `crates/demeteo-runner/src/rpc/lifecycle.rs` (mirrored-tests convention). `super` = that module.

// ── retry_step params (effort re-pin) ───────────────────────────────
//
// Every field on the wire is optional-by-default so a desktop app older
// than this runner keeps working. These pin that contract for `effort`.

#[test]
fn retry_params_without_effort_deserialize_to_none() {
    let params: super::RetryStepParams = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
    }))
    .expect("an old client omits model/agent_kind/effort entirely");
    assert_eq!(params.effort, None);
    assert_eq!(params.model, None);
}

#[test]
fn retry_params_carry_the_effort_re_pin() {
    let params: super::RetryStepParams = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
        "model": "sonnet",
        "effort": "xhigh",
    }))
    .expect("the canonical lowercase spelling is the wire format");
    assert_eq!(
        params.effort,
        Some(demeteo_core::domain::models::EffortLevel::XHigh)
    );
}

#[test]
fn an_unknown_effort_on_the_wire_is_rejected_not_silently_dropped() {
    let res: Result<super::RetryStepParams, _> = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
        "effort": "turbo",
    }));
    assert!(res.is_err());
}

// ── retry_step mode (retry vs replay) ───────────────────────────────
//
// The two rewinds are not interchangeable: the retry arm calls
// `step_retry`, which refuses any step that is not failed / interrupted /
// pending, and it keeps a sequence step's landed prefix. A replay targets
// a completed step and must drop that prefix. Routing both through the
// retry arm is what made remote replay always answer "Cannot retry a step
// in 'completed' status".

#[test]
fn an_omitted_mode_is_a_retry() {
    let params: super::RetryStepParams = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
    }))
    .expect("a desktop older than this runner omits `mode` entirely");
    assert_eq!(
        params.mode,
        super::RetryMode::Retry,
        "the default must stay `retry`, or an old desktop's Retry button \
         silently becomes a Replay and drops a sequence step's landed prefix"
    );
}

#[test]
fn replay_is_selected_by_its_wire_spelling() {
    let params: super::RetryStepParams = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
        "mode": "replay",
    }))
    .expect("snake_case is the wire format");
    assert_eq!(params.mode, super::RetryMode::Replay);
}

#[test]
fn an_unknown_mode_is_rejected_rather_than_defaulting_to_retry() {
    let res: Result<super::RetryStepParams, _> = serde_json::from_value(serde_json::json!({
        "run_id": "run-1",
        "step_execution_id": "se-1",
        "mode": "rewind",
    }));
    assert!(
        res.is_err(),
        "a mode this runner does not understand must fail loudly — silently \
         performing the other rewind is worse than refusing"
    );
}

// The method's tests live in a module named for it so that
// `cargo test -p demeteo-runner set_step_assignment` — the filter matches a
// test's whole path — actually selects them. None of the assertions below
// spells the method in its own name, so without the module that command
// runs zero tests and reports `ok`.
mod set_step_assignment {
    // ── set_step_assignment params ──────────────────────────────────────
    //
    // The wire carries the whole trio, and an absent dimension is *unpinned*,
    // not *unchanged* — so the empty payload is a request, not a no-op.

    #[test]
    fn an_assignment_with_no_dimensions_is_the_reset_to_inherited_request() {
        let params: SetStepAssignmentParams = serde_json::from_value(serde_json::json!({
            "run_id": "run-1",
            "step_execution_id": "se-1",
        }))
        .expect("the reset spelling omits all three dimensions");
        assert!(
            assignment_of(&params).is_inherit(),
            "an empty trio must reach the executor as the un-pin request — \
             treating it as 'nothing to do' would leave Reset with no wire spelling"
        );
    }

    #[test]
    fn an_assignment_carries_all_three_dimensions() {
        let params: SetStepAssignmentParams = serde_json::from_value(serde_json::json!({
            "run_id": "run-1",
            "step_execution_id": "se-1",
            "agent_kind": "claude-code",
            "model": "sonnet",
            "effort": "xhigh",
        }))
        .expect("the canonical lowercase spelling is the wire format");
        assert_eq!(params.agent_kind.as_deref(), Some("claude-code"));
        assert_eq!(params.model.as_deref(), Some("sonnet"));
        assert_eq!(
            params.effort,
            Some(demeteo_core::domain::models::EffortLevel::XHigh)
        );
    }

    // ── set_step_assignment ownership (MC-D2) ───────────────────────────
    //
    // A bare `step_execution_id` would otherwise be a bearer capability: any
    // tunnelled caller who learned one could re-point another client's step at
    // an agent of their choosing. `assignment_target` is the gate, and what it
    // owes beyond refusing is *saying nothing*: a foreign step and an absent
    // one must be indistinguishable, or a client can probe another's ids for
    // existence one refusal at a time.

    use super::super::{assignment_of, assignment_target, SetStepAssignmentParams};
    use demeteo_core::adapters::database::SqliteAdapter;
    use demeteo_core::domain::feature_origin::FeatureOrigin;
    use demeteo_core::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
    use demeteo_core::domain::models::{Feature, Project, StepExecution};
    use demeteo_core::ports::db::{FeatureRepository, ProjectRepository};
    use demeteo_core::ports::runner_run::RunnerRunPort;

    /// Two runs owned by two clients, each with one step:
    /// `se-a` under `run-a` (owned by `client-A`) and `se-b` under `run-b`
    /// (owned by `client-B`).
    fn two_tenant_db() -> SqliteAdapter {
        let db = empty_db();
        seed_tenant(&db, "a", "client-A");
        seed_tenant(&db, "b", "client-B");
        db
    }

    /// The same world with nothing in it — the runner a probed id genuinely
    /// does not exist on. Paired with [`two_tenant_db`] it is what makes the
    /// byte-identity assertion meaningful: the *same* id is asked for on both,
    /// so anything but an identical answer is the probe itself.
    fn empty_db() -> SqliteAdapter {
        let dir = std::env::temp_dir().join(format!(
            "demeteo_runner_assign_{}_{}_{}",
            std::process::id(),
            demeteo_core::paths::now_ms(),
            NEXT_DB.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let conn = demeteo_core::db::init_db(dir).expect("init_db");
        let db = SqliteAdapter::new(conn).expect("migrations run");
        ProjectRepository::add(
            &db,
            Project {
                id: ProjectId::from("p-assign"),
                name: "assign".to_string(),
                compute_type: "local".to_string(),
                remote_host: None,
                status: "idle".to_string(),
                nodes: 0,
                spend: 0.0,
                tokens: 0,
                created_at: 1_700_000_000,
            },
        )
        .expect("seed the project");
        db
    }

    /// `now_ms()` alone collides between two databases built inside the same
    /// millisecond, and they would then share one file.
    static NEXT_DB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn seed_tenant(db: &SqliteAdapter, suffix: &str, client_id: &str) {
        let feature_id = FeatureId::from(format!("f-{}", suffix));
        FeatureRepository::add(
            db,
            Feature {
                id: feature_id.clone(),
                project_id: ProjectId::from("p-assign"),
                workflow_id: None,
                workflow_version_id: None,
                title: format!("feature {}", suffix),
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
                mr_state: Some("none".to_string()),
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
            },
        )
        .expect("seed the feature");
        FeatureRepository::step_create(
            db,
            StepExecution {
                id: StepExecutionId::from(format!("se-{}", suffix)),
                feature_id: feature_id.clone(),
                step_id: StepId::from("s-implement".to_string()),
                step_index: 0,
                step_kind: "agent".to_string(),
                status: "pending".to_string(),
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
            },
        )
        .expect("seed the step");
        let run_id = format!("run-{}", suffix);
        RunnerRunPort::get_or_create(db, &run_id, "{}", client_id, 1_700_000_000)
            .expect("seed the run");
        RunnerRunPort::update_status(
            db,
            &run_id,
            "running",
            Some("p-assign"),
            Some(feature_id.as_str()),
            None,
            None,
            1_700_000_000,
        )
        .expect("attach the feature to the run");
    }

    fn params(run_id: &str, step_execution_id: &str) -> SetStepAssignmentParams {
        SetStepAssignmentParams {
            run_id: run_id.to_string(),
            step_execution_id: step_execution_id.to_string(),
            agent_kind: Some("claude-code".to_string()),
            model: None,
            effort: None,
        }
    }

    #[test]
    fn the_owner_reaches_its_own_step() {
        let db = two_tenant_db();

        assert_eq!(
            assignment_target(&db, &db, &params("run-b", "se-b"), "client-B"),
            Ok(()),
            "the gate must let the owning client through, or nothing else it \
             refuses means anything"
        );
    }

    #[test]
    fn a_foreign_step_is_indistinguishable_from_an_absent_one() {
        // The same id, asked for twice: once where it exists under another
        // client's run, once where it exists nowhere at all. Both answers must
        // be the same bytes — asking for a different id in each would compare
        // nothing, since the id is inside the message.
        let foreign = assignment_target(
            &two_tenant_db(),
            &two_tenant_db(),
            &params("run-a", "se-a"),
            "client-B",
        )
        .expect_err("client-B does not own se-a");
        let absent = assignment_target(
            &empty_db(),
            &empty_db(),
            &params("run-a", "se-a"),
            "client-B",
        )
        .expect_err("this runner has never heard of se-a");

        assert_eq!(
            foreign, absent,
            "a step that exists but isn't yours must refuse in exactly the same \
             bytes as one that does not exist — any difference is an existence \
             probe over the tunnel (MC-D2)"
        );
        assert_eq!(absent, "no such step: se-a");
    }

    #[test]
    fn naming_the_wrong_run_for_your_own_step_refuses_the_same_way() {
        let db = two_tenant_db();

        let mismatched = assignment_target(&db, &db, &params("run-a", "se-b"), "client-B")
            .expect_err("se-b belongs to run-b, not the run-a the caller named");

        assert_eq!(
            mismatched,
            assignment_target(&db, &db, &params("run-a", "se-b"), "client-A")
                .expect_err("and client-A cannot reach se-b at all"),
            "the pairing check must not answer differently from the ownership \
             check, or it re-opens the probe the ownership check closes"
        );
    }
}
