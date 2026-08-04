//! Putting the bytes a prompt points at where the agent can actually read them.
//!
//! Both functions dispatch through `machine_id` and must keep doing so — see
//! [`materialize_external_artifact_paths`], which records what happened the last
//! time one of them used host-local `std::fs`.

use crate::domain::attachment::AttachedFile;
use crate::ports::attachment_store::AttachmentStore;
use crate::ports::execution::ExecutionPort;

/// Copy each user attachment into `{wt_path}/artifacts/_context/attachments/`
/// so the agent's `external_directory: deny` accepts the file when its
/// `Read` tool is called on it. Idempotent: re-running with the same
/// `(sha256, ext)` is a no-op when the destination already exists.
/// Logs a warning when the on-disk size differs from the recorded
/// `size` (sha256 hash mismatch is the most likely cause).
///
/// `exec` and `machine_id` identify the target worktree's host. Reads
/// always come from the local FS attachment store; writes go through
/// the machine-aware exec port (SFTP for remote, `std::fs` for local)
/// so the file lands where the agent will run.
pub(crate) async fn materialize_user_attachments_to_worktree(
    feature_id: &str,
    attachments: &[AttachedFile],
    attachment_store: &dyn AttachmentStore,
    wt_path: &str,
    exec: &dyn ExecutionPort,
    machine_id: &str,
) -> Vec<String> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let dest_root = std::path::Path::new(wt_path)
        .join("artifacts")
        .join("_context")
        .join("attachments");
    let dest_root_str = dest_root.to_string_lossy().to_string();
    // Ensure the destination directory exists on the target machine
    // (works for both local and remote via the exec port). Fail loud
    // here so the caller surfaces the error instead of silently
    // shipping a prompt that points at files the agent can't read.
    if exec
        .create_dir_all(machine_id, &dest_root_str)
        .await
        .is_err()
    {
        tracing::warn!(
            dest_root = %dest_root_str,
            machine_id = machine_id,
            "failed to create user-attachments _context/ dir on target machine; \
             agent reads will be blocked by external_directory: deny"
        );
        return Vec::new();
    }

    let mut copied = Vec::with_capacity(attachments.len());
    for att in attachments {
        let ext = crate::domain::attachment::resolved_ext(att);
        let src_path = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
        if !src_path.exists() {
            tracing::warn!(
                feature_id = feature_id,
                sha256 = %att.sha256,
                "user attachment source file is missing on disk; skipping pre-spawn copy"
            );
            continue;
        }
        let dest = dest_root.join(format!("{}.{}", att.sha256, ext));
        let dest_str = dest.to_string_lossy().to_string();
        // Idempotency check via exec.get_metadata so it dispatches
        // SFTP for remote and stat() for local — same primitive either
        // way. When the file already exists we sanity-check the size
        // against the source bytes (sha256 collision is impossible in
        // practice; this catches stale-file bugs).
        let already_exists =
            matches!(exec.get_metadata(machine_id, &dest_str).await, Ok(meta) if !meta.is_dir);
        if already_exists {
            if let (Ok(src_bytes), Ok(dst_meta)) = (
                std::fs::read(&src_path),
                exec.get_metadata(machine_id, &dest_str).await,
            ) {
                let src_len = src_bytes.len() as u64;
                if dst_meta.size != src_len {
                    tracing::warn!(
                        feature_id = feature_id,
                        src = %src_path.display(),
                        dst = %dest.display(),
                        src_bytes = src_len,
                        dst_bytes = dst_meta.size,
                        sha256 = %att.sha256,
                        "user-attach re-copy found existing worktree file with different size; \
                         possible stale copy or sha256 collision"
                    );
                }
            }
            copied.push(dest_str);
            continue;
        }
        // Read the source locally (FsAttachmentStore is host-local)
        // and push the bytes to the target machine via exec — the
        // binary-safe variant handles image/png / application/pdf
        // attachments that the String overload would mangle.
        match std::fs::read(&src_path) {
            Ok(bytes) => match exec.write_file_bytes(machine_id, &dest_str, &bytes).await {
                Ok(_) => {
                    copied.push(dest_str);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        feature_id = feature_id,
                        src = %src_path.display(),
                        dst = %dest_str,
                        "failed to copy user attachment into worktree _context/"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    feature_id = feature_id,
                    src = %src_path.display(),
                    "failed to read user attachment source from disk"
                );
            }
        }
    }
    copied
}

