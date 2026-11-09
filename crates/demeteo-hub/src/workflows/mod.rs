//! The Workflows a Hub offers: the desktop's bundled starters
//! (`src-tauri/workflows/`), embedded at build time by `build.rs`. They can be
//! chosen and never edited, and they are the Hub's own: a dispatch to a runner
//! signs a starter from here and never one a desktop reported. That retires
//! decision 65's resolution of a runner's Workflow through a source desktop's
//! snapshot.
//!
//! The Hub does not link core, so it reads a starter only as far as it needs
//! to list it and draw it. Whether the step list is one a runner can run is
//! the runner's to judge, and `src-tauri/tests/hub_starters.rs` holds every
//! starter to that judgement.

pub mod graph;
pub mod handlers;

use crate::api::snapshot_view::WorkflowGraphView;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/starters.rs"));

#[derive(Debug, Clone, PartialEq)]
pub struct Starter {
    /// `wf-starter-…`, the id a desktop seeds the same file under.
    pub id: String,
    pub name: String,
    pub description: String,
    /// [`version_of`] `steps_json`.
    pub version: u32,
    pub steps_json: String,
    pub graph: WorkflowGraphView,
}

#[derive(Deserialize)]
struct StarterFile {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    steps: serde_json::Value,
}

/// Every embedded starter that parses, in file-name order. One that does not
/// is left out rather than failing the Hub, and fails a test instead.
pub fn starters() -> &'static [Starter] {
    static STARTERS: OnceLock<Vec<Starter>> = OnceLock::new();
    STARTERS.get_or_init(|| {
        STARTER_FILES
            .iter()
            .filter_map(|file| parsed(file))
            .collect()
    })
}

pub fn starter(id: &str) -> Option<&'static Starter> {
    starters().iter().find(|starter| starter.id == id)
}

fn parsed(file: &str) -> Option<Starter> {
    let file: StarterFile = serde_json::from_str(file).ok()?;
    if file.id.is_empty() || file.steps.as_array().is_none_or(Vec::is_empty) {
        return None;
    }
    let graph = graph::graph_of(&file.steps)?;
    let steps_json = serde_json::to_string(&file.steps).ok()?;
    Some(Starter {
        version: version_of(&steps_json),
        id: file.id,
        name: file.name,
        description: file.description,
        steps_json,
        graph,
    })
}

/// A starter file carries no version, and a runner pins one, so it is derived
/// from the steps it signs: the first four bytes of their SHA-256, as a
/// positive `i32` so every reader of a version holds it, and never 0. The same
/// steps are the same version on every Hub build; any change to them is,
/// barring a collision, another.
pub fn version_of(steps_json: &str) -> u32 {
    let digest = Sha256::digest(steps_json.as_bytes());
    let head = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (head & 0x7fff_ffff).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_embedded_file_is_a_starter_with_steps() {
        assert!(!STARTER_FILES.is_empty());
        assert_eq!(starters().len(), STARTER_FILES.len());
        for starter in starters() {
            assert!(starter.id.starts_with("wf-starter-"), "{}", starter.id);
            assert!(!starter.name.is_empty(), "{}", starter.id);
            assert!(!starter.graph.nodes.is_empty(), "{}", starter.id);
        }
    }

    #[test]
    fn starter_ids_are_unique_and_found_by_id() {
        let ids: HashSet<&str> = starters().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), starters().len());
        for starter in starters() {
            assert_eq!(super::starter(&starter.id), Some(starter));
        }
        assert_eq!(super::starter("wf-starter-nope"), None);
    }

    #[test]
    fn a_version_is_stable_positive_and_follows_the_steps() {
        let steps = r#"[{"id":"a","kind":"agent","title":"A"}]"#;
        assert_eq!(version_of(steps), version_of(steps));
        assert_ne!(
            version_of(steps),
            version_of(r#"[{"id":"a","kind":"agent","title":"B"}]"#)
        );
        for starter in starters() {
            assert_eq!(starter.version, version_of(&starter.steps_json));
            assert!(starter.version > 0 && starter.version <= i32::MAX as u32);
        }
        assert_eq!(version_of(steps), 0x6f66_a55b);
    }

    #[test]
    fn a_file_without_an_id_or_steps_is_left_out() {
        let steps = r#"[{"id":"a","kind":"agent","title":"A"}]"#;
        for file in [
            "not json".to_owned(),
            format!(r#"{{"id":"","steps":{steps}}}"#),
            r#"{"id":"wf-starter-x","steps":[]}"#.to_owned(),
            r#"{"id":"wf-starter-x","steps":{}}"#.to_owned(),
            r#"{"id":"wf-starter-x","steps":[{"id":"a"}]}"#.to_owned(),
        ] {
            assert_eq!(parsed(&file), None, "{file}");
        }
        let kept = parsed(&format!(r#"{{"id":"wf-starter-x","steps":{steps}}}"#)).unwrap();
        assert_eq!(kept.steps_json, steps);
        assert_eq!(kept.name, "");
    }
}
