// Tests extracted from `crates/demeteo-core/src/shared/win/exe.rs`
// (mirrored-tests convention). `super` = that module.
//
// Fixture PATH strings and a fixture directory listing, so the whole
// resolution order runs on the Linux development host — the reason the
// existence check is an injected predicate in the first place.

use super::*;
use std::collections::BTreeSet;

/// A directory listing that knows only the files it was given, matched the way
/// NTFS matches: case-insensitively, either separator. A candidate the
/// resolver should never have produced simply is not there, so an ordering
/// mistake fails a test rather than finding a plausible file.
struct Disk(BTreeSet<String>);

impl Disk {
    fn with(files: &[&str]) -> Self {
        Disk(files.iter().map(|f| key(f)).collect())
    }

    fn exists(&self) -> impl Fn(&Path) -> bool + '_ {
        move |path: &Path| self.0.contains(&key(&path.to_string_lossy()))
    }
}

fn key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn rendered(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect()
}

const NPM_DIR: &str = r"C:\Users\dev\AppData\Roaming\npm";
const NODE_DIR: &str = r"C:\Program Files\nodejs";

fn path_of(dirs: &[&str]) -> String {
    dirs.join(";")
}

// ── the defect this exists for ──────────────────────────────────────────────

#[test]
fn an_npm_shim_resolves_where_rust_would_have_found_nothing() {
    let disk = Disk::with(&[r"C:\Users\dev\AppData\Roaming\npm\npm.cmd"]);

    let found = resolve("npm", &path_of(&[NPM_DIR]), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Users\dev\AppData\Roaming\npm\npm.cmd".to_string())
    );
}

#[test]
fn the_extensionless_sibling_never_wins() {
    let disk = Disk::with(&[
        r"C:\Users\dev\AppData\Roaming\npm\npm",
        r"C:\Users\dev\AppData\Roaming\npm\npm.cmd",
    ]);

    let found = resolve("npm", &path_of(&[NPM_DIR]), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Users\dev\AppData\Roaming\npm\npm.cmd".to_string()),
        "the extensionless file is a bash script CreateProcess refuses to run"
    );
}

#[test]
fn an_extensionless_file_is_still_the_last_resort() {
    let disk = Disk::with(&[r"C:\Program Files\nodejs\weird"]);

    let found = resolve("weird", &path_of(&[NODE_DIR]), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Program Files\nodejs\weird".to_string())
    );
}

// ── ordering ────────────────────────────────────────────────────────────────

#[test]
fn the_first_path_entry_wins_over_a_better_extension_later() {
    let disk = Disk::with(&[
        r"C:\Users\dev\AppData\Roaming\npm\tool.cmd",
        r"C:\Program Files\nodejs\tool.exe",
    ]);

    let found = resolve("tool", &path_of(&[NPM_DIR, NODE_DIR]), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Users\dev\AppData\Roaming\npm\tool.cmd".to_string())
    );
}

#[test]
fn pathext_order_decides_inside_one_directory() {
    let disk = Disk::with(&[
        r"C:\Program Files\nodejs\tool.bat",
        r"C:\Program Files\nodejs\tool.exe",
    ]);

    let found = resolve("tool", &path_of(&[NODE_DIR]), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Program Files\nodejs\tool.exe".to_string())
    );
}

#[test]
fn candidate_order_is_every_extension_then_the_bare_name_per_directory() {
    let dirs = path_dirs(&path_of([NPM_DIR, NODE_DIR].as_slice()));

    let order = candidates("tool", &dirs, &[".exe".to_string(), ".cmd".to_string()]);

    assert_eq!(
        rendered(&order),
        vec![
            "C:/Users/dev/AppData/Roaming/npm/tool.exe",
            "C:/Users/dev/AppData/Roaming/npm/tool.cmd",
            "C:/Users/dev/AppData/Roaming/npm/tool",
            "C:/Program Files/nodejs/tool.exe",
            "C:/Program Files/nodejs/tool.cmd",
            "C:/Program Files/nodejs/tool",
        ]
    );
}

