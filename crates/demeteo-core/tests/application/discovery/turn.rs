// Tests extracted from `crates/demeteo-core/src/application/discovery/turn.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::{MachineId, ProjectId, LOCAL_MACHINE};

#[test]
fn a_resumed_turn_that_said_nothing_and_failed_is_retried_from_the_transcript() {
    assert!(should_reseed_and_retry(true, false, TurnEnding::Failed));
    assert!(should_reseed_and_retry(
        true,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_resumed_turn_that_answered_before_failing_reached_the_model() {
    assert!(!should_reseed_and_retry(true, true, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        true,
        true,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_turn_that_already_carried_the_transcript_has_nothing_to_fall_back_to() {
    assert!(!should_reseed_and_retry(false, false, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        false,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_stop_is_the_user_declining_the_turn_not_a_lost_session() {
    assert!(!should_reseed_and_retry(
        true,
        false,
        TurnEnding::Interrupted
    ));
}

#[test]
fn a_turn_that_worked_is_never_run_twice() {
    assert!(!should_reseed_and_retry(true, false, TurnEnding::Success));
}

#[test]
fn every_ending_but_a_stop_reports_what_it_spent() {
    let outcome = || TurnOutcome {
        text: "said something".to_string(),
        produced_artifacts: Vec::new(),
        cost_usd: 0.31,
        tokens: 12_400,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    for result in [
        TurnResult::Success(outcome()),
        TurnResult::Failed {
            reason: "rate limited".to_string(),
            spent: outcome(),
        },
        TurnResult::Environmental {
            reason: "wall cap".to_string(),
            spent: outcome(),
        },
    ] {
        let (_, _, spent) = split(result);
        assert_eq!(spent.cost_usd, 0.31);
        assert_eq!(spent.tokens, 12_400);
    }
    let (ending, reason, spent) = split(TurnResult::Interrupted);
    assert_eq!(ending, TurnEnding::Interrupted);
    assert_eq!(reason, None);
    assert_eq!(spent.cost_usd, 0.0);
}

#[test]
fn the_interviewer_may_read_and_run_but_never_write() {
    let p = interviewer_permissions();
    assert_eq!(p.read_fs, Access::Allow);
    assert_eq!(p.write_fs, Access::Deny);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.network, Access::Allow);
}

fn discovery() -> Discovery {
    Discovery {
        id: DiscoveryId::from("d-1".to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        title: "multi-client runner".to_string(),
        status: DiscoveryStatus::Open,
        machine_id: MachineId::from(LOCAL_MACHINE.to_string()),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        resume_session_id: None,
        worktree_path: None,
        attachments: Vec::new(),
        total_cost: 0.0,
        tokens: 0,
        created_at: 0,
        updated_at: 0,
    }
}

/// Every status put on the wire, each paired with whether the Discovery read
/// as running at the instant it was sent — which is the half of the claim's
/// contract no later assertion can recover.
type Wire = Arc<std::sync::Mutex<Vec<(String, bool)>>>;

fn recorder(turns: Arc<RunningTurns>) -> (impl Fn(&str, serde_json::Value), Wire) {
    let wire: Wire = Arc::new(std::sync::Mutex::new(Vec::new()));
    let written = wire.clone();
    let emit = move |event: &str, payload: serde_json::Value| {
        assert_eq!(event, EVENT_DISCOVERY_TURN_STATUS);
        written.lock().unwrap().push((
            payload["status"].as_str().unwrap_or_default().to_string(),
            turns.running(&DiscoveryId::from("d-1".to_string())),
        ));
    };
    (emit, wire)
}

#[tokio::test]
async fn a_turn_says_it_is_setting_up_before_it_starts_setting_up() {
    let turns = Arc::new(RunningTurns::default());
    let (emit, wire) = recorder(turns.clone());
    let d = discovery();

    let heard = wire.clone();
    let (announced_by_then, claim) = announced(&emit, &d, turns.clone(), async move {
        Ok::<Vec<(String, bool)>, String>(heard.lock().unwrap().clone())
    })
    .await
    .expect("preparing succeeded");

    assert_eq!(
        announced_by_then,
        vec![(STATUS_SETTING_UP.to_string(), true)],
        "the surface learns a turn is setting up only after it has finished setting up"
    );
    drop(claim);
    assert!(!turns.running(&d.id));
}

#[tokio::test]
async fn a_setup_that_failed_gives_the_claim_back_before_it_says_so() {
    let turns = Arc::new(RunningTurns::default());
    let (emit, wire) = recorder(turns.clone());
    let d = discovery();

    let outcome = announced(&emit, &d, turns.clone(), async {
        Err::<(), String>("This project has no checkout on 'builder'".to_string())
    })
    .await;

    assert!(outcome.is_err());
    assert_eq!(
        wire.lock().unwrap().as_slice(),
        [
            (STATUS_SETTING_UP.to_string(), true),
            (STATUS_ERROR.to_string(), false)
        ]
    );
}
