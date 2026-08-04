// Tests extracted from `crates/demeteo-core/src/shared/fs_remove.rs`
// (mirrored-tests convention). `super` = that module.
//
// The whole point of the module is behaviour no Linux filesystem can be asked
// to produce — a read-only bit that blocks a delete, a handle held open for
// three attempts, a junction. `Fake` produces all three, and answers nothing
// it was not told: an unknown path is `NotFound`, and a removal of a path it
// does not hold is an error rather than a silent success, so a walk that
// visits the wrong thing fails a test instead of passing one.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    File { readonly: bool },
    Dir,
    Link { directory: bool },
}

struct Scripted {
    path: PathBuf,
    op: RemovalOp,
    remaining: usize,
    code: i32,
}

struct Fake {
    nodes: RefCell<BTreeMap<PathBuf, Node>>,
    scripted: RefCell<Vec<Scripted>>,
    log: RefCell<Vec<String>>,
    slept: RefCell<Vec<Duration>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            nodes: RefCell::new(BTreeMap::new()),
            scripted: RefCell::new(Vec::new()),
            log: RefCell::new(Vec::new()),
            slept: RefCell::new(Vec::new()),
        }
    }

    fn dir(self, path: &str) -> Self {
        self.nodes
            .borrow_mut()
            .insert(PathBuf::from(path), Node::Dir);
        self
    }

    fn file(self, path: &str) -> Self {
        self.nodes
            .borrow_mut()
            .insert(PathBuf::from(path), Node::File { readonly: false });
        self
    }

    fn readonly_file(self, path: &str) -> Self {
        self.nodes
            .borrow_mut()
            .insert(PathBuf::from(path), Node::File { readonly: true });
        self
    }

    fn link(self, path: &str, directory: bool) -> Self {
        self.nodes
            .borrow_mut()
            .insert(PathBuf::from(path), Node::Link { directory });
        self
    }

    /// Fail the next `times` calls of `op` on `path` with `code`.
    /// `usize::MAX` is a handle nobody ever closes.
    fn failing(self, path: &str, op: RemovalOp, times: usize, code: i32) -> Self {
        self.scripted.borrow_mut().push(Scripted {
            path: PathBuf::from(path),
            op,
            remaining: times,
            code,
        });
        self
    }

    /// Something else deleted it first: the call reports `ENOENT` *and* the
    /// entry is gone, which is the race the walk has to treat as success.
    fn vanishing(self, path: &str, op: RemovalOp) -> Self {
        self.failing(path, op, 1, ENOENT)
    }

    fn scripted_error(&self, path: &Path, op: RemovalOp) -> Option<io::Error> {
        let code = {
            let mut scripted = self.scripted.borrow_mut();
            let entry = scripted
                .iter_mut()
                .find(|s| s.op == op && s.path == path && s.remaining > 0)?;
            entry.remaining -= 1;
            entry.code
        };
        if code == ENOENT {
            self.nodes.borrow_mut().remove(path);
        }
        Some(io::Error::from_raw_os_error(code))
    }

    fn note(&self, op: &str, path: &Path) {
        self.log
            .borrow_mut()
            .push(format!("{op} {}", path.display()));
    }

    fn children(&self, path: &Path) -> Vec<PathBuf> {
        self.nodes
            .borrow()
            .keys()
            .filter(|candidate| candidate.parent() == Some(path))
            .cloned()
            .collect()
    }

    fn remaining(&self) -> Vec<String> {
        self.nodes
            .borrow()
            .keys()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn removals(&self) -> Vec<String> {
        self.log
            .borrow()
            .iter()
            .filter(|line| line.starts_with("remove_"))
            .cloned()
            .collect()
    }

    fn missing(path: &Path) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such path '{}'", path.display()),
        )
    }
}

