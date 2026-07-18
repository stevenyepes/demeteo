//! System-tray icon, menu, and the window hide-to-tray lifecycle.
//!
//! Kept out of `lib.rs` to honour the existing module split: `lib.rs` only
//! wires this in (`tray::build_tray(app.handle())` inside `.setup()` and a
//! call to [`cleanup_terminal_sessions`] on the true-quit close path).
//!
//! The tray reaches the `"main"` window and terminal state purely through the
//! cloneable [`tauri::AppHandle`]/[`tauri::Manager`] — no hand-rolled `Mutex`
//! around a tray handle (spec Constraint 8). A tray-backend failure (common on
//! headless / minimal Linux sessions) is logged and swallowed rather than
//! panicking startup (spec Constraint 9).

use crate::commands::app_session::{parse_run_in_background, RUN_IN_BACKGROUND_KEY};
use crate::state::AppContext;
use crate::terminal;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

/// Stable menu-item ids. These are a contract (spec §4): the tray menu-event
/// handler dispatches on them and tests assert they never drift.
pub const MENU_ID_SHOW: &str = "show";
pub const MENU_ID_HIDE: &str = "hide";
pub const MENU_ID_QUIT: &str = "quit";

/// The label used for the `"main"` webview window throughout the app.
const MAIN_WINDOW: &str = "main";

/// Records whether the system tray was actually created at startup.
///
/// Managed in Tauri state by [`build_tray`] so the `CloseRequested` handler can
/// consult it: when the tray backend is unavailable (headless / minimal Linux
/// sessions), closing the window must behave as if `run_in_background` were OFF.
/// Otherwise a user with background mode ON could hide the window with **no tray
/// icon to restore it and no tray "Quit" to exit** — an unrecoverable hidden
/// state with live sessions (spec §5 Linux edge case: "tray backend unavailable
/// → close behaves as OFF").
#[derive(Debug, Clone, Copy)]
pub struct TrayStatus {
    pub available: bool,
}

/// Whether a tray icon is actually available to restore/quit the window.
///
/// Returns `false` when [`TrayStatus`] was never managed (tray build failed
/// before it could register, or `build_tray` was never called) — the safe
/// default, since without a tray the only sane close behaviour is the OFF path.
pub fn tray_available<R: tauri::Runtime, M: Manager<R>>(manager: &M) -> bool {
    manager
        .try_state::<TrayStatus>()
        .map(|s| s.available)
        .unwrap_or(false)
}

/// What closing the main window should do, decided purely from the preference.
///
/// Factored out so the branch is unit-testable without a live window (the
/// `on_window_event` handler in `lib.rs` only translates this into
/// `prevent_close`+`hide` vs. session cleanup).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// `run_in_background` is ON — keep the process alive, just hide the window.
    HideToTray,
    /// `run_in_background` is OFF — tear down sessions and let the close proceed.
    Cleanup,
}

/// Map the effective "hide to tray" decision onto the close behaviour.
///
/// The caller passes `run_in_background && tray_available` — hiding to a tray
/// that doesn't exist would strand the window (see [`TrayStatus`]), so the
/// availability guard lives at the call site and this stays a pure 1-arg map.
pub fn close_action(hide_to_tray: bool) -> CloseAction {
    if hide_to_tray {
        CloseAction::HideToTray
    } else {
        CloseAction::Cleanup
    }
}

/// Read the `run_in_background` preference through any [`tauri::Manager`].
///
/// Reuses `commands::app_session::{RUN_IN_BACKGROUND_KEY, parse_run_in_background}`
/// (the same key + decode the `set_run_in_background` command writes) so the key
/// and encoding have a single source of truth and can never drift between the
/// writer and this close-path reader. An absent key or any value other than the
/// string `"true"` is treated as `false`, so a missing/corrupt row can never
/// flip the app into background mode. Returns `false` when the `AppContext` is
/// not (yet) managed.
pub fn run_in_background_enabled<R: tauri::Runtime, M: Manager<R>>(manager: &M) -> bool {
    manager
        .try_state::<AppContext>()
        .map(|ctx| {
            parse_run_in_background(
                ctx.app_settings
                    .get_app_session(RUN_IN_BACKGROUND_KEY)
                    .ok()
                    .flatten(),
            )
        })
        .unwrap_or(false)
}

/// Tear down every active terminal (SSH/PTY) session.
///
/// This is the exact loop that used to live inline in `lib.rs`'s
/// `CloseRequested` arm; it is shared by the close-while-OFF path and the tray
/// `"Quit"` handler so the two can never diverge. Runs only on a true quit —
/// never on the hide-to-tray path (spec Constraint 4).
pub fn cleanup_terminal_sessions<R: tauri::Runtime, M: Manager<R>>(manager: &M) {
    if let Some(state) = manager.try_state::<terminal::SessionState>() {
        if let Ok(sessions) = state.sessions.lock() {
            for active in sessions.values() {
                match &active.write_sink {
                    terminal::WriteSink::Ssh(ch) => {
                        if let Ok(mut chan) = ch.lock() {
                            let _ = chan.close();
                        }
                    }
                    terminal::WriteSink::LocalPty(_) => {
                        // Local PTY child is killed when keepalive drops.
                    }
                }
            }
        }
    }
}

/// Make the main window visible and focused.
///
/// `pub(crate)` so the single-instance callback in `lib.rs` can reuse this
/// exact unminimize/show/focus sequence when a second launch is intercepted,
/// instead of duplicating it.
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hide the main window (process keeps running in the background).
fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
}

/// Run the true-quit path: tear down sessions, then terminate the process.
fn quit_app(app: &tauri::AppHandle) {
    cleanup_terminal_sessions(app);
    app.exit(0);
}

