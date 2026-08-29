//! The structured diagram an Ask turn may carry, alongside its prose
//! (`docs/ask-canvas/probe/Nodes.html`).
//!
//! Synchronous and total, per the rule on [`crate::domain`].
//!
//! A turn's output is prose **plus an optional JSON block**, exactly as
//! [`crate::domain::discovery_question`]'s interview block is. The block is
//! not stored anywhere the prose is not: it rides inside the assistant
//! message's own text and is re-derived on read by [`parse_ask_turn`]. A
//! column of its own would let the two disagree about what a turn said, and
//! there is nothing about a canvas that the message text does not already
//! settle.
//!
//! [`canvas_block_shape_example`] is the single source for the shape: the
//! message that reports a malformed block quotes it rather than re-spelling
//! the JSON, so the two cannot drift.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where a node sits and what it does, fixed vocabulary matching
/// `docs/ask-canvas/probe/Nodes.html`'s four node tones. Ruby is never a
/// node role — it is reserved for failure/stopped states elsewhere in the
/// app (see `App.css`'s ruby tokens), so a fifth variant must not be added
/// without updating that surface first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// tone-violet / tx-violet — Feature, ExecutionDriver, Sync.
    Orchestration,
    /// tone-cyan / tx-cyan — StepCapability, PermissionProfile,
    /// ExecutionPort, git_ops::scope.
    Boundary,
    /// tone-emerald / tx-emerald — Worktree, opencode: a running agent.
    Agent,
    /// tone-amber / tx-amber — Gate: needs a human.
    NeedsHuman,
}

/// What kind of diagram the block is drawing. Reserved for the renderer's
/// choice of default framing; this module does not branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasKind {
    Architecture,
    Journey,
    Dataflow,
}

/// One box on the canvas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasNode {
    /// Stable within the block; what an edge names.
    pub id: String,
    pub title: String,
    pub role: NodeRole,
    /// The file or module this node names, when it has one — e.g.
    /// `git_ops::scope`. Absent for nodes that name a person or a concept.
    #[serde(default)]
    pub path: Option<String>,
    /// Index into the block's `stages`.
    pub stage: usize,
    /// Index into the block's `lanes`.
    pub lane: usize,
}

/// Which direction an edge reads as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// The normal direction: work moving forward.
    HandsOff,
    /// A redirect, retry, or rework — the arrow the reader would otherwise
    /// read as a mistake in the diagram rather than a real path.
    GoesBack,
}

/// One arrow between two [`CanvasNode`]s, named by id rather than position
/// so nodes may be reordered without touching edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

/// The JSON object an Ask turn may append to its prose: stages and lanes
/// declared up front, then nodes naming their cell.
///
/// Stages and lanes are authored groupings, not something the topology
/// could infer — `docs/ask-canvas/probe/Nodes.html`'s "01 · Orchestrator",
/// "02 · Policy & fence" stages and "01 · The person", "02 · Demeteo" lanes
/// exist because a layout algorithm has no swimlane concept, and one lane
/// cell is deliberately left with no node in it to read "nobody is acting
/// — the lane is waiting." Declaring cells is what keeps that renderable.
///
/// Every field is required — unlike [`crate::domain::discovery_question::InterviewBlock`],
/// whose all-optional fields need [`find_block`]'s accept predicate to tell
/// a real block from an unrelated `{}`, a JSON object that deserializes into
/// this type has already declared the whole shape. That is also why this
/// type does not derive `Default`: a kind-less or title-less canvas is not
/// a block, so there is no meaningful zero value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskCanvas {
    pub kind: CanvasKind,
    pub title: String,
    pub stages: Vec<String>,
    pub lanes: Vec<String>,
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
}

/// One assistant turn, split into what a human reads and what the UI
/// renders as a diagram.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskTurn {
    /// The turn text with the block cut out of it, whether or not that
    /// block turned out to be usable.
    pub prose: String,
    pub canvas: Option<AskCanvas>,
    /// Why the turn carried no usable canvas — set both by a block that
    /// parsed and was refused and by one that never parsed at all. Never
    /// promoted to a turn-level error: the turn keeps its prose either way.
    #[serde(default)]
    pub canvas_error: Option<String>,
}

/// The shape the assistant is asked to emit, and the shape an error message
/// quotes back at it.
pub fn canvas_block_shape_example() -> String {
    r#"{"kind": "architecture", "title": "...", "stages": ["01 · Orchestrator", "02 · Policy & fence"], "lanes": ["01 · The person", "02 · Demeteo"], "nodes": [{"id": "n1", "title": "...", "role": "orchestration", "stage": 0, "lane": 1}, {"id": "n2", "title": "...", "role": "boundary", "path": "git_ops::scope", "stage": 1, "lane": 1}], "edges": [{"from": "n1", "to": "n2", "kind": "hands_off"}]}"#
        .to_string()
}

