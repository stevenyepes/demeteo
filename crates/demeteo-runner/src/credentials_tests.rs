// Tests extracted from `crates/demeteo-runner/src/credentials.rs`
// (mirrored-tests convention). `super` = that module.

use super::{CredentialStore, RunnerCredentialSource};
use async_trait::async_trait;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::worktree::git_ops::GitOpsHelper;
use demeteo_core::domain::feature_origin::Refspec;
use demeteo_core::domain::ids::ProviderId;
use demeteo_core::domain::models::{Platform, ProviderInstance};
use demeteo_core::ports::db::AppSettingsRepository;
use demeteo_core::ports::execution::{ExecutionPort, InteractiveHandle, ProgramRequest, SftpEntry};
use demeteo_core::ports::git_credentials::GitCredentialSource;
use demeteo_core::ports::run_events::RunEventsPort;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use demeteo_core::test_dir::TestDir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PROVIDER: &str = "runner-provider";
const PAT_A: &str = "tok-run-a-31f9c2";
const PAT_B: &str = "tok-run-b-7d0e44";

struct FakeRuns(Mutex<Vec<RunnerRun>>);

impl FakeRuns {
    fn new(rows: &[(&str, Option<&str>)]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(
            rows.iter()
                .map(|(run_id, project_id)| RunnerRun {
                    run_id: run_id.to_string(),
                    project_id: project_id.map(str::to_string),
                    feature_id: None,
                    spec_json: String::new(),
                    status: "running".to_string(),
                    error: None,
                    created_at: 0,
                    updated_at: 0,
                    resume_count: 0,
                    pushed_branch: None,
                    owner_client_id: String::new(),
                })
                .collect(),
        )))
    }
}

impl RunnerRunPort for FakeRuns {
    fn get_or_create(&self, _: &str, _: &str, _: &str, _: i64) -> Result<RunnerRun, String> {
        Err("unused".into())
    }
    fn update_status(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: i64,
    ) -> Result<(), String> {
        Err("unused".into())
    }
    fn get(&self, _: &str) -> Result<Option<RunnerRun>, String> {
        Err("unused".into())
    }
    fn list(&self) -> Result<Vec<RunnerRun>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn mark_all_running_interrupted(&self, _: i64) -> Result<(), String> {
        Err("unused".into())
    }
    fn bump_resume_count(&self, _: &str) -> Result<i64, String> {
        Err("unused".into())
    }
    fn cancel_if_active(&self, _: &str, _: i64) -> Result<Option<RunnerRun>, String> {
        Err("unused".into())
    }
}

fn workspace() -> PathBuf {
    Path::new("workspace").join("demeteo-runner")
}

fn repo_dir(workspace: &Path, project_id: &str) -> String {
    workspace
        .join("projects")
        .join(project_id)
        .join("repos")
        .join("widgets")
        .to_string_lossy()
        .into_owned()
}

fn two_runs() -> RunnerCredentialSource {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    creds.insert("run-b", PAT_B.to_string());
    let source = RunnerCredentialSource::new(creds, &workspace());
    source.bind(FakeRuns::new(&[
        ("run-a", Some("p-a")),
        ("run-b", Some("p-b")),
    ]));
    source
}

#[test]
fn each_run_resolves_only_its_own_credential_from_its_repo_dir() {
    let source = two_runs();
    let ws = workspace();

    assert_eq!(
        source.pat_for(PROVIDER, &repo_dir(&ws, "p-a")),
        Ok(PAT_A.to_string())
    );
    assert_eq!(
        source.pat_for(PROVIDER, &repo_dir(&ws, "p-b")),
        Ok(PAT_B.to_string())
    );
}

#[test]
fn the_provider_id_never_decides_whose_credential_it_is() {
    let source = two_runs();

    assert_eq!(
        source.pat_for("anything-else", &repo_dir(&workspace(), "p-b")),
        Ok(PAT_B.to_string())
    );
}

