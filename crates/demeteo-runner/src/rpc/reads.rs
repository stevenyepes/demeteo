use crate::services::RunnerServices;
use demeteo_core::domain::ids::{FeatureId, ThreadId};
use demeteo_core::domain::models::{Feature, Message, SequenceStateMirror, StepExecution};
use demeteo_core::ports::run_events::RunEvent;
use demeteo_core::ports::runner_run::RunnerRun;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::ownership::require_owner;
use super::RunIdParams;

#[derive(Debug, Deserialize)]
struct StreamEventsParams {
    run_id: String,
    #[serde(default)]
    from_offset: i64,
}

#[derive(Debug, Deserialize)]
struct SequenceStateParams {
    run_id: String,
    node_id: String,
}

#[derive(Debug, Deserialize)]
struct ReadArtifactParams {
    run_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct ListMessagesParams {
    run_id: String,
    thread_id: String,
}

/// List runs owned by `client_id` only (MC-D2). The unfiltered `list()`
/// returns *every* client's runs; the multi-client contract is that a
/// client sees only its own, so filter here. A legacy client (`""`) sees
/// the legacy tenant's runs, matching pre-multi-client behavior.
pub(super) fn list_runs(
    svc: &Arc<RunnerServices>,
    client_id: &str,
) -> Result<Vec<RunnerRun>, String> {
    Ok(svc
        .ctx
        .runner_runs
        .list()?
        .into_iter()
        .filter(|r| r.owner_client_id == client_id)
        .collect())
}

/// `RunnerRun` plus a couple of fields the return inbox (M6.2) needs
/// that `RunnerRun` alone doesn't carry, since they live on the
/// runner's own `features`/gate state, not the coarse run-status column:
///
/// - `mr_url` — the PR/MR URL, once one exists, to deep-link "PR ready".
/// - `parked_gate_id` — set when a *dangerous* gate is currently parked
///   awaiting a human (M5.1). Unlike `over-budget`/`needs-credentials`,
///   a parked gate doesn't change `RunnerRun.status` (the feature is
///   still nominally "running", just blocked on this one decision), so
///   without this field the inbox can't distinguish "parked, needs you"
///   from "running fine" for an unattended run.
#[derive(Debug, Serialize)]
pub(super) struct RunStatusView {
    #[serde(flatten)]
    run: RunnerRun,
    mr_url: Option<String>,
    parked_gate_id: Option<String>,
}

pub(super) async fn get_status(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunStatusView, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = require_owner(svc, &params.run_id, client_id)?;

    let mut mr_url = None;
    let mut parked_gate_id = None;
    if let Some(fid) = run.feature_id.as_ref() {
        if let Ok(Some(feature)) = svc.ctx.features.get(&FeatureId::from(fid.clone())) {
            mr_url = feature.mr_url.clone();
            if let Ok(Some(gate_dec)) = svc.ctx.presenter.gate_pending_for_run(fid).await {
                if crate::run::gate_is_dangerous(&svc.ctx, &feature, &gate_dec) {
                    parked_gate_id = Some(gate_dec.step_execution_id.as_str().to_string());
                }
            }
        }
    }
    Ok(RunStatusView {
        run,
        mr_url,
        parked_gate_id,
    })
}

/// Resolve a `run_id` to the `FeatureId` its background execution
/// bootstrapped, **gated by ownership** (MC-D2). The C4 read-model RPCs
/// (`get_feature`/`list_steps`/`read_artifact`/`list_messages`/
/// `get_worktree`) all key on `run_id` — the laptop's idempotency key —
/// and hop through the run's feature to reach the engine's own
/// `features`/`threads`/artifact state; routing them all through this one
/// resolver means the `require_owner` check is applied uniformly and none
/// can forget it. `Err` (uniform "no such run") if the run is unknown,
/// owned by another client, or hasn't reached feature-bootstrap yet.
fn feature_id_for_run(
    svc: &Arc<RunnerServices>,
    run_id: &str,
    client_id: &str,
) -> Result<FeatureId, String> {
    let run = require_owner(svc, run_id, client_id)?;
    let fid = run
        .feature_id
        .ok_or_else(|| format!("run {} has not bootstrapped a feature yet", run_id))?;
    Ok(FeatureId::from(fid))
}

/// C4.1: the runner's own `Feature` row for a run, so the laptop can
/// hydrate a read-only shadow of it (C4.2) and render it with the same
/// fidelity as a native feature (status/model/mr_url/aggregate cost).
pub(super) fn get_feature(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<Option<Feature>, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id, client_id)?;
    svc.ctx.features.get(&fid)
}

/// C4.1: the run's step executions in creation order, each carrying its
/// own cost/tokens/artifact refs — the shadow step list the laptop
/// hydrates so `RunView::steps` serves a runner feature transparently.
pub(super) fn list_steps(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<Vec<StepExecution>, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id, client_id)?;
    svc.ctx.features.steps_for_feature(&fid)
}

