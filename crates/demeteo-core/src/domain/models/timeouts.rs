//! Agent turn timeout configuration.
//!
//! Three-tier timeout strategy (see [`crate::adapters::agent::event_stream::turn`]
//! for the runtime that consumes these values):
//!
//! - `fast_timeout_s`: when no event (stdout **and** stderr both silent) has
//!   arrived for this many seconds after at least one event was seen, the
//!   turn is aborted with `"Agent blocked: no output for Ns (stdout and stderr
//!   both silent)"`. This is the message the user sees when an agent hangs.
//!   Defaults to **300s (5 min)** — generous enough for complex coding tasks
//!   that occasionally go quiet while the model thinks.
//!
//! - `normal_timeout_s`: when no event has ever arrived for this many seconds
//!   after the turn started, the turn is aborted with `"Agent response timed
//!   out (no output for Ns)"`. This is the secondary guard for agents that
//!   never produce their first event at all. Defaults to **600s (10 min)**.
//!
//! - `wall_cap_s`: absolute upper bound regardless of activity. Defaults to
//!   **1800s (30 min)**. After this cap the turn is unconditionally killed.
//!
//! Values are stored as a JSON-encoded [`AgentTimeouts`] blob under the
//! `app_settings` key [`CONFIG_KEY`] via
//! [`crate::application::timeouts`]. All call sites read the effective values
//! through [`crate::application::timeouts::resolve_effective`].

use serde::{Deserialize, Serialize};

/// `app_settings` key holding the JSON-encoded [`AgentTimeouts`].
pub const CONFIG_KEY: &str = "agent_timeouts";

/// User-configurable timeout knobs applied uniformly to every agent-turn
/// call site (planner, worker, resolver, verifier, agent step). Round-trips
/// through JSON via `app_settings`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTimeouts {
    /// "Agent blocked" threshold (stdout AND stderr silent). Default 300.
    #[serde(default = "default_fast")]
    pub fast_timeout_s: u64,
    /// "Agent response timed out" threshold (no event ever). Default 600.
    #[serde(default = "default_normal")]
    pub normal_timeout_s: u64,
    /// Absolute wall-clock cap per turn. Default 1800.
    #[serde(default = "default_wall")]
    pub wall_cap_s: u64,
}

fn default_fast() -> u64 {
    300
}
fn default_normal() -> u64 {
    600
}
fn default_wall() -> u64 {
    1800
}

impl Default for AgentTimeouts {
    fn default() -> Self {
        Self {
            fast_timeout_s: default_fast(),
            normal_timeout_s: default_normal(),
            wall_cap_s: default_wall(),
        }
    }
}

impl AgentTimeouts {
    /// Build a [`AgentTimeouts`] from raw user input, clamping each value
    /// into a sane range. Returns `Err` with a human-readable reason when
    /// any value is out of range.
    ///
    /// Rules:
    /// * `fast_timeout_s` ∈ [10, 3600] (10s – 1h)
    /// * `normal_timeout_s` ∈ [fast, 7200] (≥ fast, ≤ 2h)
    /// * `wall_cap_s` ∈ [normal, 14400] (≥ normal, ≤ 4h)
    pub fn validated(fast: u64, normal: u64, wall: u64) -> Result<Self, String> {
        if !(10..=3600).contains(&fast) {
            return Err(format!(
                "fast_timeout_s must be between 10 and 3600 seconds, got {}",
                fast
            ));
        }
        if !(10..=7200).contains(&normal) {
            return Err(format!(
                "normal_timeout_s must be between 10 and 7200 seconds, got {}",
                normal
            ));
        }
        if !(10..=14400).contains(&wall) {
            return Err(format!(
                "wall_cap_s must be between 10 and 14400 seconds, got {}",
                wall
            ));
        }
        if normal < fast {
            return Err(format!(
                "normal_timeout_s ({}) must be ≥ fast_timeout_s ({})",
                normal, fast
            ));
        }
        if wall < normal {
            return Err(format!(
                "wall_cap_s ({}) must be ≥ normal_timeout_s ({})",
                wall, normal
            ));
        }
        Ok(Self {
            fast_timeout_s: fast,
            normal_timeout_s: normal,
            wall_cap_s: wall,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_documented_thresholds() {
        let t = AgentTimeouts::default();
        assert_eq!(t.fast_timeout_s, 300);
        assert_eq!(t.normal_timeout_s, 600);
        assert_eq!(t.wall_cap_s, 1800);
    }

    #[test]
    fn json_round_trip() {
        let original = AgentTimeouts {
            fast_timeout_s: 120,
            normal_timeout_s: 240,
            wall_cap_s: 900,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: AgentTimeouts = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let parsed: AgentTimeouts = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, AgentTimeouts::default());
    }

    #[test]
    fn validated_rejects_out_of_range_fast() {
        assert!(AgentTimeouts::validated(5, 600, 1800).is_err());
        assert!(AgentTimeouts::validated(4000, 600, 1800).is_err());
    }

    #[test]
    fn validated_rejects_normal_less_than_fast() {
        assert!(AgentTimeouts::validated(300, 100, 1800).is_err());
    }

    #[test]
    fn validated_rejects_wall_less_than_normal() {
        assert!(AgentTimeouts::validated(100, 200, 100).is_err());
    }

    #[test]
    fn validated_accepts_monotonic_values() {
        assert!(AgentTimeouts::validated(300, 600, 1800).is_ok());
        assert!(AgentTimeouts::validated(10, 10, 10).is_ok());
    }
}
