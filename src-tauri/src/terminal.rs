use crate::domain::models::Machine;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use ssh2::Session;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, Runtime, State};

// Phase-2 drain OSC scanner (TERMINAL_ACTIVITY_PLAN §2b). Wired into
// `drain_local` / `drain_ssh` (T2.3): a hooked session (one carrying an
// `activity_nonce`) runs every chunk through an `ActivityScanner` before
// broadcast, stripping our private OSC and emitting `terminal-session-activity`
// for each parsed state.
pub(crate) mod activity_scanner;

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(1);

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
    /// (TERMINAL_ACTIVITY_PLAN §2b — anti-spoof + cross-session TTY-bleed
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
const SCROLLBACK_MAX_BYTES: usize = 256 * 1024;

/// Fallback PTY dimensions used when the frontend does not supply a size at
/// session start (e.g. reconnect, or a caller that has not measured its
/// surface yet). Kept at the classic 80x24 — crucially *narrower* than any
/// realistic terminal viewport so the shell's first prompt never wraps wider
/// than the visible area (a wider default made Powerlevel10k's full-width
/// frame wrap and the command line appear duplicated). The frontend sends the
/// real size via `resize_terminal_session` right after it mounts and fits.
const DEFAULT_TERM_COLS: u16 = 80;
const DEFAULT_TERM_ROWS: u16 = 24;

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
    fn push_scrollback(&mut self, chunk: &[u8]) {
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
    fn snapshot_scrollback(&self) -> Vec<u8> {
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
type SessionHandles = (
    ReadSource,
    WriteSink,
    Arc<Mutex<SessionKeepalive>>,
    Option<u32>,
    Option<String>,
);

#[derive(Default)]
pub struct SessionState {
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
    /// Per-session resolved-activity records (TERMINAL_ACTIVITY_PLAN §2). The
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

/// The per-session activity record both signal sources feed
/// (TERMINAL_ACTIVITY_PLAN §2 precedence latch). Fields are folded in by
/// `apply_cadence` (the sweep) and `apply_hook` (the scanner); the resolved
/// state is computed by `resolve` and emitted (deduped) by `resolve_and_emit`.
#[derive(Default)]
pub struct SessionActivity {
    /// Last cadence read by the sweep: `Some("working")` or
    /// `Some("awaiting_input")`, `None` until the first tick. The precedence
    /// *floor* — only consulted for a session that has NOT reported an explicit
    /// hook state (`hook` is `None`).
    cadence: Option<&'static str>,
    /// Last explicit working/awaiting_input reported by a hook, or `None` until
    /// the first one arrives. Once a session's hooks speak, the hook is
    /// **authoritative** over the cadence floor for the working ↔ awaiting_input
    /// axis: a TUI agent like Claude Code repaints continuously (blinking
    /// cursor, footer, the reporter OSC's own bytes), so the byte cadence never
    /// falls quiet and would otherwise pin the session to `working` forever
    /// even after `Stop` fired `awaiting_input`. Latching to the hook lets the
    /// cheap universal floor and the precise hook layer coexist without the
    /// floor clobbering the hook (TERMINAL_ACTIVITY_PLAN §2).
    hook: Option<&'static str>,
    /// Latched `true` by a scanner `awaiting_approval`; cleared by any
    /// non-approval explicit hook (working / awaiting_input / exit). While set
    /// it survives the cadence floor's `awaiting_input` (§2's latch rule).
    approval_latched: bool,
    /// Latched `true` by the frontend on-screen recognizer (Phase 3) when an
    /// agent's approval prompt is rendered, cleared when it disappears. A
    /// SECOND, source-independent approval latch alongside the hook's
    /// `approval_latched`: a non-hooked agent (Codex/OpenCode/hand-started) has
    /// no hook to set the first one, so recognition feeds this instead. It owns
    /// its own retraction (the prompt leaving the screen), exactly as the hook
    /// latch owns its `working`/`Stop` retraction — the two never fight because
    /// a given agent uses at most one source. Either being set resolves to
    /// `awaiting_approval` (TERMINAL_ACTIVITY_PLAN §Phase 3: "screen-sourced
    /// awaiting_approval behaves exactly like the hook-sourced one").
    screen_approval: bool,
    /// A `SessionEnd` (`exit`) hook was seen — the top of the precedence order.
    exited: bool,
    /// The last state actually emitted for this session, used to dedup
    /// (emit only on real change).
    last_emitted: Option<String>,
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
/// (TERMINAL_ACTIVITY_PLAN §2). `state` is one of `"working"`,
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
/// that prevents Claude launching twice (TERMINAL_ACTIVITY_PLAN §2c). `None`
/// for plain shells and non-hooked agents (the frontend writes its own line,
/// or nothing). serde serialises the fields snake_case (`session_id`,
/// `launch_command`).
#[derive(Serialize, Clone)]
pub struct StartedSession {
    pub session_id: String,
    pub launch_command: Option<String>,
}

const IDLE_TIMEOUT_SECS: u64 = 600;

/// The four activity states that ride the `terminal-session-activity` wire and
/// the OSC `state=` field. Centralised so the reporter-hook builder
/// (`build_claude_activity_settings`) and the resolver (`apply_hook` /
/// `apply_cadence` / `cadence_state` / `resolve`) share ONE source of truth: a
/// rename in one site becomes a compile error instead of a silent no-op in
/// `apply_hook`'s `_ =>` arm. Kept as `&str` consts (not an enum) so the wire
/// format and serde serialisation stay byte-for-byte unchanged, and so they can
/// be used both as values and as `match` patterns.
mod activity_state {
    pub const WORKING: &str = "working";
    pub const AWAITING_INPUT: &str = "awaiting_input";
    pub const AWAITING_APPROVAL: &str = "awaiting_approval";
    pub const EXIT: &str = "exit";
}

/// Whether an agent kind self-reports activity via injected hooks (Phase 2).
/// Only Claude for now; extended as more agents grow a hook transport. This is
/// a pure agent-CAPABILITY predicate and is deliberately OS-agnostic: whether
/// the hook TRANSPORT can actually be used for a given session (it is
/// POSIX-only) is a separate, transport-scoped decision made by
/// [`hook_transport_supported`]. Keeping the two apart is what lets a Windows
/// client keep its always-POSIX SSH sessions hooked while degrading only its
/// cmd.exe LOCAL sessions to unhooked.
fn is_hooked_agent_kind(kind: &str) -> bool {
    kind == "claude-code"
}

/// Whether the injected-hook activity transport can be used for a session,
/// given whether that session is LOCAL (`is_local == true`) or SSH
/// (`is_local == false`). The transport is POSIX-only: `activity_reporter_command`
/// emits `printf '%s' '<json>'` (no `printf`, single-quotes misbehave under
/// cmd.exe) and `build_agent_launch_command` appends `--settings <path>` quoted
/// with the POSIX `shell_single_quote` (wrong for cmd.exe, and %USERPROFILE%
/// temp paths often contain spaces).
///
/// So on a Windows client a LOCAL agent (which runs under cmd.exe) degrades to
/// UNHOOKED: `activity_nonce` stays `None`, the whole `--settings` launch
/// override is bypassed, `start_terminal_session` returns `launch_command:
/// None`, and the frontend writes the plain base command (bare `claude`, which
/// cmd.exe resolves to `claude.cmd` via PATHEXT). Activity is then best-effort
/// via the on-screen OSC scanner (`activity_scanner.rs`), the existing fallback
/// for non-hooked agents.
///
/// The SSH path is deliberately UNAFFECTED: an SSH session always targets a
/// POSIX remote shell (`shell_escape_posix`, `cd … && clear`), so a hooked
/// remote agent keeps self-reporting regardless of the client OS. The
/// Windows/POSIX split is therefore keyed on the TARGET shell (local vs.
/// remote), never on the client's compile-time OS alone — which would wrongly
/// disable remote hooks for Windows clients (critic C2).
fn hook_transport_supported(is_local: bool) -> bool {
    if cfg!(target_os = "windows") {
        // Windows LOCAL sessions run under cmd.exe → the POSIX transport is
        // invalid there. SSH sessions (`!is_local`) stay hooked: their remote
        // shell is always POSIX.
        !is_local
    } else {
        true
    }
}

/// Mint an unguessable per-session activity nonce (hex). Sourced from the OS
/// CSPRNG so a repo script cannot predict it and spoof `awaiting_approval`
/// (which would become notification spam once Phase 2e fires OS
/// notifications). A CSPRNG failure — astronomically unlikely on supported
/// platforms — degrades to a time-derived seed so the session still launches;
/// the nonce only gates which sequences are trusted, never correctness.
fn generate_activity_nonce() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes = nanos.to_le_bytes();
    }
    use std::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// POSIX single-quote a value for safe interpolation into a shell command:
/// wrap in `'…'` and replace every embedded `'` with `'\''` (close-quote,
/// escaped-quote, re-open-quote). The canonical way to pass an arbitrary
/// string (here the serialised `--settings` JSON, which itself contains single
/// quotes from the reporter commands) through the shell verbatim.
fn shell_single_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    out.push_str(&value.replace('\'', "'\\''"));
    out.push('\'');
    out
}

/// Defensively quote a value for interpolation into a **cmd.exe** command
/// line: wrap it in double quotes and prefix every cmd metacharacter (`"`, `%`,
/// `^`, `&`, `|`, `<`, `>`) with a `^` so a malformed branch name cannot break
/// out of the argument and inject a second command. The Windows sibling of
/// [`shell_single_quote`].
///
/// cmd.exe's quoting is famously weak: double quotes stop word-splitting but a
/// stray `"` still closes the string early, after which `& | < >` regain their
/// command-boundary meaning. The `^` prefix defends the string against exactly
/// that break-out — for any metacharacter that lands *outside* the quotes (an
/// injected `"` having closed them) the caret neutralises it; for one that
/// stays *inside* the quotes the caret is a harmless literal (cmd does not
/// treat `^` as an escape inside quotes) and the quotes already neutralise the
/// char. Either way the string cannot start a second command. The cost is that
/// a branch name genuinely containing one of these metacharacters gets a
/// literal `^` in it and fails `git checkout` — a tolerated, safe-by-default
/// outcome for the generated branch ids this actually runs on. Note `%VAR%`
/// still expands inside cmd double quotes; realistic ASCII branch ids contain
/// no `%`, so this is defence-in-depth rather than a complete `%` guard.
///
/// Compiled on all platforms (only *called* on Windows) so its
/// command-injection guard keeps executed unit-test coverage on the POSIX CI
/// leg, mirroring [`shell_single_quote`]'s cross-platform test.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn cmd_double_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if matches!(ch, '"' | '%' | '^' | '&' | '|' | '<' | '>') {
            out.push('^');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// The reporter command injected under one hook: a shell command that prints
/// the `terminalSequence` hook-JSON to stdout, which Claude then writes to the
/// PTY verbatim (transport verified in `docs/spikes/terminal-activity-
/// transport.md`). `` / `` are LITERAL 6-char JSON escapes in the
/// printed payload — Claude's JSON parse of the hook stdout turns them into
/// ESC / BEL as it emits the sequence, so the drain scanner sees
/// `ESC ]777;demeteo;…BEL`. `printf '%s'` over a single-quoted literal keeps
/// the shell from interpreting the backslashes.
fn activity_reporter_command(nonce: &str, state: &str) -> String {
    let payload = format!(
        "{{\"terminalSequence\":\"\\u001b]777;demeteo;v=1;nonce={nonce};state={state}\\u0007\"}}"
    );
    format!("printf '%s' {}", shell_single_quote(&payload))
}

/// Build the compact `--settings` JSON injecting Claude's activity reporter
/// hooks for `nonce` (TERMINAL_ACTIVITY_PLAN §2c/§2d). Returns `None` for any
/// non-Claude kind. serde_json guarantees correct escaping of the nested
/// quotes/backslashes in each reporter command.
///
/// The event→state map (§2d): `UserPromptSubmit`/`PreToolUse`/`PostToolUse` →
/// `working`; `Notification` `permission_prompt` → `awaiting_approval` and
/// `idle_prompt` → `awaiting_input`; `Stop` → `awaiting_input`; `SessionEnd` →
/// `exit`. `--settings` deep-merges `hooks` but replaces the array under each
/// event key, so these ephemerally replace the user's own hooks on those
/// events for this session only (never touches the user's files).
fn build_claude_activity_settings(kind: &str, nonce: &str) -> Option<String> {
    if kind != "claude-code" {
        return None;
    }
    // (event, optional matcher, reported state) — §2d. Claude honors `matcher`
    // for `Notification` (filtering by notification type, e.g.
    // `permission_prompt` / `idle_prompt`), so the two Notification groups do
    // NOT both fire on the same notification.
    let specs: [(&str, Option<&str>, &str); 7] = [
        ("UserPromptSubmit", None, activity_state::WORKING),
        ("PreToolUse", None, activity_state::WORKING),
        ("PostToolUse", None, activity_state::WORKING),
        (
            "Notification",
            Some("permission_prompt"),
            activity_state::AWAITING_APPROVAL,
        ),
        (
            "Notification",
            Some("idle_prompt"),
            activity_state::AWAITING_INPUT,
        ),
        ("Stop", None, activity_state::AWAITING_INPUT),
        ("SessionEnd", None, activity_state::EXIT),
    ];
    let mut hooks: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (event, matcher, state) in specs {
        let mut group = serde_json::Map::new();
        if let Some(m) = matcher {
            group.insert(
                "matcher".to_string(),
                serde_json::Value::String(m.to_string()),
            );
        }
        group.insert(
            "hooks".to_string(),
            serde_json::json!([{
                "type": "command",
                "command": activity_reporter_command(nonce, state),
            }]),
        );
        match hooks.entry(event.to_string()) {
            serde_json::map::Entry::Vacant(e) => {
                e.insert(serde_json::Value::Array(vec![serde_json::Value::Object(
                    group,
                )]));
            }
            serde_json::map::Entry::Occupied(mut e) => {
                if let serde_json::Value::Array(arr) = e.get_mut() {
                    arr.push(serde_json::Value::Object(group));
                }
            }
        }
    }
    let settings = serde_json::json!({ "hooks": serde_json::Value::Object(hooks) });
    serde_json::to_string(&settings).ok()
}

/// Ephemeral settings-file path for a session's reporter hooks: the OS temp dir
/// plus a nonce-keyed name. The nonce is random per launch, so two concurrent
/// hooked sessions never collide, and it is already exposed in the process argv
/// — the file adds no new secret.
fn activity_settings_file_path(nonce: &str) -> PathBuf {
    std::env::temp_dir().join(format!("demeteo-claude-activity-{nonce}.json"))
}

/// Write the reporter-hooks JSON to the session's ephemeral settings file and
/// return its path. The file is a per-session throwaway (removed on session
/// teardown, see `ActiveSession`'s `Drop`) — it is NOT the user's
/// `~/.claude/settings.json` nor the project `.claude/settings*.json`, so
/// demeteo never mutates the user's or the project's own agent settings.
fn write_activity_settings_file(nonce: &str, settings_json: &str) -> std::io::Result<PathBuf> {
    let path = activity_settings_file_path(nonce);
    std::fs::write(&path, settings_json)?;
    Ok(path)
}

/// The remote reporter-hooks settings path for a hooked SSH launch (T4.1). A
/// local temp path is meaningless on the far host, so remote hooked sessions
/// place the file here — a fixed, nonce-keyed name under the remote `/tmp` (the
/// one directory a POSIX SSH target reliably lets us write). Left behind on
/// teardown (unlike the local file, which `ActiveSession`'s `Drop` removes):
/// a stale nonce-named JSON in the remote `/tmp` is tiny and harmless, and we
/// hold no live channel to clean it once the session's gone.
fn remote_activity_settings_path(nonce: &str) -> String {
    format!("/tmp/demeteo-claude-activity-{nonce}.json")
}

/// SFTP the reporter-hooks JSON onto the remote host at `remote_path` so a
/// remote-launched Claude can read `--settings <remote_path>` (T4.1). Must run
/// while `sess` is still blocking (before `set_blocking(false)`) and before the
/// drain thread starts, so this synchronous write never races the interactive
/// read over the shared libssh2 session. A failure (no SFTP subsystem,
/// unwritable `/tmp`) is surfaced to the caller, which then degrades the launch
/// to unhooked rather than pointing Claude at a missing file.
fn write_remote_settings_file(
    sess: &Session,
    remote_path: &str,
    settings_json: &str,
) -> Result<(), String> {
    let sftp = sess.sftp().map_err(|e| format!("open SFTP: {e}"))?;
    let mut file = sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("create {remote_path}: {e}"))?;
    file.write_all(settings_json.as_bytes())
        .map_err(|e| format!("write {remote_path}: {e}"))?;
    Ok(())
}

/// Build the launch line for a hooked agent: the base command the frontend
/// would have written, with `--settings <path>` appended (preserving any user
/// args already in `base_command`). We point Claude at a FILE rather than
/// inline JSON so the command line stays short — the full inline
/// `--settings '<json>'` is ~1.4 KB and a PTY's canonical-mode input line caps
/// at 1024 bytes (`MAX_CANON`), which truncated the launch mid-quote so it
/// never ran. Returns `None` for a non-hooked kind — the frontend then writes
/// its own base command (or nothing). Backend half of the single-write
/// contract (§2c).
fn build_agent_launch_command(base_command: &str, settings_path: &Path) -> String {
    format!(
        "{base_command} --settings {}",
        shell_single_quote(&settings_path.to_string_lossy())
    )
}

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

pub fn connect_ssh(
    machine: &Machine,
    secret: Option<String>,
) -> Result<(Session, TcpStream), String> {
    crate::ssh_util::connect(machine, secret)
}

/// Starts a terminal session on the given machine.
///
/// `agent_kind` is the coding-agent kind the frontend is launching into this
/// fresh session (e.g. `"claude-code"`), or `None` for a plain shell. It
/// seeds the session's agent label immediately so the tab shows the badge
/// before the foreground detector has run its first pass.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn start_terminal_session(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    session_state: State<'_, SessionState>,
    machine_id: String,
    work_dir: Option<String>,
    work_branch: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    agent_kind: Option<String>,
    launch_command: Option<String>,
) -> Result<StartedSession, String> {
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )?;
    let machine_name = machine.name.clone();
    // Resolve the initial PTY size so the shell draws its very first prompt at
    // (near) the real terminal width. `0` is treated as "unset" defensively —
    // a zero-column PTY would make prompts render incoherently.
    let cols = cols.filter(|c| *c > 0).unwrap_or(DEFAULT_TERM_COLS);
    let rows = rows.filter(|r| *r > 0).unwrap_or(DEFAULT_TERM_ROWS);

    let session_id = format!("sess_{}", SESSION_COUNTER.fetch_add(1, Ordering::SeqCst));
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Seed an empty `Broadcast`: no subscriber and no permanent seed
    // channel. Output produced before the surface attaches (shell
    // startup, the `git checkout` bootstrap, the first prompt) flows
    // into the scrollback ring and is replayed on the first
    // `attach_terminal_session`, so nothing is lost and nothing races
    // the attach (TERMINALS_VIEW_SPEC §3).
    let frontend_channel: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
    let display_title: Mutex<Option<String>> = Mutex::new(None);
    let connected = Arc::new(AtomicBool::new(true));
    // Seed the last-output instant to "now" so the session reads as
    // recently-active until its first quiet window elapses.
    let last_output_at: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));

    // Normalise an empty agent kind to `None` so a plain shell never carries
    // a phantom badge.
    let agent_kind = agent_kind.filter(|k| !k.trim().is_empty());

    let is_local = machine.auth_type == "local";
    // For a hooked agent kind (Claude) whose transport is usable on this
    // session, mint a per-session nonce so the drain scanner accepts only THIS
    // launch's activity sequences, and build the reporter-hooks JSON. Both are
    // computed BEFORE the transport starts so the remote start path can SFTP the
    // JSON onto the far host while its session is still blocking and before the
    // drain thread reads (T4.1). `hook_transport_supported(is_local)` degrades a
    // Windows LOCAL (cmd.exe) session to unhooked while leaving the always-POSIX
    // SSH path hooked regardless of client OS — see that fn's doc.
    let activity_nonce = agent_kind
        .as_deref()
        .filter(|k| is_hooked_agent_kind(k) && hook_transport_supported(is_local))
        .map(|_| generate_activity_nonce());
    // Reporter-hooks JSON, transport-agnostic. `Some` only for a hooked launch
    // that also carries a base command to augment (§2c/§2d). Never the user's
    // `~/.claude` nor the project `.claude/` settings — always a throwaway.
    let activity_settings_json = match (
        agent_kind.as_deref(),
        activity_nonce.as_deref(),
        launch_command.as_deref(),
    ) {
        (Some(kind), Some(nonce), Some(_base)) => build_claude_activity_settings(kind, nonce),
        _ => None,
    };

    let (read_source, write_sink, keepalive, child_pid, remote_settings_path) = if is_local {
        start_local_pty(&machine_id, &work_dir, &work_branch, cols, rows)?
    } else {
        // Hand the remote start path the (remote path, JSON) so it places the
        // settings file over SFTP and reports the path back for the launch line.
        let remote_settings = match (activity_settings_json.as_deref(), activity_nonce.as_deref()) {
            (Some(json), Some(nonce)) => Some((remote_activity_settings_path(nonce), json)),
            _ => None,
        };
        start_ssh_session(
            &machine,
            &work_dir,
            &work_branch,
            cols,
            rows,
            remote_settings,
        )?
    };

    // Backend half of the single-write contract (§2c): `launch_override` `Some`
    // ⇒ the frontend writes this augmented `claude --settings <path>` instead of
    // its own base command. Local writes an ephemeral file to the OS temp dir
    // (`--settings <file>`, not ~1.4 KB of inline JSON, which overran the PTY's
    // 1024-byte canonical input limit); remote already SFTP'd the file above and
    // just references the returned remote path. Either way a placement failure
    // degrades to an unhooked launch (frontend writes plain `base`) rather than
    // breaking the launch — the nonce only gates precise activity, not
    // correctness.
    let (launch_override, activity_settings_path) = match (
        activity_settings_json.as_deref(),
        activity_nonce.as_deref(),
        launch_command.as_deref(),
    ) {
        (Some(json), Some(nonce), Some(base)) => {
            if is_local {
                match write_activity_settings_file(nonce, json) {
                    Ok(path) => (Some(build_agent_launch_command(base, &path)), Some(path)),
                    Err(e) => {
                        eprintln!(
                            "[terminal] activity settings file write failed: {e}; launching unhooked"
                        );
                        (None, None)
                    }
                }
            } else {
                // The remote file lives on the far host, so it is NOT stored in
                // `activity_settings_path` (whose `Drop` does a LOCAL
                // `remove_file`); it is left in the remote `/tmp` (harmless).
                match remote_settings_path {
                    Some(remote_path) => (
                        Some(build_agent_launch_command(base, Path::new(&remote_path))),
                        None,
                    ),
                    None => {
                        eprintln!(
                            "[terminal] remote activity settings write failed; launching unhooked"
                        );
                        (None, None)
                    }
                }
            }
        }
        _ => (None, None),
    };

    spawn_drain(
        &read_source,
        app.clone(),
        session_id.clone(),
        machine_id.clone(),
        created_at,
        frontend_channel.clone(),
        connected.clone(),
        last_output_at.clone(),
        activity_nonce.clone(),
    );

    let mut sessions = session_state
        .sessions
        .lock()
        .map_err(|_| "Failed to lock sessions".to_string())?;
    sessions.insert(
        session_id.clone(),
        ActiveSession {
            read_source,
            write_sink,
            _keepalive: keepalive,
            machine_id: machine_id.clone(),
            machine_name: machine_name.clone(),
            created_at,
            child_pid,
            agent: Mutex::new(agent_kind.clone()),
            activity_nonce,
            activity_settings_path,
            last_output_at,
            frontend_channel,
            display_title,
            work_dir,
            work_branch,
            connected,
        },
    );

    let _ = app.emit(
        "terminal-session-started",
        SessionInfo {
            session_id: session_id.clone(),
            machine_id,
            created_at,
            title: None,
            machine_name: Some(machine_name),
            agent: agent_kind,
        },
    );

    Ok(StartedSession {
        session_id,
        launch_command: launch_override,
    })
}

