// Tests extracted from `src-tauri/src/terminal.rs` (mirrored-tests convention). `super` = that module.

use super::branch_bootstrap_line;

/// `None` (no pipeline context, e.g. `ProjectHome`) must skip the
/// bootstrap entirely — no `git checkout`, no `clear`, no noise.
#[test]
fn branch_bootstrap_returns_none_when_branch_absent() {
    assert!(branch_bootstrap_line(&None).is_none());
}

/// Empty / whitespace-only strings are treated as absent so a stray
/// `info.branch === ""` upstream never injects an empty-arg command.
#[test]
fn branch_bootstrap_returns_none_for_blank_branch() {
    assert!(branch_bootstrap_line(&Some(String::new())).is_none());
    assert!(branch_bootstrap_line(&Some("   ".to_string())).is_none());
}

/// A well-formed branch produces a `checkout || switch` line and
/// always ends with `clear\n` so the prompt lands on the new branch.
#[test]
fn branch_bootstrap_emits_checkout_then_switch_with_clear() {
    let line = branch_bootstrap_line(&Some("demeteo/features/abc".into()))
        .expect("bootstrap must be Some");
    assert!(
        line.starts_with("git checkout demeteo/features/abc"),
        "unexpected line: {line:?}"
    );
    assert!(
        line.contains("|| git switch demeteo/features/abc"),
        "missing switch fallback: {line:?}"
    );
    assert!(
        line.trim_end().ends_with("clear"),
        "missing clear: {line:?}"
    );
    assert!(
        line.ends_with('\n'),
        "must terminate with newline: {line:?}"
    );
}

/// Branch names containing shell metacharacters (`;`, `$`, quotes)
/// must be shell-escaped so a malicious / malformed feature id cannot
/// inject extra commands. The escape function itself is unit-tested
/// in `shared/shell.rs`; this test guards the wiring here.
#[test]
fn branch_bootstrap_escapes_shell_metacharacters() {
    let line =
        branch_bootstrap_line(&Some("evil;rm -rf /".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'evil;rm -rf /'"),
        "metachars must be wrapped in single quotes: {line:?}"
    );
    // The unescaped form must NOT appear — that would be the
    // command-injection vector.
    assert!(
        !line.contains(" checkout evil;rm"),
        "unescaped branch leaked into command: {line:?}"
    );
}

/// A `branch` with a stray single quote is the trickiest case: it
/// must be quoted and the inner `'` escaped via the standard
/// `'\''` POSIX trick.
#[test]
fn branch_bootstrap_handles_inner_single_quote() {
    let line = branch_bootstrap_line(&Some("feat'bad".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'feat'\\''bad'"),
        "inner single quote must be escaped: {line:?}"
    );
}

/// Surrounding whitespace is trimmed so `"  main  "` (e.g. from a UI
/// input) doesn't produce `git checkout   main` with extra spaces
/// that git refuses.
#[test]
fn branch_bootstrap_trims_surrounding_whitespace() {
    let line = branch_bootstrap_line(&Some("  feat/x  ".into())).expect("bootstrap must be Some");
    assert!(
        line.contains(" checkout feat/x "),
        "branch not trimmed: {line:?}"
    );
}

// =============================================================================
// Multi-channel broadcast + rename tests (implementation-spec.md §2.1, §4,
// §5). These exercise the new `Vec<Channel<Vec<u8>>>` storage and the new
// `rename_terminal_session` command while running synchronously against a
// real local PTY — no Tauri webview needed.
// =============================================================================

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;

use super::{
    attach_terminal_session, detach_terminal_session, list_terminal_sessions,
    rename_terminal_session, ActiveSession, Broadcast, ReadSource, SessionState, WriteSink,
};

/// Wraps a set of channels in an `Arc<Mutex<Broadcast>>` with an empty
/// scrollback — the shape `ActiveSession.frontend_channel` expects.
fn broadcast_with(channels: Vec<Channel<Vec<u8>>>) -> Arc<Mutex<Broadcast>> {
    let mut broadcast = Broadcast::new();
    broadcast.channels = channels;
    Arc::new(Mutex::new(broadcast))
}

/// Number of currently-attached subscriber channels on a broadcast.
fn channel_count(broadcast: &Arc<Mutex<Broadcast>>) -> usize {
    broadcast.lock().expect("broadcast lock").channels.len()
}

/// Builds a `Channel<Vec<u8>>` whose `on_message` closure captures the most
/// recent payload into a shared `Vec<u8>` for assertions. `Vec<u8>` is
/// serialised through `serde_json` (Tauri's `IpcResponse` impl for any
/// `Serialize`), so the closure deserialises the JSON array back into a
/// `Vec<u8>` before storing it.
fn capturing_channel() -> (Channel<Vec<u8>>, Arc<Mutex<Vec<u8>>>) {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json_str) = body {
            if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(&json_str) {
                *captured_clone.lock().expect("capture lock") = bytes;
            }
        }
        Ok(())
    });
    (channel, captured)
}

