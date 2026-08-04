//! What the push invocation must and must not contain. Every assertion here is
//! about text this process hands to `git`, so it holds on every platform —
//! which is the point: the credential path has no `cfg` arm to diverge on.

use super::*;

const PAT: &str = "glpat-not-a-real-token";

fn request() -> ProgramRequest {
    push_request("/w/repo", "demeteo/f-1", "oauth2", PAT)
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
