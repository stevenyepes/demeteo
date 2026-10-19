// Tests extracted from `crates/demeteo-runner/src/run.rs` (mirrored-tests convention). `super` = that module.

use super::merge_project_settings;
use demeteo_core::adapters::step_executor::setup::fetch_default_settings;
use demeteo_core::domain::feature_origin::FeatureOrigin;
use demeteo_core::domain::ids::ProjectId;

#[test]
fn none_client_reproduces_detected_strategy() {
    // MC-D4: an old client (no settings) → detected strategy verbatim,
    // exactly the pre-multi-client behavior.
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "trunk".to_string();
    detected.test_command = Some("cargo test".to_string());
    let out = merge_project_settings(detected, None, ProjectId::from("p1".to_string()), None);
    assert_eq!(out.project_id.as_str(), "p1");
    assert_eq!(out.worktree_strategy.default_branch, "trunk");
    assert_eq!(
        out.worktree_strategy.test_command.as_deref(),
        Some("cargo test")
    );
}

#[test]
fn client_tunables_win_but_detected_default_branch_wins() {
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "stale-main".to_string();
    client.worktree_strategy.test_command = Some("npm test".to_string());
    client.worktree_strategy.prepare_command = Some("npm ci".to_string());
    client.worktree_strategy.extra_writable_paths = vec!["node_modules/".to_string()];
    client.feature_lifecycle = "manual".to_string();

    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    detected.test_command = Some("SHOULD NOT WIN".to_string());

    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p2".to_string()),
        None,
    );
    // Detected default_branch (read from origin/HEAD) wins over the
    // client's stale copy…
    assert_eq!(out.worktree_strategy.default_branch, "master");
    // …but the client wins on every other tunable.
    assert_eq!(
        out.worktree_strategy.test_command.as_deref(),
        Some("npm test")
    );
    assert_eq!(
        out.worktree_strategy.prepare_command.as_deref(),
        Some("npm ci")
    );
    assert_eq!(
        out.worktree_strategy.extra_writable_paths,
        vec!["node_modules/".to_string()]
    );
    assert_eq!(out.feature_lifecycle, "manual");
    assert_eq!(out.project_id.as_str(), "p2");
}

#[test]
fn empty_detected_branch_falls_back_to_client_then_main() {
    // Detected blank, client has a value → keep the client's.
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "develop".to_string();
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "   ".to_string();
    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p3".to_string()),
        None,
    );
    assert_eq!(out.worktree_strategy.default_branch, "develop");

    // Detected blank AND client blank → "main" (never an empty branch).
    let mut client2 = fetch_default_settings();
    client2.worktree_strategy.default_branch = String::new();
    let mut detected2 = fetch_default_settings().worktree_strategy;
    detected2.default_branch = String::new();
    let out2 = merge_project_settings(
        detected2,
        Some(client2),
        ProjectId::from("p4".to_string()),
        None,
    );
    assert_eq!(out2.worktree_strategy.default_branch, "main");
}

/// Detection succeeding with empty output is the way a blank arrives with no
/// client to fall through to: `git rev-parse --abbrev-ref origin/HEAD` returning
/// `Ok("")` is not an error, so `detect_worktree_strategy` hands back a strategy
/// naming no branch. The row it lands in is what `create_feature_branch`,
/// `merge_base` and the squash all read.
#[test]
fn a_blank_detected_branch_with_no_client_still_names_one() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = String::new();
    let out = merge_project_settings(detected, None, ProjectId::from("p8".to_string()), None);
    assert_eq!(out.worktree_strategy.default_branch, "main");
}

// ── The base a run declared ──────────────────────────────────────────────────
//
// Why the run's base outranks every other claimant on a runner-side
// `default_branch` is on `merge_project_settings`.

#[test]
fn a_declared_base_beats_the_detected_default_branch() {
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "main".to_string();
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();

    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p5".to_string()),
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        }
        .base_branch(None),
    );

    assert_eq!(
        out.worktree_strategy.default_branch, "release/2.0",
        "the detected default is a branch this run did not declare, and every \
         answer that falls back to this field would be about that one instead"
    );
}

#[test]
fn a_pull_request_run_measures_itself_against_the_branch_it_merges_into() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/pull/12/head".to_string(),
        label: "PR #12".to_string(),
    };

    let out = merge_project_settings(
        detected,
        None,
        ProjectId::from("p6".to_string()),
        origin.base_branch(Some("develop")),
    );

    assert_eq!(out.worktree_strategy.default_branch, "develop");
}

#[test]
fn a_blank_declared_base_leaves_the_detected_one_standing() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    let out = merge_project_settings(
        detected,
        None,
        ProjectId::from("p7".to_string()),
        Some("   "),
    );
    assert_eq!(
        out.worktree_strategy.default_branch, "master",
        "the row never carries an empty branch name"
    );
}