/// Reject a canvas the surface could not render. Returns a human-readable
/// reason, or `None` when it is drawable.
///
/// An unknown `role` is not checked here — [`NodeRole`] already rejects it
/// at deserialize time, before a value of this type can exist, and that
/// failure is surfaced through [`refused_tail`] instead. Every other rule
/// is a rendering failure rather than a taste judgement.
pub fn validate_canvas(canvas: &AskCanvas) -> Option<String> {
    for node in &canvas.nodes {
        if node.stage >= canvas.stages.len() {
            return Some(format!(
                "node '{}' names stage {}, but the block only declares {} stage(s)",
                node.id,
                node.stage,
                canvas.stages.len()
            ));
        }
        if node.lane >= canvas.lanes.len() {
            return Some(format!(
                "node '{}' names lane {}, but the block only declares {} lane(s)",
                node.id,
                node.lane,
                canvas.lanes.len()
            ));
        }
    }

    let index: HashMap<&str, usize> = canvas
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id.as_str(), i))
        .collect();
    for edge in &canvas.edges {
        for id in [edge.from.as_str(), edge.to.as_str()] {
            if !index.contains_key(id) {
                return Some(format!("edge names unknown node '{id}'"));
            }
        }
    }

    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, node) in canvas.nodes.iter().enumerate() {
        if seen.insert(node.id.as_str(), i).is_some() {
            return Some(format!("node id '{}' appears more than once", node.id));
        }
    }
    None
}

/// Split an assistant turn into prose and the block it carried.
///
/// Tolerant about where the block sits, through
/// [`crate::domain::json_block`]. A turn with no block is prose — the
/// assistant is not obliged to draw a diagram every turn.
///
/// A turn that *tried* to carry one and failed is not prose, though, which
/// is the difference [`refused_tail`] makes: the raw object would otherwise
/// be rendered at the reader as though the assistant had said it. This is
/// also the only path an unknown `role` string can take, since it fails
/// [`AskCanvas`] deserialization before [`validate_canvas`] ever runs.
pub fn parse_ask_turn(text: &str) -> AskTurn {
    if let Some((span, canvas)) = find_block(text) {
        let error = validate_canvas(&canvas);
        return AskTurn {
            prose: without(text, span),
            canvas: error.is_none().then_some(canvas),
            canvas_error: error,
        };
    }
    match refused_tail(text) {
        Some((span, canvas_error)) => AskTurn {
            prose: without(text, span),
            canvas_error,
            ..Default::default()
        },
        None => AskTurn {
            prose: text.trim().to_string(),
            ..Default::default()
        },
    }
}

fn without(text: &str, span: (usize, usize)) -> String {
    let mut kept = String::with_capacity(text.len());
    kept.push_str(&text[..span.0]);
    kept.push_str(&text[span.1..]);
    kept.trim().to_string()
}

/// The block and the byte span it occupied, so the prose can be cut free of
/// it.
///
/// The accept rule is trivial here, unlike
/// [`crate::domain::discovery_question`]'s: every field of [`AskCanvas`] is
/// required, so a candidate that deserializes into one has already declared
/// the whole shape. There is no all-optional-fields case for an unrelated
/// `{}` to hide behind.
fn find_block(text: &str) -> Option<((usize, usize), AskCanvas)> {
    crate::domain::json_block::find_json_block(text, |_: &AskCanvas| true)
}

/// The keys that make a trailing object this turn's own block rather than
/// something the assistant quoted.
const DECLARED_KEYS: [&str; 6] = [
    "\"kind\"",
    "\"title\"",
    "\"stages\"",
    "\"lanes\"",
    "\"nodes\"",
    "\"edges\"",
];

/// A block the turn ended on that [`find_block`] would not take: the span to
/// cut, and what to tell the user about it.
///
/// Reaching a `serde_json::from_str::<AskCanvas>` failure here is how an
/// unknown `role` string is reported: [`find_block`] never accepted the
/// candidate in the first place, since deserializing an [`AskCanvas`] fails
/// as soon as one of its nodes names a `role` outside [`NodeRole`]'s four
/// tokens, and serde's own message already names the bad value.
///
/// Naming one of [`DECLARED_KEYS`] is what makes this safe to run on every
/// turn with no block. Prose ends in a brace-wrapped identifier often enough
/// that position alone would accuse the assistant of a malformed canvas
/// every time it signed off with `{feature_id}`.
fn refused_tail(text: &str) -> Option<((usize, usize), Option<String>)> {
    let (start, end) = crate::domain::json_block::trailing_object(text)?;
    let tail = &text[start..end];
    if !DECLARED_KEYS.iter().any(|key| tail.contains(key)) {
        return None;
    }
    let reason = serde_json::from_str::<AskCanvas>(tail).err().map(|e| {
        format!(
            "the block it ended on could not be read as a canvas ({e}); expected shape: {}",
            canvas_block_shape_example()
        )
    });
    Some(((start, end), reason))
}

#[cfg(test)]
#[path = "../../tests/domain/ask_canvas.rs"]
mod tests;
