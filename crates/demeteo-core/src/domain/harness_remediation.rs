//! The shape of a terminal environment-not-ready message.
//!
//! Wording, not choreography: what a user is told when a failure is one that
//! editing the code cannot fix. The adapter supplies the machine, the worktree
//! and the command it actually ran; this decides what the reader sees, and is
//! therefore assertable without a port double — which is the whole reason the
//! never-ran fast paths can be tested at all.

/// Build the user-facing environment-not-ready message (C6.3): the triage
/// reason + remediation plus the concrete context the orchestrator holds — the
/// exact failing command, the target machine, and a copy-pasteable reproduce
/// line — so the failure says *what* ran, *where*, and *how to reproduce/fix*.
pub fn build_environment_message(
    machine: &str,
    wt_path: &str,
    cmd: &str,
    reason: &str,
    remediation: &str,
) -> String {
    let reproduce = if machine.is_empty() || machine == "local" {
        format!("  cd {} && {}", wt_path, cmd)
    } else {
        format!("  ssh {}\n  cd {} && {}", machine, wt_path, cmd)
    };
    let remediation_line = if remediation.trim().is_empty() {
        String::new()
    } else {
        format!("\nRemediation: {}\n", remediation.trim())
    };
    format!(
        "Environment not ready — this failure is not something editing the code can fix.\n\n\
         {}\n{}\nFailing command: {}\nMachine: {}\nReproduce:\n{}\n",
        reason.trim(),
        remediation_line,
        cmd,
        if machine.is_empty() { "local" } else { machine },
        reproduce,
    )
}
