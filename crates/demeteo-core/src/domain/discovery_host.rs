//! Which host an interview runs on (`docs/PRD_DISCOVERY.md` §4.5).
//!
//! Separated from `application::discovery::create` because it is the whole of
//! the rule, and because a rule spelled beside the row insert is a rule that
//! reads the caller's input and then quietly does something else — which is
//! the bug this exists to have failed on.

use crate::domain::ids::{MachineId, LOCAL_MACHINE};

/// The machine a new Discovery is opened against.
///
/// `chosen` is the picker's value, blank or absent when the user left it
/// alone. What stands in for it is the project's own host, because that is
/// where Demeteo cloned the repository — but §4.5 makes the host part of the
/// interviewer choice, so a value the user did give is never overridden by it.
///
/// Whether the named machine is configured is not answered here; that is a
/// lookup, and this stays reachable from a test without one.
pub fn interviewer_machine(
    chosen: Option<&str>,
    project_is_local: bool,
    project_host: Option<&str>,
) -> Result<MachineId, String> {
    if let Some(chosen) = chosen.map(str::trim).filter(|c| !c.is_empty()) {
        return Ok(MachineId::from(chosen.to_string()));
    }
    if project_is_local {
        return Ok(MachineId::from(LOCAL_MACHINE.to_string()));
    }
    project_host
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(|h| MachineId::from(h.to_string()))
        .ok_or_else(|| "Remote project has no configured machine".to_string())
}

#[cfg(test)]
#[path = "../../tests/domain/discovery_host.rs"]
mod tests;
