use crate::services::RunnerServices;
use serde::Deserialize;
use std::sync::Arc;

use super::ownership::require_owner_of_step;

#[derive(Debug, Deserialize)]
struct DecideGateParams {
    /// The RPC surface is `decide_gate(run_id, gate_id, decision)` (M5.3);
    /// `run_id` isn't needed to apply the decision (`gate_id` — the
    /// step_execution_id — is already globally unique) but is accepted
    /// so a future multi-run laptop can validate the pairing without a
    /// wire-format change.
    #[allow(dead_code)]
    run_id: String,
    gate_id: String,
    decision: String,
    #[serde(default)]
    feedback: Option<String>,
}

/// M5.3: clear a gate the unattended policy parked (`gate_class:
/// "dangerous"`, M5.1) from the laptop. Delegates straight to
/// `GatePresenter::gate_decide` — the same application-level call the
/// desktop app's `gate_decide` Tauri command uses — so there is exactly
/// one gate-decision code path regardless of which side of the tunnel
/// the decision came from. Gated by [`require_owner_of_step`] so a client
/// can only decide gates on runs it owns (MC-D2).
pub(super) async fn decide_gate(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<(), String> {
    let params: DecideGateParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    require_owner_of_step(svc, &params.gate_id, client_id)?;
    svc.ctx
        .presenter
        .gate_decide(
            &params.gate_id,
            &params.decision,
            params.feedback.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())
}