// ── RunSpec wire format (effort) ────────────────────────────────────
//
// `RunSpec` is the contract between the desktop app and this runner. The
// spec's version-skew rule (AGENTS.md §9.1) only holds if `effort` is
// genuinely optional in both directions.

#[test]
fn a_spec_without_effort_deserializes_to_none() {
    // An older desktop app sends no `effort` key at all. The runner must
    // accept the spec and inherit (project default, else High) — not fail.
    let spec: demeteo_core::domain::run_spec::RunSpec = serde_json::from_value(serde_json::json!({
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "/tmp/repo",
        "workflow_json": { "steps": [] },
        "agent_kind": "claude-code",
        "model": "sonnet",
    }))
    .expect("a pre-effort spec must still parse");
    assert_eq!(spec.effort, None);
}

#[test]
fn a_spec_with_effort_round_trips_through_the_wire() {
    let spec: demeteo_core::domain::run_spec::RunSpec = serde_json::from_value(serde_json::json!({
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "/tmp/repo",
        "workflow_json": { "steps": [] },
        "agent_kind": "codex",
        "model": "gpt-5",
        "effort": "max",
    }))
    .expect("a spec carrying effort must parse");
    assert_eq!(
        spec.effort,
        Some(demeteo_core::domain::models::EffortLevel::Max)
    );

    let json = serde_json::to_string(&spec).expect("RunSpec serializes");
    let back: demeteo_core::domain::run_spec::RunSpec =
        serde_json::from_str(&json).expect("RunSpec round-trips");
    assert_eq!(back.effort, spec.effort);
    // The canonical spelling is the lowercase one, on the wire as in the DB.
    assert!(json.contains("\"effort\":\"max\""));
}

// ── the branch the terminal push sends ──────────────────────────────────────
//
// A run's branch is whatever its bootstrap cut and recorded. Rebuilding it
// here from `branch_prefix` + feature id was right only for origins that
// derive their branch that way, and the runner is the one side of the fleet
// with no UI to notice it pushed the wrong name.

use super::branch_to_push;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::models::{Feature, Project};
use demeteo_core::ports::db::{FeatureRepository, ProjectRepository};

const PREFIX: &str = "demeteo/features/";

fn seeded_db(label: &str, resolved_branch: Option<&str>) -> SqliteAdapter {
    let dir = demeteo_core::test_dir::scratch(&format!("demeteo_runner_push_{label}"));
    let conn = demeteo_core::db::init_db(dir).expect("init_db");
    let db = SqliteAdapter::new(conn).expect("migrations run");
    ProjectRepository::add(
        &db,
        Project {
            id: ProjectId::from("p-push"),
            name: "push".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1_700_000_000,
        },
    )
    .expect("seed the project");
    FeatureRepository::add(
        &db,
        Feature {
            id: FeatureId::from("f-push"),
            project_id: ProjectId::from("p-push"),
            workflow_id: None,
            workflow_version_id: None,
            title: "push me".to_string(),
            description: String::new(),
            status: "completed".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: 1_700_000_000,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::Ref {
                fetch_spec: "refs/pull/42/head".to_string(),
                label: "PR #42".to_string(),
            },
            diff_base_branch: None,
            resolved_branch: resolved_branch.map(str::to_string),
        },
    )
    .expect("seed the feature");
    db
}

#[test]
fn the_push_sends_the_branch_the_run_recorded() {
    let db = seeded_db("recorded", Some("demeteo/pr-42-review"));

    assert_eq!(
        branch_to_push(&db, &FeatureId::from("f-push"), PREFIX),
        Ok("demeteo/pr-42-review".to_string())
    );
}

/// A row written before V41 recorded a branch, which is the one case the
/// derivation is still the answer for.
#[test]
fn a_run_that_recorded_no_branch_falls_back_to_the_derivation() {
    let db = seeded_db("underived", None);

    assert_eq!(
        branch_to_push(&db, &FeatureId::from("f-push"), PREFIX),
        Ok("demeteo/features/f-push".to_string())
    );
}

#[test]
fn a_missing_feature_stops_the_push_instead_of_naming_a_branch() {
    let db = seeded_db("missing", Some("demeteo/pr-42-review"));

    assert!(branch_to_push(&db, &FeatureId::from("f-gone"), PREFIX).is_err());
}

use super::{settle_run, RunOutcome};
use demeteo_core::ports::run_events::RunEventsPort;
use demeteo_core::ports::runner_run::RunnerRunPort;

