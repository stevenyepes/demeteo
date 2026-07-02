use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

const WORKSPACE_BASE_DIR_KEY: &str = "workspace_base_dir";

/// Identity bundle returned to the frontend on startup.
///
/// The frontend renders these on the About screen: `version` comes from
/// `CARGO_PKG_VERSION` (always matches `tauri.conf.json`); `channel` comes
/// from the compile-time `DEMETEO_RELEASE_CHANNEL` env var, defaulting to
/// `"stable"`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppInfo {
    pub version: String,
    pub channel: String,
}

#[tauri::command]
pub fn get_app_session(
    ctx: State<'_, AppContext>,
    key: String,
) -> Result<Option<String>, AppError> {
    ctx.app_settings
        .get_app_session(&key)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_app_session(
    ctx: State<'_, AppContext>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    ctx.app_settings
        .set_app_session(&key, &value)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn delete_app_session(ctx: State<'_, AppContext>, key: String) -> Result<(), AppError> {
    ctx.app_settings
        .delete_app_session(&key)
        .map_err(AppError::from)
}

/// Returns the binary's version + release channel for the About screen.
///
/// Pure constants — no `State` lookup required.
#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        channel: crate::RELEASE_CHANNEL.to_string(),
    }
}

/// Returns the **effective** workspace directory currently in use.
/// This is the resolved value (override if set, otherwise app data dir).
#[tauri::command]
pub fn get_workspace_dir(ctx: State<'_, AppContext>) -> String {
    ctx.workspace_dir.to_string_lossy().to_string()
}

/// Returns the stored workspace directory override (or `None` if using default).
#[tauri::command]
pub fn get_workspace_dir_setting(ctx: State<'_, AppContext>) -> Result<Option<String>, AppError> {
    ctx.app_settings
        .get_app_session(WORKSPACE_BASE_DIR_KEY)
        .map(|v| v.filter(|s| !s.trim().is_empty()))
        .map_err(AppError::from)
}

/// Persist a workspace directory override.
///
/// Pass `None` (or an empty string) to clear the override and revert to
/// the default app data directory. The change takes effect after restarting
/// the app; existing projects remain in their current location until
/// re-bootstrapped.
#[tauri::command]
pub fn set_workspace_dir_setting(
    ctx: State<'_, AppContext>,
    path: Option<String>,
) -> Result<(), AppError> {
    let value = path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_default();
    ctx.app_settings
        .set_app_session(WORKSPACE_BASE_DIR_KEY, &value)
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `get_app_info` must surface both `CARGO_PKG_VERSION` and the
    /// compile-time `RELEASE_CHANNEL` constant.
    #[test]
    fn app_info_matches_constants() {
        let info = get_app_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.channel, crate::RELEASE_CHANNEL);
    }

    /// Default channel is `"stable"` when `DEMETEO_RELEASE_CHANNEL` is not
    /// set at compile time. Override the constant via env to verify the
    /// fallback path; since the constant is baked at compile time, this
    /// test simply ensures the chosen value is one of the two known
    /// channels (a regression guard, not a true env-driven test).
    #[test]
    fn release_channel_is_a_known_value() {
        let c = crate::RELEASE_CHANNEL;
        assert!(
            c == "stable" || c == "nightly",
            "unexpected release channel: {c}"
        );
    }

    /// The serialized payload must use snake-case-friendly field names so
    /// the TS wrapper can decode without manual renaming.
    #[test]
    fn app_info_serializes_field_names() {
        let info = AppInfo {
            version: "0.1.0".to_string(),
            channel: "nightly".to_string(),
        };
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["channel"], "nightly");
        assert!(json.get("version").is_some());
        assert!(json.get("channel").is_some());
    }
}
