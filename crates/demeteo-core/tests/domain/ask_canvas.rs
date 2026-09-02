// Tests extracted from `crates/demeteo-core/src/domain/ask_canvas.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::AskThreadId;
use crate::domain::models::ask::CanvasPathVerdict;
use crate::domain::models::MessageRole;

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

#[test]
fn an_edge_pointing_at_its_own_node_is_refused() {
    let mut c = canvas();
    c.edges[0].to = c.edges[0].from.clone();
    assert!(validate_canvas(&c).unwrap().contains("itself"));
}

/// Not a taste judgement: the renderer keys an edge by its endpoints and its
/// kind, so a repeat has no drawing of its own to be given.
#[test]
fn the_same_edge_declared_twice_is_refused() {
    let mut c = canvas();
    let dup = c.edges[0].clone();
    c.edges.push(dup);
    assert!(validate_canvas(&c).unwrap().contains("more than once"));
}

/// The two placements the renderer absorbs rather than refuses: it tiles a
/// shared cell and routes from positions rather than from the label, so
/// neither costs the reader the whole answer.
#[test]
fn two_nodes_in_one_cell_and_a_backwards_hands_off_are_both_drawable() {
    let mut c = canvas();
    let mut second = c.nodes[1].clone();
    second.id = "n3".to_string();
    second.stage = c.nodes[0].stage;
    second.lane = c.nodes[0].lane;
    c.nodes.push(second);
    c.edges.push(CanvasEdge {
        from: "n2".to_string(),
        to: "n1".to_string(),
        kind: EdgeKind::HandsOff,
    });

    assert_eq!(validate_canvas(&c), None);
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

/// The vocabulary is the only place a model is told these tokens exist, and
/// nothing else goes red when one is missing from it. No match below has a
/// wildcard arm, on the same terms as `no_role_is_ruby`.
#[test]
fn the_vocabulary_names_every_token_a_block_may_carry() {
    fn role(role: NodeRole) -> &'static str {
        match role {
            NodeRole::Orchestration => "orchestration",
            NodeRole::Boundary => "boundary",
            NodeRole::Agent => "agent",
            NodeRole::NeedsHuman => "needs_human",
        }
    }
    fn block_kind(kind: CanvasKind) -> &'static str {
        match kind {
            CanvasKind::Architecture => "architecture",
            CanvasKind::Journey => "journey",
            CanvasKind::Dataflow => "dataflow",
        }
    }
    fn edge_kind(kind: EdgeKind) -> &'static str {
        match kind {
            EdgeKind::HandsOff => "hands_off",
            EdgeKind::GoesBack => "goes_back",
        }
    }

    let vocabulary = canvas_block_vocabulary();
    let tokens = [
        role(NodeRole::Orchestration),
        role(NodeRole::Boundary),
        role(NodeRole::Agent),
        role(NodeRole::NeedsHuman),
        block_kind(CanvasKind::Architecture),
        block_kind(CanvasKind::Journey),
        block_kind(CanvasKind::Dataflow),
        edge_kind(EdgeKind::HandsOff),
        edge_kind(EdgeKind::GoesBack),
    ];
    for token in tokens {
        assert!(
            vocabulary.contains(token),
            "'{token}' deserializes but the prompt never tells the model it exists"
        );
    }
}

fn message_with(
    canvas_paths: Option<Vec<CanvasPathVerdict>>,
    checked_commit_sha: Option<String>,
) -> AskMessage {
    AskMessage {
        id: "m1".to_string(),
        thread_id: AskThreadId::new("t1"),
        role: MessageRole::Assistant,
        text: String::new(),
        cost_usd: None,
        tokens: None,
        turn_activity: None,
        canvas_paths,
        checked_commit_sha,
        created_at: 0,
    }
}

#[test]
fn a_pinned_snapshot_round_trips_the_canvas_paths_and_commit_sha() {
    let paths = vec![
        CanvasPathVerdict {
            node_id: "n1".to_string(),
            path: "src/lib.rs".to_string(),
            resolved: true,
        },
        CanvasPathVerdict {
            node_id: "n2".to_string(),
            path: "src/gone.rs".to_string(),
            resolved: false,
        },
    ];
    let message = message_with(Some(paths.clone()), Some("deadbeef".to_string()));
    let turn = AskTurn {
        prose: "prose".to_string(),
        canvas: Some(canvas()),
        canvas_error: None,
    };

    let snapshot = build_pinned_canvas_snapshot("t1", &message, &turn, 1_700_000_123_456)
        .expect("a turn with a canvas must produce a snapshot");

    assert_eq!(snapshot.thread_id, "t1");
    assert_eq!(snapshot.message_id, "m1");
    assert_eq!(snapshot.canvas, canvas());
    assert_eq!(snapshot.canvas_paths, paths);
    assert_eq!(snapshot.checked_commit_sha, Some("deadbeef".to_string()));
    assert_eq!(snapshot.pinned_at, 1_700_000_123_456);
}

#[test]
fn a_turn_with_no_canvas_cannot_be_pinned() {
    let message = message_with(None, None);
    let turn = AskTurn::default();

    assert!(build_pinned_canvas_snapshot("t1", &message, &turn, 0).is_err());
}