/// Copy any external artifact paths referenced in a path-manifest prompt
/// into `{wt_path}/artifacts/_context/` so the agent can read them
/// without needing `external_directory: allow`.
///
/// Opencode's `external_directory: deny` restricts all tool access to
/// the worktree `--dir`. Artifact paths in path manifests are absolute
/// paths under the app data directory (e.g. `~/Library/Application
/// Support/…/artifacts/…`) — outside the worktree. This function
/// copies those files into the worktree before the agent runs so the
/// Read tool succeeds.
///
/// Path manifests use the format `- \`/absolute/path\`` (one path per
/// bullet). Any absolute path NOT already under `wt_path` is copied to
/// `{wt_path}/artifacts/_context/` and the path is rewritten
/// in the returned prompt.
///
/// `exec` and `machine_id` identify the target worktree's host — the
/// write goes through the machine-aware exec port (SFTP for remote
/// machines, `std::fs` for local) so the bytes actually land where the
/// agent will run. Using host-local `std::fs` here was the regression
/// that broke the simple-task pipeline on remote machines: the
/// implement step's opencode agent ended up with a path manifest
/// pointing at a path that exists only on the Tauri host, and the
/// `external_directory: deny` fence then blocked every Read.
pub(crate) async fn materialize_external_artifact_paths(
    prompt: &str,
    wt_path: &str,
    exec: &dyn ExecutionPort,
    machine_id: &str,
) -> String {
    let wt = std::path::Path::new(wt_path);
    let mut result = prompt.to_string();
    let mut rewrites: Vec<(String, String)> = Vec::new();

    // Scan for backtick-quoted absolute paths: `- `/some/path`
    let mut search = prompt;
    while let Some(tick_pos) = search.find("- `") {
        let after_tick = &search[tick_pos + 3..];
        if !after_tick.starts_with('/') {
            search = &search[tick_pos + 1..];
            continue;
        }
        let close = match after_tick.find('`') {
            Some(p) => p,
            None => break,
        };
        let abs_path = &after_tick[..close];
        let path = std::path::Path::new(abs_path);

        if !path.starts_with(wt)
            && !rewrites.iter().any(|(old, _)| old == abs_path)
            && path.is_file()
        {
            if let Some(file_name) = path.file_name() {
                let dest_dir = wt.join("artifacts").join("_context");
                let dest = dest_dir.join(file_name);
                let dest_str = dest.to_string_lossy().to_string();
                let dest_dir_str = dest_dir.to_string_lossy().to_string();

                // Source is always the local FS artifact store; read it
                // locally. Push the bytes to the worktree via the
                // machine-aware exec port so remote worktrees receive
                // the file over SSH and local worktrees stay on std::fs.
                // The previous implementation used std::fs::copy and
                // std::fs::create_dir_all unconditionally, which
                // silently failed (or created a phantom local file at
                // the remote worktree's path string) for remote steps
                // — see AGENTS.md / docs for the regression writeup.
                if let Ok(content) = std::fs::read_to_string(path) {
                    if exec.create_dir_all(machine_id, &dest_dir_str).await.is_ok()
                        && exec
                            .write_file(machine_id, &dest_str, &content)
                            .await
                            .is_ok()
                    {
                        rewrites.push((abs_path.to_string(), dest_str));
                    }
                }
            }
        }
        search = &search[tick_pos + 1..];
    }

    for (old, new) in &rewrites {
        result = result.replace(old.as_str(), new.as_str());
    }
    result
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/materialize.rs"]
mod tests;
