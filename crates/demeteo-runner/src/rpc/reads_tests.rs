// Tests extracted from `crates/demeteo-runner/src/rpc/reads.rs` (mirrored-tests convention). `super` = that module.

use super::is_declared_artifact;

fn refs<'a>(
    pairs: &'a [(Option<&'a str>, Vec<String>)],
) -> impl IntoIterator<Item = (Option<&'a str>, &'a [String])> {
    pairs.iter().map(|(s, m)| (*s, m.as_slice()))
}

#[test]
fn matches_single_artifact_path() {
    let steps = [(Some("/w/report.md"), vec![])];
    assert!(is_declared_artifact(refs(&steps), "/w/report.md"));
}

#[test]
fn matches_within_artifact_paths_list() {
    let steps = [(None, vec!["/w/a.txt".to_string(), "/w/b.txt".to_string()])];
    assert!(is_declared_artifact(refs(&steps), "/w/b.txt"));
}

#[test]
fn rejects_undeclared_path() {
    // The security-relevant case: a path no step declared must not
    // be readable over the control socket, even a plausible sibling.
    let steps = [
        (Some("/w/report.md"), vec!["/w/a.txt".to_string()]),
        (None, vec!["/w/b.txt".to_string()]),
    ];
    assert!(!is_declared_artifact(refs(&steps), "/w/../.ssh/id_rsa"));
    assert!(!is_declared_artifact(refs(&steps), "/w/report.md.bak"));
    assert!(!is_declared_artifact(refs(&steps), "/etc/passwd"));
}

#[test]
fn rejects_when_no_steps_declare_anything() {
    let steps: [(Option<&str>, Vec<String>); 0] = [];
    assert!(!is_declared_artifact(refs(&steps), "/w/report.md"));
}
