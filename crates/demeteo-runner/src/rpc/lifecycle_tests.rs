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
