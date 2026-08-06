//! Opt-in verbatim capture of what an agent process wrote on stdout.
//!
//! Every one-shot CLI agent narrates its turn as nd-JSON, and the drain loop
//! in `cli_runtime` keeps only what the runtime's `parse_event` recognises.
//! The codex adapter turns each `command_execution` item into
//! `AgentEvent::ToolCall`, which no consumer renders — `stream_agent_turn`
//! counts it for liveness and everything else matches `AgentEvent::Text`. So
//! the literal commands an agent ran are read off the wire and dropped, and
//! afterwards there is no way to answer whether an agent misbehaved because of
//! the environment Demeteo forwarded, the prompt it built, or the harness's own
//! sandbox. That question is the whole reason this module exists.
//!
//! Off unless [`TRACE_DIR_ENV`] names a directory: the capture is unabridged
//! agent output, which belongs on a developer's disk by explicit request and
//! nowhere else. Absent, nothing here allocates, opens, or writes.
//!
//! It lives in the shared runtime rather than in one adapter because the
//! question is never agent-specific — the same variable captures codex,
//! claude-code, opencode, hermes and pi, on either transport.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::shared::secret_scrub::scrub_secrets;

/// Names the directory raw turn captures are written to.
pub const TRACE_DIR_ENV: &str = "DEMETEO_AGENT_TRACE";

/// Ceiling on the session component of a capture's file name.
///
/// A session id is `<agent kind>-<thread id>`, and a thread id concatenates a
/// feature id, a step id and a task id, so the untruncated name can approach
/// the whole of Windows' 260-character path limit on its own — at which point
/// the capture silently stops existing on exactly one of the three desktop
/// targets.
const MAX_SESSION_COMPONENT: usize = 120;

/// Where captures go, or `None` when tracing is off.
///
/// Set-but-blank counts as off: exporting an empty value is how a variable is
/// unset in a shell profile or a CI matrix, and treating it as a path would
/// scatter captures through the process's working directory instead.
pub(crate) fn trace_dir(raw: Option<String>) -> Option<PathBuf> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// The file one turn is captured to: the session it belongs to, then its
/// ordinal within that session.
///
/// The ordinal is zero-padded so a directory listing is in turn order, and
/// every character outside `[A-Za-z0-9._-]` is replaced rather than rejected.
/// Neither half of a session id promises to be a legal path segment on all
/// three desktop targets, and a surviving separator would write the capture
/// into a sibling directory rather than the one the caller named.
pub(crate) fn trace_file_name(session_id: &str, turn: u64) -> String {
    let mut safe = String::with_capacity(session_id.len().min(MAX_SESSION_COMPONENT));
    for c in session_id.chars() {
        if safe.len() == MAX_SESSION_COMPONENT {
            break;
        }
        let keep = c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        safe.push(if keep { c } else { '_' });
    }
    if safe.is_empty() {
        safe.push_str("agent");
    }
    format!("{}.turn{:03}.jsonl", safe, turn)
}

/// One turn's capture file.
pub(crate) struct TurnTrace {
    /// `None` once a write has failed. The sink retires itself on the first
    /// error so a full disk costs one debug line rather than one per output
    /// line, and never anything the turn can observe.
    file: Option<std::fs::File>,
}

impl TurnTrace {
    /// The capture for one turn, or `None` when tracing is off or the file
    /// cannot be created.
    pub(crate) fn open(session_id: &str, turn: u64) -> Option<Self> {
        let dir = trace_dir(std::env::var(TRACE_DIR_ENV).ok())?;
        Self::open_in(&dir, session_id, turn)
    }

    pub(crate) fn open_in(dir: &Path, session_id: &str, turn: u64) -> Option<Self> {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::debug!(dir = %dir.display(), error = %e, "agent trace directory unavailable");
            return None;
        }
        let path = dir.join(trace_file_name(session_id, turn));
        match std::fs::File::create(&path) {
            Ok(file) => Some(Self { file: Some(file) }),
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "agent trace file unavailable");
                None
            }
        }
    }

    /// Append one line of agent output, scrubbed.
    ///
    /// Unbuffered, because the turns worth tracing are the ones that hang and
    /// are killed — a buffered writer's last KiB dies with the drain thread,
    /// and that tail is where the evidence is.
    pub(crate) fn record(&mut self, line: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(e) = writeln!(file, "{}", scrub_secrets(line)) {
            tracing::debug!(error = %e, "agent trace write failed; sink retired");
            self.file = None;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/trace.rs"]
mod tests;