/// Pick the shell to spawn for a local PTY. Split by platform because the
/// env var that names the interactive shell differs: POSIX exports `SHELL`,
/// Windows has no such variable and instead names the command processor via
/// `COMSPEC`. Spawning `/bin/bash` on Windows makes ConPTY/`portable-pty`
/// return Err and the session dies with "Failed to spawn shell", so the
/// Windows arm falls back to `cmd.exe` — the one interpreter guaranteed to
/// exist there.
#[cfg(target_os = "windows")]
fn select_local_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}

/// POSIX: honour the user's `$SHELL`, falling back to `/bin/bash`.
#[cfg(not(target_os = "windows"))]
fn select_local_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

pub(crate) fn start_local_pty(
    machine_id: &str,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionHandles, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to open PTY: {}", e))?;

    let shell = select_local_shell();
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    // Ensure a UTF-8 locale on macOS. A GUI launch (Finder/Dock) can hand the
    // app an environment with no `LANG`/`LC_*`, dropping the shell into the C
    // locale where it mishandles multibyte prompt output. `en_US.UTF-8` and the
    // bare `UTF-8` `LC_CTYPE` are Darwin idioms that are always valid there.
    //
    // Deliberately macOS-only: on Linux/BSD these exact values are risky —
    // `en_US.UTF-8` is frequently not generated and `LC_CTYPE=UTF-8` is not a
    // valid locale name, so forcing them yields `setlocale` warnings and a C
    // fallback. Desktop launchers on those platforms already propagate the
    // session locale, so no override is needed. Only set when absent so a
    // user's real locale is never overridden.
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        if std::env::var_os("LC_CTYPE").is_none() {
            cmd.env("LC_CTYPE", "UTF-8");
        }
    }
    if let Some(dir) = work_dir {
        cmd.cwd(dir);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;
    // Capture the shell pid before `child` is moved into the keepalive — the
    // foreground-agent detector walks the process tree rooted here.
    let child_pid = child.process_id();

    // Close the slave end in the parent — the child inherited it.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;
    // take_writer can only be called once — do it before moving master into keepalive.
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to take PTY writer: {}", e))?;

    // Bootstrap the branch if requested. The shell may not be ready to
    // accept input instantly, but stdin pipes buffer the bytes; bash will
    // process them on startup. A non-existent branch is swallowed: the
    // PTY remains usable on whatever branch the repo is currently on.
    if let Some(bootstrap) = branch_bootstrap_line(work_branch) {
        let _ = writer.write_all(bootstrap.as_bytes());
        let _ = writer.flush();
    }

    let read_source = ReadSource::LocalPty(Arc::new(Mutex::new(reader)));
    let write_sink = WriteSink::LocalPty(Arc::new(Mutex::new(writer)));
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::LocalPty {
        master: pair.master,
        child,
    }));

    let _ = machine_id; // suppress unused warning
                        // A local session never places a remote settings file (T4.1).
    Ok((read_source, write_sink, keepalive, child_pid, None))
}

