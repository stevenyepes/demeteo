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
    let md = Artifact {
        name: "spec".into(),
        mime: "text/markdown".into(),
        content: "x".into(),
        source: ArtifactSource::AgentText,
    };
    let diff = Artifact {
        name: "diff".into(),
        mime: "application/x-diff".into(),
        content: "y".into(),
        source: ArtifactSource::Diff {
            base: "a".into(),
            head: "b".into(),
            path_filter: None,
        },
    };
    let r1 = store.put("f1", "s1", &md).unwrap();
    let r2 = store.put("f1", "s1", &diff).unwrap();
    assert!(r1.ends_with("spec.md"));
    assert!(r2.ends_with("diff.diff"));
}

#[test]
fn list_for_step_returns_insertion_order() {
    let (store, _dir) = temp_store();
    store
        .put("f1", "s1", &Artifact::agent_text("a", "1"))
        .unwrap();
    store
        .put("f1", "s1", &Artifact::agent_text("b", "2"))
        .unwrap();
    store
        .put("f1", "s2", &Artifact::agent_text("c", "3"))
        .unwrap();
    let mut s1 = store.list_for_step("f1", "s1").unwrap();
    s1.sort();
    assert_eq!(s1.len(), 2);
    let s2 = store.list_for_step("f1", "s2").unwrap();
    assert_eq!(s2.len(), 1);
}

#[test]
fn clear_step_removes_artifacts() {
    let (store, _dir) = temp_store();
    store
        .put("f1", "s1", &Artifact::agent_text("a", "1"))
        .unwrap();
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
    let listed = store.list_for_step("f1", "s1").unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].ends_with(".worktree-ref.json"));
}
