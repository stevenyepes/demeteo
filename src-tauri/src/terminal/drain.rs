use ssh2::Channel;
use std::io::Read;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::activity::{apply_hook, resolve_and_emit};
use super::activity_scanner::ActivityScanner;
use super::model::{
    elapsed_since_last_output, touch_last_output, Broadcast, ReadSource, SessionInfo, SessionState,
    IDLE_TIMEOUT_SECS,
};

/// Spawns the appropriate drain thread for a freshly-built transport,
/// forwarding output into the session's `Broadcast`. Shared by
/// `start_terminal_session` and `reconnect_terminal_session` so both wire
/// up the drain identically (TERMINALS_VIEW_SPEC §3.1).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_drain<R: Runtime>(
    read_source: &ReadSource,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
    nonce: Option<String>,
) {
    match read_source {
        ReadSource::Ssh(ch) => {
            let ch = ch.clone();
            thread::spawn(move || {
                drain_ssh(
                    ch,
                    app,
                    session_id,
                    machine_id,
                    created_at,
                    frontend_channel,
                    connected,
                    last_output_at,
                    nonce,
                );
            });
        }
        ReadSource::LocalPty(reader) => {
            let reader = reader.clone();
            thread::spawn(move || {
                drain_local(
                    reader,
                    app,
                    session_id,
                    machine_id,
                    created_at,
                    frontend_channel,
                    connected,
                    last_output_at,
                    nonce,
                );
            });
        }
    }
}

/// Feed one drain chunk through a session's activity scanner: broadcast the
/// stripped `forward` bytes (our OSC removed) and hand back the parsed activity
/// states for the caller to emit. Factored out of the drain loop so the
/// feed→forward→events wiring is unit-testable without a live Tauri `emit`
/// (TERMINAL_ACTIVITY §6). The drain calls this, then emits one
/// `terminal-session-activity` per returned state.
pub(crate) fn drain_scan_and_forward(
    scanner: &mut ActivityScanner,
    chunk: &[u8],
    frontend_channel: &Arc<Mutex<Broadcast>>,
) -> Vec<String> {
    let out = scanner.feed(chunk);
    send_chunk(frontend_channel, out.forward);
    out.events
}

/// Broadcast one drain chunk, running it through the session's activity
/// scanner first when the session is hooked (`scanner` present). Shared by
/// `drain_local` and `drain_ssh` so both transports handle activity
/// identically — which is what lets remote reuse the scanner unchanged in
/// Phase 4. A plain shell (`scanner` `None`) keeps the raw forward path.
fn forward_drained_chunk<R: Runtime>(
    scanner: Option<&mut ActivityScanner>,
    chunk: &[u8],
    frontend_channel: &Arc<Mutex<Broadcast>>,
    app: &AppHandle<R>,
    session_id: &str,
) {
    match scanner {
        Some(sc) => {
            let states = drain_scan_and_forward(sc, chunk, frontend_channel);
            if states.is_empty() {
                return;
            }
            // Route each scanner state through the shared per-session resolver
            // instead of emitting directly, so the §2 precedence latch decides
            // what actually reaches the wire (an `awaiting_approval` now
            // survives the next cadence tick's `awaiting_input`). The resolver
            // reaches `SessionState` the same way the sweep does.
            let session_state = app.state::<SessionState>();
            for state in states {
                resolve_and_emit(app, session_id, &session_state.activity, |sa| {
                    apply_hook(sa, &state);
                });
            }
        }
        None => send_chunk(frontend_channel, chunk.to_vec()),
    }
}

