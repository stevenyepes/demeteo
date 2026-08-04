//! Reconstructing the `PATH` a spawned child should see.
//!
//! A GUI process inherits the environment block captured by whatever launched
//! it — on Windows, Explorer at logon. Installers write `HKCU\Environment` and
//! broadcast `WM_SETTINGCHANGE`, which only a window with a message pump acts
//! on, so a tool installed *while Demeteo is running* is absent from
//! `std::env::var("PATH")` until the app restarts. Re-reading the two
//! Environment keys is the only way a running process learns about it.
//!
//! What that recovers is shim-based version managers (Volta, pyenv-win,
//! nvm-windows, `mise activate --shims`) and nothing more. fnm and
//! `mise activate pwsh` inject their directory from `$PROFILE` at shell
//! startup, so their tools are on no PATH that any non-interactive child of a
//! GUI process can read, under any shell. That gap is stated, not hidden — see
//! docs/WINDOWS_PARITY.md, "Stated capability gaps".
//!
//! Ordering, de-duplication and `%VAR%` expansion are deliberately `cfg`-free
//! so they are testable without a Windows host; only the registry read is not.
//! Moving any of them behind the `cfg` puts them out of reach of every test
//! that runs before CI.

use std::collections::HashSet;
use std::path::PathBuf;

/// The PATH entries of a `;`-separated Windows PATH string, in order.
///
/// Quoting is respected because a quoted entry may itself contain `;`; empty
/// entries are dropped, since Windows resolves an empty entry as the current
/// directory (GHSA-2mqj-m65w-jghx) and Demeteo's CWD is an agent-written
/// worktree.
pub fn split_path_list(raw: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in raw.chars() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => push_entry(&mut entries, &mut current),
            _ => current.push(ch),
        }
    }
    push_entry(&mut entries, &mut current);
    entries
}

fn push_entry(entries: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        entries.push(trimmed.to_string());
    }
    current.clear();
}

