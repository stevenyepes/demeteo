//! Pushing a branch to `origin`, and the credential path that authenticates
//! it.
//!
//! Every push Demeteo makes goes through here. That was not always true, and
//! the gap is what this module exists to close: the credential path lived
//! inside `mr_publisher`, which also rewrites `origin` to a deliberately
//! token-free URL on its way out (see [`credential_host`]). So the moment a
//! project published one merge request, the *other* three pushes — a resolved
//! sync, the Publish affordance, a clean sync merge — were issuing a bare `git
//! push` against a remote that had no credential to offer and no terminal to
//! ask on. They failed with `fatal: could not read Password` for the rest of
//! the project's life.
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

use std::time::Duration;

use crate::domain::git_push::credential_host;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ExecutionPort, ProgramRequest};

/// Ceiling on the push itself. Its job is not to bound a slow network but to
/// bound a git that has decided to *ask* something: an unattended run has
/// nobody to answer, and before this the push had no deadline at all.
const PUSH_TIMEOUT: Duration = Duration::from_secs(300);

/// The variable the inline helper reads the token from. Same name
/// `crates/demeteo-runner/src/git_askpass.rs` uses for the same secret, so a
/// reader who has met one recognises the other.
pub(crate) const PAT_ENV_VAR: &str = "DEMETEO_GIT_PAT";

/// The variable the inline helper reads the provider-side username from. It is
/// not a secret; it rides the environment beside the token only so the two
/// cannot disagree.
pub(crate) const USER_ENV_VAR: &str = "DEMETEO_GIT_USERNAME";

/// What one push needs to authenticate, when it needs anything at all.
pub(crate) struct GitCredential {
    /// The provider-side username half of basic auth.
    pub user: &'static str,
    pub pat: String,
}

/// The credential a push from `repo_dir` will need, or `None` when it needs
/// none.
///
/// Read from the repository's own `origin` rather than from the project row,
/// for two reasons. The push sites do not agree on what they hold — one has a
/// `ProjectId`, one a feature, one only a worktree path — and threading a
/// project through four layers to reach the same answer is how three of them
/// came not to have it at all. And the remote is the thing that actually
/// decides: a project configured with a provider but cloned over ssh needs no
/// token, and a `None` here is the correct answer rather than a degraded one.
///
/// Every failure degrades to `None` deliberately — an unreadable remote, no
/// provider for the host, no PAT in the keyring. The push then runs exactly as
/// it did before this module existed, and if git cannot authenticate it says
/// so in words [`is_credential_failure`](crate::domain::git_push::is_credential_failure)
/// can read. Refusing to push at all would be worse: it would break the ssh
/// projects that never needed a token, to protect the ones that did.
pub(crate) async fn credential_for_repo(
    exec: &dyn ExecutionPort,
    app_settings: &dyn AppSettingsRepository,
    machine_str: &str,
    repo_dir: &str,
) -> Option<GitCredential> {
    let remote = exec
        .run_program(
            machine_str,
            git_request(repo_dir, ["remote", "get-url", "origin"]),
        )
        .await
        .ok()?;
    let host = credential_host(remote.trim())?;
    let provider = app_settings
        .get_provider_instances()
        .ok()?
        .into_iter()
        .find(|p| p.host == host)?;
    Some(GitCredential {
        user: remote_user(&provider.kind),
        pat: crate::adapters::mr_publisher::resolve_pat_for(&provider.id.0).ok()?,
    })
}

/// The username half of provider basic-auth. GitHub accepts any username
/// against a PAT but documents `x-access-token`; GitLab requires `oauth2`.
pub(crate) fn remote_user(provider_kind: &str) -> &'static str {
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
pub(crate) fn credential_helper() -> String {
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
///
/// `force` is the merge-request publisher's alone, and is spelled at the call
/// site rather than defaulted here: it re-points a branch that was squashed
/// under an open MR, which is what that path means to do and what every other
/// push must never do to a branch a person may have committed to since.
pub(crate) fn push_request(
    repo_dir: &str,
    branch: &str,
    force: bool,
    credential: Option<&GitCredential>,
) -> ProgramRequest {
    let mut args = vec!["-C".to_string(), repo_dir.to_string()];
    if credential.is_some() {
        args.extend([
            "-c".to_string(),
            "credential.helper=".to_string(),
            "-c".to_string(),
            format!("credential.helper={}", credential_helper()),
        ]);
    }
    args.push("push".to_string());
    if force {
        args.push("-f".to_string());
    }
    args.extend(["origin".to_string(), branch.to_string()]);

    // `GIT_TERMINAL_PROMPT=0` rides every push, credentialed or not: a push
    // that stops to ask is unanswerable in all of them, and the difference
    // between blocking until a wall-clock cap and failing in words a caller
    // can diagnose is the whole of what the user sees.
    let mut env: std::collections::BTreeMap<String, String> = [
        ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
        ("GCM_INTERACTIVE".to_string(), "false".to_string()),
        ("GCM_GUI_PROMPT".to_string(), "0".to_string()),
    ]
    .into_iter()
    .collect();
    if let Some(cred) = credential {
        env.insert(PAT_ENV_VAR.to_string(), cred.pat.clone());
        env.insert(USER_ENV_VAR.to_string(), cred.user.to_string());
    }

    ProgramRequest {
        executable: "git".to_string(),
        args,
        env,
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
pub(crate) fn redacted(message: &str, pat: &str) -> String {
    let masked = if pat.is_empty() {
        std::borrow::Cow::Borrowed(message)
    } else {
        std::borrow::Cow::Owned(message.replace(pat, "***"))
    };
    crate::shared::secret_scrub::scrub_secrets(&masked).into_owned()
}

/// A failed push, said in the words of whichever half of it went wrong, with
/// any token the message carried removed.
///
/// The two readings are not interchangeable advice: a remote that refused a
/// push heard it, and fetching usually fixes what it objected to; a push that
/// never got past the credential exchange will fail again identically after any
/// number of fetches. Telling the user the second was the first is what sent
/// them round that loop.
pub(crate) fn push_failure(error: &str, credential: Option<&GitCredential>) -> String {
    let clean = redacted(error, credential.map(|c| c.pat.as_str()).unwrap_or(""));
    if crate::domain::git_push::is_credential_failure(&clean) {
        return format!(
            "Git could not authenticate to origin, so nothing was pushed. \
             Connect the provider or refresh its token in Preferences → Providers, \
             then try again.\n\n{}",
            clean
        );
    }
    clean
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
#[path = "../../tests/infrastructure/git_push.rs"]
mod tests;
