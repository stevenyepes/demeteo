#[cfg(target_os = "windows")]
use super::ProcessTree;
use super::{
    agent_kind_for_binary, apply_cadence, apply_hook, apply_screen, attach_terminal_session,
    branch_bootstrap_line, branch_bootstrap_line_posix, build_agent_launch_command,
    build_claude_activity_settings, cadence_state, cmd_double_quote, decide_and_record,
    detach_terminal_session, detect_agent_in_command, drain_scan_and_forward, emit_disconnected,
    hook_transport_supported, is_hooked_agent_kind, list_terminal_sessions, reconnect_with_machine,
    remote_activity_settings_path, rename_terminal_session, resolve, select_local_shell,
    send_chunk, shell_single_quote, should_clear_activity_on_agent_exit, start_local_pty,
    sweep_activity_once, write_activity_settings_file, ActiveSession, Broadcast, ReadSource,
    SessionActivity, SessionState, WriteSink, CADENCE_WINDOW, SCROLLBACK_MAX_BYTES,
};

#[cfg(test)]
#[path = "terminal/activity.rs"]
mod activity;
#[cfg(test)]
#[path = "terminal/agent_detector.rs"]
mod agent_detector;
#[cfg(test)]
#[path = "terminal/bootstrap.rs"]
mod bootstrap;
#[cfg(test)]
#[path = "terminal/broadcast.rs"]
mod broadcast;
#[cfg(test)]
#[path = "terminal/hooks.rs"]
mod hooks;
#[cfg(test)]
#[path = "terminal/reconnect.rs"]
mod reconnect;
#[cfg(test)]
#[path = "terminal/support.rs"]
mod support;
