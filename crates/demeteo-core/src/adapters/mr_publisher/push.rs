use std::sync::Arc;

use crate::ports::execution::{ExecutionPort, ProgramRequest};

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
    exec.run_program(
        machine_str,
        git_request(&target_dir, ["remote", "set-url", "origin", &remote_url]),
    )
    .await
    .map_err(|e| format!("Failed to update remote origin URL: {}", e))?;

    // Short-lived askpass helper: the PAT lives only in this file's
    // bytes (written directly, never interpolated into a shell command
    // string) for the brief window between writing it and deleting it after
    // the push. The structured invocation carries only its path in the
    // environment, never the secret itself.
    let askpass_path = format!("{}/.demeteo-mr-askpass.sh", target_dir);
    let askpass_script = format!(
        "#!/bin/sh\nprintf '%s' '{}'\n",
        push.pat.replace('\'', "'\\''")
    );
    exec.write_file_bytes(machine_str, &askpass_path, askpass_script.as_bytes())
        .await
        .map_err(|e| format!("Failed to write askpass helper: {}", e))?;
    if let Err(error) = exec.set_file_mode(machine_str, &askpass_path, 0o700).await {
        let _ = exec.remove_file(machine_str, &askpass_path).await;
        return Err(format!("Failed to protect askpass helper: {}", error));
    }

    // Push the local feature branch to origin remote before creating MR.
    // We use `-f` to force push so subsequent publish_mr calls can update
    // the remote branch if the feature was retried/replayed. The askpass
    // helper is removed immediately after regardless of outcome.
    let result = exec
        .run_program(
            machine_str,
            ProgramRequest {
                executable: "git".to_string(),
                args: vec![
                    "-C".to_string(),
                    target_dir,
                    "push".to_string(),
                    "-f".to_string(),
                    "origin".to_string(),
                    push.source_branch.to_string(),
                ],
                env: [
                    ("GIT_ASKPASS".to_string(), askpass_path.clone()),
                    ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
                ]
                .into_iter()
                .collect(),
                ..ProgramRequest::default()
            },
        )
        .await;
    let cleanup = exec.remove_file(machine_str, &askpass_path).await;
    if let Err(error) = result {
        return Err(format!(
            "Failed to push feature branch to origin: {}",
            error
        ));
    }
    cleanup.map_err(|e| format!("Failed to remove askpass helper: {}", e))?;

    Ok(())
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(str::to_string).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}