fn run_db(label: &str) -> SqliteAdapter {
    let dir = demeteo_core::test_dir::scratch(&format!("demeteo_runner_settle_{label}"));
    let db = SqliteAdapter::new(demeteo_core::db::init_db(dir).expect("init_db"))
        .expect("migrations run");
    db.get_or_create("run-1", "{}", "", 1)
        .expect("seed the run");
    db
}

/// The laptop learns a detached run's ending from its event log, so a run
/// that ended in an error must say so there, not only on its status row.
#[test]
fn a_run_that_errors_leaves_a_failed_event_beside_its_status() {
    let db = run_db("err");

    settle_run(&db, &db, "run-1", Err("pre-push hook refused".to_string()));

    let run = RunnerRunPort::get(&db, "run-1")
        .expect("read")
        .expect("row");
    assert_eq!(run.status, "failed");
    assert_eq!(run.error.as_deref(), Some("pre-push hook refused"));
    let events = db.list_since("run-1", 0).expect("events");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "failed");
    assert!(
        events[0]
            .payload_json
            .as_deref()
            .unwrap_or("")
            .contains("pre-push hook refused"),
        "{events:?}"
    );
}

#[test]
fn a_run_that_settles_records_its_outcome_and_no_event() {
    let db = run_db("ok");

    settle_run(
        &db,
        &db,
        "run-1",
        Ok(RunOutcome {
            project_id: Some("p-1".to_string()),
            feature_id: Some("f-1".to_string()),
            status: "pr_ready".to_string(),
            pushed_branch: Some("demeteo/features/f-1".to_string()),
            pr_url: Some("https://example.invalid/pr/1".to_string()),
        }),
    );

    let run = RunnerRunPort::get(&db, "run-1")
        .expect("read")
        .expect("row");
    assert_eq!(run.status, "pr_ready");
    assert_eq!(run.pushed_branch.as_deref(), Some("demeteo/features/f-1"));
    assert!(db.list_since("run-1", 0).expect("events").is_empty());
}

use super::push_leased;
use demeteo_core::test_dir::TestDir;
use std::path::{Path, PathBuf};
use std::process::Command;

const PUSH_BRANCH: &str = "demeteo/features/f-race";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A clone whose branch origin has never heard of, and an origin that creates
/// that branch itself at `winner` while the clone's push is in flight — the
/// loser of two drivers pushing one run. The objects of both commits are
/// already on origin so the hook's ref points at something real.
struct RacedPush {
    _root: TestDir,
    clone: PathBuf,
    local: String,
}

