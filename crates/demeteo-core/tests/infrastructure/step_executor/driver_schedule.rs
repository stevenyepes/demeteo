//! Pure-half coverage for the run loop's scheduler glue (P1.12,
//! `driver/run_loop/schedule.rs`): DB-status → `NodeState` derivation and
//! the redirect rewind set. The impure halves (persist_skip /
//! reset_for_redirect) are exercised end-to-end by the P0.2 starter
//! baseline and the topology-equivalence conformance gates.

use super::*;
use crate::adapters::step_executor::scheduler::NodeState;
use crate::domain::ids::{FeatureId, StepId};
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;
use crate::domain::models::StepExecution;
use crate::domain::workflow_graph::WorkflowGraph;

fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> WorkflowGraph {
    let nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|id| {
            serde_json::json!({ "id": id, "type": "agent", "title": id,
                "config": { "prompt_template": "p" } })
        })
        .collect();
    let edges: Vec<serde_json::Value> = edges
        .iter()
        .map(|(from, to)| serde_json::json!({ "from": from, "to": to }))
        .collect();
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-glue",
        "name": "Glue",
        "nodes": nodes,
        "edges": edges
    }))
    .unwrap();
    WorkflowGraph::build(&def).expect("fixture graph builds")
}

fn exec(step_id: &str, index: u32, status: &str, error: Option<&str>) -> StepExecution {
    StepExecution {
        id: format!("se-f-{step_id}").into(),
        feature_id: FeatureId::from("f-1"),
        step_id: StepId::from(step_id),
        step_index: index,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: error.map(str::to_string),
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

// ---------- node_state_for ----------

#[test]
fn completed_and_skipped_are_terminal_everything_else_is_pending() {
    assert_eq!(node_state_for("completed", None), NodeState::Completed);
    assert_eq!(
        node_state_for(STATUS_SKIPPED, Some("guard failed")),
        NodeState::Skipped {
            reason: "guard failed".into()
        }
    );
    // v1 resume semantics: the first non-completed step re-dispatches
    // whatever its status was — all of these must schedule as Pending.
    for status in [
        "pending",
        "running",
        "verifying",
        "awaiting_gate",
        "failed",
        "interrupted",
        "cancelled",
    ] {
        assert_eq!(
            node_state_for(status, Some("stale")),
            NodeState::Pending,
            "{status} must derive to Pending"
        );
    }
}

#[test]
fn skipped_without_message_gets_an_empty_reason() {
    assert_eq!(
        node_state_for(STATUS_SKIPPED, None),
        NodeState::Skipped { reason: "".into() }
    );
}

// ---------- derive_states ----------

#[test]
fn derive_states_covers_every_graph_node_and_tolerates_missing_rows() {
    let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
    // No row for "c" — bookkeeping lag maps to Pending.
    let rows = vec![
        exec("a", 0, "completed", None),
        exec("b", 1, "failed", Some("boom")),
    ];
    let states = derive_states(&g, &rows);
    assert_eq!(states.len(), 3);
    assert_eq!(states[&StepId::from("a")], NodeState::Completed);
    assert_eq!(states[&StepId::from("b")], NodeState::Pending);
    assert_eq!(states[&StepId::from("c")], NodeState::Pending);
}

#[test]
fn derived_chain_states_make_exactly_the_v1_cursor_step_ready() {
    use crate::adapters::step_executor::scheduler::evaluate_ready_set;
    let nodes = ["a", "b", "c", "d"];
    let g = graph(&nodes, &[("a", "b"), ("b", "c"), ("c", "d")]);
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2, "id": "wf-glue", "name": "Glue",
        "nodes": nodes.iter().map(|id| serde_json::json!({
            "id": id, "type": "agent", "title": id,
            "config": { "prompt_template": "p" } })).collect::<Vec<_>>(),
        "edges": [ {"from": "a", "to": "b"}, {"from": "b", "to": "c"},
                   {"from": "c", "to": "d"} ]
    }))
    .unwrap();

    // completed prefix a,b — interrupted c (watchdog restart shape).
    let rows = vec![
        exec("a", 0, "completed", None),
        exec("b", 1, "completed", None),
        exec(
            "c",
            2,
            "interrupted",
            Some("Step interrupted by system restart"),
        ),
        exec("d", 3, "pending", None),
    ];
    let states = derive_states(&g, &rows);
    let rs = evaluate_ready_set(&def, &g, &states, &|_, _| None).expect("schedules");
    assert_eq!(
        rs.ready.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec!["c"],
        "exactly the old step_index resume target is ready"
    );
    assert!(rs.skip.is_empty());
}

