use std::path::Path;

use super::support::{
    appending_capturing_channel, broadcast_with, shell_single_unquote, wait_until,
};
use super::{
    build_agent_launch_command, build_claude_activity_settings, drain_scan_and_forward,
    hook_transport_supported, is_hooked_agent_kind, remote_activity_settings_path,
    shell_single_quote, write_activity_settings_file,
};

#[test]
fn is_hooked_agent_kind_is_claude_only() {
    assert!(is_hooked_agent_kind("claude-code"));
    assert!(!is_hooked_agent_kind("opencode"));
    assert!(!is_hooked_agent_kind("codex"));
    assert!(!is_hooked_agent_kind(""));
}

#[test]
fn hook_transport_supported_for_ssh_on_every_client() {
    assert!(hook_transport_supported(false));
}

#[cfg(not(target_os = "windows"))]
#[test]
fn hook_transport_supported_for_local_on_posix() {
    assert!(hook_transport_supported(true));
}

#[cfg(target_os = "windows")]
#[test]
fn hook_transport_unsupported_for_local_on_windows() {
    assert!(!hook_transport_supported(true));
    assert!(hook_transport_supported(false));
}

#[test]
fn shell_single_quote_round_trips_embedded_quote() {
    assert_eq!(shell_single_quote("plain"), "'plain'");
    assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
    for original in ["", "it's", "a'b'c", "no quotes", "'leading", "trailing'"] {
        assert_eq!(
            shell_single_unquote(&shell_single_quote(original)),
            original,
            "round-trip failed for {original:?}"
        );
    }
}

#[test]
fn build_claude_activity_settings_none_for_non_hooked() {
    assert!(build_claude_activity_settings("opencode", "abc123").is_none());
    assert!(build_claude_activity_settings("codex", "abc123").is_none());
    assert!(build_claude_activity_settings("", "abc123").is_none());
}

#[test]
fn build_agent_launch_command_points_at_settings_file() {
    let path = Path::new("/tmp/demeteo-claude-activity-abc123.json");
    let cmd = build_agent_launch_command("claude --resume", path);
    assert_eq!(
        cmd,
        "claude --resume --settings '/tmp/demeteo-claude-activity-abc123.json'"
    );
    assert!(
        cmd.len() < 1024,
        "launch line must fit MAX_CANON, got {} bytes",
        cmd.len()
    );
}

#[test]
fn write_activity_settings_file_writes_valid_json_and_fits_launch_line() {
    let nonce = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let json = build_claude_activity_settings("claude-code", nonce).expect("settings JSON");
    assert!(
        json.len() > 1024,
        "precondition: inline JSON would have overrun MAX_CANON ({} bytes)",
        json.len()
    );
    let path = write_activity_settings_file(nonce, &json).expect("write settings file");
    let read_back = std::fs::read_to_string(&path).expect("read settings file back");
    let _: serde_json::Value = serde_json::from_str(&read_back).expect("file must hold valid JSON");
    let cmd = build_agent_launch_command("claude", &path);
    assert!(
        cmd.len() < 1024,
        "launch line under MAX_CANON: {} bytes",
        cmd.len()
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn remote_activity_settings_path_is_nonce_keyed_under_tmp_and_fits_launch_line() {
    let nonce = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let remote = remote_activity_settings_path(nonce);
    assert_eq!(
        remote,
        format!("/tmp/demeteo-claude-activity-{nonce}.json"),
        "remote path must be the nonce-keyed /tmp target the SFTP write uses"
    );
    let cmd = build_agent_launch_command("claude", Path::new(&remote));
    assert_eq!(cmd, format!("claude --settings '{remote}'"));
    assert!(
        cmd.len() < 1024,
        "remote launch line under MAX_CANON: {} bytes",
        cmd.len()
    );
}

#[test]
fn build_claude_activity_settings_injects_valid_hooks() {
    let nonce = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let json_text = build_claude_activity_settings("claude-code", nonce)
        .expect("claude-code must produce settings");
    let settings: serde_json::Value =
        serde_json::from_str(&json_text).expect("settings must be valid JSON");

    let hooks = settings
        .get("hooks")
        .and_then(|h| h.as_object())
        .expect("settings.hooks object");

    let mut commands: Vec<String> = Vec::new();
    for groups in hooks.values() {
        for group in groups.as_array().expect("event value is an array") {
            for hook in group["hooks"].as_array().expect("group.hooks array") {
                commands.push(
                    hook["command"]
                        .as_str()
                        .expect("command string")
                        .to_string(),
                );
            }
        }
    }

    for c in &commands {
        assert!(c.contains(nonce), "reporter missing nonce: {c}");
        assert!(
            c.contains("\\u001b"),
            "reporter missing literal ESC escape: {c}"
        );
        assert!(
            c.contains("\\u0007"),
            "reporter missing literal BEL escape: {c}"
        );
    }

    let has = |event: &str, matcher: Option<&str>, state: &str| -> bool {
        hooks
            .get(event)
            .and_then(|v| v.as_array())
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    let matcher_ok = match matcher {
                        Some(m) => g.get("matcher").and_then(|v| v.as_str()) == Some(m),
                        None => g.get("matcher").is_none(),
                    };
                    let state_ok = g["hooks"]
                        .as_array()
                        .and_then(|h| h.first())
                        .and_then(|h| h["command"].as_str())
                        .is_some_and(|c| c.contains(&format!("state={state}")));
                    matcher_ok && state_ok
                })
            })
    };
    assert!(has("UserPromptSubmit", None, "working"));
    assert!(has("PreToolUse", None, "working"));
    assert!(has("PostToolUse", None, "working"));
    assert!(has(
        "Notification",
        Some("permission_prompt"),
        "awaiting_approval"
    ));
    assert!(has("Notification", Some("idle_prompt"), "awaiting_input"));
    assert!(has("Stop", None, "awaiting_input"));
    assert!(has("SessionEnd", None, "exit"));
}

#[test]
fn drain_scan_and_forward_strips_sequence_and_surfaces_event() {
    let nonce = "feedface";
    let mut scanner = super::super::activity_scanner::ActivityScanner::new(nonce.to_string());

    let (channel, captured) = appending_capturing_channel();
    let frontend = broadcast_with(vec![channel]);

    let mut chunk = Vec::new();
    chunk.extend_from_slice(b"before ");
    chunk.extend_from_slice(b"\x1b]777;demeteo;v=1;nonce=feedface;state=awaiting_approval\x07");
    chunk.extend_from_slice(b" after");

    let events = drain_scan_and_forward(&mut scanner, &chunk, &frontend);
    assert_eq!(
        events,
        vec!["awaiting_approval".to_string()],
        "the parsed activity state must surface for the drain to emit"
    );

    assert!(
        wait_until(|| captured.lock().expect("lock").as_slice() == b"before  after"),
        "the demeteo sequence must be stripped from the forwarded bytes"
    );
}
