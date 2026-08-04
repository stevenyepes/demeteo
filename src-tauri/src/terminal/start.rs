use crate::state::AppContext;
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

use super::drain::spawn_drain;
use super::hooks::{
    build_agent_launch_command, build_claude_activity_settings, hook_transport_supported,
    is_hooked_agent_kind, new_activity_nonce, remote_activity_settings_path,
    write_activity_settings_file,
};
use super::model::{
    session_counter_next, spawn_pty_size, ActiveSession, Broadcast, SessionInfo, SessionState,
    StartedSession,
};
use super::transport::{start_local_pty, start_ssh_session};

/// Starts a terminal session on the given machine.
///
/// `agent_kind` is the coding-agent kind the frontend is launching into this
/// fresh session (e.g. `"claude-code"`), or `None` for a plain shell. It
/// seeds the session's agent label immediately so the tab shows the badge
/// before the foreground detector has run its first pass.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    work_dir: Option<String>,
    work_branch: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    agent_kind: Option<String>,
    launch_command: Option<String>,
) -> Result<StartedSession, String> {
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;
    let machine_name = machine.name.clone();
    // Resolve the initial PTY size so the shell draws its very first prompt at
    // (near) the real terminal width.
    let (cols, rows) = spawn_pty_size(cols, rows);

    let session_id = format!("sess_{}", session_counter_next());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Seed an empty `Broadcast`: no subscriber and no permanent seed
    // channel. Output produced before the surface attaches (shell
    // startup, the `git checkout` bootstrap, the first prompt) flows
    // into the scrollback ring and is replayed on the first
    // `attach_terminal_session`, so nothing is lost and nothing races
    // the attach (TERMINALS_VIEW_SPEC §3).
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let display_title: Mutex<Option<String>> = Mutex::new(None);
    let connected = Arc::new(AtomicBool::new(true));
    // Seed the last-output instant to "now" so the session reads as
    // recently-active until its first quiet window elapses.
    let last_output_at: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    // Normalise an empty agent kind to `None` so a plain shell never carries
    // a phantom badge.
    let agent_kind = agent_kind.filter(|k| !k.trim().is_empty());

    let is_local = machine.auth_type == "local";
    // For a hooked agent kind (Claude) whose transport is usable on this
    // session, mint a per-session nonce so the drain scanner accepts only THIS
    // launch's activity sequences, and build the reporter-hooks JSON. Both are
    // computed BEFORE the transport starts so the remote start path can SFTP the
    // JSON onto the far host while its session is still blocking and before the
    // drain thread reads (T4.1). `hook_transport_supported(is_local)` degrades a
    // Windows LOCAL (cmd.exe) session to unhooked while leaving the always-POSIX
    // SSH path hooked regardless of client OS — see that fn's doc.
    let activity_nonce = agent_kind
        .as_deref()
        .filter(|k| is_hooked_agent_kind(k) && hook_transport_supported(is_local))
        .map(|_| new_activity_nonce());
    // Reporter-hooks JSON, transport-agnostic. `Some` only for a hooked launch
    // that also carries a base command to augment (§2c/§2d). Never the user's
    // `~/.claude` nor the project `.claude/` settings — always a throwaway.
    let activity_settings_json = match (
        agent_kind.as_deref(),
        activity_nonce.as_deref(),
        launch_command.as_deref(),
    ) {
        (Some(kind), Some(nonce), Some(_base)) => build_claude_activity_settings(kind, nonce),
        _ => None,
    };

    let (read_source, write_sink, keepalive, child_pid, remote_settings_path) = if is_local {
        start_local_pty(&machine_id, &work_dir, &work_branch, cols, rows)?
    } else {
        // Hand the remote start path the (remote path, JSON) so it places the
        // settings file over SFTP and reports the path back for the launch line.
        let remote_settings = match (activity_settings_json.as_deref(), activity_nonce.as_deref()) {
            (Some(json), Some(nonce)) => Some((remote_activity_settings_path(nonce), json)),
            _ => None,
        };
        start_ssh_session(
            &machine,
            &work_dir,
            &work_branch,
            cols,
            rows,
            remote_settings,
        )?
    };

    // Backend half of the single-write contract (§2c): `launch_override` `Some`
    // ⇒ the frontend writes this augmented `claude --settings <path>` instead of
    // its own base command. Local writes an ephemeral file to the OS temp dir
    // (`--settings <file>`, not ~1.4 KB of inline JSON, which overran the PTY's
    // 1024-byte canonical input limit); remote already SFTP'd the file above and
    // just references the returned remote path. Either way a placement failure
    // degrades to an unhooked launch (frontend writes plain `base`) rather than
    // breaking the launch — the nonce only gates precise activity, not
    // correctness.
    let (launch_override, activity_settings_path) = match (
        activity_settings_json.as_deref(),
        activity_nonce.as_deref(),
        launch_command.as_deref(),
    ) {
        (Some(json), Some(nonce), Some(base)) => {
            if is_local {
                match write_activity_settings_file(nonce, json) {
                    Ok(path) => (Some(build_agent_launch_command(base, &path)), Some(path)),
                    Err(e) => {
                        eprintln!(
                            "[terminal] activity settings file write failed: {e}; launching unhooked"
                        );
                        (None, None)
                    }
                }
            } else {
                // The remote file lives on the far host, so it is NOT stored in
                // `activity_settings_path` (whose `Drop` does a LOCAL
                // `remove_file`); it is left in the remote `/tmp` (harmless).
                match remote_settings_path {
                    Some(remote_path) => (
                        Some(build_agent_launch_command(base, Path::new(&remote_path))),
                        None,
                    ),
                    None => {
                        eprintln!(
                            "[terminal] remote activity settings write failed; launching unhooked"
                        );
                        (None, None)
                    }
                }
            }
        }
        _ => (None, None),
    };

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.clone(),
        machine_id.clone(),
        created_at,
        frontend_channel.clone(),
        connected.clone(),
        last_output_at.clone(),
        activity_nonce.clone(),
    );

    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    sessions.insert(
        session_id.clone(),
        ActiveSession {
            read_source,
            write_sink,
            _keepalive: keepalive,
            machine_id: machine_id.clone(),
            machine_name: machine_name.clone(),
            created_at,
            child_pid,
            agent: Mutex::new(agent_kind.clone()),
            activity_nonce,
            activity_settings_path,
            last_output_at,
            frontend_channel,
            display_title,
            work_dir,
            work_branch,
            connected,
        },
    );

    let _ = app.emit(
        "terminal-session-started",
        SessionInfo {
            session_id: session_id.clone(),
            machine_id,
            created_at,
            title: None,
            machine_name: Some(machine_name),
            agent: agent_kind,
        },
    );

    Ok(StartedSession {
        session_id,
        launch_command: launch_override,
    })
}