/// Builds a `Channel<Vec<u8>>` whose `on_message` closure APPENDS every
/// payload into a shared `Vec<u8>` so tests can assert on the
/// concatenation of multiple output chunks (e.g. "did channel A receive
/// the byte after the second attach?").
fn appending_capturing_channel() -> (Channel<Vec<u8>>, Arc<Mutex<Vec<u8>>>) {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json_str) = body {
            if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(&json_str) {
                captured_clone
                    .lock()
                    .expect("capture lock")
                    .extend_from_slice(&bytes);
            }
        }
        Ok(())
    });
    (channel, captured)
}

/// Builds a `Channel<Vec<u8>>` whose `on_message` closure always
/// returns `Err`. The drain thread / `send_chunk` helper must treat
/// such a channel as dead and prune it from the subscriber Vec so we
/// don't keep cloning every output chunk for a subscriber that can
/// never receive it. This stands in for the real-world "frontend
/// unmounted the surface and discarded the Channel" race.
fn dead_channel() -> Channel<Vec<u8>> {
    Channel::new(|_body| -> tauri::Result<()> { Err(tauri::Error::FailedToReceiveMessage) })
}

/// Polls `predicate` every 10ms until it returns `true` or `timeout_ms`
/// elapses. Final evaluation is performed after the loop to avoid an
/// off-by-one when the condition flips on the deadline tick.
fn wait_for(timeout_ms: u64, predicate: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

/// Builds a fully-formed `ActiveSession` for the local PTY path and a
/// drain-thread join handle. The drain thread loops on the PTY reader
/// and forwards every chunk through `send_chunk` to the supplied
/// `frontend_channel` (the SAME `Arc` the test then assigns to the
/// `ActiveSession` — that's how the broadcast wires up end-to-end).
/// Returns the writer handle so the test can push bytes through the
/// PTY master and the drain-thread `JoinHandle` for clean teardown.
/// The keepalive is `Arc::clone`d into both the drain thread closure
/// and the `ActiveSession`, so the PTY master + child outlive either.
#[allow(clippy::type_complexity)]
fn spawn_test_session(
    frontend_channel: Arc<Mutex<Broadcast>>,
    display_title: Mutex<Option<String>>,
) -> (
    Arc<Mutex<Box<dyn Write + Send>>>,
    JoinHandle<()>,
    ActiveSession,
) {
    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");
    let reader = match &read_source {
        ReadSource::LocalPty(r) => r.clone(),
        ReadSource::Ssh(_) => unreachable!("local pty path"),
    };
    let writer_handle: Arc<Mutex<Box<dyn Write + Send>>> = match &write_sink {
        WriteSink::LocalPty(w) => w.clone(),
        WriteSink::Ssh(_) => unreachable!("local pty path"),
    };
    let keepalive_for_thread = keepalive.clone();
    let frontend_channel_for_thread = frontend_channel.clone();

    let drain_handle = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            let read_result = reader.lock().expect("reader lock").read(&mut buffer);
            match read_result {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = buffer[..n].to_vec();
                    super::send_chunk(&frontend_channel_for_thread, chunk);
                }
            }
        }
        // Drop the keepalive clone at the very end — this is what
        // keeps the PTY master + shell child alive for the duration of
        // the test. Without it the master would close immediately and
        // the shell would see EOF before the assertions run.
        drop(keepalive_for_thread);
    });

    let session = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel,
        display_title,
        work_dir: None,
        work_branch: None,
        connected: Arc::new(AtomicBool::new(true)),
    };

    (writer_handle, drain_handle, session)
}

