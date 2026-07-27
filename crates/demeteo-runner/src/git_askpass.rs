//! Per-run git askpass (docs/REMOTE_EXECUTION.md M4.3,
//! docs/REMOTE_EXECUTION.md §6.2 hardening).
//!
//! The M1 stopgap embedded the PAT straight into the clone/push URL
//! (`https://x-access-token:{pat}@host/...`), which puts the secret in
//! the child process's argv — visible to any local user via `ps aux` or
//! `/proc/<pid>/cmdline`, and liable to end up in shell history. This
//! module replaces that: the runner spawns `git` directly (bypassing the
//! generic `ExecutionPort` shell-string API, which has no way to set
//! per-child env without embedding it in the command string) with
//! `GIT_ASKPASS` pointed at a small, secret-free helper script. The PAT
//! itself only ever exists as an env var on that one `git` child process
//! — never in the URL, never in the command line, never written to the
//! runner's disk.
//!
//! The askpass script is written once (its contents contain no secret)
//! and reused for every run.

use std::path::{Path, PathBuf};
use std::process::Stdio;

const ASKPASS_SCRIPT: &str = "#!/bin/sh\nprintf '%s' \"$DEMETEO_GIT_PAT\"\n";
const PAT_ENV_VAR: &str = "DEMETEO_GIT_PAT";

/// Idempotently (re)write the generic askpass helper to
/// `<data_dir>/git-askpass.sh` with `0700` perms and return its path.
pub fn ensure_askpass_script(data_dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("git-askpass.sh");
    std::fs::write(&path, ASKPASS_SCRIPT)?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o700);
    std::fs::set_permissions(&path, perms)?;
    Ok(path)
}

/// Run `git` with the given args. When `pat` is `Some`, `GIT_ASKPASS` is
/// pointed at the helper script and the PAT rides an env var scoped to
/// this single child process only — callers should pass `None` for
/// git operations that never touch `origin` (e.g. a pure local
/// `rev-parse`).
pub async fn run_git(
    askpass_path: &Path,
    args: &[String],
    pat: Option<&str>,
) -> Result<String, String> {
    let askpass_path = askpass_path.to_path_buf();
    let args = args.to_vec();
    let pat = pat.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut cmd = std::process::Command::new("git");
        cmd.args(&args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        demeteo_core::shared::proc::sanitize_child_env(&mut cmd);
        if let Some(pat) = pat.as_deref() {
            cmd.env("GIT_ASKPASS", &askpass_path);
            cmd.env(PAT_ENV_VAR, pat);
            // Never fall back to an interactive terminal prompt — a
            // hung `git` waiting on stdin would wedge the run forever
            // on a headless box.
            cmd.env("GIT_TERMINAL_PROMPT", "0");
        }
        let output = cmd
            .output()
            .map_err(|e| format!("failed to spawn git: {}", e))?;
        let mut out = String::from_utf8_lossy(&output.stdout).to_string();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&stderr);
            return Err(format!(
                "git {:?} failed (exit {:?}): {}",
                args,
                output.status.code(),
                out
            ));
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {}", e))?
}
