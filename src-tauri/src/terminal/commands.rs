use crate::domain::models::Machine;
use crate::state::AppContext;
use portable_pty::PtySize;
use std::io::Write;
use std::sync::atomic::Ordering;
use tauri::{ipc::Channel, AppHandle, Emitter, Runtime, State};

use super::activity::report_screen_activity_inner;
use super::drain::{emit_ended, spawn_drain};
use super::model::{
    SessionInfo, SessionKeepalive, SessionState, WriteSink, DEFAULT_TERM_COLS, DEFAULT_TERM_ROWS,
};
use super::transport::{start_local_pty, start_ssh_session};

#[tauri::command]
pub fn set_machine_secret(machine_id: String, secret: String) -> Result<(), String> {
    let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine_id))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .set_password(&secret)
        .map_err(|e| format!("Failed to store secret in keyring: {}", e))?;
    crate::credential_cache::set(&format!("machine_{}", machine_id), &secret);
    Ok(())
}

#[tauri::command]
pub fn delete_machine_secret(machine_id: String) -> Result<(), String> {
    let entry = keyring::Entry::new("demeteo", &format!("machine_{}", machine_id))
        .map_err(|e| format!("Keyring error: {}", e))?;
    let _ = entry.delete_credential();
    crate::credential_cache::invalidate(&format!("machine_{}", machine_id));
    Ok(())
}

#[tauri::command]
pub fn write_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                chan.write_all(data.as_bytes())
                    .map_err(|e| format!("Failed to write to terminal: {}", e))?;
                chan.flush()
                    .map_err(|e| format!("Failed to flush terminal: {}", e))?;
            }
            WriteSink::LocalPty(writer) => {
                let mut w = writer
                    .lock()
                    .map_err(|_| "Failed to lock PTY writer".to_string())?;
                w.write_all(data.as_bytes())
                    .map_err(|e| format!("Failed to write to PTY: {}", e))?;
                w.flush()
                    .map_err(|e| format!("Failed to flush PTY: {}", e))?;
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn resize_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                chan.request_pty_size(cols, rows, None, None)
                    .map_err(|e| format!("Failed to resize terminal: {}", e))?;
            }
            WriteSink::LocalPty(_) => {
                if let Ok(keepalive) = active._keepalive.lock() {
                    if let SessionKeepalive::LocalPty { master, .. } = &*keepalive {
                        master
                            .resize(PtySize {
                                rows: rows as u16,
                                cols: cols as u16,
                                pixel_width: 0,
                                pixel_height: 0,
                            })
                            .map_err(|e| format!("Failed to resize PTY: {}", e))?;
                    }
                }
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn close_terminal_session(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.remove(&session_id) {
        // Claim the connected→disconnected transition before tearing the
        // transport down, so the drain thread's EOF does not emit a
        // spurious `terminal-session-disconnected` after the
        // `terminal-session-ended` below.
        active.connected.store(false, Ordering::SeqCst);
        match &active.write_sink {
            WriteSink::Ssh(ch) => {
                let mut chan = ch
                    .lock()
                    .map_err(|_| "Failed to lock channel".to_string())?;
                let _ = chan.close();
            }
            WriteSink::LocalPty(_) => {
                // Child is killed when keepalive drops
            }
        }
        // Explicit close → permanent end. Drop the sessions lock before
        // emitting to keep the critical section tight.
        let machine_id = active.machine_id.clone();
        let created_at = active.created_at;
        drop(sessions);
        emit_ended(&app, &session_id, &machine_id, created_at);
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

#[tauri::command]
pub fn list_terminal_sessions(
    session_state: State<'_, SessionState>,
) -> Result<Vec<SessionInfo>, String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    Ok(sessions
        .iter()
        .map(|(id, s)| SessionInfo {
            session_id: id.clone(),
            machine_id: s.machine_id.clone(),
            created_at: s.created_at,
            title: s.display_title.lock().ok().and_then(|g| g.clone()),
            machine_name: Some(s.machine_name.clone()),
            agent: s.agent.lock().ok().and_then(|g| g.clone()),
        })
        .collect())
}

#[tauri::command]
pub fn close_machine_sessions(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    machine_id: String,
) -> Result<usize, String> {
    let ended: Vec<(String, u64)> = {
        let mut sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let to_close: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.machine_id == machine_id)
            .map(|(id, _)| id.clone())
            .collect();
        to_close
            .into_iter()
            .filter_map(|id| {
                sessions.remove(&id).map(|s| {
                    // Claim the transition so the drain thread's EOF is
                    // swallowed rather than emitting a spurious
                    // `terminal-session-disconnected` after the ended
                    // event below.
                    s.connected.store(false, Ordering::SeqCst);
                    (id, s.created_at)
                })
            })
            .collect()
    };
    let count = ended.len();
    // Explicit kill-all → each removed session is a permanent end.
    for (id, created_at) in &ended {
        emit_ended(&app, id, &machine_id, *created_at);
    }
    Ok(count)
}

#[tauri::command]
pub fn attach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    tauri_channel: Channel<Vec<u8>>,
) -> Result<(), String> {
    // Clone the `Broadcast` handle and drop the sessions lock before the
    // (potentially large) scrollback replay, so a replay never blocks
    // other session commands.
    let broadcast = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        sessions
            .get(&session_id)
            .ok_or_else(|| "Session not found".to_string())?
            .frontend_channel
            .clone()
    };

    let mut guard = broadcast
        .lock()
        .map_err(|_| "Failed to lock frontend channel".to_string())?;
    // Deduplicate by channel id: a rapid remount (useEffect cleanup
    // racing the next mount's attach) can otherwise pile duplicate
    // subscribers onto the Vec, and every output chunk would then
    // clone the payload once per stale entry. If the same id is
    // already present we replace it in place so the existing position
    // is preserved (LIFO detach stays predictable).
    let new_id = tauri_channel.id();
    if let Some(pos) = guard.channels.iter().position(|c| c.id() == new_id) {
        guard.channels[pos] = tauri_channel.clone();
    } else {
        guard.channels.push(tauri_channel.clone());
    }
    // Replay the accumulated scrollback to ONLY the newly-attached
    // channel, while still holding the `Broadcast` lock. Holding the
    // lock guarantees the ordering `scrollback → live`: any concurrent
    // `send_chunk` serializes on this lock, so it cannot enqueue a live
    // chunk to this channel before the scrollback lands, and — because
    // the chunk was appended to scrollback under the same lock — the
    // replay never duplicates a live chunk this subscriber also sees.
    // Existing subscribers are untouched: the replay targets the new
    // channel alone.
    if guard.scrollback_bytes > 0 {
        let replay = guard.snapshot_scrollback();
        let _ = tauri_channel.send(replay);
    }
    Ok(())
}

