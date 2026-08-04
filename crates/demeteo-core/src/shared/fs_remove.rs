//! Deleting a tree that Git wrote.
//!
//! `std::fs::remove_dir_all` cannot delete a Git repository on Windows. Git
//! writes loose objects and `.idx`/`.pack` files with the read-only bit set,
//! and `DeleteFileW` refuses a file carrying `FILE_ATTRIBUTE_READONLY` — std
//! neither clears it nor retries. The SFTP arm of the same port method
//! succeeds against the same tree, so a worktree teardown that works over SSH
//! fails locally: a divergence between two transports, which
//! `docs/EXECUTION_PARITY.md` forbids outright rather than tolerating as a
//! local quirk.
//!
//! Retrying is the second half. Defender and the search indexer open handles
//! on files that were closed moments ago, and the resulting
//! `ERROR_SHARING_VIOLATION` clears on its own within a second or so after a
//! large build. Git's own `mingw_unlink` concedes the point and retries — but
//! for ~71ms, which is short enough to be indistinguishable from not retrying
//! once `node_modules` has just been written.
//!
//! ## What is behind a `cfg` and what deliberately is not
//!
//! Only two things need one: clearing the read-only attribute, which must stay
//! a no-op everywhere else because `set_readonly(false)` on Unix is `chmod +w`
//! and would delete files this call has always refused to delete; and which
//! table of OS error codes the host retries. Everything that *decides* — the
//! walk order, the backoff schedule, the classification of an error, when to
//! stop retrying — is ordinary code reached by tests on the Linux development
//! host, because a decision behind a `cfg` is a decision whose first
//! observation costs a CI round trip.
//!
//! The syscalls are taken through [`TreeFs`] for the same reason: the fake in
//! the tests can hold a file open forever, which no test on this host can ask
//! the real filesystem to do.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `ERROR_ACCESS_DENIED` — also what a virus scanner's open handle looks like
/// when it was opened without `FILE_SHARE_DELETE`.
const ERROR_ACCESS_DENIED: i32 = 5;
/// `ERROR_SHARING_VIOLATION`.
const ERROR_SHARING_VIOLATION: i32 = 32;
/// `ERROR_DIR_NOT_EMPTY` — reached even after every child was deleted, because
/// a Windows delete is a *pending* delete until the last handle to the file
/// closes, and the directory is not empty until then.
const ERROR_DIR_NOT_EMPTY: i32 = 145;

/// The Win32 error codes a later attempt can plausibly clear.
///
/// Defined unconditionally, and not read from `windows-sys`, so the tests that
/// pin the classification run on every host rather than only on the one that
/// cannot run them.
pub const WINDOWS_TRANSIENT: [i32; 3] = [
    ERROR_ACCESS_DENIED,
    ERROR_SHARING_VIOLATION,
    ERROR_DIR_NOT_EMPTY,
];

/// What this host retries.
///
/// Empty off Windows on purpose. `EACCES` there means the directory's mode
/// denies the unlink, which no amount of waiting changes, and a permission
/// error that used to surface immediately must not start taking two seconds
/// to surface.
#[cfg(windows)]
pub const HOST_TRANSIENT: &[i32] = &WINDOWS_TRANSIENT;
#[cfg(not(windows))]
pub const HOST_TRANSIENT: &[i32] = &[];

/// The first wait, doubled for each subsequent one.
pub const FIRST_BACKOFF: Duration = Duration::from_millis(1);

/// How long one operation may spend waiting in total, across all its retries.
///
/// Two seconds is chosen against the observed behaviour this exists for — an
/// indexer or scanner handle on a just-closed file — and against the cost of
/// being wrong: the price of waiting too long is one slow teardown, and the
/// price of waiting too little is a leaked worktree that a user has to delete
/// by hand.
pub const BACKOFF_BUDGET: Duration = Duration::from_millis(2_000);

/// The waits between attempts, shortest first.
///
/// Doubling from [`FIRST_BACKOFF`] until the total reaches
/// [`BACKOFF_BUDGET`], so the early attempts cost microseconds and only a
/// genuinely stuck path pays the whole budget.
pub fn backoff_schedule() -> Vec<Duration> {
    let mut delays = Vec::new();
    let mut total = Duration::ZERO;
    let mut next = FIRST_BACKOFF;
    while total < BACKOFF_BUDGET {
        delays.push(next);
        total += next;
        next *= 2;
    }
    delays
}

