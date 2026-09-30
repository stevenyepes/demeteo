use super::client_id::{install_id_in, stamp_client_id};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::state::AppContext;

pub(super) fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

pub(super) async fn remote_rpc(
    ctx: &AppContext,
    machine_id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    remote_rpc_via(&*ctx.exec, &*ctx.app_settings, machine_id, method, params).await
}

/// [`remote_rpc`] over the two ports it reads, for a caller that holds no
/// [`AppContext`].
pub(super) async fn remote_rpc_via(
    exec: &dyn ExecutionPort,
    settings: &dyn AppSettingsRepository,
    machine_id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client_id = install_id_in(settings)?;
    let params = stamp_client_id(params, &client_id);
    exec.control_rpc(machine_id, method, params).await
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/rpc.rs"]
mod tests;
