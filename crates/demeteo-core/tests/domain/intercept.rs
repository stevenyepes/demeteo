use super::*;
use crate::domain::action::AgentAction;

#[test]
fn preview_truncates_long_content() {
    let big = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let p = InterceptPayload::from_action(
        "i1".into(),
        "t1".into(),
        "m1".into(),
        &AgentAction::Edit {
            path: "/x".into(),
            content: big,
        },
    );
    let preview = p.preview.unwrap();
    assert!(preview.contains("..."));
    assert!(preview.lines().count() <= 13);
}

#[test]
fn bash_target_is_full_command() {
    let p = InterceptPayload::from_action(
        "i1".into(),
        "t1".into(),
        "m1".into(),
        &AgentAction::RunBash {
            cmd: "cargo build --release".into(),
        },
    );
    assert_eq!(p.target, "cargo build --release");
    assert_eq!(p.preview.as_deref(), Some("cargo build --release"));
    assert!(p.tool_call_id.is_none());
}

#[test]
fn from_agent_tool_call_records_tool_call_id() {
    let p = InterceptPayload::from_agent_tool_call(
        "i1".into(),
        "t1".into(),
        "m1".into(),
        "tc-99".into(),
        &AgentAction::Read { path: "/x".into() },
    );
    assert_eq!(p.tool_call_id.as_deref(), Some("tc-99"));
}