impl RacedPush {
    fn new(tag: &str, winner_is_local: bool) -> Self {
        let root = TestDir::new(&format!("demeteo_runner_push_{tag}"));
        let origin = root.path().join("origin.git");
        let clone = root.path().join("clone");
        git(
            root.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        git(
            root.path(),
            &["clone", "-q", origin.to_str().unwrap(), "clone"],
        );
        std::fs::write(clone.join("README"), "base").unwrap();
        git(&clone, &["add", "README"]);
        git(&clone, &["commit", "-q", "-m", "base"]);
        git(&clone, &["push", "-q", "origin", "main"]);
        git(&clone, &["checkout", "-q", "-b", "elsewhere"]);
        std::fs::write(clone.join("E"), "elsewhere").unwrap();
        git(&clone, &["add", "E"]);
        git(&clone, &["commit", "-q", "-m", "elsewhere"]);
        let elsewhere = git(&clone, &["rev-parse", "HEAD"]);
        git(&clone, &["push", "-q", "origin", "elsewhere"]);
        git(&clone, &["checkout", "-q", "-b", PUSH_BRANCH, "main"]);
        std::fs::write(clone.join("L"), "local").unwrap();
        git(&clone, &["add", "L"]);
        git(&clone, &["commit", "-q", "-m", "local"]);
        let local = git(&clone, &["rev-parse", "HEAD"]);
        git(&clone, &["push", "-q", "origin", "HEAD:refs/heads/staging"]);
        git(&origin, &["update-ref", "-d", "refs/heads/staging"]);
        git(
            &clone,
            &[
                "update-ref",
                "-d",
                &format!("refs/remotes/origin/{PUSH_BRANCH}"),
            ],
        );
        let winner = if winner_is_local { &local } else { &elsewhere };
        let hook = origin.join("hooks").join("pre-receive");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\nunset GIT_QUARANTINE_PATH GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES\ngit update-ref refs/heads/{PUSH_BRANCH} {winner}\n"),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&hook).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&hook, perms).unwrap();
        Self {
            _root: root,
            clone,
            local,
        }
    }

    async fn push(&self) -> Result<(), String> {
        push_leased(
            Path::new("/nonexistent/askpass"),
            self.clone.to_str().unwrap(),
            PUSH_BRANCH,
            "unused",
        )
        .await
    }
}

#[tokio::test]
async fn a_push_that_lost_the_race_to_itself_is_a_success() {
    let raced = RacedPush::new("self", true);

    raced
        .push()
        .await
        .expect("origin already holds the commit being pushed");

    assert_eq!(
        git(&raced.clone, &["ls-remote", "origin", PUSH_BRANCH]),
        format!("{}\trefs/heads/{PUSH_BRANCH}", raced.local)
    );
    let tracking = git(
        &raced.clone,
        &["for-each-ref", "refs/remotes/origin", "--format=%(refname)"],
    );
    assert!(!tracking.contains(PUSH_BRANCH), "{tracking}");
}

#[tokio::test]
async fn a_push_that_lost_the_race_to_another_commit_is_still_an_error() {
    let raced = RacedPush::new("other", false);

    let error = raced
        .push()
        .await
        .expect_err("origin holds a different commit");
    assert!(error.contains("reference already exists"), "{error}");

    assert!(
        error.starts_with("failed to push feature branch to origin:"),
        "{error}"
    );
}

#[tokio::test]
async fn an_up_to_date_push_with_no_tracking_ref_succeeds() {
    let root = TestDir::new("demeteo_runner_push_uptodate");
    let origin = root.path().join("origin.git");
    let clone = root.path().join("clone");
    git(
        root.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    git(
        root.path(),
        &["clone", "-q", origin.to_str().unwrap(), "clone"],
    );
    std::fs::write(clone.join("README"), "base").unwrap();
    git(&clone, &["add", "README"]);
    git(&clone, &["commit", "-q", "-m", "base"]);
    git(&clone, &["push", "-q", "origin", "main"]);
    git(&clone, &["checkout", "-q", "-b", PUSH_BRANCH]);
    std::fs::write(clone.join("L"), "local").unwrap();
    git(&clone, &["add", "L"]);
    git(&clone, &["commit", "-q", "-m", "local"]);
    let local = git(&clone, &["rev-parse", "HEAD"]);
    git(&clone, &["push", "-q", "origin", PUSH_BRANCH]);
    git(
        &clone,
        &[
            "update-ref",
            "-d",
            &format!("refs/remotes/origin/{PUSH_BRANCH}"),
        ],
    );
    let remote_before = git(&clone, &["ls-remote", "origin", PUSH_BRANCH]);
    assert_eq!(remote_before, format!("{local}\trefs/heads/{PUSH_BRANCH}"));

    push_leased(
        Path::new("/nonexistent/askpass"),
        clone.to_str().unwrap(),
        PUSH_BRANCH,
        "unused",
    )
    .await
    .expect("origin already holds the commit being pushed");

    assert_eq!(
        git(&clone, &["ls-remote", "origin", PUSH_BRANCH]),
        remote_before
    );
    // git itself records the tracking ref on an up-to-date push, so unlike the
    // lost-race case there is none to be absent; it must agree with origin.
    assert_eq!(
        git(
            &clone,
            &["rev-parse", &format!("refs/remotes/origin/{PUSH_BRANCH}")]
        ),
        local
    );
}

// ── one driver per run ──────────────────────────────────────────────────────
//
// The entry under test is `try_acquire` + `start_driver`: the registry decision, the pass loop,
// the settle and the secret release. The pass itself is a stand-in that pushes
// to a real bare origin, because the real one needs a whole `AppContext`.

use super::{reopen_via, start_driver, DriverPorts};
use crate::credentials::CredentialStore;
use crate::driver::{Acquire, DriverGuard, RunDrivers};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::ports::db::FeaturePatch;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Barrier, Notify};
use tokio::task::JoinHandle;

const RUN_ID: &str = "run-1";
const PANIC_PAYLOAD: &str = "payload-that-could-embed-a-secret";
const PR_URL: &str = "https://example.invalid/pr/1";

/// What an entry into the registry came to: the driver's task, or nothing
/// because a live driver took the request.
enum Spawned {
    Driving(JoinHandle<()>),
    Attached,
}

type PassFuture = Pin<Box<dyn Future<Output = Result<RunOutcome, String>> + Send>>;

#[derive(Clone, Copy)]
enum PassKind {
    Push,
    Status(&'static str),
    Panic,
}

struct Scenario {
    _root: TestDir,
    origin: PathBuf,
    clone: PathBuf,
    pushes_log: PathBuf,
    spool_file: PathBuf,
    db: Arc<SqliteAdapter>,
    creds: Arc<CredentialStore>,
    drivers: Arc<RunDrivers>,
    spec: Arc<RunSpec>,
    passes: Arc<AtomicUsize>,
    parked: Arc<Notify>,
    release: Arc<Notify>,
}

impl Scenario {
    fn new(tag: &str) -> Self {
        let root = TestDir::new(&format!("demeteo_runner_driver_{tag}"));
        let origin = root.path().join("origin.git");
        let clone = root.path().join("clone");
        git(
            root.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        git(
            root.path(),
            &["clone", "-q", origin.to_str().unwrap(), "clone"],
        );
        std::fs::write(clone.join("README"), "base").unwrap();
        git(&clone, &["add", "README"]);
        git(&clone, &["commit", "-q", "-m", "base"]);
        git(&clone, &["push", "-q", "origin", "main"]);
        git(&clone, &["checkout", "-q", "-b", PUSH_BRANCH, "main"]);
        std::fs::write(clone.join("F"), "feature").unwrap();
        git(&clone, &["add", "F"]);
        git(&clone, &["commit", "-q", "-m", "feature"]);

        let pushes_log = root.path().join("pushes.log");
        let hook = origin.join("hooks").join("pre-receive");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\necho push >> {}\n", pushes_log.display()),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&hook).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&hook, perms).unwrap();

        let spool = root.path().join("spool").join(RUN_ID);
        std::fs::create_dir_all(&spool).unwrap();
        let spool_file = spool.join("shot.png");
        std::fs::write(&spool_file, "png").unwrap();
        let spec: RunSpec = serde_json::from_value(serde_json::json!({
            "title": "t",
            "description": "d",
            "provider": { "kind": "github", "host": "github.com" },
            "repo_path": "o/r",
            "workflow_json": { "steps": [] },
            "agent_kind": "claude-code",
            "model": "sonnet",
            "attachments": [{ "staged_path": spool_file.to_str().unwrap() }],
        }))
        .expect("spec parses");

        let db = Arc::new(seeded_db(tag, Some(PUSH_BRANCH)));
        db.get_or_create(RUN_ID, "{}", "", 1).expect("seed the run");
        RunnerRunPort::update_status(
            db.as_ref(),
            RUN_ID,
            "running",
            Some("p-push"),
            Some("f-push"),
            None,
            None,
            2,
        )
        .expect("record the ids");

        Self {
            _root: root,
            origin,
            clone,
            pushes_log,
            spool_file,
            db,
            creds: Arc::new(CredentialStore::new()),
            drivers: Arc::new(RunDrivers::default()),
            spec: Arc::new(spec),
            passes: Arc::new(AtomicUsize::new(0)),
            parked: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    fn ports(&self) -> DriverPorts {
        DriverPorts {
            creds: self.creds.clone(),
            run_events: self.db.clone(),
            runner_runs: self.db.clone(),
            features: self.db.clone(),
        }
    }

    /// The first pass reports in on `parked` and holds until `release`; any
    /// later pass runs straight through.
    fn pass(
        &self,
        kind: PassKind,
    ) -> impl FnMut(Option<String>, Option<String>) -> PassFuture + Send + 'static {
        let passes = self.passes.clone();
        let parked = self.parked.clone();
        let release = self.release.clone();
        let creds = self.creds.clone();
        let db = self.db.clone();
        let clone = self.clone.to_str().unwrap().to_string();
        move |project_id, feature_id| {
            let passes = passes.clone();
            let parked = parked.clone();
            let release = release.clone();
            let creds = creds.clone();
            let db = db.clone();
            let clone = clone.clone();
            Box::pin(async move {
                if passes.fetch_add(1, Ordering::SeqCst) == 0 {
                    parked.notify_one();
                    release.notified().await;
                }
                let status = match kind {
                    PassKind::Status(status) => status.to_string(),
                    PassKind::Panic => panic!("{PANIC_PAYLOAD}"),
                    PassKind::Push => {
                        let pat = creds
                            .get(RUN_ID)
                            .ok_or_else(|| "no PAT for the push".to_string())?;
                        push_leased(Path::new("/nonexistent/askpass"), &clone, PUSH_BRANCH, &pat)
                            .await?;
                        FeatureRepository::update(
                            db.as_ref(),
                            &FeatureId::from("f-push"),
                            &FeaturePatch {
                                mr_url: Some(Some(PR_URL.to_string())),
                                ..FeaturePatch::default()
                            },
                        )?;
                        "pr_ready".to_string()
                    }
                };
                Ok(RunOutcome {
                    project_id,
                    feature_id,
                    status,
                    pushed_branch: Some(PUSH_BRANCH.to_string()),
                    pr_url: None,
                })
            })
        }
    }

    fn spawn(&self, kind: PassKind) -> Spawned {
        match self.drivers.try_acquire(RUN_ID) {
            Acquire::Attached => Spawned::Attached,
            Acquire::Driver(guard) => Spawned::Driving(self.start(guard, kind)),
        }
    }

    fn start(&self, guard: DriverGuard, kind: PassKind) -> JoinHandle<()> {
        start_driver(
            self.ports(),
            guard,
            RUN_ID.to_string(),
            self.spec.clone(),
            self.pass(kind),
        )
    }

    async fn parked(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.parked.notified())
            .await
            .expect("the driver never reached its pass");
    }

    async fn finish(&self, driving: Spawned) {
        self.release.notify_one();
        let Spawned::Driving(handle) = driving else {
            panic!("expected to be the driver");
        };
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("the driver did not finish")
            .expect("the driver task panicked");
    }

    fn status(&self) -> String {
        RunnerRunPort::get(self.db.as_ref(), RUN_ID)
            .expect("read")
            .expect("row")
            .status
    }

    fn event_kinds(&self) -> Vec<String> {
        self.db
            .list_since(RUN_ID, 0)
            .expect("events")
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }

    fn pushes_to_origin(&self) -> usize {
        std::fs::read_to_string(&self.pushes_log)
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }
}

#[tokio::test]
async fn two_entries_for_one_run_push_once_and_end_pr_ready() {
    let s = Arc::new(Scenario::new("concurrent"));
    s.creds.insert(RUN_ID, "pat".to_string());
    let barrier = Arc::new(Barrier::new(2));
    let entries: Vec<_> = (0..2)
        .map(|_| {
            let (s, barrier) = (s.clone(), barrier.clone());
            tokio::spawn(async move {
                barrier.wait().await;
                s.spawn(PassKind::Push)
            })
        })
        .collect();

    s.parked().await;
    let mut driving = None;
    let mut attached = 0;
    for entry in entries {
        match entry.await.expect("entry task") {
            Spawned::Attached => attached += 1,
            other => driving = Some(other),
        }
    }
    assert_eq!(attached, 1, "exactly one entry may attach");
    s.finish(driving.expect("one entry must drive")).await;

    assert_eq!(s.pushes_to_origin(), 1, "origin saw more than one push");
    assert_eq!(
        s.passes.load(Ordering::SeqCst),
        1,
        "the re-arm re-ran a published run"
    );
    assert_eq!(s.status(), "pr_ready");
    assert!(
        !s.event_kinds().contains(&"failed".to_string()),
        "{:?}",
        s.event_kinds()
    );
    assert_eq!(
        git(
            &s.origin,
            &["rev-parse", &format!("refs/heads/{PUSH_BRANCH}")]
        ),
        git(&s.clone, &["rev-parse", "HEAD"])
    );
}

#[tokio::test]
async fn an_attached_caller_leaves_the_pat_while_the_driver_is_parked_before_its_push() {
    let s = Scenario::new("attached");
    s.creds.insert(RUN_ID, "pat".to_string());
    let driving = s.spawn(PassKind::Push);
    s.parked().await;

    assert!(matches!(s.spawn(PassKind::Push), Spawned::Attached));

    assert_eq!(s.creds.get(RUN_ID).as_deref(), Some("pat"));
    assert!(s.spool_file.exists(), "the attacher deleted the spool");
    assert_eq!(s.status(), "running", "the attacher settled the run");
    assert!(s.event_kinds().is_empty(), "{:?}", s.event_kinds());
    assert_eq!(s.pushes_to_origin(), 0);

    s.finish(driving).await;
    assert_eq!(s.pushes_to_origin(), 1);
    assert_eq!(s.status(), "pr_ready");
    assert_eq!(s.creds.get(RUN_ID), None, "the run's end wipes the PAT");
    assert!(!s.spool_file.exists(), "the run's end deletes the spool");
}

#[tokio::test]
async fn a_request_that_arrives_during_a_pass_runs_one_more() {
    let s = Scenario::new("rearm");
    let driving = s.spawn(PassKind::Status("completed"));
    s.parked().await;
    assert!(matches!(
        s.spawn(PassKind::Status("completed")),
        Spawned::Attached
    ));

    s.finish(driving).await;

    assert_eq!(s.passes.load(Ordering::SeqCst), 2);
    assert_eq!(s.status(), "completed");
}

#[tokio::test]
async fn a_pass_that_hands_off_internally_needs_one_acquisition() {
    let s = Scenario::new("handoff");
    let (passes, parked, release) = (s.passes.clone(), s.parked.clone(), s.release.clone());
    let Acquire::Driver(guard) = s.drivers.try_acquire(RUN_ID) else {
        panic!("a fresh run has no driver");
    };
    let driving = Spawned::Driving(start_driver(
        s.ports(),
        guard,
        RUN_ID.to_string(),
        s.spec.clone(),
        move |project_id, feature_id| {
            let (passes, parked, release) = (passes.clone(), parked.clone(), release.clone());
            async move {
                passes.fetch_add(1, Ordering::SeqCst);
                async fn create() {
                    tokio::task::yield_now().await;
                }
                async fn tail(parked: &Notify, release: &Notify) {
                    parked.notify_one();
                    release.notified().await;
                }
                create().await;
                tail(&parked, &release).await;
                Ok(RunOutcome {
                    project_id,
                    feature_id,
                    status: "completed".to_string(),
                    pushed_branch: None,
                    pr_url: None,
                })
            }
        },
    ));
    s.parked().await;
    s.finish(driving).await;

    assert_eq!(
        s.passes.load(Ordering::SeqCst),
        1,
        "the hand-off asked for a re-arm"
    );
    assert_eq!(s.status(), "completed");
    assert!(
        matches!(s.spawn(PassKind::Status("completed")), Spawned::Driving(_)),
        "the slot outlived the driver"
    );
}

#[tokio::test]
async fn a_needs_credentials_park_keeps_the_pat_and_the_spool() {
    let s = Scenario::new("parked");
    s.creds.insert(RUN_ID, "pat".to_string());
    s.release.notify_one();
    let driving = s.spawn(PassKind::Status("needs-credentials"));
    s.finish(driving).await;

    assert_eq!(s.status(), "needs-credentials");
    assert_eq!(s.creds.get(RUN_ID).as_deref(), Some("pat"));
    assert!(s.spool_file.exists());
    assert!(!s.event_kinds().contains(&"failed".to_string()));
}

#[tokio::test]
async fn a_pass_that_panics_fails_the_run_and_frees_the_slot() {
    let s = Scenario::new("panic");
    s.creds.insert(RUN_ID, "pat".to_string());
    s.release.notify_one();
    let driving = s.spawn(PassKind::Panic);
    s.finish(driving).await;

    let row = s.row();
    assert_eq!(row.status, "failed");
    assert_eq!(row.error.as_deref(), Some("driver pass panicked"));
    assert_eq!(s.creds.get(RUN_ID), None);
    assert!(!s.spool_file.exists());
    let events = s.db.list_since(RUN_ID, 0).expect("events");
    let failed = events
        .iter()
        .find(|e| e.kind == "failed")
        .expect("a failed event");
    assert!(!failed
        .payload_json
        .as_deref()
        .unwrap_or_default()
        .contains(PANIC_PAYLOAD));
    assert!(matches!(
        s.spawn(PassKind::Status("pr_ready")),
        Spawned::Driving(_)
    ));
}

// ── re-opening a run ────────────────────────────────────────────────────────

impl Scenario {
    fn set_row(&self, status: &str, error: Option<&str>) {
        RunnerRunPort::update_status(self.db.as_ref(), RUN_ID, status, None, None, error, None, 3)
            .expect("set the row");
    }

