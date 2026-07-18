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
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;

use super::{
    attach_terminal_session, detach_terminal_session, list_terminal_sessions,
    rename_terminal_session, ActiveSession, ReadSource, SessionState, WriteSink,
};

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
    frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>>,
    display_title: Mutex<Option<String>>,
) -> (
    Arc<Mutex<Box<dyn Write + Send>>>,
    JoinHandle<()>,
    ActiveSession,
) {
    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None).expect("start_local_pty");
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
    };

    (writer_handle, drain_handle, session)
}

/// Inserts a fully-formed `ActiveSession` into a managed `SessionState`
/// and returns the live `session_id`. Used by the list / rename tests
/// that need a real session entry without exercising the PTY plumbing.
fn insert_test_session(state: &SessionState, session_id: &str) {
    let (read_source, write_sink, keepalive) =
        super::start_local_pty("local", &None, &None).expect("start_local_pty");
    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let display_title: Mutex<Option<String>> = Mutex::new(None);
    let active = ActiveSession {
        read_source,
        write_sink,
        _keepalive: keepalive,
        machine_id: "local".to_string(),
        created_at: 0,
        frontend_channel,
        display_title,
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

    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
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

    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
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

    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
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
    let subscribers = active.lock().expect("subscribers lock");
    assert_eq!(
        subscribers.len(),
        1,
        "re-attaching the same channel id must not grow the Vec"
    );
    assert_eq!(
        subscribers[0].id(),
        cid,
        "stored channel must be the one we attached"
    );
}

/// `seed_frontend_channel` is the helper `start_terminal_session` uses
/// to wire the caller's IPC channel as the initial subscriber. Shell
/// startup output must be deliverable to that channel so the first
/// prompt is not irretrievably lost (spec §1 AC #1 + previous-attempt
/// feedback #1).
#[test]
fn seed_frontend_channel_supplied_receives_send_chunk_output() {
    let (channel, captured) = capturing_channel();
    let seed = super::seed_frontend_channel(channel.clone());

    // The seed Vec must contain exactly the channel we supplied, no
    // copy / clone shenanigans.
    {
        let guard = seed.lock().expect("seed lock");
        assert_eq!(guard.len(), 1, "seed Vec must hold exactly one entry");
        assert_eq!(guard[0].id(), channel.id(), "stored channel id mismatch");
    }

    // Broadcasting a chunk through `send_chunk` must deliver to the
    // seed channel — proves the helper produces a wire-compatible
    // `Arc<Mutex<Vec<Channel<Vec<u8>>>>>` for the drain thread.
    super::send_chunk(&seed, b"hello\r\n".to_vec());
    assert_eq!(
        *captured.lock().expect("captured lock"),
        b"hello\r\n".to_vec(),
        "seed channel must receive the broadcast chunk"
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

    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
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
    let subscribers = active.lock().expect("subscribers lock").len();
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

    let frontend_channel: Arc<Mutex<Vec<Channel<Vec<u8>>>>> =
        Arc::new(Mutex::new(vec![live_channel, dead]));

    // First broadcast: live channel receives the chunk, the dead
    // channel's send() fails, and `send_chunk` removes the dead entry.
    super::send_chunk(&frontend_channel, b"alpha\r\n".to_vec());

    assert_eq!(
        *captured_live.lock().expect("live lock"),
        b"alpha\r\n".to_vec(),
        "live channel must receive the first chunk"
    );

    // Confirm the dead channel has been pruned.
    let after_first = frontend_channel.lock().expect("subs lock").len();
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
        frontend_channel.lock().expect("subs lock").len(),
        1,
        "dead subscriber must stay pruned across multiple chunks"
    );

    // Sanity: the remaining entry is the live channel, not the dead
    // one (channel id is the only stable identity we have).
    let subscribers = frontend_channel.lock().expect("subs lock");
    assert_ne!(
        subscribers[0].id(),
        dead_id,
        "pruned dead channel must not reappear"
    );
}
