//! Terminal-worktree parity gate: the single POSIX script
//! [`WorktreeOpsPort::create_terminal_worktree`] sends through `ExecutionPort`,
//! asserted by one function against the local subprocess adapter and against a
//! real loopback sshd.
//!
//! A recording `ExecutionPort` double proves a string was *built*; it cannot
//! prove two transports do the same thing with it. The bug that motivates this
//! gate had exactly that shape — one prune, `read_dir` locally and a
//! `"$dir"/*` glob remotely, agreeing on every string and disagreeing on
//! dot-entries. The rationale for where the area now lives is on
//! `terminal_worktree_area` in
//! `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs`.
//!
//! Every observation is made through the same `ExecutionPort` the
//! implementation used, never through `std::fs`: reading the desktop host's
//! filesystem is vacuous on the SSH leg, and an assertion that branched on
//! `machine_id` would let the transports disagree by construction.

use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::paths::{shell_escape_posix, REPOS_SUBDIR, TERMINAL_WORKTREES_SUBDIR};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{TerminalWorktreeRequest, WorktreeOpsPort};
use rusqlite::Connection;
use std::sync::Arc;

/// A repository name carrying a space, so the escaping the adapter applies is
/// exercised through a transport that adds its own quoting layer rather than
/// only through a double that records the string.
const REPO_NAME: &str = "sample repo";

/// A request naming no base, so the start point stays the primary checkout's
/// HEAD. The base-branch leg builds its own.
fn terminal_request(branch: &str, worktree_name: &str) -> TerminalWorktreeRequest {
    TerminalWorktreeRequest {
        branch: branch.to_string(),
        base_branch: None,
        worktree_name: worktree_name.to_string(),
    }
}

async fn sh(port: &Arc<dyn ExecutionPort>, machine_id: &str, command: &str) -> String {
    port.run_command(machine_id, command)
        .await
        .unwrap_or_else(|e| panic!("conformance setup command failed: {command}\n{e}"))
}

/// `ls -A` rather than `list_dir`, and `LC_ALL=C sort` rather than the shell's
/// collation: dot-entries and ordering are the two things the prune divergence
/// turned on, so neither may be left to the transport's discretion.
async fn entries(port: &Arc<dyn ExecutionPort>, machine_id: &str, dir: &str) -> Vec<String> {
    sh(
        port,
        machine_id,
        &format!("ls -A {} | LC_ALL=C sort", shell_escape_posix(dir)),
    )
    .await
    .lines()
    .map(str::to_string)
    .collect()
}

async fn rev(
    port: &Arc<dyn ExecutionPort>,
    machine_id: &str,
    repo: &str,
    reference: &str,
) -> Result<String, String> {
    port.run_command(
        machine_id,
        &format!(
            "git -C {} rev-parse --verify --quiet {}",
            shell_escape_posix(repo),
            shell_escape_posix(reference)
        ),
    )
    .await
    .map(|sha| sha.trim().to_string())
}

async fn exists(port: &Arc<dyn ExecutionPort>, machine_id: &str, path: &str) -> bool {
    port.run_command(machine_id, &format!("[ -e {} ]", shell_escape_posix(path)))
        .await
        .is_ok()
}

/// Lay down `<base>/project/repos/<REPO_NAME>` as a one-commit git repository on
/// the target, and hand back the project root and repository directory.
async fn make_project_repo(
    port: &Arc<dyn ExecutionPort>,
    machine_id: &str,
    base: &str,
) -> (String, String) {
    let project_root = format!("{base}/project");
    let repo = format!("{project_root}/{REPOS_SUBDIR}/{REPO_NAME}");
    let safe_repo = shell_escape_posix(&repo);

    sh(
        port,
        machine_id,
        &format!(
            "set -eu; rm -rf {base}; mkdir -p {safe_repo}; \
             git -C {safe_repo} init -q -b main; \
             git -C {safe_repo} config user.email ci@demeteo.test; \
             git -C {safe_repo} config user.name CI",
            base = shell_escape_posix(base),
        ),
    )
    .await;
    port.write_file(machine_id, &format!("{repo}/README.md"), "conformance\n")
        .await
        .expect("seeds the repository with a file to commit");
    sh(
        port,
        machine_id,
        &format!("set -eu; git -C {safe_repo} add README.md; git -C {safe_repo} commit -q -m init"),
    )
    .await;

    (project_root, repo)
}

