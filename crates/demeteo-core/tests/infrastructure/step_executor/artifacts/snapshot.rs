use super::*;

#[test]
fn test_parse_status_porcelain_basic() {
    let raw = "?? untracked.md\n";
    let set = parse_status_porcelain(raw);
    assert!(set.contains("untracked.md"));

    let raw_mod = " M modified.rs\n";
    let set = parse_status_porcelain(raw_mod);
    assert!(set.contains("modified.rs"));

    let raw_rename = "R  old.txt -> new.txt\n";
    let set = parse_status_porcelain(raw_rename);
    assert!(set.contains("new.txt"));
    assert!(!set.contains("old.txt"));

    // Branch info line is dropped
    let raw_branch = "## main...origin/main\n";
    let set = parse_status_porcelain(raw_branch);
    assert!(set.is_empty());
}

#[test]
fn test_parse_status_porcelain_dedup() {
    let raw = "?? dup.md\n?? dup.md\n";
    let set = parse_status_porcelain(raw);
    assert_eq!(set.len(), 1);
}

#[tokio::test]
async fn test_snapshot_delta_detects_new_files() {
    let temp = temp_git_repo("snapshot_delta_new");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    // Write & commit a baseline file so the repo isn't empty.
    exec.write_file(machine, &format!("{}/baseline.rs", temp), "fn main() {}")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
            shell_esc(&temp),
            shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    // Snapshot the clean repo.
    let snap = WorktreeSnapshot::capture(&exec, machine, &temp).await;
    assert!(snap.dirty.is_empty());

    // Simulate the agent writing a new file and modifying
    // baseline.rs.
    exec.write_file(machine, &format!("{}/new.md", temp), "# New\n")
        .await
        .unwrap();
    exec.write_file(machine, &format!("{}/baseline.rs", temp), "fn main(){}\n")
        .await
        .unwrap();

    // Delta with always_include empty: the new file should appear.
    let changed = snap.delta(&exec, machine, &temp, &[], &[]).await;
    assert!(
        changed.contains(&"new.md".to_string()),
        "expected new.md in delta, got {:?}",
        changed
    );
    // baseline.rs was clean *before* the step and is now modified.
    // `git status --porcelain` will report it as " M" so it's dirty now.
    assert!(
        changed.contains(&"baseline.rs".to_string()),
        "expected baseline.rs in delta (modified by step), got {:?}",
        changed
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_snapshot_delta_always_include() {
    let temp = temp_git_repo("snapshot_delta_always");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/base.md", temp), "# base\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
            shell_esc(&temp),
            shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    // Make base.md dirty before the step starts.
    exec.write_file(machine, &format!("{}/base.md", temp), "# dirty\n")
        .await
        .unwrap();
    let snap = WorktreeSnapshot::capture(&exec, machine, &temp).await;
    assert!(snap.dirty.contains("base.md"));

    // Step refines base.md further.
    exec.write_file(machine, &format!("{}/base.md", temp), "# final\n")
        .await
        .unwrap();

    // Without always_include, base.md is excluded because it was
    // already dirty at step start.
    let without = snap.delta(&exec, machine, &temp, &[], &[]).await;
    assert!(
        !without.contains(&"base.md".to_string()),
        "base.md should NOT appear without always_include, got {:?}",
        without
    );

    // With always_include = ["base.md"], it appears regardless.
    let with = snap.delta(&exec, machine, &temp, &["base.md"], &[]).await;
    assert!(
        with.contains(&"base.md".to_string()),
        "base.md should appear with always_include, got {:?}",
        with
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_snapshot_delta_excludes_scaffolding() {
    let temp = temp_git_repo("snapshot_exclude");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{}/base.md", temp), "# b\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
            shell_esc(&temp),
            shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    let snap = WorktreeSnapshot::capture(&exec, machine, &temp).await;

    // Write scaffolding files that the delta should filter.
    std::fs::create_dir_all(format!("{}/.git/tmp", temp)).unwrap();
    std::fs::write(format!("{}/.git/tmp/x", temp), "x").unwrap();
    std::fs::create_dir_all(format!("{}/.demeteo/data", temp)).unwrap();
    std::fs::write(format!("{}/.demeteo/data/y", temp), "y").unwrap();

    let changed = snap.delta(&exec, machine, &temp, &[], &[]).await;
    assert!(
        !changed.iter().any(|p| p.starts_with(".git")),
        "should exclude .git paths, got {:?}",
        changed
    );
    assert!(
        !changed.iter().any(|p| p.starts_with(".demeteo")),
        "should exclude .demeteo paths, got {:?}",
        changed
    );

    let _ = std::fs::remove_dir_all(&temp);
}

// ── worktree_is_missing ─────────────────────────────────────────────────────
//
// Driven against the real local adapter rather than a double: the question is
// whether the *port* answers it, and a double would only re-assert the mapping
// this file already writes. It is also the one case where "the answer is
// wrong" costs the step every write the agent made.

#[tokio::test]
async fn a_real_directory_is_not_missing() {
    let temp = temp_git_repo("worktree_present");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();

    assert!(!WorktreeSnapshot::worktree_is_missing(&exec, "local", &temp).await);

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn a_deleted_directory_is_missing() {
    let temp = temp_git_repo("worktree_deleted");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    std::fs::remove_dir_all(&temp).unwrap();

    assert!(WorktreeSnapshot::worktree_is_missing(&exec, "local", &temp).await);
}

/// A regular file where the worktree should be is not a worktree. `test -d`
/// answered this too, and the `is_dir` test is what keeps the answer once the
/// question is a metadata lookup that succeeds for files.
#[tokio::test]
async fn a_file_where_the_worktree_should_be_is_missing() {
    let temp = temp_git_repo("worktree_is_a_file");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let file = format!("{temp}/not-a-worktree");
    std::fs::write(&file, "").unwrap();

    assert!(WorktreeSnapshot::worktree_is_missing(&exec, "local", &file).await);

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