#[test]
fn worktree_sync_and_cache_siblings_resolve_to_their_repos_run() {
    let source = two_runs();
    let repo = repo_dir(&workspace(), "p-b");

    for sibling in [
        format!("{repo}_wt_f-1-step-s-implement"),
        format!("{repo}_wt_sync_demeteo-features-f-1"),
        format!("{repo}_cache_demeteo-features-f-1"),
    ] {
        assert_eq!(
            source.pat_for(PROVIDER, &sibling),
            Ok(PAT_B.to_string()),
            "{sibling}"
        );
    }
}

#[test]
fn a_path_outside_the_workspace_gets_no_credential() {
    let source = two_runs();
    let elsewhere = Path::new("elsewhere")
        .join("projects")
        .join("p-a")
        .join("repos")
        .join("widgets");

    assert!(source
        .pat_for(PROVIDER, &elsewhere.to_string_lossy())
        .is_err());
    assert!(source
        .pat_for(PROVIDER, &workspace().join("projects").to_string_lossy())
        .is_err());
}

#[test]
fn a_path_climbing_out_of_its_project_gets_no_credential() {
    let source = two_runs();
    let climbing = workspace()
        .join("projects")
        .join("p-a")
        .join("..")
        .join("p-b")
        .join("repos")
        .join("widgets");

    assert!(source
        .pat_for(PROVIDER, &climbing.to_string_lossy())
        .is_err());
}

#[test]
fn an_unbound_source_gets_no_credential_without_panicking() {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    let source = RunnerCredentialSource::new(creds, &workspace());

    assert!(source
        .pat_for(PROVIDER, &repo_dir(&workspace(), "p-a"))
        .is_err());
}

#[test]
fn a_run_without_a_credential_gets_none_and_never_anothers() {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-b", PAT_B.to_string());
    let source = RunnerCredentialSource::new(creds, &workspace());
    source.bind(FakeRuns::new(&[
        ("run-a", Some("p-a")),
        ("run-b", Some("p-b")),
    ]));

    assert!(source
        .pat_for(PROVIDER, &repo_dir(&workspace(), "p-a"))
        .is_err());
}

#[test]
fn a_project_no_run_owns_gets_no_credential() {
    let source = two_runs();

    assert!(source
        .pat_for(PROVIDER, &repo_dir(&workspace(), "p-unclaimed"))
        .is_err());
}

#[test]
fn a_run_not_yet_attached_to_a_project_matches_nothing() {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    let source = RunnerCredentialSource::new(creds, &workspace());
    source.bind(FakeRuns::new(&[("run-a", None)]));

    assert!(source
        .pat_for(PROVIDER, &repo_dir(&workspace(), "p-a"))
        .is_err());
}

#[test]
fn a_project_whose_runs_disagree_on_the_credential_gets_none() {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    creds.insert("run-b", PAT_B.to_string());
    let source = RunnerCredentialSource::new(creds, &workspace());
    source.bind(FakeRuns::new(&[
        ("run-a", Some("p-shared")),
        ("run-b", Some("p-shared")),
    ]));

    assert!(source
        .pat_for(PROVIDER, &repo_dir(&workspace(), "p-shared"))
        .is_err());
}

#[test]
fn a_removed_credential_is_no_longer_served() {
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    let source = RunnerCredentialSource::new(creds.clone(), &workspace());
    source.bind(FakeRuns::new(&[("run-a", Some("p-a"))]));
    let repo = repo_dir(&workspace(), "p-a");
    assert_eq!(source.pat_for(PROVIDER, &repo), Ok(PAT_A.to_string()));

    creds.remove("run-a");

    assert!(source.pat_for(PROVIDER, &repo).is_err());
}

#[derive(Default)]
struct RecordingExec {
    requests: Mutex<Vec<ProgramRequest>>,
}