/// Build the POSIX-shell bootstrap line that performs a `git checkout` of the
/// supplied feature branch on shell startup. Returns `None` when no branch was
/// supplied, so callers can skip the write entirely for `ProjectHome`-style
/// flows (no pipeline context).
///
/// The line is shell-escaped defensively — the feature id is generated, but
/// a branch containing a stray quote or `;` would otherwise become a
/// command-injection vector. The trailing `clear` mirrors the existing
/// `cd … && clear` behaviour in the SSH path so the prompt lands cleanly.
/// Missing-branch failures are intentionally tolerated (`|| true`-style
/// fallback) so a not-yet-started feature still opens a usable terminal.
///
/// Always compiled — the syntax targets a POSIX shell, which is the correct
/// choice for **every remote SSH host regardless of client OS** (`start_ssh_
/// session` calls this directly) and for local sessions on non-Windows hosts
/// (via [`branch_bootstrap_line`]). The Windows/cmd.exe split lives in
/// [`branch_bootstrap_line`] and applies only to the *local* PTY, never to the
/// remote shell.
fn branch_bootstrap_line_posix(branch: &Option<String>) -> Option<String> {
    let raw = branch.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = crate::paths::shell_escape_posix(raw);
    Some(format!(
        "git checkout {safe} 2>/dev/null || git switch {safe} 2>/dev/null; clear\n"
    ))
}