/// Renders entries back into a PATH string, re-quoting any that need it.
pub fn join_path_list(entries: &[String]) -> String {
    entries
        .iter()
        .map(|entry| {
            if entry.contains(';') {
                format!("\"{entry}\"")
            } else {
                entry.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Identity of a PATH entry for de-duplication: separator direction and a
/// trailing separator are not distinctions Windows makes, and neither is case.
fn path_key(entry: &str) -> String {
    let normalised = entry.replace('/', "\\");
    let trimmed = normalised.trim_end_matches('\\');
    if trimmed.is_empty() {
        normalised.to_lowercase()
    } else {
        trimmed.to_lowercase()
    }
}

/// Concatenates PATH strings, keeping the first occurrence of each entry.
///
/// First-wins, rather than last-wins, is what keeps resolution order stable:
/// an entry a later source repeats does not move, so a setup that already
/// resolves the tools it wants resolves the same ones afterwards.
pub fn merge_path_entries<'a>(sources: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<String> = Vec::new();
    for entry in sources.into_iter().flat_map(split_path_list) {
        if seen.insert(path_key(&entry)) {
            merged.push(entry);
        }
    }
    merged
}

/// Substitutes `%VAR%` references, as `ExpandEnvironmentStringsW` would.
///
/// The Environment keys hold `REG_EXPAND_SZ`, so a user PATH read raw is full
/// of `%USERPROFILE%\...` entries that resolve to nothing. An undefined name
/// and an unpaired `%` are both left exactly as written, and a substituted
/// value is not rescanned.
pub fn expand_env_refs(raw: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match chars[i + 1..].iter().position(|&c| c == '%') {
            Some(offset) => {
                let end = i + 1 + offset;
                let name: String = chars[i + 1..end].iter().collect();
                match lookup(&name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(&name);
                        out.push('%');
                    }
                }
                i = end + 1;
            }
            None => {
                out.extend(&chars[i..]);
                break;
            }
        }
    }
    out
}

/// Directories that hold executables no installer reliably puts on PATH.
///
/// A missing variable drops only the entries derived from it; the caller
/// decides which of these exist.
pub fn windows_shim_dirs(
    appdata: Option<&str>,
    local_appdata: Option<&str>,
    user_profile: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(appdata) = appdata {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    if let Some(local_appdata) = local_appdata {
        let local = PathBuf::from(local_appdata);
        dirs.push(local.join("Programs"));
        dirs.push(local.join("mise").join("shims"));
        dirs.push(local.join("Volta").join("bin"));
    }
    if let Some(user_profile) = user_profile {
        let profile = PathBuf::from(user_profile);
        dirs.push(profile.join(".cargo").join("bin"));
        dirs.push(profile.join(".local").join("bin"));
        dirs.push(profile.join("scoop").join("shims"));
    }
    dirs
}

/// The PATH a Windows child should see.
///
/// Inherited first: it is the only source that carries entries a launcher or
/// this process added, and preserving its order preserves which copy of a tool
/// wins. The Environment keys then contribute whatever has been installed since
/// logon, and `appended` the shim directories nothing puts on PATH at all —
/// appended rather than prepended, so a directory that is already on PATH keeps
/// the priority the user gave it.
pub fn compose_windows_path(
    inherited: &str,
    machine: Option<&str>,
    user: Option<&str>,
    appended: &[String],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> String {
    let machine = machine
        .map(|v| expand_env_refs(v, lookup))
        .unwrap_or_default();
    let user = user.map(|v| expand_env_refs(v, lookup)).unwrap_or_default();
    let appended = join_path_list(appended);
    let merged = merge_path_entries([
        inherited,
        machine.as_str(),
        user.as_str(),
        appended.as_str(),
    ]);
    join_path_list(&merged)
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND,
        RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    const USER_ENVIRONMENT: &str = "Environment";
    const MACHINE_ENVIRONMENT: &str =
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

    pub(super) fn user_path() -> Option<String> {
        read_string(HKEY_CURRENT_USER, USER_ENVIRONMENT, "Path")
    }

    pub(super) fn machine_path() -> Option<String> {
        read_string(HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT, "Path")
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Reads a `REG_SZ`/`REG_EXPAND_SZ` value without expanding it: expansion
    /// here would use this process's stale environment block, which is the
    /// thing the caller is working around.
    fn read_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
        let subkey = wide(subkey);
        let value = wide(value);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;

        let mut size: u32 = 0;
        // SAFETY: both name pointers are NUL-terminated UTF-16 that outlive the
        // call, and a null data pointer is the documented way to ask for the
        // size in bytes alone.
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size as usize).div_ceil(2) + 1];
        let mut written = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        // SAFETY: buffer is 2-aligned as the u16 data this value type holds
        // requires, and `written` is its true capacity in bytes.
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut written,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }

        let chars = (written as usize) / std::mem::size_of::<u16>();
        let filled = buffer.get(..chars.min(buffer.len()))?;
        let text = match filled.iter().position(|&c| c == 0) {
            Some(end) => &filled[..end],
            None => filled,
        };
        Some(String::from_utf16_lossy(text))
    }
}

/// The persisted user PATH, unexpanded, or `None` when it cannot be read.
#[cfg(windows)]
pub fn user_environment_path() -> Option<String> {
    registry::user_path()
}

