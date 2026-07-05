use super::*;

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
