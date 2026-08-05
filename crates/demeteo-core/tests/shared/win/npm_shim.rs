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

fn key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

const SHIM: &str = r"C:\Users\dev\AppData\Local\mise\installs\node\24\codex.cmd";
const ENTRY: &str =
    r"C:\Users\dev\AppData\Local\mise\installs\node\24\node_modules\@openai\codex\bin\codex.js";
const NODE: &str = r"C:\Users\dev\AppData\Local\mise\installs\node\24\node.exe";

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
    assert_eq!(launch.node.to_string_lossy().replace('/', "\\"), NODE);
    assert_eq!(
        launch.entrypoint.to_string_lossy().replace('/', "\\"),
        ENTRY
    );
}

#[test]
fn a_shim_cannot_name_an_entrypoint_outside_its_node_modules_directory() {
    let disk = Disk::with(&[NODE, ENTRY]);
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
