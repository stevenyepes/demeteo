//! What an instance reports of the harnesses installed on its own machine and
//! the models each one lists (docs/HUB.md §5).
//!
//! The probe runs on the instance and only there: a Hub, or a desktop
//! looking at a runner, would read the wrong machine and need that machine's
//! credentials. This module decides when a probe is due, how a finished
//! probe folds into the cache, and what of the cache crosses the socket.

use std::collections::HashSet;

use demeteo_hub_protocol::{Effort, HarnessCatalogEntry, ModelChoice};

use crate::domain::models::{AgentKind, ConfigOptionValue, EffortLevel, ModelSource};
use crate::shared::secret_scrub::scrub_secrets;

/// How often a connected instance re-probes.
pub const HARNESS_PROBE_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// A (re)connect probes only when the last probe finished at least this long
/// ago, so a socket that flaps does not run every harness's CLI each time.
pub const HARNESS_PROBE_CONNECT_FLOOR_SECS: u64 = 15 * 60;

/// How long one harness's model listing may take before it counts as
/// [`HarnessProbeOutcome::Unreachable`].
pub const HARNESS_PROBE_TIMEOUT_SECS: u64 = 60;

/// How many models one catalog entry carries. opencode lists every model of
/// every provider it knows; past this a picker is unusable anyway, and the
/// cap bounds what the catalog adds to a snapshot.
pub const MAX_CATALOG_MODELS: usize = 500;

/// A model's value and name are each cut to this many characters.
pub const MAX_CATALOG_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeTrigger {
    Connected,
    Timer,
}

/// Whether a probe should start now. `None` (never probed) is always due,
/// and so is a clock that has moved behind the last probe.
///
/// The timer is due an interval *less the floor* after the last probe, not a
/// whole interval: its ticks are counted from the session's start, and the
/// probe that start triggered finished after it, so comparing against the
/// whole interval would skip every other tick.
pub fn probe_due(last_finished_ms: Option<i64>, now_ms: i64, trigger: ProbeTrigger) -> bool {
    let Some(last) = last_finished_ms else {
        return true;
    };
    let Some(elapsed) = now_ms.checked_sub(last).filter(|e| *e >= 0) else {
        return true;
    };
    let floor = secs_to_ms(HARNESS_PROBE_CONNECT_FLOOR_SECS);
    let wait = match trigger {
        ProbeTrigger::Connected => floor,
        ProbeTrigger::Timer => secs_to_ms(HARNESS_PROBE_INTERVAL_SECS) - floor,
    };
    elapsed >= wait
}

fn secs_to_ms(secs: u64) -> i64 {
    i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX)
}

/// What asking one harness for its models answered.
#[derive(Debug, Clone)]
pub enum HarnessProbeOutcome {
    Listed {
        models: Vec<ConfigOptionValue>,
        source: ModelSource,
    },
    /// The binary is not on the machine.
    NotInstalled,
    /// No answer: a transport error or a timeout. Distinct from
    /// [`NotInstalled`](Self::NotInstalled) for the reason
    /// [`Availability::Unknown`](crate::domain::models::Availability::Unknown)
    /// is.
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct HarnessProbe {
    pub kind: String,
    pub outcome: HarnessProbeOutcome,
    pub finished_ms: i64,
}

/// One cached harness, as the instance holds it. Unlike what crosses, it
/// keeps each model's description and image flag for local use.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub kind: String,
    pub models: Vec<ConfigOptionValue>,
    pub effort_levels: Vec<EffortLevel>,
    /// `None` for a harness that has never answered a listing: it is held
    /// with no models rather than left out, and its models are unknown.
    pub source: Option<ModelSource>,
    pub probed_at_ms: i64,
}

/// `previous` with `fresh` folded in. A listed harness replaces its entry, an
/// uninstalled one loses it, and one that did not answer keeps what it last
/// listed, or, having never listed, gets an entry with no models and no
/// source: a host that is briefly unreachable must not read as having nothing
/// installed. Kinds `fresh` does not mention are kept as they were.
pub fn merged(
    previous: &[CatalogEntry],
    fresh: Vec<HarnessProbe>,
    effort_levels: impl Fn(&str) -> Vec<EffortLevel>,
) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = previous.to_vec();
    for probe in fresh {
        let at = out.iter().position(|e| e.kind == probe.kind);
        match probe.outcome {
            HarnessProbeOutcome::Listed { models, source } => {
                let entry = CatalogEntry {
                    effort_levels: effort_levels(&probe.kind),
                    kind: probe.kind,
                    models,
                    source: Some(source),
                    probed_at_ms: probe.finished_ms,
                };
                match at {
                    Some(i) => out[i] = entry,
                    None => out.push(entry),
                }
            }
            HarnessProbeOutcome::NotInstalled => {
                if let Some(i) = at {
                    out.remove(i);
                }
            }
            HarnessProbeOutcome::Unreachable if at.is_none() => out.push(CatalogEntry {
                effort_levels: effort_levels(&probe.kind),
                kind: probe.kind,
                models: Vec::new(),
                source: None,
                probed_at_ms: probe.finished_ms,
            }),
            HarnessProbeOutcome::Unreachable => {}
        }
    }
    out
}

/// The catalog as it crosses the socket. Like the rest of the snapshot it is
/// copied field by field: a model's `description` and `supports_images` are
/// never read, because codex's description names the config file it came
/// from. Only supported kinds cross, sorted by kind so equal caches give
/// equal snapshots; within one, the harness's own order is kept, and a value
/// listed twice keeps its first name.
pub fn outbound_catalog(entries: &[CatalogEntry]) -> Vec<HarnessCatalogEntry> {
    let mut kinds = HashSet::new();
    let mut out: Vec<HarnessCatalogEntry> = entries
        .iter()
        .filter(|e| AgentKind::is_supported(&e.kind) && kinds.insert(e.kind.as_str()))
        .map(outbound_entry)
        .collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind));
    out
}

fn outbound_entry(entry: &CatalogEntry) -> HarnessCatalogEntry {
    let mut values = HashSet::new();
    let models = entry
        .models
        .iter()
        .filter(|m| !m.value.is_empty() && !m.value.contains('\0') && !m.name.contains('\0'))
        .map(|m| ModelChoice {
            value: outbound_text(&m.value),
            name: outbound_text(&m.name),
        })
        .filter(|m| values.insert(m.value.clone()))
        .take(MAX_CATALOG_MODELS)
        .collect();
    HarnessCatalogEntry {
        kind: entry.kind.clone(),
        models,
        effort_levels: Some(entry.effort_levels.iter().copied().map(effort).collect()),
        source: entry.source.map(|source| source.as_str().to_owned()),
        probed_at: entry.probed_at_ms.div_euclid(1000),
    }
}

fn outbound_text(text: &str) -> String {
    scrub_secrets(text)
        .chars()
        .take(MAX_CATALOG_CHARS)
        .collect()
}

/// Exhaustive rather than a serde round-trip: the protocol crate mirrors
/// core's spellings without sharing them, and a level added to
/// `EffortLevel` must fail to compile here instead of failing to parse at
/// runtime and silently dropping.
pub(super) fn effort(level: EffortLevel) -> Effort {
    match level {
        EffortLevel::Low => Effort::Low,
        EffortLevel::Medium => Effort::Medium,
        EffortLevel::High => Effort::High,
        EffortLevel::XHigh => Effort::Xhigh,
        EffortLevel::Max => Effort::Max,
    }
}
