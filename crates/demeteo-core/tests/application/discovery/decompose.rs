// Tests extracted from
// `crates/demeteo-core/src/application/discovery/decompose/mod.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::DiscoveryId;
use crate::domain::models::EffortLevel;
use crate::domain::ticket_plan::PlannedTicket;
use proposal::ChangeKind;

fn choices() -> Choices {
    Choices {
        workflows: vec![(
            WorkflowId::from("wf-standard".to_string()),
            "Standard Feature".to_string(),
        )],
        agents: vec!["claude-code".to_string(), "opencode".to_string()],
    }
}

fn row(id: &str, seq: i64) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: format!("ticket {seq}"),
        description: "settled earlier".to_string(),
        acceptance: vec!["it works".to_string()],
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
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

fn as_written(row: &Ticket) -> PlannedTicket {
    PlannedTicket {
        id: row.id.0.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        acceptance: row.acceptance.clone(),
        files: row.files.clone(),
        test_command: row.test_command.clone(),
        blocked_by: row.blocked_by.iter().map(|id| id.0.clone()).collect(),
        workflow: None,
        agent: None,
        model: None,
        effort: None,
        why: None,
    }
}

fn answer(tickets: &[PlannedTicket]) -> String {
    serde_json::to_string(&TicketPlan {
        tickets: tickets.to_vec(),
    })
    .expect("the plan should serialize")
}

/// The one source for the shape has to reach both places §5.2 needs it: the
/// prompt that asks for it, and the refusal for an answer that carried none.
#[test]
fn the_prompt_and_the_unreadable_answer_quote_the_same_shape() {
    let shape = ticket_plan_json_shape_example();
    assert!(prompt::decompose_request(&[], &choices()).contains(&shape));

    let rejected = read_plan("I think we should start with auth.", &[], &choices())
        .err()
        .expect("prose is not a plan");
    assert!(
        rejected.message().contains(&shape),
        "{}",
        rejected.message()
    );
}

/// A re-decomposition that restates the plan it inherited is the normal path
/// (§5.3), and it must produce no changes at all — otherwise every pass would
/// offer the user a screen of edits that change nothing.
#[test]
fn a_plan_that_restates_the_stored_one_proposes_nothing() {
    let rows = vec![row("t-1", 1), row("t-2", 2)];
    let plan = TicketPlan {
        tickets: rows.iter().map(as_written).collect(),
    };
    let pass = Pass::resolve(plan, rows, &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(pass.diff.unchanged.len(), 2);
    assert!(pass.changes(&choices()).is_empty());
}

/// §5.4 puts the execution choices in the user's hands. A pass that says
/// nothing about them says nothing — it must not read as a proposal to clear
/// the routing someone set by hand.
#[test]
fn omitting_an_execution_choice_keeps_what_the_ticket_already_had() {
    let mut stored = row("t-1", 1);
    stored.workflow_id = Some(WorkflowId::from("wf-standard".to_string()));
    stored.agent_kind = Some("opencode".to_string());
    stored.effort = Some(EffortLevel::Max);

    let plan = TicketPlan {
        tickets: vec![as_written(&stored)],
    };
    let pass =
        Pass::resolve(plan, vec![stored], &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(pass.diff.unchanged, ["t-1"]);
    assert_eq!(
        pass.proposed[0].body.agent_kind.as_deref(),
        Some("opencode")
    );
    assert_eq!(pass.proposed[0].body.effort, Some(EffortLevel::Max));
}

/// The planned half is the decomposition's to write, so an omitted
/// `test_command` means the project's default rather than "leave it".
#[test]
fn omitting_a_planned_field_clears_it() {
    let mut stored = row("t-1", 1);
    stored.test_command = Some("cargo test -p demeteo-runner".to_string());
    let mut silent = as_written(&stored);
    silent.test_command = None;
    let plan = TicketPlan {
        tickets: vec![silent],
    };
    let pass =
        Pass::resolve(plan, vec![stored], &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(pass.diff.revised, ["t-1"]);
    let changes = pass.changes(&choices());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].fields.len(), 1);
    assert_eq!(changes[0].fields[0].field, "test command");
    assert_eq!(changes[0].fields[0].now, "");
}

/// A name the resolver would reject is the agent's own mistake and it is still
/// in context, so it comes back as a re-askable reason that lists what it may
/// have named — not as a ticket with a silently dropped workflow.
#[test]
fn an_invented_workflow_name_is_refused_with_the_real_ones() {
    let mut ticket = as_written(&row("t-1", 1));
    ticket.workflow = Some("Whatever Feature".to_string());
    let rejected = Pass::resolve(
        TicketPlan {
            tickets: vec![ticket],
        },
        Vec::new(),
        &choices(),
    )
    .err()
    .expect("an unknown workflow must be refused");
    let message = rejected.message();
    assert!(message.contains("Whatever Feature"), "{message}");
    assert!(message.contains("Standard Feature"), "{message}");
    assert!(rejected.violations().is_empty());
}

/// §5.3 holds a started ticket immutable, and the refusal is re-asked rather
/// than surfaced as a dead end — but it also survives onto the proposal, where
/// the modal renders it per ticket.
#[test]
fn revising_a_started_ticket_is_refused_and_names_it() {
    let mut started = row("t-1", 1);
    started.state = TicketState::Started;
    let mut reworded = as_written(&started);
    reworded.title = "a different title".to_string();

    let rejected = Pass::resolve(
        TicketPlan {
            tickets: vec![reworded],
        },
        vec![started],
        &choices(),
    )
    .err()
    .expect("a started ticket cannot be revised");
    assert_eq!(rejected.violations().len(), 1);
    assert_eq!(rejected.violations()[0].id, "t-1");
    assert!(rejected.message().contains("t-1"), "{}", rejected.message());
}

/// The addition has no `seq` yet — §5.3 assigns one at apply and never
/// reissues it, so a proposal has no number to show.
#[test]
fn an_addition_carries_its_rationale_and_no_number() {
    let mut fresh = as_written(&row("t-9", 9));
    fresh.why = Some("Learned while implementing #1.".to_string());
    fresh.workflow = Some("standard feature".to_string());
    let pass = Pass::resolve(
        TicketPlan {
            tickets: vec![fresh],
        },
        Vec::new(),
        &choices(),
    )
    .unwrap_or_else(|e| panic!("{}", e.message()));

    let changes = pass.changes(&choices());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Added);
    assert!(changes[0].seq.is_none());
    assert_eq!(
        changes[0].why.as_deref(),
        Some("Learned while implementing #1.")
    );
    assert_eq!(
        changes[0].workflow_name.as_deref(),
        Some("Standard Feature")
    );
}

