use super::GitOpsHelper;
use crate::ports::execution::ProgramRequest;
#[cfg(feature = "keyring")]
use keyring::Entry;

impl GitOpsHelper {
    /// Retrieve the token for the given provider from Keyring (cached in-process).
    pub fn get_provider_pat(&self, provider_id: &str) -> Result<String, String> {
        crate::credential_cache::get_or_fetch(provider_id, || {
            #[cfg(feature = "keyring")]
            {
                let entry = Entry::new("demeteo", provider_id)
                    .map_err(|e| format!("Failed to access keyring: {}", e))?;
                entry.get_password().map_err(|e| {
                    format!(
                        "Token not found in keyring for provider '{}': {}",
                        provider_id, e
                    )
                })
            }
            #[cfg(not(feature = "keyring"))]
            {
                Err("OS-keyring credential cache is disabled in this build".to_string())
            }
        })
    }

    /// Run clone operation. Clones to either local or remote path based on compute_type
    pub async fn clone_repository(
        &self,
        machine_id: Option<&str>,
        provider_id: &str,
        repo_path: &str,
        target_dir: &str,
    ) -> Result<(), String> {
        // Resolve provider instance
        let providers = self.app_settings.get_provider_instances()?;
        let provider_id_typed = crate::domain::ids::ProviderId::from(provider_id.to_string());
        let provider = providers
            .into_iter()
            .find(|p| p.id == provider_id_typed)
            .ok_or_else(|| format!("Provider not found in DB: {}", provider_id))?;

        let pat = self.get_provider_pat(provider_id)?;

        // Construct the clone URL with credentials
        let clone_url = if provider.kind.to_lowercase() == "github" {
            format!(
                "https://x-access-token:{}@{}/{}",
                pat, provider.host, repo_path
            )
        } else {
            format!("https://oauth2:{}@{}/{}", pat, provider.host, repo_path)
        };

        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        if let Some(parent) = std::path::Path::new(target_dir).parent() {
            let parent = parent.to_string_lossy().into_owned();
            self.exec.create_dir_all(machine_str, &parent).await?;
        }
        let output = self
            .exec
            .run_program(
                machine_str,
                ProgramRequest {
                    executable: "git".to_string(),
                    args: vec!["clone".to_string(), clone_url, target_dir.to_string()],
                    ..ProgramRequest::default()
                },
            )
            .await?;
        println!("[GitOps] Clone output: {}", output);

        Ok(())
    }
}
