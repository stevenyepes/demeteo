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
        inline: false,
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
        inline: false,
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
            inline: false,
        },
        ArtifactDecl {
            name: "wt-ref".into(),
            capture: ArtifactCapture::Worktree { path: None },
            mode: crate::domain::artifact::ArtifactMode::None,
            inline: false,
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
        &[],
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
async fn test_commit_worktree_changes_warns_when_agent_writes_only_land_under_artifacts() {
    // Repro of the docs-update bug: the agent writes the *real* doc
    // body into `artifacts/s-draft.md` instead of the real path. With
    // `commit_artifacts=true` the report is allowed onto the branch
    // (legacy behaviour), but the user's actual deliverable is
    // stranded as the body of the summary report and never lands at
    // `docs/new.md`. The guard log's "stage only contains artifact
    // paths while the agent reported writes outside" branch fires.
    //
    // Verifying the warn is captured in stderr would require a
    // tracing subscriber; we verify the observable side effects
    // instead: the commit contains the (stranded) report, the real
    // path is still absent, and the function still succeeds.
    let temp = temp_git_repo("commit_worktree_stranded");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/base.md", temp), "# base\n")
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

    // Agent "strands" the real doc body under artifacts/.
    exec.write_file(
        machine,
        &format!("{}/artifacts/s-draft.md", temp),
        "# Real doc body that should have been at docs/new.md\n",
    )
    .await
    .unwrap();

    let non_artifact_writes = vec!["docs/new.md".to_string()];

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "docs: draft",
        "artifacts/",
        // commit_artifacts=true so we can observe what landed on
        // the branch (the stranded artifact report). The default
        // (`false`) would have produced an empty commit and a
        // different guard-log branch — covered by
        // `test_commit_worktree_changes_happy_path_…` below.
        true,
        &non_artifact_writes,
    )
    .await
    .unwrap();
    assert!(!sha.is_empty());

    let committed_files = exec
        .run_command(
            machine,
            &format!(
                "git -C \"{}\" show --name-only --pretty=format: {}",
                temp, sha
            ),
        )
        .await
        .unwrap();
    assert!(
        committed_files.contains("artifacts/s-draft.md"),
        "expected artifacts/s-draft.md in commit (commit_artifacts=true), got: {committed_files}"
    );
    assert!(
        !committed_files.contains("docs/new.md"),
        "the doc body should NOT be in the commit (it was stranded under artifacts/), got: {committed_files}"
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_commit_worktree_changes_warns_when_stage_is_empty_despite_non_artifact_writes() {
    // The agent reports it wrote `docs/new.md` but the file is not in
    // the worktree (e.g. capability-scoped out, or the agent's write
    // was reverted by the post-step guard). The stage ends up empty
    // even though `non_artifact_writes` is non-empty — the guard's
    // "stage is empty but the agent reported writes outside" branch
    // fires. `--allow-empty` keeps the commit reachable for downstream
    // gating; the warning is the only signal.
    let temp = temp_git_repo("commit_worktree_empty_stage");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/base.md", temp), "# base\n")
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

    let non_artifact_writes = vec!["docs/new.md".to_string()];

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "docs: missing deliverable",
        "artifacts/",
        false,
        &non_artifact_writes,
    )
    .await
    .unwrap();
    assert!(!sha.is_empty());

    // The commit should be empty (the message + no file changes).
    // `git show --name-only --pretty=format:` against an empty
    // commit produces a blank body — we assert that no tracked
    // path was added.
    let committed_files = exec
        .run_command(
            machine,
            &format!(
                "git -C \"{}\" show --name-only --pretty=format: {}",
                temp, sha
            ),
        )
        .await
        .unwrap();
    assert!(
        !committed_files.contains("docs/new.md"),
        "the doc body should NOT be in the commit (the agent never wrote it to disk), got: {committed_files}"
    );
    assert!(
        committed_files.trim().is_empty(),
        "expected empty commit (no files added), got: {committed_files:?}"
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_commit_worktree_changes_happy_path_when_non_artifact_write_lands_in_stage() {
    // The agent writes a new doc at its real path. With
    // `commit_artifacts=false` the new doc lands in the stage, no warn.
    // This is the regression-prevention anchor: if the new doc body ever
    // disappears from the stage again, this test will fail (the commit
    // will be empty) and the guard warn will have fired too.
    let temp = temp_git_repo("commit_worktree_happy");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/base.md", temp), "# base\n")
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

    // Real path, not under artifacts/.
    exec.write_file(machine, &format!("{}/docs/new.md", temp), "# Real\n")
        .await
        .unwrap();
    // Plus the summary report the orchestrator captures under
    // artifacts/ — that's the one that should stay out of the commit.
    exec.write_file(
        machine,
        &format!("{}/artifacts/s-draft.md", temp),
        "summary",
    )
    .await
    .unwrap();

    let non_artifact_writes = vec!["docs/new.md".to_string()];

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "docs: new",
        "artifacts/",
        false,
        &non_artifact_writes,
    )
    .await
    .unwrap();
    assert!(!sha.is_empty());

    let committed_files = exec
        .run_command(
            machine,
            &format!(
                "git -C \"{}\" show --name-only --pretty=format: {}",
                temp, sha
            ),
        )
        .await
        .unwrap();
    assert!(
        committed_files.contains("docs/new.md"),
        "expected docs/new.md in commit, got: {committed_files}"
    );
    assert!(
        !committed_files.contains("artifacts/s-draft.md"),
        "artifacts/s-draft.md should NOT be in the commit (commit_artifacts=false), got: {committed_files}"
    );

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
