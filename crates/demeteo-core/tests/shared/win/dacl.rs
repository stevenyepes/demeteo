use super::*;
use std::path::{Path, PathBuf};

const WRITE_DAC: u32 = 0x0004_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_READ_DATA: u32 = 0x0000_0001;
/// `INHERIT_ONLY_ACE`. The flag Windows adds when it keeps an inheritable ACE
/// on a container as an effective entry plus a propagate-only twin.
const INHERIT_ONLY_ACE: u32 = 0x0000_0008;

fn worktree() -> PathBuf {
    PathBuf::from("/w/repo_wt_a1b2c3d4")
}

fn dir(name: &str) -> Entry {
    Entry {
        path: worktree().join(name),
        is_dir: true,
        is_symlink: false,
    }
}

fn file(name: &str) -> Entry {
    Entry {
        path: worktree().join(name),
        is_dir: false,
        is_symlink: false,
    }
}

fn link(name: &str) -> Entry {
    Entry {
        path: worktree().join(name),
        is_dir: false,
        is_symlink: true,
    }
}

fn fenced(entries: &[Entry], writable: &[&str]) -> Vec<PathBuf> {
    let writable: Vec<PathBuf> = writable.iter().map(PathBuf::from).collect();
    fence_plan(entries, &writable, &worktree())
        .into_iter()
        .map(|ace| ace.path)
        .collect()
}

/// A mask that denies writing but not deleting is not a fence — an agent
/// removes the file it was refused permission to edit. Both delete rights have
/// to be in it, because Windows honours a delete that either the child or its
/// parent directory permits.
#[test]
fn the_mask_denies_deleting_as_well_as_writing() {
    for right in [
        FILE_WRITE_DATA,
        FILE_APPEND_DATA,
        FILE_WRITE_EA,
        FILE_WRITE_ATTRIBUTES,
        FILE_DELETE_CHILD,
        DELETE,
    ] {
        assert_eq!(DENY_MASK & right, right, "missing right {right:#x}");
    }
}

/// Denying `WRITE_DAC` would deny Demeteo's own teardown, which runs under the
/// same token the ACE names. `GENERIC_WRITE` maps onto `WRITE_DAC` for a file
/// object, so it is the same mistake spelled shorter.
#[test]
fn the_mask_leaves_the_fences_own_removal_reachable() {
    assert_eq!(DENY_MASK & WRITE_DAC, 0);
    assert_eq!(DENY_MASK & GENERIC_WRITE, 0);
}

/// The fence covers what the step may not write and nothing else — the same
/// selection the Unix `chmod a-w` walk makes from the same inputs.
#[test]
fn only_the_non_writable_top_level_entries_are_fenced() {
    let entries = [
        dir("src"),
        dir("artifacts"),
        dir("docs"),
        file("Cargo.toml"),
    ];
    assert_eq!(
        fenced(&entries, &["artifacts"]),
        vec![
            worktree().join("src"),
            worktree().join("docs"),
            worktree().join("Cargo.toml"),
        ]
    );
}

/// A writable path nested under a top-level entry keeps that entry unfenced:
/// an inheritable deny on `docs` would deny creating the very file the step
/// declared it would write.
#[test]
fn a_top_level_entry_a_writable_path_lies_under_is_left_alone() {
    let entries = [dir("docs"), dir("src")];
    assert_eq!(
        fenced(&entries, &["docs/report.md"]),
        vec![worktree().join("src")]
    );
}

/// A `ReadOnly` step arrives with an empty writable set, and every entry is
/// then fenced — the sentinel that means it has already been resolved by
/// `apply_artifact_scope` before the plan is built.
#[test]
fn an_empty_writable_set_fences_everything() {
    let entries = [dir("src"), dir("artifacts")];
    assert_eq!(
        fenced(&entries, &[]),
        vec![worktree().join("src"), worktree().join("artifacts")]
    );
}

/// `node_modules` in a provisioned worktree is a link into the feature's
/// shared dependency cache. `SetNamedSecurityInfoW` cannot address a reparse
/// point without following it, so fencing one would fence a directory outside
/// the worktree that outlives this step.
#[test]
fn a_symlinked_dependency_cache_is_never_touched() {
    let entries = [link("node_modules"), dir("src")];
    assert_eq!(fenced(&entries, &[]), vec![worktree().join("src")]);
    assert_eq!(
        revoke_candidates(&entries),
        vec![worktree().join("src")],
        "teardown must skip it too, or it would rewrite the cache's ACL"
    );
}

