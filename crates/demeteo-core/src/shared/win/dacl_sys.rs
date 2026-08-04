//! The Windows calls the artifact-scope fence is made of.
//!
//! Everything this acts on is decided next door in `dacl.rs`, where a Linux
//! test can reach it; what is left here is one identity lookup, one directory
//! read, and the get-merge-set triple that installs or removes a single ACE.
//! Reviewing it by eye is meant to be enough, and it stays that way only while
//! nothing here decides anything.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, ACCESS_MODE, DENY_ACCESS,
    EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    GetAce, GetLengthSid, GetTokenInformation, TokenUser, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::dacl::{self, Entry};

/// The constants `dacl.rs` states by value, because it compiles for targets
/// this crate is not linked into. They are ABI-frozen, so this can only fail
/// on a typo — which is the one failure a value copied by hand actually has.
const _: () = {
    use windows_sys::Win32::Security as sec;
    use windows_sys::Win32::Storage::FileSystem as fs;

    assert!(dacl::FILE_WRITE_DATA == fs::FILE_WRITE_DATA);
    assert!(dacl::FILE_APPEND_DATA == fs::FILE_APPEND_DATA);
    assert!(dacl::FILE_WRITE_EA == fs::FILE_WRITE_EA);
    assert!(dacl::FILE_WRITE_ATTRIBUTES == fs::FILE_WRITE_ATTRIBUTES);
    assert!(dacl::FILE_DELETE_CHILD == fs::FILE_DELETE_CHILD);
    assert!(dacl::DELETE == fs::DELETE);
    assert!(dacl::NO_INHERITANCE == sec::NO_INHERITANCE);
    assert!(dacl::OBJECT_INHERIT_ACE == sec::OBJECT_INHERIT_ACE);
    assert!(dacl::CONTAINER_INHERIT_ACE == sec::CONTAINER_INHERIT_ACE);
    assert!(dacl::INHERITED_ACE == sec::INHERITED_ACE);
};

/// What one [`set_entry_ace`] call installs, as `EXPLICIT_ACCESS_W` spells it.
/// `REVOKE_ACCESS` reads neither of the other two; they are still stated so
/// the call reads the same in both directions.
struct AceSpec {
    mode: ACCESS_MODE,
    mask: u32,
    inheritance: u32,
}

/// Deny the token's user everything in [`DENY_MASK`](dacl::DENY_MASK) on every
/// top-level entry the step may not write.
///
/// Applying the fence lifts whatever an earlier attempt left behind first, so
/// a step cancelled between fence and teardown cannot leave a deny standing on
/// a path the next step is entitled to write. That is what makes the two
/// halves needing to be exact inverses a property and not a nicety.
pub fn fence(worktree: &Path, writable_paths: &[PathBuf]) -> Result<(), String> {
    unfence(worktree)?;

    let sid = token_user_sid()?;
    for ace in dacl::fence_plan(&entries(worktree)?, writable_paths, worktree) {
        set_entry_ace(
            &ace.path,
            &sid,
            AceSpec {
                mode: DENY_ACCESS,
                mask: ace.mask,
                inheritance: ace.inheritance,
            },
        )?;
    }
    Ok(())
}

/// Remove the fence's ACE wherever it is still present.
///
/// The revoke goes on the top-level entry alone. Its inheritable copies in the
/// subtree are the system's, propagated when the ACE was set, and withdrawing
/// the source is what withdraws them — which is what makes teardown cost one
/// call per top-level entry rather than one per file.
pub fn unfence(worktree: &Path) -> Result<(), String> {
    if !worktree.is_dir() {
        return Ok(());
    }
    let sid = token_user_sid()?;
    for path in dacl::revoke_candidates(&entries(worktree)?) {
        if dacl::carries_fence(&read_aces(&path)?, sid.bytes()) {
            set_entry_ace(
                &path,
                &sid,
                AceSpec {
                    mode: REVOKE_ACCESS,
                    mask: dacl::DENY_MASK,
                    inheritance: dacl::NO_INHERITANCE,
                },
            )?;
        }
    }
    Ok(())
}

