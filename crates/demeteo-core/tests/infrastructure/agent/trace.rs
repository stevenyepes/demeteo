// Tests extracted from `crates/demeteo-core/src/adapters/agent/trace.rs` (mirrored-tests convention). `super` = that module.

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "demeteo-trace-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_blank_or_absent_variable_leaves_tracing_off() {
    assert_eq!(trace_dir(None), None);
    assert_eq!(trace_dir(Some(String::new())), None);
    assert_eq!(trace_dir(Some("   ".to_string())), None);
    assert_eq!(
        trace_dir(Some("/var/log/demeteo".to_string())),
        Some(PathBuf::from("/var/log/demeteo"))
    );
}

/// A thread id is built from a feature id, a step id and a task id, none of
/// which is promised to be a legal path segment. Both separators are checked
/// against the same input on every host: a backslash is an ordinary character
/// off Windows, so asserting on one platform's separator alone would pass
/// while the other still escaped the directory.
#[test]
fn a_separator_in_the_session_id_cannot_escape_the_trace_directory() {
    let name = trace_file_name("codex-feat/42\\s-implement", 1, 0);
    assert!(
        !name.contains('/') && !name.contains('\\'),
        "separator survived into the file name: {name}"
    );
    assert_eq!(name, "codex-feat_42_s-implement.turn001.jsonl");

    let dir = Path::new("traces");
    assert_eq!(
        dir.join(trace_file_name("../../etc", 1, 0)),
        Path::new("traces").join(".._.._etc.turn001.jsonl")
    );
}

#[test]
fn turns_are_padded_so_a_listing_is_in_turn_order() {
    let mut names = vec![
        trace_file_name("codex-t", 10, 0),
        trace_file_name("codex-t", 2, 0),
        trace_file_name("codex-t", 1, 0),
    ];
    names.sort();
    assert_eq!(
        names,
        vec![
            "codex-t.turn001.jsonl",
            "codex-t.turn002.jsonl",
            "codex-t.turn010.jsonl",
        ]
    );
}

#[test]
fn a_session_id_that_sanitizes_to_nothing_still_names_a_visible_file() {
    let name = trace_file_name("", 1, 0);
    assert_eq!(name, "agent.turn001.jsonl");
    assert!(!name.starts_with('.'), "capture would be hidden: {name}");
}

#[test]
fn an_oversized_session_id_is_capped() {
    let name = trace_file_name(&"x".repeat(400), 1, 0);
    assert!(
        name.len() <= MAX_SESSION_COMPONENT + ".turn001.jsonl".len(),
        "unbounded file name: {} chars",
        name.len()
    );
    assert!(name.starts_with("xxx") && name.ends_with(".turn001.jsonl"));
}

/// The capture is verbatim agent output, and an agent's own command log is
/// exactly where a clone URL or a token-shaped argument shows up. Scrubbing
/// happens on the way to disk, so nothing upstream has to be trusted to have
/// redacted it first.
#[test]
fn a_credential_in_agent_output_is_scrubbed_before_it_reaches_disk() {
    let dir = TempDir::new("scrub");
    let mut trace =
        TurnTrace::open_in(&dir.0, "codex-feat42", 1).expect("trace file must be creatable");
    trace.record(r#"{"command":"git push https://x:ghp_abcdef1234567890@github.com/o/r"}"#);
    trace.record(r#"{"command":"gh auth login --with-token github_pat_0123456789abcdef"}"#);

    let written = std::fs::read_to_string(dir.0.join("codex-feat42.turn001.jsonl"))
        .expect("the capture must exist under the directory it was opened in");
    assert!(
        !written.contains("ghp_abcdef1234567890") && !written.contains("github_pat_0123456789"),
        "a credential reached the capture: {written}"
    );
    assert!(
        written.contains("git push") && written.contains("gh auth login"),
        "scrubbing swallowed the command itself: {written}"
    );
    assert_eq!(written.lines().count(), 2);
}

/// The turns worth tracing are the ones that hang and get killed, and a
/// killed drain thread drops its writer without running any flush. So the
/// bytes have to be readable while the trace is still open.
#[test]
fn a_recorded_line_is_on_disk_before_the_trace_is_dropped() {
    let dir = TempDir::new("unbuffered");
    let mut trace =
        TurnTrace::open_in(&dir.0, "codex-feat42", 7).expect("trace file must be creatable");
    trace.record(r#"{"type":"item.completed","item":{"command":"bash -lc ls"}}"#);

    let written = std::fs::read_to_string(dir.0.join("codex-feat42.turn007.jsonl"))
        .expect("the capture must exist while the trace is still open");
    assert!(
        written.contains("bash -lc ls"),
        "nothing flushed: {written}"
    );
}

/// A retry of the same task computes the same session id and restarts its turn
/// counter, so the second attempt names the first attempt's file — and the
/// attempt worth reading is usually the one that failed.
#[test]
fn a_second_capture_of_the_same_turn_never_overwrites_the_first() {
    let dir = TempDir::new("retry");
    let mut first =
        TurnTrace::open_in(&dir.0, "codex-f1-s-implement-t-3", 1).expect("first attempt");
    first.record(r#"{"command":"the attempt that misbehaved"}"#);
    drop(first);

    let mut second = TurnTrace::open_in(&dir.0, "codex-f1-s-implement-t-3", 1).expect("the retry");
    second.record(r#"{"command":"the re-run that worked"}"#);

    let original =
        std::fs::read_to_string(dir.0.join("codex-f1-s-implement-t-3.turn001.jsonl")).unwrap();
    assert!(
        original.contains("the attempt that misbehaved"),
        "the failing attempt's capture was truncated by the retry: {original}"
    );
    let retried =
        std::fs::read_to_string(dir.0.join("codex-f1-s-implement-t-3.turn001.1.jsonl")).unwrap();
    assert!(
        retried.contains("the re-run that worked"),
        "the retry's own capture is missing: {retried}"
    );
}

#[test]
fn an_unwritable_directory_yields_no_trace_rather_than_an_error() {
    let dir = TempDir::new("blocked");
    std::fs::create_dir_all(&dir.0).expect("temp dir");
    let blocked = dir.0.join("file-not-a-dir");
    std::fs::write(&blocked, b"x").expect("temp file");

    assert!(TurnTrace::open_in(&blocked, "codex-feat42", 1).is_none());
}
