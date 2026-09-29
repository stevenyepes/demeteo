use crate::ports::db::AppSettingsRepository;

const INSTALL_ID_KEY: &str = "install_id";

pub(super) fn install_id_in(settings: &dyn AppSettingsRepository) -> Result<String, String> {
    if let Some(id) = settings.app_setting_get(INSTALL_ID_KEY)? {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = format!("client-{}", crate::paths::new_id());
    settings.app_setting_set(INSTALL_ID_KEY, &id)?;
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