/// Build the bootstrap line for the **local** PTY, choosing shell syntax by the
/// compile-time host OS: the POSIX form on non-Windows, the cmd.exe form on
/// Windows. This selector is used ONLY for the local shell — the SSH/remote
/// path deliberately bypasses it and always calls
/// [`branch_bootstrap_line_posix`] because the remote host is unconditionally a
/// POSIX shell. Both arms share the same `None`-on-absent/blank contract and
/// the same checkout-then-switch tolerance.
#[cfg(not(target_os = "windows"))]
fn branch_bootstrap_line(branch: &Option<String>) -> Option<String> {
    branch_bootstrap_line_posix(branch)
}

/// cmd.exe variant of [`branch_bootstrap_line`]. Emits `2>nul` (cmd's null
/// sink), `||`/`&` command chaining, and `cls` (cmd's screen clear), with a
/// CRLF terminator so cmd.exe treats the buffered bytes as one finished
/// command line. The branch is quoted with [`cmd_double_quote`] rather than
/// POSIX single quotes. Used only for the local Windows PTY — never for the
/// SSH path (see [`branch_bootstrap_line_posix`]).
#[cfg(target_os = "windows")]
fn branch_bootstrap_line(branch: &Option<String>) -> Option<String> {
    let raw = branch.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = cmd_double_quote(raw);
    Some(format!(
        "git checkout {safe} 2>nul || git switch {safe} 2>nul & cls\r\n"
    ))
}

