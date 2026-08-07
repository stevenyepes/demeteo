use super::*;
use std::collections::BTreeSet;

struct Disk(BTreeSet<String>);

impl Disk {
    fn with(files: &[&str]) -> Self {
        Self(files.iter().map(|path| key(path)).collect())
    }

    fn exists(&self) -> impl Fn(&Path) -> bool + '_ {
        move |path| self.0.contains(&key(&path.to_string_lossy()))
    }
}

/// Index a path the way a filesystem would answer for it, which means
/// collapsing `..` rather than comparing the literal text.
///
/// A `Disk` that matches text alone rejects `…/node_modules/../outside.js`
/// because no such string was registered — so the traversal case passes
/// whether or not [`direct_launch`] rejects the traversal, and the guard is
/// asserted by nothing. Resolving the segments is what puts the escape target
/// genuinely within reach and leaves the guard as the only thing standing in
/// its way.
fn key(path: &str) -> String {
    let lowered = path.replace('\\', "/").to_ascii_lowercase();
    let mut resolved: Vec<&str> = Vec::new();
    for segment in lowered.split('/') {
        match segment {
            "." => {}
            ".." => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved.join("/")
}

/// Forward slashes, though a real shim sits at a backslash path. `Path` treats
/// `\` as an ordinary character off Windows, so `C:\…\codex.cmd` is one
/// filename there: `parent()` is `""`, every join lands somewhere `Disk` has
/// never heard of, and `direct_launch` returns `None` for a reason that has
/// nothing to do with what is being asserted. That reads as a pass in the two
/// `is_none()` cases below — the traversal guard would be verified nowhere but
/// Windows. Windows accepts either separator, so one spelling is honest on all
/// three.
const SHIM: &str = "C:/Users/dev/AppData/Local/mise/installs/node/24/codex.cmd";
const ENTRY: &str =
    "C:/Users/dev/AppData/Local/mise/installs/node/24/node_modules/@openai/codex/bin/codex.js";
const NODE: &str = "C:/Users/dev/AppData/Local/mise/installs/node/24/node.exe";
const OUTSIDE: &str = "C:/Users/dev/AppData/Local/mise/installs/node/24/outside.js";

#[test]
fn npm_shim_runs_node_and_its_entrypoint_without_cmd() {
    let disk = Disk::with(&[NODE, ENTRY]);
    let launch = direct_launch(
        Path::new(SHIM),
        r#""%_prog%" "%dp0%\node_modules\@openai\codex\bin\codex.js" %*"#,
        None,
        &disk.exists(),
    )
    .expect("the npm shim shape is launchable");
    assert_eq!(key(&launch.node.to_string_lossy()), key(NODE));
    assert_eq!(key(&launch.entrypoint.to_string_lossy()), key(ENTRY));
}

#[test]
fn a_shim_cannot_name_an_entrypoint_outside_its_node_modules_directory() {
    // `OUTSIDE` is on the disk deliberately: the escape has to be reachable
    // for its rejection to mean the guard rejected it.
    let disk = Disk::with(&[NODE, ENTRY, OUTSIDE]);
    assert!(direct_launch(
        Path::new(SHIM),
        r#""%_prog%" "%dp0%\node_modules\..\outside.js" %*"#,
        None,
        &disk.exists(),
    )
    .is_none());
}

#[test]
fn a_missing_entrypoint_keeps_the_normal_spawn_diagnostic() {
    let disk = Disk::with(&[NODE]);
    assert!(direct_launch(
        Path::new(SHIM),
        r#""%_prog%" "%dp0%\node_modules\@openai\codex\bin\codex.js" %*"#,
        None,
        &disk.exists(),
    )
    .is_none());
}
