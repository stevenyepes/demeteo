use std::sync::Arc;

use crate::ports::execution::ExecutionPort;

pub(super) struct BranchPush<'a> {
    pub compute_type: &'a str,
    pub remote_host: Option<&'a str>,
    pub project_id: &'a str,
    pub workspace_dir: &'a std::path::Path,
    pub repo_path: &'a str,
    pub provider_kind: &'a str,
    pub provider_host: &'a str,
    pub pat: &'a str,
    pub source_branch: &'a str,
}

pub(super) async fn push_feature_branch(
    exec: &Arc<dyn ExecutionPort>,
    push: &BranchPush<'_>,
) -> Result<(), String> {
    // Resolve target directory of the repository.
    let target_dir = if push.compute_type.eq_ignore_ascii_case("local") {
        crate::paths::repo_target_dir_local(push.workspace_dir, push.project_id, push.repo_path)
            .to_string_lossy()
            .to_string()
    } else {
        crate::paths::repo_target_dir_str(
            exec,
            push.compute_type,
            push.remote_host,
            push.project_id,
            push.repo_path,
            None,
        )
        .await?
    };

    let machine_str = push
        .remote_host
        .unwrap_or(crate::domain::ids::LOCAL_MACHINE);

    // Point origin at a token-free URL — only a bare provider
    // username, never the PAT. The PAT itself is supplied to `git
    // push` via a short-lived GIT_ASKPASS helper below, mirroring
    // the runner's own M4.3 hardening
    // (crates/demeteo-runner/src/git_askpass.rs): embedding it in
    // the URL instead would put it in this process's argv (visible
    // via `ps`/`/proc/<pid>/cmdline` to any local user) *and*
    // persist it in `.git/config` on disk indefinitely via `git
    // remote set-url`.
    let remote_user = if push.provider_kind.to_lowercase() == "github" {
        "x-access-token"
    } else {
        "oauth2"
    };
    let remote_url = format!(
        "https://{}@{}/{}",
        remote_user, push.provider_host, push.repo_path
    );
    let set_url_cmd = format!(
        "git -C {} remote set-url origin {}",
        crate::paths::shell_escape_posix(&target_dir),
        crate::paths::shell_escape_posix(&remote_url)
    );
    exec.run_command(machine_str, &set_url_cmd)
        .await
        .map_err(|e| format!("Failed to update remote origin URL: {}", e))?;

    // Short-lived askpass helper: the PAT lives only in this file's
    // bytes (written directly, never interpolated into a shell
    // command string) for the brief window between writing it and
    // deleting it after the push. `ExecutionPort::run_command` has
    // no per-call env-var API (it's a single opaque shell string
    // for both the local and SSH-remote adapters), so this is the
    // one available mechanism that works for both without ever
    // putting the secret in argv.
    let askpass_path = format!("{}/.demeteo-mr-askpass.sh", target_dir);
    let askpass_script = format!(
        "#!/bin/sh\nprintf '%s' '{}'\n",
        push.pat.replace('\'', "'\\''")
    );
    exec.write_file_bytes(machine_str, &askpass_path, askpass_script.as_bytes())
        .await
        .map_err(|e| format!("Failed to write askpass helper: {}", e))?;
    exec.run_command(
        machine_str,
        &format!(
            "chmod 700 {}",
            crate::paths::shell_escape_posix(&askpass_path)
        ),
    )
    .await
    .map_err(|e| format!("Failed to chmod askpass helper: {}", e))?;

    // Push the local feature branch to origin remote before creating MR.
    // We use `-f` to force push so subsequent publish_mr calls can update
    // the remote branch if the feature was retried/replayed. The askpass
    // helper is removed immediately after regardless of outcome.
    let push_cmd = format!(
        "GIT_ASKPASS={} GIT_TERMINAL_PROMPT=0 git -C {} push -f origin {}; rc=$?; rm -f {}; exit $rc",
        crate::paths::shell_escape_posix(&askpass_path),
        crate::paths::shell_escape_posix(&target_dir),
        crate::paths::shell_escape_posix(push.source_branch),
        crate::paths::shell_escape_posix(&askpass_path),
    );
    exec.run_command(machine_str, &push_cmd)
        .await
        .map_err(|e| format!("Failed to push feature branch to origin: {}", e))?;

    Ok(())
}