    fn row(&self) -> demeteo_core::ports::runner_run::RunnerRun {
        RunnerRunPort::get(self.db.as_ref(), RUN_ID)
            .expect("read")
            .expect("row")
    }

    fn reopen(&self, kind: PassKind) -> Spawned {
        self.reopen_through(self.db.as_ref(), kind)
    }

    fn reopen_through(&self, runs: &dyn RunnerRunPort, kind: PassKind) -> Spawned {
        reopen_via(runs, &self.drivers, RUN_ID, |guard| self.start(guard, kind))
            .expect("reopen")
            .map_or(Spawned::Attached, Spawned::Driving)
    }
}

/// The run table as seen by a re-opening caller. It errors on every write,
/// and runs `before_call` ahead of the first call of any kind,
/// which is where a test makes the live driver finish "in the middle" of the
/// caller's work.
struct WatchedRuns {
    inner: Arc<SqliteAdapter>,
    calls: AtomicUsize,
    before_call: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl WatchedRuns {
    fn new(inner: Arc<SqliteAdapter>) -> Self {
        Self {
            inner,
            calls: AtomicUsize::new(0),
            before_call: std::sync::Mutex::new(None),
        }
    }

    fn enter(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.before_call.lock().unwrap().take() {
            hook();
        }
    }
}

impl RunnerRunPort for WatchedRuns {
    fn get_or_create(
        &self,
        run_id: &str,
        spec_json: &str,
        owner_client_id: &str,
        now: i64,
    ) -> Result<demeteo_core::ports::runner_run::RunnerRun, String> {
        self.enter();
        self.inner
            .get_or_create(run_id, spec_json, owner_client_id, now)
    }

