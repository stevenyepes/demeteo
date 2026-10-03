// Tests for `prepare_origin` in `crates/demeteo-core/src/adapters/worktree/git_ops/clone.rs` (mirrored-tests convention). `super` = that module.

use super::{prepare_origin, ORIGIN_PROBE_TIMEOUT};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::git_push::fetch_request;
use crate::adapters::git_push::GitCredential;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::adapters::worktree::git_ops::divergence::{refresh_base_ref, BASE_FETCH_TIMEOUT};
use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::ports::db::AppSettingsRepository;
use rusqlite::Connection;
use std::sync::Arc;

const REPO: &str = "/repos/widgets";
const GET_URL: &str = "git -C /repos/widgets remote get-url origin";
const CONFIG_URL: &str = "git -C /repos/widgets config --get remote.origin.url";
const PAT: &str = "ghp-not-a-real-token";

fn db_with_provider(provider_id: &str, kind: &str, host: &str) -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    db.add_provider_instance(ProviderInstance {
        id: ProviderId::from(provider_id),
        kind: kind.to_string(),
        host: host.to_string(),
        username: "someone".to_string(),
        avatar_url: String::new(),
        created_at: 0,
    })
    .unwrap();
    db
}

fn set_url_key(clean: &str) -> String {
    format!("git -C {REPO} remote set-url origin {clean}")
}

/// A double that answers the probe with `origin` and `set_url` only for the
/// exact rewrite named; any other `set-url` is unscripted and therefore `Err`.
/// A rewrite also reads the raw config value, which here is the same text: no
/// `insteadOf` rule is in play.
fn exec_for(origin: &str, expected_set_url: Option<&str>) -> Arc<ScriptedExec> {
    let key = expected_set_url.map(set_url_key);
    let mut programs = vec![(GET_URL, Ok(origin))];
    if let Some(key) = key.as_deref() {
        programs.push((CONFIG_URL, Ok(origin)));
        programs.push((key, Ok("")));
    }
    Arc::new(ScriptedExec::new(&[]).with_programs(&programs))
}

fn set_urls(exec: &ScriptedExec) -> Vec<String> {
    exec.requests()
        .iter()
        .map(|r| r.args.join(" "))
        .filter(|a| a.contains("remote set-url"))
        .collect()
}

async fn prepare(
    exec: &Arc<ScriptedExec>,
    db: &SqliteAdapter,
    provider_id: &str,
    pat: Option<&str>,
) -> Option<GitCredential> {
    match pat {
        Some(pat) => crate::credential_cache::set(provider_id, pat),
        None => crate::credential_cache::invalidate(provider_id),
    }
    prepare_origin(exec.as_ref(), db, "local", REPO).await
}

#[tokio::test]
async fn a_token_bearing_origin_is_rewritten_once_and_the_credential_returned() {
    let db = db_with_provider("prov-mig-basic", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    let credential = prepare(&exec, &db, "prov-mig-basic", Some(PAT))
        .await
        .expect("a credential");

    assert_eq!(credential.pat, PAT);
    assert_eq!(credential.user, "x-access-token");
    assert_eq!(
        set_urls(&exec),
        vec![format!(
            "-C {REPO} remote set-url origin https://x-access-token@github.com/acme/widgets"
        )]
    );
    assert_eq!(
        exec.requests().len(),
        3,
        "one probe, one raw-config read, one rewrite"
    );
}

#[tokio::test]
async fn a_second_run_after_the_rewrite_issues_only_the_probe() {
    let db = db_with_provider("prov-mig-idem", "github", "github.com");
    let exec = exec_for("https://x-access-token@github.com/acme/widgets\n", None);

    let credential = prepare(&exec, &db, "prov-mig-idem", Some(PAT)).await;

    assert!(credential.is_some());
    assert_eq!(exec.requests().len(), 1);
    assert!(set_urls(&exec).is_empty());
}

#[tokio::test]
async fn origins_that_are_not_password_bearing_https_are_never_rewritten() {
    for (i, origin) in [
        "ssh://git@github.com/acme/widgets.git\n",
        "git@github.com:acme/widgets.git\n",
        "/srv/git/widgets.git\n",
        "https://github.com/acme/widgets\n",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("prov-mig-skip-{i}");
        let db = db_with_provider(&id, "github", "github.com");
        let exec = exec_for(origin, None);

        prepare(&exec, &db, &id, Some(PAT)).await;

        assert!(set_urls(&exec).is_empty(), "{origin}");
        assert_eq!(exec.requests().len(), 1, "{origin}");
    }
}

#[tokio::test]
async fn an_origin_for_a_host_with_no_provider_keeps_its_password() {
    let db = db_with_provider("prov-mig-other", "github", "github.com");
    let exec = exec_for("https://user:secret@git.example.org/acme/widgets\n", None);

    let credential = prepare(&exec, &db, "prov-mig-other", Some(PAT)).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty());
}

