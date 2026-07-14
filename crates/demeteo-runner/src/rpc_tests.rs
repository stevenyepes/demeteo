// Tests extracted from `crates/demeteo-runner/src/rpc.rs` (mirrored-tests convention). `super` = that module.

use super::{check_owner, is_declared_artifact, no_such_run};
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

fn refs<'a>(
    pairs: &'a [(Option<&'a str>, Vec<String>)],
) -> impl IntoIterator<Item = (Option<&'a str>, &'a [String])> {
    pairs.iter().map(|(s, m)| (*s, m.as_slice()))
}

#[test]
fn matches_single_artifact_path() {
    let steps = [(Some("/w/report.md"), vec![])];
    assert!(is_declared_artifact(refs(&steps), "/w/report.md"));
}

#[test]
fn matches_within_artifact_paths_list() {
    let steps = [(None, vec!["/w/a.txt".to_string(), "/w/b.txt".to_string()])];
    assert!(is_declared_artifact(refs(&steps), "/w/b.txt"));
}

#[test]
fn rejects_undeclared_path() {
    // The security-relevant case: a path no step declared must not
    // be readable over the control socket, even a plausible sibling.
    let steps = [
        (Some("/w/report.md"), vec!["/w/a.txt".to_string()]),
        (None, vec!["/w/b.txt".to_string()]),
    ];
    assert!(!is_declared_artifact(refs(&steps), "/w/../.ssh/id_rsa"));
    assert!(!is_declared_artifact(refs(&steps), "/w/report.md.bak"));
    assert!(!is_declared_artifact(refs(&steps), "/etc/passwd"));
}

#[test]
fn rejects_when_no_steps_declare_anything() {
    let steps: [(Option<&str>, Vec<String>); 0] = [];
    assert!(!is_declared_artifact(refs(&steps), "/w/report.md"));
}

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