/// The persisted machine PATH, unexpanded, or `None` when it cannot be read.
#[cfg(windows)]
pub fn machine_environment_path() -> Option<String> {
    registry::machine_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_lowercase(), (*v).to_string()))
            .collect();
        move |name: &str| {
            let name = name.to_lowercase();
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn split_drops_empty_entries_and_keeps_order() {
        assert_eq!(
            split_path_list(r"C:\a;;C:\b; C:\c ;"),
            vec![r"C:\a".to_string(), r"C:\b".into(), r"C:\c".into()]
        );
    }

    #[test]
    fn split_respects_quoted_separators() {
        assert_eq!(
            split_path_list(r#"C:\a;"C:\odd;dir";C:\b"#),
            vec![r"C:\a".to_string(), r"C:\odd;dir".into(), r"C:\b".into()]
        );
    }

    #[test]
    fn join_requotes_entries_containing_a_separator() {
        let entries = vec![r"C:\a".to_string(), r"C:\odd;dir".into()];
        assert_eq!(join_path_list(&entries), r#"C:\a;"C:\odd;dir""#);
        assert_eq!(split_path_list(&join_path_list(&entries)), entries);
    }

    #[test]
    fn merge_deduplicates_case_separator_and_trailing_slash() {
        assert_eq!(
            merge_path_entries([
                r"C:\Program Files\Git\bin;C:\Windows",
                r"c:/program files/git/bin/;C:\Windows\;C:\Extra"
            ]),
            vec![
                r"C:\Program Files\Git\bin".to_string(),
                r"C:\Windows".into(),
                r"C:\Extra".into()
            ]
        );
    }

    #[test]
    fn merge_keeps_the_first_occurrence_in_place() {
        assert_eq!(
            merge_path_entries([r"C:\first;C:\second", r"C:\second;C:\third"]),
            vec![
                r"C:\first".to_string(),
                r"C:\second".into(),
                r"C:\third".into()
            ]
        );
    }

    #[test]
    fn expand_substitutes_defined_names_only() {
        let lookup = lookup(&[("USERPROFILE", r"C:\Users\dev")]);
        assert_eq!(
            expand_env_refs(r"%USERPROFILE%\bin;%NOPE%\bin", &lookup),
            r"C:\Users\dev\bin;%NOPE%\bin"
        );
    }

    #[test]
    fn expand_is_case_insensitive_and_handles_unpaired_percent() {
        let lookup = lookup(&[("USERPROFILE", r"C:\Users\dev")]);
        assert_eq!(
            expand_env_refs(r"%userprofile%\bin", &lookup),
            r"C:\Users\dev\bin"
        );
        assert_eq!(expand_env_refs(r"C:\100%\bin", &lookup), r"C:\100%\bin");
        assert_eq!(expand_env_refs("%%", &lookup), "%%");
    }

    #[test]
    fn expand_does_not_rescan_a_substituted_value() {
        let lookup = lookup(&[("A", "%B%"), ("B", "boom")]);
        assert_eq!(expand_env_refs("%A%", &lookup), "%B%");
    }

    #[test]
    fn shim_dirs_cover_every_manager_and_skip_unset_variables() {
        let dirs = windows_shim_dirs(Some("/appdata"), Some("/local"), Some("/profile"));
        let rendered: Vec<String> = dirs
            .iter()
            .map(|d| d.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            rendered,
            vec![
                "/appdata/npm",
                "/local/Programs",
                "/local/mise/shims",
                "/local/Volta/bin",
                "/profile/.cargo/bin",
                "/profile/.local/bin",
                "/profile/scoop/shims",
            ]
        );

        assert!(windows_shim_dirs(None, None, None).is_empty());
        assert_eq!(windows_shim_dirs(Some("/appdata"), None, None).len(), 1);
    }

    #[test]
    fn compose_appends_what_the_registry_gained_since_logon() {
        let lookup = lookup(&[("USERPROFILE", r"C:\Users\dev")]);
        let composed = compose_windows_path(
            r"C:\Windows\system32;C:\Windows",
            Some(r"C:\Windows\system32;C:\Program Files\nodejs"),
            Some(r"%USERPROFILE%\AppData\Roaming\npm"),
            &[r"C:\Users\dev\.local\bin".to_string()],
            &lookup,
        );
        assert_eq!(
            composed,
            [
                r"C:\Windows\system32",
                r"C:\Windows",
                r"C:\Program Files\nodejs",
                r"C:\Users\dev\AppData\Roaming\npm",
                r"C:\Users\dev\.local\bin",
            ]
            .join(";")
        );
    }

    #[test]
    fn compose_never_repeats_an_entry_the_process_already_has() {
        let lookup = lookup(&[]);
        let composed = compose_windows_path(
            r"C:\Users\dev\scoop\shims;C:\Windows",
            Some(r"C:\WINDOWS\"),
            Some(r"c:/users/dev/scoop/shims"),
            &[r"C:\Users\dev\scoop\shims".to_string()],
            &lookup,
        );
        assert_eq!(composed, r"C:\Users\dev\scoop\shims;C:\Windows");
    }

    #[test]
    fn compose_tolerates_unreadable_registry_values() {
        let lookup = lookup(&[]);
        assert_eq!(
            compose_windows_path(r"C:\Windows", None, None, &[], &lookup),
            r"C:\Windows"
        );
    }
}