fn start_ssh_session(
    machine: &Machine,
    work_dir: &Option<String>,
    work_branch: &Option<String>,
    cols: u16,
    rows: u16,
    remote_settings: Option<(String, &str)>,
) -> Result<SessionHandles, String> {
    let secret = match machine.auth_type.as_str() {
        "password" | "key" => {
            let key = format!("machine_{}", machine.id);
            crate::credential_cache::get_or_fetch(&key, || {
                let entry = keyring::Entry::new("demeteo", &key)
                    .map_err(|e| format!("Keyring error: {}", e))?;
                entry
                    .get_password()
                    .map_err(|e| format!("Keyring error: {}", e))
            })
            .ok()
        }
        _ => None,
    };

    let (sess, tcp) = connect_ssh(machine, secret)?;
    sess.set_keepalive(true, 30);
    let mut ssh_chan = sess
        .channel_session()
        .map_err(|e| format!("Failed to open SSH channel: {}", e))?;
    ssh_chan
        .request_pty(
            "xterm-256color",
            None,
            Some((cols as u32, rows as u32, 0, 0)),
        )
        .map_err(|e| format!("Failed to request PTY: {}", e))?;
    ssh_chan
        .shell()
        .map_err(|e| format!("Failed to start shell: {}", e))?;

    // Place the reporter-hooks settings file on the far host over SFTP for a
    // hooked launch (T4.1). Done here — session still blocking, drain not yet
    // spawned — so the synchronous write never races the interactive read.
    // A failure yields `None`, and the caller degrades the launch to unhooked
    // rather than pointing `claude --settings` at a missing file.
    let remote_settings_path = match remote_settings {
        Some((path, json)) => match write_remote_settings_file(&sess, &path, json) {
            Ok(()) => Some(path),
            Err(e) => {
                eprintln!("[terminal] remote activity settings SFTP failed: {e}");
                None
            }
        },
        None => None,
    };

    if let Some(dir) = work_dir {
        let cd_cmd = format!("cd {} && clear\n", crate::paths::shell_escape_posix(dir));
        let _ = ssh_chan.write_all(cd_cmd.as_bytes());
        let _ = ssh_chan.flush();
    }
    // The remote is ALWAYS a POSIX shell, so the bootstrap must use POSIX
    // syntax regardless of the client OS. Calling `branch_bootstrap_line` here
    // would send cmd.exe syntax (`2>nul`, `& cls`) to a remote `bash`/`sh` on a
    // Windows client — creating a stray `nul` file and a `cls: command not
    // found` error in the remote repo. `branch_bootstrap_line_posix` is
    // compile-time-independent of the client OS and keeps the SSH path correct.
    if let Some(bootstrap) = branch_bootstrap_line_posix(work_branch) {
        let _ = ssh_chan.write_all(bootstrap.as_bytes());
        let _ = ssh_chan.flush();
    }

    sess.set_blocking(false);
    let arc_chan = Arc::new(Mutex::new(ssh_chan));
    let read_source = ReadSource::Ssh(arc_chan.clone());
    let write_sink = WriteSink::Ssh(arc_chan);
    let keepalive = Arc::new(Mutex::new(SessionKeepalive::Ssh { session: sess, tcp }));
    // No local pid for a remote session — the shell runs on the far host.
    Ok((
        read_source,
        write_sink,
        keepalive,
        None,
        remote_settings_path,
    ))
}