/// The worktree root gets no ACE of its own: an inheritable deny there would
/// propagate into `artifacts/`, where the agent's access is an inherited grant
/// that a deny outranks.
#[test]
fn the_worktree_root_never_appears_in_the_plan() {
    let root = Entry {
        path: worktree(),
        is_dir: true,
        is_symlink: false,
    };
    let entries = [root, dir("src")];
    let planned = fenced(&entries, &[]);
    assert!(
        !planned.contains(&worktree()),
        "the root was fenced: {planned:?}"
    );
}

/// A directory's ACE has to reach the files created after the fence goes up,
/// or the agent's own new files are the ones it does not cover. A leaf carries
/// no inheritance flags at all, so that teardown recognises its own ACE.
#[test]
fn a_directory_fences_its_future_children_and_a_file_fences_only_itself() {
    let entries = [dir("src"), file("Cargo.toml")];
    let plan = fence_plan(&entries, &[], &worktree());
    assert_eq!(
        plan[0].inheritance,
        CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE
    );
    assert_eq!(plan[1].inheritance, NO_INHERITANCE);
    assert!(plan.iter().all(|ace| ace.mask == DENY_MASK));
}

fn fence_ace() -> Ace {
    Ace {
        kind: AceKind::Deny,
        mask: DENY_MASK,
        flags: CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE,
        sid: vec![1, 5, 0, 0],
    }
}

/// Teardown writes a DACL only where it finds the ACE it wrote itself.
/// Anything else on the object was put there by someone else, and removing it
/// would make the teardown a policy change rather than an inverse.
#[test]
fn teardown_recognises_only_the_fences_own_ace() {
    let sid = vec![1u8, 5, 0, 0];
    assert!(carries_fence(&[fence_ace()], &sid));

    let another_user = Ace {
        sid: vec![1, 5, 0, 1],
        ..fence_ace()
    };
    let allowing = Ace {
        kind: AceKind::Allow,
        ..fence_ace()
    };
    let a_wider_mask = Ace {
        mask: DENY_MASK | WRITE_DAC,
        ..fence_ace()
    };
    let a_read_deny = Ace {
        mask: FILE_READ_DATA,
        ..fence_ace()
    };
    let undecoded = Ace {
        kind: AceKind::Other,
        mask: 0,
        sid: Vec::new(),
        ..fence_ace()
    };
    let propagated_from_above = Ace {
        flags: fence_ace().flags | INHERITED_ACE,
        ..fence_ace()
    };
    for ace in [
        another_user,
        allowing,
        a_wider_mask,
        a_read_deny,
        undecoded,
        propagated_from_above.clone(),
    ] {
        assert!(!is_fence_ace(&ace, &sid), "matched {ace:?}");
    }
    assert!(!carries_fence(&[propagated_from_above], &sid));
}

/// The failure this predicate exists in its current shape to fix. On a real
/// Windows worktree the fence went up, denied every write it was meant to,
/// and then teardown recognised nothing and reported success over a `src/`
/// that was still unwritable. Windows may store an ACE with flags other than
/// the ones `SetEntriesInAclW` was handed — an inheritable entry on a
/// container can be kept as an effective ACE plus an `INHERIT_ONLY_ACE` twin —
/// so anything that turns on a byte-exact round-trip of the flags can match
/// nothing at all.
#[test]
fn a_fence_ace_whose_flags_windows_stored_differently_is_still_recognised() {
    let sid = vec![1u8, 5, 0, 0];
    for flags in [
        NO_INHERITANCE,
        CONTAINER_INHERIT_ACE,
        OBJECT_INHERIT_ACE,
        CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE | INHERIT_ONLY_ACE,
    ] {
        let stored = Ace {
            flags,
            ..fence_ace()
        };
        assert!(is_fence_ace(&stored, &sid), "skipped {stored:?}");
    }
}