#[allow(clippy::too_many_arguments)]
fn drain_ssh<R: Runtime>(
    ch: Arc<Mutex<Channel>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
    nonce: Option<String>,
) {
    // Re-seed to "now" at drain start so the idle timeout (and the activity
    // sweep) measure from this transport's lifetime, not a stale value carried
    // over the shared field from a long-ago disconnect on reconnect.
    touch_last_output(&last_output_at);
    // A hooked session (nonce present) scans every chunk for our activity OSC;
    // a plain shell keeps the raw fast path untouched. Engages only on ESC.
    let mut scanner = nonce.map(ActivityScanner::new);
    let mut buffer = [0u8; 8192];
    loop {
        let result = match ch.lock() {
            Ok(mut channel) => channel.read(&mut buffer),
            Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
        };
        match result {
            Ok(0) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                // Feed the one shared last-output field (also read by the
                // idle-timeout check below and the activity sweep) so both
                // transports have a single source of truth.
                touch_last_output(&last_output_at);
                forward_drained_chunk(
                    scanner.as_mut(),
                    &buffer[..n],
                    &frontend_channel,
                    &app,
                    &session_id,
                );
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if elapsed_since_last_output(&last_output_at).as_secs() > IDLE_TIMEOUT_SECS {
                    emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_local<R: Runtime>(
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    app: AppHandle<R>,
    session_id: String,
    machine_id: String,
    created_at: u64,
    frontend_channel: Arc<Mutex<Broadcast>>,
    connected: Arc<AtomicBool>,
    last_output_at: Arc<Mutex<Instant>>,
    nonce: Option<String>,
) {
    // Re-seed to "now" at drain start so the activity sweep measures from this
    // transport's lifetime rather than a stale value carried over on reconnect.
    touch_last_output(&last_output_at);
    // A hooked session (nonce present) scans every chunk for our activity OSC;
    // a plain shell keeps the raw fast path untouched. Engages only on ESC.
    let mut scanner = nonce.map(ActivityScanner::new);
    let mut buffer = [0u8; 8192];
    loop {
        let result = match reader.lock() {
            Ok(mut source) => source.read(&mut buffer),
            Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
        };
        match result {
            Ok(0) | Err(_) => {
                emit_disconnected(&app, &session_id, &machine_id, created_at, &connected);
                break;
            }
            Ok(n) => {
                // Feed the shared last-output field the activity sweep reads.
                touch_last_output(&last_output_at);
                forward_drained_chunk(
                    scanner.as_mut(),
                    &buffer[..n],
                    &frontend_channel,
                    &app,
                    &session_id,
                );
            }
        }
    }
}

/// The transport (PTY/SSH child) dropped unexpectedly. Mark the session
/// disconnected but KEEP it in the map — its `Broadcast` (scrollback +
/// title) survives so `reconnect_terminal_session` can rebuild the
/// transport in place (TERMINALS_VIEW_SPEC §3.1). Distinct from
/// `emit_ended`, which fires only on an explicit close that removes the
/// session.
pub(crate) fn emit_disconnected<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    machine_id: &str,
    created_at: u64,
    connected: &Arc<AtomicBool>,
) {
    // Emit only on the genuine connected→disconnected transition. An
    // explicit close (`close_terminal_session` / `close_machine_sessions`)
    // pre-sets `connected = false` before tearing the transport down, so
    // the drain thread's subsequent EOF is swallowed here instead of
    // racing a spurious `terminal-session-disconnected` in *after* the
    // `terminal-session-ended` the close already emitted. `swap` returns
    // the previous value: `false` means someone (a close, or an earlier
    // disconnect) already claimed the transition, so we bail.
    if !connected.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = app.emit(
        "terminal-session-disconnected",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id: machine_id.to_string(),
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
}

/// The session was explicitly closed (tab close / kill-all / tray
/// cleanup) and removed from the map. Fires `terminal-session-ended` so
/// listeners can distinguish a permanent close from a recoverable
/// disconnect (TERMINALS_VIEW_SPEC §3.1).
pub(crate) fn emit_ended<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    machine_id: &str,
    created_at: u64,
) {
    let _ = app.emit(
        "terminal-session-ended",
        SessionInfo {
            session_id: session_id.to_string(),
            machine_id: machine_id.to_string(),
            created_at,
            title: None,
            machine_name: None,
            agent: None,
        },
    );
}

/// Appends a chunk to the session's scrollback and broadcasts it to
/// every currently-attached subscriber.
///
/// Under the lock we do two cheap things: push the chunk into the
/// scrollback ring (trimming to the byte cap on whole-chunk
/// boundaries) and clone the channel list. The actual `send()` calls
/// happen outside the lock so a slow/dead subscriber cannot block
/// attach/detach or the scrollback bookkeeping. Per-channel errors are
/// swallowed: one dead subscriber must not prevent the others from
/// receiving output. When a `send` fails, the dead channel is pruned so
/// subsequent chunks don't keep cloning the payload for a subscriber
/// that can never receive it (TERMINALS_VIEW_SPEC §3).
pub(crate) fn send_chunk(frontend_channel: &Arc<Mutex<Broadcast>>, chunk: Vec<u8>) {
    let snapshot: Vec<tauri::ipc::Channel<Vec<u8>>> = {
        let mut guard = match frontend_channel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        guard.push_scrollback(&chunk);
        guard.channels.clone()
    };
    for chan in &snapshot {
        if chan.send(chunk.clone()).is_err() {
            if let Ok(mut guard) = frontend_channel.lock() {
                guard.channels.retain(|c| c.id() != chan.id());
            }
        }
    }
}
