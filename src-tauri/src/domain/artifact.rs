//! Artifact domain model — the durable per-step record.
//!
//! Replaces the ad-hoc `step_executions.artifact_path: Option<String>` model
//! (which conflated the agent's chat stream with the actual produced
//! artifact) with a typed, first-class value object that can describe
//! derived content (diffs, worktree pointers) and inline agent output
//! uniformly.
//!
//! See `docs/REDESIGN_DDD_MODEL.md` for the bounded context and
//! `AGENT_INTEGRATION.md` §3.4 for the AgentEvent side of this contract.

use serde::{Deserialize, Serialize};

/// How much of the artifact the executor should persist and how the
/// next step should consume it. Mirrors the per-step `artifact_mode`
/// in the workflow JSON (locked decision 28 in
/// `docs/REDESIGN_DECISIONS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMode {
    /// Persist the full artifact body to `ArtifactStore` and inject
    /// it verbatim into the next step's prompt where referenced.
    #[default]
    Full,
    /// Persist a short summary only (first N lines, or extracted
    /// headers for structured content). The next step still sees the
    /// summary; the full content is on disk for the user.
    SummaryOnly,
    /// Do not persist. Useful for navigation pointers (`WorktreeRef`)
    /// where the artifact is just an "open in editor" CTA.
    None,
}

impl ArtifactMode {
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "full" => Self::Full,
            "summary_only" => Self::SummaryOnly,
            "none" => Self::None,
            _ => Self::Full,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::SummaryOnly => "summary_only",
            Self::None => "none",
        }
    }
}

/// What the artifact *is* and where it came from. The discriminator
/// lets the frontend (`ArtifactViewer.tsx`) dispatch on `mime` and the
/// backend (`ArtifactStore`) decide whether to read from the worktree
/// at materialization time or just persist the supplied content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactSource {
    /// Agent wrote a file via a `write`/`edit` tool call. `path` is
    /// relative to the step's worktree root. The `ArtifactStore` (or
    /// the `FsArtifactStore` adapter) reads the file from disk at
    /// materialization time.
    ToolWrite { path: String },
    /// Agent emitted the artifact as a final reply (e.g. a JSON
    /// manifest, a typed analysis, a structured summary). The content
    /// is the literal reply text, post-streaming.
    AgentText,
    /// Synthesized at materialization time: a unified diff between two
    /// git refs. `base` and `head` are branch / SHA / refspec strings
    /// resolvable by `git rev-parse`. `path_filter` optionally restricts
    /// the diff to a single path (e.g. `"src/lib.rs"`).
    Diff {
        base: String,
        head: String,
        path_filter: Option<String>,
    },
    /// Navigation pointer, not content. `mime` is
    /// `application/x-demeteo-worktree-ref` and the frontend renders an
    /// "Open in editor" CTA that deep-links into the existing SFTP +
    /// Monaco file view for this branch + path.
    WorktreeRef {
        machine_id: String,
        branch: String,
        path: String,
    },
    /// Reserved for future kinds (e.g. MR URLs from `MrPublisher`,
    /// JUnit XML from a test runner, screenshots from a browser tool).
    /// The `ref_` is opaque to the executor; `ArtifactStore.put` is
    /// the only thing that interprets it.
    External { ref_: String },
}

/// A single, typed artifact produced (or derived) by a step.
///
/// The `name` is the durable identifier within the step. The `mime`
/// is what the frontend dispatches on. The `content` is the post-write
/// content for `AgentText`, the file body for `ToolWrite` (read back
/// at materialization time), or a small JSON envelope for `WorktreeRef`
/// (the structured payload the UI needs to render the CTA).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Logical name within the step. The orchestrator stores artifacts
    /// under `<app_local_data_dir>/artifacts/<feature_id>/<step_id>/<name>.<ext>`
    /// and the executor names files by `<name>` + an extension inferred
    /// from `mime`.
    pub name: String,
    /// IANA media type. Examples used in this codebase:
    /// `text/markdown`, `text/x-diff`, `application/json`,
    /// `application/x-junit+xml`, `application/x-demeteo-worktree-ref`.
    pub mime: String,
    /// For `ToolWrite` the post-write file content; for `AgentText` the
    /// full agent reply; for `WorktreeRef` a small JSON envelope
    /// `{machine_id, branch, path}`; for `Diff` the unified diff body.
    pub content: String,
    pub source: ArtifactSource,
}

impl Artifact {
    pub fn tool_write(name: impl Into<String>, path: impl Into<String>, content: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            name: name.into(),
            mime: mime_for_path(&path),
            content: content.into(),
            source: ArtifactSource::ToolWrite { path },
        }
    }

    pub fn agent_text(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mime: "text/markdown".into(),
            content: content.into(),
            source: ArtifactSource::AgentText,
        }
    }

    pub fn worktree_ref(name: impl Into<String>, machine_id: impl Into<String>, branch: impl Into<String>, path: impl Into<String>) -> Self {
        let machine_id = machine_id.into();
        let branch = branch.into();
        let path = path.into();
        let content = serde_json::json!({
            "machine_id": machine_id,
            "branch": branch,
            "path": path,
        })
        .to_string();
        Self {
            name: name.into(),
            mime: "application/x-demeteo-worktree-ref".into(),
            content,
            source: ArtifactSource::WorktreeRef { machine_id, branch, path },
        }
    }
}

