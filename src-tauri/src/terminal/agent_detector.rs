// ---------------------------------------------------------------------------
// Foreground coding-agent detection
// ---------------------------------------------------------------------------
//
// A background poller walks the local process table every few seconds and
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

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::activity::{apply_hook, resolve_and_emit, should_clear_activity_on_agent_exit};
use super::model::{activity_state, SessionInfo, SessionState};

/// Time between foreground-agent detection passes. A few seconds keeps the
/// badge feeling live without spawning `ps` in a tight loop.
const AGENT_DETECT_INTERVAL: Duration = Duration::from_secs(3);

/// Maps a coding-agent CLI binary name to the "kind" the frontend labels.
/// Mirrors the frontend `AGENT_CLI` table (NewTerminalMenu) so the launch
/// path and the detector agree on the same identifiers.
pub(crate) fn agent_kind_for_binary(binary: &str) -> Option<&'static str> {
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
pub(crate) fn detect_agent_in_command(command: &str) -> Option<&'static str> {
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
pub(crate) struct ProcessTree {
    pub(crate) agent_by_pid: HashMap<u32, Option<&'static str>>,
    pub(crate) children: HashMap<u32, Vec<u32>>,
}

impl ProcessTree {
    /// Capture the process table via `ps`. Returns `None` if `ps` is
    /// unavailable or fails (e.g. a locked-down sandbox) — the detector
    /// then simply skips the pass.
    #[cfg(not(target_os = "windows"))]
    pub(super) fn capture() -> Option<ProcessTree> {
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
    pub(super) fn capture() -> Option<ProcessTree> {
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
        let mut visited = HashSet::new();
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
