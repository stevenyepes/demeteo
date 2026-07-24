use crate::state::AppContext;

const INSTALL_ID_KEY: &str = "install_id";

pub(super) fn client_install_id(ctx: &AppContext) -> Result<String, String> {
    if let Some(id) = ctx.app_settings.app_setting_get(INSTALL_ID_KEY)? {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = format!("client-{}", crate::paths::new_id());
    ctx.app_settings.app_setting_set(INSTALL_ID_KEY, &id)?;
    Ok(id)
}

pub(super) fn stamp_client_id(mut params: serde_json::Value, client_id: &str) -> serde_json::Value {
    if let Some(obj) = params.as_object_mut() {
        obj.insert(
            "client_id".to_string(),
            serde_json::Value::String(client_id.to_string()),
        );
    }
    params
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/client_id.rs"]
mod tests;