impl RecordingExec {
    fn fetches(&self) -> Vec<ProgramRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.args.iter().any(|a| a == "fetch"))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl ExecutionPort for RecordingExec {
    async fn run_program(&self, _: &str, request: ProgramRequest) -> Result<String, String> {
        self.requests.lock().unwrap().push(request.clone());
        if request.args.iter().any(|a| a == "get-url") {
            Ok("https://gitlab.example.com/acme/widgets\n".to_string())
        } else if request.args.iter().any(|a| a == "fetch") {
            Ok(String::new())
        } else {
            Err(format!("unscripted: {:?}", request.args))
        }
    }
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Err("unscripted".into())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Err("unscripted".into())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unscripted".into())
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err("unscripted".into())
    }
    async fn get_metadata(&self, _: &str, _: &str) -> Result<SftpEntry, String> {
        Err("unscripted".into())
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        Err("unscripted".into())
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unscripted".into())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Err("unscripted".into())
    }
    async fn resolve_platform(&self, _: &str) -> Result<Platform, String> {
        Err("unscripted".into())
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Err("unscripted".into())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("unscripted".into())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        Err("unscripted".into())
    }
}

#[tokio::test]
async fn an_origin_fetch_carries_the_runs_pat_and_leaves_it_in_memory_only() {
    let root = TestDir::new("demeteo_runner_credential_source");
    let db = Arc::new(
        SqliteAdapter::new(demeteo_core::db::init_db(root.path().join("data")).unwrap()).unwrap(),
    );
    db.add_provider_instance(ProviderInstance {
        id: ProviderId::from(PROVIDER),
        kind: "gitlab".to_string(),
        host: "gitlab.example.com".to_string(),
        username: String::new(),
        avatar_url: String::new(),
        created_at: 0,
    })
    .unwrap();
    db.get_or_create("run-a", "{}", "", 0).unwrap();
    db.update_status("run-a", "running", Some("p-a"), None, None, None, 1)
        .unwrap();
    db.get_or_create("run-b", "{}", "", 0).unwrap();
    db.update_status("run-b", "running", Some("p-b"), None, None, None, 1)
        .unwrap();
    let creds = Arc::new(CredentialStore::new());
    creds.insert("run-a", PAT_A.to_string());
    creds.insert("run-b", PAT_B.to_string());
    let source = Arc::new(RunnerCredentialSource::new(creds, root.path()));
    source.bind(db.clone());
    let exec = Arc::new(RecordingExec::default());
    let helper = GitOpsHelper::new(db.clone(), exec.clone()).with_credential_source(source);
    let refspec = Refspec::try_from("refs/pull/7/head".to_string()).unwrap();

    helper
        .fetch_origin_refspec(None, &repo_dir(root.path(), "p-a"), &refspec)
        .await
        .expect("the recorded fetch succeeds");

    let fetches = exec.fetches();
    assert_eq!(fetches.len(), 1);
    let fetch = &fetches[0];
    let before_fetch: Vec<&String> = fetch.args.iter().take_while(|a| *a != "fetch").collect();
    assert!(
        before_fetch
            .iter()
            .any(|a| a.starts_with("credential.helper=!")),
        "{:?}",
        fetch.args
    );
    assert!(fetch.env.values().any(|v| v == PAT_A), "{:?}", fetch.env);
    assert!(fetch.env.values().all(|v| v != PAT_B));
    for request in exec.requests.lock().unwrap().iter() {
        assert!(request.args.iter().all(|a| !a.contains(PAT_A)));
    }
    assert!(
        demeteo_core::credential_cache::get_or_fetch(PROVIDER, || Err("empty".to_string()))
            .is_err(),
        "the runner's PAT reached the shared credential cache"
    );
    let events = db.list_since("run-a", 0).unwrap();
    assert!(events
        .iter()
        .all(|e| !format!("{} {:?}", e.kind, e.payload_json).contains(PAT_A)));
}
