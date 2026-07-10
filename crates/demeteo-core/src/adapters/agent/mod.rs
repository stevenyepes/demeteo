pub mod claude_code;
pub mod cli_runtime;
pub mod codex;
pub mod direct_execution;
pub mod event_stream;
pub mod hermes;
pub mod install;
pub mod noop;
pub mod opencode;
pub mod registry;
pub mod stub_runtime;

// Shared test stubs for `hermes::tests`, `claude_code::tests`, and
// `opencode::tests`. Each of those test files imports the stubs via
// `use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec}`
// instead of redeclaring the same `#[path = "_arg_test_stubs.rs"]`
// `mod stubs` three times — which would trip the
// `clippy::duplicate-mod` lint because the same source file gets
// loaded into the crate graph more than once.
#[cfg(test)]
pub(crate) mod test_stubs;

pub fn resolve_local_binary_path(binary: &str) -> Option<String> {
    if let Ok(path_var) = std::env::var("PATH") {
        for path in std::env::split_paths(&path_var) {
            let bin_path = path.join(binary);
            #[cfg(target_os = "windows")]
            {
                let mut has_ext = false;
                if let Some(ext) = bin_path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if ext_str == "exe" || ext_str == "cmd" || ext_str == "bat" {
                        has_ext = true;
                    }
                }
                if has_ext {
                    if bin_path.is_file() {
                        return Some(bin_path.to_string_lossy().to_string());
                    }
                } else {
                    for ext in &["exe", "cmd", "bat"] {
                        let path_with_ext = bin_path.with_extension(ext);
                        if path_with_ext.is_file() {
                            return Some(path_with_ext.to_string_lossy().to_string());
                        }
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                if bin_path.is_file() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if let Ok(meta) = bin_path.metadata() {
                            let mode = meta.mode();
                            let is_executable = mode & 0o111 != 0;
                            if is_executable {
                                return Some(bin_path.to_string_lossy().to_string());
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    return Some(bin_path.to_string_lossy().to_string());
                }
            }
        }
    }

    // Fallback: resolve via an **interactive login** shell so we get profile
    // additions (homebrew, nvm, mise, pyenv, etc.). We use bash explicitly
    // rather than the SHELL env var because SHELL might be set to e.g.
    // /bin/zsh which doesn't source ~/.bashrc on macOS/Linux when invoked as
    // "zsh -l".
    //
    // `-i` is load-bearing: the common developer tool-managers (`mise`,
    // `asdf`, `nvm`) put binaries on `PATH` from `~/.bashrc`, behind the
    // standard non-interactive guard (`case $- in *i*) ;; *) return;; esac`).
    // A plain `bash -l -c` hits that guard and returns *before* the tool is on
    // `PATH`, so it reports a correctly-installed agent as "missing" — the
    // exact mismatch behind a remote run's readiness probe saying "opencode
    // isn't installed" while an interactive SSH session runs it fine. This
    // must stay consistent with `ShellOptions::login_interactive()` (the SSH
    // adapter's probe/spawn) so "available" and "runnable" always agree. Job-
    // control warnings from an interactive shell without a TTY go to stderr,
    // so stdout stays clean; we still read the last non-empty line defensively
    // in case a ~/.bashrc echoes a banner ahead of the `which` result.
    #[cfg(not(target_os = "windows"))]
    {
        let shells = [
            "/bin/bash",
            "/usr/local/bin/bash",
            "/usr/bin/bash",
            "/bin/sh",
        ];
        for shell in shells {
            if std::path::Path::new(shell).exists() {
                let mut command = std::process::Command::new(shell);
                command.args(["-l", "-i", "-c", &format!("which {}", binary)]);
                // `-i` sources ~/.bashrc (mise/asdf/nvm) but also makes bash
                // attempt job control on the controlling terminal. Null stdin +
                // a detached session (own session, no controlling TTY) keep it
                // from seizing our terminal and suspending the process group;
                // the interactive flag lives in `$-`, not in stdin being a TTY,
                // so ~/.bashrc still sources. See `detach_from_controlling_tty`.
                command.stdin(std::process::Stdio::null());
                crate::shared::proc::detach_from_controlling_tty(&mut command);
                crate::shared::proc::sanitize_child_env(&mut command);
                if let Ok(output) = command.output() {
                    if output.status.success() {
                        let path_str = String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .map(|l| l.trim())
                            .rfind(|l| !l.is_empty())
                            .map(|l| l.to_string())
                            .unwrap_or_default();
                        if !path_str.is_empty() {
                            let pb = std::path::PathBuf::from(&path_str);
                            if pb.is_file() {
                                return Some(path_str);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn is_binary_on_local_path(binary: &str) -> bool {
    resolve_local_binary_path(binary).is_some()
}