/// Inserts a fully-formed `ActiveSession` into a managed `SessionState`
/// and returns the live `session_id`. Used by the list / rename tests
/// that need a real session entry without exercising the PTY plumbing.
fn insert_test_session(state: &SessionState, session_id: &str) {
    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let display_title: Mutex<Option<String>> = Mutex::new(None);
    let active = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel,
        display_title,
        work_dir: None,
        work_branch: None,
        connected: Arc::new(AtomicBool::new(true)),
    };
    let mut sessions = state.sessions.lock().expect("sessions lock");
    sessions.insert(session_id.to_string(), active);
}

/// AC #5 — a single drain thread delivers each output chunk to every
/// currently-attached `Channel<Vec<u8>>` for the session.
#[test]
fn broadcast_to_multiple_channels() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let (writer_arc, _drain_handle, session) =
        spawn_test_session(frontend_channel.clone(), Mutex::new(None));

    let session_id = "sess_broadcast".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    // Attach two distinct channels via the real command path.
    let (channel_a, captured_a) = capturing_channel();
    let (channel_b, captured_b) = capturing_channel();
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    // Push bytes through the PTY master. The drain thread reads them
    // and `send_chunk` broadcasts to both attached channels. PTY
    // canonical mode translates `\n` → `\r\n` on the read side, so the
    // assertion compares against the translated output.
    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"hello\n").expect("write hello");
        w.flush().expect("flush hello");
    }

    assert!(
        wait_for(2_000, || !captured_a.lock().expect("a lock").is_empty()),
        "channel A never received the chunk"
    );
    assert!(
        wait_for(2_000, || !captured_b.lock().expect("b lock").is_empty()),
        "channel B never received the chunk"
    );
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"hello\r\n".to_vec(),
        "channel A payload mismatch"
    );
    assert_eq!(
        *captured_b.lock().expect("b lock"),
        b"hello\r\n".to_vec(),
        "channel B payload mismatch"
    );
}

/// AC #4 — after `attach A` → `detach` → `attach B`, channel A must
/// stop receiving output and channel B must receive it. This exercises
/// the "detach clears the Vec" semantics that lets the panel re-bind on
/// focus without leaking the old subscriber.
#[test]
fn attach_after_detach_rebinds_channel() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let (writer_arc, _drain_handle, session) =
        spawn_test_session(frontend_channel.clone(), Mutex::new(None));

    let session_id = "sess_rebind".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    // 1. Attach channel A.
    let (channel_a, captured_a) = capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");

    // Sanity-write: A must receive something so we know it's wired up.
    // PTY canonical mode converts `\n` → `\r\n` on the read side.
    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"a\n").expect("write a");
        w.flush().expect("flush a");
    }
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"a\r\n"),
        "channel A didn't receive the pre-detach byte"
    );

    // 2. Detach (clears the Vec).
    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach");

    // 3. Attach channel B.
    let (channel_b, captured_b) = capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    // 4. Write again. A must NOT receive it; B must.
    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"b\n").expect("write b");
        w.flush().expect("flush b");
    }

    assert!(
        wait_for(2_000, || captured_b.lock().expect("b lock").as_slice()
            == b"b\r\n"),
        "channel B didn't receive the post-rebind byte"
    );
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"a\r\n".to_vec(),
        "channel A must remain on its pre-detach payload"
    );
}

