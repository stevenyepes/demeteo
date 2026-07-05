//! Memory agent configuration + secret access, shared by the Tauri commands
//! (read/write from the UI) and the background memory worker (read each tick).

#[cfg(feature = "keyring")]
use keyring::Entry;

use crate::domain::memory::MemoryAgentConfig;
use crate::ports::db::AppSettingsRepository;

/// `app_settings` key holding the JSON-encoded `MemoryAgentConfig`.
pub const CONFIG_KEY: &str = "memory_agent_config";
/// Keyring service + account under which the optional API key is stored.
#[cfg(feature = "keyring")]
const KEYRING_SERVICE: &str = "demeteo";
#[cfg(feature = "keyring")]
const KEYRING_ACCOUNT: &str = "memory_agent_llm";

/// Load the persisted config, falling back to `Default` (disabled) when unset
/// or unparseable.
pub fn load_config(app_settings: &dyn AppSettingsRepository) -> MemoryAgentConfig {
    match app_settings.app_setting_get(CONFIG_KEY) {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => MemoryAgentConfig::default(),
    }
}

/// Persist the config as JSON.
pub fn save_config(
    app_settings: &dyn AppSettingsRepository,
    config: &MemoryAgentConfig,
) -> Result<(), String> {
    let json = serde_json::to_string(config).map_err(|e| e.to_string())?;
    app_settings.app_setting_set(CONFIG_KEY, &json)
}

/// Fetch the optional API key from the OS keyring. Returns `None` if unset,
/// or unconditionally `None` in builds with the `keyring` feature disabled
/// (memory agent is therefore a no-op there).
pub fn load_api_key() -> Option<String> {
    #[cfg(feature = "keyring")]
    {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).ok()?;
        entry.get_password().ok().filter(|k| !k.is_empty())
    }
    #[cfg(not(feature = "keyring"))]
    {
        None
    }
}

/// Store the API key in the keyring.
pub fn set_api_key(key: &str) -> Result<(), String> {
    #[cfg(feature = "keyring")]
    {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| e.to_string())?;
        entry.set_password(key).map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "keyring"))]
    {
        let _ = key;
        Err("OS-keyring credential cache is disabled in this build".to_string())
    }
}

/// Remove the API key from the keyring (ignores "not found"). A no-op
/// (returning `Ok`) in builds with the `keyring` feature disabled.
pub fn clear_api_key() -> Result<(), String> {
    #[cfg(feature = "keyring")]
    {
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            match entry.delete_credential() {
                Ok(()) => {}
                Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
    #[cfg(not(feature = "keyring"))]
    {
        Ok(())
    }
}
