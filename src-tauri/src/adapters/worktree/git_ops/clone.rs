use super::GitOpsHelper;
use crate::paths;
use keyring::Entry;

impl GitOpsHelper {
    /// Retrieve the token for the given provider from Keyring (cached in-process).
    pub fn get_provider_pat(&self, provider_id: &str) -> Result<String, String> {
        crate::credential_cache::get_or_fetch(provider_id, || {
            let entry = Entry::new("demeteo", provider_id)
                .map_err(|e| format!("Failed to access keyring: {}", e))?;
            entry.get_password().map_err(|e| {
                format!(
                    "Token not found in keyring for provider '{}': {}",
                    provider_id, e
                )
            })
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

        // Ensure parent directory exists
        let machine_str = machine_id.unwrap_or("local");
        let path = std::path::Path::new(target_dir);
        if let Some(parent) = path.parent() {
            let parent_str = parent.to_str().unwrap_or("");
            self.exec
                .run_command(
                    machine_str,
                    &format!("mkdir -p {}", paths::shell_escape_posix(parent_str)),
                )
                .await?;
        }

        // Run clone
        let clone_cmd = format!(
            "git clone \"{}\" {}",
            clone_url,
            paths::shell_escape_posix(target_dir)
        );
        let output = self.exec.run_command(machine_str, &clone_cmd).await?;
        tracing::debug!(output = %output, "GitOps clone output");

        Ok(())
    }
}