/// AC #3 — `detach_terminal_session` must NOT remove the session from
/// the `SessionState` HashMap. The panel relies on this to keep the
/// tab alive while its `TerminalSurface` is unmounted.
#[test]
fn list_sessions_returns_session_after_detach() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_detach_listing".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        insert_test_session(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach");

    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(
        listed.len(),
        1,
        "session must remain in the listing after detach"
    );
    assert_eq!(listed[0].session_id, session_id);
    assert_eq!(listed[0].machine_id, "local");
    assert_eq!(listed[0].title, None, "no rename yet → title is None");
}

/// Data-model extension — `rename_terminal_session` mutates the
/// `display_title` slot under the sessions lock, and the next
/// `list_terminal_sessions` call surfaces the new value.
#[test]
fn rename_terminal_session_updates_title_and_listing() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_rename".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        insert_test_session(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    rename_terminal_session(state_ref.clone(), session_id.clone(), "build".to_string())
        .expect("rename");

    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].title,
        Some("build".to_string()),
        "rename must surface in the listing"
    );

    // Whitespace-only / empty after trim → title cleared (back to None).
    rename_terminal_session(
        app.state::<SessionState>(),
        session_id.clone(),
        "   ".to_string(),
    )
    .expect("rename blank");
    let listed_after_clear = list_terminal_sessions(app.state::<SessionState>()).expect("list");
    assert_eq!(
        listed_after_clear[0].title, None,
        "blank title must clear the stored value"
    );

    // Long title → length-capped at 64 chars (character-count, not bytes).
    let long_title: String = "x".repeat(200);
    rename_terminal_session(
        app.state::<SessionState>(),
        session_id.clone(),
        long_title.clone(),
    )
    .expect("rename long");
    let listed_after_long = list_terminal_sessions(app.state::<SessionState>()).expect("list");
    let stored = listed_after_long[0]
        .title
        .as_ref()
        .expect("title stored after long rename");
    assert_eq!(stored.chars().count(), 64, "title must cap at 64 chars");
    assert!(
        stored.chars().all(|c| c == 'x'),
        "stored value must be the first 64 chars verbatim"
    );
}

/// `detach_terminal_session` with multiple attached subscribers must
/// remove only the most recently attached one (LIFO). The previous
/// "clear the whole vec" behaviour silently killed every other
/// tab's subscription whenever one tab unmounted; the LIFO policy
/// preserves the older subscribers' channels so a future split-view
/// tab or transient remount race cannot wipe out siblings.
#[test]
fn detach_only_removes_last_attached_subscriber() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let (writer_arc, _drain_handle, session) =
        spawn_test_session(frontend_channel.clone(), Mutex::new(None));

    let session_id = "sess_lifo_detach".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    // Attach A, then B. Both must receive broadcast chunks.
    let (channel_a, captured_a) = appending_capturing_channel();
    let (channel_b, captured_b) = appending_capturing_channel();
    let id_a = channel_a.id();
    let id_b = channel_b.id();
    assert_ne!(id_a, id_b, "capturing_channel must produce distinct ids");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"pre\n").expect("write pre");
        w.flush().expect("flush pre");
    }
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"pre\r\n"),
        "channel A didn't receive the pre-detach byte"
    );
    assert!(
        wait_for(2_000, || captured_b.lock().expect("b lock").as_slice()
            == b"pre\r\n"),
        "channel B didn't receive the pre-detach byte"
    );

    // Detach once. LIFO must remove B; A must remain subscribed.
    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach 1");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"post\n").expect("write post");
        w.flush().expect("flush post");
    }
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"pre\r\npost\r\n"),
        "channel A should still receive post-detach output"
    );
    assert_eq!(
        *captured_b.lock().expect("b lock"),
        b"pre\r\n".to_vec(),
        "channel B must not receive any further output after detach"
    );

    // Detach again. With A still in the Vec, the second pop removes A.
    // After this the Vec is empty but the session itself stays alive.
    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach 2");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"final\n").expect("write final");
        w.flush().expect("flush final");
    }
    // Give the drain thread a moment to forward any (incorrect) writes.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"pre\r\npost\r\n".to_vec(),
        "channel A must not receive output after its detach"
    );

    // Session itself still alive in the listing.
    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(
        listed.len(),
        1,
        "session must remain in the listing after detaches"
    );
}

