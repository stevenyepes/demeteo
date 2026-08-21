// Tests extracted from `crates/demeteo-core/src/application/discovery/context.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, TicketId};
use crate::domain::models::TicketState;

fn feature(title: &str, status: &str) -> Feature {
    Feature {
        id: FeatureId::from(title.to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: title.to_string(),
        description: String::new(),
        status: status.to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: None,
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

fn ticket(seq: i64, title: &str, state: TicketState) -> Ticket {
    Ticket {
        id: TicketId::from(format!("t-{seq}")),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: title.to_string(),
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
fn the_project_half_stops_at_the_bound_and_says_how_much_it_dropped() {
    let features: Vec<Feature> = (0..RECENT_FEATURES + 3)
        .map(|i| feature(&format!("run {i}"), "completed"))
        .collect();
    let rendered = render_from(&features, &[]);

    assert!(rendered.contains(&format!("run {}", RECENT_FEATURES - 1)));
    assert!(!rendered.contains(&format!("run {RECENT_FEATURES}")));
    assert!(rendered.contains("3 older runs not listed"));
}

#[test]
fn a_features_pr_state_rides_beside_its_status() {
    let mut f = feature("runner auth", "completed");
    f.mr_state = Some("open".to_string());
    let rendered = render_from(&[f], &[]);
    assert!(rendered.contains("[completed, pr open] runner auth"));
}

#[test]
fn the_whole_ticket_set_is_shown_however_long_it_gets() {
    let tickets: Vec<Ticket> = (1..=RECENT_FEATURES as i64 * 3)
        .map(|i| ticket(i, &format!("ticket {i}"), TicketState::Unstarted))
        .collect();
    let rendered = render_from(&[], &tickets);
    for t in &tickets {
        assert!(
            rendered.contains(&format!("#{} [unstarted] {}", t.seq, t.title)),
            "ticket {} was dropped from the context the additive diff reads",
            t.seq
        );
    }
}

#[test]
fn an_edge_is_rendered_by_the_number_a_user_says_out_loud() {
    let mut second = ticket(2, "auth", TicketState::Unstarted);
    second.blocked_by = vec![TicketId::from("t-1".to_string())];
    let rendered = render_from(
        &[],
        &[ticket(1, "registry", TicketState::Unstarted), second],
    );
    assert!(rendered.contains("blocked by #1"));
}

#[test]
fn a_dropped_ticket_carries_its_reason() {
    let mut t = ticket(3, "shared token", TicketState::Dropped);
    t.drop_reason = Some("revoking one laptop rotates every laptop".to_string());
    let rendered = render_from(&[], &[t]);
    assert!(rendered.contains("dropped: revoking one laptop"));
}

#[test]
fn both_halves_say_so_when_they_are_empty() {
    let rendered = render_from(&[], &[]);
    assert!(rendered.contains("No runs in flight"));
    assert!(rendered.contains("None yet."));
}

#[test]
fn a_long_title_is_clipped_rather_than_carried_whole() {
    let long = "x".repeat(SUMMARY_CHARS * 2);
    let rendered = render_from(&[feature(&long, "running")], &[]);
    assert!(rendered.contains('…'));
    assert!(!rendered.contains(&long));
}
