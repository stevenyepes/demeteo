use crate::application::providers::ProviderValidationResult;
use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::error::AppError;
use crate::state::AppContext;
use keyring::Entry;
use tauri::State;

#[tauri::command]
pub async fn validate_provider_pat(
    ctx: State<'_, AppContext>,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderValidationResult, AppError> {
    crate::application::providers::validate_pat(&ctx, provider_type, host, pat)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn fetch_provider_repos(
    ctx: State<'_, AppContext>,
    provider_id: String,
) -> Result<Vec<String>, AppError> {
    crate::application::providers::fetch_repos(&ctx, provider_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn connect_provider_instance(
    ctx: State<'_, AppContext>,
    provider_type: String,
    host: String,
    pat: String,
) -> Result<ProviderInstance, AppError> {
    crate::application::providers::connect_instance(&ctx, provider_type, host, pat)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn list_provider_instances(
    ctx: State<'_, AppContext>,
) -> Result<Vec<ProviderInstance>, AppError> {
    ctx.app_settings
        .get_provider_instances()
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn delete_provider_instance(
    ctx: State<'_, AppContext>,
    provider_id: String,
) -> Result<(), AppError> {
    let provider_id_typed = ProviderId::from(provider_id.clone());
    if let Ok(entry) = Entry::new("demeteo", &provider_id) {
        let _ = entry.delete_credential();
    }
    crate::credential_cache::invalidate(&provider_id);
    ctx.app_settings
        .delete_provider_instance(&provider_id_typed)
        .map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/providers.rs"]
mod tests;
