use rusqlite::Connection;

use super::super::super::SqliteAdapter;
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline, HarnessBaselineRun};
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::Feature;
use crate::domain::models::Project;
use crate::domain::models::{EffortLevel, StepOverride};
use crate::ports::db::ProjectRepository;
use crate::ports::db::{FeaturePatch, FeatureRepository};

fn setup() -> SqliteAdapter {
    let conn = Connection::open_in_memory().unwrap();
    SqliteAdapter::new(conn).unwrap()
}

fn make_feature(adapter: &SqliteAdapter, id: &str, project_id: &str) -> FeatureId {
    let fid = FeatureId::from(id.to_string());
    let pid = ProjectId::from(project_id.to_string());
    let _ = ProjectRepository::add(
        adapter,
        Project {
            id: pid.clone(),
            name: format!("project_{}", project_id),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    FeatureRepository::add(
        adapter,
        Feature {
            effort: None,
            id: fid.clone(),
            project_id: pid,
            workflow_id: None,
            workflow_version_id: None,
            title: "Test Feature".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
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
        },
    )
    .unwrap();
    fid
}

#[test]
fn feature_description_round_trips_through_get_and_get_active() {
    let adapter = setup();
    let pid = ProjectId::from("p_desc".to_string());
    let fid = FeatureId::from("f_desc".to_string());
    let _ = ProjectRepository::add(
        &adapter,
        Project {
            id: pid.clone(),
            name: "desc project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    let body = "Implement OAuth\n\nWith PKCE and refresh tokens.";
    FeatureRepository::add(
        &adapter,
        Feature {
            effort: None,
            id: fid.clone(),
            project_id: pid.clone(),
            workflow_id: None,
            workflow_version_id: None,
            title: "Add OAuth".to_string(),
            description: body.to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
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
        },
    )
    .unwrap();

    // Persisted, and returned by the single-feature read the pipeline view uses.
    assert_eq!(adapter.get(&fid).unwrap().unwrap().description, body);
    // ...and by the active-list read the project home renders.
    let active = adapter.get_active(&pid).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].description, body);
}

#[test]
fn feature_max_budget_usd_round_trips_through_get_and_get_active() {
    let adapter = setup();
    let pid = ProjectId::from("p_budget".to_string());
    let fid = FeatureId::from("f_budget".to_string());
    let _ = ProjectRepository::add(
        &adapter,
        Project {
            id: pid.clone(),
            name: "budget project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    // A sub-dollar value guards against an INTEGER column or a truncating
    // cast: REAL must round-trip the fraction exactly.
    FeatureRepository::add(
        &adapter,
        Feature {
            effort: None,
            id: fid.clone(),
            project_id: pid.clone(),
            workflow_id: None,
            workflow_version_id: None,
            title: "Budgeted".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: Some(12.5),
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
        },
    )
    .unwrap();

    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().max_budget_usd,
        Some(12.5)
    );
    let active = adapter.get_active(&pid).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].max_budget_usd, Some(12.5));
}

#[test]
fn feature_update_status_preserves_cost_and_duration() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f13", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("completed".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.status, "completed");
    assert_eq!(f.total_cost, 0.0);
    assert_eq!(f.duration, "0s");
}

#[test]
fn feature_update_cost_set_explicitly() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f14", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: Some(Some(99.9)),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 99.9);
}

#[test]
fn feature_update_cost_skipped_with_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f15", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: None,
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 0.0);
}

#[test]
fn feature_update_cost_flattened_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f16", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 0.0);
}

/// AC7: a feature launched with an effort and a per-step effort override reads
/// back with exactly those values (the per-step one riding inside the
/// `step_overrides_json` blob, not a column of its own).
#[test]
fn feature_effort_and_step_override_effort_round_trip() {
    let adapter = setup();
    let pid = ProjectId::from("p_effort".to_string());
    let fid = FeatureId::from("f_effort".to_string());
    let _ = ProjectRepository::add(
        &adapter,
        Project {
            id: pid.clone(),
            name: "effort project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    FeatureRepository::add(
        &adapter,
        Feature {
            id: fid.clone(),
            project_id: pid.clone(),
            workflow_id: None,
            workflow_version_id: None,
            title: "Effort".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            effort: Some(EffortLevel::XHigh),
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: vec![StepOverride {
                step_id: "s-impl".to_string(),
                agent_kind: None,
                model: None,
                effort: Some(EffortLevel::Low),
            }],
            attachments: Vec::new(),
            harness_baseline: None,
        },
    )
    .unwrap();

    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.effort, Some(EffortLevel::XHigh));
    assert_eq!(f.step_overrides[0].effort, Some(EffortLevel::Low));
    // ...and through the active-list read the project home uses.
    let active = adapter.get_active(&pid).unwrap();
    assert_eq!(active[0].effort, Some(EffortLevel::XHigh));
}

