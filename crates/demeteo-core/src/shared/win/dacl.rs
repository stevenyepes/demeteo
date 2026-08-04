//! What the Windows artifact-scope fence denies, and which objects get it.
//!
//! The policy this serves belongs to
//! `adapters/worktree/git_ops/scope.rs`, whose module header states what the
//! fence is worth. What lives here is everything that fence has to *decide*:
//! the access mask, the inheritance flags, which
//! top-level entries are covered, and whether an ACL already carries the
//! fence's own ACE. None of it calls Windows, so all of it is reachable from a
//! Linux test — the reason given in this directory's module header, which
//! applies with extra force to a security boundary. `dacl_sys.rs` holds the
//! calls that cannot be.

use std::path::{Path, PathBuf};

/// `FILE_ADD_FILE` seen from a directory: the right that creates a new file
/// in it, and the right that overwrites an existing one.
pub const FILE_WRITE_DATA: u32 = 0x0000_0002;
/// `FILE_ADD_SUBDIRECTORY` seen from a directory.
pub const FILE_APPEND_DATA: u32 = 0x0000_0004;
pub const FILE_WRITE_EA: u32 = 0x0000_0010;
pub const FILE_DELETE_CHILD: u32 = 0x0000_0040;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
pub const DELETE: u32 = 0x0001_0000;

/// The rights a fenced entry denies to the token's own user.
///
/// Both delete rights are here and both are load-bearing: Windows permits a
/// delete when the child grants `DELETE` **or** its directory grants
/// `FILE_DELETE_CHILD`, so a mask naming one of them leaves the other route
/// open — and an agent deletes `src/main.rs` through a fence that will not let
/// it edit the same file.
///
/// `WRITE_DAC` is deliberately absent, and adding it would be a regression
/// rather than a hardening. Demeteo's own teardown runs under the same token,
/// so an ACE denying `WRITE_DAC` would deny the revoke that lifts the fence —
/// and it would buy nothing, because the user owns a worktree Demeteo created
/// on their behalf and an owner is granted `WRITE_DAC` implicitly whatever the
/// DACL says.
pub const DENY_MASK: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | FILE_DELETE_CHILD
    | DELETE;

pub const NO_INHERITANCE: u32 = 0x0;
pub const OBJECT_INHERIT_ACE: u32 = 0x1;
pub const CONTAINER_INHERIT_ACE: u32 = 0x2;
pub const INHERITED_ACE: u32 = 0x10;

/// The flags that make one ACE on a directory cover the whole subtree under
/// it, including what is created after the fence goes up.
///
/// Stamping the files that exist at fence time instead leaves everything the
/// agent creates during its own turn unfenced, and costs one call per file
/// over a `node_modules` tree. Inheritance buys both: coverage of what does
/// not exist yet, at one call per top-level entry.
pub const SUBTREE_INHERITANCE: u32 = CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE;

/// `ACCESS_DENIED_ACE_TYPE`, the one ACE header type the fence writes and the
/// only one [`is_fence_ace`] will match.
pub const ACCESS_DENIED_ACE_TYPE: u8 = 1;

/// One entry of the worktree's top level, as the fence needs to see it.
///
/// `is_dir` and `is_symlink` are read without following the path: a directory
/// symlink is `is_symlink`, never `is_dir`, which is what keeps the two rules
/// below from overlapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// One deny ACE and the object it goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyAce {
    pub path: PathBuf,
    pub mask: u32,
    pub inheritance: u32,
}

/// An access-allowed or access-denied ACE already on an object, reduced to the
/// four fields [`is_fence_ace`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    pub denies: bool,
    pub mask: u32,
    pub flags: u32,
    pub sid: Vec<u8>,
}

