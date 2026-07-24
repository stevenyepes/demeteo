use super::model::{activity_state, ActivityInfo, SessionActivity, SessionState};
use crate::ports::notification::{DomainEvent, NotificationPort};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

// ---------------------------------------------------------------------------
// Activity cadence sweep (working ↔ awaiting_input)
// ---------------------------------------------------------------------------
//
// A second background poller — modelled on `spawn_agent_detector` — resolves
// the universal working/waiting floor from the byte cadence the drain already
// records in `ActiveSession.last_output_at` (TERMINAL_ACTIVITY_PLAN §3, §4).
// Every tick it snapshots each session under the lock, releases it, then for
// each session WITH an agent present resolves `working` (output within the
// cadence window) vs `awaiting_input` (gone quiet) and emits
// `terminal-session-activity` ONLY when that state changed since the last
// emit. Plain-shell sessions (agent `None`) are never emitted for.

/// Cadence window: output seen within this of a sweep tick reads as
/// `working`; quieter than this reads as `awaiting_input`
/// (TERMINAL_ACTIVITY_PLAN §7.2 — start at ~1s, tune against real agents).
pub(crate) const CADENCE_WINDOW: Duration = Duration::from_millis(1000);

/// Time between activity sweeps. ~250ms keeps `working` appearing within one
/// tick of the first byte and `awaiting_input` settling ≤ ~1s after silence
/// (TERMINAL_ACTIVITY_PLAN §5).
const ACTIVITY_SWEEP_INTERVAL: Duration = Duration::from_millis(250);

/// Pure cadence read for a single session, factored out of the sweep loop so it
/// is unit-testable without spinning a real thread. Resolve `working` when the
/// session produced output within the cadence window, else `awaiting_input`.
/// Dedup and the agent-gate no longer live here — the shared resolver
/// (`SessionActivity` / `resolve_and_emit`) owns them now.
pub(crate) fn cadence_state(since_last_output: Duration) -> &'static str {
    if since_last_output <= CADENCE_WINDOW {
        activity_state::WORKING
    } else {
        activity_state::AWAITING_INPUT
    }
}

// ---------------------------------------------------------------------------
// Activity precedence resolver (TERMINAL_ACTIVITY_PLAN §2)
// ---------------------------------------------------------------------------
//
// Both signal sources fold their reading into a session's `SessionActivity`
// and route through `resolve_and_emit`; `resolve` applies the §2 precedence,
// so the sources no longer race on the wire.

/// Apply one cadence read (from the sweep) to a session's record. Cadence is
/// the precedence floor — it never touches the approval latch.
pub(crate) fn apply_cadence(sa: &mut SessionActivity, cadence: &'static str) {
    sa.cadence = Some(cadence);
}

/// Apply one explicit scanner (hook) state to a session's record. The approval
/// latch is set by `awaiting_approval` and cleared by ANY non-approval explicit
/// signal — a `working` resume, a `Stop`/idle → `awaiting_input`, or `exit` —
/// which is exactly §2's "clears on a working signal or its own source
/// retracting."
pub(crate) fn apply_hook(sa: &mut SessionActivity, state: &str) {
    match state {
        activity_state::AWAITING_APPROVAL => sa.approval_latched = true,
        activity_state::WORKING => {
            sa.approval_latched = false;
            sa.hook = Some(activity_state::WORKING);
        }
        activity_state::AWAITING_INPUT => {
            sa.approval_latched = false;
            sa.hook = Some(activity_state::AWAITING_INPUT);
        }
        activity_state::EXIT => {
            sa.approval_latched = false;
            sa.exited = true;
        }
        // Unknown states are ignored — the scanner only ever yields the four
        // above, but keep the resolver total.
        _ => {}
    }
}

/// Apply one on-screen recognizer reading (Phase 3, T3.3) to a session's
/// record: `present` = the agent's approval prompt is currently rendered. Sets
/// or clears the screen-sourced approval latch and nothing else — recognition
/// is strict approval-only (it never asserts working/idle; the cadence floor
/// and hooks own those). The retraction (`present = false`) is the recognizer's
/// own source clearing, mirroring how `apply_hook` clears the hook latch on a
/// non-approval signal.
pub(crate) fn apply_screen(sa: &mut SessionActivity, present: bool) {
    sa.screen_approval = present;
}

/// Resolve the §2 precedence for a record (highest first): a seen `exit` wins,
/// then a latched `awaiting_approval` from EITHER source (hook or on-screen
/// recognizer), then an explicit hook working/awaiting_input (authoritative over
/// the cadence floor once the session's hooks have spoken), else the cadence
/// floor (`working` until the first cadence read). The hook tier is what stops a
/// TUI agent's never-quiet byte cadence from re-pinning an idle session to
/// `working` after its `Stop` hook reported `awaiting_input`.
pub(crate) fn resolve(sa: &SessionActivity) -> &'static str {
    if sa.exited {
        activity_state::EXIT
    } else if sa.approval_latched || sa.screen_approval {
        activity_state::AWAITING_APPROVAL
    } else if let Some(hook) = sa.hook {
        hook
    } else {
        sa.cadence.unwrap_or(activity_state::WORKING)
    }
}