/// A row written before V29 has `effort` NULL, and a row carrying a value this
/// build doesn't know (a downgrade, a hand-edited DB) must degrade to "inherit"
/// rather than failing the read.
#[test]
fn feature_effort_is_none_when_absent_or_unknown() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_stale", "p_stale");
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);

    adapter
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE features SET effort = 'ultra' WHERE id = ?1",
            rusqlite::params![fid.0],
        )
        .unwrap();
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);
}

/// `Some(Some(v))` pins the effort; `Some(None)` clears it back to inherit;
/// a `None` patch field leaves the column alone.
#[test]
fn feature_patch_sets_and_clears_effort() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_patch_effort", "p_patch_effort");

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            effort: Some(Some(EffortLevel::Max)),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().effort,
        Some(EffortLevel::Max)
    );

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("done".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().effort,
        Some(EffortLevel::Max)
    );

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            effort: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);
}

// ── Harness baseline (V37, decision 44) ──────────────────────────────────────

fn baseline_run(name: &str, exit_ok: bool, fingerprint: &str) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok,
        fingerprint: fingerprint.to_string(),
        output_ref: Some(format!("/artifacts/{name}.log")),
        environment: None,
        failing_tests: None,
        measured_at: 1_700,
        producer: BaselineProducer::Node,
    }
}

#[test]
fn feature_harness_baseline_round_trips_unchanged() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_baseline", "p_baseline");
    let record = HarnessBaseline {
        base_sha: "0f1e2d3c".to_string(),
        harnesses: vec![
            baseline_run("lint", true, ""),
            baseline_run("unit", false, "assertion failed: <WT>/src/lib.rs:12"),
        ],
    };

    FeatureRepository::merge_harness_baseline(&adapter, &fid, &record).unwrap();

    let stored = adapter
        .get(&fid)
        .unwrap()
        .unwrap()
        .harness_baseline
        .unwrap();
    assert_eq!(
        stored, record,
        "the record must survive the column verbatim"
    );
    // ...and through the list read the project home renders, which has its
    // own column list and its own row mapping.
    let active = adapter
        .get_active(&ProjectId::from("p_baseline".to_string()))
        .unwrap();
    assert_eq!(active[0].harness_baseline.as_ref(), Some(&record));
}

#[test]
fn feature_harness_baseline_survives_the_insert_path() {
    // `add` is the path a detached run's whole-`Feature` mirror takes on the
    // first poll; `merge_harness_baseline` never runs on the desktop.
    let adapter = setup();
    let pid = ProjectId::from("p_insert".to_string());
    let fid = make_feature(&adapter, "f_insert_seed", "p_insert");
    let mut feature = adapter.get(&fid).unwrap().unwrap();
    feature.id = FeatureId::from("f_inserted".to_string());
    feature.harness_baseline = Some(HarnessBaseline {
        base_sha: "cafebabe".to_string(),
        harnesses: vec![baseline_run("unit", false, "fp")],
    });
    FeatureRepository::add(&adapter, feature.clone()).unwrap();

    let stored = adapter
        .get(&FeatureId::from("f_inserted".to_string()))
        .unwrap()
        .unwrap();
    assert_eq!(stored.harness_baseline, feature.harness_baseline);
    let _ = pid;
}