#[test]
fn a_name_that_carries_an_extension_is_tried_literally_first() {
    let dirs = path_dirs(NODE_DIR);

    let order = candidates("git.exe", &dirs, &[".com".to_string()]);

    assert_eq!(
        rendered(&order),
        vec![
            "C:/Program Files/nodejs/git.exe",
            "C:/Program Files/nodejs/git.exe.com",
        ]
    );
}

// ── what is not searched ────────────────────────────────────────────────────

#[test]
fn an_empty_path_entry_is_never_searched() {
    let disk = Disk::with(&["npm.cmd", r"C:\Program Files\nodejs\npm.cmd"]);

    let found = resolve("npm", &format!(";;{NODE_DIR}"), None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Program Files\nodejs\npm.cmd".to_string()),
        "an empty entry is the working directory, which an agent writes to"
    );
}

#[test]
fn a_relative_path_entry_is_never_searched() {
    assert_eq!(
        path_dirs(r"node_modules\.bin;.;..\tools"),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        rendered(&path_dirs(r"C:\tools;\\build\share\bin;D:/other")),
        vec!["C:/tools", "//build/share/bin", "D:/other"]
    );
}

#[test]
fn a_drive_relative_entry_is_not_absolute() {
    assert_eq!(path_dirs(r"C:tools"), Vec::<PathBuf>::new());
}

#[test]
fn a_quoted_entry_may_hold_a_separator() {
    assert_eq!(
        rendered(&path_dirs(r#"C:\a;"C:\odd;dir";C:\b"#)),
        vec!["C:/a", "C:/odd;dir", "C:/b"]
    );
}

// ── PATHEXT ─────────────────────────────────────────────────────────────────

#[test]
fn a_missing_or_blank_pathext_falls_back_to_the_windows_default() {
    let expected = vec![".com", ".exe", ".bat", ".cmd", ".vbs", ".js", ".ws", ".msc"];
    assert_eq!(extensions(None), expected);
    assert_eq!(extensions(Some("   ")), expected);
}

#[test]
fn pathext_is_lowercased_dotted_and_deduplicated() {
    assert_eq!(
        extensions(Some(".COM;EXE;;.exe;.PS1")),
        vec![".com", ".exe", ".ps1"]
    );
}

#[test]
fn a_custom_pathext_is_honoured_in_its_own_order() {
    let disk = Disk::with(&[
        r"C:\Program Files\nodejs\tool.exe",
        r"C:\Program Files\nodejs\tool.ps1",
    ]);

    let found = resolve("tool", NODE_DIR, Some(".PS1;.EXE"), &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\Program Files\nodejs\tool.ps1".to_string())
    );
}

// ── the shape of the answer ─────────────────────────────────────────────────

#[test]
fn matching_is_case_insensitive_as_the_filesystem_is() {
    let disk = Disk::with(&[r"C:\Program Files\nodejs\NPM.CMD"]);

    assert!(resolve("npm", NODE_DIR, None, &disk.exists()).is_some());
}

#[test]
fn the_answer_is_returned_in_windows_form() {
    let disk = Disk::with(&["C:/tools/bin/tool.exe"]);

    let found = resolve("tool", "C:/tools/bin", None, &disk.exists());

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"C:\tools\bin\tool.exe".to_string())
    );
}

#[test]
fn a_qualified_name_is_extended_rather_than_searched() {
    let disk = Disk::with(&[r"D:\tools\tool.cmd", r"C:\Program Files\nodejs\tool.exe"]);

    let found = resolve(
        r"D:\tools\tool",
        &path_of(&[NODE_DIR]),
        None,
        &disk.exists(),
    );

    assert_eq!(
        found.map(|p| p.to_string_lossy().into_owned()),
        Some(r"D:\tools\tool.cmd".to_string())
    );
}

#[test]
fn nothing_on_disk_is_no_answer_rather_than_a_guess() {
    let disk = Disk::with(&[r"C:\Program Files\nodejs\other.exe"]);

    assert_eq!(resolve("tool", NODE_DIR, None, &disk.exists()), None);
    assert_eq!(resolve("  ", NODE_DIR, None, &disk.exists()), None);
    assert_eq!(resolve("tool", "", None, &disk.exists()), None);
}