/// What one filesystem entry is, to a caller that must not follow a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// A symlink, junction or other reparse point. `directory` decides which
    /// call unlinks it: Windows removes a link *to* a directory with
    /// `RemoveDirectoryW`, Unix unlinks every symlink regardless of target.
    /// Since `FileType::is_dir` is false for a Unix symlink and true for a
    /// Windows directory link, that difference needs no `cfg` — the platform
    /// answers it through the metadata.
    Link {
        directory: bool,
    },
}

/// One entry, as [`TreeFs::inspect`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub readonly: bool,
}

/// [`EntryKind`] from the two questions a `FileType` answers.
pub fn classify_entry(is_symlink: bool, is_dir: bool) -> EntryKind {
    if is_symlink {
        EntryKind::Link { directory: is_dir }
    } else if is_dir {
        EntryKind::Dir
    } else {
        EntryKind::File
    }
}

/// Which call failed, so a leftover names the operation and not just a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalOp {
    Inspect,
    ReadDir,
    RemoveFile,
    RemoveDir,
}

impl std::fmt::Display for RemovalOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            RemovalOp::Inspect => "inspect",
            RemovalOp::ReadDir => "read directory",
            RemovalOp::RemoveFile => "remove file",
            RemovalOp::RemoveDir => "remove directory",
        };
        f.write_str(name)
    }
}

/// Whether waiting can help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Already gone, which is what this call wanted.
    Absent,
    Transient,
    Permanent,
}

/// Classify an error against a table of retryable OS codes.
///
/// `NotFound` outranks the table: a concurrent delete of something we were
/// about to delete is a success, and on Windows it is also the ordinary
/// outcome of a pending delete completing between two attempts.
pub fn disposition(error: &io::Error, transient: &[i32]) -> Disposition {
    if error.kind() == io::ErrorKind::NotFound {
        return Disposition::Absent;
    }
    match error.raw_os_error() {
        Some(code) if transient.contains(&code) => Disposition::Transient,
        _ => Disposition::Permanent,
    }
}

/// A path the walk could not remove, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalFailure {
    pub path: PathBuf,
    pub op: RemovalOp,
    pub kind: io::ErrorKind,
    pub os_error: Option<i32>,
    pub attempts: u32,
    pub message: String,
}

impl RemovalFailure {
    fn new(path: &Path, op: RemovalOp, error: &io::Error, attempts: u32) -> Self {
        Self {
            path: path.to_path_buf(),
            op,
            kind: error.kind(),
            os_error: error.raw_os_error(),
            attempts,
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for RemovalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "could not {} '{}' after {} attempt(s): {}",
            self.op,
            self.path.display(),
            self.attempts,
            self.message
        )
    }
}

/// The result of removing a tree, in the shape a cleanup queue needs.
///
/// A caller that has to enqueue a leftover for a later sweep cannot do that
/// from a formatted string, which is why this is not a `Result<(), String>`
/// with the paths spelled inside the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalOutcome {
    /// Nothing of the tree is left on disk.
    Removed,
    /// The root did not exist. Distinct from [`RemovalOutcome::Removed`]
    /// because a teardown may treat it as done while
    /// `ExecutionPort::remove_dir_all` keeps reporting the error its bare
    /// `std::fs` implementation always reported.
    Absent { root: PathBuf },
    /// The root still exists. `leftovers` names the paths that stopped it.
    Incomplete {
        root: PathBuf,
        leftovers: Vec<RemovalFailure>,
    },
}

impl RemovalOutcome {
    /// The `ExecutionPort` shape. The message keeps the prefix the bare
    /// `std::fs` implementation used, so nothing reading a log sees the
    /// operation change name.
    pub fn into_result(self) -> Result<(), String> {
        match self {
            RemovalOutcome::Removed => Ok(()),
            RemovalOutcome::Absent { root } => Err(format!(
                "Failed to remove directory '{}': the path does not exist",
                root.display()
            )),
            RemovalOutcome::Incomplete { root, leftovers } => {
                let detail = leftovers
                    .first()
                    .map(|failure| failure.to_string())
                    .unwrap_or_else(|| "the directory is still present".to_string());
                Err(format!(
                    "Failed to remove directory '{}': {} path(s) left, first: {}",
                    root.display(),
                    leftovers.len(),
                    detail
                ))
            }
        }
    }
}