#[test]
fn feature_harness_baseline_replicates_through_the_update_patch() {
    // The `hydrate_shadow_feature` path. Missing this site fails only on
    // *update*, silently, and only on a detached run — the desktop would show
    // a subtraction it could not explain.
    let adapter = setup();
    let fid = make_feature(&adapter, "f_replicated", "p_replicated");
    let record = HarnessBaseline {
        base_sha: "9988ff".to_string(),
        harnesses: vec![baseline_run("integration", false, "fp-int")],
    };

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("running".to_string()),
            harness_baseline: Some(Some(record.clone())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().harness_baseline,
        Some(record)
    );

    // A patch that says nothing about the baseline leaves it alone...
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("done".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(adapter
        .get(&fid)
        .unwrap()
        .unwrap()
        .harness_baseline
        .is_some());

    // ...and `Some(None)` clears it back to "no baseline measured".
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            harness_baseline: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(adapter
        .get(&fid)
        .unwrap()
        .unwrap()
        .harness_baseline
        .is_none());
}

#[test]
fn a_partial_measurement_merges_into_the_stored_record() {
    // HB2b's lazy fallback measures the one gate that went red. The other
    // gates' measurements must survive the write.
    let adapter = setup();
    let fid = make_feature(&adapter, "f_partial", "p_partial");
    FeatureRepository::merge_harness_baseline(
        &adapter,
        &fid,
        &HarnessBaseline {
            base_sha: "abc123".to_string(),
            harnesses: vec![
                baseline_run("lint", true, ""),
                baseline_run("unit", true, ""),
            ],
        },
    )
    .unwrap();

    let mut remeasured = baseline_run("unit", false, "fp-unit");
    remeasured.producer = BaselineProducer::Fallback;
    FeatureRepository::merge_harness_baseline(
        &adapter,
        &fid,
        &HarnessBaseline {
            base_sha: "abc123".to_string(),
            harnesses: vec![remeasured],
        },
    )
    .unwrap();

    let stored = adapter
        .get(&fid)
        .unwrap()
        .unwrap()
        .harness_baseline
        .unwrap();
    assert_eq!(stored.harnesses.len(), 2, "the untouched gate must survive");
    assert!(stored.harness("lint").unwrap().exit_ok);
    let unit = stored.harness("unit").unwrap();
    assert!(!unit.exit_ok);
    assert_eq!(unit.producer, BaselineProducer::Fallback);
}

#[test]
fn a_feature_with_no_baseline_reads_as_absent_not_as_green() {
    // The inversion HB2c's decision table cannot survive: "nobody measured"
    // must never arrive looking like "everything passed".
    let adapter = setup();
    let fid = make_feature(&adapter, "f_none", "p_none");
    let feature = adapter.get(&fid).unwrap().unwrap();
    assert!(
        feature.harness_baseline.is_none(),
        "an unmeasured feature has no baseline at all"
    );
    let active = adapter
        .get_active(&ProjectId::from("p_none".to_string()))
        .unwrap();
    assert!(active[0].harness_baseline.is_none());
}

#[test]
fn a_corrupt_baseline_column_reads_as_absent() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_corrupt", "p_corrupt");
    {
        let conn = adapter.conn.lock().unwrap();
        conn.execute(
            "UPDATE features SET harness_baseline_json = ?2 WHERE id = ?1",
            rusqlite::params![fid.0, "{ this is not json"],
        )
        .unwrap();
    }
    assert!(adapter
        .get(&fid)
        .unwrap()
        .unwrap()
        .harness_baseline
        .is_none());
}

#[test]
fn the_baseline_column_is_restored_on_a_database_that_predates_v37() {
    // A database whose `refinery` history already records V37 will never
    // re-run the `.sql`, so a row created before the column existed is only
    // rescued by the defensive `add_column_if_missing`. Dropping the column
    // from a migrated database reproduces exactly that state.
    let mut conn = Connection::open_in_memory().unwrap();
    crate::adapters::database::migration::run(&mut conn).unwrap();
    conn.execute("ALTER TABLE features DROP COLUMN harness_baseline_json", [])
        .unwrap();

    let adapter = SqliteAdapter::new(conn).unwrap();
    let fid = make_feature(&adapter, "f_pre_v37", "p_pre_v37");
    let record = HarnessBaseline {
        base_sha: "77aa".to_string(),
        harnesses: vec![baseline_run("unit", false, "fp")],
    };
    FeatureRepository::merge_harness_baseline(&adapter, &fid, &record).unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().harness_baseline,
        Some(record)
    );
}
