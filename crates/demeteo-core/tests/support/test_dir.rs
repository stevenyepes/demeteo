//! Where a test puts the files it needs, and how they stop outliving it.
//!
//! A bare `std::env::temp_dir().join(format!("demeteo-…-{nanos}"))` is never
//! removed: most fixtures hand the path to `build_core_context` and drop it,
//! so nothing is left holding it when the test ends. Each `cargo test` leaked
//! about forty such trees, and a tmpfs `/tmp` that had collected ten thousand
//! of them (15 GB) then tripped the 20 GiB dependency-cache disk guard in the
//! worktree tests — a full `/tmp` reported as a failing test.
//!
//! Two layers, because neither covers the other's gap:
//!
//! - [`TestDir`] removes its tree when it drops, including on a panicking
//!   assertion. Use it whenever the test can hold the guard.
//! - [`scratch`] is for a fixture that can only hand a path onward (an
//!   `AppContext` keeps no guard). Every path, guarded or not, lives under one
//!   root per process, and the first call in a process removes the roots that
//!   earlier processes left. That also catches what no `Drop` can: a run
//!   killed by a signal.
//!
//! Stale means *older than* [`STALE_AFTER`], not *some other pid*: test
//! processes run concurrently (several binaries, or one process per test under
//! nextest), and a root that is merely someone else's may still be in use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;
use std::time::{Duration, SystemTime};

use crate::shared::fs_remove;

/// How long a run root may go without a new entry before another process
/// reclaims it. Every [`scratch`] call adds an entry, which refreshes the
/// root's mtime, so this bounds the gap between two fixtures in one process,
/// not the length of the run.
pub const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

const ROOT: &str = "demeteo-tests";

static SWEEP: Once = Once::new();
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty directory for this test, removed when the guard drops.
pub struct TestDir(PathBuf);

impl TestDir {
    pub fn new(tag: &str) -> TestDir {
        TestDir(scratch(tag))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        remove(&self.0);
    }
}

/// A fresh, empty directory for a fixture that cannot keep a guard.
///
/// Reclaimed by a later process once its run is stale, not by this one.
pub fn scratch(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(ROOT);
    SWEEP.call_once(|| sweep(&base));
    let path = base
        .join(std::process::id().to_string())
        .join(format!("{tag}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&path);
    if let Err(e) = std::fs::create_dir_all(&path) {
        panic!("test scratch dir {}: {e}", path.display());
    }
    path
}

/// Whether a run root another process left may be removed.
///
/// A root whose age cannot be read is kept: a clock that went backwards or a
/// filesystem without mtimes is no evidence the run is over.
pub fn is_stale(own: bool, age: Option<Duration>) -> bool {
    !own && age.is_some_and(|age| age >= STALE_AFTER)
}

fn sweep(base: &Path) {
    let own = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        if is_stale(entry.file_name() == own.as_str(), age) {
            remove(&entry.path());
        }
    }
}

/// Remove a tree a test may have fenced.
///
/// The artifact-scope fence (`adapters/worktree/git_ops/scope.rs`) strips
/// write permission from directories, and on Unix an entry cannot be unlinked
/// from a directory its owner cannot write — so a plain `remove_dir_all`
/// silently leaves exactly the trees the fence protected. Windows' read-only
/// attribute is cleared by `fs_remove` itself, but the fence's deny ACL is
/// not lifted by anything here: a Windows test that fences a tree must
/// un-fence it before its guard drops, or the tree outlives the sweep too.
fn remove(root: &Path) {
    restore_write(root);
    let _ = fs_remove::remove_dir_all(root).into_result();
}

#[cfg(unix)]
fn restore_write(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o700);
        let _ = std::fs::set_permissions(&dir, perms);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            stack.extend(entries.flatten().map(|e| e.path()));
        }
    }
}

#[cfg(not(unix))]
fn restore_write(_root: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_runs_root_is_reclaimed_once_it_goes_quiet() {
        assert!(is_stale(false, Some(STALE_AFTER)));
        assert!(!is_stale(false, Some(STALE_AFTER - Duration::from_secs(1))));
    }

    #[test]
    fn this_runs_root_and_an_unreadable_age_are_kept() {
        assert!(!is_stale(true, Some(STALE_AFTER * 10)));
        assert!(!is_stale(false, None));
    }

    #[test]
    fn a_guard_removes_a_tree_the_fence_made_read_only() {
        let dir = TestDir::new("test-dir-fenced");
        let fenced = dir.path().join("fenced");
        std::fs::create_dir_all(&fenced).unwrap();
        std::fs::write(fenced.join("artifact.md"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fenced, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists(), "{} survived its guard", path.display());
    }

    #[test]
    fn scratch_paths_are_distinct_and_live_under_this_runs_root() {
        let a = scratch("test-dir-distinct");
        let b = scratch("test-dir-distinct");
        assert_ne!(a, b);
        let root = std::env::temp_dir()
            .join(ROOT)
            .join(std::process::id().to_string());
        assert!(a.starts_with(&root) && b.starts_with(&root));
        remove(&a);
        remove(&b);
    }
}
