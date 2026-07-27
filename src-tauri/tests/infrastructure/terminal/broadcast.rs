use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tauri::Manager;

use super::support::{
    appending_capturing_channel, broadcast_with, capturing_channel, channel_count, dead_channel,
    wait_until, TestSessionBuilder,
};
use super::{
    attach_terminal_session, detach_terminal_session, list_terminal_sessions,
    rename_terminal_session, Broadcast, SessionState,
};

#[test]
fn broadcast_to_multiple_channels() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().live_drain().build();
    let writer_arc = handles.writer.expect("live drain writer");
    let _drain_handle = handles.drain;

    let session_id = "sess_broadcast".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    // Appending, not replacing: the PTY is free to deliver `hello\r\n` in more
    // than one chunk, and each side has to be judged on everything it received
    // rather than on whichever fragment landed last.
    let (channel_a, captured_a) = appending_capturing_channel();
    let (channel_b, captured_b) = appending_capturing_channel();
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"hello\n").expect("write hello");
        w.flush().expect("flush hello");
    }

    // Wait for the whole payload, not merely for the first byte of it. Waiting
    // on `!is_empty()` and then asserting the full contents is a race the test
    // lost under load: a chunk boundary after "he" satisfies "not empty", and
    // the assertion then compared a half-delivered buffer.
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"hello\r\n"),
        "channel A never received the full chunk, got {:?}",
        captured_a.lock().expect("a lock"),
    );
    assert!(
        wait_until(|| captured_b.lock().expect("b lock").as_slice() == b"hello\r\n"),
        "channel B never received the full chunk, got {:?}",
        captured_b.lock().expect("b lock"),
    );
}

#[test]
fn attach_after_detach_rebinds_channel() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().live_drain().build();
    let writer_arc = handles.writer.expect("live drain writer");
    let _drain_handle = handles.drain;

    let session_id = "sess_rebind".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    let (channel_a, captured_a) = capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"a\n").expect("write a");
        w.flush().expect("flush a");
    }
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"a\r\n"),
        "channel A didn't receive the pre-detach byte"
    );

    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach");

    let (channel_b, captured_b) = capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"b\n").expect("write b");
        w.flush().expect("flush b");
    }

    assert!(
        wait_until(|| captured_b.lock().expect("b lock").as_slice() == b"b\r\n"),
        "channel B didn't receive the post-rebind byte"
    );
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"a\r\n".to_vec(),
        "channel A must remain on its pre-detach payload"
    );
}

#[test]
fn list_sessions_returns_session_after_detach() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_detach_listing".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new().install(&state_ref, &session_id);
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

#[test]
fn rename_terminal_session_updates_title_and_listing() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_rename".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new().install(&state_ref, &session_id);
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

#[test]
fn detach_only_removes_last_attached_subscriber() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().live_drain().build();
    let writer_arc = handles.writer.expect("live drain writer");
    let _drain_handle = handles.drain;

    let session_id = "sess_lifo_detach".to_string();
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
    assert_ne!(id_a, id_b, "capturing_channel must produce distinct ids");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"pre\n").expect("write pre");
        w.flush().expect("flush pre");
    }
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"pre\r\n"),
        "channel A didn't receive the pre-detach byte"
    );
    assert!(
        wait_until(|| captured_b.lock().expect("b lock").as_slice() == b"pre\r\n"),
        "channel B didn't receive the pre-detach byte"
    );

    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach 1");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"post\n").expect("write post");
        w.flush().expect("flush post");
    }
    assert!(
        wait_until(|| { captured_a.lock().expect("a lock").as_slice() == b"pre\r\npost\r\n" }),
        "channel A should still receive post-detach output"
    );
    assert_eq!(
        *captured_b.lock().expect("b lock"),
        b"pre\r\n".to_vec(),
        "channel B must not receive any further output after detach"
    );

    detach_terminal_session(state_ref.clone(), session_id.clone(), None).expect("detach 2");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"final\n").expect("write final");
        w.flush().expect("flush final");
    }
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"pre\r\npost\r\n".to_vec(),
        "channel A must not receive output after its detach"
    );

    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(
        listed.len(),
        1,
        "session must remain in the listing after detaches"
    );
}

#[test]
fn attach_with_same_channel_id_is_idempotent() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let session_id = "sess_idem".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new().install(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    let (channel, _captured) = capturing_channel();
    let cid = channel.id();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel.clone())
        .expect("attach 1");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel.clone())
        .expect("attach 2");
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach 3");

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