/// Compute the emit decision for a session's record and fold it back in. If the
/// resolved state differs from what was last emitted, return it (and record it
/// as the new last-emitted); otherwise return `None` (dedup). On `exit` the
/// record is REMOVED after emitting once, so a reused session id starts clean.
/// Pure over the map — no `AppHandle` — so the precedence/dedup logic is
/// unit-testable in isolation (TERMINAL_ACTIVITY_PLAN §6).
pub(crate) fn decide_and_record(
    map: &mut HashMap<String, SessionActivity>,
    id: &str,
) -> Option<String> {
    // Single lookup: `resolve` returns a `&'static str` (it does not borrow the
    // record), so the `&mut` borrow ends before the `exit` branch removes the
    // entry.
    let sa = map.get_mut(id)?;
    let resolved = resolve(sa);
    if sa.last_emitted.as_deref() == Some(resolved) {
        return None;
    }
    if resolved == activity_state::EXIT {
        map.remove(id);
    } else {
        sa.last_emitted = Some(resolved.to_string());
    }
    Some(resolved.to_string())
}

/// Stuck-`working` backstop (T4.2). Whether the agent detector should clear a
/// session's activity because its agent just left the process tree
/// (`agent_left`, a Some→None transition). Guarded on an existing record: a
/// plain shell — or an agent that never emitted activity — has none, and must
/// NOT get a spurious `exit`.
///
/// Why the detector, not a silence TTL: the cadence floor SKIPS hooked sessions
/// (a TUI agent repaints continuously — blinking cursor, rotating tips — so its
/// byte stream never falls quiet), so silence can never reclaim a hooked
/// session, and the hook tier outranks cadence in `resolve`. If a `SessionEnd`
/// (or `Stop`) hook is lost, `working` would otherwise spin forever. The
/// LOCAL process detector is the one signal that reliably says "the agent is
/// gone" independent of the (possibly-lost) hook. The alive-but-idle case (lost
/// `Stop`, agent still running) is instead recovered by Claude's own
/// `idle_prompt` Notification (§2d → `awaiting_input`) and the next
/// `UserPromptSubmit`. Remote hooked sessions have no `ps` to lean on and rely
/// solely on those hook signals (documented gap, matching remote presence
/// detection's own constraint).
pub(crate) fn should_clear_activity_on_agent_exit(
    map: &HashMap<String, SessionActivity>,
    id: &str,
    agent_left: bool,
) -> bool {
    agent_left && map.contains_key(id)
}

/// The single emit choke point both signal sources route through. Under the
/// `activity` lock: fold the source's reading into the session's record
/// (`mutate`, creating the record on first signal), then compute the emit
/// decision. The lock is released BEFORE `app.emit` so we never hold
/// `SessionState.activity` across IPC (locking discipline: lock → mutate +
/// decide → unlock → emit).
pub(crate) fn resolve_and_emit<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    activity: &Mutex<HashMap<String, SessionActivity>>,
    mutate: impl FnOnce(&mut SessionActivity),
) {
    let emit = {
        let mut map = match activity.lock() {
            Ok(m) => m,
            Err(_) => return,
        };
        mutate(map.entry(id.to_string()).or_default());
        decide_and_record(&mut map, id)
    };
    if let Some(state) = emit {
        let _ = app.emit(
            "terminal-session-activity",
            ActivityInfo {
                session_id: id.to_string(),
                state: state.clone(),
            },
        );
        // A real transition into `awaiting_approval` is a needs-a-decision event:
        // route it through the NotificationPort so the OS notification fires when
        // demeteo is backgrounded/unfocused. The focus/permission/
        // `run_in_background` gating (and de-dup vs. the in-app indicator) lives in
        // the port adapter and is deliberately reused — not reimplemented here.
        // Because `resolve_and_emit` only yields `Some(..)` on a real transition
        // and `awaiting_approval` is a latch (§2), this fires exactly once per
        // approval gate. `try_state` (not `state`) so a context without the port
        // managed (e.g. unit tests) doesn't panic. No `sessions`/`activity` lock
        // is taken here — the lock was released above (lock-ordering safety), and
        // `label: None` keeps a tab-title lookup off this path for now.
        if state == activity_state::AWAITING_APPROVAL {
            if let Some(port) = app.try_state::<Arc<dyn NotificationPort>>() {
                // Fire on a detached thread. `port.emit` reads the
                // `run_in_background` preference and probes window
                // focus/visibility, and this choke point runs on the per-session
                // PTY drain thread (the scanner path) — doing that blocking work
                // inline would stall output forwarding for the session until the
                // probes return. The approval edge is rare and latched, so a
                // one-off thread is cheap. Clone the `Arc` out of the managed
                // state first so nothing borrows `app` into the thread.
                let port = port.inner().clone();
                let session_id = id.to_string();
                thread::spawn(move || {
                    let _ = port.emit(&DomainEvent::TerminalAwaitingApproval {
                        session_id,
                        label: None,
                    });
                });
            }
        }
    }
}