/// Build the system-tray icon + menu and register its event handlers.
///
/// Never panics: a tray-backend failure is logged and swallowed so the app
/// still starts (spec AC-1 / Constraint 9). Callers therefore ignore the
/// `Ok(())` result — it is `Result` only so menu construction can bubble up an
/// unexpected internal error during development.
pub fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_ID_SHOW, "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, MENU_ID_HIDE, "Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_ID_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        // Render the glyph monochrome in the macOS menu bar (spec Q1:
        // `iconAsTemplate: true`); ignored on other platforms.
        .icon_as_template(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_ID_SHOW => show_main_window(app),
            MENU_ID_HIDE => hide_main_window(app),
            MENU_ID_QUIT => quit_app(app),
            _ => {}
        })
        // Left-click-to-restore. On Linux most tray backends never emit click
        // events (the menu is the only interaction), so this is effectively a
        // no-op there and the "Show" menu item is the supported restore path.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // Reuse the existing app window icon for the tray glyph (spec Q1: a bespoke
    // monochrome glyph is out of scope). Without an icon the tray entry can be
    // invisible/hard to hit on some platforms, so warn rather than silently
    // building an icon-less tray (critic m2).
    match app.default_window_icon() {
        Some(icon) => builder = builder.icon(icon.clone()),
        None => {
            tracing::warn!("no default window icon configured; building tray without an icon glyph")
        }
    }

    match builder.build(app) {
        Ok(_tray) => {
            // Record that the tray really came up so the close handler will
            // honour the hide-to-tray path (spec AC-4/AC-5).
            app.manage(TrayStatus { available: true });
            Ok(())
        }
        Err(e) => {
            // Degrade gracefully instead of aborting startup — the tray is a
            // convenience, and the window/close paths keep working without it.
            // Crucially, record the failure so `CloseRequested` falls back to
            // the OFF (cleanup + exit) path and never strands the app in a
            // hidden, unrecoverable state (spec §5 Linux edge case, critic C1).
            app.manage(TrayStatus { available: false });
            #[cfg(target_os = "linux")]
            tracing::warn!(
                error = %e,
                "system tray unavailable on this Linux session; continuing without a tray icon (close will behave as if run-in-background were OFF)"
            );
            #[cfg(not(target_os = "linux"))]
            tracing::warn!(error = %e, "system tray unavailable; continuing without a tray icon (close will behave as if run-in-background were OFF)");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hide-vs-cleanup decision follows the preference exactly.
    #[test]
    fn close_action_maps_preference() {
        assert_eq!(close_action(true), CloseAction::HideToTray);
        assert_eq!(close_action(false), CloseAction::Cleanup);
    }

    /// The tray menu ids are a stable contract the handler dispatches on.
    #[test]
    fn menu_ids_are_stable() {
        assert_eq!(MENU_ID_SHOW, "show");
        assert_eq!(MENU_ID_HIDE, "hide");
        assert_eq!(MENU_ID_QUIT, "quit");
    }

    /// With no `AppContext` managed, the preference reader must fall back to
    /// `false` — a missing/late-managed context can never flip the app into
    /// background mode (spec: default is OFF; Constraint 5). Exercised against a
    /// real (headless) `AppHandle` via Tauri's `MockRuntime`.
    #[test]
    fn run_in_background_defaults_off_without_context() {
        let app = tauri::test::mock_app();
        assert!(!run_in_background_enabled(app.handle()));
    }

    /// The shared session-cleanup helper must degrade to a no-op (never panic)
    /// when `SessionState` is not managed — the tray `Quit` path and the OFF
    /// close path both rely on this (spec Constraint 9: never panic).
    #[test]
    fn cleanup_is_noop_without_session_state() {
        let app = tauri::test::mock_app();
        cleanup_terminal_sessions(app.handle());
    }

    /// With no `TrayStatus` managed (tray build failed before registering, or
    /// `build_tray` was never called), availability is `false` — the safe
    /// default that keeps the close path from hiding to a nonexistent tray.
    #[test]
    fn tray_unavailable_without_status() {
        let app = tauri::test::mock_app();
        assert!(!tray_available(app.handle()));
    }

    /// `tray_available` reflects the managed `TrayStatus` flag.
    #[test]
    fn tray_available_reflects_managed_status() {
        let app = tauri::test::mock_app();
        app.manage(TrayStatus { available: true });
        assert!(tray_available(app.handle()));

        let app_off = tauri::test::mock_app();
        app_off.manage(TrayStatus { available: false });
        assert!(!tray_available(app_off.handle()));
    }

    /// The effective close decision requires BOTH the preference and a live
    /// tray: background ON but tray unavailable must fall back to `Cleanup`,
    /// preventing the unrecoverable hidden-window state (spec §5 Linux edge
    /// case). This mirrors the `hide_to_tray` expression in `lib.rs`.
    #[test]
    #[allow(clippy::nonminimal_bool)]
    fn hide_to_tray_requires_preference_and_tray() {
        // Compute the AND inside a closure so clippy's `nonminimal_bool` lint
        // cannot constant-fold the truth-table expressions into bare bools.
        fn combine(bg: bool, tray: bool) -> bool {
            bg && tray
        }
        // preference ON, tray up  -> hide
        assert_eq!(close_action(combine(true, true)), CloseAction::HideToTray);
        // preference ON, tray down -> cleanup (the C1 fallback)
        assert_eq!(close_action(combine(true, false)), CloseAction::Cleanup);
        // preference OFF, tray up  -> cleanup
        assert_eq!(close_action(combine(false, true)), CloseAction::Cleanup);
        // preference OFF, tray down -> cleanup
        assert_eq!(close_action(combine(false, false)), CloseAction::Cleanup);
    }
}
