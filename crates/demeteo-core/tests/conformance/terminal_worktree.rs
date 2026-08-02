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
use crate::ports::worktree_ops::WorktreeOpsPort;
use rusqlite::Connection;
use std::sync::Arc;

/// A repository name carrying a space, so the escaping the adapter applies is
/// exercised through a transport that adds its own quoting layer rather than
/// only through a double that records the string.
const REPO_NAME: &str = "sample repo";

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
            "terminal/session",
            "session-one",
        )
        .await
        .expect("create_terminal_worktree must succeed against a real repository");

    assert_eq!(
        created.path,
        format!("{area}/session-one"),
        "the destination must be <project_root>/{TERMINAL_WORKTREES_SUBDIR}/<repo_name>/<name>",
    );
    assert!(
        !created
            .path
            .starts_with(&format!("{project_root}/{REPOS_SUBDIR}/")),
        "the area must not live under the pruned repos/ directory: {}",
        created.path,
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
        exists(&port, machine_id, &format!("{}/README.md", created.path)).await,
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
    assert_eq!(listed[0].path, created.path);
    assert_eq!(listed[0].branch, created.branch);
    assert_eq!(created.branch.as_deref(), Some("terminal/session"));
    assert!(!created.is_locked);

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
            "terminal/already-exists",
            "second-session",
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
            "terminal/collision",
            "session-one",
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
        exists(&port, machine_id, &format!("{}/README.md", created.path)).await,
        "the occupying worktree's contents must survive the refused request",
    );

    sh(
        &port,
        machine_id,
        &format!("rm -rf {}", shell_escape_posix(&base)),
    )
    .await;

    terminal_listing_through_a_symlinked_root(&port, machine_id, &anchor, nanos).await;
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
            "terminal/linked",
            "session-linked",
        )
        .await
        .expect("creates a terminal worktree through the symlinked root");

    assert_eq!(
        created.path,
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
        [(created.path.as_str(), Some("terminal/linked"))],
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

/// The same assertions against the loopback sshd C2.2 stands up. This is the
/// only leg that can catch a `create_terminal_worktree` whose script means one
/// thing to a local `sh -c` and another to a remote one; the local leg above
/// stays green through exactly that divergence.
#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn ssh_client_adapter_satisfies_the_terminal_worktree_contract() {
    let t = super::ssh_target::target();
    let machine_id = "ssh-terminal-worktree";
    let port = super::ssh_target::adapter(&t, t.port, machine_id);

    port.run_command(machine_id, &format!("mkdir -p {}", t.workdir))
        .await
        .expect("failed to create the remote conformance workdir");

    terminal_worktree_contract(port, machine_id, &t.workdir).await;
}