/// The SID of the user this process runs as, copied out of the token before
/// the token buffer goes away.
///
/// Read from the token rather than resolved from a name: `whoami` answers
/// `AzureAD\<display name>` on an Entra-joined machine, which is a string
/// `icacls` frequently cannot resolve back to a SID, and which is stable
/// across neither a rename nor a locale.
fn token_user_sid() -> Result<Sid, String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no close,
    // and `token` is a live out-parameter closed on every path below.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(format!("open process token failed: {}", last_error()));
    }

    let sid = read_token_user(token);
    // SAFETY: `token` was opened successfully above and is not used after this.
    unsafe { CloseHandle(token) };
    sid
}

fn read_token_user(token: HANDLE) -> Result<Sid, String> {
    let mut needed: u32 = 0;
    // SAFETY: a null buffer of zero length is the documented way to ask
    // `GetTokenInformation` for the size it needs; it fails and sets `needed`.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(format!("size token user failed: {}", last_error()));
    }

    // `TOKEN_USER` holds a pointer and the SID it points at holds `u32`s, so
    // the buffer has to be aligned for both; a `Vec<u8>` is aligned for
    // neither.
    let mut buffer: Vec<u64> = vec![0; (needed as usize).div_ceil(8)];
    let mut written: u32 = needed;
    // SAFETY: the buffer holds at least `needed` bytes at pointer alignment,
    // and `written` states its size in bytes as the API requires.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut written,
        )
    };
    if read == 0 {
        return Err(format!("read token user failed: {}", last_error()));
    }

    // SAFETY: the call above succeeded, so the buffer holds a `TOKEN_USER`
    // whose `Sid` points into that same buffer, which outlives this read.
    unsafe {
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        Sid::copy_from(sid)
    }
}

/// A SID this process owns, kept aligned for the APIs that read one.
struct Sid {
    words: Vec<u32>,
    len: usize,
}

impl Sid {
    /// # Safety
    ///
    /// `sid` must point at a valid SID that outlives the call.
    unsafe fn copy_from(sid: PSID) -> Result<Sid, String> {
        let len = GetLengthSid(sid) as usize;
        if len == 0 {
            return Err(format!("measure sid failed: {}", last_error()));
        }
        let mut words: Vec<u32> = vec![0; len.div_ceil(4)];
        std::ptr::copy_nonoverlapping(sid.cast::<u8>(), words.as_mut_ptr().cast::<u8>(), len);
        Ok(Sid { words, len })
    }

    fn as_psid(&self) -> PSID {
        self.words.as_ptr() as PSID
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `words` owns at least `len` initialised bytes, and `u8`
        // imposes no alignment a `Vec<u32>` does not already meet.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast::<u8>(), self.len) }
    }
}

/// The worktree's top level, with the two facts
/// [`fence_plan`](dacl::fence_plan) selects on.
///
/// `symlink_metadata` rather than `metadata`: the question is what the entry
/// *is*, and following a junction would report the target's kind and lose the
/// only signal that says to leave that entry alone.
fn entries(worktree: &Path) -> Result<Vec<Entry>, String> {
    let read = std::fs::read_dir(worktree)
        .map_err(|e| format!("read_dir({}) failed: {}", worktree.display(), e))?;
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| format!("read_dir({}) failed: {}", worktree.display(), e))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("stat({}) failed: {}", path.display(), e))?;
        out.push(Entry {
            is_dir: meta.is_dir(),
            is_symlink: meta.is_symlink(),
            path,
        });
    }
    Ok(out)
}

/// Merge one explicit entry into the object's existing DACL and write it back.
///
/// The existing DACL is read rather than replaced, so the fence is a layer
/// over what the worktree inherited rather than a new access policy for it —
/// and so `REVOKE_ACCESS` has something to remove the ACE *from*. A null DACL
/// grants everyone everything, and merging a deny into nothing would leave the
/// object granting nobody anything, so that case is refused rather than
/// guessed at.
fn set_entry_ace(path: &Path, sid: &Sid, spec: AceSpec) -> Result<(), String> {
    let name = wide(path);
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();

    // SAFETY: `name` is a nul-terminated wide string owned by this frame, and
    // both out-parameters are live. The descriptor is freed on every path.
    let read = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if read != ERROR_SUCCESS {
        return Err(format!("read ACL of {} failed: {}", path.display(), read));
    }
    let result = write_merged_dacl(&name, path, acl, sid, spec);
    // SAFETY: `descriptor` was allocated by the successful call above, and
    // neither it nor the `acl` pointing into it is used after this.
    unsafe { LocalFree(descriptor) };
    result
}