/// Exercise the `create_terminal_worktree` contract against `port`, using
/// `machine_id` to address the target and `scratch` as a pre-existing writable
/// directory on it.
///
/// `scratch` is resolved with `pwd -P` before anything is built below it, so
/// the paths this asserts against are the physical ones git reports — the local
/// leg's temp dir is reached through `/var` → `/private/var` on macOS, and a
/// logical path would make the same assertion pass remotely and fail locally
/// for a reason that has nothing to do with the port.
pub async fn terminal_worktree_contract(
    port: Arc<dyn ExecutionPort>,
    machine_id: &str,
    scratch: &str,
) {
    let anchor = sh(
        &port,
        machine_id,
        &format!("cd {} && pwd -P", shell_escape_posix(scratch)),
    )
    .await
    .trim()
    .to_string();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the Unix epoch")
        .as_nanos();
    let base = format!("{anchor}/demeteo-terminal-worktree-{nanos}");

    let (project_root, repo) = make_project_repo(&port, machine_id, &base).await;
    let head = rev(&port, machine_id, &repo, "HEAD")
        .await
        .expect("the seeded repository has a HEAD");

    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let ops: Arc<dyn WorktreeOpsPort> = Arc::new(GitOpsHelper::new(db, port.clone()));

    let area = format!("{project_root}/{TERMINAL_WORKTREES_SUBDIR}/{REPO_NAME}");

    // --- the worktree lands in the controlled area and nowhere else --------
    let created = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/session", "session-one"),
        )
        .await
        .expect("create_terminal_worktree must succeed against a real repository");

    assert_eq!(
        created.worktree.path,
        format!("{area}/session-one"),
        "the destination must be <project_root>/{TERMINAL_WORKTREES_SUBDIR}/<repo_name>/<name>",
    );
    assert!(
        !created
            .worktree
            .path
            .starts_with(&format!("{project_root}/{REPOS_SUBDIR}/")),
        "the area must not live under the pruned repos/ directory: {}",
        created.worktree.path,
    );
    assert_eq!(
        entries(&port, machine_id, &project_root).await,
        vec![
            REPOS_SUBDIR.to_string(),
            TERMINAL_WORKTREES_SUBDIR.to_string()
        ],
        "creation must add exactly one directory to the project root",
    );
    assert_eq!(
        entries(&port, machine_id, &area).await,
        vec!["session-one".to_string()],
        "the area must hold the requested name and nothing else",
    );
    assert_eq!(
        entries(&port, machine_id, &format!("{project_root}/{REPOS_SUBDIR}")).await,
        vec![REPO_NAME.to_string()],
        "creation must leave nothing beside the primary checkout",
    );
    assert!(
        exists(
            &port,
            machine_id,
            &format!("{}/README.md", created.worktree.path)
        )
        .await,
        "the destination must be a real checkout, not an empty directory",
    );

    // --- the returned metadata is what git actually created ----------------
    let listed = ops
        .list_worktrees(Some(machine_id), &repo)
        .await
        .expect("lists the repository's worktrees");
    assert_eq!(
        listed.len(),
        1,
        "exactly one linked worktree must exist; got {listed:?}",
    );
    assert_eq!(listed[0].path, created.worktree.path);
    assert_eq!(listed[0].branch, created.worktree.branch);
    assert_eq!(created.worktree.branch.as_deref(), Some("terminal/session"));
    assert!(!created.worktree.is_locked);

    // --- the branch is cut at the primary HEAD, which does not move --------
    assert_eq!(
        rev(&port, machine_id, &repo, "refs/heads/terminal/session")
            .await
            .expect("the requested branch exists"),
        head,
        "-b must create the terminal branch at the primary checkout's HEAD",
    );
    assert_eq!(
        rev(&port, machine_id, &repo, "HEAD")
            .await
            .expect("the primary checkout still has a HEAD"),
        head,
        "creation must not move the primary checkout",
    );
    assert_eq!(
        ops.get_head_branch(Some(machine_id), &repo)
            .await
            .as_deref(),
        Some("main"),
        "creation must not check anything out in the primary checkout",
    );

    // --- an existing branch is rejected, never reused -----------------------
    sh(
        &port,
        machine_id,
        &format!(
            "git -C {} branch terminal/already-exists",
            shell_escape_posix(&repo)
        ),
    )
    .await;
    let taken = rev(
        &port,
        machine_id,
        &repo,
        "refs/heads/terminal/already-exists",
    )
    .await
    .expect("the pre-existing branch resolves");
    let error = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/already-exists", "second-session"),
        )
        .await
        .expect_err("an existing branch must not be reused");
    assert!(error.contains("git worktree add"), "{error}");
    assert_eq!(
        rev(
            &port,
            machine_id,
            &repo,
            "refs/heads/terminal/already-exists"
        )
        .await
        .expect("the pre-existing branch still resolves"),
        taken,
        "a rejected request must not reset the branch it was refused",
    );
    assert!(
        !exists(&port, machine_id, &format!("{area}/second-session")).await,
        "a rejected request must not leave a destination behind",
    );

    // --- a name collision is reported without touching what is there -------
    let collision = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/collision", "session-one"),
        )
        .await
        .expect_err("an occupied destination must not be reused");
    assert!(
        collision.contains("destination already exists"),
        "{collision}"
    );
    assert!(
        rev(&port, machine_id, &repo, "refs/heads/terminal/collision")
            .await
            .is_err(),
        "a refused collision must not create the branch it was asked for",
    );
    let after = ops
        .list_worktrees(Some(machine_id), &repo)
        .await
        .expect("lists the repository's worktrees");
    assert_eq!(
        after
            .iter()
            .map(|w| (w.path.clone(), w.branch.clone(), w.is_locked))
            .collect::<Vec<_>>(),
        listed
            .iter()
            .map(|w| (w.path.clone(), w.branch.clone(), w.is_locked))
            .collect::<Vec<_>>(),
        "collision handling must neither remove, move, nor re-branch the worktree already there",
    );
    assert!(
        exists(
            &port,
            machine_id,
            &format!("{}/README.md", created.worktree.path)
        )
        .await,
        "the occupying worktree's contents must survive the refused request",
    );

    sh(
        &port,
        machine_id,
        &format!("rm -rf {}", shell_escape_posix(&base)),
    )
    .await;

    terminal_branch_is_cut_from_origin(&port, machine_id, &anchor, nanos).await;
}

