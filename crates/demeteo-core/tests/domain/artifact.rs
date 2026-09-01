use super::*;
use crate::domain::ask_canvas::{AskCanvas, CanvasKind, PinnedCanvasSnapshot};

fn pinned_canvas_snapshot() -> PinnedCanvasSnapshot {
    PinnedCanvasSnapshot {
        thread_id: "t1".to_string(),
        message_id: "m1".to_string(),
        canvas: AskCanvas {
            kind: CanvasKind::Architecture,
            title: "Demeteo orchestration".to_string(),
            stages: vec!["Orchestrator".to_string()],
            lanes: vec!["Demeteo".to_string()],
            nodes: vec![],
            edges: vec![],
        },
        canvas_paths: vec![],
        checked_commit_sha: Some("deadbeef".to_string()),
        pinned_at: 1234,
    }
}

#[test]
fn pinned_ask_canvas_artifact_has_no_dot_in_name_and_round_trips_the_snapshot() {
    let snapshot = pinned_canvas_snapshot();
    let a = Artifact::pinned_ask_canvas("t1", "m1", &snapshot).unwrap();

    assert_eq!(a.mime, "application/json");
    assert_eq!(a.name, "m1");
    assert!(!a.name.contains('.'));
    assert!(matches!(
        a.source,
        ArtifactSource::PinnedAskCanvas { ref thread_id, ref message_id }
            if thread_id == "t1" && message_id == "m1"
    ));

    let round_tripped: PinnedCanvasSnapshot = serde_json::from_str(&a.content).unwrap();
    assert_eq!(round_tripped, snapshot);
}

#[test]
fn artifact_mode_round_trip() {
    for (s, m) in [
        ("full", ArtifactMode::Full),
        ("summary_only", ArtifactMode::SummaryOnly),
        ("none", ArtifactMode::None),
    ] {
        assert_eq!(ArtifactMode::from_str_loose(s), m);
        assert_eq!(m.as_str(), s);
    }
}

#[test]
fn worktree_ref_envelope_is_valid_json() {
    let a = Artifact::worktree_ref("file::src/lib.rs", "local", "feature/slug", "src/lib.rs");
    let parsed: serde_json::Value = serde_json::from_str(&a.content).unwrap();
    assert_eq!(parsed["machine_id"], "local");
    assert_eq!(parsed["branch"], "feature/slug");
    assert_eq!(parsed["path"], "src/lib.rs");
    assert_eq!(a.mime, "application/x-demeteo-worktree-ref");
}

#[test]
fn tool_write_artifact_infers_mime_from_extension() {
    let md = Artifact::tool_write("spec", "docs/spec.md", "# Spec\n");
    assert_eq!(md.mime, "text/markdown");

    let rs = Artifact::tool_write("lib", "src/lib.rs", "// lib\n");
    assert_eq!(rs.mime, "text/x-rust");

    let diff = Artifact::tool_write("code-diff", "code.diff", "--- a\n+++ b\n");
    assert_eq!(diff.mime, "text/x-diff");

    let json = Artifact::tool_write("cfg", "config.json", "{}\n");
    assert_eq!(json.mime, "application/json");

    let plain = Artifact::tool_write("notes", "NOTES", "no extension");
    assert_eq!(plain.mime, "text/plain");

    let upper = Artifact::tool_write("spec", "Docs/SPEC.MD", "# S\n");
    assert_eq!(upper.mime, "text/markdown");

    assert!(matches!(md.source, ArtifactSource::ToolWrite { ref path } if path == "docs/spec.md"));
}

#[test]
fn mime_for_path_known_extensions() {
    assert_eq!(mime_for_path("foo.md"), "text/markdown");
    assert_eq!(mime_for_path("a/b/c.diff"), "text/x-diff");
    assert_eq!(mime_for_path("x/y/z.tsx"), "text/tsx");
    assert_eq!(mime_for_path("PY"), "text/plain");
    assert_eq!(mime_for_path(""), "text/plain");
}

#[test]
fn artifact_decl_serializes_with_tag() {
    let d = ArtifactDecl {
        name: "spec".into(),
        capture: ArtifactCapture::LastWriteTo {
            path: "docs/spec.md".into(),
        },
        mode: ArtifactMode::Full,
        inline: false,
    };
    let s = serde_json::to_string(&d).unwrap();
    let back: ArtifactDecl = serde_json::from_str(&s).unwrap();
    assert_eq!(back, d);
}
