//! Proving the repository is actually on the machine that is about to work in
//! it, and saying what to do when it is not.
//!
//! Filesystem calls on one port and a string. It reads no other port and no
//! executor state, which is what lets it be tested against a single double
//! rather than an `ExecutionDriver` — and its output is the first thing a user
//! sees when a workspace was never bootstrapped, so the remediation sentence is
//! the load-bearing part, not the exit status.
//!
//! The existence question is asked with [`ExecutionPort::get_metadata`] rather
//! than a `test -d` shell body. A shell that cannot start answers non-zero,
//! which this probe reads as "the repository is gone" — so the one machine that
//! most needs a truthful message (`docs/WINDOWS_PARITY.md` Phase 3) is the one
//! guaranteed to be told a lie about its own disk.

use crate::ports::execution::{ExecutionPort, SftpEntry};

/// Everything the failure message is built from, unflattened.
///
/// The port answers are carried as data so the message is produced by a pure
/// function: the sentence a user meets is the part worth pinning, and pinning
/// it must not require a transport.
struct RepoAbsence<'a> {
    /// Why the target could not be confirmed as a directory — the port's own
    /// error, or `None` when the path resolved but is not a directory.
    target_error: Option<&'a str>,
    /// The target's parent, whose contents are the evidence a bootstrap clone
    /// never ran.
    parent: &'a str,
    /// Entry names in `parent`, or the error reading it produced.
    listing: Result<Vec<String>, &'a str>,
    /// The machine's own home directory, which is what a workspace root is
    /// derived from — a surprising value here explains a path that looks right
    /// and points nowhere.
    home: Result<String, String>,
}

/// Verify `target_dir` exists on `machine_id`, and report what was seen if it
/// does not.
///
/// Runs identically on every transport — these are port filesystem calls, and
/// the caller has already resolved the path for local or remote. That is the
/// point: a probe that branched here would be testing something other than what
/// the run is about to do.
///
/// The listing of the *parent* is read for the failure message's sake: an empty
/// listing is the signature of a workspace whose bootstrap clone never ran,
/// which is the one cause the user can fix themselves, and the remediation
/// names it. Nothing is read on the success path.
pub(crate) async fn verify_repo_present(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    target_dir: &str,
) -> Result<(), String> {
    let target = exec.get_metadata(machine_id, target_dir).await;
    if matches!(&target, Ok(entry) if entry.is_dir) {
        return Ok(());
    }

    let parent = std::path::Path::new(target_dir)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let listing = exec.list_dir(machine_id, &parent).await;

    Err(render_absence(
        machine_id,
        target_dir,
        &RepoAbsence {
            target_error: target.as_ref().err().map(String::as_str),
            parent: &parent,
            listing: listing
                .as_ref()
                .map(|entries| entries.iter().map(describe_entry).collect())
                .map_err(String::as_str),
            home: exec.resolve_home(machine_id).await,
        },
    ))
}

fn describe_entry(entry: &SftpEntry) -> String {
    format!("{} {}", if entry.is_dir { "d" } else { "-" }, entry.name)
}

fn render_absence(machine_id: &str, target_dir: &str, absence: &RepoAbsence<'_>) -> String {
    let mut out = format!(
        "Repository target dir does not exist on '{}': {}\n\
         Diagnostics:\n",
        machine_id, target_dir
    );
    match absence.target_error {
        Some(error) => out.push_str(&format!("  probe failed: {}\n", error)),
        None => out.push_str("  the path exists but is not a directory\n"),
    }
    match &absence.home {
        Ok(home) => out.push_str(&format!("  home on that machine: {}\n", home)),
        Err(error) => out.push_str(&format!("  home on that machine is unknown: {}\n", error)),
    }
    match &absence.listing {
        Ok(names) if names.is_empty() => {
            out.push_str(&format!("  contents of {}: (empty)\n", absence.parent))
        }
        Ok(names) => {
            out.push_str(&format!("  contents of {}:\n", absence.parent));
            for name in names {
                out.push_str(&format!("    {}\n", name));
            }
        }
        Err(error) => out.push_str(&format!(
            "  contents of {} could not be read: {}\n",
            absence.parent, error
        )),
    }
    out.push_str(
        "\nIf the parent dir listing is empty, the bootstrap clone \
         did not actually run for this project — re-save the \
         workspace settings to trigger a fresh bootstrap.",
    );
    out
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/impl_traits/repo_probe.rs"]
mod tests;
