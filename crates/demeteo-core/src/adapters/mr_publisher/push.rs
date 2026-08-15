//! Pushing the feature branch to `origin` before an MR is opened, and the
//! credential path that authenticates it.
//!
//! ## Why the PAT rides an inline helper rather than a file or the URL
//!
//! Three ways to hand `git` a token, two of which this module deliberately
//! does not take:
//!
//! - **In the URL** (`https://user:tok@host/…`) puts it in this process's
//!   argv, readable by any local user through `ps`/`/proc/<pid>/cmdline`, and
//!   `git remote set-url` then persists it in `.git/config` indefinitely.
//! - **In a `GIT_ASKPASS` script on disk** — what this module used to do —
//!   fails on Windows twice over. The `0o700` that protected it was a
//!   documented no-op there, leaving a provider PAT under whatever the parent
//!   directory's ACL grants; and `CreateProcess` does not honour `#!`, so the
//!   `/bin/sh` script could not have run anyway. It also *never runs* on a
//!   default Git for Windows install: per `gitcredentials(7)` git consults
//!   credential **helpers** before `GIT_ASKPASS`, and the installer writes
//!   `credential.helper = manager` into the system gitconfig — so GCM answers
//!   first, either with a stale cached identity or with a **GUI** prompt that
//!   `GIT_TERMINAL_PROMPT=0` does not suppress, wedging an unattended run for
//!   as long as nobody is watching.
//!
//! So: `-c credential.helper=` to reset the accumulated helper list (an empty
//! value is the documented reset), then one inline helper that reads the token
//! from its own environment. Same invocation on all three desktop OSes, no
//! file, and nothing secret in argv — the argv carries only the helper's
//! *source text*, which names an environment variable.
//!
//! ## The one place the token is still observable
//!
//! `ExecutionPort::run_program` is argv-shaped, but the SSH transport can only
//! render argv and env into a shell command string for `channel.exec`. So on a
//! *remote* push the PAT is in the remote shell's own command line for the
//! length of the push, and in the `Err` text of a failed one. [`redacted`]
//! closes the second half. The first belongs to the transport rather than to
//! this module, and it is bounded only by how long the invocation carrying it
//! lives — which is why the PAT is handed to a single `git push` and must not
//! be added to the environment of anything longer-lived.

use std::sync::Arc;
use std::time::Duration;

use crate::ports::execution::{ExecutionPort, ProgramRequest};

/// Ceiling on the push itself. Its job is not to bound a slow network but to
/// bound a git that has decided to *ask* something: an unattended run has
/// nobody to answer, and before this the push had no deadline at all.
const PUSH_TIMEOUT: Duration = Duration::from_secs(300);

/// The variable the inline helper reads the token from. Same name
/// `crates/demeteo-runner/src/git_askpass.rs` uses for the same secret, so a
/// reader who has met one recognises the other.
const PAT_ENV_VAR: &str = "DEMETEO_GIT_PAT";

/// The variable the inline helper reads the provider-side username from. It is
/// not a secret; it rides the environment beside the token only so the two
/// cannot disagree.
const USER_ENV_VAR: &str = "DEMETEO_GIT_USERNAME";

pub(super) struct BranchPush<'a> {
    pub compute_type: &'a str,
    pub remote_host: Option<&'a str>,
    pub project_id: &'a str,
    pub workspace_dir: &'a std::path::Path,
    pub repo_path: &'a str,
    pub provider_kind: &'a str,
    pub provider_host: &'a str,
    pub pat: &'a str,
    pub source_branch: &'a str,
}

