use std::collections::HashMap as StdHashMap;
use std::time::{Duration, Instant};

use tauri::Manager;

use super::support::{activity_last_emitted, TestSessionBuilder};
use super::{
    apply_cadence, apply_hook, apply_screen, cadence_state, decide_and_record, resolve,
    should_clear_activity_on_agent_exit, sweep_activity_once, SessionActivity, SessionState,
    CADENCE_WINDOW,
};

#[test]
fn cadence_state_resolves_by_window() {
    assert_eq!(cadence_state(Duration::from_millis(0)), "working");
    assert_eq!(
        cadence_state(CADENCE_WINDOW),
        "working",
        "output exactly at the window edge still counts as working"
    );
    assert_eq!(
        cadence_state(CADENCE_WINDOW + Duration::from_millis(200)),
        "awaiting_input"
    );
}

#[test]
fn resolve_floor_passes_cadence_through() {
    let mut sa = SessionActivity::default();
    assert_eq!(resolve(&sa), "working");
    apply_cadence(&mut sa, "working");
    assert_eq!(resolve(&sa), "working");
    apply_cadence(&mut sa, "awaiting_input");
    assert_eq!(resolve(&sa), "awaiting_input");
}

#[test]
fn latch_survives_cadence_tick() {
    let mut sa = SessionActivity::default();
    apply_hook(&mut sa, "awaiting_approval");
    apply_cadence(&mut sa, "awaiting_input");
    assert_eq!(
        resolve(&sa),
        "awaiting_approval",
        "the approval latch must survive the cadence floor's awaiting_input"
    );
}

#[test]
fn latch_clears_on_working_hook() {
    let mut sa = SessionActivity::default();
    apply_hook(&mut sa, "awaiting_approval");
    apply_hook(&mut sa, "working");
    assert_eq!(resolve(&sa), "working");
}

#[test]
fn clear_on_agent_exit_only_with_record_and_departure() {
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();
    map.insert("s1".to_string(), SessionActivity::default());

    assert!(should_clear_activity_on_agent_exit(&map, "s1", true));
    assert!(!should_clear_activity_on_agent_exit(&map, "absent", true));
    assert!(!should_clear_activity_on_agent_exit(&map, "s1", false));
}

#[test]
fn agent_exit_clears_stuck_working_record() {
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();
    let mut sa = SessionActivity::default();
    apply_hook(&mut sa, "working");
    sa.last_emitted = Some("working".to_string());
    map.insert("s1".to_string(), sa);
    assert_eq!(resolve(map.get("s1").unwrap()), "working");

    apply_hook(map.get_mut("s1").unwrap(), "exit");
    let emitted = decide_and_record(&mut map, "s1");
    assert_eq!(emitted.as_deref(), Some("exit"));
    assert!(
        !map.contains_key("s1"),
        "an exited session's record is removed so a reused id starts clean"
    );
}

#[test]
fn latch_clears_on_awaiting_input_hook() {
    let mut sa = SessionActivity::default();
    apply_hook(&mut sa, "awaiting_approval");
    apply_hook(&mut sa, "awaiting_input");
    assert_eq!(resolve(&sa), "awaiting_input");
}

#[test]
fn hook_awaiting_input_survives_cadence_working_tick() {
    let mut sa = SessionActivity::default();
    apply_hook(&mut sa, "working");
    apply_hook(&mut sa, "awaiting_input");
    assert_eq!(resolve(&sa), "awaiting_input");
    apply_cadence(&mut sa, "working");
    assert_eq!(
        resolve(&sa),
        "awaiting_input",
        "a hooked session's awaiting_input must survive the never-quiet cadence floor"
    );
    apply_hook(&mut sa, "working");
    assert_eq!(resolve(&sa), "working");
}

#[test]
fn canonical_ordering_latches_then_clears() {
    let id = "sess";
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();

    apply_hook(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));

    apply_hook(map.entry(id.into()).or_default(), "awaiting_approval");
    assert_eq!(
        decide_and_record(&mut map, id).as_deref(),
        Some("awaiting_approval")
    );

    apply_cadence(map.entry(id.into()).or_default(), "awaiting_input");
    assert_eq!(
        decide_and_record(&mut map, id),
        None,
        "a cadence awaiting_input must not disturb the latched approval"
    );

    apply_hook(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));
}

#[test]
fn dedup_suppresses_unchanged_state() {
    let id = "sess";
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();

    apply_cadence(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));
    apply_cadence(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id), None);
    apply_cadence(map.entry(id.into()).or_default(), "awaiting_input");
    assert_eq!(
        decide_and_record(&mut map, id).as_deref(),
        Some("awaiting_input")
    );
}

#[test]
fn exit_emits_and_clears_record() {
    let id = "sess";
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();

    apply_cadence(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));

    apply_hook(map.entry(id.into()).or_default(), "exit");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("exit"));
    assert!(
        !map.contains_key(id),
        "the record must be dropped after exit so a reused id starts clean"
    );
}