/// Which entries get an ACE, mirroring the Unix `chmod a-w` walk one for one.
///
/// Three rules, each with a consequence that is not visible from the call:
///
/// - **Per top-level entry, never the worktree root.** A single inheritable
///   deny on the root would propagate into `artifacts/` too, and there is no
///   explicit allow to countermand it — the agent's access to its own artifact
///   directory comes from an *inherited* grant, which a deny ACE outranks. The
///   step would then be denied the writes it exists to make.
/// - **`writable_paths` is compared in both directions.** `rel` under a
///   writable path is writable, and so is an entry a writable path lies
///   *under*: `artifacts/report.md` makes the `artifacts` entry writable, or
///   the fence would deny the directory the file has to be created in.
/// - **Symlinks and junctions are skipped**, for the reason the Unix arm's
///   `[ -L ]` guard is load-bearing there. `SetNamedSecurityInfoW` has no
///   open-reparse-point mode, so it edits the DACL of the *target* — which for
///   `node_modules` is the feature's shared dependency cache, outside the
///   worktree and outliving this step. Fencing one `ArtifactsOnly` step would
///   leave every later step of the feature unable to write the cache, and
///   nothing would lift it. Nothing is lost: a reparse point's own DACL is not
///   what governs access through it, its target is by construction outside the
///   tree the fence reasons about, and the diff guard still sees the link.
pub fn fence_plan(entries: &[Entry], writable_paths: &[PathBuf], worktree: &Path) -> Vec<DenyAce> {
    entries
        .iter()
        .filter(|entry| entry.path != worktree)
        .filter(|entry| !entry.is_symlink)
        .filter(|entry| !is_writable(&relative(&entry.path, worktree), writable_paths))
        .map(|entry| DenyAce {
            path: entry.path.clone(),
            mask: DENY_MASK,
            inheritance: inheritance_for(entry.is_dir),
        })
        .collect()
}

/// The entries teardown asks the ACL of. Every non-symlink entry is asked,
/// including the ones [`fence_plan`] left writable: which entries were fenced
/// is not recorded anywhere, and re-deriving it would need the step's
/// `writable_paths`, which a teardown running after a crash does not have.
/// [`is_fence_ace`] is what makes asking harmless — an entry that never
/// carried the fence's ACE is not written to at all.
pub fn revoke_candidates(entries: &[Entry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| !entry.is_symlink)
        .map(|entry| entry.path.clone())
        .collect()
}

/// A directory fences everything under it, now and later; a file fences only
/// itself. Inheritance flags on a leaf object are meaningless to Windows, and
/// stating them anyway would make the fence's own ACE unrecognisable to
/// [`is_fence_ace`] on teardown.
pub fn inheritance_for(is_dir: bool) -> u32 {
    if is_dir {
        SUBTREE_INHERITANCE
    } else {
        NO_INHERITANCE
    }
}

/// Whether this object's DACL still carries the ACE the fence itself wrote.
///
/// The match is exact on all four fields, which is what makes teardown a true
/// inverse rather than a reset:
///
/// - **Not inherited.** A copy of the ACE propagated down from a fenced parent
///   is removed by revoking the parent's, and revoking on the child instead
///   would rewrite a DACL the system is about to recompute anyway.
/// - **Exactly [`DENY_MASK`].** A deny ACE naming the same user with some
///   other mask was put there by someone else, and removing it would make
///   teardown a policy change.
/// - **Denying, not allowing.** The user's inherited full-control grant is
///   the thing the fence is layered over; matching it would revoke the access
///   the fence exists to restore.
pub fn is_fence_ace(ace: &Ace, sid: &[u8]) -> bool {
    ace.denies && ace.mask == DENY_MASK && ace.flags & INHERITED_ACE == 0 && ace.sid == sid
}

/// Whether any ACE on the object is the fence's own.
pub fn carries_fence(aces: &[Ace], sid: &[u8]) -> bool {
    aces.iter().any(|ace| is_fence_ace(ace, sid))
}

fn is_writable(rel: &Path, writable_paths: &[PathBuf]) -> bool {
    writable_paths
        .iter()
        .any(|allowed| rel.starts_with(allowed) || allowed.starts_with(rel))
}

fn relative(path: &Path, worktree: &Path) -> PathBuf {
    path.strip_prefix(worktree).unwrap_or(path).to_path_buf()
}

#[cfg(test)]
#[path = "../../../tests/shared/win/dacl.rs"]
mod tests;
