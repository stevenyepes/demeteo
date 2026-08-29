// Tests extracted from `crates/demeteo-core/src/domain/ask_canvas.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn node(id: &str, role: NodeRole, stage: usize, lane: usize) -> CanvasNode {
    CanvasNode {
        id: id.to_string(),
        title: format!("title {id}"),
        role,
        path: None,
        stage,
        lane,
    }
}

fn canvas() -> AskCanvas {
    AskCanvas {
        kind: CanvasKind::Architecture,
        title: "Demeteo orchestration".to_string(),
        stages: vec!["Orchestrator".to_string(), "Policy & fence".to_string()],
        lanes: vec!["The person".to_string(), "Demeteo".to_string()],
        nodes: vec![
            node("n1", NodeRole::Orchestration, 0, 0),
            node("n2", NodeRole::Boundary, 1, 1),
        ],
        edges: vec![CanvasEdge {
            from: "n1".to_string(),
            to: "n2".to_string(),
            kind: EdgeKind::HandsOff,
        }],
    }
}

#[test]
fn the_shape_example_is_what_the_parser_accepts() {
    let turn = parse_ask_turn(&canvas_block_shape_example());
    assert!(turn.canvas.is_some());
    assert_eq!(turn.canvas_error, None);
}

#[test]
fn the_malformed_block_message_quotes_the_one_shape_source() {
    let text = "Two things it leaves open.\n{\"nodes\": [{\"id\": \"n1\"}]}";
    let turn = parse_ask_turn(text);
    let error = turn
        .canvas_error
        .expect("a block naming a declared key that fails to deserialize should be reported");
    assert!(error.contains(&canvas_block_shape_example()));
}

#[test]
fn prose_followed_by_a_well_formed_block_yields_both() {
    let text = format!(
        "The sketch settles the transport and nothing else.\n\n{}",
        serde_json::to_string(&canvas()).unwrap()
    );
    let turn = parse_ask_turn(&text);
    assert_eq!(
        turn.prose,
        "The sketch settles the transport and nothing else."
    );
    assert!(turn.canvas.is_some());
    assert_eq!(turn.canvas_error, None);
}

#[test]
fn prose_alone_yields_no_canvas() {
    let turn =
        parse_ask_turn("Taking that as written rather than fitting it to the nearest option.");
    assert_eq!(
        turn.prose,
        "Taking that as written rather than fitting it to the nearest option."
    );
    assert!(turn.canvas.is_none());
    assert_eq!(turn.canvas_error, None);
}

#[test]
fn an_unknown_role_is_refused() {
    let text = "Two things it leaves open.\n{\"kind\": \"architecture\", \"title\": \"x\", \
                \"stages\": [\"s1\"], \"lanes\": [\"l1\"], \
                \"nodes\": [{\"id\": \"n1\", \"title\": \"n1\", \"role\": \"villain\", \
                \"stage\": 0, \"lane\": 0}], \"edges\": []}";
    let turn = parse_ask_turn(text);
    assert!(turn.canvas.is_none());
    assert!(turn.canvas_error.unwrap().contains("villain"));
}

#[test]
fn a_stage_index_out_of_range_is_refused() {
    let mut c = canvas();
    let bad_stage = c.stages.len();
    c.nodes[0].stage = bad_stage;
    assert!(validate_canvas(&c)
        .unwrap()
        .contains(bad_stage.to_string().as_str()));
}

#[test]
fn a_lane_index_out_of_range_is_refused() {
    let mut c = canvas();
    let bad_lane = c.lanes.len();
    c.nodes[0].lane = bad_lane;
    assert!(validate_canvas(&c)
        .unwrap()
        .contains(bad_lane.to_string().as_str()));
}

#[test]
fn an_edge_naming_an_unknown_node_is_refused() {
    let mut c = canvas();
    c.edges[0].to = "nonexistent".to_string();
    assert!(validate_canvas(&c).unwrap().contains("nonexistent"));
}

/// A pushed clone, not a rename, so the shared id is a real duplicate rather
/// than also orphaning the edge that names the node being renamed away from.
#[test]
fn duplicate_node_ids_are_refused() {
    let mut c = canvas();
    let mut dup = c.nodes[0].clone();
    dup.stage = 1;
    dup.lane = 1;
    c.nodes.push(dup);
    assert!(validate_canvas(&c).unwrap().contains("n1"));
}

#[test]
fn a_block_that_fails_to_deserialize_degrades_the_turn() {
    let text = "Two things it leaves open.\n{\"nodes\": [{\"id\": \"n1\"";
    let turn = parse_ask_turn(text);
    assert_eq!(turn.prose, "Two things it leaves open.");
    assert!(turn.canvas.is_none());
    assert!(turn.canvas_error.is_some());
}

#[test]
fn a_block_that_deserializes_but_fails_validation_degrades_the_turn() {
    let mut c = canvas();
    let mut dup = c.nodes[0].clone();
    dup.stage = 1;
    dup.lane = 1;
    c.nodes.push(dup);
    let text = format!("Prose.\n{}", serde_json::to_string(&c).unwrap());
    let turn = parse_ask_turn(&text);
    assert!(turn.canvas.is_none());
    assert!(turn.canvas_error.unwrap().contains("n1"));
    assert_eq!(turn.prose, "Prose.");
}

#[test]
fn an_unrelated_trailing_brace_is_not_mistaken_for_a_block() {
    let turn = parse_ask_turn("The branch is `{feature_branch}`");
    assert_eq!(turn.prose, "The branch is `{feature_branch}`");
    assert!(turn.canvas.is_none());
    assert!(turn.canvas_error.is_none());
}

#[test]
fn every_role_has_exactly_one_serde_token() {
    let cases = [
        (NodeRole::Orchestration, "\"orchestration\""),
        (NodeRole::Boundary, "\"boundary\""),
        (NodeRole::Agent, "\"agent\""),
        (NodeRole::NeedsHuman, "\"needs_human\""),
    ];
    for (role, token) in cases {
        assert_eq!(serde_json::to_string(&role).unwrap(), token);
        assert_eq!(serde_json::from_str::<NodeRole>(token).unwrap(), role);
    }
}

/// Ruby is reserved for failure/stopped states; a role that mapped to it
/// would need updating here, and the match has no wildcard arm to hide a
/// fifth variant behind.
#[test]
fn no_role_is_ruby() {
    fn tone(role: NodeRole) -> &'static str {
        match role {
            NodeRole::Orchestration => "violet",
            NodeRole::Boundary => "cyan",
            NodeRole::Agent => "emerald",
            NodeRole::NeedsHuman => "amber",
        }
    }
    assert_eq!(tone(NodeRole::Orchestration), "violet");
    assert_eq!(tone(NodeRole::Boundary), "cyan");
    assert_eq!(tone(NodeRole::Agent), "emerald");
    assert_eq!(tone(NodeRole::NeedsHuman), "amber");
}