#[tauri::command]
pub fn detach_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    channel_id: Option<u32>,
) -> Result<(), String> {
    let sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    if let Some(active) = sessions.get(&session_id) {
        if let Ok(mut guard) = active.frontend_channel.lock() {
            match channel_id {
                Some(id) => {
                    // Channel-specific detach: the caller knows exactly
                    // which subscriber it owns (via `Channel::id()` on
                    // the frontend side) and only that entry is removed.
                    // This is race-safe against a fresh attach that
                    // happens to be racing the unmount cleanup of a
                    // previous surface — the cleanup can't accidentally
                    // pop the new channel.
                    let before = guard.channels.len();
                    guard.channels.retain(|c| c.id() != id);
                    if guard.channels.len() == before {
                        // Unknown id → no-op. We deliberately do NOT
                        // fall back to LIFO pop here: the caller
                        // committed to a specific id, and a stale id
                        // means either the attach raced us (and is now
                        // gone) or the caller is mistaken. Either way
                        // we don't want to evict a peer subscriber.
                    }
                }
                None => {
                    // Backward-compat fallback for callers that don't
                    // track channel identity: LIFO pop. Matches the V1
                    // single-subscriber semantics — the typical V1 case
                    // has only one channel attached per session, so
                    // the pop removes it.
                    guard.channels.pop();
                }
            }
        }
        Ok(())
    } else {
        Err("Session not found".to_string())
    }
}

/// Frontend on-screen recognizer (Phase 3, T3.3) reports whether an agent's
/// approval prompt is currently rendered in a session.
///
/// Thin Tauri entry point; the latch/retract semantics, the shared-resolver
/// routing and the agent gate are documented on
/// [`report_screen_activity_inner`], which does the work.
#[tauri::command]
pub fn report_terminal_screen_activity(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    session_id: String,
    present: bool,
) -> Result<(), String> {
    report_screen_activity_inner(&app, &session_state, &session_id, present)
}

/// Persist a user-supplied tab title for the session. The frontend calls
/// this on every commit of the inline rename input. We trim
/// surrounding whitespace and length-cap at 64 chars server-side so a
/// runaway UI input cannot balloon the panel or leak into log lines.
/// An empty string after trim is treated as "clear the title" and
/// stores `None` so the panel falls back to its default-name strategy.
#[tauri::command]
pub fn rename_terminal_session(
    session_state: State<'_, SessionState>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    const TITLE_MAX_CHARS: usize = 64;
    let trimmed = title.trim();
    let stored = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(TITLE_MAX_CHARS).collect::<String>())
    };
    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    let active = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    let mut title_slot = active
        .display_title
        .lock()
        .map_err(|_| "Failed to lock title".to_string())?;
    *title_slot = stored;
    Ok(())
}

