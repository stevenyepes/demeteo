// Tests for `src/adapters/mr_publisher/provider.rs` (mirrored-tests
// convention). `super` resolves to that module.

use super::*;
use crate::domain::ids::ProviderId;

/// An `AppSettingsRepository` that answers the one call this resolver makes and
/// **panics on everything else** — AGENTS.md §7: a double that answers every
/// call is asserted against a default, not an answer.
struct ProvidersDouble {
    instances: Vec<ProviderInstance>,
}

impl ProvidersDouble {
    fn of(instances: &[(&str, &str, &str)]) -> Self {
        Self {
            instances: instances
                .iter()
                .map(|(id, kind, host)| ProviderInstance {
                    id: ProviderId::from(*id),
                    kind: kind.to_string(),
                    host: host.to_string(),
                    username: "someone".to_string(),
                    avatar_url: String::new(),
                    created_at: 0,
                })
                .collect(),
        }
    }
}

impl AppSettingsRepository for ProvidersDouble {
    fn add_provider_instance(&self, _p: ProviderInstance) -> Result<(), String> {
        panic!("unscripted add_provider_instance")
    }
    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String> {
        Ok(self.instances.clone())
    }
    fn delete_provider_instance(&self, _id: &ProviderId) -> Result<(), String> {
        panic!("unscripted delete_provider_instance")
    }
    fn get_app_session(&self, _key: &str) -> Result<Option<String>, String> {
        panic!("unscripted get_app_session")
    }
    fn set_app_session(&self, _key: &str, _value: &str) -> Result<(), String> {
        panic!("unscripted set_app_session")
    }
    fn delete_app_session(&self, _key: &str) -> Result<(), String> {
        panic!("unscripted delete_app_session")
    }
    fn app_setting_get(&self, _key: &str) -> Result<Option<String>, String> {
        panic!("unscripted app_setting_get")
    }
    fn app_setting_set(&self, _key: &str, _value: &str) -> Result<(), String> {
        panic!("resolving a provider must not write")
    }
}

fn two_instances() -> ProvidersDouble {
    ProvidersDouble::of(&[
        ("prov-gh", "github", "github.com"),
        ("prov-gl", "gitlab", "gitlab.example.com"),
    ])
}

#[test]
fn a_repo_holding_a_host_matches_that_host() {
    let resolved = resolve_provider(&two_instances(), &ProviderId::from("gitlab.example.com"))
        .expect("the host is one of the configured instances");
    assert_eq!(resolved.id.0, "prov-gl");
}

/// The project-creation path writes the instance **id** into
/// `Repository::provider_id`, which matches no host at all.
#[test]
fn a_repo_holding_an_instance_id_matches_that_id() {
    let resolved = resolve_provider(&two_instances(), &ProviderId::from("prov-gh"))
        .expect("the id is one of the configured instances");
    assert_eq!(resolved.host, "github.com");
}

/// A self-hosted host that no longer has an instance row still names its kind,
/// which is enough to publish against the one instance of that kind.
#[test]
fn an_unknown_host_falls_back_to_the_first_instance_of_its_kind() {
    let resolved = resolve_provider(&two_instances(), &ProviderId::from("gitlab.internal.corp"))
        .expect("the kind is inferable from the host");
    assert_eq!(resolved.id.0, "prov-gl");
}