/// Best-effort IANA mime for a worktree-relative path, derived from the
/// file extension. Mirrors `FsArtifactStore::ext_for_mime` in the inverse
/// direction so the on-disk file extension matches the stored mime.
///
/// New mimes should be added to *both* this table and
/// `FsArtifactStore::ext_for_mime` so the pair stays in sync.
pub fn mime_for_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if let Some((_, ext)) = lower.rsplit_once('.') {
        match ext {
            "md" | "markdown" => return "text/markdown".into(),
            "diff" | "patch" => return "text/x-diff".into(),
            "json" => return "application/json".into(),
            "txt" => return "text/plain".into(),
            "html" | "htm" => return "text/html".into(),
            "css" => return "text/css".into(),
            "csv" => return "text/csv".into(),
            "xml" => return "application/xml".into(),
            "ts" => return "text/typescript".into(),
            "tsx" => return "text/tsx".into(),
            "js" | "jsx" | "mjs" | "cjs" => return "text/javascript".into(),
            "py" => return "text/x-python".into(),
            "rb" => return "text/x-ruby".into(),
            "rs" => return "text/x-rust".into(),
            "go" => return "text/x-go".into(),
            "java" => return "text/x-java".into(),
            "kt" | "kts" => return "text/x-kotlin".into(),
            "swift" => return "text/x-swift".into(),
            "c" | "h" => return "text/x-c".into(),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => return "text/x-c++".into(),
            "sh" | "bash" | "zsh" => return "text/x-shellscript".into(),
            "yaml" | "yml" => return "text/yaml".into(),
            "toml" => return "text/x-toml".into(),
            "sql" => return "text/x-sql".into(),
            "vue" => return "text/x-vue".into(),
            "svelte" => return "text/x-svelte".into(),
            "lock" => return "application/x-demeteo-skip".into(),
            _ => {}
        }
    }
    "text/plain".into()
}

/// What to capture for a single declared artifact. The `name` on the
/// outer `ArtifactDecl` is what the rest of the codebase sees; this
/// enum is the *how*.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactCapture {
    /// Pull an `AgentEvent::ArtifactProduced` whose `artifact.name` matches.
    ByName { name: String },
    /// Take the last `ToolWrite` artifact whose `path` matches this glob
    /// (relative to the step worktree). E.g. `"docs/spec.md"`,
    /// `"**/*.test-report.md"`. If no match, the capture is dropped
    /// and a warning is logged.
    LastWriteTo { path: String },
    /// Every `ToolWrite` artifact the step produced. For parallel /
    /// implement steps that fan out many files. Each becomes its own
    /// artifact reference; last-write-wins is per-name, not per-path.
    AllWrites,
    /// Detect all files changed since `base_ref` via `git diff --name-only`.
    /// Artifacts are named from file basenames. `base` describes the left side
    /// (see [`DiffBase`]). `path_filter` optionally restricts to a glob pattern.
    ChangedFiles { base: DiffBase, path_filter: Option<String> },
    /// Synthesize a unified diff at materialization time. `base`
    /// describes the left side; `head` is the step's worktree HEAD
    /// unless overridden.
    Diff { base: DiffBase, path_filter: Option<String> },
    /// Emit one `WorktreeRef` artifact per file matched by `path`
    /// (or one for the worktree root if `path` is `None`). Stored in
    /// `artifact_paths`; `mode: None` is the typical choice since
    /// these are navigation pointers, not content.
    Worktree { path: Option<String> },
}

/// What the left side of a `Diff` capture refers to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffBase {
    /// The branch this step's worktree was forked from (i.e. the
    /// merge-base of the worktree's branch and the project default
    /// branch, or `main` if that's not resolvable).
    WorktreeBase,
    /// A specific ref / SHA resolvable by `git rev-parse`.
    Ref { value: String },
    /// The previous completed step's worktree tip — for sequential
    /// refinement (rebase, fixup) steps.
    PreviousStep,
}

/// One declared artifact. The `StepConfig.artifacts` list is the
/// per-step contract: what the step promises to produce, where to
/// find it, and how much to persist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactDecl {
    /// Logical name. The `ArtifactStore` writes the file as
    /// `<name>.<ext>` where `<ext>` is inferred from `mime`.
    pub name: String,
    /// How to capture. See [`ArtifactCapture`].
    pub capture: ArtifactCapture,
    /// How much to persist. See [`ArtifactMode`].
    pub mode: ArtifactMode,
}

impl ArtifactDecl {
    /// Last-write-to a specific path under the worktree, full content.
    /// The most common declaration for text-producing agent steps.
    pub fn full_path(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            capture: ArtifactCapture::LastWriteTo { path: path.into() },
            mode: ArtifactMode::Full,
        }
    }
}

#[cfg(test)]
mod tests {
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
            capture: ArtifactCapture::LastWriteTo { path: "docs/spec.md".into() },
            mode: ArtifactMode::Full,
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: ArtifactDecl = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }
}