/// A requested base is fetched before it is used, so the session starts on
/// upstream's tip rather than on whatever this checkout last saw.
///
/// The shape being caught: `origin/main` moves, nobody pulls, and a worktree
/// "from main" is cut at a local ref that is days old. Nothing about that looks
/// wrong afterwards — the branch exists, the files are there, and the missing
/// commits only surface as a conflict at merge time. So origin is advanced here
/// behind the checkout's back, and the new branch has to land on the commit the
/// checkout has never seen.
async fn terminal_branch_is_cut_from_origin(
    port: &Arc<dyn ExecutionPort>,
    machine_id: &str,
    anchor: &str,
    nanos: u128,
) {
    let base = format!("{anchor}/demeteo-terminal-worktree-origin-{nanos}");
    let (project_root, repo) = make_project_repo(port, machine_id, &base).await;
    let origin = format!("{base}/origin.git");
    let publisher = format!("{base}/publisher");
    let stale = rev(port, machine_id, &repo, "HEAD")
        .await
        .expect("the seeded repository has a HEAD");

    sh(
        port,
        machine_id,
        &format!(
            // `-b main` on the bare repository, not just on the checkout: a
            // bare repo's HEAD comes from the *target host's* `init.defaultBranch`,
            // and a clone of one whose HEAD names a branch that was never pushed
            // lands on an unborn branch of the wrong name. That is `master` on
            // the conformance container and `main` on most developer machines,
            // so leaving it to the host makes this setup pass locally and fail
            // over SSH for a reason that has nothing to do with the port.
            "set -eu; git init -q --bare -b main {origin}; \
             git -C {repo} remote add origin {origin}; \
             git -C {repo} push -q origin main; \
             git clone -q {origin} {publisher}; \
             git -C {publisher} config user.email ci@demeteo.test; \
             git -C {publisher} config user.name CI; \
             git -C {publisher} commit -q --allow-empty -m upstream-moved; \
             git -C {publisher} push -q origin HEAD:refs/heads/main",
            origin = shell_escape_posix(&origin),
            repo = shell_escape_posix(&repo),
            publisher = shell_escape_posix(&publisher),
        ),
    )
    .await;
    let upstream = sh(
        port,
        machine_id,
        &format!(
            "git -C {} rev-parse --verify HEAD",
            shell_escape_posix(&publisher)
        ),
    )
    .await
    .trim()
    .to_string();
    assert_ne!(
        upstream, stale,
        "origin must be ahead of the checkout, or this proves nothing"
    );
    assert_eq!(
        rev(port, machine_id, &repo, "refs/remotes/origin/main")
            .await
            .expect("the checkout tracked origin/main at push time"),
        stale,
        "the checkout's view of origin must still be the stale one before the create",
    );

    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let ops: Arc<dyn WorktreeOpsPort> = Arc::new(GitOpsHelper::new(db, port.clone()));

    let created = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &TerminalWorktreeRequest {
                branch: "terminal/fresh".to_string(),
                base_branch: Some("main".to_string()),
                worktree_name: "fresh".to_string(),
            },
        )
        .await
        .expect("creates a terminal worktree from a named base");

    assert_eq!(
        created.base_ref, "origin/main",
        "the caller is told which ref was used, because the fallback is silent otherwise",
    );
    assert_eq!(
        rev(port, machine_id, &repo, "refs/heads/terminal/fresh")
            .await
            .expect("the terminal branch exists"),
        upstream,
        "the branch must start at the fetched origin tip, not at the stale local ref",
    );
    assert_eq!(
        rev(port, machine_id, &repo, "refs/heads/main")
            .await
            .expect("the local default branch still resolves"),
        stale,
        "refreshing a base must not move the branch the user has checked out",
    );

    // A base nobody has is refused rather than quietly resolved to HEAD, which
    // is the one outcome that would put a session somewhere it did not ask for.
    let missing = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &TerminalWorktreeRequest {
                branch: "terminal/nowhere".to_string(),
                base_branch: Some("no-such-base".to_string()),
                worktree_name: "nowhere".to_string(),
            },
        )
        .await
        .expect_err("an unknown base must not fall through to HEAD");
    assert!(missing.contains("no-such-base"), "{missing}");
    assert!(
        rev(port, machine_id, &repo, "refs/heads/terminal/nowhere")
            .await
            .is_err(),
        "a refused base must not create the branch it was asked for",
    );

    sh(
        port,
        machine_id,
        &format!("rm -rf {}", shell_escape_posix(&base)),
    )
    .await;
}

