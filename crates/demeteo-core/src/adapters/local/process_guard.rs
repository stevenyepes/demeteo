//! What a spawned command's whole process tree is confined to.
//!
//! One guarantee, two mechanisms with nothing in common: a Windows Job Object
//! the child is assigned to, and on Unix the process *group* a `setsid` child
//! leads. Neither is expressible in the other's terms, which is why they are
//! two types — but the spawn sites hold one of each unconditionally and branch
//! on neither, so the guarantee reads the same from `execution.rs` on every
//! platform.

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
};

/// What the job limits, and the one thing it deliberately permits.
///
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`0x2000`) is the reaping: every
/// process still in the job dies when the last handle to it closes, which is
/// what takes a compiler down with the package manager that started it.
///
/// `JOB_OBJECT_LIMIT_BREAKAWAY_OK` (`0x800`) is for Hermes, which launches its
/// gateway with `CREATE_BREAKAWAY_FROM_JOB`; inside a job without this bit that
/// call fails `ERROR_ACCESS_DENIED` and the gateway never starts. Its
/// neighbour `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` (`0x1000`) grants the same
/// exit without the asking — every ordinary agent child would leave the tree
/// whether it meant to or not, and the reaping above would be reaping an empty
/// job. Do not reach for it because something failed to break away: a process
/// that has to say so is the entire distinction.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) const JOB_LIMIT_FLAGS: u32 = 0x2000 | 0x0800;

#[cfg(windows)]
const _: () = {
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    };

    assert!(JOB_LIMIT_FLAGS == JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK);
    assert!(JOB_LIMIT_FLAGS & JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK == 0);
};

/// The Windows Job Object a spawned tree is confined to — and nothing at all
/// anywhere else, where the process group [`KillGroupOnDrop`] signals is the
/// same guarantee by other means. Callers hold one either way, so no spawn
/// site branches on the platform.
///
/// Constructed **before** the spawn it guards. Job membership is forward-only:
/// a grandchild started before the child is assigned escapes for good, and
/// there is no re-parenting it afterwards — so the two configuration syscalls
/// that used to sit inside that window (`CreateJobObjectW`,
/// `SetInformationJobObject`) are lifted out of it, leaving only
/// `AssignProcessToJobObject`.
///
/// **The window is narrowed, not closed, and nothing here should be read as
/// claiming otherwise.** Closing it means spawning `CREATE_SUSPENDED` and
/// resuming only once the assignment has landed, and `ResumeThread` needs the
/// primary thread's handle — which `std::process` closes on the way out and
/// never exposes. Reaching it costs either an open-coded `CreateProcessW`,
/// re-implementing std's stdio and handle-inheritance plumbing along with it,
/// or a crate; the crate is an AGENTS.md §6 gate and has not been asked for.
///
/// Every operation is best effort. A job that could not be created, could not
/// be configured, or could not adopt its child degrades to `kill_on_drop`'s
/// direct-child kill — the guarantee this adapter had before any job existed —
/// and says so at debug level. Returning `Err` instead, as this once did, makes
/// a teardown guarantee a precondition for running anything at all, and
/// `AssignProcessToJobObject` answers `ERROR_ACCESS_DENIED` for a process that
/// has already exited: a command fast enough to finish first would have failed
/// *because* it succeeded. Cargo's `util/job.rs` degrades for the same reason.
#[derive(Default)]
pub(super) struct ProcessGuard {
    #[cfg(windows)]
    job: Option<WindowsJob>,
}

impl ProcessGuard {
    /// Create the job the next spawn is to be assigned to.
    pub(super) fn armed() -> Self {
        Self {
            #[cfg(windows)]
            job: WindowsJob::create(),
        }
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    pub(super) fn adopt(&self, child: &tokio::process::Child) {
        // `tokio::process::Child` exposes its handle as an `Option` rather
        // than implementing `AsRawHandle`: the handle is gone once the child
        // has been reaped, and nothing needs confining after that.
        #[cfg(windows)]
        self.assign(child.raw_handle().map(|handle| handle as HANDLE));
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    pub(super) fn adopt_sync(&self, child: &std::process::Child) {
        #[cfg(windows)]
        self.assign(Some(child.as_raw_handle() as HANDLE));
    }

    #[cfg(windows)]
    fn assign(&self, process: Option<HANDLE>) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        match process {
            Some(process) => job.adopt(process),
            None => tracing::debug!("child exited before it could be confined to a job"),
        }
    }

    /// Kill the tree now. The caller kills the direct child regardless, so a
    /// failure here costs the grandchildren and nothing else.
    pub(super) fn terminate(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            job.terminate();
        }
    }

