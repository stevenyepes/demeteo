//! Proving the repository is actually on the machine that is about to work in
//! it, and saying what to do when it is not.
//!
//! One `ExecutionPort` call and a string. It reads no other port and no
//! executor state, which is what lets it be tested against a single double
//! rather than an `ExecutionDriver` — and its output is the first thing a user
//! sees when a workspace was never bootstrapped, so the remediation sentence is
//! the load-bearing part, not the exit status.

use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Marker the probe echoes so its answer survives whatever the login shell
/// prints around it. Both halves of the check read it: the `exists` line is the
/// verdict, and the leading `home=`/`pwd=` line tells a reader of a failure
/// which account and directory the probe actually ran as.
const DIAG: &str = "__DEMETEO_DIAG__";

/// Verify `target_dir` exists on `machine_id`, and report what was seen if it
/// does not.
///
/// Runs identically on every transport — this is one `run_command`, and the
/// caller has already resolved the path for local or remote. That is the point:
/// a probe that branched here would be testing something other than what the
/// run is about to do.
///
/// The `ls -la` of the *parent* is in the probe for the failure message's sake:
/// an empty listing is the signature of a workspace whose bootstrap clone never
/// ran, which is the one cause the user can fix themselves, and the remediation
/// names it.
pub(crate) async fn verify_repo_present(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    target_dir: &str,
) -> Result<(), String> {
    let parent_dir = std::path::Path::new(target_dir)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let probe = format!(
        "echo {DIAG} home=\\\"$HOME\\\" pwd=\\\"$PWD\\\"; \
         ls -la {} 2>&1; \
         test -d {} && echo {DIAG} exists || echo {DIAG} missing",
        paths::shell_escape_posix(&parent_dir),
        paths::shell_escape_posix(target_dir),
    );
    let probe_output = exec
        .run_command(machine_id, &probe)
        .await
        .unwrap_or_else(|e| format!("probe failed: {}", e));
    if probe_output.contains(&format!("{DIAG} exists")) {
        return Ok(());
    }
    Err(format!(
        "Repository target dir does not exist on '{}': {}\n\
         Remote diagnostic probe output:\n{}\n\n\
         If the parent dir listing is empty, the bootstrap clone \
         did not actually run for this project — re-save the \
         workspace settings to trigger a fresh bootstrap.",
        machine_id, target_dir, probe_output
    ))
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/impl_traits/repo_probe.rs"]
mod tests;
