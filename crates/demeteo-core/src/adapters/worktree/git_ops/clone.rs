use super::{git_request_vec, GitOpsHelper};
use crate::ports::execution::{ExecutionPort, ProgramRequest};
#[cfg(feature = "keyring")]
use keyring::Entry;

/// Line endings are decided once, in Demeteo's own clone, and never again.
///
/// Git for Windows ships `core.autocrlf=true`, so a checkout there rewrites
/// every text file to CRLF — including the checked-in shell script a project's
/// own test command runs, which `bash` then rejects with `bad interpreter`.
/// The same feature is green on the always-Linux runner and red on a Windows
/// desktop, for a file no step touched: a parity break with no code change
/// behind it.
///
/// `false` rather than `input` because the property that matters is that the
/// index and the working tree hold the same bytes, and `input` only promises
/// that for files that are already LF on disk.
const AUTOCRLF: (&str, &str) = ("core.autocrlf", "false");

/// Raises the child toolchains' path ceiling; it does **not** raise Demeteo's
/// own. `CreateProcessW` receives `lpCurrentDirectory` with the `\\?\` prefix
/// deliberately stripped by std, so an agent spawn into a deep worktree fails
/// before `node_modules` does — short path segments are the fix for that, and
/// this is the fix for everything running *inside* the worktree afterwards.
const LONG_PATHS: (&str, &str) = ("core.longpaths", "true");

/// Whether the clone lands on a Windows filesystem.
///
/// Only the desktop host can be Windows: remote execution is Linux-only (R2,
/// `docs/REMOTE_EXECUTION.md`), so a named machine is a Linux machine no
/// matter what the desktop runs.
fn clones_to_windows(machine_id: &str) -> bool {
    cfg!(windows) && crate::domain::ids::MachineId::from(machine_id.to_string()).is_local()
}

/// The settings written into the clone's own config, in application order.
fn clone_config(windows_target: bool) -> Vec<(&'static str, &'static str)> {
    let mut config = vec![AUTOCRLF];
    if windows_target {
        config.push(LONG_PATHS);
    }
    config
}

/// `git config` argv, one per setting, for a repository reached with `-C`.
///
/// `--local` is not redundant with `-C`: without it a `git config` that cannot
/// see a repository falls through to the user's `~/.gitconfig`, which is the
/// one file Demeteo may never write (AGENTS.md §2). With it, the same case is
/// an error.
fn clone_config_args(windows_target: bool) -> Vec<Vec<String>> {
    clone_config(windows_target)
        .into_iter()
        .map(|(key, value)| {
            vec![
                "config".to_string(),
                "--local".to_string(),
                key.to_string(),
                value.to_string(),
            ]
        })
        .collect()
}

/// Argv for the clone itself.
///
/// `core.longpaths` is the one setting that also has to ride the command line:
/// git reads it from *repository* config, and during a clone there is no
/// repository yet to have read it from. Every other setting is applied
/// afterwards by [`configure_clone`] — see [`super::git_request`] for why that
/// asymmetry is not a matter of taste.
fn clone_args(clone_url: &str, target_dir: &str, windows_target: bool) -> Vec<String> {
    let mut args = Vec::new();
    if windows_target {
        args.push("-c".to_string());
        args.push(format!("{}={}", LONG_PATHS.0, LONG_PATHS.1));
    }
    args.push("clone".to_string());
    args.push(clone_url.to_string());
    args.push(target_dir.to_string());
    args
}

/// Write Demeteo's settings into the clone it just made.
///
/// Linked worktrees share the common config, so every subtask worktree cut
/// from this clone inherits the same answer without a second write and without
/// a chance to disagree with the index.
async fn configure_clone(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    target_dir: &str,
    windows_target: bool,
) -> Result<(), String> {
    for args in clone_config_args(windows_target) {
        exec.run_program(machine_id, git_request_vec(target_dir, args))
            .await
            .map_err(|e| format!("Failed to configure clone at {}: {}", target_dir, e))?;
    }
    Ok(())
}

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
        let windows_target = clones_to_windows(machine_str);
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
                    args: clone_args(&clone_url, target_dir, windows_target),
                    ..ProgramRequest::default()
                },
            )
            .await?;
        println!("[GitOps] Clone output: {}", output);
        configure_clone(self.exec.as_ref(), machine_str, target_dir, windows_target).await?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/clone.rs"]
mod tests;
