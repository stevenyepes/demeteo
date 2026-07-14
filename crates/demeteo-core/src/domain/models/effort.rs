use serde::{Deserialize, Serialize};

use super::agent_config::AgentKind;

/// Canonical Demeteo effort ladder — the reasoning budget a step asks its
/// agent for, a peer of the model rather than a property of it.
///
/// The wire/DB/argv form is the lowercase string (`low`, `medium`, `high`,
/// `xhigh`, `max`); `#[serde(rename_all = "lowercase")]` renders `XHigh` as
/// `"xhigh"`, and that spelling is the single canonical one everywhere —
/// SQLite, `steps_json`, Tauri IPC, `RunSpec`, and the TS mirror.
///
/// `Ord` is the ladder order (`Low < Medium < High < XHigh < Max`);
/// [`EffortLevel::clamp_for`] depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl EffortLevel {
    /// Every level, in ladder order — mirrors [`AgentKind::ALL`].
    pub const ALL: [EffortLevel; 5] = [
        EffortLevel::Low,
        EffortLevel::Medium,
        EffortLevel::High,
        EffortLevel::XHigh,
        EffortLevel::Max,
    ];

    /// The terminal fallback of the resolution chain: the one place the
    /// "default effort is high" decision lives.
    pub const DEFAULT: EffortLevel = EffortLevel::High;

    /// Effort for the verifier turn when its `VerifierConfig` does not pin
    /// one. Interpreting harness output into a verdict is a small-model job
    /// (see `VerifierConfig::model`), and it runs on *every* retry — letting
    /// it inherit a blanket `high` would multiply the cost of every loop.
    pub const VERIFIER_DEFAULT: EffortLevel = EffortLevel::Low;

    /// Effort for the environment/regression triage turn — a classification,
    /// not reasoning work.
    pub const TRIAGE: EffortLevel = EffortLevel::Low;

    /// Effort for the finalize turn (write the PR title/body).
    pub const FINALIZE: EffortLevel = EffortLevel::Medium;

    /// The stable lowercase identifier used on the wire, in the DB, and in
    /// agent argv/env.
    pub fn as_str(self) -> &'static str {
        match self {
            EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High => "high",
            EffortLevel::XHigh => "xhigh",
            EffortLevel::Max => "max",
        }
    }

    /// Parse a stored/wire effort string. Returns `None` for unknown values
    /// so a stale DB row or a spec from a newer client degrades to "inherit"
    /// rather than panicking — mirrors [`AgentKind::parse`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(EffortLevel::Low),
            "medium" => Some(EffortLevel::Medium),
            "high" => Some(EffortLevel::High),
            "xhigh" => Some(EffortLevel::XHigh),
            "max" => Some(EffortLevel::Max),
            _ => None,
        }
    }

    /// The levels an agent actually accepts per invocation.
    ///
    /// - `ClaudeCode`: `--effort` takes all five; the CLI clamps per-model itself.
    /// - `Codex`: `max` exists only on some `gpt-5.6-*` models, so it is not
    ///   declared — `Max` clamps down to `XHigh`.
    /// - `Opencode`: `--variant` is a per-model pass-through; the canonical
    ///   ladder goes through unchanged and opencode ignores a level its model
    ///   doesn't offer.
    /// - `Hermes`: empty — effort lives only in `~/.hermes/config.yaml`, and
    ///   there is no per-invocation control to drive. Honest degradation.
    pub fn supported_for(kind: AgentKind) -> &'static [EffortLevel] {
        const ALL: &[EffortLevel] = &[
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
            EffortLevel::Max,
        ];
        const NO_MAX: &[EffortLevel] = &[
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
        ];
        match kind {
            AgentKind::ClaudeCode => ALL,
            AgentKind::Codex => NO_MAX,
            AgentKind::Opencode => ALL,
            AgentKind::Hermes => &[],
        }
    }

    /// Project `level` onto what `kind` actually supports.
    ///
    /// `None` when the agent declares no levels at all (inject nothing).
    /// Otherwise the level itself if supported, else the highest supported
    /// level strictly below it, else the lowest supported level. Total by
    /// construction: the result is always `None` or a member of
    /// [`supported_for`](Self::supported_for)`(kind)`, so an adapter can never
    /// emit a level its agent would reject (codex) or silently ignore
    /// (opencode).
    pub fn clamp_for(kind: AgentKind, level: EffortLevel) -> Option<EffortLevel> {
        let supported = Self::supported_for(kind);
        if supported.is_empty() {
            return None;
        }
        if supported.contains(&level) {
            return Some(level);
        }
        supported
            .iter()
            .copied()
            .filter(|l| *l < level)
            .max()
            .or_else(|| supported.iter().copied().min())
    }
}

impl std::fmt::Display for EffortLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/models/effort.rs"]
mod tests;
