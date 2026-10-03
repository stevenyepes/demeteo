use super::{git_request_vec, GitOpsHelper};
use crate::adapters::git_push::{
    clone_request, credential_for_remote, redacted, remote_user, GitCredential,
};
use crate::domain::git_push::{embeds_secret, host_without_port, token_free_origin};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ExecutionPort, ProgramRequest};
#[cfg(feature = "keyring")]
use keyring::Entry;
use std::time::Duration;

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

/// Deadline for the origin probe and rewrite in [`prepare_origin`]: both are
/// local config reads and writes, so it only ever fires on a wedged transport.
const ORIGIN_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Argv for the clone itself, after the credential pair [`clone_request`]
/// puts in front.
///
/// The URL names the provider's username and no token: the PAT reaches git
/// through the credential helper, so it is in neither argv nor the `origin`
/// the clone records in its own config.
///
/// `core.longpaths` is the one setting that also has to ride the command line:
/// git reads it from *repository* config, and during a clone there is no
/// repository yet to have read it from. Every other setting is applied
/// afterwards by [`configure_clone`] — see [`super::git_request`] for why that
/// asymmetry is not a matter of taste.
fn clone_args(
    user: &str,
    host: &str,
    repo_path: &str,
    target_dir: &str,
    windows_target: bool,
) -> Vec<String> {
    let mut args = Vec::new();
    if windows_target {
        args.push("-c".to_string());
        args.push(format!("{}={}", LONG_PATHS.0, LONG_PATHS.1));
    }
    args.push("clone".to_string());
    args.push(format!("https://{user}@{host}/{repo_path}"));
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

fn bounded(request: ProgramRequest) -> ProgramRequest {
    ProgramRequest {
        timeout: Some(ORIGIN_PROBE_TIMEOUT),
        ..request
    }
}

/// `origin` as git resolves it, and the token-free form of it when it embeds
/// a password.
struct Origin {
    url: String,
    clean: Option<String>,
}

/// One bounded `remote get-url origin`, with the pure [`token_free_origin`]
/// deciding whether it embeds a password. `None` when git cannot say.
///
/// `get-url` expands `insteadOf`, which is what git itself fetches from and so
/// the right thing to decide a credential on — and the wrong thing to write
/// back, see [`migrate_origin`].
async fn probe_origin(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Option<Origin> {
    let url = exec
        .run_program(
            machine_id,
            bounded(git_request_vec(
                repo_dir,
                ["remote", "get-url", "origin"].map(String::from).to_vec(),
            )),
        )
        .await
        .ok()?
        .trim()
        .to_string();
    let clean = token_free_origin(&url);
    Some(Origin { url, clean })
}

/// The credential a fetch from `repo_dir` needs, after moving a password out
/// of its `origin` when one is there to move.
///
/// One `remote get-url origin` serves both questions: [`probe_origin`] decides
/// whether the URL embeds a password, and [`credential_for_remote`] answers the
/// lookup. The rewrite runs first because the lookup declines a
/// password-bearing URL. The migration half is [`migrate_origin`], which is
/// also what [`scrub_origin`] runs on its own. An unreadable origin degrades to
/// no migration and no credential, as in
/// [`credential_for_repo`](crate::adapters::git_push::credential_for_repo).
pub(crate) async fn prepare_origin(
    exec: &dyn ExecutionPort,
    app_settings: &dyn AppSettingsRepository,
    machine_id: &str,
    repo_dir: &str,
) -> Option<GitCredential> {
    let origin = probe_origin(exec, machine_id, repo_dir).await?;
    match origin.clean.as_deref() {
        None => credential_for_remote(app_settings, &origin.url),
        Some(clean) => {
            migrate_origin(exec, app_settings, machine_id, repo_dir, &origin.url, clean).await
        }
    }
}

/// [`prepare_origin`] for a caller that wants the migration and not the
/// credential.
///
/// The provider lookup behind a credential reaches the OS keyring, which can
/// prompt to unlock, so a token-free, ssh, path or unreadable origin stops
/// after the probe and never gets there.
pub(crate) async fn scrub_origin(
    exec: &dyn ExecutionPort,
    app_settings: &dyn AppSettingsRepository,
    machine_id: &str,
    repo_dir: &str,
) {
    let Some(origin) = probe_origin(exec, machine_id, repo_dir).await else {
        return;
    };
    if let Some(clean) = origin.clean.as_deref() {
        migrate_origin(exec, app_settings, machine_id, repo_dir, &origin.url, clean).await;
    }
}

/// Replace the password-bearing `remote` (its token-free form is `clean`) in
/// the repo's config, and return the credential that now has to carry it.
///
/// The rewrite cannot be undone — the old token is gone from the config once
/// `set-url` returns — so it runs only when the keyring holds *exactly the
/// secret being removed*: a provider for the host, a non-empty PAT, and a PAT
/// equal to the password the URL embeds ([`embeds_secret`], raw or
/// percent-decoded). The keyring is trusted only when it matches. A stale entry,
/// or a different account's token for the same host, would otherwise replace a
/// working credential with one that fails, and nothing could bring the first
/// back. On a mismatch the config is left alone, a redacted warning is logged,
/// and `None` is returned so the fetch authenticates with the URL it already
/// has; the cost is that the token stays in `.git/config` — see
/// `docs/KNOWN_ISSUES.md`. A password the user put there for a host Demeteo has
/// no provider for is theirs to keep, and an empty PAT authenticates nothing.
///
/// `remote` is what `get-url` printed, with any `insteadOf` rule applied; the
/// rewrite writes the repo's *own* config, so it also requires the raw
/// `remote.origin.url` to be that same text. Otherwise the password came from
/// the user's rule, writing `clean` would bake the expansion into the repo and
/// cut it off from the rule, and the fetch already authenticates through it:
/// `None`, config untouched.
///
/// A failed rewrite is not fatal, the credential is still returned because this
/// fetch can authenticate either way. It is only logged if the origin still
/// carries its password once re-read: parallel runs each find the legacy URL and
/// race to `set-url`, and the loser on `config.lock` has nothing to warn about.
async fn migrate_origin(
    exec: &dyn ExecutionPort,
    app_settings: &dyn AppSettingsRepository,
    machine_id: &str,
    repo_dir: &str,
    remote: &str,
    clean: &str,
) -> Option<GitCredential> {
    let credential = credential_for_remote(app_settings, clean)?;
    if credential.pat.is_empty() {
        return None;
    }
    if !embeds_secret(remote, &credential.pat) {
        tracing::warn!(
            repo_dir,
            "origin embeds a token that does not match the keyring's: left in place, not migrated"
        );
        return None;
    }
    let raw = exec
        .run_program(
            machine_id,
            bounded(git_request_vec(
                repo_dir,
                ["config", "--get", "remote.origin.url"]
                    .map(String::from)
                    .to_vec(),
            )),
        )
        .await
        .ok();
    if raw.as_deref().map(str::trim) != Some(remote) {
        return None;
    }
    let set_url = ["remote", "set-url", "origin", clean]
        .map(String::from)
        .to_vec();
    if let Err(error) = exec
        .run_program(machine_id, bounded(git_request_vec(repo_dir, set_url)))
        .await
    {
        let still_carries_password = probe_origin(exec, machine_id, repo_dir)
            .await
            .is_none_or(|origin| origin.clean.is_some());
        if still_carries_password {
            tracing::warn!(
                repo_dir,
                error = redacted(&error.replace(remote, "<origin>"), &credential.pat),
                "origin still carries its password: set-url failed"
            );
        }
    }
    Some(credential)
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

        let credential = GitCredential {
            user: remote_user(&provider.kind),
            pat,
            host: host_without_port(&provider.host).to_string(),
        };

        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let windows_target = clones_to_windows(machine_str);
        if let Some(parent) = std::path::Path::new(target_dir).parent() {
            let parent = parent.to_string_lossy().into_owned();
            self.exec.create_dir_all(machine_str, &parent).await?;
        }
        let args = clone_args(
            credential.user,
            &provider.host,
            repo_path,
            target_dir,
            windows_target,
        );
        self.exec
            .run_program(machine_str, clone_request(args, &credential))
            .await
            .map_err(|e| redacted(&e, &credential.pat))?;
        configure_clone(self.exec.as_ref(), machine_str, target_dir, windows_target).await?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/clone.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/origin_migration.rs"]
mod migration_tests;