/// `attach_terminal_session` is idempotent for the same channel id:
/// a rapid remount that re-attaches the same `Channel<Vec<u8>>` must
/// NOT grow the subscriber Vec. Otherwise stale duplicate subscribers
/// would clone every output chunk repeatedly and inflate the lock's
/// critical section on the hot path.
#[test]
fn attach_with_same_channel_id_is_idempotent() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_idem".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        insert_test_session(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    let (channel, _captured) = capturing_channel();
    let cid = channel.id();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel.clone())
        .expect("attach 1");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel.clone())
        .expect("attach 2");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach 3");

    // Inspect the Vec directly to confirm idempotency.
    let active = {
        let sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions
            .get(&session_id)
            .expect("session")
            .frontend_channel
            .clone()
    };
    let guard = active.lock().expect("subscribers lock");
    assert_eq!(
        guard.channels.len(),
        1,
        "re-attaching the same channel id must not grow the Vec"
    );
    assert_eq!(
        guard.channels[0].id(),
        cid,
        "stored channel must be the one we attached"
    );
}

/// Channel-specific detach: the caller passes the channel id it owns
/// and only that entry is removed. The V1 backward-compat fallback
/// (LIFO pop when no id is supplied) is covered by
/// `detach_only_removes_last_attached_subscriber`. This test exists
/// because the previous-attempt feedback flagged detach-without-id as
/// a multi-subscriber foot-gun — see `critic-review.md` #5 and the
/// validation report's AC #4 race.
#[test]
fn detach_with_channel_id_removes_only_matching() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let (writer_arc, _drain_handle, session) =
        spawn_test_session(frontend_channel.clone(), Mutex::new(None));

    let session_id = "sess_targeted_detach".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    let (channel_a, captured_a) = appending_capturing_channel();
    let (channel_b, captured_b) = appending_capturing_channel();
    let id_a = channel_a.id();
    let id_b = channel_b.id();
    assert_ne!(id_a, id_b);
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    // Detach by B's id. A MUST stay subscribed (this is the
    // channel-specific behaviour; the legacy LIFO would also pop B
    // here, but the test below proves id-based detach refuses to evict
    // an unknown id).
    detach_terminal_session(state_ref.clone(), session_id.clone(), Some(id_b)).expect("detach B");

    // Sanity-write: only A should receive it.
    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"only-a\n").expect("write");
        w.flush().expect("flush");
    }
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"only-a\r\n"),
        "channel A should still receive output after channel B was detached"
    );
    assert_eq!(
        *captured_b.lock().expect("b lock"),
        Vec::<u8>::new(),
        "channel B must not receive output after targeted detach"
    );

    // Now detach by A's id. After this, the Vec should be empty but
    // the session itself stays alive in the HashMap.
    detach_terminal_session(state_ref.clone(), session_id.clone(), Some(id_a)).expect("detach A");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"nobody\n").expect("write");
        w.flush().expect("flush");
    }
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"only-a\r\n".to_vec(),
        "channel A must not receive output after its targeted detach"
    );

    // Detach with an unknown id is a no-op (does NOT fall back to LIFO
    // pop — the caller committed to a specific id, and evicting a
    // peer subscriber would be worse than no-op).
    let unknown_id = id_b.wrapping_add(0xDEAD_BEEF_u32);
    detach_terminal_session(state_ref.clone(), session_id.clone(), Some(unknown_id))
        .expect("detach unknown");

    let active = {
        let sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions
            .get(&session_id)
            .expect("session")
            .frontend_channel
            .clone()
    };
    let subscribers = channel_count(&active);
    assert_eq!(subscribers, 0, "unknown-id detach must not evict anyone");

    // Session itself still alive in the listing.
    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(
        listed.len(),
        1,
        "session stays alive after targeted detaches"
    );
}