/// Removal takes the worktree it was given and nothing else.
///
/// `git worktree remove --force` deletes a directory outright, so the only
/// thing between this and a user's uncommitted work — or a running step's
/// checkout, or the primary one — is the area check. It is asserted here rather
/// than only against a double, because a path that classifies one way locally
/// and another over SSH is exactly the divergence this file exists for.
async fn terminal_worktree_removal(
    port: &Arc<dyn ExecutionPort>,
    machine_id: &str,
    anchor: &str,
    nanos: u128,
) {
    let base = format!("{anchor}/demeteo-terminal-worktree-removal-{nanos}");
    let (project_root, repo) = make_project_repo(port, machine_id, &base).await;

    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let ops: Arc<dyn WorktreeOpsPort> = Arc::new(GitOpsHelper::new(db, port.clone()));

    let retired = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/retired", "retired"),
        )
        .await
        .expect("creates the worktree to retire")
        .worktree;
    let kept = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/kept", "kept"),
        )
        .await
        .expect("creates the worktree to keep")
        .worktree;

    // --- the primary checkout is not removable through this path ------------
    let refused = ops
        .remove_terminal_worktree(Some(machine_id), &repo, &project_root, &repo, false)
        .await
        .expect_err("the primary checkout must never be removable as a terminal worktree");
    assert!(refused.contains("not a terminal worktree"), "{refused}");
    assert!(
        exists(port, machine_id, &format!("{repo}/README.md")).await,
        "a refused removal must not touch the checkout it was aimed at",
    );

    // --- a clean worktree goes, and takes nothing with it -------------------
    ops.remove_terminal_worktree(Some(machine_id), &repo, &project_root, &retired.path, false)
        .await
        .expect("removes a clean terminal worktree");
    assert!(
        !exists(port, machine_id, &retired.path).await,
        "removal must delete the directory, not just unregister it",
    );
    assert_eq!(
        ops.list_terminal_worktrees(Some(machine_id), &repo, &project_root)
            .await
            .expect("lists the surviving terminal worktrees")
            .iter()
            .map(|worktree| worktree.path.as_str())
            .collect::<Vec<_>>(),
        [kept.path.as_str()],
        "removal must leave every other worktree registered",
    );
    assert!(
        rev(port, machine_id, &repo, "refs/heads/terminal/retired")
            .await
            .is_ok(),
        "the branch outlives its worktree: the directory is recreatable, the commits are not",
    );
    // The administrative entry has to go with it, or re-creating the same name
    // fails against a worktree Git still believes in.
    ops.create_terminal_worktree(
        Some(machine_id),
        &repo,
        &project_root,
        &terminal_request("terminal/retired-again", "retired"),
    )
    .await
    .expect("the removed name must be reusable");

    // --- work in progress is not discarded without being asked -------------
    port.write_file(machine_id, &format!("{}/scratch.txt", kept.path), "wip\n")
        .await
        .expect("leaves an untracked file in the worktree");
    let dirty = ops
        .remove_terminal_worktree(Some(machine_id), &repo, &project_root, &kept.path, false)
        .await
        .expect_err("git must refuse to discard untracked files");
    assert!(dirty.contains("git worktree remove failed"), "{dirty}");
    assert!(
        exists(port, machine_id, &format!("{}/scratch.txt", kept.path)).await,
        "a refused removal must leave the work it refused to discard",
    );
    ops.remove_terminal_worktree(Some(machine_id), &repo, &project_root, &kept.path, true)
        .await
        .expect("force is the user's answer to that refusal");
    assert!(
        !exists(port, machine_id, &kept.path).await,
        "a forced removal must actually remove",
    );

    sh(
        port,
        machine_id,
        &format!("rm -rf {}", shell_escape_posix(&base)),
    )
    .await;
}