impl TreeFs for Fake {
    fn inspect(&self, path: &Path) -> io::Result<Entry> {
        self.note("inspect", path);
        if let Some(error) = self.scripted_error(path, RemovalOp::Inspect) {
            return Err(error);
        }
        match self.nodes.borrow().get(path) {
            Some(Node::File { readonly }) => Ok(Entry {
                kind: EntryKind::File,
                readonly: *readonly,
            }),
            Some(Node::Dir) => Ok(Entry {
                kind: EntryKind::Dir,
                readonly: false,
            }),
            Some(Node::Link { directory }) => Ok(Entry {
                kind: EntryKind::Link {
                    directory: *directory,
                },
                readonly: false,
            }),
            None => Err(Self::missing(path)),
        }
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        self.note("read_dir", path);
        if let Some(error) = self.scripted_error(path, RemovalOp::ReadDir) {
            return Err(error);
        }
        match self.nodes.borrow().get(path) {
            Some(Node::Dir) => Ok(()),
            Some(_) => Err(io::Error::other(format!(
                "read_dir on a non-directory '{}'",
                path.display()
            ))),
            None => Err(Self::missing(path)),
        }?;
        Ok(self.children(path))
    }

    fn clear_readonly(&self, path: &Path) {
        self.note("clear_readonly", path);
        if let Some(Node::File { readonly }) = self.nodes.borrow_mut().get_mut(path) {
            *readonly = false;
        }
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.note("remove_file", path);
        if let Some(error) = self.scripted_error(path, RemovalOp::RemoveFile) {
            return Err(error);
        }
        match self.nodes.borrow().get(path) {
            Some(Node::File { readonly: true }) => {
                return Err(io::Error::from_raw_os_error(ACCESS_DENIED))
            }
            Some(Node::File { .. }) | Some(Node::Link { directory: false }) => {}
            Some(_) => {
                return Err(io::Error::other(format!(
                    "remove_file on a directory '{}'",
                    path.display()
                )))
            }
            None => return Err(Self::missing(path)),
        }
        self.nodes.borrow_mut().remove(path);
        Ok(())
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        self.note("remove_dir", path);
        if let Some(error) = self.scripted_error(path, RemovalOp::RemoveDir) {
            return Err(error);
        }
        match self.nodes.borrow().get(path) {
            Some(Node::Dir) if !self.children(path).is_empty() => {
                return Err(io::Error::from_raw_os_error(DIR_NOT_EMPTY))
            }
            Some(Node::Dir) | Some(Node::Link { directory: true }) => {}
            Some(_) => {
                return Err(io::Error::other(format!(
                    "remove_dir on a file '{}'",
                    path.display()
                )))
            }
            None => return Err(Self::missing(path)),
        }
        self.nodes.borrow_mut().remove(path);
        Ok(())
    }

    fn sleep(&self, delay: Duration) {
        self.slept.borrow_mut().push(delay);
    }
}

const ACCESS_DENIED: i32 = 5;
const SHARING_VIOLATION: i32 = 32;
const DIR_NOT_EMPTY: i32 = 145;
/// `EACCES` on Linux — a permission error that is not in the Windows table.
const EACCES: i32 = 13;
/// `ENOENT` on Linux, `ERROR_FILE_NOT_FOUND` on Windows: both map to
/// `ErrorKind::NotFound`, which is the only thing the walk reads.
const ENOENT: i32 = 2;

/// Every walk test runs the Windows table, because that is the platform whose
/// decisions are under test and the one no test here executes on.
fn run(fake: &Fake, root: &str) -> RemovalOutcome {
    remove_tree(
        fake,
        Path::new(root),
        &WINDOWS_TRANSIENT,
        &backoff_schedule(),
    )
}

fn leftovers(outcome: &RemovalOutcome) -> &[RemovalFailure] {
    match outcome {
        RemovalOutcome::Incomplete { leftovers, .. } => leftovers,
        other => panic!("expected an incomplete removal, got {other:?}"),
    }
}

#[test]
fn every_child_is_removed_before_the_directory_holding_it() {
    let fake = Fake::new()
        .dir("/wt")
        .dir("/wt/.git")
        .dir("/wt/.git/objects")
        .file("/wt/.git/objects/pack.idx")
        .file("/wt/.git/HEAD")
        .dir("/wt/src")
        .file("/wt/src/main.rs");

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert!(fake.remaining().is_empty(), "left {:?}", fake.remaining());
    assert_eq!(
        fake.removals(),
        vec![
            "remove_file /wt/.git/HEAD",
            "remove_file /wt/.git/objects/pack.idx",
            "remove_dir /wt/.git/objects",
            "remove_dir /wt/.git",
            "remove_file /wt/src/main.rs",
            "remove_dir /wt/src",
            "remove_dir /wt",
        ]
    );
}

