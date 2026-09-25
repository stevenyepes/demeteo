// Tests for `crates/demeteo-core/src/domain/sequence/report.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

fn call(id: &str, action: ActionKind, target: &str) -> AgentEvent {
    AgentEvent::ToolCall {
        tool_call_id: id.to_string(),
        intercept_id: format!("ic-{id}"),
        action,
        target: target.to_string(),
        preview: None,
    }
}

fn update(id: &str, status: ToolCallStatus) -> AgentEvent {
    AgentEvent::ToolCallUpdate {
        tool_call_id: id.to_string(),
        status,
        preview: None,
    }
}

fn observed(events: &[AgentEvent]) -> Vec<ObservedCommand> {
    let mut o = CommandObserver::default();
    for e in events {
        o.observe(e);
    }
    o.into_commands()
}

#[test]
fn a_shell_call_takes_the_status_its_harness_reported() {
    let cmds = observed(&[
        call("1", ActionKind::RunBash, "cargo test -p core"),
        update("1", ToolCallStatus::Completed),
        call("2", ActionKind::RunBash, "npm test"),
        update(
            "2",
            ToolCallStatus::Failed {
                reason: "1 failing".into(),
            },
        ),
    ]);
    assert_eq!(
        cmds,
        vec![
            ObservedCommand {
                command: "cargo test -p core".into(),
                status: ObservedStatus::Completed,
            },
            ObservedCommand {
                command: "npm test".into(),
                status: ObservedStatus::Failed {
                    reason: "1 failing".into()
                },
            },
        ]
    );
}

/// The load-bearing rule: a command whose end nobody reported is not a pass.
#[test]
fn a_call_with_no_final_status_is_not_observed_never_completed() {
    let cmds = observed(&[
        call("1", ActionKind::RunBash, "cargo test"),
        update("1", ToolCallStatus::InProgress { message: None }),
    ]);
    assert_eq!(cmds[0].status, ObservedStatus::NotObserved);
}

#[test]
fn reads_and_edits_are_not_commands() {
    let cmds = observed(&[
        call("1", ActionKind::Read, "src/lib.rs"),
        call("2", ActionKind::Edit, "src/lib.rs"),
        update("1", ToolCallStatus::Completed),
    ]);
    assert!(cmds.is_empty(), "{cmds:?}");
}

#[test]
fn an_update_for_an_unknown_call_is_ignored() {
    let cmds = observed(&[update("ghost", ToolCallStatus::Completed)]);
    assert!(cmds.is_empty());
}

#[test]
fn a_long_summary_keeps_its_tail_and_says_so() {
    let text = format!("{}VERDICT LINES", "x".repeat(MAX_SELF_REPORT_CHARS + 50));
    let r = TicketReport::new("t1", "Title", &text, Vec::new(), 1);
    assert!(r.self_report_truncated);
    assert!(r.self_reported.ends_with("VERDICT LINES"));
    assert!(r.self_reported.chars().count() <= MAX_SELF_REPORT_CHARS + 1);
}

#[test]
fn the_command_cap_keeps_the_latest_and_counts_the_rest() {
    let cmds: Vec<ObservedCommand> = (0..MAX_OBSERVED_COMMANDS + 5)
        .map(|i| ObservedCommand {
            command: format!("cmd {i}"),
            status: ObservedStatus::Completed,
        })
        .collect();
    let r = TicketReport::new("t1", "Title", "", cmds, 1);
    assert_eq!(r.observed.len(), MAX_OBSERVED_COMMANDS);
    assert_eq!(r.observed_omitted, 5);
    assert_eq!(r.observed[0].command, "cmd 5");
}

#[test]
fn a_fragment_round_trips_through_its_stored_json() {
    let r = TicketReport::new(
        "t1",
        "Title",
        "done",
        vec![ObservedCommand {
            command: "npm test".into(),
            status: ObservedStatus::Failed {
                reason: "boom".into(),
            },
        }],
        7,
    );
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<TicketReport>(&json).unwrap(), r);
}

#[test]
fn fragment_refs_are_told_apart_from_the_report_and_the_diff() {
    assert!(is_fragment_ref(&format!(
        "/data/artifacts/f/s-implement/{}.json",
        fragment_name("ticket-1")
    )));
    assert!(!is_fragment_ref(
        "/data/artifacts/f/s-implement/implementation-report.md"
    ));
    assert!(!is_fragment_ref(
        "/data/artifacts/f/s-implement/code-diff.diff"
    ));
    assert!(!is_fragment_ref(
        "/data/artifacts/f/s-tickets/task-list.json"
    ));
}

#[test]
fn nothing_to_report_renders_nothing() {
    assert_eq!(render_implementation_report(&[]), None);
}

#[test]
fn the_report_labels_claims_and_observations_apart_in_time_order() {
    let later = TicketReport::new(
        "t2",
        "Second",
        "AC1: MET\nall green",
        vec![ObservedCommand {
            command: "cargo test".into(),
            status: ObservedStatus::NotObserved,
        }],
        20,
    );
    let earlier = TicketReport::new(
        "t1",
        "First",
        "",
        vec![ObservedCommand {
            command: "npm test".into(),
            status: ObservedStatus::Failed {
                reason: "exit 1".into(),
            },
        }],
        10,
    );
    let md = render_implementation_report(&[later, earlier]).unwrap();

    assert!(md.find("`t1`").unwrap() < md.find("`t2`").unwrap(), "{md}");
    assert!(
        md.contains("- observed: `npm test` — failed — exit 1"),
        "{md}"
    );
    assert!(
        md.contains("- observed: `cargo test` — not observed"),
        "{md}"
    );
    assert!(md.contains("> AC1: MET"), "{md}");
    assert!(md.contains("**self-reported**"), "{md}");
    assert!(md.contains("without a summary"), "{md}");
    assert!(
        md.lines()
            .filter(|l| l.starts_with("- observed:"))
            .all(|l| !l.contains("pass")),
        "{md}"
    );
}

#[test]
fn a_ticket_with_no_observed_command_says_it_is_absence_of_evidence() {
    let md = render_implementation_report(&[TicketReport::new("t1", "T", "ran tests", vec![], 1)])
        .unwrap();
    assert!(md.contains("No shell command was observed"), "{md}");
}