#[test]
fn screen_latch_survives_cadence_tick() {
    let mut sa = SessionActivity::default();
    apply_screen(&mut sa, true);
    apply_cadence(&mut sa, "awaiting_input");
    assert_eq!(
        resolve(&sa),
        "awaiting_approval",
        "a rendered approval prompt must survive the cadence floor's awaiting_input"
    );
}

#[test]
fn screen_latch_survives_cadence_working_tick() {
    let mut sa = SessionActivity::default();
    apply_screen(&mut sa, true);
    apply_cadence(&mut sa, "working");
    assert_eq!(resolve(&sa), "awaiting_approval");
}

#[test]
fn screen_latch_clears_on_retract() {
    let mut sa = SessionActivity::default();
    apply_cadence(&mut sa, "working");
    apply_screen(&mut sa, true);
    assert_eq!(resolve(&sa), "awaiting_approval");
    apply_screen(&mut sa, false);
    assert_eq!(
        resolve(&sa),
        "working",
        "retracting the on-screen prompt clears the screen latch"
    );
}

#[test]
fn screen_and_hook_latches_are_independent() {
    let mut sa = SessionActivity::default();
    apply_screen(&mut sa, true);
    apply_hook(&mut sa, "working");
    assert_eq!(resolve(&sa), "awaiting_approval");
    apply_screen(&mut sa, false);
    assert_eq!(resolve(&sa), "working");
}

#[test]
fn canonical_screen_ordering_latches_then_clears() {
    let id = "sess";
    let mut map: StdHashMap<String, SessionActivity> = StdHashMap::new();

    apply_cadence(map.entry(id.into()).or_default(), "working");
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));

    apply_screen(map.entry(id.into()).or_default(), true);
    assert_eq!(
        decide_and_record(&mut map, id).as_deref(),
        Some("awaiting_approval")
    );

    apply_cadence(map.entry(id.into()).or_default(), "working");
    assert_eq!(
        decide_and_record(&mut map, id),
        None,
        "a cadence working tick must not disturb the latched screen approval"
    );

    apply_screen(map.entry(id.into()).or_default(), false);
    assert_eq!(decide_and_record(&mut map, id).as_deref(), Some("working"));
}

#[test]
fn sweep_records_agent_sessions_and_gates_plain_shells() {
    let app = tauri::test::mock_app();
    app.manage(SessionState::default());

    let agent_id = "sess_activity_agent".to_string();
    let shell_id = "sess_activity_shell".to_string();

    let agent_last_output = {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new()
            .agent("claude-code")
            .install(&state_ref, &agent_id);
        TestSessionBuilder::new().install(&state_ref, &shell_id);
        let sessions = state_ref.sessions.lock().expect("sessions lock");
        sessions
            .get(&agent_id)
            .expect("agent session")
            .last_output_at
            .clone()
    };

    sweep_activity_once(app.handle());
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    assert_eq!(
        activity_last_emitted(&state_ref, &agent_id).as_deref(),
        Some("working"),
        "agent session with recent output must resolve working"
    );
    assert!(
        activity_last_emitted(&state_ref, &shell_id).is_none(),
        "a plain shell (agent None) must never produce a record"
    );

    *agent_last_output.lock().expect("last_output lock") =
        Instant::now() - (CADENCE_WINDOW + Duration::from_secs(1));

    sweep_activity_once(app.handle());
    assert_eq!(
        activity_last_emitted(&state_ref, &agent_id).as_deref(),
        Some("awaiting_input"),
        "agent session gone quiet must transition to awaiting_input"
    );
    assert!(
        activity_last_emitted(&state_ref, &shell_id).is_none(),
        "plain shell must remain gated across sweeps"
    );
}

#[test]
fn sweep_skips_hooked_sessions() {
    let app = tauri::test::mock_app();
    app.manage(SessionState::default());

    let hooked_id = "sess_hooked_agent".to_string();
    let unhooked_id = "sess_unhooked_agent".to_string();

    {
        let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
        TestSessionBuilder::new()
            .agent("claude-code")
            .activity_nonce("deadbeef")
            .install(&state_ref, &hooked_id);
        TestSessionBuilder::new()
            .agent("claude-code")
            .install(&state_ref, &unhooked_id);
    }

    sweep_activity_once(app.handle());
    let state_ref: tauri::State<'_, SessionState> = app.state::<SessionState>();
    assert!(
        activity_last_emitted(&state_ref, &hooked_id).is_none(),
        "a hooked session must be skipped by the cadence sweep (scanner-driven only)"
    );
    assert_eq!(
        activity_last_emitted(&state_ref, &unhooked_id).as_deref(),
        Some("working"),
        "an unhooked agent session still resolves via the cadence floor"
    );
}