#[tokio::test]
async fn an_origin_whose_provider_has_no_pat_keeps_its_password() {
    let db = db_with_provider("prov-mig-nopat", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n",
        None,
    );

    let credential = prepare(&exec, &db, "prov-mig-nopat", None).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty());
}

#[tokio::test]
async fn an_empty_pat_skips_the_migration_and_offers_no_credential() {
    let db = db_with_provider("prov-mig-empty", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    let credential = prepare(&exec, &db, "prov-mig-empty", Some("")).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty());
}

#[tokio::test]
async fn an_embedded_token_that_differs_from_the_keyring_is_left_in_place() {
    let db = db_with_provider("prov-mig-mismatch", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:ghp-working-embedded@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    let (credential, logged) =
        capture_logs(prepare(&exec, &db, "prov-mig-mismatch", Some(PAT))).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty(), "the irreversible rewrite ran");
    assert_eq!(exec.requests().len(), 1, "only the probe");
    assert!(logged.contains("does not match"), "{logged:?}");
    for secret in [
        "ghp-working-embedded",
        PAT,
        "x-access-token",
        "github.com/acme",
    ] {
        assert!(
            !logged.contains(secret),
            "log leaked {secret:?}: {logged:?}"
        );
    }
}

#[tokio::test]
async fn a_percent_encoded_embedded_token_equal_to_the_keyring_pat_is_migrated() {
    let db = db_with_provider("prov-mig-encoded", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:p%40ss%2Fw@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    let credential = prepare(&exec, &db, "prov-mig-encoded", Some("p@ss/w"))
        .await
        .expect("a credential");

    assert_eq!(credential.pat, "p@ss/w");
    assert_eq!(set_urls(&exec).len(), 1);
}

#[tokio::test]
async fn a_password_with_an_unencoded_slash_is_never_migrated() {
    let db = db_with_provider("prov-mig-slash", "github", "github.com");
    let exec = exec_for(
        "https://x-access-token:p/ss@github.com/acme/widgets\n",
        None,
    );

    let credential = prepare(&exec, &db, "prov-mig-slash", Some("p/ss")).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty());
}

/// Every event's fields, as text, emitted while `work` runs. The test runtime is
/// current-thread, so the thread-local default subscriber sees the whole future.
async fn capture_logs<T>(work: impl std::future::Future<Output = T>) -> (T, String) {
    use std::sync::Mutex;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    struct Sink(Arc<Mutex<String>>);
    impl Visit for Sink {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .lock()
                .unwrap()
                .push_str(&format!("{}={value:?} ", field.name()));
        }
    }
    struct Capture(Arc<Mutex<String>>);
    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            event.record(&mut Sink(self.0.clone()));
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    let buffer = Arc::new(Mutex::new(String::new()));
    let _guard = tracing::subscriber::set_default(Capture(buffer.clone()));
    let out = work.await;
    let logged = buffer.lock().unwrap().clone();
    (out, logged)
}

const LEGACY: &str = "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n";
const CLEAN: &str = "https://x-access-token@github.com/acme/widgets\n";

/// The origin as git reports it, then as it reads after the failed `set-url`.
fn exec_with_failing_set_url(reread: Result<&str, &str>) -> Arc<ScriptedExec> {
    let set_url = set_url_key("https://x-access-token@github.com/acme/widgets");
    Arc::new(
        ScriptedExec::new(&[])
            .with_programs(&[
                (CONFIG_URL, Ok(LEGACY)),
                (set_url.as_str(), Err("could not lock config file")),
            ])
            .with_program_queue(GET_URL, &[Ok(LEGACY), reread]),
    )
}

#[tokio::test]
async fn a_failed_set_url_still_returns_the_credential() {
    let db = db_with_provider("prov-mig-setfail", "github", "github.com");
    let exec = exec_with_failing_set_url(Ok(LEGACY));

    let credential = prepare(&exec, &db, "prov-mig-setfail", Some(PAT))
        .await
        .expect("a credential");

    assert_eq!(credential.pat, PAT);
    assert_eq!(set_urls(&exec).len(), 1, "the rewrite was attempted");
}

#[tokio::test]
async fn a_failed_set_url_that_a_concurrent_run_already_fixed_logs_nothing() {
    let db = db_with_provider("prov-mig-race", "github", "github.com");
    let exec = exec_with_failing_set_url(Ok(CLEAN));

    let (credential, logged) = capture_logs(prepare(&exec, &db, "prov-mig-race", Some(PAT))).await;

    assert_eq!(credential.expect("a credential").pat, PAT);
    assert!(!logged.contains("still carries"), "{logged:?}");
    assert_eq!(
        exec.programs().iter().filter(|p| *p == GET_URL).count(),
        2,
        "the origin was read again after the failure"
    );
}