/// Same argument for the mask. A deny naming this token's user and denying
/// only rights the fence already denies is the fence's, or has the fence's
/// effect; leaving it up strands the worktree, and taking it down costs at
/// most a right the user was denying themselves. An empty mask denies nothing
/// and is nobody's fence.
#[test]
fn a_fence_ace_stored_with_fewer_of_its_own_rights_is_still_recognised() {
    let sid = vec![1u8, 5, 0, 0];
    for mask in [DELETE, FILE_WRITE_DATA | FILE_APPEND_DATA, DENY_MASK] {
        let stored = Ace {
            mask,
            ..fence_ace()
        };
        assert!(is_fence_ace(&stored, &sid), "skipped {stored:?}");
    }
    let denies_nothing = Ace {
        mask: 0,
        ..fence_ace()
    };
    assert!(!is_fence_ace(&denies_nothing, &sid));
}

/// Teardown rewrites the DACL rather than revoking the trustee, so what it
/// keeps is the whole question. Positions, because an ACE type this file does
/// not decode is copied back byte for byte and would be lost if the ACL were
/// rebuilt from the decoded view.
#[test]
fn the_rewritten_dacl_keeps_everything_that_is_not_the_fences_own() {
    let sid = vec![1u8, 5, 0, 0];
    let aces = vec![
        fence_ace(),
        Ace {
            kind: AceKind::Deny,
            sid: vec![1, 5, 0, 1],
            ..fence_ace()
        },
        Ace {
            kind: AceKind::Other,
            mask: 0,
            sid: Vec::new(),
            ..fence_ace()
        },
        Ace {
            kind: AceKind::Allow,
            ..fence_ace()
        },
        Ace {
            flags: fence_ace().flags | INHERITED_ACE,
            ..fence_ace()
        },
        Ace {
            mask: DELETE,
            flags: NO_INHERITANCE,
            ..fence_ace()
        },
    ];
    assert_eq!(retained_ace_indices(&aces, &sid), vec![1, 2, 3, 4]);
}

/// An ACL with no fence ACE on it is one teardown must not write at all: a
/// rewrite that changes nothing still resets the object's inheritance state,
/// and the entries teardown asks about include every one the fence
/// deliberately left writable.
#[test]
fn an_untouched_acl_retains_every_position_it_had() {
    let sid = vec![1u8, 5, 0, 0];
    let aces = vec![
        Ace {
            kind: AceKind::Allow,
            ..fence_ace()
        },
        Ace {
            kind: AceKind::Deny,
            sid: vec![1, 5, 0, 1],
            ..fence_ace()
        },
    ];
    assert_eq!(retained_ace_indices(&aces, &sid), vec![0, 1]);
    assert!(!carries_fence(&aces, &sid));
}

/// The header byte is classified once, here, so that every other rule in this
/// file can compare a kind instead of re-deriving one — and so that a type the
/// fence never writes is never mistaken for one it does.
#[test]
fn only_the_two_ace_types_the_fence_understands_are_decoded() {
    assert_eq!(ace_kind(ACCESS_ALLOWED_ACE_TYPE), AceKind::Allow);
    assert_eq!(ace_kind(ACCESS_DENIED_ACE_TYPE), AceKind::Deny);
    for object_or_callback in [0x05u8, 0x06, 0x09, 0x0a, 0x0b] {
        assert_eq!(ace_kind(object_or_callback), AceKind::Other);
    }
}

/// Teardown has no record of which entries were fenced — a crash between
/// fence and teardown leaves none — so it asks every entry and lets the ACL
/// answer.
#[test]
fn teardown_asks_about_entries_the_fence_left_writable() {
    let entries = [dir("src"), dir("artifacts")];
    assert_eq!(
        fenced(&entries, &["artifacts"]),
        vec![worktree().join("src")]
    );
    assert_eq!(
        revoke_candidates(&entries),
        vec![worktree().join("src"), worktree().join("artifacts")]
    );
}

/// The plan is built from absolute paths and a worktree to make them relative
/// against; a path that is not under the worktree is compared whole rather
/// than silently treated as relative to it.
#[test]
fn an_entry_outside_the_worktree_is_matched_on_its_whole_path() {
    let stray = Entry {
        path: PathBuf::from("/elsewhere/artifacts"),
        is_dir: true,
        is_symlink: false,
    };
    let planned = fence_plan(
        &[stray],
        &[PathBuf::from("artifacts")],
        Path::new("/w/repo_wt_a1b2c3d4"),
    );
    assert_eq!(
        planned.len(),
        1,
        "an unrelated path is not the writable one"
    );
}