/// `send_chunk` prunes subscribers whose `send()` returns `Err`. This
/// prevents the hot path from cloning every output chunk for a
/// subscriber that can no longer receive it (a torn-down webview
/// Channel, a peer surface that lost its callback, etc.). Covered in
/// the previous-attempt feedback as the "stale-channel risk".
#[test]
fn send_chunk_prunes_dead_subscribers() {
    let (live_channel, captured_live) = capturing_channel();
    let dead = dead_channel();
    let dead_id = dead.id();

    let frontend_channel = broadcast_with(vec![live_channel, dead]);

    // First broadcast: live channel receives the chunk, the dead
    // channel's send() fails, and `send_chunk` removes the dead entry.
    super::send_chunk(&frontend_channel, b"alpha\r\n".to_vec());

    assert_eq!(
        *captured_live.lock().expect("live lock"),
        b"alpha\r\n".to_vec(),
        "live channel must receive the first chunk"
    );

    // Confirm the dead channel has been pruned.
    let after_first = channel_count(&frontend_channel);
    assert_eq!(
        after_first, 1,
        "dead subscriber must be pruned after a failed send"
    );

    // Subsequent broadcasts do NOT retry sending to the dead channel
    // (and the Vec length stays at 1 — the prune was permanent).
    super::send_chunk(&frontend_channel, b"beta\r\n".to_vec());
    assert_eq!(
        *captured_live.lock().expect("live lock"),
        b"beta\r\n".to_vec(),
        "live channel must keep receiving after the dead peer is pruned"
    );
    assert_eq!(
        channel_count(&frontend_channel),
        1,
        "dead subscriber must stay pruned across multiple chunks"
    );

    // Sanity: the remaining entry is the live channel, not the dead
    // one (channel id is the only stable identity we have).
    let guard = frontend_channel.lock().expect("subs lock");
    assert_ne!(
        guard.channels[0].id(),
        dead_id,
        "pruned dead channel must not reappear"
    );
}

// =============================================================================
// Scrollback broadcast tests (TERMINALS_VIEW_SPEC §3, §12). These exercise the
// `Broadcast { channels, scrollback }` replacement for the PR #58 seed-channel
// mechanism: a fresh attach replays accumulated output exactly once to the new
// channel, existing subscribers never re-see it, the ring trims at the byte
// cap on whole-chunk boundaries, and nothing is lost across the start→attach
// gap.
// =============================================================================

/// A `send_chunk` before any attach must accumulate into scrollback, and
/// the first `attach_terminal_session` replays that scrollback to the new
/// channel only — nothing is lost across the start→attach gap (F1/§3).
#[test]
fn attach_replays_scrollback_to_new_channel() {
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));

    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");
    let session = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel: frontend_channel.clone(),
        display_title: Mutex::new(None),
        work_dir: None,
        work_branch: None,
        connected: Arc::new(AtomicBool::new(true)),
    };
    let session_id = "sess_scrollback_replay".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    // Output produced BEFORE any surface attaches — accumulates in
    // scrollback with zero subscribers.
    super::send_chunk(&frontend_channel, b"startup-line\r\n".to_vec());

    // The surface mounts and attaches. It must receive the buffered
    // scrollback via the replay.
    let (channel, captured) = appending_capturing_channel();
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach");

    assert!(
        wait_for(2_000, || captured.lock().expect("lock").as_slice()
            == b"startup-line\r\n"),
        "attach must replay the pre-attach scrollback to the new channel"
    );
}

