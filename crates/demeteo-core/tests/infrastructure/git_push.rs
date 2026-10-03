//! What the push invocation must and must not contain. Every assertion here is
//! about text this process hands to `git`, so it holds on every platform —
//! which is the point: the credential path has no `cfg` arm to diverge on.

use super::*;

const PAT: &str = "glpat-not-a-real-token";
const HOST: &str = "gitlab.example.com";

fn credential() -> GitCredential {
    GitCredential {
        user: "oauth2",
        pat: PAT.to_string(),
        host: HOST.to_string(),
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

/// The host the helper answers for travels with the secret, beside it and
/// never in argv.
#[test]
fn the_host_binding_rides_the_environment_beside_the_credential() {
    let req = request();

    assert_eq!(req.env.get(HOST_ENV_VAR).map(String::as_str), Some(HOST));
    assert!(!req.args.iter().any(|a| a.contains(HOST)), "{:?}", req.args);
    assert!(credential_helper().contains(&format!("${}", HOST_ENV_VAR)));
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

/// `push_request` is what every push site hands the transport; factoring its
/// credential half out must not move a byte of it.
#[test]
fn a_credentialed_force_push_is_exactly_this_argv_and_env() {
    let req = request();

    assert_eq!(req.executable, "git");
    assert_eq!(
        req.args,
        [
            "-C".to_string(),
            "/w/repo".to_string(),
            "-c".to_string(),
            "credential.helper=".to_string(),
            "-c".to_string(),
            format!("credential.helper={}", credential_helper()),
            "push".to_string(),
            "-f".to_string(),
            "origin".to_string(),
            "demeteo/f-1".to_string(),
        ]
    );
    let mut env = crate::domain::git_push::unattended_env();
    env.insert(PAT_ENV_VAR.to_string(), PAT.to_string());
    env.insert(USER_ENV_VAR.to_string(), "oauth2".to_string());
    env.insert(HOST_ENV_VAR.to_string(), HOST.to_string());
    assert_eq!(req.env, env);
    assert_eq!(req.timeout, Some(PUSH_TIMEOUT));
}

#[test]
fn an_uncredentialed_push_is_exactly_this_argv_and_env() {
    let req = push_request("/w/repo", "b", false, None);

    assert_eq!(
        req.args,
        ["-C", "/w/repo", "push", "origin", "b"].map(String::from)
    );
    assert_eq!(req.env, crate::domain::git_push::unattended_env());
    assert_eq!(req.timeout, Some(PUSH_TIMEOUT));
}

mod clone {
    use super::*;

    fn request() -> ProgramRequest {
        clone_request(
            ["clone", "https://oauth2@h/r", "/t"]
                .map(String::from)
                .to_vec(),
            &credential(),
        )
    }

    #[test]
    fn the_credential_pair_leads_and_the_callers_args_follow_unchanged() {
        let req = request();

        let mut expected = credential_args();
        expected.extend(["clone", "https://oauth2@h/r", "/t"].map(String::from));
        assert_eq!(req.args, expected);
        assert_eq!(req.executable, "git");
        assert_eq!(&req.args[..4], &credential_args()[..]);
    }

    #[test]
    fn it_has_no_working_directory_flag_and_no_token_in_argv() {
        let req = request();

        assert!(!req.args.iter().any(|a| a == "-C"), "{:?}", req.args);
        assert!(!req.args.iter().any(|a| a.contains(PAT)), "{:?}", req.args);
    }

    #[test]
    fn it_carries_the_credential_and_prompt_suppression_but_no_push_deadline() {
        let req = request();

        let mut env = crate::domain::git_push::unattended_env();
        env.insert(PAT_ENV_VAR.to_string(), PAT.to_string());
        env.insert(USER_ENV_VAR.to_string(), "oauth2".to_string());
        env.insert(HOST_ENV_VAR.to_string(), HOST.to_string());
        assert_eq!(req.env, env);
        assert_eq!(req.timeout, None);
    }
}

mod fetch {
    use super::*;

    fn args() -> Vec<String> {
        ["origin", "--", "main"].map(String::from).to_vec()
    }

    #[test]
    fn a_credentialed_fetch_resets_then_installs_the_helper_before_the_subcommand() {
        let req = fetch_request("/w/repo", args(), Some(&credential()));

        let mut expected = vec!["-C".to_string(), "/w/repo".to_string()];
        expected.extend(credential_args());
        expected
            .extend(["fetch", "--no-recurse-submodules", "origin", "--", "main"].map(String::from));
        assert_eq!(req.args, expected);
        assert_eq!(req.executable, "git");
    }

    #[test]
    fn a_credentialed_fetch_carries_the_credential_and_every_prompt_suppression() {
        let req = fetch_request("/w/repo", args(), Some(&credential()));

        let mut env = crate::domain::git_push::unattended_env();
        env.insert(PAT_ENV_VAR.to_string(), PAT.to_string());
        env.insert(USER_ENV_VAR.to_string(), "oauth2".to_string());
        env.insert(HOST_ENV_VAR.to_string(), HOST.to_string());
        assert_eq!(req.env, env);
        for arg in &req.args {
            assert!(!arg.contains(PAT), "token leaked into argv: {arg}");
        }
    }

    #[test]
    fn an_uncredentialed_fetch_installs_no_helper_but_still_cannot_be_asked() {
        let req = fetch_request("/w/repo", args(), None);

        assert_eq!(
            req.args,
            ["-C", "/w/repo", "fetch", "origin", "--", "main"].map(String::from)
        );
        assert_eq!(req.env, crate::domain::git_push::unattended_env());
    }

    #[test]
    fn a_credentialed_fetch_forbids_submodule_recursion_before_the_callers_args() {
        let req = fetch_request("/w/repo", args(), Some(&credential()));

        let fetch_at = req.args.iter().position(|a| a == "fetch").unwrap();
        assert_eq!(req.args[fetch_at + 1], "--no-recurse-submodules");
        assert_eq!(req.args[fetch_at + 2], "origin");
    }

    #[test]
    fn a_credentialless_fetch_leaves_submodule_recursion_to_git() {
        let req = fetch_request("/w/repo", args(), None);

        assert!(
            !req.args.iter().any(|a| a.contains("submodules")),
            "{:?}",
            req.args
        );
    }

    #[test]
    fn a_fetch_inherits_no_push_deadline() {
        assert_eq!(
            fetch_request("/w/repo", args(), Some(&credential())).timeout,
            None
        );
        assert_eq!(fetch_request("/w/repo", args(), None).timeout, None);
    }
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
        assert_eq!(cred.host, "github.com");
        assert_eq!(cred.pat, PAT);
    }

    /// `sanitize_host` keeps a `:port`, so a self-hosted provider is stored with
    /// one — while git's `host=` and `credential_host` both drop it.
    #[tokio::test]
    async fn a_provider_on_a_port_is_matched_and_bound_without_it() {
        let exec = ScriptedExec::new(&[])
            .with_programs(&[(GET_URL, Ok("https://oauth2@git.internal:8443/o/r.git\n"))]);
        let db = seeded("git.internal:8443", "gitlab", "prov-ported");

        let cred = credential_for_repo(&exec, db.as_ref(), "local", REPO)
            .await
            .expect("the port is not part of the host a provider is matched by");

        assert_eq!(cred.host, "git.internal");
        assert_eq!(cred.user, "oauth2");
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

/// The helper run the way git runs it: one `sh -c` over the helper text with
/// the operation appended, the credential in the environment, and git's
/// `key=value` request on stdin.
#[cfg(unix)]
mod helper_execution {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn run(operation: &str, stdin: &str) -> (String, bool) {
        let helper = credential_helper();
        let script = format!("{} {}", helper.trim_start_matches('!'), operation);
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .env(PAT_ENV_VAR, PAT)
            .env(USER_ENV_VAR, "oauth2")
            .env(HOST_ENV_VAR, HOST)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // `store` and `erase` return without reading stdin, so the write can
        // lose the race to the helper's exit. Git ignores that EPIPE too.
        match child.stdin.take().unwrap().write_all(stdin.as_bytes()) {
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            other => other.unwrap(),
        }
        let out = child.wait_with_output().unwrap();
        (String::from_utf8(out.stdout).unwrap(), out.status.success())
    }

    fn request_for(host: &str) -> String {
        format!("protocol=https\nhost={host}\n\n")
    }

    #[test]
    fn it_answers_for_the_provider_host() {
        let (out, ok) = run("get", &request_for(HOST));

        assert!(ok);
        assert_eq!(out, format!("username=oauth2\npassword={PAT}\n"));
    }

    #[test]
    fn it_answers_nothing_for_any_other_host_and_still_succeeds() {
        for host in ["evil.example", "gitlab.example.com.evil.example", ""] {
            let (out, ok) = run("get", &request_for(host));

            assert!(ok, "host {host:?}");
            assert_eq!(out, "", "host {host:?} was handed the credential");
        }
    }

    #[test]
    fn a_port_on_the_provider_host_still_matches() {
        let (out, ok) = run("get", &request_for(&format!("{HOST}:8443")));

        assert!(ok);
        assert!(out.contains(PAT), "{out}");
    }

    #[test]
    fn a_request_naming_no_host_gets_nothing() {
        let (out, ok) = run("get", "protocol=https\n\n");

        assert!(ok);
        assert_eq!(out, "");
    }

    #[test]
    fn store_and_erase_print_nothing_and_succeed() {
        for operation in ["store", "erase"] {
            let (out, ok) = run(operation, &request_for(HOST));

            assert!(ok, "{operation}");
            assert_eq!(out, "", "{operation}");
        }
    }

    /// The ssh transport wraps the whole body in one `sh -c '…'`, so the helper
    /// text is single-quoted twice over (as a git arg, then as the body). It has
    /// to come out the other end byte-for-byte or the remote helper is a
    /// different program.
    #[test]
    fn the_helper_survives_the_remote_shell_quoting() {
        let helper = credential_helper();
        let quoted = crate::paths::shell_escape_posix(&helper);
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {quoted}"))
            .output()
            .unwrap();

        assert_eq!(String::from_utf8(out.stdout).unwrap(), helper);
    }
}
