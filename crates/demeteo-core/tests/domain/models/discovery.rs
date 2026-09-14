// Tests extracted from `crates/demeteo-core/src/domain/models/discovery.rs` (mirrored-tests convention). `super` = that module.

use super::*;

use crate::domain::agent_event::{StopReason, ToolCallStatus};
use crate::domain::ids::{DiscoveryId, TicketId};
use crate::domain::ticket_graph::TicketNodeState;

fn ticket(seq: i64, state: TicketState) -> Ticket {
    Ticket {
        id: TicketId::from(format!("t-{seq}")),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        attachments: Vec::new(),
        state,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn an_empty_ticket_list_leaves_the_base_branch_editable() {
    assert!(base_branch_lock_refusal(&[]).is_none());
}

#[test]
fn all_unstarted_tickets_leave_the_base_branch_editable() {
    let tickets = vec![
        ticket(1, TicketState::Unstarted),
        ticket(2, TicketState::Unstarted),
    ];
    assert!(base_branch_lock_refusal(&tickets).is_none());
}

#[test]
fn a_started_ticket_locks_the_base_branch() {
    let tickets = vec![
        ticket(1, TicketState::Unstarted),
        ticket(2, TicketState::Started),
    ];
    let refusal = base_branch_lock_refusal(&tickets).expect("a started ticket locks it");
    assert!(refusal.contains("#2"), "{refusal}");
}

#[test]
fn a_dropped_ticket_locks_the_base_branch() {
    let tickets = vec![
        ticket(1, TicketState::Unstarted),
        ticket(2, TicketState::Dropped),
    ];
    let refusal = base_branch_lock_refusal(&tickets).expect("a dropped ticket locks it");
    assert!(refusal.contains("#2"), "{refusal}");
}

fn call(action: ActionKind, target: &str) -> AgentEvent {
    AgentEvent::ToolCall {
        tool_call_id: format!("t-{target}"),
        intercept_id: "i".to_string(),
        action,
        target: target.to_string(),
        preview: None,
    }
}

#[test]
fn a_turn_that_did_nothing_says_so() {
    let activity = TurnActivity::default();
    assert!(activity.is_empty());
}

#[test]
fn each_action_lands_on_its_own_counter() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::Read, "src/a.rs"));
    activity.observe(&call(ActionKind::Read, "src/b.rs"));
    activity.observe(&call(ActionKind::Edit, "src/a.rs"));
    activity.observe(&call(ActionKind::Write, "src/c.rs"));
    activity.observe(&call(ActionKind::RunBash, "cargo metadata"));

    assert_eq!(activity.reads, 2);
    assert_eq!(activity.edits, 1);
    assert_eq!(activity.writes, 1);
    assert_eq!(activity.ran, 1);
    assert_eq!(activity.commands, vec!["cargo metadata".to_string()]);
    assert!(!activity.is_empty());
}

#[test]
fn nothing_but_a_tool_call_is_counted() {
    let mut activity = TurnActivity::default();
    activity.observe(&AgentEvent::Text {
        delta: "reading".to_string(),
    });
    activity.observe(&AgentEvent::ToolCallUpdate {
        tool_call_id: "t".to_string(),
        status: ToolCallStatus::Completed,
        preview: None,
    });
    activity.observe(&AgentEvent::TurnComplete {
        stop_reason: StopReason::EndOfTurn,
        usage: None,
    });
    assert!(activity.is_empty());
}

#[test]
fn the_same_command_twice_is_named_once_and_counted_twice() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::RunBash, "rg discovery"));
    activity.observe(&call(ActionKind::RunBash, "rg discovery"));
    assert_eq!(activity.ran, 2);
    assert_eq!(activity.commands.len(), 1);
}

#[test]
fn the_command_sample_is_bounded_but_the_count_is_not() {
    let mut activity = TurnActivity::default();
    for i in 0..(TurnActivity::MAX_COMMANDS + 4) {
        activity.observe(&call(ActionKind::RunBash, &format!("cmd-{i}")));
    }
    assert_eq!(activity.commands.len(), TurnActivity::MAX_COMMANDS);
    assert_eq!(activity.ran as usize, TurnActivity::MAX_COMMANDS + 4);
}

#[test]
fn a_script_shaped_command_is_kept_as_its_first_line_and_capped() {
    let mut activity = TurnActivity::default();
    let script = format!("{}\nrm -rf /\n", "e".repeat(400));
    activity.observe(&call(ActionKind::RunBash, &script));

    let kept = &activity.commands[0];
    assert_eq!(kept.chars().count(), TurnActivity::MAX_COMMAND_CHARS);
    assert!(!kept.contains('\n'));
    assert!(!kept.contains("rm -rf"));
}

#[test]
fn a_blank_command_is_not_remembered() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::RunBash, "   "));
    assert_eq!(activity.ran, 1);
    assert!(activity.commands.is_empty());
}

/// The surface reads these keys by name; renaming one silently empties the
/// meta line rather than failing anything.
#[test]
fn the_wire_shape_is_the_one_the_surface_reads() {
    let mut activity = TurnActivity::default();
    activity.observe(&call(ActionKind::Read, "src/a.rs"));
    activity.observe(&call(ActionKind::RunBash, "git log --oneline"));

    let json = serde_json::to_value(&activity).expect("serializable");
    assert_eq!(json["reads"], 1);
    assert_eq!(json["edits"], 0);
    assert_eq!(json["writes"], 0);
    assert_eq!(json["ran"], 1);
    assert_eq!(json["commands"][0], "git log --oneline");
}

