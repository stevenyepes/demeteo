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

pub fn sanitize_child_env(cmd: &mut Command) {
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
