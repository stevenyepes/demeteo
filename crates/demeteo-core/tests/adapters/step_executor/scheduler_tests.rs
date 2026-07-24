//! Table-driven coverage for the pure ready-set scheduler (task P1.11):
//! chains, diamond fan-in per join mode, skip propagation, when-guard
//! skips, the deadlock invariant, and the state-machine predicates.

use super::*;
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

fn def(nodes: &[&str], edges: serde_json::Value) -> WorkflowDefinitionV2 {
    let nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|spec| {
            // "id" or "id:join"
            let (id, join) = match spec.split_once(':') {
                Some((id, join)) => (id, Some(join)),
                None => (*spec, None),
            };
            let mut n = serde_json::json!({ "id": id, "type": "agent", "title": id,
                "config": { "prompt_template": "p" } });
            if let Some(j) = join {
                n["join"] = j.into();
            }
            n
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-sched",
        "name": "Sched",
        "nodes": nodes,
        "edges": edges
    }))
    .unwrap()
}

fn states(entries: &[(&str, NodeState)]) -> HashMap<StepId, NodeState> {
    entries
        .iter()
        .map(|(id, s)| (StepId::from(*id), s.clone()))
        .collect()
}

fn no_outputs(_: &str, _: &str) -> Option<ExprValue> {
    None
}

fn skipped(reason: &str) -> NodeState {
    NodeState::Skipped {
        reason: reason.into(),
    }
}

fn ready_ids(rs: &ReadySet) -> Vec<&str> {
    rs.ready.iter().map(|s| s.as_str()).collect()
}

fn skip_ids(rs: &ReadySet) -> Vec<&str> {
    rs.skip.iter().map(|(s, _)| s.as_str()).collect()
}

// ---------- roots and chains ----------

#[test]
fn roots_are_ready_immediately() {
    let d = def(
        &["a", "b", "c"],
        serde_json::json!([{ "from": "a", "to": "c" }, { "from": "b", "to": "c" }]),
    );
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[
        ("a", NodeState::Pending),
        ("b", NodeState::Pending),
        ("c", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(ready_ids(&rs), vec!["a", "b"], "both roots, join pending");
    assert!(rs.skip.is_empty());
}

#[test]
fn chain_advances_one_node_per_completion() {
    let d = def(
        &["a", "b", "c"],
        serde_json::json!([{ "from": "a", "to": "b" }, { "from": "b", "to": "c" }]),
    );
    let g = WorkflowGraph::build(&d).unwrap();

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Pending),
        ("c", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(ready_ids(&rs), vec!["b"], "c stays pending behind b");

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Running),
        ("c", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty(), "nothing new while b runs");
}

// ---------- diamond fan-in per join mode ----------

fn diamond(join_of_d: &str) -> WorkflowDefinitionV2 {
    let d_spec = format!("d:{join_of_d}");
    let nodes: Vec<&str> = vec!["a", "b", "c", d_spec.as_str()];
    def(
        &nodes,
        serde_json::json!([
            { "from": "a", "to": "b" }, { "from": "a", "to": "c" },
            { "from": "b", "to": "d" }, { "from": "c", "to": "d" }
        ]),
    )
}

#[test]
fn all_success_waits_for_every_branch() {
    let d = diamond("all_success");
    let g = WorkflowGraph::build(&d).unwrap();

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Completed),
        ("c", NodeState::Running),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty(), "one branch still running");

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Completed),
        ("c", NodeState::Completed),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(ready_ids(&rs), vec!["d"]);
}

#[test]
fn all_success_skips_on_first_failed_branch() {
    let d = diamond("all_success");
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Failed),
        ("c", NodeState::Running),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(
        skip_ids(&rs),
        vec!["d"],
        "failure decides without waiting for c"
    );
    assert!(rs.skip[0].1.contains("'b' failed"), "{}", rs.skip[0].1);
}

#[test]
fn any_success_fires_eagerly_and_skips_only_when_all_fail() {
    let d = diamond("any_success");
    let g = WorkflowGraph::build(&d).unwrap();

    // One branch done, other still running → eager fire.
    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Completed),
        ("c", NodeState::Running),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(ready_ids(&rs), vec!["d"]);

    // One failed, one running → undecided.
    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Failed),
        ("c", NodeState::Running),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty() && rs.skip.is_empty());

    // Both failed → skip, reason mentions both.
    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Failed),
        ("c", NodeState::Failed),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(skip_ids(&rs), vec!["d"]);
    assert!(rs.skip[0].1.contains("no dependency succeeded"));
}