/// A row written before V49 carries no activity, and reads back as absent
/// rather than as a turn that did nothing.
#[test]
fn a_message_stored_without_activity_reads_as_none() {
    let stored = serde_json::json!({
        "id": "m-1",
        "discovery_id": "d-1",
        "role": "assistant",
        "content": "hello",
        "created_at": 1,
    });
    let message: DiscoveryMessage = serde_json::from_value(stored).expect("deserializable");
    assert!(message.activity.is_none());
}

#[test]
fn a_name_is_trimmed_before_it_is_measured() {
    let at_cap = "n".repeat(TITLE_MAX_CHARS);
    let padded = format!("  {at_cap}\n");
    assert_eq!(validate_title(&padded).unwrap(), at_cap);
}

#[test]
fn a_name_one_character_past_the_cap_is_refused() {
    assert!(validate_title(&"n".repeat(TITLE_MAX_CHARS)).is_ok());
    let refusal = validate_title(&"n".repeat(TITLE_MAX_CHARS + 1))
        .expect_err("one character past the cap is past the cap");
    assert!(
        refusal.contains(&(TITLE_MAX_CHARS + 1).to_string()),
        "{refusal}"
    );
}

/// The cap counts what the list has to show, not how the name is encoded — a
/// byte count refuses a shorter name for being written in another script.
#[test]
fn a_name_is_measured_in_characters_not_bytes() {
    let name = "決定".repeat(TITLE_MAX_CHARS / 2);
    assert!(name.len() > TITLE_MAX_CHARS);
    assert!(validate_title(&name).is_ok());
}

#[test]
fn a_name_of_only_whitespace_is_no_name() {
    assert!(validate_title("   \n ").is_err());
}

#[test]
fn a_new_branch_name_starting_with_a_dash_is_refused() {
    assert!(validate_new_branch_name("-oops").is_err());
}

#[test]
fn a_new_branch_name_with_embedded_whitespace_is_refused() {
    assert!(validate_new_branch_name("feature x").is_err());
}

#[test]
fn a_new_branch_name_that_is_only_whitespace_is_refused() {
    assert!(validate_new_branch_name("   ").is_err());
}

#[test]
fn a_new_branch_name_with_embedded_dotdot_is_refused() {
    assert!(validate_new_branch_name("feature..x").is_err());
}

#[test]
fn a_normal_new_branch_name_is_accepted() {
    assert!(validate_new_branch_name("release/2.1").is_ok());
}

fn bare_discovery(base_branch: Option<String>) -> Discovery {
    Discovery {
        id: DiscoveryId::from("d-1".to_string()),
        project_id: crate::domain::ids::ProjectId::from("p-1".to_string()),
        title: "rework the sync surface".to_string(),
        status: DiscoveryStatus::Open,
        machine_id: crate::domain::ids::MachineId::from("local".to_string()),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        resume_session_id: None,
        worktree_path: None,
        base_branch,
        integration_mr_url: None,
        integration_mr_state: None,
        attachments: Vec::new(),
        total_cost: 0.0,
        tokens: 0,
        created_at: 0,
        updated_at: 0,
    }
}

fn node(id: &str, mr_state: Option<&str>) -> TicketNode {
    TicketNode {
        id: id.to_string(),
        state: TicketNodeState::Unstarted,
        blocked_by: Vec::new(),
        mr_state: mr_state.map(str::to_string),
        force_started: false,
    }
}

#[test]
fn a_discovery_with_a_base_branch_has_something_to_publish_from() {
    assert!(missing_base_branch_refusal(&bare_discovery(Some("develop".to_string()))).is_none());
}

#[test]
fn a_discovery_with_no_base_branch_has_nothing_to_publish_from() {
    assert!(missing_base_branch_refusal(&bare_discovery(None)).is_some());
}

#[test]
fn an_empty_node_list_has_no_merged_ticket() {
    assert!(no_merged_ticket_refusal(&[]).is_some());
}

#[test]
fn no_open_or_closed_or_unattempted_node_counts_as_merged() {
    let nodes = vec![
        node("t-1", None),
        node("t-2", Some("open")),
        node("t-3", Some("closed")),
    ];
    assert!(no_merged_ticket_refusal(&nodes).is_some());
}

#[test]
fn one_merged_node_is_enough() {
    let nodes = vec![node("t-1", Some("open")), node("t-2", Some("merged"))];
    assert!(no_merged_ticket_refusal(&nodes).is_none());
}

#[test]
fn the_pr_body_names_every_ticket_and_leaks_nothing_else() {
    let mut secret = ticket(1, TicketState::Unstarted);
    secret.description = "top-secret-rationale".to_string();
    secret.acceptance = vec!["top-secret-acceptance".to_string()];
    secret.files = vec!["top/secret/file.rs".to_string()];

    let mut merged = ticket(2, TicketState::Unstarted);
    merged.id = TicketId::from("t-2".to_string());

    let never_started = ticket(3, TicketState::Unstarted);

    let tickets = vec![secret.clone(), merged.clone(), never_started.clone()];
    let nodes = vec![node(merged.id.as_str(), Some("merged"))];

    let body = integration_pr_body(&tickets, &nodes);

    assert!(body.contains(&secret.title));
    assert!(body.contains(&merged.title));
    assert!(body.contains(&never_started.title));
    assert!(!body.contains("top-secret-rationale"));
    assert!(!body.contains("top-secret-acceptance"));
    assert!(!body.contains("top/secret/file.rs"));
}
