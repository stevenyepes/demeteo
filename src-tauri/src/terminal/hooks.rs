use ssh2::Session;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::model::activity_state;

/// Whether an agent kind self-reports activity via injected hooks (Phase 2).
/// Only Claude for now; extended as more agents grow a hook transport. This is
/// a pure agent-CAPABILITY predicate and is deliberately OS-agnostic: whether
/// the hook TRANSPORT can actually be used for a given session (it is
/// POSIX-only) is a separate, transport-scoped decision made by
/// [`hook_transport_supported`]. Keeping the two apart is what lets a Windows
/// client keep its always-POSIX SSH sessions hooked while degrading only its
/// cmd.exe LOCAL sessions to unhooked.
pub(crate) fn is_hooked_agent_kind(kind: &str) -> bool {
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
pub(crate) fn hook_transport_supported(is_local: bool) -> bool {
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
pub(crate) fn shell_single_quote(value: &str) -> String {
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
pub(crate) fn cmd_double_quote(value: &str) -> String {
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
pub(crate) fn build_claude_activity_settings(kind: &str, nonce: &str) -> Option<String> {
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
pub(crate) fn write_activity_settings_file(
    nonce: &str,
    settings_json: &str,
) -> std::io::Result<PathBuf> {
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
pub(crate) fn remote_activity_settings_path(nonce: &str) -> String {
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
pub(crate) fn build_agent_launch_command(base_command: &str, settings_path: &Path) -> String {
    format!(
        "{base_command} --settings {}",
        shell_single_quote(&settings_path.to_string_lossy())
    )
}

pub(crate) fn new_activity_nonce() -> String {
    generate_activity_nonce()
}

pub(crate) fn upload_remote_settings(
    sess: &Session,
    remote_path: &str,
    settings_json: &str,
) -> Result<(), String> {
    write_remote_settings_file(sess, remote_path, settings_json)
}