#[test]
fn a_directory_link_is_unlinked_and_never_walked() {
    let fake = Fake::new()
        .dir("/wt")
        .link("/wt/vendor", true)
        .dir("/wt/vendor/deep")
        .file("/wt/vendor/deep/keep.txt");

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert!(
        !fake.log().iter().any(|line| line == "read_dir /wt/vendor"),
        "the walk descended into a reparse point: {:?}",
        fake.log()
    );
    assert_eq!(
        fake.remaining(),
        vec!["/wt/vendor/deep", "/wt/vendor/deep/keep.txt"],
        "content behind the link was deleted through it"
    );
    assert!(fake
        .removals()
        .contains(&"remove_dir /wt/vendor".to_string()));
}

#[test]
fn a_file_link_is_unlinked_as_a_file() {
    let fake = Fake::new().dir("/wt").link("/wt/config", false);

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert_eq!(
        fake.removals(),
        vec!["remove_file /wt/config", "remove_dir /wt"]
    );
}

#[test]
fn a_link_is_never_handed_to_the_readonly_clear() {
    // `set_permissions` follows a link, so clearing the attribute on one
    // rewrites the target's — outside the tree being deleted.
    let fake = Fake::new().dir("/wt").link("/wt/vendor", true);

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert!(
        !fake.log().iter().any(|l| l.starts_with("clear_readonly")),
        "{:?}",
        fake.log()
    );
}

#[test]
fn a_readonly_file_has_the_attribute_cleared_before_the_unlink() {
    let fake = Fake::new()
        .dir("/wt")
        .readonly_file("/wt/pack.idx")
        .file("/wt/HEAD");

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    let log = fake.log();
    let cleared = log
        .iter()
        .position(|l| l == "clear_readonly /wt/pack.idx")
        .expect("the read-only attribute was never cleared");
    let unlinked = log
        .iter()
        .position(|l| l == "remove_file /wt/pack.idx")
        .expect("the file was never unlinked");
    assert!(cleared < unlinked);
    assert!(
        !log.iter().any(|l| l == "clear_readonly /wt/HEAD"),
        "a writable file was rewritten anyway"
    );
}

#[test]
fn a_held_handle_that_lets_go_costs_only_the_early_delays() {
    let fake = Fake::new().dir("/wt").file("/wt/target.db").failing(
        "/wt/target.db",
        RemovalOp::RemoveFile,
        3,
        SHARING_VIOLATION,
    );

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert_eq!(
        *fake.slept.borrow(),
        vec![
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(4)
        ]
    );
}

#[test]
fn a_pending_delete_that_clears_lets_the_directory_go() {
    let fake = Fake::new().dir("/wt").dir("/wt/sub").failing(
        "/wt/sub",
        RemovalOp::RemoveDir,
        2,
        DIR_NOT_EMPTY,
    );

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert_eq!(fake.slept.borrow().len(), 2);
}

#[test]
fn giving_up_names_the_path_the_operation_and_the_code() {
    let fake = Fake::new().dir("/wt").file("/wt/locked.dll").failing(
        "/wt/locked.dll",
        RemovalOp::RemoveFile,
        usize::MAX,
        SHARING_VIOLATION,
    );

    let outcome = run(&fake, "/wt");
    let failures = leftovers(&outcome);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].path, PathBuf::from("/wt/locked.dll"));
    assert_eq!(failures[0].op, RemovalOp::RemoveFile);
    assert_eq!(failures[0].os_error, Some(SHARING_VIOLATION));
    assert_eq!(failures[0].attempts as usize, backoff_schedule().len() + 1);
    assert!(matches!(
        outcome,
        RemovalOutcome::Incomplete { ref root, .. } if root == Path::new("/wt")
    ));
}