/// The listing must be anchored on what Git resolved, not on the paths
/// configuration handed in.
///
/// Everything above builds under a `pwd -P` anchor, so logical and physical
/// agree and an implementation that anchored on `project_root` would pass. Here
/// the project is built under a real directory and driven entirely through a
/// symlink to it, which is the shape a listing filter gets wrong: Git replays
/// the resolved path, and a comparison against the configured one matches
/// nothing.
async fn terminal_listing_through_a_symlinked_root(
    port: &Arc<dyn ExecutionPort>,
    machine_id: &str,
    anchor: &str,
    nanos: u128,
) {
    let physical = format!("{anchor}/demeteo-terminal-worktree-linked-{nanos}");
    let logical = format!("{physical}-link");
    sh(
        port,
        machine_id,
        &format!(
            "set -eu; rm -rf {p} {l}; mkdir -p {p}; ln -s {p} {l}",
            p = shell_escape_posix(&physical),
            l = shell_escape_posix(&logical),
        ),
    )
    .await;

    // Prove the premise on this transport before asserting anything with it: a
    // target where the link did not take would pass every clause below for the
    // wrong reason.
    let resolved = sh(
        port,
        machine_id,
        &format!("cd {} && pwd -P", shell_escape_posix(&logical)),
    )
    .await
    .trim()
    .to_string();
    assert_eq!(
        resolved, physical,
        "the symlinked root must resolve elsewhere, or this clause proves nothing"
    );

    let (physical_root, _) = make_project_repo(port, machine_id, &physical).await;
    let logical_root = format!("{logical}/project");
    let logical_repo = format!("{logical_root}/{REPOS_SUBDIR}/{REPO_NAME}");

    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let ops: Arc<dyn WorktreeOpsPort> = Arc::new(GitOpsHelper::new(db, port.clone()));

    let created = ops
        .create_terminal_worktree(
            Some(machine_id),
            &logical_repo,
            &logical_root,
            &terminal_request("terminal/linked", "session-linked"),
        )
        .await
        .expect("creates a terminal worktree through the symlinked root");

    assert_eq!(
        created.worktree.path,
        format!("{physical_root}/{TERMINAL_WORKTREES_SUBDIR}/{REPO_NAME}/session-linked"),
        "Git records the resolved path, which is what the listing has to match",
    );

    let listed = ops
        .list_terminal_worktrees(Some(machine_id), &logical_repo, &logical_root)
        .await
        .expect("lists terminal locations through the symlinked root");
    assert_eq!(
        listed
            .iter()
            .map(|w| (w.path.as_str(), w.branch.as_deref()))
            .collect::<Vec<_>>(),
        [(created.worktree.path.as_str(), Some("terminal/linked"))],
        "the worktree just created must be the one listed back",
    );

    sh(
        port,
        machine_id,
        &format!(
            "rm -rf {} {}",
            shell_escape_posix(&physical),
            shell_escape_posix(&logical)
        ),
    )
    .await;
}

