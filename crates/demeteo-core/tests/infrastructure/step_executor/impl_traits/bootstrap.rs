// Tests for `src/adapters/step_executor/impl_traits/bootstrap.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// The e2e `origin_cut` suite covers what the row holds after a cut that
// worked. This covers the other half — the write failing — which was
// discarded with `let _ =` until it became fatal, and which no test could
// have observed while it lived inside the bootstrap's `async fn`.
//
// Against the real adapter rather than a double: the error being propagated
// is the one SQLite actually produces, and a double asserting a `map_err`
// against a string it was handed itself proves nothing about the port.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::ProjectId;
use crate::domain::models::{Feature, Project};
use crate::ports::db::{FeatureRepository, ProjectRepository};

const F_ID: &str = "f-record";
const P_ID: &str = "p-record";
const BRANCH: &str = "demeteo/features/f-record";

/// A database with the feature's project already in it — `features` carries a
/// foreign key onto `projects`.
fn seeded() -> SqliteAdapter {
    let db = SqliteAdapter::new(rusqlite::Connection::open_in_memory().expect("in-memory db"))
        .expect("migrations run");
    ProjectRepository::add(
        &db,
        Project {
            id: ProjectId::from(P_ID),
            name: "record".to_string(),
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
    FeatureRepository::add(&db, feature()).expect("seed the feature");
    db
}

fn feature() -> Feature {
    Feature {
        id: FeatureId::from(F_ID),
        project_id: ProjectId::from(P_ID),
        workflow_id: None,
        workflow_version_id: None,
        title: "record the branch".to_string(),
        description: String::new(),
        status: "bootstrapping".to_string(),
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
    }
}

#[test]
fn the_recorded_branch_is_what_later_readers_get_back() {
    let db = seeded();

    record_run_branch(&db, &FeatureId::from(F_ID), BRANCH).expect("record the branch");

    let stored = db
        .get(&FeatureId::from(F_ID))
        .expect("read back")
        .expect("the feature is still there");
    assert_eq!(stored.resolved_branch.as_deref(), Some(BRANCH));
    assert_eq!(stored.run_branch("demeteo/features/"), BRANCH);
}

/// A run whose branch could not be recorded is a run whose branch is
/// derivable only from a `branch_prefix` the user may edit under it, so the
/// failure has to leave the cut phase rather than be swallowed into a
/// "completed" the row does not support.
#[test]
fn a_write_that_fails_is_reported_rather_than_discarded() {
    let db = seeded();
    db.conn
        .lock()
        .expect("not poisoned")
        .execute("DROP TABLE features", [])
        .expect("drop the table the write needs");

    let error = record_run_branch(&db, &FeatureId::from(F_ID), BRANCH)
        .expect_err("the write cannot have succeeded");
    assert!(
        error.contains(BRANCH),
        "the failure has to name the branch that was cut: {error}"
    );
}