/// The syscalls the walk makes.
///
/// A trait so the walk is reachable from a test: the interesting states — a
/// handle held open for a fixed number of attempts, a read-only bit, a
/// directory link that must not be followed — are all states this host cannot
/// be asked to produce, and every one of them is a decision the walk makes
/// rather than a syscall it performs.
pub trait TreeFs {
    /// Never follows a link. Following one while deleting is how a delete
    /// leaves the tree it was given.
    fn inspect(&self, path: &Path) -> io::Result<Entry>;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    /// Best-effort and deliberately unreported: an entry whose attribute could
    /// not be cleared still gets its unlink attempted, and that unlink names
    /// the real reason. A failure here is only ever a worse version of the
    /// error the next line produces.
    fn clear_readonly(&self, path: &Path);
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    fn sleep(&self, delay: Duration);
}

/// The real filesystem.
pub struct HostFs;

impl TreeFs for HostFs {
    fn inspect(&self, path: &Path) -> io::Result<Entry> {
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();
        Ok(Entry {
            kind: classify_entry(file_type.is_symlink(), file_type.is_dir()),
            readonly: meta.permissions().readonly(),
        })
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut children = Vec::new();
        for entry in std::fs::read_dir(path)? {
            children.push(entry?.path());
        }
        Ok(children)
    }

    fn clear_readonly(&self, path: &Path) {
        #[cfg(windows)]
        {
            let Ok(meta) = std::fs::symlink_metadata(path) else {
                return;
            };
            let mut perms = meta.permissions();
            // `permissions_set_readonly_false` fires on the Unix meaning of
            // this call, which is a chmod to world-writable. This arm exists
            // only on Windows, where it clears one attribute bit and grants
            // nobody anything.
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
        #[cfg(not(windows))]
        {
            let _ = path;
        }
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir(path)
    }

    fn sleep(&self, delay: Duration) {
        std::thread::sleep(delay);
    }
}

/// Remove `root` and everything under it.
///
/// The typed entry point: a caller that must enqueue what was left behind
/// reads [`RemovalOutcome::Incomplete`] rather than parsing a message.
pub fn remove_dir_all(root: &Path) -> RemovalOutcome {
    remove_tree(&HostFs, root, HOST_TRANSIENT, &backoff_schedule())
}

enum Step {
    Visit(PathBuf),
    /// The directory itself, once everything below it has been visited.
    /// `mark` is the failure count at the moment it was pushed, which is what
    /// makes "did anything under here survive" a subtraction rather than a
    /// prefix search.
    Finish {
        path: PathBuf,
        mark: usize,
    },
}

/// Remove `root` through `fs`, retrying `transient` codes on the `backoff`
/// schedule.
///
/// Depth-first and post-order: a directory is removed only after everything in
/// it, because there is no recursive unlink on either platform.
pub fn remove_tree(
    fs: &dyn TreeFs,
    root: &Path,
    transient: &[i32],
    backoff: &[Duration],
) -> RemovalOutcome {
    let mut walk = Walk {
        fs,
        transient,
        backoff,
        failures: Vec::new(),
        gave_up: false,
    };
    walk.run(root)
}

struct Walk<'a> {
    fs: &'a dyn TreeFs,
    transient: &'a [i32],
    backoff: &'a [Duration],
    failures: Vec<RemovalFailure>,
    gave_up: bool,
}

enum Visited {
    Gone,
    Handled,
}

