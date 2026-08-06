//! Which file on disk `run_program("npm", …)` actually runs on Windows.
//!
//! `CreateProcessW` applies no `PATHEXT` and Rust's `Command` appends only
//! `.exe`, so a program name that resolves on Linux resolves to nothing here
//! the moment the tool ships as a `.cmd` shim — which is how everything
//! installed by npm ships, several of the agent runtimes among them. The
//! resolution `cmd.exe` performs for a human typing the same word is therefore
//! something Demeteo has to perform for itself, and one resolver has to serve
//! both the availability probe and the spawn, or "available" and "runnable"
//! disagree.
//!
//! Two departures from `SearchPath`, both deliberate:
//!
//! - **Neither the current directory nor a relative `PATH` entry is
//!   searched.** `CreateProcess` consults the working directory before `PATH`
//!   (GHSA-2mqj-m65w-jghx) and Demeteo's working directory is a worktree an
//!   agent writes into, so a `git.exe` an agent drops beside its source would
//!   win. Handing the spawn an absolute path is what actually forecloses that;
//!   `NoDefaultCurrentDirectoryInExePath` in `shared/proc.rs` covers the
//!   searches the children then do for themselves.
//! - **A bare name resolves to a `PATHEXT` match before an extensionless
//!   file.** `%APPDATA%\npm` holds both `npm`, a bash script, and `npm.cmd`;
//!   handing `CreateProcess` the former earns "not a valid Win32
//!   application". The extensionless file stays last, for a PE that simply
//!   carries no suffix.
//!
//! Everything but the existence check is a pure function of two strings, so it
//! is tested on the Linux development host — see the module header next door.

use std::path::{Path, PathBuf};

use super::win_path;

/// `%PATHEXT%`'s documented default, used when the variable is absent or
/// blank. Windows itself falls back to this, so a machine that has somehow
/// lost the variable still resolves what every other machine resolves.
pub const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.MSC";

/// The first candidate that exists, as an absolute Windows-form path, or
/// `None`.
///
/// `None` is not an error: the caller spawns the bare name and lets
/// `CreateProcess` produce the message it always produced, which keeps a
/// failure on Windows reading like the same failure on Linux.
pub fn resolve(
    name: &str,
    path: &str,
    pathext: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if name.trim().is_empty() {
        return None;
    }
    let dirs = path_dirs(path);
    let extensions = extensions(pathext);
    candidates(name, &dirs, &extensions)
        .into_iter()
        .find(|candidate| exists(candidate))
        .map(|candidate| windows_form(&candidate))
}

/// The searchable directories of a `;`-separated `PATH`, in order.
///
/// Quoted entries may contain a `;` of their own. An empty entry means the
/// current directory to Windows and a relative one is resolved against it, so
/// both are dropped rather than searched — see the module header.
///
pub fn path_dirs(raw: &str) -> Vec<PathBuf> {
    split_semicolons(raw)
        .into_iter()
        .filter(|entry| is_absolute(entry))
        .map(|entry| win_path(&entry))
        .collect()
}

/// The suffixes to try, in `%PATHEXT%` order, lowercased.
///
/// Lowercasing loses nothing: the existence check is a Windows filesystem
/// lookup, which is case-insensitive, while `%PATHEXT%` is conventionally
/// written in upper case and files are conventionally not. An entry written
/// without its leading dot still means an extension.
pub fn extensions(pathext: Option<&str>) -> Vec<String> {
    let raw = pathext
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PATHEXT);
    let mut out: Vec<String> = Vec::new();
    for entry in split_semicolons(raw) {
        let extension = match entry.strip_prefix('.') {
            Some(rest) => format!(".{}", rest.to_ascii_lowercase()),
            None => format!(".{}", entry.to_ascii_lowercase()),
        };
        if extension.len() > 1 && !out.contains(&extension) {
            out.push(extension);
        }
    }
    out
}

/// Every path that could be this program, in the order they are to be tried.
///
/// Directory-major, matching `cmd.exe`: the whole extension list is tried in
/// one directory before the next directory is looked at, so an earlier `PATH`
/// entry wins even when a later one holds a higher-priority extension.
///
/// A name that already carries a directory is not a `PATH` search at all —
/// `CreateProcess` does not treat it as one either — so it is only extended.
pub fn candidates(name: &str, dirs: &[PathBuf], extensions: &[String]) -> Vec<PathBuf> {
    let name = win_path(name);
    if name.parent().is_some_and(|dir| !dir.as_os_str().is_empty()) {
        return with_extensions(&name, extensions);
    }
    dirs.iter()
        .flat_map(|dir| with_extensions(&dir.join(&name), extensions))
        .collect()
}

fn with_extensions(base: &Path, extensions: &[String]) -> Vec<PathBuf> {
    let suffixed = extensions
        .iter()
        .map(|extension| PathBuf::from(format!("{}{}", base.to_string_lossy(), extension)));
    let literal = std::iter::once(base.to_path_buf());
    if base.extension().is_some() {
        literal.chain(suffixed).collect()
    } else {
        suffixed.chain(literal).collect()
    }
}

/// Whether a `PATH` entry names a place rather than a place relative to
/// wherever the process happens to be.
///
/// Decided on the text, because [`Path::is_absolute`] answers for the host it
/// is compiled for: `C:\tools` is absolute on Windows and relative on the
/// Linux machine this is tested on.
fn is_absolute(entry: &str) -> bool {
    let entry = entry.trim().trim_matches('"').replace('\\', "/");
    if entry.starts_with("//") {
        return true;
    }
    let bytes = entry.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

/// Back to backslashes for the answer alone. The separator is free-choice for
/// `CreateProcess`, but std spawns a `.bat`/`.cmd` through `cmd.exe`, which
/// reads a leading `/` on a command-line token as the start of a switch.
fn windows_form(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('/', "\\"))
}

/// The entries of a `;`-separated Windows `PATH`, quoting respected and empty
/// entries dropped.
///
/// `src-tauri/src/env_path.rs` composes the string this splits back apart and
/// calls this to do it. One splitter and not two: a fix to the quoting — an
/// unpaired `"`, an escaped one — applied to the composer alone would leave the
/// resolver disagreeing with it about where one entry ends, and a tool on the
/// PATH the app installed would then be unfindable against that same PATH.
pub fn split_semicolons(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in raw.chars() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => push_trimmed(&mut out, &mut current),
            _ => current.push(ch),
        }
    }
    push_trimmed(&mut out, &mut current);
    out
}

fn push_trimmed(out: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    current.clear();
}

/// Resolve against this process's `PATH` and `%PATHEXT%`.
///
/// The `PATH` read here is the one `enrich_env_path` in `src-tauri/src/lib.rs`
/// rebuilt from the two Environment registry keys at startup and installed
/// into the process block, so this sees the reconstruction without repeating
/// the registry read per lookup — and a spawned child inherits the same list
/// the answer was chosen from.
#[cfg(windows)]
pub fn resolve_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    let pathext = std::env::var("PATHEXT").ok();
    resolve(name, &path, pathext.as_deref(), &|candidate: &Path| {
        candidate.is_file()
    })
}

#[cfg(test)]
#[path = "../../../tests/shared/win/exe.rs"]
mod tests;
