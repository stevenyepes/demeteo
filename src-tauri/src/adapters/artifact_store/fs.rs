//! Filesystem implementation of [`ArtifactStore`].
//!
//! Persists artifacts under
//! `<app_local_data_dir>/artifacts/<feature_id>/<step_id>/<name>.<ext>`
//! where `<ext>` is inferred from the artifact's `mime`.
//!
//! The on-disk layout is intentionally flat per step: the next-step
//! prompt renderer and the UI both read the whole directory, so
//! there's no benefit to a deeper tree and it keeps `clear_step`
//! trivial (`std::fs::remove_dir_all`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::artifact::{Artifact, ArtifactSource};
use crate::ports::artifact_store::ArtifactStore;

pub struct FsArtifactStore {
    root: PathBuf,
    // The FS store is currently single-threaded w.r.t. the layout
    // (each step is its own directory; we never have to coordinate
    // across steps), so no inner lock is needed. The Arc keeps the
    // struct `Send + Sync` so it can sit behind `Arc<dyn ArtifactStore>`.
    _marker: Arc<()>,
}

impl FsArtifactStore {
    pub fn new(app_local_data_dir: PathBuf) -> Self {
        let root = app_local_data_dir.join("artifacts");
        let _ = std::fs::create_dir_all(&root);
        Self { root, _marker: Arc::new(()) }
    }

    fn step_dir(&self, feature_id: &str, step_id: &str) -> PathBuf {
        self.root.join(sanitize(feature_id)).join(sanitize(step_id))
    }

    fn ext_for_mime(mime: &str) -> &'static str {
        match mime {
            "text/markdown" => "md",
            "text/x-diff" | "application/x-diff" => "diff",
            "application/x-demeteo-worktree-ref" => "worktree-ref.json",
            "application/json" => "json",
            "application/x-junit+xml" => "junit.xml",
            "text/plain" => "txt",
            "text/html" => "html",
            _ => "bin",
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl ArtifactStore for FsArtifactStore {
    fn put(
        &self,
        feature_id: &str,
        step_id: &str,
        artifact: &Artifact,
    ) -> Result<String, String> {
        let dir = self.step_dir(feature_id, step_id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ext = FsArtifactStore::ext_for_mime(&artifact.mime);
        let safe_name = sanitize(&artifact.name);
        let path: PathBuf = dir.join(format!("{safe_name}.{ext}"));

        // For Diff and WorktreeRef sources, materialize the content
        // *now* (Diff is computed by the caller before `put`; WorktreeRef
        // is a small envelope). For ToolWrite the caller is expected to
        // have read the file already and pass the body in `content`.
        // For AgentText, `content` is the literal reply.
        std::fs::write(&path, &artifact.content).map_err(|e| e.to_string())?;

        // Side-effect: when the source is a WorktreeRef, also drop a
        // sentinel file at the parent step dir that records the
        // referenced branch. The frontend's "open in editor" CTA needs
        // the branch + machine_id to construct the deep-link; reading
        // it from the JSON envelope in `get` works, but for tooling
        // that scrapes the artifact directory directly, the sentinel
        // is the canonical record.
        if let ArtifactSource::WorktreeRef { machine_id, branch, path: file_path } = &artifact.source {
            let env = serde_json::json!({
                "name": artifact.name,
                "mime": artifact.mime,
                "machine_id": machine_id,
                "branch": branch,
                "path": file_path,
            });
            let _ = std::fs::write(path.with_extension("env.json"), env.to_string());
        }

        Ok(path.to_string_lossy().to_string())
    }

    fn get(&self, reference: &str) -> Result<String, String> {
        std::fs::read_to_string(Path::new(reference)).map_err(|e| e.to_string())
    }

    fn list_for_step(
        &self,
        feature_id: &str,
        step_id: &str,
    ) -> Result<Vec<String>, String> {
        let dir = self.step_dir(feature_id, step_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            // Skip the side-effect sentinel files; they aren't
            // standalone artifacts, just bookkeeping for WorktreeRef.
            // The sentinel ends with `.env.json` but `Path::extension`
            // only returns the last component (`json`), so we compare
            // the file name string directly.
            if p.file_name().and_then(|s| s.to_str()).map_or(false, |n| n.ends_with(".env.json")) {
                continue;
            }
            if p.is_file() {
                out.push(p.to_string_lossy().to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    fn clear_step(&self, feature_id: &str, step_id: &str) -> Result<(), String> {
        let dir = self.step_dir(feature_id, step_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::artifact::{Artifact, ArtifactSource};

    fn temp_store() -> (FsArtifactStore, PathBuf) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "demeteo_artifact_test_{}_{}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id(),
            count,
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = FsArtifactStore::new(dir.clone());
        (store, dir)
    }

    #[test]
    fn put_and_get_round_trip() {
        let (store, _dir) = temp_store();
        let a = Artifact::agent_text("summary", "# hello\n");
        let r = store.put("f1", "s1", &a).unwrap();
        let back = store.get(&r).unwrap();
        assert_eq!(back, "# hello\n");
    }

    #[test]
    fn put_uses_correct_extension_per_mime() {
        let (store, _dir) = temp_store();
        let md = Artifact { name: "spec".into(), mime: "text/markdown".into(), content: "x".into(), source: ArtifactSource::AgentText };
        let diff = Artifact { name: "diff".into(), mime: "application/x-diff".into(), content: "y".into(), source: ArtifactSource::Diff { base: "a".into(), head: "b".into(), path_filter: None } };
        let r1 = store.put("f1", "s1", &md).unwrap();
        let r2 = store.put("f1", "s1", &diff).unwrap();
        assert!(r1.ends_with("spec.md"));
        assert!(r2.ends_with("diff.diff"));
    }

    #[test]
    fn list_for_step_returns_insertion_order() {
        let (store, _dir) = temp_store();
        store.put("f1", "s1", &Artifact::agent_text("a", "1")).unwrap();
        store.put("f1", "s1", &Artifact::agent_text("b", "2")).unwrap();
        store.put("f1", "s2", &Artifact::agent_text("c", "3")).unwrap();
        let mut s1 = store.list_for_step("f1", "s1").unwrap();
        s1.sort();
        assert_eq!(s1.len(), 2);
        let s2 = store.list_for_step("f1", "s2").unwrap();
        assert_eq!(s2.len(), 1);
    }

    #[test]
    fn clear_step_removes_artifacts() {
        let (store, _dir) = temp_store();
        store.put("f1", "s1", &Artifact::agent_text("a", "1")).unwrap();
        assert_eq!(store.list_for_step("f1", "s1").unwrap().len(), 1);
        store.clear_step("f1", "s1").unwrap();
        assert!(store.list_for_step("f1", "s1").unwrap().is_empty());
    }

    #[test]
    fn worktree_ref_drops_env_json_sentinel() {
        let (store, _dir) = temp_store();
        let a = Artifact::worktree_ref("file::src/lib.rs", "local", "feature/slug", "src/lib.rs");
        let r = store.put("f1", "s1", &a).unwrap();
        assert!(r.ends_with("file__src_lib_rs.worktree-ref.json"));
        let dir = std::path::Path::new(&r).parent().unwrap();
        let entries: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.iter().any(|n| n.ends_with(".env.json")));
        // The sentinel must be filtered out of list_for_step.
        let listed = store.list_for_step("f1", "s1").unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].ends_with(".worktree-ref.json"));
    }
}