#[test]
fn all_done_runs_regardless_of_outcomes_but_waits_for_all() {
    let d = diamond("all_done");
    let g = WorkflowGraph::build(&d).unwrap();

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Failed),
        ("c", NodeState::Running),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty(), "c not terminal yet");

    let s = states(&[
        ("a", NodeState::Completed),
        ("b", NodeState::Failed),
        ("c", skipped("guard")),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(
        ready_ids(&rs),
        vec!["d"],
        "failed + skipped both count as done"
    );
}

// ---------- skip propagation ----------

#[test]
fn skips_cascade_through_a_dead_branch_in_one_evaluation() {
    // a → b → c → d, a failed: b, c, d all resolve in one call.
    let d = def(
        &["a", "b", "c", "d"],
        serde_json::json!([
            { "from": "a", "to": "b" }, { "from": "b", "to": "c" }, { "from": "c", "to": "d" }
        ]),
    );
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[
        ("a", NodeState::Failed),
        ("b", NodeState::Pending),
        ("c", NodeState::Pending),
        ("d", NodeState::Pending),
    ]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(skip_ids(&rs), vec!["b", "c", "d"]);
    assert!(rs.skip[1].1.contains("'b' was skipped"), "{}", rs.skip[1].1);
    assert!(rs.ready.is_empty());
}

#[test]
fn cancelled_and_interrupted_dependencies_propagate_as_skips() {
    for (state, needle) in [
        (NodeState::Cancelled, "was cancelled"),
        (NodeState::Interrupted, "was interrupted"),
    ] {
        let d = def(&["a", "b"], serde_json::json!([{ "from": "a", "to": "b" }]));
        let g = WorkflowGraph::build(&d).unwrap();
        let s = states(&[("a", state), ("b", NodeState::Pending)]);
        let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
        assert_eq!(skip_ids(&rs), vec!["b"]);
        assert!(rs.skip[0].1.contains(needle), "{}", rs.skip[0].1);
    }
}

// ---------- when guards ----------

#[test]
fn guard_false_skips_and_guard_true_passes() {
    let mk = || {
        let mut d = def(
            &["critic", "gate-ship"],
            serde_json::json!([{ "from": "critic", "to": "gate-ship",
                "when": "${{ nodes.critic.outputs.verdict != 'FAIL' }}" }]),
        );
        d.nodes[1].node_type = "gate".into();
        d
    };
    let d = mk();
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[
        ("critic", NodeState::Completed),
        ("gate-ship", NodeState::Pending),
    ]);

    let pass = |_: &str, _: &str| Some(ExprValue::Str("PASS_WITH_NOTES".into()));
    let rs = evaluate_ready_set(&d, &g, &s, &pass).unwrap();
    assert_eq!(ready_ids(&rs), vec!["gate-ship"]);

    let fail = |_: &str, _: &str| Some(ExprValue::Str("FAIL".into()));
    let rs = evaluate_ready_set(&d, &g, &s, &fail).unwrap();
    assert_eq!(skip_ids(&rs), vec!["gate-ship"]);
    assert!(rs.skip[0].1.contains("evaluated false"), "{}", rs.skip[0].1);
}

#[test]
fn guard_evaluation_error_is_an_unsatisfiable_edge_not_a_silent_pass() {
    let d = def(
        &["a", "b"],
        serde_json::json!([{ "from": "a", "to": "b",
            "when": "${{ nodes.a.outputs.missing == 1 }}" }]),
    );
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[("a", NodeState::Completed), ("b", NodeState::Pending)]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert_eq!(skip_ids(&rs), vec!["b"]);
    assert!(
        rs.skip[0].1.contains("could not be evaluated"),
        "{}",
        rs.skip[0].1
    );
}

#[test]
fn guards_are_not_evaluated_before_the_source_completes() {
    // The resolver would error if called; a running source must keep the
    // edge undecided without touching the guard.
    let d = def(
        &["a", "b"],
        serde_json::json!([{ "from": "a", "to": "b",
            "when": "${{ nodes.a.outputs.verdict == 'PASS' }}" }]),
    );
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[("a", NodeState::Running), ("b", NodeState::Pending)]);
    let explode = |_: &str, _: &str| -> Option<ExprValue> {
        panic!("guard must not be evaluated while the source is running")
    };
    let rs = evaluate_ready_set(&d, &g, &s, &explode).unwrap();
    assert!(rs.ready.is_empty() && rs.skip.is_empty());
}

