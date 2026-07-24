use super::client_id::{client_install_id, stamp_client_id};
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
    let client_id = client_install_id(ctx)?;
    let params = stamp_client_id(params, &client_id);
    ctx.exec.control_rpc(machine_id, method, params).await
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/rpc.rs"]
mod tests;
