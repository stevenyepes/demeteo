//! In-memory, run-scoped git-provider credential store
//! (docs/REMOTE_EXECUTION.md M4.2, docs/REMOTE_EXECUTION.md §6.2).
//!
//! Holds the PAT the laptop injects via `inject_credentials` keyed by
//! `run_id`. Deliberately the *only* place this process holds a git
//! secret: never written to the runner's SQLite, artifacts, git config,
//! or logs. Entries are removed the moment a run reaches a terminal
//! state (success, failure, or cancel) — a compromised *idle* runner
//! therefore has no git secret to steal.

use demeteo_core::ports::git_credentials::GitCredentialSource;
use demeteo_core::ports::runner_run::RunnerRunPort;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
pub struct CredentialStore {
    pats: Mutex<HashMap<String, String>>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-supplying a PAT for a run already holding one overwrites it —
    /// this is the "re-inject after a runner restart" path (§6.2), not
    /// just first-injection.
    pub fn insert(&self, run_id: &str, pat: String) {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), pat);
    }

    pub fn get(&self, run_id: &str) -> Option<String> {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(run_id)
            .cloned()
    }

    /// Wipe the credential for a run. Called on every terminal state
    /// (completed / failed / cancelled / pr_ready) so the PAT's
    /// in-memory lifetime never outlives the run it was injected for.
    pub fn remove(&self, run_id: &str) {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(run_id);
    }
}

/// Answers core's origin git calls with the PAT of the run that owns the
/// repository, resolved repo dir → project → run → [`CredentialStore`].
///
/// Never keyed on the provider id: every run registers the same
/// `runner-provider` row (`INSERT OR REPLACE`), so it names no run. A step or
/// sync worktree and a dependency cache are siblings of their repo dir under
/// the same project root, so they resolve to the run their repo does. Any miss
/// is an `Err`; the call then runs unauthenticated rather than with another
/// run's token.
///
/// `runner_runs` is bound after the context is built, because the context
/// takes this source and cannot be reached from it.
pub struct RunnerCredentialSource {
    creds: Arc<CredentialStore>,
    projects_dir: PathBuf,
    runner_runs: OnceLock<Arc<dyn RunnerRunPort>>,
}

impl RunnerCredentialSource {
    pub fn new(creds: Arc<CredentialStore>, workspace_dir: &Path) -> Self {
        Self {
            creds,
            projects_dir: workspace_dir.join("projects"),
            runner_runs: OnceLock::new(),
        }
    }

    pub fn bind(&self, runner_runs: Arc<dyn RunnerRunPort>) {
        let _ = self.runner_runs.set(runner_runs);
    }

    fn project_id_of(&self, repo_dir: &str) -> Option<String> {
        let rest = Path::new(repo_dir).strip_prefix(&self.projects_dir).ok()?;
        if rest.components().any(|c| matches!(c, Component::ParentDir)) {
            return None;
        }
        match rest.components().next()? {
            Component::Normal(id) => id.to_str().map(str::to_string),
            _ => None,
        }
    }
}

impl GitCredentialSource for RunnerCredentialSource {
    fn pat_for(&self, _provider_id: &str, repo_dir: &str) -> Result<String, String> {
        let project_id = self
            .project_id_of(repo_dir)
            .ok_or("repository is not under a project of this runner")?;
        let runs = self
            .runner_runs
            .get()
            .ok_or("run registry is not bound yet")?
            .list()?;
        let mut pats: Vec<String> = Vec::new();
        for run in runs
            .iter()
            .filter(|r| r.project_id.as_deref() == Some(project_id.as_str()))
        {
            if let Some(pat) = self.creds.get(&run.run_id) {
                if !pats.contains(&pat) {
                    pats.push(pat);
                }
            }
        }
        match pats.len() {
            1 => Ok(pats.remove(0)),
            0 => Err("no run of this project holds a credential".to_string()),
            _ => Err("runs of this project hold different credentials".to_string()),
        }
    }
}

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod tests;
