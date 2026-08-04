use serde::Serialize;
use ssh2::Session;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

pub(crate) static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn session_counter_next() -> usize {
    SESSION_COUNTER.fetch_add(1, Ordering::SeqCst)
}

pub enum ReadSource {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    LocalPty(Arc<Mutex<Box<dyn Read + Send>>>),
}

pub enum WriteSink {
    Ssh(Arc<Mutex<ssh2::Channel>>),
    LocalPty(Arc<Mutex<Box<dyn Write + Send>>>),
}

pub struct ActiveSession {
    pub read_source: ReadSource,
    pub write_sink: WriteSink,
    /// Kept alive for the lifetime of the session.
    pub _keepalive: Arc<Mutex<SessionKeepalive>>,
    pub machine_id: String,
    /// Friendly machine name (`Machine.name`) resolved at start, so
    /// `list_terminal_sessions` can hand the frontend a human label for
    /// restored tabs instead of the raw machine id.
    pub machine_name: String,
    pub created_at: u64,
    /// OS process id of the local shell child, used by the foreground-agent
    /// detector to walk the process tree. `None` for SSH sessions (the
    /// child lives on the remote host, out of reach of local `ps`).
    pub child_pid: Option<u32>,
    /// Coding-agent kind currently detected in this session (`"claude-code"`,
    /// `"opencode"`, …) or `None` for a plain shell. Seeded from the launch
    /// command; the detector overwrites it for local sessions as the
    /// foreground process changes.
    pub agent: Mutex<Option<String>>,
    /// Per-session activity nonce for hooked agent kinds (Claude), or `None`
    /// for a plain shell or a non-hooked agent. Minted at `start` and embedded
    /// in the reporter hooks injected via `--settings`; the drain's
    /// `ActivityScanner` accepts only sequences carrying exactly this nonce
    /// (TERMINAL_ACTIVITY §2b — anti-spoof + cross-session TTY-bleed
    /// gate). Retained so `reconnect_terminal_session` rebuilds the scanner
    /// with the SAME nonce the still-running agent's hooks emit.
    pub activity_nonce: Option<String>,
    /// Path to the ephemeral, per-session settings file handed to Claude via
    /// `claude --settings <path>` (the reporter hooks — see
    /// `write_activity_settings_file`). `None` for a plain shell or a non-hooked
    /// agent. The file lives in the OS temp dir, is written at `start`, and is
    /// removed when this `ActiveSession` is dropped (`Drop` below). We pass a
    /// file PATH rather than inline JSON because the full `--settings '<json>'`
    /// command line is ~1.4 KB — well over a PTY's 1024-byte canonical-mode
    /// input limit (`MAX_CANON`), which truncated the launch line mid-quote so
    /// it never executed. A short `--settings <path>` sidesteps that entirely
    /// and, being a per-session throwaway, never touches the user's `~/.claude`
    /// or the project's `.claude/` settings.
    pub activity_settings_path: Option<PathBuf>,
    /// Wall-clock instant of the most recent chunk this session read off its
    /// PTY/SSH transport. Written by the drain thread on every chunk (local
    /// and SSH feed this one shared field) and read by the activity sweep to
    /// resolve `working` (recent output) vs `awaiting_input` (gone quiet).
    /// Seeded to the session's start instant so a freshly-started session
    /// reads as recently-active. Shared (`Arc`) so the drain thread can
    /// update it after the transport is swapped in on reconnect.
    pub last_output_at: Arc<Mutex<Instant>>,
    /// Output fan-out + scrollback for the session (TERMINALS_VIEW_SPEC
    /// §3). The drain thread appends every chunk to the scrollback ring
    /// and broadcasts it to every attached channel; a freshly-attached
    /// surface replays the accumulated scrollback so no output is ever
    /// lost between `start` and the first `attach`, and none is doubled.
    pub frontend_channel: Arc<Mutex<Broadcast>>,
    /// User-supplied tab title. `None` until the frontend calls
    /// `rename_terminal_session`; truncated/trimmed server-side.
    pub display_title: Mutex<Option<String>>,
    /// Spawn parameters retained so `reconnect_terminal_session` can
    /// rebuild an identical transport (same cwd / branch bootstrap) after
    /// an unexpected drop (TERMINALS_VIEW_SPEC §3.1).
    pub work_dir: Option<String>,
    pub work_branch: Option<String>,
    /// `true` while a live PTY/SSH child is attached. The drain thread
    /// flips it to `false` when it exits on an unexpected transport drop;
    /// `reconnect_terminal_session` flips it back to `true`. Reconnect
    /// refuses to run while this is `true` (transport still live).
    pub connected: Arc<AtomicBool>,
}