fn write_merged_dacl(
    name: &[u16],
    path: &Path,
    current: *mut ACL,
    sid: &Sid,
    spec: AceSpec,
) -> Result<(), String> {
    if current.is_null() {
        return Err(format!("{} has no discretionary ACL", path.display()));
    }

    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: spec.mask,
        grfAccessMode: spec.mode,
        grfInheritance: spec.inheritance,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.as_psid().cast(),
        },
    };

    let mut merged: *mut ACL = std::ptr::null_mut();
    // SAFETY: `access` names a SID this process owns and which outlives the
    // call, `current` is the live DACL the caller read, and `merged` is a live
    // out-parameter freed below.
    let built = unsafe { SetEntriesInAclW(1, &access, current, &mut merged) };
    if built != ERROR_SUCCESS {
        return Err(format!(
            "build ACL for {} failed: {}",
            path.display(),
            built
        ));
    }

    // SAFETY: `name` is nul-terminated and `merged` is the ACL just built.
    let written = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            merged,
            std::ptr::null(),
        )
    };
    // SAFETY: `merged` was allocated by `SetEntriesInAclW` and is not used
    // after this.
    unsafe { LocalFree(merged.cast()) };
    if written != ERROR_SUCCESS {
        return Err(format!(
            "write ACL of {} failed: {}",
            path.display(),
            written
        ));
    }
    Ok(())
}

/// The object's allow and deny ACEs, in ACL order.
///
/// Object-typed and callback ACEs are skipped rather than decoded: they carry
/// their SID at a different offset, the fence never writes one, and
/// [`is_fence_ace`](dacl::is_fence_ace) could not match one if it did.
fn read_aces(path: &Path) -> Result<Vec<dacl::Ace>, String> {
    let name = wide(path);
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();

    // SAFETY: as in `set_entry_ace` — nul-terminated name, live
    // out-parameters, descriptor freed below.
    let read = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if read != ERROR_SUCCESS {
        return Err(format!("read ACL of {} failed: {}", path.display(), read));
    }

    // SAFETY: `acl` is either null or the live DACL of the descriptor freed
    // below, and every ACE it reports belongs to it.
    let aces = unsafe { collect_aces(acl) };
    // SAFETY: `descriptor` was allocated by the successful call above, and
    // nothing pointing into it is used after this.
    unsafe { LocalFree(descriptor) };
    Ok(aces)
}

/// # Safety
///
/// `acl` must be null or point at a valid ACL that outlives the call.
unsafe fn collect_aces(acl: *mut ACL) -> Vec<dacl::Ace> {
    if acl.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0..u32::from((*acl).AceCount) {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        if GetAce(acl, index, &mut ace) == 0 || ace.is_null() {
            continue;
        }
        let header = *ace.cast::<ACE_HEADER>();
        let denies = header.AceType == dacl::ACCESS_DENIED_ACE_TYPE;
        if !denies && header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        let sid: PSID = ace.cast::<u8>().add(SID_OFFSET).cast();
        out.push(dacl::Ace {
            denies,
            mask: *ace.cast::<u8>().add(MASK_OFFSET).cast::<u32>(),
            flags: u32::from(header.AceFlags),
            sid: std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize).to_vec(),
        });
    }
    out
}

/// An allow or deny ACE is a header, then a `u32` mask, then the SID inline.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const MASK_OFFSET: usize = std::mem::size_of::<ACE_HEADER>();
const SID_OFFSET: usize = MASK_OFFSET + std::mem::size_of::<u32>();

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_error() -> u32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default() as u32
}