#[test]
fn a_directory_holding_a_leftover_is_not_reported_as_a_second_failure() {
    let fake = Fake::new()
        .dir("/wt")
        .dir("/wt/node_modules")
        .file("/wt/node_modules/esbuild.exe")
        .failing(
            "/wt/node_modules/esbuild.exe",
            RemovalOp::RemoveFile,
            usize::MAX,
            SHARING_VIOLATION,
        );

    let outcome = run(&fake, "/wt");
    let failures = leftovers(&outcome);
    assert_eq!(
        failures
            .iter()
            .map(|f| f.path.display().to_string())
            .collect::<Vec<_>>(),
        vec!["/wt/node_modules/esbuild.exe"]
    );
}

#[test]
fn nothing_is_retried_after_the_first_path_is_given_up_on() {
    let fake = Fake::new()
        .dir("/wt")
        .file("/wt/a.dll")
        .file("/wt/b.dll")
        .failing(
            "/wt/a.dll",
            RemovalOp::RemoveFile,
            usize::MAX,
            ACCESS_DENIED,
        )
        .failing(
            "/wt/b.dll",
            RemovalOp::RemoveFile,
            usize::MAX,
            ACCESS_DENIED,
        );

    let outcome = run(&fake, "/wt");
    assert_eq!(leftovers(&outcome).len(), 2);
    assert_eq!(
        fake.slept.borrow().len(),
        backoff_schedule().len(),
        "the second locked path paid the retry budget again"
    );
    assert_eq!(leftovers(&outcome)[1].attempts, 1);
}

#[test]
fn what_can_be_deleted_is_still_deleted_around_a_leftover() {
    let fake = Fake::new()
        .dir("/wt")
        .dir("/wt/src")
        .file("/wt/src/main.rs")
        .file("/wt/locked.dll")
        .failing(
            "/wt/locked.dll",
            RemovalOp::RemoveFile,
            usize::MAX,
            ACCESS_DENIED,
        );

    let outcome = run(&fake, "/wt");
    assert_eq!(leftovers(&outcome).len(), 1);
    assert_eq!(fake.remaining(), vec!["/wt", "/wt/locked.dll"]);
}

#[test]
fn a_child_that_vanished_under_us_is_not_a_failure() {
    let fake = Fake::new()
        .dir("/wt")
        .file("/wt/ghost")
        .file("/wt/keep")
        .vanishing("/wt/ghost", RemovalOp::RemoveFile);

    assert_eq!(run(&fake, "/wt"), RemovalOutcome::Removed);
    assert!(
        fake.slept.borrow().is_empty(),
        "a vanished path was waited on"
    );
}

#[test]
fn a_missing_root_is_absent_rather_than_removed() {
    let fake = Fake::new();
    assert_eq!(
        run(&fake, "/wt"),
        RemovalOutcome::Absent {
            root: PathBuf::from("/wt")
        }
    );
}

#[test]
fn a_root_that_is_a_file_is_refused_rather_than_deleted() {
    let fake = Fake::new().file("/wt");

    let outcome = run(&fake, "/wt");
    assert_eq!(leftovers(&outcome)[0].kind, io::ErrorKind::NotADirectory);
    assert_eq!(fake.remaining(), vec!["/wt"]);
    assert!(fake.removals().is_empty());
}

#[test]
fn an_unreadable_directory_is_reported_and_its_parent_is_not() {
    let fake = Fake::new().dir("/wt").dir("/wt/priv").failing(
        "/wt/priv",
        RemovalOp::ReadDir,
        usize::MAX,
        ACCESS_DENIED,
    );

    let outcome = run(&fake, "/wt");
    let failures = leftovers(&outcome);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].op, RemovalOp::ReadDir);
}

#[test]
fn the_host_table_does_not_retry_a_permission_error_off_windows() {
    let fake = Fake::new().dir("/wt").file("/wt/denied").failing(
        "/wt/denied",
        RemovalOp::RemoveFile,
        usize::MAX,
        EACCES,
    );

    let outcome = remove_tree(&fake, Path::new("/wt"), HOST_TRANSIENT, &backoff_schedule());
    let failures = leftovers(&outcome);
    assert_eq!(failures[0].attempts, 1, "a Unix EACCES was retried");
    assert!(fake.slept.borrow().is_empty());
    if cfg!(windows) {
        assert_eq!(HOST_TRANSIENT, WINDOWS_TRANSIENT.as_slice());
    } else {
        assert!(HOST_TRANSIENT.is_empty());
    }
}

