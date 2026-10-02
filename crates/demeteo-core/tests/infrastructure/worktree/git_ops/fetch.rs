// Tests for `GitOpsHelper::fetch` in `crates/demeteo-core/src/adapters/worktree/git_ops/mod.rs` (mirrored-tests convention). `super` = that module.

use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::git_push::{credential_helper, PAT_ENV_VAR, USER_ENV_VAR};
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ProgramRequest;
use rusqlite::Connection;
use std::sync::Arc;

const REPO: &str = "/repos/widgets";
const GET_URL: &str = "git -C /repos/widgets remote get-url origin";
const PAT: &str = "ghp-not-a-real-token";

fn db_with_provider(provider_id: &str) -> Arc<SqliteAdapter> {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    db.add_provider_instance(ProviderInstance {
        id: ProviderId::from(provider_id),
        kind: "github".to_string(),
        host: "github.com".to_string(),
        username: "someone".to_string(),
        avatar_url: String::new(),
        created_at: 0,
    })
    .unwrap();
    db
}

/// Runs the shared fetch against a strict double that knows only the origin
/// probe and the fetch itself, and returns the fetch request it saw.
async fn fetch_against(origin: &str, provider_id: &str, seed_pat: bool) -> ProgramRequest {
    if seed_pat {
        crate::credential_cache::set(provider_id, PAT);
    }
    let fetch_key = "git -C /repos/widgets fetch origin main";
    let credentialed_key = format!(
        "git -C /repos/widgets -c credential.helper= -c credential.helper={} fetch --no-recurse-submodules origin main",
        credential_helper()
    );
    let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&[
        (GET_URL, Ok(origin)),
        (fetch_key, Ok("")),
        (&credentialed_key, Ok("")),
    ]));
    GitOpsHelper::new(db_with_provider(provider_id), exec.clone())
        .fetch("local", REPO, ["origin", "main"].map(String::from).to_vec())
        .await
        .expect("the scripted fetch succeeds");

    let requests = exec.requests();
    assert_eq!(requests.len(), 2, "one probe, one fetch: {requests:?}");
    requests[1].clone()
}

#[tokio::test]
async fn a_fetch_from_a_matching_provider_carries_the_helper_and_the_credential() {
    let req = fetch_against(
        "https://x-access-token@github.com/acme/widgets\n",
        "prov-fetch-match",
        true,
    )
    .await;

    let helper = format!("credential.helper={}", credential_helper());
    assert!(req.args.contains(&helper), "{:?}", req.args);
    assert_eq!(req.env.get(PAT_ENV_VAR).map(String::as_str), Some(PAT));
    assert_eq!(
        req.env.get(USER_ENV_VAR).map(String::as_str),
        Some("x-access-token")
    );
    assert!(!req.args.iter().any(|a| a.contains(PAT)));
}

#[tokio::test]
async fn a_fetch_from_an_ssh_origin_installs_no_helper() {
    let req = fetch_against("git@github.com:acme/widgets.git\n", "prov-fetch-ssh", true).await;

    assert!(!req.args.iter().any(|a| a.starts_with("credential.helper")));
    assert!(!req.env.contains_key(PAT_ENV_VAR));
    assert_eq!(
        req.env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
        Some("0")
    );
}

#[tokio::test]
async fn a_fetch_from_a_path_origin_installs_no_helper() {
    let req = fetch_against("/srv/git/widgets.git\n", "prov-fetch-path", true).await;

    assert!(!req.args.iter().any(|a| a.starts_with("credential.helper")));
    assert!(!req.env.contains_key(PAT_ENV_VAR));
}

#[tokio::test]
async fn a_fetch_with_no_pat_to_offer_is_still_issued_without_a_helper() {
    crate::credential_cache::invalidate("prov-fetch-no-pat");
    let req = fetch_against(
        "https://x-access-token@github.com/acme/widgets\n",
        "prov-fetch-no-pat",
        false,
    )
    .await;

    assert!(!req.args.iter().any(|a| a.starts_with("credential.helper")));
    assert!(!req.env.contains_key(PAT_ENV_VAR));
}
