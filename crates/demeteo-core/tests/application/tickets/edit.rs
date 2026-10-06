// Tests extracted from `crates/demeteo-core/src/application/tickets/edit.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId};
use crate::domain::models::{Machine, Project, TicketState};
use crate::domain::run_placement::detached_target_refusal;

fn row(seq: i64, blocked_by: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::from(format!("t-{seq}")),
        discovery_id: DiscoveryId::from("d-1"),
        seq,
        title: format!("ticket {seq}"),
        description: "why".to_string(),
        acceptance: vec!["it works".to_string()],
        files: Vec::new(),
        blocked_by: blocked_by.iter().map(|id| TicketId::from(*id)).collect(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn edit(blocked_by: &[&str]) -> TicketEdit {
    TicketEdit {
        title: "a title".to_string(),
        description: "a description".to_string(),
        acceptance: vec!["it works".to_string()],
        files: Vec::new(),
        blocked_by: blocked_by.iter().map(|id| (*id).to_string()).collect(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: None,
    }
    .normalized()
}

/// §5.4: the lock is the Feature, and nothing else about the row.
#[test]
fn a_started_ticket_refuses_the_edit_and_names_its_number() {
    let mut started = row(2, &[]);
    started.state = TicketState::Started;
    started.feature_id = Some(FeatureId::from("f-1"));
    let refusal = refusal(&started, &[row(1, &[]), started.clone()], &edit(&[]))
        .expect("a started ticket is locked");
    assert!(refusal.contains("#2"), "{refusal}");
}

/// §5.3 draws immutability at *has a Feature*, and a dropped ticket has none —
/// `diff_proposal` lets a re-decomposition revise one on the same reading.
#[test]
fn a_dropped_ticket_is_still_editable() {
    let mut dropped = row(1, &[]);
    dropped.state = TicketState::Dropped;
    dropped.drop_reason = Some("the plan moved on".to_string());
    assert_eq!(refusal(&dropped, &[dropped.clone()], &edit(&[])), None);
}

/// The hazard a per-edge check cannot see: neither ticket is on a cycle until
/// this edge exists, so the whole resulting set is what gets validated.
#[test]
fn an_edge_that_closes_a_cycle_is_refused_and_names_both_by_number() {
    let one = row(1, &[]);
    let two = row(2, &["t-1"]);
    let refusal = refusal(&one, &[one.clone(), two], &edit(&["t-2"]))
        .expect("1 waiting on 2 waiting on 1 is a cycle");
    assert!(refusal.contains("cycle"), "{refusal}");
    assert!(
        refusal.contains("#1") && refusal.contains("#2"),
        "{refusal}"
    );
}

/// §6.2 closes the graph over one Discovery. The stranger keeps its stored id
/// in the message, having no number in this plan to be named by.
#[test]
fn an_edge_out_of_the_discovery_is_refused() {
    let one = row(1, &[]);
    let refusal = refusal(&one, std::slice::from_ref(&one), &edit(&["t-99"]))
        .expect("an edge outside the discovery is not an edge");
    assert!(refusal.contains("t-99"), "{refusal}");
}

#[test]
fn a_ticket_may_not_wait_on_itself() {
    let one = row(1, &[]);
    assert!(refusal(&one, std::slice::from_ref(&one), &edit(&["t-1"])).is_some());
}

/// The title is the run's name and the plan's only handle on the ticket.
#[test]
fn an_emptied_title_is_refused() {
    let one = row(1, &[]);
    let mut blanked = edit(&[]);
    blanked.title = "   ".to_string();
    assert!(refusal(&one, std::slice::from_ref(&one), &blanked).is_some());
}

/// What a form leaves behind: half-typed rows, and selects the user set back
/// to the inherit option. An empty string in a nullable column would read as a
/// choice everywhere downstream.
#[test]
fn blank_rows_and_blank_selects_do_not_reach_the_row() {
    let normalized = TicketEdit {
        title: "  a title  ".to_string(),
        description: "a description".to_string(),
        acceptance: vec!["it works".to_string(), "   ".to_string()],
        files: vec!["  src/a.rs".to_string(), String::new()],
        blocked_by: vec!["t-2".to_string(), " t-2 ".to_string(), String::new()],
        test_command: Some("  ".to_string()),
        workflow_id: Some(String::new()),
        agent_kind: Some(" claude-code ".to_string()),
        model: None,
        effort: None,
        machine_id: Some(" \t ".to_string()),
    }
    .normalized();

    assert_eq!(normalized.machine_id, None);
    assert_eq!(normalized.title, "a title");
    assert_eq!(normalized.acceptance, vec!["it works".to_string()]);
    assert_eq!(normalized.files, vec!["src/a.rs".to_string()]);
    assert_eq!(normalized.blocked_by, vec!["t-2".to_string()]);
    assert_eq!(normalized.test_command, None);
    assert_eq!(normalized.workflow_id, None);
    assert_eq!(normalized.agent_kind.as_deref(), Some("claude-code"));
}

/// Clearing a column is `Some(None)`, never the absence [`TicketPatch`]
/// reserves for *leave alone* — the whole reason the wire carries the whole
/// form.
#[test]
fn clearing_a_field_writes_a_null_rather_than_leaving_it_alone() {
    let patch = edit(&[]).patch();
    assert!(matches!(patch.model, Some(None)));
    assert!(matches!(patch.workflow_id, Some(None)));
    assert!(matches!(patch.effort, Some(None)));
    assert!(matches!(patch.machine_id, Some(None)));
    assert!(patch.state.is_none());
    assert!(patch.drop_reason.is_none());
    assert!(patch.feature_id.is_none());
}

/// A stored ticket in a fresh local project, which is as much as [`update`]
/// reads — plus the one remote machine the placement tests choose.
fn stored_ticket(tag: &str) -> (AppContext, TicketId) {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-ticket-edit-{tag}"));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "name fixture".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    ctx.machines
        .add(Machine {
            id: MachineId::from("gpu-box".to_string()),
            name: "gpu-box".to_string(),
            host: "example.internal".to_string(),
            port: 22,
            username: "demeteo".to_string(),
            auth_type: "key".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        })
        .expect("the machine is stored");
    let discovery = open_discovery(
        &ctx,
        NewDiscovery {
            project_id: project_id.as_str().to_string(),
            title: "placement".to_string(),
            agent_kind: "claude-code".to_string(),
            model: None,
            effort: None,
            machine_id: None,
            staged_attachments: Vec::new(),
        },
    )
    .expect("the discovery opens");
    let mut t = row(1, &[]);
    t.discovery_id = discovery.id;
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    (ctx, t.id)
}

fn placed(machine_id: &str) -> TicketEdit {
    TicketEdit {
        machine_id: Some(machine_id.to_string()),
        ..edit(&[])
    }
}

fn stored_machine(ctx: &AppContext, id: &TicketId) -> Option<MachineId> {
    ctx.tickets
        .get(id)
        .expect("the ticket reads back")
        .expect("the ticket exists")
        .machine_id
}

#[tokio::test]
async fn saving_a_chosen_machine_persists_it() {
    let (ctx, id) = stored_ticket("chosen");
    update(&ctx, &id, &placed("gpu-box")).expect("a configured machine saves");
    assert_eq!(
        stored_machine(&ctx, &id),
        Some(MachineId::from("gpu-box".to_string()))
    );
}

/// `"local"` is a choice, not a blank: it is how a ticket opts out of a
/// detached default, so it must not normalise to the inherit `None`.
#[tokio::test]
async fn local_is_saved_as_local() {
    let (ctx, id) = stored_ticket("local");
    update(&ctx, &id, &placed("local")).expect("the desktop needs no machine row");
    assert_eq!(
        stored_machine(&ctx, &id),
        Some(MachineId::from("local".to_string()))
    );
}

#[tokio::test]
async fn a_blank_placement_clears_a_stored_one() {
    let (ctx, id) = stored_ticket("blank");
    update(&ctx, &id, &placed("gpu-box")).expect("a configured machine saves");
    update(&ctx, &id, &placed("   ")).expect("a blank placement saves");
    assert_eq!(stored_machine(&ctx, &id), None);
}

#[tokio::test]
async fn an_unknown_machine_is_refused_by_name_and_nothing_is_saved() {
    let (ctx, id) = stored_ticket("unknown");
    let mut renamed = placed("nowhere");
    renamed.title = "a renamed title".to_string();
    let refusal = update(&ctx, &id, &renamed).expect_err("no row carries that id");
    assert!(refusal.contains("nowhere"), "{refusal}");

    let after = ctx
        .tickets
        .get(&id)
        .expect("the ticket reads back")
        .expect("the ticket exists");
    assert_eq!(after.machine_id, None);
    assert_eq!(after.title, "ticket 1");
}

/// A desktop row has an id like any other, so a lookup alone would store it
/// as a detached choice that only a launch refuses.
#[tokio::test]
async fn a_desktop_machine_is_refused_at_save_and_nothing_is_saved() {
    let (ctx, id) = stored_ticket("desktop");
    let desktop = Machine {
        id: MachineId::from("this-desktop".to_string()),
        name: "this-desktop".to_string(),
        auth_type: "local".to_string(),
        ..ctx
            .machines
            .get_machine(&MachineId::from("gpu-box".to_string()))
            .expect("the machine reads back")
            .expect("the machine exists")
    };
    ctx.machines
        .add(desktop.clone())
        .expect("the desktop is stored");
    let mut renamed = placed("this-desktop");
    renamed.title = "a renamed title".to_string();
    let refusal = update(&ctx, &id, &renamed).expect_err("a desktop is not a remote runner");
    assert_eq!(
        Some(refusal),
        detached_target_refusal(&desktop.id, Some(&desktop))
    );

    let after = ctx
        .tickets
        .get(&id)
        .expect("the ticket reads back")
        .expect("the ticket exists");
    assert_eq!(after.machine_id, None);
    assert_eq!(after.title, "ticket 1");
}

/// The drawer echoes the stored placement on every save, so a check that ran
/// on it would refuse a title edit over a field the user never touched once
/// the machine's row is gone. Start refuses the stale id instead.
#[tokio::test]
async fn an_unchanged_placement_on_a_deleted_machine_does_not_block_the_save() {
    let (ctx, id) = stored_ticket("deleted");
    update(&ctx, &id, &placed("gpu-box")).expect("a configured machine saves");
    let gpu_box = MachineId::from("gpu-box".to_string());
    ctx.machines
        .delete(&gpu_box)
        .expect("the machine is deleted");

    let mut renamed = placed("gpu-box");
    renamed.title = "a renamed title".to_string();
    update(&ctx, &id, &renamed).expect("the placement did not change");

    let after = ctx
        .tickets
        .get(&id)
        .expect("the ticket reads back")
        .expect("the ticket exists");
    assert_eq!(after.title, "a renamed title");
    assert_eq!(after.machine_id, Some(gpu_box));
}

/// The wire shape the drawer sends, with `key` removed when given.
fn wire(without: Option<&str>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "title": "a title",
        "description": "",
        "acceptance": [],
        "files": [],
        "blocked_by": [],
        "test_command": null,
        "workflow_id": null,
        "agent_kind": null,
        "model": null,
        "effort": null,
        "machine_id": null,
    });
    if let Some(key) = without {
        value
            .as_object_mut()
            .expect("the wire shape is an object")
            .remove(key);
    }
    value
}

/// An absent nullable key would otherwise read as `null` — for `machine_id`,
/// a save that silently clears the ticket's placement back to Default.
#[test]
fn every_nullable_key_is_required_and_null_is_its_clear() {
    let edit: TicketEdit = serde_json::from_value(wire(None)).expect("explicit nulls parse");
    assert_eq!(edit.machine_id, None);

    for key in [
        "test_command",
        "workflow_id",
        "agent_kind",
        "model",
        "effort",
        "machine_id",
    ] {
        let error = serde_json::from_value::<TicketEdit>(wire(Some(key)))
            .expect_err("an absent key is refused");
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}
