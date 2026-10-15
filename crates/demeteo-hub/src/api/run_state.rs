//! What one Feature's mirrored events say about it now: its status, the state
//! of each Step, how its Tickets stand and which Gates wait on a human.
//!
//! The mirror is the only run state the Hub holds (`docs/HUB.md` §5), so
//! every view of a run is derived here and never stored. The fold is
//! synchronous and free of the store, so the read model is reachable from a
//! test without one (AGENTS.md §3).

use super::port::MirrorEvent;
use demeteo_hub_protocol::{RunEvent, RunEventKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunState {
    pub status: FeatureStatus,
    /// Keyed by `step_id`. A step event that names no Step is not counted.
    pub steps: BTreeMap<String, StepState>,
    pub tickets: TicketProgress,
    /// In the order the Gates began waiting.
    pub pending_gates: Vec<PendingGate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl FeatureStatus {
    pub const fn is_terminal(self) -> bool {
        match self {
            Self::Running => false,
            Self::Completed | Self::Failed | Self::Cancelled => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Running,
    Completed,
    Failed,
}

/// How many Tickets stand where, each counted once by its latest event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TicketProgress {
    pub completed: u32,
    pub failed: u32,
    pub running: u32,
}

/// A Gate whose `gate_pending` has no later `gate_decided`. `since` is when
/// it began waiting, in Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingGate {
    pub feature_id: String,
    pub step_execution_id: String,
    pub step_id: Option<String>,
    pub since: u64,
}

#[derive(Clone, Copy)]
enum TicketState {
    Running,
    Completed,
    Failed,
}

/// `events` are one Feature's, in offset order. A Feature with no
/// feature-level event yet is running: its first events may still be on
/// their way. A terminal Feature has no pending Gate whatever its log says,
/// because no decision on one could reach a run that has ended.
pub fn fold(events: &[RunEvent]) -> RunState {
    let mut status = FeatureStatus::Running;
    let mut steps = BTreeMap::new();
    let mut tickets = BTreeMap::new();
    let mut pending_gates: Vec<PendingGate> = Vec::new();

    for event in events {
        let payload = &event.payload;
        match event.kind {
            RunEventKind::FeatureStarted => status = FeatureStatus::Running,
            RunEventKind::FeatureCompleted => status = FeatureStatus::Completed,
            RunEventKind::FeatureFailed => status = FeatureStatus::Failed,
            RunEventKind::FeatureCancelled => status = FeatureStatus::Cancelled,
            RunEventKind::StepStarted => {
                step(&mut steps, payload.step_id.as_ref(), StepState::Running)
            }
            RunEventKind::StepCompleted => {
                step(&mut steps, payload.step_id.as_ref(), StepState::Completed);
            }
            RunEventKind::StepFailed => {
                step(&mut steps, payload.step_id.as_ref(), StepState::Failed)
            }
            RunEventKind::TicketStarted => {
                ticket(
                    &mut tickets,
                    payload.ticket_id.as_ref(),
                    TicketState::Running,
                );
            }
            RunEventKind::TicketCompleted => {
                ticket(
                    &mut tickets,
                    payload.ticket_id.as_ref(),
                    TicketState::Completed,
                );
            }
            RunEventKind::TicketFailed => {
                ticket(
                    &mut tickets,
                    payload.ticket_id.as_ref(),
                    TicketState::Failed,
                );
            }
            RunEventKind::GatePending => {
                if let Some(execution) = &payload.step_execution_id {
                    decided(&mut pending_gates, &event.feature_id, execution);
                    pending_gates.push(PendingGate {
                        feature_id: event.feature_id.clone(),
                        step_execution_id: execution.clone(),
                        step_id: payload.step_id.clone(),
                        since: event.occurred_at,
                    });
                }
            }
            RunEventKind::GateDecided => {
                if let Some(execution) = &payload.step_execution_id {
                    decided(&mut pending_gates, &event.feature_id, execution);
                }
            }
        }
    }

    if status.is_terminal() {
        pending_gates.clear();
    }
    RunState {
        status,
        steps,
        tickets: progress(tickets.values()),
        pending_gates,
    }
}

/// The readable events of `events`, grouped by Feature with each group in
/// the order it was given. An unreadable row is left out: it says which event
/// it was but not what happened, and one such row must not cost a whole run
/// its view.
pub fn by_feature(events: &[MirrorEvent]) -> BTreeMap<String, Vec<RunEvent>> {
    let mut features: BTreeMap<String, Vec<RunEvent>> = BTreeMap::new();
    for event in events {
        match event {
            MirrorEvent::Readable(event) => features
                .entry(event.feature_id.clone())
                .or_default()
                .push(event.clone()),
            MirrorEvent::Unreadable { .. } => {}
        }
    }
    features
}

fn step(steps: &mut BTreeMap<String, StepState>, step_id: Option<&String>, state: StepState) {
    if let Some(step_id) = step_id {
        steps.insert(step_id.clone(), state);
    }
}

fn ticket(
    tickets: &mut BTreeMap<String, TicketState>,
    ticket_id: Option<&String>,
    state: TicketState,
) {
    if let Some(ticket_id) = ticket_id {
        tickets.insert(ticket_id.clone(), state);
    }
}

fn decided(pending: &mut Vec<PendingGate>, feature_id: &str, step_execution_id: &str) {
    pending.retain(|gate| {
        gate.feature_id != feature_id || gate.step_execution_id != step_execution_id
    });
}

fn progress<'a>(tickets: impl Iterator<Item = &'a TicketState>) -> TicketProgress {
    let mut progress = TicketProgress::default();
    for state in tickets {
        let count = match state {
            TicketState::Running => &mut progress.running,
            TicketState::Completed => &mut progress.completed,
            TicketState::Failed => &mut progress.failed,
        };
        *count = count.saturating_add(1);
    }
    progress
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::RunEventPayload;

    #[derive(Default)]
    struct Ids {
        step: Option<&'static str>,
        execution: Option<&'static str>,
        ticket: Option<&'static str>,
    }

    fn event(feature: &str, offset: u64, kind: RunEventKind, ids: Ids) -> RunEvent {
        RunEvent {
            v: 1,
            feature_id: feature.to_owned(),
            offset,
            kind,
            occurred_at: 1_700_000_000 + offset,
            payload: RunEventPayload {
                step_id: ids.step.map(str::to_owned),
                step_execution_id: ids.execution.map(str::to_owned),
                ticket_id: ids.ticket.map(str::to_owned),
            },
        }
    }

    /// Each kind in turn, offsets counted from 1, all of Feature `f`.
    fn log(kinds: impl IntoIterator<Item = (RunEventKind, Ids)>) -> Vec<RunEvent> {
        kinds
            .into_iter()
            .zip(1..)
            .map(|((kind, ids), offset)| event("f", offset, kind, ids))
            .collect()
    }

    fn step_id(step: &'static str) -> Ids {
        Ids {
            step: Some(step),
            ..Ids::default()
        }
    }

    fn ticket_id(ticket: &'static str) -> Ids {
        Ids {
            ticket: Some(ticket),
            ..Ids::default()
        }
    }

    fn gate(execution: &'static str) -> Ids {
        Ids {
            step: Some("s-gate"),
            execution: Some(execution),
            ..Ids::default()
        }
    }

    fn f1() -> Vec<RunEvent> {
        log([
            (RunEventKind::FeatureStarted, Ids::default()),
            (RunEventKind::StepStarted, step_id("s1")),
            (RunEventKind::TicketStarted, ticket_id("t1")),
            (RunEventKind::TicketCompleted, ticket_id("t1")),
            (RunEventKind::TicketStarted, ticket_id("t2")),
            (RunEventKind::GatePending, gate("e1")),
        ])
    }

    #[test]
    fn a_feature_with_no_feature_event_is_running() {
        assert_eq!(fold(&[]).status, FeatureStatus::Running);
        let only_a_step = log([(RunEventKind::StepStarted, step_id("s1"))]);
        assert_eq!(fold(&only_a_step).status, FeatureStatus::Running);
    }

    #[test]
    fn each_feature_event_sets_the_status_and_the_last_one_wins() {
        for (kind, status) in [
            (RunEventKind::FeatureStarted, FeatureStatus::Running),
            (RunEventKind::FeatureCompleted, FeatureStatus::Completed),
            (RunEventKind::FeatureFailed, FeatureStatus::Failed),
            (RunEventKind::FeatureCancelled, FeatureStatus::Cancelled),
        ] {
            let events = log([
                (RunEventKind::FeatureCancelled, Ids::default()),
                (kind, Ids::default()),
            ]);
            assert_eq!(fold(&events).status, status, "{kind:?}");
        }
    }

    #[test]
    fn each_step_event_sets_its_step_and_the_latest_one_wins() {
        for (kind, state) in [
            (RunEventKind::StepStarted, StepState::Running),
            (RunEventKind::StepCompleted, StepState::Completed),
            (RunEventKind::StepFailed, StepState::Failed),
        ] {
            let events = log([
                (RunEventKind::StepFailed, step_id("s1")),
                (kind, step_id("s1")),
                (RunEventKind::StepStarted, step_id("s2")),
            ]);
            let steps = fold(&events).steps;
            assert_eq!(
                steps,
                BTreeMap::from([
                    ("s1".to_owned(), state),
                    ("s2".to_owned(), StepState::Running)
                ]),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_step_event_that_names_no_step_is_not_counted() {
        let events = log([(RunEventKind::StepStarted, Ids::default())]);
        assert!(fold(&events).steps.is_empty());
    }

    #[test]
    fn each_ticket_event_counts_its_ticket_once_by_its_latest_event() {
        for (kind, expected) in [
            (
                RunEventKind::TicketStarted,
                TicketProgress {
                    running: 1,
                    ..TicketProgress::default()
                },
            ),
            (
                RunEventKind::TicketCompleted,
                TicketProgress {
                    completed: 1,
                    ..TicketProgress::default()
                },
            ),
            (
                RunEventKind::TicketFailed,
                TicketProgress {
                    failed: 1,
                    ..TicketProgress::default()
                },
            ),
        ] {
            let events = log([
                (RunEventKind::TicketStarted, ticket_id("t1")),
                (RunEventKind::TicketFailed, ticket_id("t1")),
                (kind, ticket_id("t1")),
                (RunEventKind::TicketStarted, Ids::default()),
            ]);
            assert_eq!(fold(&events).tickets, expected, "{kind:?}");
        }
    }

    #[test]
    fn a_gate_pending_event_leaves_its_gate_waiting_since_it_occurred() {
        let events = log([(RunEventKind::GatePending, gate("e1"))]);
        assert_eq!(
            fold(&events).pending_gates,
            vec![PendingGate {
                feature_id: "f".to_owned(),
                step_execution_id: "e1".to_owned(),
                step_id: Some("s-gate".to_owned()),
                since: 1_700_000_001,
            }]
        );
    }

    #[test]
    fn a_gate_decided_event_ends_the_wait_of_its_own_gate_alone() {
        let events = log([
            (RunEventKind::GatePending, gate("e1")),
            (RunEventKind::GatePending, gate("e2")),
            (RunEventKind::GateDecided, gate("e1")),
        ]);
        let pending = fold(&events).pending_gates;
        assert_eq!(
            pending
                .iter()
                .map(|gate| gate.step_execution_id.as_str())
                .collect::<Vec<_>>(),
            ["e2"]
        );
    }

    #[test]
    fn a_gate_pending_again_after_its_decision_waits_again() {
        let events = log([
            (RunEventKind::GatePending, gate("e1")),
            (RunEventKind::GateDecided, gate("e1")),
            (RunEventKind::GatePending, gate("e1")),
        ]);
        let pending = fold(&events).pending_gates;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].since, 1_700_000_003);
    }

    #[test]
    fn a_gate_event_that_names_no_execution_is_not_counted() {
        let events = log([(RunEventKind::GatePending, step_id("s-gate"))]);
        assert!(fold(&events).pending_gates.is_empty());
    }

    #[test]
    fn f1_is_running_with_one_ticket_done_one_running_and_gate_e1_waiting() {
        let state = fold(&f1());

        assert_eq!(state.status, FeatureStatus::Running);
        assert_eq!(
            state.steps,
            BTreeMap::from([("s1".to_owned(), StepState::Running)])
        );
        assert_eq!(
            state.tickets,
            TicketProgress {
                completed: 1,
                failed: 0,
                running: 1,
            }
        );
        assert_eq!(
            state.pending_gates,
            vec![PendingGate {
                feature_id: "f".to_owned(),
                step_execution_id: "e1".to_owned(),
                step_id: Some("s-gate".to_owned()),
                since: 1_700_000_006,
            }]
        );
    }

    #[test]
    fn f2_ending_in_feature_completed_is_completed_with_no_gate_waiting() {
        let state = fold(&log([
            (RunEventKind::FeatureStarted, Ids::default()),
            (RunEventKind::StepStarted, step_id("s1")),
            (RunEventKind::StepCompleted, step_id("s1")),
            (RunEventKind::FeatureCompleted, Ids::default()),
        ]));

        assert_eq!(state.status, FeatureStatus::Completed);
        assert!(state.pending_gates.is_empty());
    }

    #[test]
    fn f3_whose_gate_was_decided_has_no_gate_waiting() {
        let state = fold(&log([
            (RunEventKind::FeatureStarted, Ids::default()),
            (RunEventKind::GatePending, gate("e2")),
            (RunEventKind::GateDecided, gate("e2")),
        ]));

        assert_eq!(state.status, FeatureStatus::Running);
        assert!(state.pending_gates.is_empty());
    }

    #[test]
    fn a_terminal_feature_has_no_gate_waiting_though_one_was_never_decided() {
        for terminal in [
            RunEventKind::FeatureCompleted,
            RunEventKind::FeatureFailed,
            RunEventKind::FeatureCancelled,
        ] {
            let mut events = f1();
            events.push(event("f", 7, terminal, Ids::default()));
            assert!(fold(&events).pending_gates.is_empty(), "{terminal:?}");
        }
    }

    #[test]
    fn mirror_rows_are_grouped_by_feature_in_order_without_the_unreadable_ones() {
        let a1 = event("a", 1, RunEventKind::FeatureStarted, Ids::default());
        let b2 = event("b", 2, RunEventKind::FeatureStarted, Ids::default());
        let a3 = event("a", 3, RunEventKind::StepStarted, step_id("s1"));
        let rows = [
            MirrorEvent::Readable(a1.clone()),
            MirrorEvent::Unreadable {
                feature_id: "a".to_owned(),
                offset: 2,
            },
            MirrorEvent::Readable(a3.clone()),
            MirrorEvent::Readable(b2.clone()),
            MirrorEvent::Unreadable {
                feature_id: "c".to_owned(),
                offset: 4,
            },
        ];

        assert_eq!(
            by_feature(&rows),
            BTreeMap::from([("a".to_owned(), vec![a1, a3]), ("b".to_owned(), vec![b2])])
        );
    }

    #[test]
    fn the_states_serialize_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&[FeatureStatus::Running, FeatureStatus::Cancelled]).unwrap(),
            r#"["running","cancelled"]"#
        );
        assert_eq!(
            serde_json::to_string(&StepState::Completed).unwrap(),
            r#""completed""#
        );
    }
}
