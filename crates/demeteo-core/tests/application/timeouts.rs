// Tests extracted from `crates/demeteo-core/src/application/timeouts.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::ports::db::AppSettingsRepository;
use std::collections::HashMap;
use std::sync::Mutex;

/// Minimal in-memory `AppSettingsRepository` for unit tests.
struct InMemoryAppSettings {
    map: Mutex<HashMap<String, String>>,
}

impl InMemoryAppSettings {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }
}

impl AppSettingsRepository for InMemoryAppSettings {
    fn add_provider_instance(
        &self,
        _p: crate::domain::models::ProviderInstance,
    ) -> Result<(), String> {
        unimplemented!()
    }
    fn get_provider_instances(
        &self,
    ) -> Result<Vec<crate::domain::models::ProviderInstance>, String> {
        unimplemented!()
    }
    fn delete_provider_instance(&self, _id: &crate::domain::ids::ProviderId) -> Result<(), String> {
        unimplemented!()
    }
    fn get_app_session(&self, _key: &str) -> Result<Option<String>, String> {
        unimplemented!()
    }
    fn set_app_session(&self, _key: &str, _value: &str) -> Result<(), String> {
        unimplemented!()
    }
    fn delete_app_session(&self, _key: &str) -> Result<(), String> {
        unimplemented!()
    }
    fn app_setting_get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn app_setting_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
}

#[test]
fn resolve_returns_defaults_when_key_missing() {
    let store = InMemoryAppSettings::new();
    let resolved = resolve_effective(&store);
    assert_eq!(resolved, AgentTimeouts::default());
}

#[test]
fn resolve_returns_defaults_when_json_malformed() {
    let store = InMemoryAppSettings::new();
    store
        .app_setting_set(CONFIG_KEY, "{not valid json")
        .unwrap();
    let resolved = resolve_effective(&store);
    assert_eq!(resolved, AgentTimeouts::default());
}

#[test]
fn save_and_load_round_trip() {
    let store = InMemoryAppSettings::new();
    let cfg = AgentTimeouts {
        fast_timeout_s: 120,
        normal_timeout_s: 240,
        wall_cap_s: 900,
    };
    save(&store, &cfg).unwrap();
    let loaded = load(&store);
    assert_eq!(loaded, cfg);
}

#[test]
fn sanitize_clamps_out_of_range_values() {
    let bogus = AgentTimeouts {
        fast_timeout_s: 999_999, // way over the 3600 cap
        normal_timeout_s: 1,     // under the 10 floor and below fast
        wall_cap_s: 50,          // below normal
    };
    let safe = sanitize(bogus);
    // Should land inside the documented envelope.
    assert!((10..=3600).contains(&safe.fast_timeout_s));
    assert!((10..=7200).contains(&safe.normal_timeout_s));
    assert!((10..=14400).contains(&safe.wall_cap_s));
    assert!(safe.normal_timeout_s >= safe.fast_timeout_s);
    assert!(safe.wall_cap_s >= safe.normal_timeout_s);
}
