//! The I/O half of `shared/win/posix_shell.rs`: read the registry, look for
//! `git.exe`, run the probe. Nothing here decides anything — every branch that
//! could be wrong lives next door, where a Linux test can reach it.

use std::path::Path;
use std::process::{Command, Stdio};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SAM_FLAGS, REG_SZ,
    REG_VALUE_TYPE,
};

use super::exe;
use super::posix_shell::{ShellHost, ShellSearch};
use crate::shared::proc::harden_child_spawn;

const GIT_KEY: &str = r"SOFTWARE\GitForWindows";
const INSTALL_PATH: &str = "InstallPath";

pub fn search() -> ShellSearch {
    ShellSearch {
        override_bash: env_value("DEMETEO_BASH_PATH"),
        registry_install_paths: registry_install_paths(),
        git_exec_path: git_exec_path(),
        git_exe: git_exe_on_path(),
        program_files: env_value("ProgramFiles"),
        program_files_x86: env_value("ProgramFiles(x86)"),
        local_app_data: env_value("LOCALAPPDATA"),
    }
}

pub struct WindowsHost;

impl ShellHost for WindowsHost {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn bash_version(&self, bash: &Path) -> Result<String, String> {
        let mut command = Command::new(bash);
        command
            .args(["-c", "echo ${BASH_VERSION:-none}"])
            .stdin(Stdio::null());
        harden_child_spawn(&mut command);
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("exited with {:?}", output.status.code()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Both hives under both registry views. A 32-bit Demeteo reading the default
/// view would miss a 64-bit Git's key entirely, and vice versa.
fn registry_install_paths() -> Vec<String> {
    let mut out = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Some(value) = read_string(hive, view, GIT_KEY, INSTALL_PATH) {
                if !out.contains(&value) {
                    out.push(value);
                }
            }
        }
    }
    out
}

fn read_string(hive: HKEY, view: REG_SAM_FLAGS, subkey: &str, value: &str) -> Option<String> {
    let subkey = wide(subkey);
    let value = wide(value);
    let mut key: HKEY = std::ptr::null_mut();

    // SAFETY: both name pointers are nul-terminated UTF-16 owned by this
    // frame, `key` is a live out-parameter, and the handle is closed on every
    // path below.
    let opened = unsafe {
        RegOpenKeyExW(
            hive,
            subkey.as_ptr(),
            0,
            KEY_READ | view,
            &mut key as *mut HKEY,
        )
    };
    if opened != ERROR_SUCCESS {
        return None;
    }

    let read = read_key_string(key, &value);
    // SAFETY: `key` was opened successfully above and is not used afterwards.
    unsafe { RegCloseKey(key) };
    read
}

fn read_key_string(key: HKEY, value: &[u16]) -> Option<String> {
    let mut kind: REG_VALUE_TYPE = 0;
    let mut bytes: u32 = 0;

    // SAFETY: a null data pointer with a live size out-parameter is the
    // documented way to ask RegQueryValueExW for the required buffer size.
    let sized = unsafe {
        RegQueryValueExW(
            key,
            value.as_ptr(),
            std::ptr::null(),
            &mut kind as *mut REG_VALUE_TYPE,
            std::ptr::null_mut(),
            &mut bytes as *mut u32,
        )
    };
    if sized != ERROR_SUCCESS || bytes == 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
        return None;
    }

    let mut buffer: Vec<u16> = vec![0; (bytes as usize).div_ceil(2)];
    let mut capacity: u32 = bytes;
    // SAFETY: the buffer is a `Vec<u16>`, so it is aligned for the wide value
    // the API writes, and `capacity` states its size in bytes as required.
    let read = unsafe {
        RegQueryValueExW(
            key,
            value.as_ptr(),
            std::ptr::null(),
            &mut kind as *mut REG_VALUE_TYPE,
            buffer.as_mut_ptr().cast::<u8>(),
            &mut capacity as *mut u32,
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }

    let units = (capacity as usize).div_ceil(2).min(buffer.len());
    let text: String = String::from_utf16_lossy(&buffer[..units])
        .trim_end_matches('\0')
        .to_string();
    (!text.trim().is_empty()).then_some(text)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn git_exec_path() -> Option<String> {
    let mut command = Command::new(git_exe_on_path().unwrap_or_else(|| "git".to_string()));
    command.arg("--exec-path").stdin(Stdio::null());
    harden_child_spawn(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

fn git_exe_on_path() -> Option<String> {
    exe::resolve_on_path("git").map(|path| path.to_string_lossy().into_owned())
}