impl Walk<'_> {
    fn run(&mut self, root: &Path) -> RemovalOutcome {
        let mut stack = vec![Step::Visit(root.to_path_buf())];
        let mut first = true;
        while let Some(step) = stack.pop() {
            match step {
                Step::Visit(path) => {
                    let visited = self.visit(&path, first, &mut stack);
                    if first {
                        if let Visited::Gone = visited {
                            return RemovalOutcome::Absent {
                                root: root.to_path_buf(),
                            };
                        }
                        first = false;
                    }
                }
                Step::Finish { path, mark } => {
                    let survivors_below = self.failures.len() > mark;
                    let removed =
                        self.attempt(&path, RemovalOp::RemoveDir, || self.fs.remove_dir(&path));
                    match removed {
                        Ok(_) => {}
                        Err(failure) => {
                            // A directory that still holds something we gave
                            // up on is not a second problem; reporting it as
                            // one buries the leaf that actually blocked the
                            // teardown under one entry per ancestor.
                            if survivors_below {
                                self.gave_up = true;
                            } else {
                                self.record(failure);
                            }
                        }
                    }
                }
            }
        }

        if self.failures.is_empty() {
            RemovalOutcome::Removed
        } else {
            RemovalOutcome::Incomplete {
                root: root.to_path_buf(),
                leftovers: std::mem::take(&mut self.failures),
            }
        }
    }

    fn visit(&mut self, path: &Path, is_root: bool, stack: &mut Vec<Step>) -> Visited {
        let inspected = self.attempt(path, RemovalOp::Inspect, || self.fs.inspect(path));
        let entry = match inspected {
            Ok(Some(entry)) => entry,
            Ok(None) => return Visited::Gone,
            Err(failure) => {
                self.record(failure);
                return Visited::Handled;
            }
        };

        if is_root && entry.kind != EntryKind::Dir {
            // `std::fs::remove_dir_all` refuses a non-directory root, and a
            // teardown that started deleting single files because a path was
            // mistyped is worse than one that reports the mistake.
            self.failures.push(RemovalFailure {
                path: path.to_path_buf(),
                op: RemovalOp::Inspect,
                kind: io::ErrorKind::NotADirectory,
                os_error: None,
                attempts: 1,
                message: "not a directory".to_string(),
            });
            self.gave_up = true;
            return Visited::Handled;
        }

        if entry.readonly && !matches!(entry.kind, EntryKind::Link { .. }) {
            self.fs.clear_readonly(path);
        }

        match entry.kind {
            EntryKind::File | EntryKind::Link { directory: false } => {
                let removed =
                    self.attempt(path, RemovalOp::RemoveFile, || self.fs.remove_file(path));
                if let Err(failure) = removed {
                    self.record(failure);
                }
            }
            EntryKind::Link { directory: true } => {
                let removed = self.attempt(path, RemovalOp::RemoveDir, || self.fs.remove_dir(path));
                if let Err(failure) = removed {
                    self.record(failure);
                }
            }
            EntryKind::Dir => {
                let listed = self.attempt(path, RemovalOp::ReadDir, || self.fs.read_dir(path));
                let children = match listed {
                    Ok(Some(children)) => children,
                    Ok(None) => return Visited::Handled,
                    Err(failure) => {
                        self.record(failure);
                        return Visited::Handled;
                    }
                };
                stack.push(Step::Finish {
                    path: path.to_path_buf(),
                    mark: self.failures.len(),
                });
                for child in children.into_iter().rev() {
                    stack.push(Step::Visit(child));
                }
            }
        }
        Visited::Handled
    }

    /// `Ok(None)` is the entry being gone, which every operation here treats
    /// as the outcome it wanted.
    fn attempt<T>(
        &self,
        path: &Path,
        op: RemovalOp,
        mut call: impl FnMut() -> io::Result<T>,
    ) -> Result<Option<T>, RemovalFailure> {
        let delays = self.delays();
        let mut attempts: u32 = 1;
        loop {
            let error = match call() {
                Ok(value) => return Ok(Some(value)),
                Err(error) => error,
            };
            match disposition(&error, self.transient) {
                Disposition::Absent => return Ok(None),
                Disposition::Transient if (attempts as usize) <= delays.len() => {
                    self.fs.sleep(delays[attempts as usize - 1]);
                    attempts += 1;
                }
                _ => return Err(RemovalFailure::new(path, op, &error, attempts)),
            }
        }
    }

    /// Nothing is retried once one path has been given up on.
    ///
    /// Whatever holds a handle on one file in a tree — a scanner, an indexer,
    /// an editor — holds it on the neighbours too, so the alternative is the
    /// full budget multiplied by the file count, for a tree that is going onto
    /// the cleanup queue either way. Deleting what can be deleted still frees
    /// the disk and shrinks the retry.
    fn delays(&self) -> &[Duration] {
        if self.gave_up {
            &[]
        } else {
            self.backoff
        }
    }

    fn record(&mut self, failure: RemovalFailure) {
        self.gave_up = true;
        self.failures.push(failure);
    }
}

#[cfg(test)]
#[path = "../../tests/shared/fs_remove.rs"]
mod tests;