#[tokio::test]
async fn a_failed_set_url_that_left_the_password_in_place_logs_it_redacted() {
    let db = db_with_provider("prov-mig-stuck", "github", "github.com");
    let exec = exec_with_failing_set_url(Ok(LEGACY));

    let (credential, logged) = capture_logs(prepare(&exec, &db, "prov-mig-stuck", Some(PAT))).await;

    assert!(credential.is_some());
    assert!(logged.contains("still carries"), "{logged:?}");
    for secret in [PAT, "x-access-token", "github.com/acme"] {
        assert!(
            !logged.contains(secret),
            "log leaked {secret:?}: {logged:?}"
        );
    }
}

#[tokio::test]
async fn a_failed_set_url_followed_by_an_unreadable_origin_still_warns() {
    let db = db_with_provider("prov-mig-reread-err", "github", "github.com");
    let exec = exec_with_failing_set_url(Err("transport gone"));

    let (_, logged) = capture_logs(prepare(&exec, &db, "prov-mig-reread-err", Some(PAT))).await;

    assert!(logged.contains("still carries"), "{logged:?}");
}

#[tokio::test]
async fn an_origin_expanded_by_insteadof_is_not_baked_into_the_repo_config() {
    let db = db_with_provider("prov-mig-instead", "github", "github.com");
    let set_url = set_url_key("https://x-access-token@github.com/acme/widgets");
    let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&[
        (GET_URL, Ok(LEGACY)),
        (CONFIG_URL, Ok("https://github.com/acme/widgets\n")),
        (set_url.as_str(), Ok("")),
    ]));

    let credential = prepare(&exec, &db, "prov-mig-instead", Some(PAT)).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty(), "the expansion was written back");
    assert_eq!(exec.requests().len(), 2, "get-url and the raw config read");
}

#[tokio::test]
async fn an_unreadable_raw_config_value_skips_the_rewrite() {
    let db = db_with_provider("prov-mig-raw-err", "github", "github.com");
    let set_url = set_url_key("https://x-access-token@github.com/acme/widgets");
    let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&[
        (GET_URL, Ok(LEGACY)),
        (CONFIG_URL, Err("exit 1")),
        (set_url.as_str(), Ok("")),
    ]));

    let credential = prepare(&exec, &db, "prov-mig-raw-err", Some(PAT)).await;

    assert!(credential.is_none());
    assert!(set_urls(&exec).is_empty());
}

#[tokio::test]
async fn a_self_hosted_origin_keeps_its_port_and_dot_git_and_its_own_username() {
    let db = db_with_provider("prov-mig-self", "gitlab", "git.example.org");
    let exec = exec_for(
        "https://oauth2:ghp-not-a-real-token@git.example.org:8443/team/widgets.git\n",
        Some("https://oauth2@git.example.org:8443/team/widgets.git"),
    );

    let credential = prepare(&exec, &db, "prov-mig-self", Some(PAT))
        .await
        .expect("a credential");

    assert_eq!(credential.user, "oauth2");
    assert_eq!(set_urls(&exec).len(), 1);
}

#[tokio::test]
async fn an_uppercase_provider_kind_still_gets_the_github_username() {
    let db = db_with_provider("prov-mig-upper", "GitHub", "github.com");
    let exec = exec_for(
        "https://someone:ghp-not-a-real-token@github.com/acme/widgets\n",
        Some("https://someone@github.com/acme/widgets"),
    );

    let credential = prepare(&exec, &db, "prov-mig-upper", Some(PAT))
        .await
        .expect("a credential");

    assert_eq!(credential.user, "x-access-token");
    assert_eq!(
        set_urls(&exec),
        vec![format!(
            "-C {REPO} remote set-url origin https://someone@github.com/acme/widgets"
        )],
        "the username half already in the URL is kept, not replaced by the provider's"
    );
}

#[tokio::test]
async fn an_unreadable_origin_migrates_nothing_and_offers_no_credential() {
    let db = db_with_provider("prov-mig-unread", "github", "github.com");
    let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&[(GET_URL, Err("no such remote"))]));

    let credential = prepare(&exec, &db, "prov-mig-unread", Some(PAT)).await;

    assert!(credential.is_none());
    assert_eq!(exec.requests().len(), 1);
}

