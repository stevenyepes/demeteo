//! Application service for global agent-turn timeouts.
//!
//! Timeouts are stored as JSON in the `app_settings` KV table under the
//! [`CONFIG_KEY`] key. The shape is [`AgentTimeouts`]; the resolver
//! [`resolve_effective`] returns the configured values or the built-in
//! defaults when nothing is persisted yet.
//!
//! Every agent-turn call site (planner, worker, resolver, verifier, agent
//! step) reads its values through [`resolve_effective`]. This is the single
//! wiring point — change it here once and every turn honors the new value.

use crate::domain::models::{AgentTimeouts, CONFIG_KEY};
use crate::ports::db::AppSettingsRepository;

/// Load the persisted config, falling back to [`AgentTimeouts::default`] when
/// unset or unparseable. Failures (missing key, malformed JSON) all collapse
/// to the default so a corrupt row can never block an agent turn.
pub fn load(app_settings: &dyn AppSettingsRepository) -> AgentTimeouts {
    match app_settings.app_setting_get(CONFIG_KEY) {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => AgentTimeouts::default(),
    }
}

/// Persist the config as JSON.
pub fn save(
    app_settings: &dyn AppSettingsRepository,
    config: &AgentTimeouts,
) -> Result<(), String> {
    let json = serde_json::to_string(config).map_err(|e| e.to_string())?;
    app_settings.app_setting_set(CONFIG_KEY, &json)
}

/// Resolve the effective timeouts for one agent turn. Pulls the persisted
/// config (or defaults) and validates the values; an out-of-range persisted
/// row falls back to defaults rather than blocking the run. This is the
/// function every call site should use.
pub fn resolve_effective(app_settings: &dyn AppSettingsRepository) -> AgentTimeouts {
    let loaded = load(app_settings);
    sanitize(loaded)
}

/// Clamp an arbitrary [`AgentTimeouts`] into the safe operating envelope.
/// Used to recover from a hand-edited `app_settings` row that violates
/// monotonicity (`normal ≥ fast`, `wall ≥ normal`) or the hard caps.
fn sanitize(t: AgentTimeouts) -> AgentTimeouts {
    AgentTimeouts::validated(
        t.fast_timeout_s.clamp(10, 3600),
        t.normal_timeout_s.clamp(10, 7200).max(t.fast_timeout_s),
        t.wall_cap_s.clamp(10, 14400).max(t.normal_timeout_s),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        fn delete_provider_instance(
            &self,
            _id: &crate::domain::ids::ProviderId,
        ) -> Result<(), String> {
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
}
