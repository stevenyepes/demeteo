use std::process::Command;

#[cfg(target_os = "linux")]
const APPIMAGE_MOUNT_PREFIX: &str = "/tmp/.mount_";

#[cfg(target_os = "linux")]
fn strip_appimage_entries(raw: &str) -> Option<String> {
    let kept: Vec<&str> = raw
        .split(':')
        .filter(|e| !e.starts_with(APPIMAGE_MOUNT_PREFIX))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(":"))
    }
}

/// Run the child in its own session (no controlling terminal) so an
/// **interactive** shell (`bash -i`) can't seize *our* controlling terminal
/// for job control. Without this, a locally-spawned `bash -l -i -c` succeeds
/// at `tcsetpgrp` on the terminal shared with `tauri dev`/`npm`, leaving the
/// foreground process group pointing at the (now-exited) child — the parent
/// shell's next terminal read then raises SIGTTIN and stops the whole process
/// group (`suspended (tty input)`, app goes unresponsive). With no controlling
/// terminal the shell just prints "no job control in this shell" to stderr and
/// runs normally. `setsid(2)` is async-signal-safe, so it's safe in `pre_exec`.
/// A no-op on Windows.
#[cfg_attr(not(unix), allow(unused_variables))]
pub fn detach_from_controlling_tty(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and we ignore its result — it
        // only fails (EPERM) when we're already a process-group leader, in
        // which case we still lack a controlling terminal, which is what we
        // want. Nothing else runs in the forked child before exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
}

/// Ask `CreateProcess` for no console window. `DETACHED_PROCESS` (`0x8`) is
/// one digit away and is not a substitute: it also detaches the child from the
/// job and console it is meant to stay inside.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What every non-PTY Windows child is told, and — as much of the point — what
/// it is deliberately not told.
///
/// `NoDefaultCurrentDirectoryInExePath` removes the working directory from the
/// search `CreateProcess` performs for the executable it is handed
/// (GHSA-2mqj-m65w-jghx). Demeteo's working directory is a worktree an agent
/// writes into, so without it an agent that commits a `git.exe` beside its
/// source has that file run as git.
///
/// `MSYS2_ENV_CONV_EXCL=*` stops the MSYS runtime rewriting the *values* of
/// environment variables Demeteo sets: they are Windows paths, and a POSIX
/// translation of them names nothing. Its sibling `MSYS2_ARG_CONV_EXCL` is
/// left unset on purpose — a user's script body legitimately passes `/c/...`
/// to a native `.exe` and expects the conversion. `MSYS_NO_PATHCONV` is not
/// used for either job: it disables conversion by being *defined*, so a `0`
/// does not turn it back on.
pub const WINDOWS_CHILD_ENV: [(&str, &str); 2] = [
    ("NoDefaultCurrentDirectoryInExePath", "1"),
    ("MSYS2_ENV_CONV_EXCL", "*"),
];

/// Whether an inherited variable is one of Git for Windows' own and must not
/// reach a child.
///
/// `compat/mingw.c::setup_windows_environment` prepends `<root>\usr\bin` to
/// `PATH` and enables `#!` resolution only when `MSYSTEM` is *unset*: a git
/// that sees an inherited value concludes it is already inside an MSYS shell
/// and does neither, which silently stops the user's own hooks running. Since
/// Demeteo runs script bodies under Git Bash, every child of one inherits it.
pub fn is_msys_env_var(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name == "MSYSTEM" || name == "MSYS" || name.starts_with("MSYS2_")
}

/// Whether a variable is removed from a child's block outright.
///
/// [`is_msys_env_var`] on its own is not that question: `MSYS2_ENV_CONV_EXCL`
/// is one of Git for Windows' own *and* one [`WINDOWS_CHILD_ENV`] sets, so a
/// removal pass keyed on the predicate alone is only correct while it runs
/// before the set pass — an ordering the compiler does not check and no test
/// on this host can observe, since the passes are behind `cfg(windows)`.
/// Deciding it here makes the two commute.
pub fn must_strip_from_child(name: &str) -> bool {
    is_msys_env_var(name)
        && !WINDOWS_CHILD_ENV
            .iter()
            .any(|(set, _)| set.eq_ignore_ascii_case(name))
}

/// Spawn hygiene for every child that is not a PTY: no console window, no
/// executable search through the working directory, and none of the MSYS
/// state a Git Bash ancestor left in the environment.
///
/// Never call this on a `portable-pty` child — a terminal session's whole
/// purpose is the console this suppresses.
///
/// The creation flags are *set*, not merged, because Win32 offers no way to
/// read back what a `Command` already carries. A site that needs another flag
/// passes [`CREATE_NO_WINDOW`] itself, after this.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn harden_child_spawn(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let explicit: Vec<std::ffi::OsString> = cmd
            .get_envs()
            .map(|(name, _)| name.to_os_string())
            .collect();
        for name in std::env::vars_os()
            .map(|(name, _)| name)
            .chain(explicit)
            .filter(|name| must_strip_from_child(&name.to_string_lossy()))
        {
            cmd.env_remove(&name);
        }
        for (name, value) in WINDOWS_CHILD_ENV {
            cmd.env(name, value);
        }
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn sanitize_child_env(
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] cmd: &mut Command,
) {
    #[cfg(target_os = "linux")]
    {
        for var in ["LD_LIBRARY_PATH", "LD_PRELOAD"] {
            let Some(raw) = std::env::var_os(var) else {
                continue;
            };
            let Some(raw) = raw.to_str() else {
                continue;
            };
            match strip_appimage_entries(raw) {
                Some(cleaned) => {
                    cmd.env(var, cleaned);
                }
                None => {
                    cmd.env_remove(var);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/shared/proc.rs"]
mod tests;