// ---------- deadlock invariant + errors ----------

#[test]
fn deadlock_reported_when_nothing_can_advance() {
    // Legal graph, but the driver's states are corrupt: b pending while
    // its only dependency is... itself never scheduled (a stuck without
    // any active node). Simplest reproduction: a pending node whose
    // dependency state row is missing → edge undecided forever.
    let d = def(&["a", "b"], serde_json::json!([{ "from": "a", "to": "b" }]));
    let g = WorkflowGraph::build(&d).unwrap();
    // 'a' has no state row (bookkeeping lag) and 'b' is pending: nothing
    // active, nothing decidable.
    let s = states(&[("b", NodeState::Pending)]);
    let err = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap_err();
    match err {
        ScheduleError::Deadlock(stuck) => {
            assert_eq!(stuck, vec![StepId::from("b")]);
        }
        other => panic!("expected deadlock, got {other:?}"),
    }
}

#[test]
fn no_deadlock_while_something_is_active_or_decidable() {
    let d = def(&["a", "b"], serde_json::json!([{ "from": "a", "to": "b" }]));
    let g = WorkflowGraph::build(&d).unwrap();

    // a runs → waiting is normal.
    let s = states(&[("a", NodeState::Running), ("b", NodeState::Pending)]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty() && rs.skip.is_empty());

    // a awaits a gate for days → still not a deadlock.
    let s = states(&[("a", NodeState::AwaitingGate), ("b", NodeState::Pending)]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty() && rs.skip.is_empty());

    // a awaits retry (policy will revive it) → not a deadlock.
    let s = states(&[("a", NodeState::AwaitingRetry), ("b", NodeState::Pending)]);
    let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
    assert!(rs.ready.is_empty() && rs.skip.is_empty());
}

#[test]
fn unknown_state_key_is_refused() {
    let d = def(&["a"], serde_json::json!([]));
    let g = WorkflowGraph::build(&d).unwrap();
    let s = states(&[("a", NodeState::Pending), ("ghost", NodeState::Running)]);
    assert_eq!(
        evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap_err(),
        ScheduleError::UnknownNode("ghost".into())
    );
}

// ---------- non-pending states are never re-promoted ----------

#[test]
fn only_pending_nodes_are_promoted() {
    let d = def(&["a", "b"], serde_json::json!([{ "from": "a", "to": "b" }]));
    let g = WorkflowGraph::build(&d).unwrap();
    for state in [
        NodeState::Ready,
        NodeState::Running,
        NodeState::Verifying,
        NodeState::AwaitingGate,
        NodeState::AwaitingRetry,
        NodeState::Completed,
        NodeState::Failed,
        NodeState::Cancelled,
        NodeState::Interrupted,
        skipped("x"),
    ] {
        let s = states(&[("a", NodeState::Completed), ("b", state.clone())]);
        let rs = evaluate_ready_set(&d, &g, &s, &no_outputs).unwrap();
        assert!(
            rs.ready.is_empty() && rs.skip.is_empty(),
            "state {state:?} must not be re-decided"
        );
    }
}

// ---------- state machine predicates (every PRD transition edge) ----------

#[test]
fn state_predicates_match_the_prd_diagram() {
    use NodeState::*;
    let terminal = [Completed, Failed, Cancelled, Interrupted, skipped("r")];
    let active = [Ready, Running, Verifying, AwaitingGate, AwaitingRetry];

    for s in &terminal {
        assert!(s.is_terminal(), "{s:?}");
        assert!(!s.is_active(), "{s:?}");
    }
    for s in &active {
        assert!(!s.is_terminal(), "{s:?}");
        assert!(s.is_active(), "{s:?}");
    }
    assert!(!Pending.is_terminal() && !Pending.is_active());
    assert!(Completed.is_success());
    for s in [
        Failed,
        Cancelled,
        Interrupted,
        skipped("r"),
        Pending,
        Running,
    ] {
        assert!(!s.is_success(), "{s:?}");
    }
}