pub(super) async fn push_feature_branch(
    exec: &Arc<dyn ExecutionPort>,
    push: &BranchPush<'_>,
) -> Result<(), String> {
    // Resolve target directory of the repository.
    let target_dir = if push.compute_type.eq_ignore_ascii_case("local") {
        crate::paths::repo_target_dir_local(push.workspace_dir, push.project_id, push.repo_path)
            .to_string_lossy()
            .to_string()
    } else {
        crate::paths::repo_target_dir_str(
            exec,
            push.compute_type,
            push.remote_host,
            push.project_id,
            push.repo_path,
            None,
        )
        .await?
    };

    let machine_str = push
        .remote_host
        .unwrap_or(crate::domain::ids::LOCAL_MACHINE);

    let remote_user = remote_user(push.provider_kind);
    let remote_url = format!(
        "https://{}@{}/{}",
        remote_user, push.provider_host, push.repo_path
    );
    exec.run_program(
        machine_str,
        git_request(&target_dir, ["remote", "set-url", "origin", &remote_url]),
    )
    .await
    .map_err(|e| format!("Failed to update remote origin URL: {}", e))?;

    // `-f` so a retried or replayed feature can update a branch it already
    // pushed.
    exec.run_program(
        machine_str,
        push_request(&target_dir, push.source_branch, remote_user, push.pat),
    )
    .await
    .map_err(|e| {
        format!(
            "Failed to push feature branch to origin: {}",
            redacted(&e, push.pat)
        )
    })?;

    Ok(())
}

/// The username half of provider basic-auth. GitHub accepts any username
/// against a PAT but documents `x-access-token`; GitLab requires `oauth2`.
fn remote_user(provider_kind: &str) -> &'static str {
    if provider_kind.eq_ignore_ascii_case("github") {
        "x-access-token"
    } else {
        "oauth2"
    }
}

/// The credential helper git runs, as git will see it.
///
/// A leading `!` makes the rest a shell command line; git appends the
/// operation (`get`/`store`/`erase`) to it and runs the whole thing through
/// one `sh -c`, which on Windows is the `sh.exe` Git for Windows already
/// bundles and puts on its children's `PATH`. Hence the shell *function*: it
/// is what gives the appended operation somewhere to land as `$1`.
///
/// It answers `get` and nothing else. The explicit `return 0` is why the
/// `test` may fail: without it, `store` and `erase` — whose output git
/// discards — would exit non-zero and read as a broken helper.
fn credential_helper() -> String {
    format!(
        "!f() {{ test \"$1\" = get && printf \"username=%s\\npassword=%s\\n\" \"${}\" \"${}\"; return 0; }}; f",
        USER_ENV_VAR, PAT_ENV_VAR
    )
}

/// The push invocation, assembled where a test can read it.
///
/// The two `-c credential.helper=` are ordered and both required: the empty
/// one clears every helper the system, global and repository configs
/// accumulated (Git for Windows' `manager` among them), the second installs
/// ours as the only one. Command-line config is applied last, so the reset
/// reaches helpers no matter which file declared them.
///
/// `GCM_INTERACTIVE`/`GCM_GUI_PROMPT` are belt-and-braces for the same reason
/// the reset exists — they matter only if a GCM survives it.
pub(super) fn push_request(
    target_dir: &str,
    source_branch: &str,
    remote_user: &str,
    pat: &str,
) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: vec![
            "-C".to_string(),
            target_dir.to_string(),
            "-c".to_string(),
            "credential.helper=".to_string(),
            "-c".to_string(),
            format!("credential.helper={}", credential_helper()),
            "push".to_string(),
            "-f".to_string(),
            "origin".to_string(),
            source_branch.to_string(),
        ],
        env: [
            (PAT_ENV_VAR.to_string(), pat.to_string()),
            (USER_ENV_VAR.to_string(), remote_user.to_string()),
            ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
            ("GCM_INTERACTIVE".to_string(), "false".to_string()),
            ("GCM_GUI_PROMPT".to_string(), "0".to_string()),
        ]
        .into_iter()
        .collect(),
        timeout: Some(PUSH_TIMEOUT),
        ..ProgramRequest::default()
    }
}

/// Strip the token from text on its way to a caller that will store or display
/// it.
///
/// `shared::secret_scrub` masks *recognisable* tokens — the GitHub and GitLab
/// prefixes — and a self-hosted provider's PAT matches none of them. Here the
/// exact secret is in hand, so match on it and let the scrubber catch whatever
/// else the same message carries.
fn redacted(message: &str, pat: &str) -> String {
    let masked = if pat.is_empty() {
        std::borrow::Cow::Borrowed(message)
    } else {
        std::borrow::Cow::Owned(message.replace(pat, "***"))
    };
    crate::shared::secret_scrub::scrub_secrets(&masked).into_owned()
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(str::to_string).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/mr_publisher/push.rs"]
mod tests;