#[test]
fn detach_with_channel_id_removes_only_matching() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().live_drain().build();
    let writer_arc = handles.writer.expect("live drain writer");
    let _drain_handle = handles.drain;

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

    detach_terminal_session(state_ref.clone(), session_id.clone(), Some(id_b)).expect("detach B");

    {
        let mut w = writer_arc.lock().expect("writer lock");
        w.write_all(b"only-a\n").expect("write");
        w.flush().expect("flush");
    }
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"only-a\r\n"),
        "channel A should still receive output after channel B was detached"
    );
    assert_eq!(
        *captured_b.lock().expect("b lock"),
        Vec::<u8>::new(),
        "channel B must not receive output after targeted detach"
    );

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

    let listed = list_terminal_sessions(state_ref).expect("list");
    assert_eq!(
        listed.len(),
        1,
        "session stays alive after targeted detaches"
    );
}

#[test]
fn send_chunk_prunes_dead_subscribers() {
    let (live_channel, captured_live) = capturing_channel();
    let dead = dead_channel();
    let dead_id = dead.id();

    let frontend_channel = broadcast_with(vec![live_channel, dead]);

    super::send_chunk(&frontend_channel, b"alpha\r\n".to_vec());

    assert_eq!(
        *captured_live.lock().expect("live lock"),
        b"alpha\r\n".to_vec(),
        "live channel must receive the first chunk"
    );

    let after_first = channel_count(&frontend_channel);
    assert_eq!(
        after_first, 1,
        "dead subscriber must be pruned after a failed send"
    );

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

    let guard = frontend_channel.lock().expect("subs lock");
    assert_ne!(
        guard.channels[0].id(),
        dead_id,
        "pruned dead channel must not reappear"
    );
}

#[test]
fn attach_replays_scrollback_to_new_channel() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().build();
    let session_id = "sess_scrollback_replay".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    super::send_chunk(&handles.broadcast, b"startup-line\r\n".to_vec());

    let (channel, captured) = appending_capturing_channel();
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach");

    assert!(
        wait_until(|| captured.lock().expect("lock").as_slice() == b"startup-line\r\n"),
        "attach must replay the pre-attach scrollback to the new channel"
    );
}

#[test]
fn attach_replay_does_not_duplicate_for_existing_subscribers() {
    let app = tauri::test::mock_app();
    let state = SessionState::default();
    app.manage(state);

    let (session, handles) = TestSessionBuilder::new().build();
    let session_id = "sess_no_dup_replay".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        let mut sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions.insert(session_id.clone(), session);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();

    let (channel_a, captured_a) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_a).expect("attach A");

    super::send_chunk(&handles.broadcast, b"live-1\r\n".to_vec());
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"live-1\r\n"),
        "channel A must receive the live chunk"
    );

    let (channel_b, captured_b) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel_b).expect("attach B");
    assert!(
        wait_until(|| captured_b.lock().expect("b lock").as_slice() == b"live-1\r\n"),
        "channel B must replay the scrollback on attach"
    );

    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *captured_a.lock().expect("a lock"),
        b"live-1\r\n".to_vec(),
        "existing subscriber A must not re-receive scrollback on B's attach"
    );

    super::send_chunk(&handles.broadcast, b"live-2\r\n".to_vec());
    assert!(
        wait_until(|| captured_a.lock().expect("a lock").as_slice() == b"live-1\r\nlive-2\r\n"),
        "A must receive the second live chunk exactly once"
    );
    assert!(
        wait_until(|| captured_b.lock().expect("b lock").as_slice() == b"live-1\r\nlive-2\r\n"),
        "B must receive the second live chunk exactly once"
    );
}

#[test]
fn scrollback_trims_at_cap_on_chunk_boundaries() {
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));

    const CHUNK: usize = 64 * 1024;
    for marker in 0u8..8 {
        super::send_chunk(&frontend_channel, vec![marker; CHUNK]);
    }

    let guard = frontend_channel.lock().expect("broadcast lock");
    assert!(
        guard.scrollback_bytes <= super::SCROLLBACK_MAX_BYTES,
        "scrollback_bytes {} exceeded the cap",
        guard.scrollback_bytes
    );
    for chunk in &guard.scrollback {
        assert_eq!(chunk.len(), CHUNK, "a retained chunk was split");
        let first = chunk[0];
        assert!(
            chunk.iter().all(|&b| b == first),
            "a retained chunk mixed markers — it was cut mid-chunk"
        );
    }
    let newest = guard.scrollback.back().expect("non-empty scrollback");
    assert_eq!(newest[0], 7, "the most recent chunk must be retained");
}
