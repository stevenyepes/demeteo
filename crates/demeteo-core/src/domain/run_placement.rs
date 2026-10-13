//! Where a run is launched, and which launch options that destination accepts.
//!
//! **A placement is a destination, not a transport.** [`RunPlacement::Local`]
//! means "the project's own compute, through the step executor": the desktop
//! for a local project, and the project's machine — attached over SSH — for a
//! remote one. [`RunPlacement::Detached`] means "submitted to that machine's
//! `demeteo-runner`, which drives it with nobody attached". Local, SSH and the
//! runner all stay behind `ExecutionPort`; nothing here, and nothing that
//! reads a placement, asks which of them carries the run (AGENTS.md §2).
//! `application::launch::launch_run` holds the one `match` on it.
//!
//! **Why a remote project's own host defaults to `Local`.** A Discovery on a
//! remote project is opened against the project's host
//! (`domain/discovery_host.rs`), and tickets there have always run on that
//! host, attached. Reading "default to the Discovery's machine" literally
//! would turn every such ticket detached, and refuse all of them on a host
//! with no runner installed. So a ticket defaults to detached only when its
//! Discovery ran somewhere *other* than the project's compute. Switching to
//! the literal rule is the one branch in [`default_ticket_placement`] that
//! compares against `project_compute_host`.
//!
//! **Why detached is always unattended.** No human is attached to a detached
//! run to answer a gate, so asking for an attended one is refused rather than
//! sent and left parked forever. For the same reason, detached-only options
//! given with a local placement are refused rather than silently dropped: the
//! caller asked for something the run would not have honoured.
//!
//! See [`crate::domain`] for why this is synchronous and port-free.

use serde::{Deserialize, Serialize};

use crate::domain::ids::{MachineId, LOCAL_MACHINE};
use crate::domain::models::Machine;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunPlacement {
    Local,
    Detached { machine_id: MachineId },
}

impl RunPlacement {
    /// The id this placement is stored as — [`placement_for`]'s inverse.
    pub fn machine_id(&self) -> MachineId {
        match self {
            RunPlacement::Local => MachineId::from(LOCAL_MACHINE.to_string()),
            RunPlacement::Detached { machine_id } => machine_id.clone(),
        }
    }
}

/// Options only a detached run reads. With a [`RunPlacement::Local`] every
/// field must be `None` — see [`check_options`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetachedOptions {
    /// `None` means the project's first repository.
    pub target_repo_id: Option<String>,
    /// `None` and `Some(true)` both send an unattended run; `Some(false)` is
    /// refused.
    pub unattended: Option<bool>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedPlacement {
    pub placement: RunPlacement,
    /// True only when neither a launch override nor the ticket's stored
    /// choice applied, so the placement came from the Discovery default.
    pub inherited: bool,
}

/// The placement a machine id names when it is chosen explicitly — as a
/// ticket's stored choice or a one-launch override.
///
/// Callers turn an id into a placement here rather than comparing it against
/// `"local"` themselves: [`MachineId::is_local`] accepts two spellings, and a
/// caller that tests one of them is a detached run aimed at the desktop.
pub fn placement_for(machine_id: &MachineId) -> RunPlacement {
    if machine_id.is_local() {
        RunPlacement::Local
    } else {
        RunPlacement::Detached {
            machine_id: machine_id.clone(),
        }
    }
}

/// The one-launch override a raw `machine_id` from a ticket-start surface
/// names, or `None` when it names none.
///
/// Every surface that accepts the raw string goes through here. A blank id
/// passed straight to [`MachineId`] would be read by [`MachineId::is_local`],
/// which accepts empty, and silently override a stored detached choice to
/// local — `"local"` is how a caller asks for that.
pub fn placement_override(raw: Option<&str>) -> Option<MachineId> {
    raw.map(str::trim)
        .filter(|id| !id.is_empty())
        .map(MachineId::from)
}