#[test]
fn the_backoff_doubles_and_outlasts_gits_own_retry() {
    let delays = backoff_schedule();
    assert_eq!(delays[0], FIRST_BACKOFF);
    for pair in delays.windows(2) {
        assert_eq!(pair[1], pair[0] * 2);
    }
    let total: Duration = delays.iter().sum();
    assert!(total >= BACKOFF_BUDGET, "{total:?}");
    assert!(total < BACKOFF_BUDGET * 2, "{total:?}");
    // `mingw_unlink` gives up after ~71ms, which is the bar this exists to
    // clear rather than match.
    assert!(total > Duration::from_millis(71) * 10);
}

#[test]
fn only_the_listed_codes_are_worth_waiting_for() {
    let transient = &WINDOWS_TRANSIENT;
    for code in [ACCESS_DENIED, SHARING_VIOLATION, DIR_NOT_EMPTY] {
        assert_eq!(
            disposition(&io::Error::from_raw_os_error(code), transient),
            Disposition::Transient,
            "code {code}"
        );
    }
    assert_eq!(
        disposition(&io::Error::from_raw_os_error(EACCES), transient),
        Disposition::Permanent
    );
    assert_eq!(
        disposition(&io::Error::other("no os code at all"), transient),
        Disposition::Permanent
    );
}

#[test]
fn a_path_that_is_already_gone_outranks_the_transient_table() {
    let not_found = io::Error::new(io::ErrorKind::NotFound, "gone");
    assert_eq!(
        disposition(&not_found, &WINDOWS_TRANSIENT),
        Disposition::Absent
    );
    assert_eq!(disposition(&not_found, HOST_TRANSIENT), Disposition::Absent);
}

#[test]
fn a_symlink_is_a_link_whichever_platform_reported_it() {
    assert_eq!(
        classify_entry(true, false),
        EntryKind::Link { directory: false }
    );
    assert_eq!(
        classify_entry(true, true),
        EntryKind::Link { directory: true }
    );
    assert_eq!(classify_entry(false, true), EntryKind::Dir);
    assert_eq!(classify_entry(false, false), EntryKind::File);
}

#[test]
fn the_port_message_keeps_the_prefix_it_always_had() {
    let removed = RemovalOutcome::Removed.into_result();
    assert_eq!(removed, Ok(()));

    let absent = RemovalOutcome::Absent {
        root: PathBuf::from("/wt"),
    }
    .into_result()
    .expect_err("an absent root still reports an error");
    assert!(
        absent.starts_with("Failed to remove directory '/wt': "),
        "{absent}"
    );

    let incomplete = RemovalOutcome::Incomplete {
        root: PathBuf::from("/wt"),
        leftovers: vec![RemovalFailure {
            path: PathBuf::from("/wt/locked.dll"),
            op: RemovalOp::RemoveFile,
            kind: io::ErrorKind::PermissionDenied,
            os_error: Some(SHARING_VIOLATION),
            attempts: 12,
            message: "the process cannot access the file".to_string(),
        }],
    }
    .into_result()
    .expect_err("a leftover still reports an error");
    assert!(incomplete.contains("/wt/locked.dll"), "{incomplete}");
    assert!(incomplete.contains("remove file"), "{incomplete}");
}

#[test]
fn the_host_filesystem_deletes_a_real_nested_tree() {
    let root = std::env::temp_dir().join(format!(
        "demeteo-fs-remove-{}-{}",
        std::process::id(),
        crate::shared::time::now_ms()
    ));
    let nested = root.join("a").join("b");
    std::fs::create_dir_all(&nested).expect("fixture");
    std::fs::write(nested.join("f.txt"), b"x").expect("fixture");

    assert_eq!(remove_dir_all(&root), RemovalOutcome::Removed);
    assert!(!root.exists());
    assert!(matches!(
        remove_dir_all(&root),
        RemovalOutcome::Absent { .. }
    ));
}