/// A second channel attaching after scrollback exists must replay it, but
/// a channel that was already attached when that scrollback was produced
/// must NOT re-receive it on the newcomer's attach (§3, §12).
#[test]
fn attach_replay_does_not_duplicate_for_existing_subscribers() {
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));

    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");
    let session = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel: frontend_channel.clone(),
        display_title: Mutex::new(None),
        work_dir: None,
        work_branch: None,
        connected: Arc::new(AtomicBool::new(true)),
    };
    let session_id = "sess_no_dup_replay".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    // Channel A attaches first (empty scrollback → no replay).
    let (channel_a, captured_a) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");

    // Output produced while only A is attached: A receives it live and it
    // lands in scrollback.
    super::send_chunk(&frontend_channel, b"live-1\r\n".to_vec());
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"live-1\r\n"),
        "channel A must receive the live chunk"
    );

    // Channel B attaches — it must replay the scrollback (`live-1`)...
    let (channel_b, captured_b) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");
    assert!(
        wait_for(2_000, || captured_b.lock().expect("b lock").as_slice()
            == b"live-1\r\n"),
        "channel B must replay the scrollback on attach"
    );

    // ...but A must NOT see `live-1` a second time from B's attach.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"live-1\r\n".to_vec(),
        "existing subscriber A must not re-receive scrollback on B's attach"
    );

    // A further live chunk reaches both exactly once.
    super::send_chunk(&frontend_channel, b"live-2\r\n".to_vec());
    assert!(
        wait_for(2_000, || captured_a.lock().expect("a lock").as_slice()
            == b"live-1\r\nlive-2\r\n"),
        "A must receive the second live chunk exactly once"
    );
    assert!(
        wait_for(2_000, || captured_b.lock().expect("b lock").as_slice()
            == b"live-1\r\nlive-2\r\n"),
        "B must receive the second live chunk exactly once"
    );
}

/// The scrollback ring trims at `SCROLLBACK_MAX_BYTES` on whole-chunk
/// boundaries: the oldest chunks are dropped wholesale so a replay never
/// starts mid-chunk (§3, §8). We push chunks well past the cap and assert
/// the replayed buffer is bounded and preserves the most recent chunks
/// intact.
#[test]
fn scrollback_trims_at_cap_on_chunk_boundaries() {
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));

    // Each chunk is 64 KiB of a distinct byte; 8 chunks = 512 KiB, twice
    // the 256 KiB cap, so the oldest chunks must be dropped.
    const CHUNK: usize = 64 * 1024;
    for marker in 0u8..8 {
        super::send_chunk(&frontend_channel, vec![marker; CHUNK]);
    }

    let guard = frontend_channel.lock().expect("broadcast lock");
    // Bounded: total retained bytes never exceed the cap.
    assert!(
        guard.scrollback_bytes <= super::SCROLLBACK_MAX_BYTES,
        "scrollback_bytes {} exceeded the cap",
        guard.scrollback_bytes
    );
    // Whole-chunk boundaries: every retained chunk is a full 64 KiB block
    // of a single marker byte — never a partial slice.
    for chunk in &guard.scrollback {
        assert_eq!(chunk.len(), CHUNK, "a retained chunk was split");
        let first = chunk[0];
        assert!(
            chunk.iter().all(|&b| b == first),
            "a retained chunk mixed markers — it was cut mid-chunk"
        );
    }
    // Most-recent-wins: the last chunk pushed (marker 7) is still present.
    let newest = guard.scrollback.back().expect("non-empty scrollback");
    assert_eq!(newest[0], 7, "the most recent chunk must be retained");
}

// =============================================================================
// Disconnect / reconnect tests (TERMINALS_VIEW_SPEC §3.1, §12). A dropped
// transport marks the session `disconnected` but keeps it (and its scrollback)
// in the map; `reconnect_terminal_session` rebuilds the transport in place,
// preserves scrollback, and refuses to run on an already-connected or unknown
// session.
// =============================================================================

use super::reconnect_with_machine;

/// The built-in `local` machine descriptor, used to drive
/// `reconnect_with_machine` without a full `AppContext`.
fn local_machine() -> crate::domain::models::Machine {
    crate::infrastructure::worktree::machine_resolver::local_machine()
}

/// Builds a disconnected `ActiveSession` (no live drain thread,
/// `connected = false`) with a pre-seeded scrollback and inserts it,
/// returning the shared `Broadcast` handle for assertions.
fn insert_disconnected_session(
    app: &tauri::App<tauri::test::MockRuntime>,
    session_id: &str,
    seed: &[u8],
) -> Arc<Mutex<Broadcast>> {
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    if !seed.is_empty() {
        super::send_chunk(&frontend_channel, seed.to_vec());
    }
    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");
    let session = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel: frontend_channel.clone(),
        display_title: Mutex::new(None),
        work_dir: None,
        work_branch: None,
        connected: Arc::new(AtomicBool::new(false)),
    };
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    state_ref
        .sessions
        .lock()
        .expect("sessions lock")
        .insert(session_id.to_string(), session);
    frontend_channel
}