/// Spawns the appropriate drain thread for a freshly-built transport,
/// forwarding output into the session's `Broadcast`. Shared by
/// `start_terminal_session` and `reconnect_terminal_session` so both wire
/// up the drain identically (TERMINALS_VIEW_SPEC §3.1).
#[allow(clippy::too_many_arguments)]
fn spawn_drain<R: Runtime>(
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
/// (TERMINAL_ACTIVITY_PLAN §6). The drain calls this, then emits one
/// `terminal-session-activity` per returned state.
fn drain_scan_and_forward(
    scanner: &mut activity_scanner::ActivityScanner,
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
    scanner: Option<&mut activity_scanner::ActivityScanner>,
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
    ch: Arc<Mutex<ssh2::Channel>>,
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
    let mut scanner = nonce.map(activity_scanner::ActivityScanner::new);
    let mut buffer = [0u8; 8192];
    loop {
        let result = ch.lock().unwrap().read(&mut buffer);
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
    let mut scanner = nonce.map(activity_scanner::ActivityScanner::new);
    let mut buffer = [0u8; 8192];
    loop {
        let result = reader.lock().unwrap().read(&mut buffer);
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

/// Stamp a session's shared last-output instant to "now". Called by both
/// drain transports on every chunk so the activity sweep has one source of
/// truth for cadence. A poisoned lock is swallowed — a missed stamp only
/// makes the sweep briefly read the session as quieter than it is, never a
/// crash on the hot output path.
fn touch_last_output(last_output_at: &Arc<Mutex<Instant>>) {
    if let Ok(mut slot) = last_output_at.lock() {
        *slot = Instant::now();
    }
}

/// How long since a session last produced output. A poisoned lock reads as
/// zero elapsed (treated as recently-active) so a transient poisoning never
/// spuriously flips a session to `awaiting_input`.
fn elapsed_since_last_output(last_output_at: &Arc<Mutex<Instant>>) -> Duration {
    last_output_at
        .lock()
        .map(|slot| slot.elapsed())
        .unwrap_or(Duration::ZERO)
}

/// The transport (PTY/SSH child) dropped unexpectedly. Mark the session
/// disconnected but KEEP it in the map — its `Broadcast` (scrollback +
/// title) survives so `reconnect_terminal_session` can rebuild the
/// transport in place (TERMINALS_VIEW_SPEC §3.1). Distinct from
/// `emit_ended`, which fires only on an explicit close that removes the
/// session.
fn emit_disconnected<R: Runtime>(
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
fn emit_ended<R: Runtime>(app: &AppHandle<R>, session_id: &str, machine_id: &str, created_at: u64) {
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
    let snapshot: Vec<Channel<Vec<u8>>> = {
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

// ---------------------------------------------------------------------------
// Foreground coding-agent detection
// ---------------------------------------------------------------------------
//
// A background poller walks the local process tree every few seconds and
// reports whether a known coding-agent CLI (Claude, OpenCode, …) is running
// under each *local* session's shell. When a session's detected agent
// changes it emits `terminal-session-agent { session_id, agent }`, so a tab
// shows an accurate "running Claude" badge even when the user launched the
// agent by hand (typed `claude` in an already-open shell) rather than via
// the New menu.
//
// SSH sessions are deliberately skipped: their shell runs on the far host,
// out of reach of a local `ps`, and probing over the shared libssh2 session
// concurrently with the interactive drain thread is unsafe. Remote tabs keep
// the agent label seeded from the launch command instead.

/// Time between foreground-agent detection passes. A few seconds keeps the
/// badge feeling live without spawning `ps` in a tight loop.
const AGENT_DETECT_INTERVAL: Duration = Duration::from_secs(3);

/// Maps a coding-agent CLI binary name to the "kind" the frontend labels.
/// Mirrors the frontend `AGENT_CLI` table (NewTerminalMenu) so the launch
/// path and the detector agree on the same identifiers.
fn agent_kind_for_binary(binary: &str) -> Option<&'static str> {
    match binary {
        "claude" => Some("claude-code"),
        "opencode" => Some("opencode"),
        "hermes" => Some("hermes"),
        "codex" => Some("codex"),
        _ => None,
    }
}

/// Scan a full command line for a known agent CLI. Matches on whole-token
/// basenames (with a script/executable suffix stripped) rather than raw
/// substrings, so a path that merely *contains* an agent name doesn't
/// false-positive. Catches native launchers (`claude`, `opencode`, `codex`)
/// directly; node-script installs that appear as `node …/claude` match on
/// the `claude` token. On Windows the agents are installed as `claude.cmd` /
/// `claude.exe` / `codex.exe`, so `.cmd`/`.exe`/`.bat` are stripped too (all
/// suffixes case-insensitively) — keeping `agent_kind_for_binary` and the
/// frontend `AGENTS` table keyed on the bare names.
fn detect_agent_in_command(command: &str) -> Option<&'static str> {
    /// Basename suffixes stripped before matching, longest first so `.mjs`
    /// wins over a hypothetical shorter overlap.
    const SUFFIXES: [&str; 5] = [".mjs", ".js", ".cmd", ".exe", ".bat"];
    for token in command.split_whitespace() {
        // Split on both separators — Windows command lines use backslashes.
        let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
        let lower = base.to_ascii_lowercase();
        let base = SUFFIXES
            .iter()
            .find_map(|suf| {
                lower
                    .strip_suffix(suf)
                    .map(|_| &base[..base.len() - suf.len()])
            })
            .unwrap_or(base);
        if let Some(kind) = agent_kind_for_binary(base) {
            return Some(kind);
        }
    }
    None
}

/// A one-shot snapshot of the local process table: each pid's parent and
/// the agent (if any) its command line names, plus a ppid→children index
/// for walking a session's subtree.
struct ProcessTree {
    agent_by_pid: HashMap<u32, Option<&'static str>>,
    children: HashMap<u32, Vec<u32>>,
}

impl ProcessTree {
    /// Capture the process table via `ps`. Returns `None` if `ps` is
    /// unavailable or fails (e.g. a locked-down sandbox) — the detector
    /// then simply skips the pass.
    #[cfg(not(target_os = "windows"))]
    fn capture() -> Option<ProcessTree> {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,command="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut agent_by_pid = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let pid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let ppid = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let command = parts.collect::<Vec<_>>().join(" ");
            agent_by_pid.insert(pid, detect_agent_in_command(&command));
            children.entry(ppid).or_default().push(pid);
        }
        Some(ProcessTree {
            agent_by_pid,
            children,
        })
    }

    /// Windows has no `ps`. Snapshot the same pid/ppid/command-line triples
    /// via `Get-CimInstance Win32_Process` (preferred over the deprecated
    /// `wmic`), emitting one tab-separated row per process so a command line
    /// with embedded spaces stays in a single field. `CREATE_NO_WINDOW` keeps
    /// the 3-second poll from flashing a console window. Any failure — missing
    /// PowerShell, non-zero exit, unparsable output — yields `None`, so the
    /// detector skips the pass exactly like the POSIX `ps` path (the badge
    /// simply never appears rather than the session breaking).
    #[cfg(target_os = "windows")]
    fn capture() -> Option<ProcessTree> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // Force UTF-8 stdout: Windows PowerShell 5.1 otherwise emits captured
        // output in the console OEM code page, which `from_utf8_lossy` would
        // mangle for non-ASCII command lines. Backtick-t is a literal tab
        // inside a PowerShell double-quoted string.
        const SCRIPT: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
            Get-CimInstance Win32_Process | ForEach-Object { \
            \"$($_.ProcessId)`t$($_.ParentProcessId)`t$($_.CommandLine)\" }";
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut agent_by_pid = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            let pid = match parts.next().and_then(|s| s.trim().parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            let ppid = match parts.next().and_then(|s| s.trim().parse::<u32>().ok()) {
                Some(p) => p,
                None => continue,
            };
            // A process without a readable command line (system/protected) is
            // still recorded so the ppid→children walk stays connected.
            let command = parts.next().unwrap_or("");
            agent_by_pid.insert(pid, detect_agent_in_command(command));
            children.entry(ppid).or_default().push(pid);
        }
        Some(ProcessTree {
            agent_by_pid,
            children,
        })
    }

    /// Find the first known agent running at or below `root` (the session's
    /// shell pid). Iterative DFS with a visited guard so a pathological
    /// process table can never spin.
    fn find_agent_under(&self, root: u32) -> Option<&'static str> {
        let mut stack = vec![root];
        let mut visited = std::collections::HashSet::new();
        while let Some(pid) = stack.pop() {
            if !visited.insert(pid) {
                continue;
            }
            if let Some(Some(kind)) = self.agent_by_pid.get(&pid) {
                return Some(kind);
            }
            if let Some(kids) = self.children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        None
    }
}

/// Spawn the background foreground-agent detector. Runs for the lifetime of
/// the app (a cheap sleeping thread); call once from setup.
pub fn spawn_agent_detector<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        thread::sleep(AGENT_DETECT_INTERVAL);
        detect_agents_once(&app);
    });
}

/// One detection pass: snapshot local sessions, capture the process table
/// once, diff each session's detected agent against its stored value, and
/// emit `terminal-session-agent` for the changes.
fn detect_agents_once<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<SessionState>();

    // Snapshot (id, shell pid, current agent) for every local session, then
    // release the lock before the (slower) `ps` call.
    let locals: Vec<(String, u32, Option<String>)> = {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        sessions
            .iter()
            .filter_map(|(id, s)| {
                let pid = s.child_pid?;
                let current = s.agent.lock().ok().and_then(|g| g.clone());
                Some((id.clone(), pid, current))
            })
            .collect()
    };
    if locals.is_empty() {
        return;
    }

    let tree = match ProcessTree::capture() {
        Some(t) => t,
        None => return,
    };

    let mut changes: Vec<(String, Option<String>)> = Vec::new();
    for (id, pid, current) in locals {
        let detected = tree.find_agent_under(pid).map(|k| k.to_string());
        if detected != current {
            changes.push((id, detected));
        }
    }
    if changes.is_empty() {
        return;
    }

    // Write the new values back (skipping sessions closed since the
    // snapshot), then emit outside the lock.
    {
        let sessions = match state.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        for (id, detected) in &changes {
            if let Some(s) = sessions.get(id) {
                if let Ok(mut g) = s.agent.lock() {
                    *g = detected.clone();
                }
            }
        }
    }
    for (id, detected) in changes {
        // Stuck-`working` backstop (T4.2): an agent that vanished from the
        // process tree (Some→None) may have skipped its `SessionEnd` hook,
        // leaving a stale activity record the sweep can no longer reach (it
        // agent-gates on presence, which is now `None`). Clear it here — but
        // only when a record exists — so the badge can't strand on a spinner
        // after the agent is gone.
        let should_clear = state
            .activity
            .lock()
            .map(|m| should_clear_activity_on_agent_exit(&m, &id, detected.is_none()))
            .unwrap_or(false);
        if should_clear {
            resolve_and_emit(app, &id, &state.activity, |sa| {
                apply_hook(sa, activity_state::EXIT)
            });
        }
        let _ = app.emit(
            "terminal-session-agent",
            SessionInfo {
                session_id: id,
                machine_id: String::new(),
                created_at: 0,
                title: None,
                machine_name: None,
                agent: detected,
            },
        );
    }
}

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
const CADENCE_WINDOW: Duration = Duration::from_millis(1000);

