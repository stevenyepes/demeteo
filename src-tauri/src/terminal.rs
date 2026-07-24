mod activity;
mod agent_detector;
mod commands;
mod drain;
mod hooks;
mod model;
mod start;
mod transport;

pub(crate) mod activity_scanner;

pub use model::{
    ActiveSession, ActivityInfo, Broadcast, ReadSource, SessionActivity, SessionInfo,
    SessionKeepalive, SessionState, StartedSession, WriteSink,
};

pub use transport::connect_ssh;

pub use activity::spawn_activity_sweep;
pub use agent_detector::spawn_agent_detector;

pub use commands::{
    attach_terminal_session, close_machine_sessions, close_terminal_session, delete_machine_secret,
    detach_terminal_session, list_terminal_sessions, reconnect_terminal_session,
    rename_terminal_session, report_terminal_screen_activity, resize_terminal_session,
    set_machine_secret, write_terminal_session,
};
pub use start::start_terminal_session;

pub use commands::{
    __cmd__attach_terminal_session, __cmd__close_machine_sessions, __cmd__close_terminal_session,
    __cmd__delete_machine_secret, __cmd__detach_terminal_session, __cmd__list_terminal_sessions,
    __cmd__reconnect_terminal_session, __cmd__rename_terminal_session,
    __cmd__report_terminal_screen_activity, __cmd__resize_terminal_session,
    __cmd__set_machine_secret, __cmd__write_terminal_session,
    __tauri_command_name_attach_terminal_session, __tauri_command_name_close_machine_sessions,
    __tauri_command_name_close_terminal_session, __tauri_command_name_delete_machine_secret,
    __tauri_command_name_detach_terminal_session, __tauri_command_name_list_terminal_sessions,
    __tauri_command_name_reconnect_terminal_session, __tauri_command_name_rename_terminal_session,
    __tauri_command_name_report_terminal_screen_activity,
    __tauri_command_name_resize_terminal_session, __tauri_command_name_set_machine_secret,
    __tauri_command_name_write_terminal_session,
};
pub use start::{__cmd__start_terminal_session, __tauri_command_name_start_terminal_session};

#[allow(unused_imports)]
pub(crate) use commands::reconnect_with_machine;
#[allow(unused_imports)]
pub(crate) use drain::{drain_local, send_chunk};
#[allow(unused_imports)]
pub(crate) use transport::start_local_pty;

#[cfg(test)]
use crate::terminal::activity::CADENCE_WINDOW;
#[cfg(test)]
use crate::terminal::activity::{
    apply_cadence, apply_hook, apply_screen, cadence_state, decide_and_record, resolve,
    should_clear_activity_on_agent_exit, sweep_activity_once,
};
#[cfg(all(test, target_os = "windows"))]
use crate::terminal::agent_detector::ProcessTree;
#[cfg(test)]
use crate::terminal::agent_detector::{agent_kind_for_binary, detect_agent_in_command};
#[cfg(test)]
use crate::terminal::drain::{drain_scan_and_forward, emit_disconnected};
#[cfg(test)]
use crate::terminal::hooks::cmd_double_quote;
#[cfg(test)]
use crate::terminal::hooks::{
    build_agent_launch_command, build_claude_activity_settings, hook_transport_supported,
    is_hooked_agent_kind, remote_activity_settings_path, shell_single_quote,
    write_activity_settings_file,
};
#[cfg(test)]
use crate::terminal::model::SCROLLBACK_MAX_BYTES;
#[cfg(test)]
use crate::terminal::transport::{
    branch_bootstrap_line, branch_bootstrap_line_posix, select_local_shell,
};

#[cfg(test)]
#[path = "../tests/infrastructure/terminal.rs"]
mod tests;