/// The runner's own resume state for one `sequence` node — plan cache,
/// checkpoint, and per-task run rows — so the laptop can mirror a detached
/// run's task list (`hydrate_shadow_feature`) the same way C4.2 already
/// mirrors `Feature`/`StepExecution`. Returned together, in one call, so
/// the laptop's write is never torn across polls: it either gets this
/// node's whole resume state or none of it (root cause of "task list not
/// shown in the implement step for detached runs" — no RPC or write path
/// existed for these three tables at all).
pub(super) fn get_sequence_state(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<SequenceStateMirror, String> {
    let params: SequenceStateParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id, client_id)?;
    let plan_json = svc
        .ctx
        .sequence_resume
        .plan_cache_get(&fid, &params.node_id)?;
    let checkpoint = svc
        .ctx
        .sequence_resume
        .sequence_checkpoint_get(&fid, &params.node_id)?;
    let steps = svc.ctx.features.steps_for_feature(&fid)?;
    let subtask_runs = match steps.iter().find(|s| s.step_id.as_str() == params.node_id) {
        Some(step) => svc.ctx.features.subtask_runs_mirror_for_step(&step.id)?,
        None => Vec::new(),
    };
    Ok(SequenceStateMirror {
        plan_json,
        checkpoint,
        subtask_runs,
    })
}

/// Variant A of the detached-run "Browse Code" fix: the runner's own
/// worktree path + branch for a run, so the laptop can point its existing
/// SFTP file browser at the *runner's* real checkout instead of the path it
/// would compute from the shadow's re-homed local project (which is wrong —
/// the code lives in the runner's workspace under the runner's project id).
///
/// The returned `machine_id` is always `"local"` (the runner is `LocalOnly`
/// — it is the machine); the laptop ignores it and substitutes the mirror's
/// real machine id, reaching this path over the same SSH it already holds to
/// the runner's box. No file bytes cross the control socket here — only the
/// path — so this widens nothing: reads go over the laptop's existing SFTP,
/// exactly like Browse Code on any other SSH machine.
pub(super) async fn get_worktree(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<demeteo_core::application::worktree::FeatureWorktreeInfo, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id, client_id)?;
    demeteo_core::application::worktree::resolve_feature_worktree(&svc.ctx, &fid).await
}

/// C4.1: the UTF-8 body of one declared artifact, for the laptop's lazy
/// artifact cache (C4.2). **Guarded:** the requested `path` must be a
/// declared artifact of one of the run's steps — the control socket is
/// not a general remote-file read (a bare `read_file` would let any
/// tunnelled caller exfiltrate arbitrary files as the runner user). The
/// read itself goes through the engine's own `ExecutionPort`, which on
/// the runner is the local subprocess adapter (the runner *is* the
/// machine), and honours the port's error contract: a missing/unreadable
/// path is an `Err`, never `Ok("")`.
pub(super) async fn read_artifact(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<String, String> {
    let params: ReadArtifactParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id, client_id)?;
    let steps = svc.ctx.features.steps_for_feature(&fid)?;
    let refs = steps
        .iter()
        .map(|s| (s.artifact_path.as_deref(), s.artifact_paths.as_slice()));
    if !is_declared_artifact(refs, &params.path) {
        return Err(format!(
            "path is not a declared artifact of run {}: {}",
            params.run_id, params.path
        ));
    }
    svc.ctx.exec.read_file("local", &params.path).await
}

/// The `read_artifact` guard: is `path` a declared artifact among these
/// steps' refs? A step declares artifacts via a single `artifact_path`
/// and/or a `artifact_paths` list; a match on either counts. Pure over
/// the two ref shapes (not `StepExecution`) so it's trivially testable
/// without building the full row.
fn is_declared_artifact<'a>(
    step_refs: impl IntoIterator<Item = (Option<&'a str>, &'a [String])>,
    path: &str,
) -> bool {
    step_refs
        .into_iter()
        .any(|(single, many)| single == Some(path) || many.iter().any(|p| p == path))
}

/// C4.1: a step's persisted agent transcript (the durable message
/// history `RunView::agent_stream` renders), so the laptop shadow can
/// show a runner run's conversation, not just its coarse event log.
///
/// `run_id` is accepted (and validated to exist + have a feature) so the
/// wire shape matches the other C4 read RPCs and a caller can't page a
/// thread on a run that never bootstrapped; the thread itself is trusted
/// by id, exactly as `decide_gate` trusts a bare `gate_id`. The socket's
/// `0600` + SSH-forwarding authz — the same boundary that already grants
/// the laptop full SFTP file read on this box — is the real access
/// control here, not a per-thread ownership check (the engine's
/// `thread_id` is a derived session key not stored on the step row, so
/// re-deriving it to gate reads would be brittle without adding any
/// boundary the tunnel doesn't already imply).
pub(super) fn list_messages(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<Vec<Message>, String> {
    let params: ListMessagesParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    // Presence/bootstrap + ownership check (MC-D2) — surfaces the uniform
    // "no such run" for a bad `run_id` or one owned by another client,
    // instead of silently returning an empty transcript or a foreign one.
    feature_id_for_run(svc, &params.run_id, client_id)?;
    svc.ctx
        .threads
        .get_messages(&ThreadId::from(params.thread_id))
}

/// R9: "catch up on everything missed by offset — never relies on a live
/// socket having been connected." A client (or a laptop mirror, once
/// M6's SSH-forwarding client exists) calls this repeatedly with the
/// highest offset it's already seen; a dropped connection just means the
/// next call's `from_offset` is a little further behind, not a gap.
pub(super) fn stream_events(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<Vec<RunEvent>, String> {
    let params: StreamEventsParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    // MC-D2: the event log is per-run and can carry run detail, so gate it
    // by ownership before returning any events for `run_id`.
    require_owner(svc, &params.run_id, client_id)?;
    svc.ctx
        .run_events
        .list_since(&params.run_id, params.from_offset)
}

#[cfg(test)]
#[path = "reads_tests.rs"]
mod tests;
