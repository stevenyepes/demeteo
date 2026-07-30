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

/// User-facing remediation for a harness command that hit its ceiling.
///
/// A command that produces no exit status inside a generous wall-clock budget
/// is overwhelmingly a runner left in **watch mode**, not a slow suite — and
/// that is a configuration defect no retry can resolve, so the message leads
/// with it. `scripts.test` is very often `vitest` or `jest --watch`, and
/// detection used to emit a bare `npm test` for any repo with a root
/// `package.json` — so this was the default path for a large class of projects,
/// not an exotic one. HB3 now reads the script and either corrects it or
/// declines to emit it, which shrinks the population that reaches here to
/// hand-written commands and watch-mode forms detection does not recognise. It
/// does not empty it: this message is still the only thing standing between a
/// user and a silent half-hour.
pub fn build_timeout_message(
    machine_str: &str,
    wt_path: &str,
    cmd: &str,
    ceiling_s: u64,
) -> String {
    build_environment_message(
        machine_str,
        wt_path,
        cmd,
        &format!(
            "The command produced no exit status within {}s and was abandoned, so nothing was \
             tested. This is not a verdict on the code — the suite never finished running.",
            ceiling_s
        ),
        "The usual cause is a test runner left in **watch mode**, which never exits: `vitest` \
         (use `vitest run`), `jest --watch` (use `jest --ci`), `cargo watch`. Check what the \
         command actually resolves to — for an `npm test` that is the `scripts.test` entry in \
         `package.json` — and change the project's test command to the one-shot form. If the \
         suite is genuinely slower than the ceiling, raise the wall-clock cap in preferences \
         instead.",
    )
}
#[cfg(test)]
#[path = "../../tests/domain/harness_remediation.rs"]
mod tests;