/// Re-establish the transport for a disconnected session in place. The
/// session shell — id, scrollback, subscribers, title — survived the
/// drop; here we spawn a fresh PTY/SSH child (re-running the same cwd /
/// branch bootstrap), attach it to the SAME `Broadcast`, spawn a new
/// drain thread on it, and emit `terminal-session-running`. Scrollback is
/// preserved as history and the new child's output appends after it
/// (TERMINALS_VIEW_SPEC §3.1). Errors if the session id is unknown or the
/// session is still connected.
#[tauri::command]
pub fn reconnect_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    session_id: String,
) -> Result<(), String> {
    let machine_id = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        sessions
            .get(&session_id)
            .ok_or_else(|| "Session not found".to_string())?
            .machine_id
            .clone()
    };
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;
    reconnect_with_machine(&app, &machine, &session_state, &session_id)
}

/// Command-agnostic core of `reconnect_terminal_session` (machine already
/// resolved). Rebuilds the transport for a disconnected session in place,
/// re-attaches it to the existing `Broadcast`, spawns a fresh drain
/// thread, and emits `terminal-session-running`. Split out so it can be
/// tested against a `local_machine()` without a full `AppContext`.
pub(crate) fn reconnect_with_machine<R: Runtime>(
    app: &AppHandle<R>,
    machine: &Machine,
    session_state: &SessionState,
    session_id: &str,
) -> Result<(), String> {
    // Snapshot the spawn params + shared handles, then release the lock.
    let (
        machine_id,
        work_dir,
        work_branch,
        frontend_channel,
        created_at,
        connected,
        last_output_at,
        activity_nonce,
    ) = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let active = sessions
            .get(session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        (
            active.machine_id.clone(),
            active.work_dir.clone(),
            active.work_branch.clone(),
            active.frontend_channel.clone(),
            active.created_at,
            active.connected.clone(),
            active.last_output_at.clone(),
            // Rebuild the scanner with the SAME nonce the still-running agent's
            // hooks emit — a reconnect must keep parsing its activity OSC.
            active.activity_nonce.clone(),
        )
    };

    // Atomically claim the reconnect: `true` means a live transport is
    // still attached, so refuse (spec: "errors on an already-connected
    // id"). This also serialises two racing reconnect calls — only one
    // wins the CAS.
    if connected
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Session is already connected".to_string());
    }

    // Reconnect has no surface size to hand off yet; spawn at the default and
    // let the frontend's post-mount `resize_terminal_session` correct it.
    let built = if machine.auth_type == "local" {
        start_local_pty(
            &machine_id,
            &work_dir,
            &work_branch,
            DEFAULT_TERM_COLS,
            DEFAULT_TERM_ROWS,
        )
    } else {
        // Reconnect re-attaches to a still-running remote agent whose settings
        // file was SFTP'd at first launch and survives the reconnect (like the
        // local file, which `Drop` only removes on teardown), so no re-placement
        // is needed here (T4.1).
        start_ssh_session(
            machine,
            &work_dir,
            &work_branch,
            DEFAULT_TERM_COLS,
            DEFAULT_TERM_ROWS,
            None,
        )
    };
    let (read_source, write_sink, keepalive, child_pid, _remote_settings_path) = match built {
        Ok(t) => t,
        Err(e) => {
            connected.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.to_string(),
        machine_id.clone(),
        created_at,
        frontend_channel,
        connected.clone(),
        last_output_at,
        activity_nonce,
    );

    // Swap the fresh transport onto the existing session.
    {
        let mut sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "Failed to lock sessions".to_string())?;
        let active = match sessions.get_mut(session_id) {
            Some(a) => a,
            None => {
                // The session was closed while we were rebuilding. Undo
                // the claim; the freshly-built transport drops here and
                // its drain thread exits on the resulting EOF.
                connected.store(false, Ordering::SeqCst);
                return Err("Session not found".to_string());
            }
        };
        active.read_source = read_source;
        active.write_sink = write_sink;
        active._keepalive = keepalive;
        // The rebuilt transport spawned a fresh shell child — repoint the
        // detector at its pid (or clear it for a remote reconnect).
        active.child_pid = child_pid;
    }

    let _ = app.emit(
        "terminal-session-running",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id,
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
    Ok(())
}