    /// The command finished on its own, so what is still running is what it
    /// deliberately left running: clearing the limits lets that outlive the
    /// handle. [`KillGroupOnDrop::disarm`] concedes the same thing at the same
    /// point on Unix, and a command whose background daemon survives on one
    /// platform and not the other is a parity break.
    pub(super) fn disarm(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            job.disarm();
        }
    }
}

#[cfg(windows)]
struct WindowsJob(HANDLE);

// SAFETY: a Win32 `HANDLE` is a process-wide kernel-object reference, not a
// pointer into this process's address space; every operation performed on it
// here (`SetInformationJobObject`, `AssignProcessToJobObject`,
// `TerminateJobObject`, `CloseHandle`) is documented as thread-safe. The
// wrapper owns it exclusively and closes it
// exactly once, in `Drop`. Needed because `LocalChildProcess` is handed out as
// a `Box<dyn InteractiveHandle>`, which the port requires to be `Send + Sync`.
#[cfg(windows)]
unsafe impl Send for WindowsJob {}
#[cfg(windows)]
unsafe impl Sync for WindowsJob {}

#[cfg(windows)]
impl WindowsJob {
    fn create() -> Option<Self> {
        // SAFETY: a null name asks for an unnamed job nothing else can open,
        // and a null attribute pointer for the default security descriptor.
        // The returned handle is owned by the value below and closed exactly
        // once, in Drop.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "no Windows Job Object for this spawn"
            );
            return None;
        }
        let job = Self(handle);
        job.set_limits(JOB_LIMIT_FLAGS).then_some(job)
    }

    fn set_limits(&self, flags: u32) -> bool {
        // SAFETY: every field of this structure is an integer or a pointer,
        // for all of which all-zero is a valid value — it is the documented
        // way to build one, since a set bit in `LimitFlags` is what gives any
        // other field meaning.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: `limits` is a fully initialised value of exactly the type
        // `JobObjectExtendedLimitInformation` names, the length passed is that
        // type's own size, and `self.0` is live until Drop.
        let configured = unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                flags,
                "Windows Job Object limits not applied"
            );
        }
        configured != 0
    }

    fn adopt(&self, process: HANDLE) {
        // SAFETY: `process` is the raw handle of a child this adapter spawned
        // and still owns, so it cannot be closed or recycled during the call;
        // `self.0` is live until Drop.
        if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "child not confined to a job; its own children will outlive a tree kill"
            );
        }
    }

    fn terminate(&self) {
        // SAFETY: self.0 is a live Job Object handle until Drop.
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "Windows Job Object not terminated"
            );
        }
    }

    fn disarm(&self) {
        self.set_limits(0);
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        // SAFETY: self.0 is exclusively owned by this wrapper and closed here
        // exactly once. Unless `disarm` ran first, KILL_ON_JOB_CLOSE takes
        // every process still inside with it.
        unsafe { CloseHandle(self.0) };
    }
}

/// Kill a spawned child's whole **process group** on drop.
///
/// `kill_on_drop` alone is not enough for this adapter: every command runs as
/// `bash -c <body>`, so killing the direct child reaps the shell and orphans
/// whatever it spawned — the `npm test` inside a hung `bash -c "npm test"`
/// would keep running (and keep writing into a worktree that is about to be
/// torn down). Killing the group takes the tree.
///
/// Armed **only** when the child called `setsid` (the `interactive` path,
/// which is what `harness_shell_options` — and therefore every `command` node
/// — uses). A child that did not `setsid` shares *demeteo's own* process
/// group, and `killpg` on that would kill the app. When disarmed the caller
/// still gets `kill_on_drop`'s direct-child kill, which is the correct floor.
pub(super) struct KillGroupOnDrop {
    pub(super) pid: Option<u32>,
    #[cfg_attr(
        not(unix),
        expect(
            dead_code,
            reason = "the group kill it arms is the Unix arm of Drop; Windows takes the tree \
                      through the job object instead"
        )
    )]
    pub(super) own_session: bool,
}

impl KillGroupOnDrop {
    /// The child exited on its own; there is no group left to signal (and a
    /// recycled pid must never be).
    pub(super) fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if let (Some(pid), true) = (self.pid, self.own_session) {
                // SAFETY: `pid` is a child we spawned with `setsid`, so it is
                // its own session and process-group leader — the group can
                // contain nothing but its descendants. ESRCH (already gone) is
                // the expected benign error and is ignored.
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/local/process_guard.rs"]
mod tests;
