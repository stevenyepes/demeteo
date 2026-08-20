//! What the push invocation must and must not contain. Every assertion here is
//! about text this process hands to `git`, so it holds on every platform —
//! which is the point: the credential path has no `cfg` arm to diverge on.

use super::*;

const PAT: &str = "glpat-not-a-real-token";

fn credential() -> GitCredential {
    GitCredential {
        user: "oauth2",
        pat: PAT.to_string(),
    }
}

fn request() -> ProgramRequest {
    push_request("/w/repo", "demeteo/f-1", true, Some(&credential()))
}

/// The whole reason the token moved out of the URL and off the disk: nothing
/// on the command line may carry it. `/proc/<pid>/cmdline` is world-readable
/// and `/proc/<pid>/environ` is not — that difference is the entire security
/// argument for putting the secret in one and not the other.
#[test]
fn the_token_is_nowhere_in_argv() {
    let req = request();
    for arg in &req.args {
        assert!(!arg.contains(PAT), "token leaked into argv: {arg}");
    }
    assert!(!req.executable.contains(PAT));
    assert_eq!(req.env.get(PAT_ENV_VAR).map(String::as_str), Some(PAT));
}

/// The helper must name the variable, not interpolate it — a helper body built
/// by substituting the secret would put it straight back in argv.
#[test]
fn the_helper_reads_the_token_from_the_environment() {
    let helper = credential_helper();
    assert!(!helper.contains(PAT));
    assert!(
        helper.contains(&format!("${}", PAT_ENV_VAR)),
        "helper must dereference {PAT_ENV_VAR}: {helper}"
    );
    assert!(
        helper.starts_with('!'),
        "git only treats a helper as a shell command line when it starts with `!`: {helper}"
    );
}

/// Order is the whole mechanism. The empty value resets every helper the
/// config files accumulated — Git for Windows' `manager`, which would
/// otherwise answer first with a stale identity or a GUI prompt — and it only
/// resets what precedes it.
#[test]
fn the_helper_list_is_reset_before_ours_is_installed() {
    let req = request();
    let reset = req
        .args
        .iter()
        .position(|a| a == "credential.helper=")
        .expect("the reset must be present");
    let ours = req
        .args
        .iter()
        .position(|a| a.starts_with("credential.helper=!"))
        .expect("our helper must be present");
    assert!(
        reset < ours,
        "the reset must precede our helper: {:?}",
        req.args
    );
    let push = req
        .args
        .iter()
        .position(|a| a == "push")
        .expect("this is a push");
    assert!(
        ours < push,
        "`-c` is only config for the subcommand when it precedes it: {:?}",
        req.args
    );
}

/// Every route a prompt could take out of an unattended run, closed. A push
/// that blocks on one of these blocks forever — nobody is watching.
#[test]
fn nothing_can_stop_and_ask() {
    let req = request();
    assert_eq!(
        req.env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        req.env.get("GCM_INTERACTIVE").map(String::as_str),
        Some("false")
    );
    assert_eq!(req.env.get("GCM_GUI_PROMPT").map(String::as_str), Some("0"));
    assert!(
        req.timeout.is_some(),
        "a push with no ceiling is the failure mode this exists to prevent"
    );
}

/// GitLab rejects any username but `oauth2` against a PAT, so this is not
/// cosmetic.
#[test]
fn the_provider_decides_the_username() {
    assert_eq!(remote_user("github"), "x-access-token");
    assert_eq!(remote_user("GitHub"), "x-access-token");
    assert_eq!(remote_user("gitlab"), "oauth2");
}

/// The SSH transport renders env into the command string it execs, and a
/// failed command's `Err` echoes that string. A self-hosted provider's token
/// matches none of `secret_scrub`'s prefixes, so the exact-value pass is the
/// one that has to catch it.
#[test]
fn a_failed_push_reports_no_token() {
    let raw = format!("Command failed: sh -c export DEMETEO_GIT_PAT={PAT}; git push");
    let out = redacted(&raw, PAT);
    assert!(!out.contains(PAT), "token survived redaction: {out}");
    assert!(
        out.contains("git push"),
        "the diagnosis must survive: {out}"
    );
}

/// An empty PAT must not turn redaction into a match on every empty span.
#[test]
fn redaction_of_an_empty_secret_is_the_identity() {
    assert_eq!(redacted("nothing to hide", ""), "nothing to hide");
}

/// A repository that authenticates itself gets the invocation it always got.
///
/// The credential is optional because the answer is genuinely optional: an ssh
/// remote needs no token, and installing an empty helper over one would be a
/// change to a push that already worked. What it must still carry is the
/// prompt suppression — a push that stops to ask is unanswerable whether or not
/// Demeteo had a token for it.
#[test]
fn an_uncredentialed_push_installs_no_helper_but_still_cannot_be_asked() {
    let req = push_request("/w/repo", "demeteo/f-1", false, None);

    assert!(
        !req.args.iter().any(|a| a.starts_with("credential.helper")),
        "no token, no helper: {:?}",
        req.args
    );
    assert!(!req.env.contains_key(PAT_ENV_VAR));
    assert!(!req.env.contains_key(USER_ENV_VAR));
    assert_eq!(
        req.env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
        Some("0"),
        "the prompt is unanswerable either way, and blocking on one is the \
         failure this closes"
    );
}