    fn update_status(
        &self,
        _run_id: &str,
        status: &str,
        _project_id: Option<&str>,
        _feature_id: Option<&str>,
        _error: Option<&str>,
        _pushed_branch: Option<&str>,
        _now: i64,
    ) -> Result<(), String> {
        self.enter();
        Err(format!("unexpected update_status({status}) by the caller"))
    }

    fn get(
        &self,
        run_id: &str,
    ) -> Result<Option<demeteo_core::ports::runner_run::RunnerRun>, String> {
        self.enter();
        RunnerRunPort::get(self.inner.as_ref(), run_id)
    }

    fn list(&self) -> Result<Vec<demeteo_core::ports::runner_run::RunnerRun>, String> {
        self.enter();
        self.inner.list()
    }

    fn mark_all_running_interrupted(&self, now: i64) -> Result<(), String> {
        self.enter();
        self.inner.mark_all_running_interrupted(now)
    }

    fn bump_resume_count(&self, run_id: &str) -> Result<i64, String> {
        self.enter();
        self.inner.bump_resume_count(run_id)
    }

    fn cancel_if_active(
        &self,
        run_id: &str,
        now: i64,
    ) -> Result<Option<demeteo_core::ports::runner_run::RunnerRun>, String> {
        self.enter();
        self.inner.cancel_if_active(run_id, now)
    }
}

#[tokio::test]
async fn a_retry_after_restart_recovery_joins_the_recovered_driver() {
    let s = Scenario::new("reconcile_retry");
    s.creds.insert(RUN_ID, "pat".to_string());
    let recovered = s.spawn(PassKind::Push);
    s.parked().await;

    assert!(matches!(s.reopen(PassKind::Push), Spawned::Attached));
    s.finish(recovered).await;

    assert_eq!(s.pushes_to_origin(), 1, "two drivers pushed one run");
    assert_eq!(s.passes.load(Ordering::SeqCst), 1);
    assert_eq!(s.status(), "pr_ready");
    assert!(
        !s.event_kinds().contains(&"failed".to_string()),
        "{:?}",
        s.event_kinds()
    );
}

#[tokio::test]
async fn a_retry_that_joins_a_live_driver_gets_its_pass_and_does_not_strand_the_run() {
    let s = Scenario::new("retry_attached");
    s.set_row("failed", Some("step broke"));
    let live = s.spawn(PassKind::Status("completed"));
    s.parked().await;

    assert!(matches!(
        s.reopen(PassKind::Status("completed")),
        Spawned::Attached
    ));
    s.finish(live).await;

    assert_eq!(s.passes.load(Ordering::SeqCst), 2, "the re-arm was dropped");
    assert_eq!(s.status(), "completed");
}

#[tokio::test]
async fn a_reopen_that_attaches_never_writes_the_row() {
    let s = Scenario::new("restore");
    s.set_row("failed", Some("step broke"));
    let live = s.spawn(PassKind::Status("completed"));
    s.parked().await;
    let runs = WatchedRuns::new(s.db.clone());

    assert!(matches!(
        s.reopen_through(&runs, PassKind::Status("completed")),
        Spawned::Attached
    ));

    let row = s.row();
    assert_eq!(row.status, "failed");
    assert_eq!(row.error.as_deref(), Some("step broke"));
    s.finish(live).await;
}

// The incident this guards is a `retry_step` racing a driver that is about to
// settle. The hook lets the live driver publish and settle `pr_ready` on the
// caller's first touch of the run table; an attacher that touches the table
// (to read a prior status, say) is then the one that loses the race, and ends
// by writing a stale `running` over `pr_ready` with nobody left to correct it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_driver_that_settles_while_a_reopen_attaches_leaves_the_row_pr_ready() {
    let s = Arc::new(Scenario::new("interleave"));
    s.creds.insert(RUN_ID, "pat".to_string());
    let live = s.spawn(PassKind::Push);
    s.parked().await;
    let runs = WatchedRuns::new(s.db.clone());
    *runs.before_call.lock().unwrap() = Some(Box::new({
        let s = s.clone();
        move || {
            s.release.notify_one();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while s.status() != "pr_ready" && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }));

    let reopened = s.reopen_through(&runs, PassKind::Push);

    assert!(matches!(reopened, Spawned::Attached));
    s.release.notify_one();
    let Spawned::Driving(handle) = live else {
        panic!("expected to be the driver");
    };
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("the driver did not finish")
        .expect("the driver task panicked");
    assert_eq!(s.status(), "pr_ready");
    assert_eq!(s.pushes_to_origin(), 1, "the re-arm pushed again");
    assert_eq!(s.passes.load(Ordering::SeqCst), 1);
    assert!(
        !s.event_kinds().contains(&"failed".to_string()),
        "{:?}",
        s.event_kinds()
    );
}

#[tokio::test]
async fn a_reopen_that_drives_leaves_the_row_running() {
    let s = Scenario::new("drives");
    s.set_row("failed", Some("step broke"));

    let driving = s.reopen(PassKind::Status("completed"));

    let row = s.row();
    assert_eq!(row.status, "running");
    assert_eq!(row.error, None);
    s.finish(driving).await;
    assert_eq!(s.status(), "completed");
}