#[tokio::test]
async fn the_probe_and_the_rewrite_are_bounded_and_the_fetches_keep_their_own_deadlines() {
    let db = db_with_provider("prov-mig-timeout", "github", "github.com");
    crate::credential_cache::set("prov-mig-timeout", PAT);
    let exec = exec_for(
        "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    refresh_base_ref(exec.as_ref(), &db, "local", REPO, "main").await;

    let requests = exec.requests();
    let with = |needle: &str| {
        requests
            .iter()
            .find(|r| r.args.join(" ").contains(needle))
            .unwrap_or_else(|| panic!("no request containing {needle:?}"))
    };
    assert_eq!(with("remote get-url").timeout, Some(ORIGIN_PROBE_TIMEOUT));
    assert_eq!(with("config --get").timeout, Some(ORIGIN_PROBE_TIMEOUT));
    assert_eq!(with("remote set-url").timeout, Some(ORIGIN_PROBE_TIMEOUT));
    assert_eq!(with(" fetch ").timeout, Some(BASE_FETCH_TIMEOUT));
    assert_eq!(fetch_request(REPO, vec![], None).timeout, None);
}

fn scrub_helper(
    db: SqliteAdapter,
    exec: &Arc<ScriptedExec>,
) -> crate::adapters::worktree::git_ops::GitOpsHelper {
    crate::adapters::worktree::git_ops::GitOpsHelper::new(Arc::new(db), exec.clone())
}

#[tokio::test]
async fn scrubbing_a_token_bearing_origin_issues_exactly_one_set_url() {
    use crate::ports::worktree_ops::WorktreeOpsPort;
    let db = db_with_provider("prov-mig-scrub", "github", "github.com");
    crate::credential_cache::set("prov-mig-scrub", PAT);
    let exec = exec_for(
        "https://x-access-token:ghp-not-a-real-token@github.com/acme/widgets\n",
        Some("https://x-access-token@github.com/acme/widgets"),
    );

    WorktreeOpsPort::scrub_origin_credentials(&scrub_helper(db, &exec), None, REPO).await;

    assert_eq!(
        set_urls(&exec),
        ["-C /repos/widgets remote set-url origin https://x-access-token@github.com/acme/widgets"]
    );
}

#[tokio::test]
async fn scrubbing_a_token_free_origin_issues_no_set_url() {
    use crate::ports::worktree_ops::WorktreeOpsPort;
    let db = db_with_provider("prov-mig-scrub-clean", "github", "github.com");
    crate::credential_cache::set("prov-mig-scrub-clean", PAT);
    let exec = exec_for("https://github.com/acme/widgets\n", None);

    WorktreeOpsPort::scrub_origin_credentials(&scrub_helper(db, &exec), None, REPO).await;

    assert!(set_urls(&exec).is_empty());
}

/// Panics on a provider lookup, which is the first thing a keyring read hangs
/// off: `credential_for_remote` lists providers before it resolves a PAT.
struct NoProviderLookup;

impl AppSettingsRepository for NoProviderLookup {
    fn add_provider_instance(&self, _: ProviderInstance) -> Result<(), String> {
        unreachable!()
    }
    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String> {
        panic!("scrub looked up a provider, which is the road to the keyring")
    }
    fn delete_provider_instance(&self, _: &ProviderId) -> Result<(), String> {
        unreachable!()
    }
    fn get_app_session(&self, _: &str) -> Result<Option<String>, String> {
        unreachable!()
    }
    fn set_app_session(&self, _: &str, _: &str) -> Result<(), String> {
        unreachable!()
    }
    fn delete_app_session(&self, _: &str) -> Result<(), String> {
        unreachable!()
    }
    fn app_setting_get(&self, _: &str) -> Result<Option<String>, String> {
        unreachable!()
    }
    fn app_setting_set(&self, _: &str, _: &str) -> Result<(), String> {
        unreachable!()
    }
}

#[tokio::test]
async fn scrubbing_an_origin_with_no_password_never_touches_the_keyring() {
    use crate::ports::worktree_ops::WorktreeOpsPort;
    let unreadable = Arc::new(ScriptedExec::new(&[]).with_programs(&[(GET_URL, Err("no remote"))]));
    let mut cases = vec![(unreadable, "unreadable")];
    for origin in [
        "https://x-access-token@github.com/acme/widgets\n",
        "https://github.com/acme/widgets\n",
        "ssh://git@github.com/acme/widgets.git\n",
        "git@github.com:acme/widgets.git\n",
        "/srv/git/widgets.git\n",
    ] {
        cases.push((exec_for(origin, None), origin));
    }

    for (exec, label) in cases {
        let helper = crate::adapters::worktree::git_ops::GitOpsHelper::new(
            Arc::new(NoProviderLookup),
            exec.clone(),
        );

        WorktreeOpsPort::scrub_origin_credentials(&helper, None, REPO).await;

        assert_eq!(exec.programs(), [GET_URL.to_string()], "{label}");
    }
}
