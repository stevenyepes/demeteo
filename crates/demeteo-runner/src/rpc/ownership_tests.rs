// Tests extracted from `crates/demeteo-runner/src/rpc/ownership.rs` (mirrored-tests convention). `super` = that module.

use super::{check_owner, no_such_run};
use demeteo_core::ports::runner_run::RunnerRun;

fn run_owned_by(owner: &str) -> RunnerRun {
    RunnerRun {
        run_id: "run-1".to_string(),
        project_id: None,
        feature_id: None,
        spec_json: "{}".to_string(),
        status: "running".to_string(),
        error: None,
        created_at: 0,
        updated_at: 0,
        resume_count: 0,
        pushed_branch: None,
        owner_client_id: owner.to_string(),
    }
}

#[test]
fn owner_match_returns_the_run() {
    let ok = check_owner(Some(run_owned_by("client-A")), "run-1", "client-A");
    assert!(ok.is_ok());
}

#[test]
fn wrong_owner_is_indistinguishable_from_absent() {
    // The load-bearing MC-D2 property: a run owned by *another* client
    // and a genuinely-absent run return the SAME error, so ownership
    // leaks no existence signal — a client can't probe foreign run ids.
    let foreign = check_owner(Some(run_owned_by("client-A")), "run-1", "client-B").unwrap_err();
    let absent = check_owner(None, "run-1", "client-B").unwrap_err();
    assert_eq!(foreign, absent);
    assert_eq!(foreign, no_such_run("run-1"));
}

#[test]
fn empty_client_is_the_single_legacy_tenant() {
    // Two legacy clients (or a pre-V26 run) share `owner_client_id ==
    // ""` — documented single-tenant behavior (Risk §7.1), not a leak.
    assert!(check_owner(Some(run_owned_by("")), "run-1", "").is_ok());
    // …but a real client id still can't reach a legacy-owned run.
    assert!(check_owner(Some(run_owned_by("")), "run-1", "client-A").is_err());
}
