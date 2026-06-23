use super::*;

#[test]
fn test_resolve_declared_artifacts_by_name() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_resolve_name_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let declarations = vec![ArtifactDecl::full_path("spec", "docs/spec.md")];
    let produced = vec![Artifact::tool_write("spec", "docs/spec.md", "# My Spec\n")];

    let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

    assert_eq!(refs.len(), 1);
    assert!(refs[0].contains("artifacts/f-test/s-impl/spec"));
    let content = store.get(&refs[0]).unwrap();
    assert_eq!(content, "# My Spec\n");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_resolve_declared_artifacts_last_write() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_resolve_last_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let declarations = vec![ArtifactDecl {
        name: "final-spec".into(),
        capture: ArtifactCapture::LastWriteTo {
            path: "docs/spec.md".into(),
        },
        mode: crate::domain::artifact::ArtifactMode::Full,
    }];

    let produced = vec![
        Artifact::tool_write("draft", "docs/spec.md", "# Draft\n"),
        Artifact::tool_write("final", "docs/spec.md", "# Final\n"),
    ];

    let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

    assert_eq!(refs.len(), 1);
    let content = store.get(&refs[0]).unwrap();
    assert_eq!(content, "# Final\n");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_resolve_declared_artifacts_all_writes() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_resolve_all_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let declarations = vec![ArtifactDecl {
        name: "all-files".into(),
        capture: ArtifactCapture::AllWrites,
        mode: crate::domain::artifact::ArtifactMode::Full,
    }];

    let produced = vec![
        Artifact::tool_write("f1", "src/lib.rs", "// lib\n"),
        Artifact::tool_write("f2", "src/main.rs", "// main\n"),
        Artifact::tool_write("f1-v2", "src/lib.rs", "// lib v2\n"),
    ];

    let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

    assert_eq!(refs.len(), 2);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_resolve_declared_artifacts_skips_diff_and_worktree() {
    use crate::domain::artifact::DiffBase;
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_resolve_skip_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let store: Arc<dyn ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );

    let declarations = vec![
        ArtifactDecl {
            name: "code-diff".into(),
            capture: ArtifactCapture::Diff {
                base: DiffBase::WorktreeBase,
                path_filter: None,
            },
            mode: crate::domain::artifact::ArtifactMode::Full,
        },
        ArtifactDecl {
            name: "wt-ref".into(),
            capture: ArtifactCapture::Worktree { path: None },
            mode: crate::domain::artifact::ArtifactMode::None,
        },
    ];

    let refs = resolve_declared_artifacts(&declarations, &[], &store, "f-test", "s-impl");

    assert!(refs.is_empty());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn test_commit_worktree_changes() {
    let temp = temp_git_repo("commit_worktree");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/src.rs", temp), "fn a() {}\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m base",
            shell_esc(&temp),
            shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    exec.write_file(machine, &format!("{}/src.rs", temp), "fn b() {}\n")
        .await
        .unwrap();
    exec.write_file(machine, &format!("{}/new.md", temp), "# Added\n")
        .await
        .unwrap();

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "worker: subtask-1",
        "artifacts/",
        true,
    )
    .await
    .unwrap();
    assert!(!sha.is_empty());

    let log = exec
        .run_command(
            machine,
            &format!("git -C {} log --oneline -1", shell_esc(&temp)),
        )
        .await
        .unwrap();
    assert!(log.contains("worker: subtask-1"));

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_compute_git_diff() {
    let temp = temp_git_repo("compute_diff");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/src.rs", temp), "fn init() {}\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m base",
            shell_esc(&temp),
            shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    let base_sha = exec
        .run_command(
            machine,
            &format!("git -C {} rev-parse HEAD", shell_esc(&temp)),
        )
        .await
        .unwrap()
        .trim()
        .to_string();

    exec.write_file(machine, &format!("{}/src.rs", temp), "fn new() {}\n")
        .await
        .unwrap();

    let diff = compute_git_diff(&exec, machine, &temp, &base_sha).await;
    assert!(!diff.is_empty());
    assert!(diff.contains("fn init()"));
    assert!(diff.contains("fn new()"));

    let diff_head = compute_git_diff(&exec, machine, &temp, "HEAD").await;
    assert!(!diff_head.is_empty());

    let diff_none = compute_git_diff(&exec, machine, &temp, "no-such-ref").await;
    assert!(diff_none.is_empty());

    let _ = std::fs::remove_dir_all(&temp);
}

fn temp_git_repo(label: &str) -> String {
    let d = std::env::temp_dir().join(format!(
        "demeteo_test_{}_{}",
        label,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path = d.to_string_lossy().to_string();
    let cmd = format!("git init -b main {}", shell_esc(&path));
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output();
    path
}

fn shell_esc(s: &str) -> String {
    crate::paths::shell_escape_posix(s)
}
