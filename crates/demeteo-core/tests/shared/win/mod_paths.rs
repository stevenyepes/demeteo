//! The path-shaped decisions this module makes, exercised from any host.

use super::*;
use std::path::PathBuf;

/// The default install root contains a space, so anything quoting this path has
/// to survive one. A renderer that dropped or escaped it would hand the agent
/// an invocation that resolves to `C:\Program`.
#[test]
fn a_resolved_shell_is_spelled_with_its_spaces_intact() {
    let shell = posix_shell::PosixShell {
        root: PathBuf::from(r"C:\Program Files\Git"),
        bash: PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
        sh: PathBuf::from(r"C:\Program Files\Git\usr\bin\sh.exe"),
    };

    assert_eq!(quotable_path(&shell), r"C:\Program Files\Git\bin\bash.exe");
}
