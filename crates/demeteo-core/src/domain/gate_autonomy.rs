//! Which gates a project lets the engine approve without asking anyone.
//!
//! **One setting, three levels, not two switches.** Auto-approving the ship
//! gate while still reviewing every gate before it is a combination nobody
//! wants, and two independent toggles would offer it. The levels are ordered
//! so a floor can be taken with `max`.
//!
//! **Decided in the engine, not per transport.** The gate step consults
//! [`auto_approves`] before it parks, so a local run, a run attached over SSH
//! and a detached runner run behave identically (AGENTS.md §2). The runner
//! does not get its own policy: it raises its copy of the project setting
//! through [`GateAutonomy::for_run`], because a detached run has nobody
//! attached to answer a review gate.
//!
//! **Approval only, never redirect or refusal.** A policy cannot judge the
//! work, so the most it may do is let the run continue. Every approval it
//! grants is recorded as the policy's, not a person's — see
//! [`crate::domain::gate_decision_log`] for why that difference reaches the
//! prompts.
//!
//! See [`crate::domain`] for why this is synchronous and port-free.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateAutonomy {
    /// Every gate waits for a person.
    #[default]
    Attended,
    /// Review gates (`gate_class` unset or `safe`) approve themselves; a
    /// `dangerous` gate — the bundled workflows' ship gate, which is what
    /// publishes the PR — still waits.
    Review,
    /// Every gate approves itself, the ship gate included.
    Full,
}

impl GateAutonomy {
    /// The least an unattended run may run with.
    pub const DETACHED_FLOOR: GateAutonomy = GateAutonomy::Review;

    /// The autonomy a run actually gets: the project's, raised to
    /// [`DETACHED_FLOOR`](Self::DETACHED_FLOOR) when nobody is attached.
    pub fn for_run(self, unattended: bool) -> Self {
        if unattended {
            self.max(Self::DETACHED_FLOOR)
        } else {
            self
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attended => "attended",
            Self::Review => "review",
            Self::Full => "full",
        }
    }

    /// Read the stored column. NULL and anything this build does not know
    /// read as [`Attended`](Self::Attended): an unreadable policy must fall
    /// toward asking, never toward approving.
    pub fn from_column(value: Option<&str>) -> Self {
        match value {
            Some("review") => Self::Review,
            Some("full") => Self::Full,
            _ => Self::Attended,
        }
    }
}

/// Whether a gate of this blast-radius class approves itself under `autonomy`.
pub fn auto_approves(autonomy: GateAutonomy, dangerous: bool) -> bool {
    match autonomy {
        GateAutonomy::Attended => false,
        GateAutonomy::Review => !dangerous,
        GateAutonomy::Full => true,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/gate_autonomy.rs"]
mod gate_autonomy_tests;
