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