/// Force is the merge-request publisher's alone.
///
/// Every other push in the app aims at a branch a person may have committed to
/// since — a resolution, a clean sync merge, the Publish button — and `-f`
/// there would overwrite their work with Demeteo's idea of the branch.
#[test]
fn only_the_caller_that_asks_for_it_force_pushes() {
    assert!(push_request("/w/repo", "b", true, None)
        .args
        .iter()
        .any(|a| a == "-f"));
    assert!(!push_request("/w/repo", "b", false, None)
        .args
        .iter()
        .any(|a| a == "-f"));
}

/// A push git could not authenticate is diagnosed as one, and says what to do.
///
/// It reached no remote, so every piece of advice about the *branch* is wrong:
/// "fetch and sync again before publishing" sent the user round a loop that
/// could not terminate.
#[test]
fn a_push_that_could_not_authenticate_says_so() {
    let said = push_failure(
        "Command failed (exit code: Some(128)): fatal: could not read Password for \
         'https://x-access-token@github.com': No such device or address",
        None,
    );

    assert!(
        said.contains("could not authenticate"),
        "the diagnosis has to lead: {said}"
    );
    assert!(
        said.contains("Preferences → Providers"),
        "and name the one thing that fixes it: {said}"
    );
    assert!(
        said.contains("could not read Password"),
        "git's own words survive: {said}"
    );
}

/// A refusal is passed through untouched, so the caller keeps its own reading
/// of what a remote that heard the push objected to.
#[test]
fn a_push_the_remote_refused_is_not_relabelled() {
    let raw = "! [rejected] main -> main (fetch first)";
    assert_eq!(push_failure(raw, None), raw);
}

/// The token must not survive into a stored or displayed failure — the SSH
/// transport renders env into the command string it execs, and a failed
/// command's `Err` echoes that string back.
#[test]
fn a_failed_push_carries_no_token_into_its_diagnosis() {
    let cred = credential();
    let said = push_failure(
        &format!("sh -c export DEMETEO_GIT_PAT={PAT}; git push"),
        Some(&cred),
    );
    assert!(!said.contains(PAT), "token survived: {said}");
}

/// Resolving the credential from the remote, which is the whole reason this is
/// reachable from a push site that holds nothing but a directory.
mod from_the_remote {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use crate::adapters::step_executor::scripted_exec::ScriptedExec;
    use crate::domain::ids::ProviderId;
    use crate::domain::models::ProviderInstance;
    use crate::ports::db::AppSettingsRepository;
    use rusqlite::Connection;
    use std::sync::Arc;

    const REPO: &str = "/repos/demeteo";
    const GET_URL: &str = "git -C /repos/demeteo remote get-url origin";

    /// A provider for `host`, with `provider_id`'s PAT already in the process
    /// cache so the lookup never reaches the OS keyring — which a test has no
    /// business writing to.
    fn seeded(host: &str, kind: &str, provider_id: &str) -> Arc<SqliteAdapter> {
        let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
        db.add_provider_instance(ProviderInstance {
            id: ProviderId::from(provider_id),
            kind: kind.to_string(),
            host: host.to_string(),
            username: "someone".to_string(),
            avatar_url: String::new(),
            created_at: 0,
        })
        .unwrap();
        crate::credential_cache::set(provider_id, PAT);
        db
    }

    /// The form `mr_publisher` leaves behind, and the one every other push then
    /// failed on: token-free, so git has nothing to offer and no terminal to
    /// ask on.
    #[tokio::test]
    async fn a_token_free_remote_is_matched_to_its_provider() {
        let exec = ScriptedExec::new(&[]).with_programs(&[(
            GET_URL,
            Ok("https://x-access-token@github.com/acme/widgets\n"),
        )]);
        let db = seeded("github.com", "github", "prov-token-free");

        let cred = credential_for_repo(&exec, db.as_ref(), "local", REPO)
            .await
            .expect("a token-free https remote needs a credential");

        assert_eq!(cred.user, "x-access-token");
        assert_eq!(cred.pat, PAT);
    }

    /// An ssh clone authenticates itself. Answering `Some` here would install a
    /// helper over a push that already worked.
    #[tokio::test]
    async fn an_ssh_remote_needs_nothing() {
        let exec = ScriptedExec::new(&[])
            .with_programs(&[(GET_URL, Ok("git@github.com:acme/widgets.git\n"))]);
        let db = seeded("github.com", "github", "prov-ssh");

        assert!(credential_for_repo(&exec, db.as_ref(), "local", REPO)
            .await
            .is_none());
    }

    /// A host nothing is configured for degrades to an uncredentialed push
    /// rather than refusing to push at all — the failure then says so in words
    /// `is_credential_failure` reads, which is more use than a refusal here.
    #[tokio::test]
    async fn an_unconfigured_host_degrades_rather_than_refusing() {
        let exec = ScriptedExec::new(&[])
            .with_programs(&[(GET_URL, Ok("https://git.internal/acme/widgets\n"))]);
        let db = seeded("github.com", "github", "prov-elsewhere");

        assert!(credential_for_repo(&exec, db.as_ref(), "local", REPO)
            .await
            .is_none());
    }

    /// The probe itself failing is not evidence either way, and must not stop
    /// the push.
    #[tokio::test]
    async fn an_unreadable_remote_degrades_too() {
        let exec = ScriptedExec::new(&[]);
        let db = seeded("github.com", "github", "prov-unreadable");

        assert!(credential_for_repo(&exec, db.as_ref(), "local", REPO)
            .await
            .is_none());
    }
}