/// Time between activity sweeps. ~250ms keeps `working` appearing within one
/// tick of the first byte and `awaiting_input` settling ≤ ~1s after silence
/// (TERMINAL_ACTIVITY_PLAN §5).
const ACTIVITY_SWEEP_INTERVAL: Duration = Duration::from_millis(250);

/// Pure cadence read for a single session, factored out of the sweep loop so it
/// is unit-testable without spinning a real thread. Resolve `working` when the
/// session produced output within the cadence window, else `awaiting_input`.
/// Dedup and the agent-gate no longer live here — the shared resolver
/// (`SessionActivity` / `resolve_and_emit`) owns them now.
fn cadence_state(since_last_output: Duration) -> &'static str {
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
fn apply_cadence(sa: &mut SessionActivity, cadence: &'static str) {
    sa.cadence = Some(cadence);
}

/// Apply one explicit scanner (hook) state to a session's record. The approval
/// latch is set by `awaiting_approval` and cleared by ANY non-approval explicit
/// signal — a `working` resume, a `Stop`/idle → `awaiting_input`, or `exit` —
/// which is exactly §2's "clears on a working signal or its own source
/// retracting."
fn apply_hook(sa: &mut SessionActivity, state: &str) {
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
fn apply_screen(sa: &mut SessionActivity, present: bool) {
    sa.screen_approval = present;
}

/// Resolve the §2 precedence for a record (highest first): a seen `exit` wins,
/// then a latched `awaiting_approval` from EITHER source (hook or on-screen
/// recognizer), then an explicit hook working/awaiting_input (authoritative over
/// the cadence floor once the session's hooks have spoken), else the cadence
/// floor (`working` until the first cadence read). The hook tier is what stops a
/// TUI agent's never-quiet byte cadence from re-pinning an idle session to
/// `working` after its `Stop` hook reported `awaiting_input`.
fn resolve(sa: &SessionActivity) -> &'static str {
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
fn decide_and_record(map: &mut HashMap<String, SessionActivity>, id: &str) -> Option<String> {
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
fn should_clear_activity_on_agent_exit(
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
fn resolve_and_emit<R: Runtime>(
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
#[tauri::command]
pub fn report_terminal_screen_activity(
    app: AppHandle,
    session_state: State<'_, SessionState>,
    session_id: String,
    present: bool,
) -> Result<(), String> {
    let has_agent = {
        let sessions = session_state
            .sessions
            .lock()
            .map_err(|_| "session state lock poisoned".to_string())?;
        match sessions.get(&session_id) {
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
            .map(|m| m.contains_key(&session_id))
            .unwrap_or(false);
        if !exists {
            return Ok(());
        }
    }
    resolve_and_emit(&app, &session_id, &session_state.activity, |sa| {
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
fn sweep_activity_once<R: Runtime>(app: &AppHandle<R>) {
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
                let elapsed = elapsed_since_last_output(&s.last_output_at);
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

#[cfg(test)]
#[path = "../tests/infrastructure/terminal.rs"]
mod tests;