/// A ticket the plan left out is a removal (§5.3), and it is a change the user
/// picks like any other — so it has to reach the modal with the number and
/// title of the row that would go.
#[test]
fn a_ticket_left_out_of_the_plan_is_offered_as_a_removal() {
    let rows = vec![row("t-1", 1), row("t-2", 2)];
    let plan = TicketPlan {
        tickets: vec![as_written(&rows[0])],
    };
    let pass = Pass::resolve(plan, rows, &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    let changes = pass.changes(&choices());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Removed);
    assert_eq!(changes[0].id, "t-2");
    assert_eq!(changes[0].seq, Some(2));
    assert_eq!(changes[0].title, "ticket 2");
}

/// The plan is read out of a turn that is allowed to be prose plus a block,
/// exactly as an interview turn is.
#[test]
fn a_plan_inside_prose_is_read() {
    let written = as_written(&row("t-1", 1));
    let text = format!(
        "Here is the plan.\n\n```json\n{}\n```\n",
        answer(&[written])
    );
    let pass = read_plan(&text, &[], &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(pass.diff.added, ["t-1"]);
}

/// The prompt is the only place a re-decomposition can learn the ids it has to
/// reuse — the interview's own context block names tickets by `#seq`.
#[test]
fn the_prompt_shows_the_stored_ids_and_which_are_fixed() {
    let mut started = row("t-1", 1);
    started.state = TicketState::Started;
    let text = prompt::decompose_request(&[started, row("t-2", 2)], &choices());
    assert!(text.contains("`t-1`"), "{text}");
    assert!(text.contains("`t-2`"), "{text}");
    let fixed = text
        .split("Already started, and therefore fixed:")
        .nth(1)
        .expect("a started ticket should be called out");
    assert!(fixed.contains("`t-1`"), "{fixed}");
    assert!(!fixed.contains("`t-2`"), "{fixed}");
}

/// The pass is stored as this payload and read back as this payload (V50), so
/// every part of it has to survive the round trip — a field that serializes
/// and will not deserialize would read as no proposal at all, silently, on the
/// visit after the one that paid for it.
#[test]
fn a_proposal_survives_being_written_down_and_read_back() {
    let rows = vec![row("keep", 1), row("drop-me", 2)];
    let mut added = as_written(&row("new", 3));
    added.id = "new".to_string();
    added.why = Some("the interview settled it".to_string());
    let plan = TicketPlan {
        tickets: vec![as_written(&rows[0]), added],
    };
    let pass =
        Pass::resolve(plan, rows.clone(), &choices()).unwrap_or_else(|e| panic!("{}", e.message()));
    let written = DecomposeProposal {
        discovery_id: "d-1".to_string(),
        first_pass: false,
        tickets: pass.plan.tickets.clone(),
        changes: pass.changes(&choices()),
        locked: proposal::locked(&rows, &HashMap::new()),
        refused: vec!["a cycle, answered".to_string()],
        refusal: None,
        violations: Vec::new(),
        cost_usd: 0.5,
        tokens: 1234,
    };

    let read: DecomposeProposal =
        serde_json::from_str(&serde_json::to_string(&written).unwrap()).unwrap();

    assert_eq!(read.discovery_id, "d-1");
    assert_eq!(read.tickets.len(), written.tickets.len());
    assert_eq!(read.changes.len(), written.changes.len());
    assert_eq!(read.changes[0].kind, written.changes[0].kind);
    assert_eq!(read.changes[0].id, written.changes[0].id);
    assert_eq!(read.refused, written.refused);
    assert_eq!(read.tokens, 1234);
}