#[tokio::test]
async fn local_subprocess_adapter_satisfies_the_terminal_worktree_contract() {
    let scratch = std::env::temp_dir().join("demeteo-terminal-worktree-conformance");
    std::fs::create_dir_all(&scratch).expect("creates the local conformance scratch dir");
    let port: Arc<dyn ExecutionPort> = Arc::new(LocalSubprocessAdapter::new());
    terminal_worktree_contract(port, "local", &scratch.to_string_lossy()).await;
}

/// SSH must fail closed until the target has a trusted-worktree helper that can
/// make the no-follow proof in one remote transaction.
#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn ssh_client_adapter_reports_trusted_worktree_unavailability() {
    let t = super::ssh_target::target();
    let machine_id = "ssh-terminal-worktree";
    let port = super::ssh_target::adapter(&t, t.port, machine_id);

    port.run_command(machine_id, &format!("mkdir -p {}", t.workdir))
        .await
        .expect("failed to create the remote conformance workdir");

    let (project_root, repo) = make_project_repo(&port, machine_id, &t.workdir).await;
    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let ops = GitOpsHelper::new(db, port);
    let error = ops
        .create_terminal_worktree(
            Some(machine_id),
            &repo,
            &project_root,
            &terminal_request("terminal/unavailable", "unavailable"),
        )
        .await
        .expect_err("SSH must not recreate the old shell transaction");
    assert!(error.contains("unavailable over SSH"), "{error}");
}