/// Frontend on-screen recognizer (Phase 3, T3.3) reports whether an agent's
/// approval prompt is currently rendered in a session. `present = true` latches
/// screen-sourced `awaiting_approval`; `present = false` retracts it. Routed
/// through the SAME resolver as the hook scanner and the cadence sweep, so the
/// §2 precedence, dedup, and the OS notification are reused verbatim — a
/// screen-sourced approval "behaves exactly like the hook-sourced one"
/// (TERMINAL_ACTIVITY_PLAN §Phase 3).
///
/// Agent-gated (defence in depth; the frontend already scans only agent tabs):
/// a session with no agent present — or an unknown/closed one — is ignored, so
/// a plain shell can never be pushed into `awaiting_approval`. A retraction for
/// a session that has no activity record yet is also a no-op: creating a fresh
/// record just to clear a latch that was never set would resolve to the cadence
/// default and emit a phantom `working`.
pub(crate) fn report_screen_activity_inner<R: Runtime>(
    app: &AppHandle<R>,
    session_state: &SessionState,
    session_id: &str,
    present: bool,
) -> Result<(), String> {
    let has_agent = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "session state lock poisoned".to_string())?;
        match sessions.get(session_id) {
            Some(s) => s.agent.lock().map(|g| g.is_some()).unwrap_or(false),
            // Unknown / already-closed session — nothing to report against.
            None => return Ok(()),
        }
    };
    if !has_agent {
        return Ok(());
    }
    // A retraction only matters when a record already exists; never CREATE one
    // here (a fresh record resolves to the cadence default `working`). The
    // recognizer only retracts after asserting, so the record normally exists.
    if !present {
        let exists = session_state
            .activity
            .lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false);
        if !exists {
            return Ok(());
        }
    }
    resolve_and_emit(app, session_id, &session_state.activity, |sa| {
        apply_screen(sa, present);
    });
    Ok(())
}

/// Spawn the background activity sweep. Runs for the lifetime of the app (a
/// cheap sleeping thread, like `spawn_agent_detector`); call once from setup.
pub fn spawn_activity_sweep<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        thread::sleep(ACTIVITY_SWEEP_INTERVAL);
        sweep_activity_once(&app);
    });
}

/// One sweep pass: snapshot each session's (agent-present, quiet-for) under
/// the `sessions` lock, release it, then feed the cadence read for every
/// agent-present session into the shared resolver. Mirrors
/// `detect_agents_once`'s snapshot-then-emit-outside-the-lock shape. Dedup and
/// the record now live in `SessionState.activity`, so a scanner
/// `awaiting_approval` survives the tick's `awaiting_input` (the §2 latch).
///
/// Agent-gate: a plain shell (agent `None`) is skipped, so it never creates or
/// emits a record. GC: records for sessions that disappeared are dropped so
/// their stale state can't linger and a reused id starts clean.
pub(crate) fn sweep_activity_once<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<SessionState>();

    // Snapshot (id, agent-present, hooked, elapsed-since-last-output) for every
    // session, then release the sessions lock before touching `activity`.
    // (Never hold `sessions` and `activity` nested — the resolver needs only
    // `activity`.)
    let snapshot: Vec<(String, bool, bool, Duration)> = {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        sessions
            .iter()
            .map(|(id, s)| {
                let has_agent = s.agent.lock().map(|g| g.is_some()).unwrap_or(false);
                let hooked = s.activity_nonce.is_some();
                let elapsed = super::model::elapsed_since_last_output(&s.last_output_at);
                (id.clone(), has_agent, hooked, elapsed)
            })
            .collect()
    };

    // GC: forget records for sessions that disappeared since the last sweep.
    // Briefly under the `activity` lock, released before the per-session emits.
    {
        let live: std::collections::HashSet<&str> =
            snapshot.iter().map(|(id, _, _, _)| id.as_str()).collect();
        if let Ok(mut map) = state.activity.lock() {
            map.retain(|id, _| live.contains(id.as_str()));
        }
    }

    for (id, has_agent, hooked, elapsed) in &snapshot {
        // Agent-gate: a plain shell never creates or emits a record.
        if !has_agent {
            continue;
        }
        // Hooked-gate: a hooked session (Claude via `--settings`) is driven
        // PURELY by its hook scanner on the drain path, not the cadence floor.
        // A TUI agent repaints continuously (blinking cursor, footer, rotating
        // placeholder tips) so its byte cadence never falls quiet — letting the
        // sweep emit `working` would pin a freshly-launched or idle Claude to a
        // false spinner in the window before/between hook events. Skipping it
        // means the session shows NO activity mark until a hook actually fires
        // (`UserPromptSubmit`→working, `Stop`→awaiting_input, …), which is the
        // honest signal (TERMINAL_ACTIVITY_PLAN §2/§3).
        if *hooked {
            continue;
        }
        let cadence = cadence_state(*elapsed);
        resolve_and_emit(app, id, &state.activity, |sa| apply_cadence(sa, cadence));
    }
}