/// The drain thread's `emit_disconnected` helper flips the shared
/// `connected` flag to `false` so the session is recognised as
/// disconnected (and reconnect-eligible) rather than removed.
#[test]
fn emit_disconnected_marks_session_not_connected() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    let connected = Arc::new(AtomicBool::new(true));
    super::emit_disconnected(&handle, "sess_x", "local", 0, &connected);
    assert!(
        !connected.load(Ordering::SeqCst),
        "an unexpected drain exit must mark the session disconnected"
    );
}

/// The connected→disconnected transition is claimed exactly once. An
/// explicit close (`close_terminal_session` / `close_machine_sessions`)
/// pre-sets `connected = false` before tearing the transport down; the
/// drain thread's trailing `emit_disconnected` must then find the flag
/// already claimed and bail, so no spurious `terminal-session-disconnected`
/// follows the `terminal-session-ended` a close already emitted.
#[test]
fn emit_disconnected_is_noop_once_transition_is_claimed() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();

    // Simulate an explicit close having already claimed the transition.
    let already_claimed = Arc::new(AtomicBool::new(false));
    // The drain thread's EOF calls this after teardown; the guard makes
    // it a no-op — it must neither re-claim nor panic.
    super::emit_disconnected(&handle, "sess_x", "local", 0, &already_claimed);
    assert!(
        !already_claimed.load(Ordering::SeqCst),
        "an already-claimed transition must stay claimed"
    );
}

/// Reconnecting a disconnected session rebuilds the transport in place:
/// the session stays in the map, `connected` flips back to `true`, and
/// the pre-disconnect scrollback is preserved as history (replayed to a
/// freshly-attached channel).
#[test]
fn reconnect_preserves_session_and_scrollback() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    app.manage(SessionState::default());

    let session_id = "sess_reconnect".to_string();
    let _broadcast = insert_disconnected_session(&app, &session_id, b"old-history\r\n");

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    reconnect_with_machine(&handle, &local_machine(), &state_ref, &session_id)
        .expect("reconnect must succeed on a disconnected session");

    // Session preserved and marked connected again.
    {
        let sessions = state_ref.sessions.lock().expect("sessions lock");
        let active = sessions.get(&session_id).expect("session preserved");
        assert!(
            active.connected.load(Ordering::SeqCst),
            "reconnect must mark the session connected"
        );
    }

    // Scrollback survived: a fresh attach replays the pre-disconnect
    // history (the new child may append a prompt after it, so match the
    // prefix).
    let (channel, captured) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach");
    assert!(
        wait_for(2_000, || captured
            .lock()
            .expect("lock")
            .starts_with(b"old-history\r\n")),
        "reconnect must preserve scrollback as replayable history"
    );
}

/// Reconnect refuses to run while the session is still connected (a live
/// transport is attached), so a stray reconnect can't spawn a second
/// child on the same session.
#[test]
fn reconnect_errors_when_already_connected() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    app.manage(SessionState::default());

    // A default `insert_test_session` is `connected = true`.
    let session_id = "sess_live".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        insert_test_session(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    let err = reconnect_with_machine(&handle, &local_machine(), &state_ref, &session_id)
        .expect_err("reconnect on a connected session must error");
    assert!(
        err.to_lowercase().contains("already connected"),
        "unexpected error: {err}"
    );
}

/// Reconnecting an unknown session id is an error, not a silent spawn.
#[test]
fn reconnect_errors_on_unknown_session() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    app.manage(SessionState::default());

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    let err = reconnect_with_machine(&handle, &local_machine(), &state_ref, "sess_nope")
        .expect_err("reconnect on an unknown id must error");
    assert!(
        err.to_lowercase().contains("not found"),
        "unexpected error: {err}"
    );
}