/// The placement a raw `machine_id` from a Feature-launch surface names.
///
/// Every surface that launches a Feature from a raw string goes through here,
/// so a padded id resolves the same everywhere. Unlike [`placement_override`],
/// a blank id is [`RunPlacement::Local`], not "no choice": a Feature launch has
/// no stored placement beneath it to fall back to.
pub fn placement_from_raw(raw: Option<&str>) -> RunPlacement {
    placement_override(raw).map_or(RunPlacement::Local, |id| placement_for(&id))
}

/// A ticket's placement when nothing was chosen for it.
///
/// `project_compute_host` is the project's `remote_host`, and `None` for a
/// project whose compute is local.
pub fn default_ticket_placement(
    discovery_machine: &MachineId,
    project_compute_host: Option<&MachineId>,
) -> RunPlacement {
    if project_compute_host == Some(discovery_machine) {
        return RunPlacement::Local;
    }
    placement_for(discovery_machine)
}

/// The placement one launch of a ticket uses: `override_` beats `stored`,
/// which beats `default`.
///
/// `stored` is the ticket's own column, where `None` means "not chosen" and
/// `"local"` means explicitly local — so a ticket can opt out of a detached
/// default.
pub fn ticket_placement(
    override_: Option<&MachineId>,
    stored: Option<&MachineId>,
    default: RunPlacement,
) -> ResolvedPlacement {
    match override_.or(stored) {
        Some(chosen) => ResolvedPlacement {
            placement: placement_for(chosen),
            inherited: false,
        },
        None => ResolvedPlacement {
            placement: default,
            inherited: true,
        },
    }
}

/// Refuses option combinations the placement would not honour.
pub fn check_options(placement: &RunPlacement, opts: &DetachedOptions) -> Result<(), String> {
    match placement {
        RunPlacement::Local => {
            let set: Vec<&str> = [
                ("target_repo_id", opts.target_repo_id.is_some()),
                ("unattended", opts.unattended.is_some()),
                ("max_cost_usd", opts.max_cost_usd.is_some()),
                ("max_wall_clock_secs", opts.max_wall_clock_secs.is_some()),
            ]
            .into_iter()
            .filter_map(|(name, is_set)| is_set.then_some(name))
            .collect();
            if set.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} only apply to a detached run; choose a remote machine or drop them",
                    set.join(", ")
                ))
            }
        }
        RunPlacement::Detached { .. } => {
            if opts.unattended == Some(false) {
                return Err(
                    "A detached run is always unattended: nobody is attached to answer its gates"
                        .to_string(),
                );
            }
            if let Some(cap) = opts.max_cost_usd {
                if !cap.is_finite() || cap <= 0.0 {
                    return Err(format!(
                        "max_cost_usd must be a finite amount above zero, got {cap}"
                    ));
                }
            }
            if opts.max_wall_clock_secs == Some(0) {
                return Err("max_wall_clock_secs must be above zero".to_string());
            }
            Ok(())
        }
    }
}

/// Why `id` cannot receive a detached run, given its `Machine` row; `None`
/// when it can be probed for a runner.
///
/// Answered before the runner probe: an id with no row would otherwise reach
/// SSH and come back as an unreadable version, which tells the user nothing.
pub fn detached_target_refusal(id: &MachineId, row: Option<&Machine>) -> Option<String> {
    let names_desktop = id.is_local() || row.is_some_and(|m| m.auth_type == "local");
    if names_desktop {
        let shown = if id.is_empty() {
            "(empty)"
        } else {
            id.as_str()
        };
        return Some(format!(
            "Machine `{shown}` is this desktop, not a remote runner. A detached run needs a \
             remote machine from Machines settings; run it locally instead."
        ));
    }
    if row.is_none() {
        return Some(unknown_machine_refusal(id));
    }
    None
}

/// Why `id` cannot be chosen as a placement: no `Machine` row carries it.
///
/// One text for every surface that accepts a machine id — the ticket editor's
/// save and the detached submit — so the user reads the same sentence whether
/// the id was stored or launched.
pub fn unknown_machine_refusal(id: &MachineId) -> String {
    format!(
        "No machine `{id}` is configured. Add it in Machines settings, or choose another \
         machine."
    )
}

#[cfg(test)]
#[path = "../../tests/domain/run_placement.rs"]
mod tests;