impl Drop for ActiveSession {
    /// Remove the ephemeral `--settings` file when the session is torn down.
    /// Fires on every removal path (explicit close, `close_machine_sessions`,
    /// the sweep GC) because they all drop the `ActiveSession`; a reconnect
    /// mutates the session in place and never drops it, so the file survives a
    /// reconnect as intended. Best-effort — a leftover temp file is harmless.
    fn drop(&mut self) {
        if let Some(path) = self.activity_settings_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Maximum bytes retained in a session's scrollback ring. Caps backend
/// memory per session; trimming happens on whole-chunk boundaries so a
/// replay never starts mid-escape-sequence (TERMINALS_VIEW_SPEC §3, §8).
pub(crate) const SCROLLBACK_MAX_BYTES: usize = 256 * 1024;

/// Fallback PTY dimensions used when the frontend does not supply a size at
/// session start (e.g. reconnect, or a caller that has not measured its
/// surface yet). Kept at the classic 80x24 — crucially *narrower* than any
/// realistic terminal viewport so the shell's first prompt never wraps wider
/// than the visible area (a wider default made Powerlevel10k's full-width
/// frame wrap and the command line appear duplicated). The frontend sends the
/// real size via `resize_terminal_session` right after it mounts and fits.
pub(crate) const DEFAULT_TERM_COLS: u16 = 80;
pub(crate) const DEFAULT_TERM_ROWS: u16 = 24;

/// Bounds a geometry must sit inside before it reaches a live PTY.
///
/// The floor sits above the degenerate sizes a frontend surface can measure
/// while it has no layout box (an 11x5 has been observed) and below any
/// viewport a user can actually see, so a resize a human did not ask for is
/// refused rather than repainting the agent's TUI into eleven columns. The
/// ceiling is what makes the `as u16` narrowing that [`PtySize`] requires
/// lossless: without it a 100_000-column request wraps to 34_464 and the PTY
/// is resized to a geometry nobody asked for.
///
/// [`PtySize`]: portable_pty::PtySize
pub(crate) const MIN_PTY_COLS: u32 = 20;
pub(crate) const MIN_PTY_ROWS: u32 = 5;
pub(crate) const MAX_PTY_DIM: u32 = 1000;

/// `Some((cols, rows))` when the geometry is plausible for a live PTY, `None`
/// otherwise. Pure and synchronous so the one decision both transports obey is
/// reachable from a test without a session, a channel, or a PTY.
pub fn checked_pty_size(cols: u32, rows: u32) -> Option<(u16, u16)> {
    let in_range = (MIN_PTY_COLS..=MAX_PTY_DIM).contains(&cols)
        && (MIN_PTY_ROWS..=MAX_PTY_DIM).contains(&rows);
    in_range.then_some((cols as u16, rows as u16))
}

/// Per-session output fan-out with a bounded scrollback buffer, all
/// guarded by a single mutex so attach-replay and live-broadcast are
/// exactly ordered (TERMINALS_VIEW_SPEC §3). Replaces the PR #58
/// seed-channel / `consumeStartupReplay` mechanism, which duplicated
/// output during the attach window and leaked a phantom subscriber.
pub struct Broadcast {
    /// Every attached surface is one element. The drain thread clones a
    /// snapshot under the lock then `send`s outside it, so a slow/dead
    /// subscriber never blocks the others.
    pub channels: Vec<Channel<Vec<u8>>>,
    /// Whole output chunks, oldest first. Never split a chunk — trimming
    /// on chunk boundaries keeps the first repaint from cutting an escape
    /// sequence.
    pub scrollback: VecDeque<Vec<u8>>,
    /// Running total of `scrollback` byte lengths (avoids re-summing the
    /// ring on every chunk).
    pub scrollback_bytes: usize,
}

impl Broadcast {
    pub(crate) fn new() -> Self {
        Broadcast {
            channels: Vec::new(),
            scrollback: VecDeque::new(),
            scrollback_bytes: 0,
        }
    }

    /// Append a chunk to the scrollback ring and trim to the byte cap on
    /// whole-chunk boundaries. Called under the `Broadcast` lock.
    pub(crate) fn push_scrollback(&mut self, chunk: &[u8]) {
        self.scrollback.push_back(chunk.to_vec());
        self.scrollback_bytes += chunk.len();
        while self.scrollback_bytes > SCROLLBACK_MAX_BYTES {
            match self.scrollback.pop_front() {
                Some(dropped) => self.scrollback_bytes -= dropped.len(),
                None => break,
            }
        }
    }

    /// Concatenate the whole scrollback ring into one buffer for replay
    /// to a newly-attached channel. Called under the `Broadcast` lock.
    pub(crate) fn snapshot_scrollback(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.scrollback_bytes);
        for chunk in &self.scrollback {
            out.extend_from_slice(chunk);
        }
        out
    }
}

pub enum SessionKeepalive {
    Ssh {
        #[allow(dead_code)]
        session: Session,
        #[allow(dead_code)]
        tcp: TcpStream,
    },
    LocalPty {
        /// Kept alive for PTY resize operations.
        master: Box<dyn portable_pty::MasterPty + Send>,
        #[allow(dead_code)]
        child: Box<dyn portable_pty::Child + Send + Sync>,
    },
}

/// The I/O handles a freshly started session hands back: the reader, the
/// writer, a shared keepalive guard, the child shell's pid (`None` when the
/// transport can't report one), and the remote path the reporter-hooks
/// `--settings` file was placed at (`Some` only for a remote *hooked* launch
/// whose SFTP write succeeded — `None` for local, for a plain shell, or when
/// the remote write failed and the launch degrades to unhooked; T4.1). Shared
/// by the local-PTY and SSH start paths so their signatures stay in lock-step.
pub(crate) type SessionHandles = (
    ReadSource,
    WriteSink,
    Arc<Mutex<SessionKeepalive>>,
    Option<u32>,
    Option<String>,
);

/// The per-session activity record both signal sources feed
/// (TERMINAL_ACTIVITY §2 precedence latch). Fields are folded in by
/// `apply_cadence` (the sweep) and `apply_hook` (the scanner); the resolved
/// state is computed by `resolve` and emitted (deduped) by `resolve_and_emit`.
#[derive(Default)]
pub struct SessionActivity {
    /// Last cadence read by the sweep: `Some("working")` or
    /// `Some("awaiting_input")`, `None` until the first tick. The precedence
    /// *floor* — only consulted for a session that has NOT reported an explicit
    /// hook state (`hook` is `None`).
    pub(super) cadence: Option<&'static str>,
    /// Last explicit working/awaiting_input reported by a hook, or `None` until
    /// the first one arrives. Once a session's hooks speak, the hook is
    /// **authoritative** over the cadence floor for the working ↔ awaiting_input
    /// axis: a TUI agent like Claude Code repaints continuously (blinking
    /// cursor, footer, the reporter OSC's own bytes), so the byte cadence never
    /// falls quiet and would otherwise pin the session to `working` forever
    /// even after `Stop` fired `awaiting_input`. Latching to the hook lets the
    /// cheap universal floor and the precise hook layer coexist without the
    /// floor clobbering the hook (TERMINAL_ACTIVITY §2).
    pub(super) hook: Option<&'static str>,
    /// Latched `true` by a scanner `awaiting_approval`; cleared by any
    /// non-approval explicit hook (working / awaiting_input / exit). While set
    /// it survives the cadence floor's `awaiting_input` (§2's latch rule).
    pub(super) approval_latched: bool,
    /// Latched `true` by the frontend on-screen recognizer (Phase 3) when an
    /// agent's approval prompt is rendered, cleared when it disappears. A
    /// SECOND, source-independent approval latch alongside the hook-sourced
    /// `approval_latched`: a non-hooked agent (Codex/OpenCode/hand-started) has
    /// no hook to set the first one, so recognition feeds this instead. It owns
    /// its own retraction (the prompt leaving the screen), exactly as the hook
    /// latch owns its `working`/`Stop` retraction — the two never fight because
    /// a given agent uses at most one source. Either being set resolves to
    /// `awaiting_approval` (TERMINAL_ACTIVITY §Phase 3: "screen-sourced
    /// awaiting_approval behaves exactly like the hook-sourced one").
    pub(super) screen_approval: bool,
    /// A `SessionEnd` (`exit`) hook was seen — the top of the precedence order.
    pub(super) exited: bool,
    /// The last state actually emitted for this session, used to dedup
    /// (emit only on real change).
    pub(super) last_emitted: Option<String>,
}

#[derive(Default)]
pub struct SessionState {
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
    /// Per-session resolved-activity records (TERMINAL_ACTIVITY §2). The
    /// single place the two signal sources — the cadence sweep and the hook
    /// scanner — meet: both fold their reading into the session's
    /// `SessionActivity` and let `resolve` apply the §2 precedence latch, so a
    /// scanner `awaiting_approval` is no longer clobbered by the next sweep
    /// tick's `awaiting_input`. Keyed by session id; an entry exists only for a
    /// session that has produced an activity signal (agent-gated), and is
    /// dropped on `exit` (and by the sweep's GC when the session disappears) so
    /// a reused id starts clean.
    pub activity: Mutex<HashMap<String, SessionActivity>>,
}

#[derive(Serialize, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub machine_id: String,
    pub created_at: u64,
    /// User-supplied tab title (`None` until renamed). New optional field
    /// — additive on the wire, ignored by older frontends (spec §2.1).
    pub title: Option<String>,
    /// Friendly machine name (`Machine.name`, e.g. "prod-gpu"), resolved at
    /// session start. `None` on lifecycle events (disconnect/ended/running)
    /// where the frontend already knows the label from the existing tab.
    /// Lets startup-reconcile show a human name instead of the raw id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_name: Option<String>,
    /// Coding-agent kind currently running in the session (e.g.
    /// `"claude-code"`, `"opencode"`), or `None` for a plain shell. Seeded
    /// from the launch command and, for local sessions, kept live by the
    /// foreground-process detector. Additive on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Payload for the additive `terminal-session-activity` event
/// (TERMINAL_ACTIVITY §2). `state` is one of `"working"`,
/// `"awaiting_input"`, `"awaiting_approval"`, or `"exit"`. The Phase 1 cadence
/// sweep emits the first two; the Phase 2 hook scanner (T2.3) additionally
/// emits `awaiting_approval` and `exit` from a hooked session's reporter
/// sequences. Kept a distinct struct from `SessionInfo` so the activity wire
/// shape stays minimal
/// (`{ session_id, state }`) and independent of the session-lifecycle
/// envelope. serde serialises the field names as-is.
#[derive(Serialize, Clone)]
pub struct ActivityInfo {
    pub session_id: String,
    pub state: String,
}

/// What `start_terminal_session` hands back to the frontend. For a hooked
/// agent kind (Claude) `launch_command` carries the backend-augmented launch
/// line (base command + injected `--settings` reporter hooks); the frontend
/// writes THAT instead of its own base command — the single-write contract
/// that prevents Claude launching twice (TERMINAL_ACTIVITY §2c). `None`
/// for plain shells and non-hooked agents (the frontend writes its own line,
/// or nothing). serde serialises the fields snake_case (`session_id`,
/// `launch_command`).
#[derive(Serialize, Clone)]
pub struct StartedSession {
    pub session_id: String,
    pub launch_command: Option<String>,
}

pub(crate) const IDLE_TIMEOUT_SECS: u64 = 600;

/// The four activity states that ride the `terminal-session-activity` wire and
/// the OSC `state=` field. Centralised so the reporter-hook builder
/// (`build_claude_activity_settings`) and the resolver (`apply_hook` /
/// `apply_cadence` / `cadence_state` / `resolve`) share ONE source of truth: a
/// rename in one site becomes a compile error instead of a silent no-op in
/// `apply_hook`'s `_ =>` arm. Kept as `&str` consts (not an enum) so the wire
/// format and serde serialisation stay byte-for-byte unchanged, and so they can
/// be used both as values and as `match` patterns.
pub(crate) mod activity_state {
    pub const WORKING: &str = "working";
    pub const AWAITING_INPUT: &str = "awaiting_input";
    pub const AWAITING_APPROVAL: &str = "awaiting_approval";
    pub const EXIT: &str = "exit";
}

/// Stamp a session's shared last-output instant to "now". Called by both
/// drain transports on every chunk so the activity sweep has one source of
/// truth for cadence. A poisoned lock is swallowed — a missed stamp only
/// makes the sweep briefly read the session as quieter than it is, never a
/// crash on the hot output path.
pub(crate) fn touch_last_output(last_output_at: &Arc<Mutex<Instant>>) {
    if let Ok(mut slot) = last_output_at.lock() {
        *slot = Instant::now();
    }
}

/// How long since a session last produced output. A poisoned lock reads as
/// zero elapsed (treated as recently-active) so a transient poisoning never
/// spuriously flips a session to `awaiting_input`.
pub(crate) fn elapsed_since_last_output(last_output_at: &Arc<Mutex<Instant>>) -> Duration {
    last_output_at
        .lock()
        .map(|slot| slot.elapsed())
        .unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_real_viewport() {
        assert_eq!(checked_pty_size(80, 24), Some((80, 24)));
    }

    #[test]
    fn rejects_a_boxless_measurement() {
        assert_eq!(checked_pty_size(11, 5), None);
    }

    #[test]
    fn rejects_a_zero_geometry() {
        assert_eq!(checked_pty_size(0, 0), None);
    }

    #[test]
    fn rejects_a_geometry_that_would_wrap_the_u16_narrowing() {
        assert_eq!(checked_pty_size(100_000, 40), None);
    }
}