// ---------- redirect_reset_set ----------

#[test]
fn chain_redirect_resets_target_through_the_tail() {
    let g = graph(&["a", "b", "c", "d"], &[("a", "b"), ("b", "c"), ("c", "d")]);
    let set = redirect_reset_set(&g, &StepId::from("b"));
    let ids: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
    assert_eq!(ids, vec!["b", "c", "d"], "v1 cursor jump: target..end");
}

#[test]
fn dag_redirect_resets_only_the_downstream_cone() {
    // a → b → d, a → c → d: redirecting to b must not touch c.
    let g = graph(
        &["a", "b", "c", "d"],
        &[("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
    );
    let set = redirect_reset_set(&g, &StepId::from("b"));
    let ids: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
    assert_eq!(ids, vec!["b", "d"]);
}

#[test]
fn redirect_to_unknown_node_resets_nothing() {
    let g = graph(&["a"], &[]);
    assert!(redirect_reset_set(&g, &StepId::from("ghost")).is_empty());
}

#[test]
fn a_skip_with_no_row_is_an_error_not_a_silent_no_op() {
    // The run loop `continue`s after persisting skips, without awaiting
    // anything. A skip that quietly fails to persist is therefore re-decided
    // identically on every following iteration — a hot spin with no exit. The
    // *ready* path already fails loudly for a node with no row; this is the
    // same decision for the skip path.
    let rows = vec![exec("a", 0, "pending", None)];
    let err = skip_target(&rows, &StepId::from("ghost"))
        .expect_err("a node with no row cannot record a skip");
    assert!(err.contains("ghost"), "the message names the node: {err}");
    assert!(err.contains("step_executions"), "{err}");
}

#[test]
fn a_skip_with_a_row_resolves_to_it() {
    let rows = vec![exec("a", 0, "pending", None), exec("b", 1, "pending", None)];
    let row = skip_target(&rows, &StepId::from("b")).expect("row b exists");
    assert_eq!(row.step_id, StepId::from("b"));
}

#[test]
fn an_unpersistable_skip_reads_as_a_run_blocker() {
    use crate::adapters::step_executor::scheduler::ScheduleError;
    let err = ScheduleError::UnpersistableSkip {
        node: "check".into(),
        error: "database is locked".into(),
    };
    let rendered = err.to_string();
    assert!(rendered.contains("check"), "{rendered}");
    assert!(rendered.contains("database is locked"), "{rendered}");
    // The point of the message: this is why the run stopped, not a warning.
    assert!(rendered.contains("forever"), "{rendered}");
}

/// A rewind clears a gate's decision row, and the run's own step list is what
/// says which nodes those are. The predicate is the whole decision — the write
/// beside it is one repository call.
mod rewound_gates {
    use super::*;
    use crate::adapters::step_executor::driver::run_loop::schedule::is_gate;
    use crate::domain::models::StepConfig;

    fn steps() -> Vec<StepConfig> {
        vec![
            StepConfig {
                id: StepId::from("s-implement"),
                kind: "sequence".to_string(),
                ..Default::default()
            },
            StepConfig {
                id: StepId::from("s-gate-review"),
                kind: "gate".to_string(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn a_gate_in_the_rewind_set_is_recognised() {
        assert!(is_gate(&steps(), &StepId::from("s-gate-review")));
    }

    #[test]
    fn a_non_gate_is_not() {
        assert!(!is_gate(&steps(), &StepId::from("s-implement")));
    }

    #[test]
    fn a_node_the_run_does_not_configure_is_not() {
        assert!(!is_gate(&steps(), &StepId::from("s-gate-ship")));
    }

    /// The case that made an `on_failure` chain re-approve itself: the failing
    /// step's rewind set contains the gate it points back at, so that gate's
    /// decision row is in scope for clearing.
    #[test]
    fn a_failure_redirect_to_a_gate_rewinds_that_gate() {
        let g = graph(
            &["s-gate-review", "s-implement", "s-validate"],
            &[
                ("s-gate-review", "s-implement"),
                ("s-implement", "s-validate"),
            ],
        );

        let rewound = redirect_reset_set(&g, &StepId::from("s-gate-review"));

        assert!(rewound.contains(&StepId::from("s-gate-review")));
        assert!(rewound.iter().any(|id| is_gate(&steps(), id)));
    }
}
