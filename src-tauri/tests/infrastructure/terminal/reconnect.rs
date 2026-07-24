use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::Manager;

use super::support::{appending_capturing_channel, local_machine, wait_until, TestSessionBuilder};
use super::{attach_terminal_session, reconnect_with_machine, SessionState};

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

#[test]
fn emit_disconnected_is_noop_once_transition_is_claimed() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();

    let already_claimed = Arc::new(AtomicBool::new(false));
    super::emit_disconnected(&handle, "sess_x", "local", 0, &already_claimed);
    assert!(
        !already_claimed.load(Ordering::SeqCst),
        "an already-claimed transition must stay claimed"
    );
}

#[test]
fn reconnect_preserves_session_and_scrollback() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    app.manage(SessionState::default());

    let session_id = "sess_reconnect".to_string();
    TestSessionBuilder::new()
        .disconnected()
        .seed_scrollback(b"old-history\r\n")
        .install(&app.state::<SessionState>(), &session_id);

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    reconnect_with_machine(&handle, &local_machine(), &state_ref, &session_id)
        .expect("reconnect must succeed on a disconnected session");

    {
        let sessions = state_ref.sessions.lock().expect("sessions lock");
        let active = sessions.get(&session_id).expect("session preserved");
        assert!(
            active.connected.load(Ordering::SeqCst),
            "reconnect must mark the session connected"
        );
    }

    let (channel, captured) = appending_capturing_channel();
    attach_terminal_session(state_ref.clone(), session_id.clone(), channel).expect("attach");
    assert!(
        wait_until(|| captured
            .lock()
            .expect("lock")
            .starts_with(b"old-history\r\n")),
        "reconnect must preserve scrollback as replayable history"
    );
}

#[test]
fn reconnect_errors_when_already_connected() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    app.manage(SessionState::default());

    let session_id = "sess_live".to_string();
    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new().install(&state_ref, &session_id);
    }

    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    let err = reconnect_with_machine(&handle, &local_machine(), &state_ref, &session_id)
        .expect_err("reconnect on a connected session must error");
    assert!(
        err.to_lowercase().contains("already connected"),
        "unexpected error: {err}"
    );
}

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
