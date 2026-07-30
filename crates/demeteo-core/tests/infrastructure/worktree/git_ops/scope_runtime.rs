use super::super::common::*;
use crate::ports::execution::ExecutionPort;

#[tokio::test]
async fn test_scope_chmod_blocks_out_of_scope_writes() {
    let (dir, helper) = make_repo("scope_block").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Add a source file the agent must not touch.
    exec.write_file("local", &format!("{repo}/src/main.rs"), "fn main() {}")
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" add ."))
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" commit -m addsrc"))
        .await
        .unwrap();

    // Open a worktree at the existing HEAD so chmod operates on a real
    // working tree (the helper expects the dir to exist and be a worktree).
    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Apply scope: only `artifacts/report.md` is writable.
    let writable = vec![std::path::PathBuf::from("artifacts/report.md")];
    helper
        .apply_artifact_scope(None, &wt, &writable)
        .await
        .expect("scope setup should succeed");

    // 1. A write to `src/main.rs` should now fail (chmod a-w on src/).
    let bad_write = exec
        .write_file("local", &format!("{wt}/src/main.rs"), "hijacked")
        .await;
    assert!(
        bad_write.is_err(),
        "write to protected path should fail under scope fence"
    );

    // 2. A write to the allowed artifacts path should succeed.
    std::fs::create_dir_all(format!("{wt}/artifacts")).unwrap();
    let good_write = exec
        .write_file("local", &format!("{wt}/artifacts/report.md"), "# report")
        .await;
    assert!(
        good_write.is_ok(),
        "write to allowed artifacts path should succeed"
    );

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_diff_guard_reverts_out_of_scope_writes() {
    let (dir, helper) = make_repo("scope_revert").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Source file committed to HEAD.
    exec.write_file("local", &format!("{repo}/src/lib.rs"), "pub fn ok() {}")
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" add ."))
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" commit -m addsrc"))
        .await
        .unwrap();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Simulate the agent bypassing the chmod fence (e.g. via
    // `chmod u+w .` shell escape) and writing to a protected path.
    std::fs::create_dir_all(format!("{wt}/src")).unwrap();
    std::fs::write(
        format!("{wt}/src/lib.rs"),
        "pub fn hijacked() {} // agent ran chmod u+w and modified me",
    )
    .unwrap();
    // And an untracked file too.
    std::fs::write(format!("{wt}/src/new_file.rs"), "evil").unwrap();

    // Run the diff guard. Writable set: only `artifacts/`.
    let writable = vec![std::path::PathBuf::from("artifacts")];
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .expect("diff guard should succeed");

    // Both writes should be reported as reverted.
    assert!(
        reverted.iter().any(|p| p == "src/lib.rs"),
        "expected src/lib.rs in reverted list, got {:?}",
        reverted
    );
    assert!(
        reverted.iter().any(|p| p == "src/new_file.rs"),
        "expected src/new_file.rs in reverted list, got {:?}",
        reverted
    );

    // The tracked file should be back to its committed content.
    let restored = std::fs::read_to_string(format!("{wt}/src/lib.rs")).unwrap();
    assert_eq!(restored, "pub fn ok() {}");

    // The untracked file should be gone.
    assert!(!std::path::Path::new(&format!("{wt}/src/new_file.rs")).exists());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_diff_guard_keeps_in_scope_writes() {
    let (dir, helper) = make_repo("scope_keep").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Agent writes the report inside the allowed scope.
    std::fs::create_dir_all(format!("{wt}/artifacts")).unwrap();
    std::fs::write(
        format!("{wt}/artifacts/research-report.md"),
        "# Research Report\n",
    )
    .unwrap();

    // Diff guard runs with the allowed scope. Should report nothing
    // reverted and leave the file in place.
    let writable = vec![std::path::PathBuf::from("artifacts/research-report.md")];
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .expect("diff guard should succeed");
    assert!(
        reverted.is_empty(),
        "in-scope write should not be reverted; got {:?}",
        reverted
    );

    let content = std::fs::read_to_string(format!("{wt}/artifacts/research-report.md")).unwrap();
    assert_eq!(content, "# Research Report\n");

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_all_writes_sentinel_disables_enforcement() {
    let (dir, helper) = make_repo("scope_off").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // `s-implement`'s parallel capture is AllWrites → sentinel returned.
    let writable = vec![std::path::PathBuf::from("__ALL_WRITES__")];

    // chmod fence is a no-op: file remains writable.
    helper
        .apply_artifact_scope(None, &wt, &writable)
        .await
        .unwrap();
    std::fs::create_dir_all(format!("{wt}/src")).unwrap();
    assert!(
        exec.write_file("local", &format!("{wt}/src/main.rs"), "ok")
            .await
            .is_ok(),
        "AllWrites sentinel must leave the worktree fully writable"
    );

    // Diff guard is a no-op: writes are not reverted.
    std::fs::write(format!("{wt}/src/whatever.rs"), "fine").unwrap();
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .unwrap();
    assert!(reverted.is_empty());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression: the fence must never chmod *through* a symlink.
///
/// A worktree's `node_modules` is a symlink into the feature's shared
/// dependency cache (`link_dependency_caches_cmd`), and `chmod` follows the
/// symlink it is handed on the command line — there is no `lchmod` on Linux,
/// so `chmod -R a-w <wt>/node_modules` cannot make the *link* read-only and
/// instead makes the cache's whole tree read-only. That cache is shared by
/// every worktree of the feature and outlives this step, so one
/// `ArtifactsOnly` verify step disables `npm`/`vite` for every step after it.
///
/// The damage is also one-way: step 1's `chmod -R u+w <wt>` cannot undo it,
/// because `-R` does not follow symlinks encountered *during* traversal. So
/// the next step provisions a clean worktree, relinks it to the same poisoned
/// cache, and fails identically — which is how feature `f-1785431165068` spent
/// its retry budget redirecting to `s-fix` over a permission bit.
#[tokio::test]
async fn test_scope_fence_does_not_chmod_through_dependency_cache_symlink() {
    let (dir, helper) = make_repo("scope_symlink").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // The feature's shared cache, seeded once and symlinked into every
    // worktree of the feature.
    let cache = format!("{}_cache_feat", repo);
    std::fs::create_dir_all(format!("{cache}/node_modules/.vite-temp")).unwrap();
    exec.run_command(
        "local",
        &format!("ln -sfn \"{cache}/node_modules\" \"{wt}/node_modules\""),
    )
    .await
    .unwrap();

    // A `verify`-capability step on a project declaring no extra writable
    // paths: writable is `artifacts/` alone, so `node_modules` is protected.
    let writable = crate::adapters::worktree::git_ops::scope::derive_writable_paths_for_scope(
        crate::domain::permission::WriteScope::ArtifactsOnly,
        None,
        &[],
    );
    helper
        .apply_artifact_scope(None, &wt, &writable)
        .await
        .expect("scope setup should succeed");

    // Vite writes its bundled config here on every `vite`/`vitest` start.
    let probe = format!("{cache}/node_modules/.vite-temp/vite.config.ts.timestamp-1.mjs");
    let wrote = exec.write_file("local", &probe, "export default {}").await;

    // Restore before asserting so a failure still cleans up after itself.
    let _ = exec
        .run_command("local", &format!("chmod -R u+w \"{cache}\" \"{wt}\""))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&wt);

    assert!(
        wrote.is_ok(),
        "fence chmod'd through the node_modules symlink into the feature's \
         shared dependency cache, disabling it for every later step"
    );
}
