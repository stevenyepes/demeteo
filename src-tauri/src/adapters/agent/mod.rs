pub mod antigravity;
pub mod claude_code;
pub mod cli_runtime;
pub mod direct_execution;
pub mod event_stream;
pub mod hermes;
pub mod install;
pub mod noop;
pub mod opencode;
pub mod registry;

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

    // Fallback: try to resolve via bash -l so we get profile additions
    // (homebrew, nvm, mise, pyenv, etc.). We use bash explicitly rather than
    // the SHELL env var because SHELL might be set to e.g. /bin/zsh which
    // doesn't source ~/.bashrc on macOS/Linux when invoked as "zsh -l".
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
                command.args(["-l", "-c", &format!("which {}", binary)]);
                crate::shared::proc::sanitize_child_env(&mut command);
                if let Ok(output) = command.output() {
                    if output.status.success() {
                        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
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
