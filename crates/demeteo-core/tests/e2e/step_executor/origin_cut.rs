//! The git a bootstrap runs to put the run's branch where its origin says.
//!
//! Asserted as the exact argv sequence rather than "the branch exists",
//! because the two arms are only distinguishable by what they asked git for:
//! both end with a branch of the same name, and a `Ref` run that quietly fell
//! back to the default branch would satisfy every other observation. The
//! double errors on anything unscripted (AGENTS.md §7), so a sequence that
//! drifts fails here instead of resolving against a default.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, CapturingNotif};
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, ProviderId, RepositoryId};
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository, WorkflowRepository};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::{FeatureLaunch, StepExecutor};
use crate::support::fake_credentials::FakeCredentialSource;

const REPO_PATH: &str = "demeteo/origin-cut";

/// One gate step: the driver parks on it, so nothing after the cut adds git of
/// its own to the sequence under assertion.
fn gate_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Origin Cut",
        "description": "Park immediately so the bootstrap's git is the whole story.",
        "steps": [{
            "id": "s-gate",
            "kind": "gate",
            "title": "Review"
        }]
    })
}

/// The git the bootstrap issued, in call order, for a run launched with
/// `origin`, and the branch it recorded — `None` when the run failed before
/// cutting one.
///
/// `script` pairs each argv *after* `git -C <repo_dir>` with what git answers;
/// the repository path is a temp dir this function is the first to know.
/// Anything the bootstrap tries that is not in it is an error it cannot
/// swallow.
async fn bootstrap_git(
    label: &str,
    origin: FeatureOrigin,
    script: &[(String, Result<&'static str, &'static str>)],
) -> (Vec<String>, Option<String>) {
    let run = bootstrap_git_via(label, origin, script, None).await;
    (run.ran, run.recorded)
}

/// What a bootstrap did, as far as these tests can see it.
struct BootstrapRun {
    ran: Vec<String>,
    recorded: Option<String>,
    requests: Vec<crate::ports::execution::ProgramRequest>,
    repo_dir: String,
    /// The `(provider_id, repo_dir)` pairs the injected source was asked for.
    asked: Vec<(String, String)>,
    events: Vec<DomainEvent>,
}

/// [`bootstrap_git`] with the executor's credential source replaced by one that
/// holds `pat` for the run's repo dir and for nothing else, and a provider for
/// `github.com` configured.
async fn bootstrap_git_via(
    label: &str,
    origin: FeatureOrigin,
    script: &[(String, Result<&'static str, &'static str>)],
    pat: Option<&str>,
) -> BootstrapRun {
    let temp_dir = scratch_dir(label);
    let project_id = format!("p-{label}");
    let feature_id = format!("f-{label}");
    let repo_dir = paths::repo_target_dir_local(&temp_dir, &project_id, REPO_PATH)
        .to_string_lossy()
        .to_string();

    let keys: Vec<String> = script
        .iter()
        .map(|(argv, _)| format!("git -C {repo_dir} {argv}"))
        .collect();
    let programs: Vec<(&str, Result<&str, &str>)> = keys
        .iter()
        .zip(script.iter())
        .map(|(key, (_, answer))| (key.as_str(), *answer))
        .collect();
    let exec = Arc::new(
        ScriptedExec::new(&[])
            .with_dirs(&[&repo_dir])
            .with_programs(&programs),
    );
    let notif = Arc::new(CapturingNotif::default());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), notif.clone(), exec.clone()).await;
    let source = pat.map(|pat| FakeCredentialSource::new(&[(repo_dir.as_str(), pat)]));
    let executor = match &source {
        Some(source) => executor.with_credential_source(source.clone()),
        None => executor,
    };
    if source.is_some() {
        crate::ports::db::AppSettingsRepository::add_provider_instance(
            &*db,
            crate::domain::models::ProviderInstance {
                id: ProviderId::from("origin-cut-provider"),
                kind: "github".to_string(),
                host: "github.com".to_string(),
                username: "someone".to_string(),
                avatar_url: String::new(),
                created_at: 0,
            },
        )
        .expect("add provider");
    }

    let projects: &dyn ProjectRepository = &*db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from(project_id.clone()),
            name: label.to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .expect("add project");
    projects
        .add_repository(crate::domain::models::Repository {
            id: RepositoryId::from(format!("r-{label}")),
            project_id: ProjectId::from(project_id.clone()),
            provider_id: ProviderId::from("origin-cut-provider"),
            repo_path: REPO_PATH.to_string(),
        })
        .expect("add repository");

    let workflows: Arc<dyn WorkflowRepository> = db.clone();
    let workflow_id = crate::application::workflows::create_from_json(&workflows, &gate_workflow())
        .expect("ingest workflow");

    executor
        .feature_start(FeatureLaunch {
            feature_id: Some(feature_id.clone()),
            project_id,
            workflow_id: workflow_id.0.clone(),
            title: "Origin Cut".to_string(),
            description: "Cut the branch from the declared origin.".to_string(),
            origin,
            ..Default::default()
        })
        .await
        .expect("feature_start returns the eager row");

    let features: &dyn FeatureRepository = &*db;
    let fid = FeatureId::from(feature_id.clone());
    let mut recorded = None;
    for _ in 0..300 {
        match features.get(&fid) {
            Ok(Some(f)) if f.resolved_branch.is_some() => {
                recorded = f.resolved_branch;
                break;
            }
            Ok(Some(f)) if f.status == "failed" => break,
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let ran = exec.programs();
    let requests = exec.requests();
    let _ = std::fs::remove_dir_all(temp_dir);
    let events = notif.events.lock().unwrap().clone();
    BootstrapRun {
        ran,
        recorded,
        requests,
        repo_dir,
        asked: source.map(|s| s.asked()).unwrap_or_default(),
        events,
    }
}

/// Every argv in `expected` succeeds, and the run is expected to cut.
async fn bootstrap_cutting(label: &str, origin: FeatureOrigin, expected: &[String]) -> Vec<String> {
    let script: Vec<(String, Result<&'static str, &'static str>)> =
        expected.iter().map(|argv| (argv.clone(), Ok(""))).collect();
    let (ran, recorded) = bootstrap_git(label, origin, &script).await;
    assert_eq!(
        recorded.as_deref(),
        Some(format!("demeteo/features/f-{label}").as_str()),
        "the cut records the branch it created, so later readers stop re-deriving it; \
         the git it ran: {ran:?}"
    );
    ran
}

/// The sequence every pre-V41 run issued — the best-effort fetch, the
/// tracking-ref probe, the ref-only fast-forward, then the cut from
/// `origin/<default>` — behind the one `origin` read that resolves the
/// credential both fetches share.
#[tokio::test]
async fn default_branch_origin_cuts_exactly_as_it_did_before_v41() {
    let expected = [
        "remote get-url origin".to_string(),
        "fetch origin -- main".to_string(),
        "rev-parse --verify origin/main".to_string(),
        "fetch origin +main:main".to_string(),
        "branch -f demeteo/features/f-origin_default origin/main".to_string(),
    ];
    let ran = bootstrap_cutting(
        "origin_default",
        FeatureOrigin::DefaultBranch,
        &expected[..],
    )
    .await;
    assert_git_prefix(&ran, &expected);
}

/// A `Branch` origin fetches its base and cuts from `origin/<base>`. The
/// refspec goes after `--`, like every other refspec this bootstrap builds.
#[tokio::test]
async fn branch_origin_cuts_from_the_tracking_ref_for_its_base() {
    let expected = [
        "remote get-url origin".to_string(),
        "fetch origin -- release/2.0".to_string(),
        "branch -f demeteo/features/f-origin_branch origin/release/2.0".to_string(),
    ];
    let ran = bootstrap_cutting(
        "origin_branch",
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        &expected[..],
    )
    .await;
    assert_git_prefix(&ran, &expected);
}

/// The half of the strictness split that is easy to lose: `origin/release/2.0`
/// is already in the clone, so an unreachable origin makes the cut stale — not
/// impossible — and the run goes on, exactly as a default-branch run does.
#[tokio::test]
async fn a_branch_origin_survives_an_unreachable_origin() {
    let script = [
        (
            "fetch origin -- release/2.0".to_string(),
            Err("fatal: could not read from remote repository"),
        ),
        (
            "branch -f demeteo/features/f-origin_branch_offline origin/release/2.0".to_string(),
            Ok(""),
        ),
    ];
    let (ran, recorded) = bootstrap_git(
        "origin_branch_offline",
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        &script[..],
    )
    .await;
    assert_eq!(
        recorded.as_deref(),
        Some("demeteo/features/f-origin_branch_offline"),
        "the git it ran: {ran:?}"
    );
}

/// The survived failure is reported as what it is. `completed` would tell the
/// stepper the sync went well, and the git error is the only thing that says
/// the run is cutting from a stale ref.
#[tokio::test]
async fn a_failed_best_effort_fetch_is_reported_as_a_warning() {
    const ERR: &str = "fatal: could not read from remote repository";
    let script = [
        ("fetch origin -- release/2.0".to_string(), Err(ERR)),
        (
            "branch -f demeteo/features/f-origin_warn origin/release/2.0".to_string(),
            Ok(""),
        ),
    ];
    let run = bootstrap_git_via(
        "origin_warn",
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        &script[..],
        None,
    )
    .await;

    let sync: Vec<(&str, Option<&str>)> = run
        .events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::BootstrapProgress {
                phase,
                status,
                detail,
                ..
            } if phase == "syncing_origin" => Some((status.as_str(), detail.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(sync.len(), 2, "running, then the outcome: {sync:?}");
    assert_eq!(sync[0].0, "running");
    assert_eq!(sync[1].0, "warning", "the git it ran: {:?}", run.ran);
    assert!(
        sync[1].1.is_some_and(|d| d.contains(ERR)),
        "the detail carries the git error: {sync:?}"
    );
}

/// And the other half: a fetched ref has no predecessor in the clone, so the
/// same failure has to stop the run rather than cut from whatever else
/// resolves.
#[tokio::test]
async fn a_ref_origin_stops_when_its_fetch_fails() {
    let script = [(
        "fetch origin -- +refs/pull/42/head:refs/demeteo/origins/pull/42/head".to_string(),
        Err("fatal: couldn't find remote ref refs/pull/42/head"),
    )];
    let (ran, recorded) = bootstrap_git(
        "origin_ref_missing",
        FeatureOrigin::Ref {
            fetch_spec: "refs/pull/42/head".to_string(),
            label: "PR #42".to_string(),
        },
        &script[..],
    )
    .await;
    assert_eq!(recorded, None, "the git it ran: {ran:?}");
    assert!(
        !ran.iter().any(|c| c.contains("branch -f")),
        "a cut after a failed fetch is a cut from whatever else resolved: {ran:?}"
    );
}

/// A `Ref` origin fetches its refspec into the private namespace and cuts from
/// what landed there — never from `origin/<default>`, which is what a silent
/// fallback would produce and what would make the run's review diff describe
/// the wrong change.
#[tokio::test]
async fn ref_origin_fetches_the_refspec_then_cuts_from_it() {
    const FETCHED: &str = "refs/demeteo/origins/pull/42/head";
    let expected = [
        "remote get-url origin".to_string(),
        format!("fetch origin -- +refs/pull/42/head:{FETCHED}"),
        format!("branch -f demeteo/features/f-origin_ref {FETCHED}"),
    ];
    let ran = bootstrap_cutting(
        "origin_ref",
        FeatureOrigin::Ref {
            fetch_spec: "refs/pull/42/head".to_string(),
            label: "PR #42".to_string(),
        },
        &expected[..],
    )
    .await;
    assert_git_prefix(&ran, &expected);
}

/// The fetch that cuts a `Ref` run authenticates through the source the
/// executor was composed with, asked about the run's own repo dir. The fake
/// holds a token for that directory only, so a helper built on the default
/// source asks nobody.
#[tokio::test]
async fn the_cut_fetch_asks_the_executors_credential_source() {
    const PAT: &str = "tok-origin-cut-6d20";
    let script = [(
        "remote get-url origin".to_string(),
        Ok("https://github.com/demeteo/origin-cut.git\n"),
    )];
    let run = bootstrap_git_via(
        "origin_ref_source",
        FeatureOrigin::Ref {
            fetch_spec: "refs/pull/42/head".to_string(),
            label: "PR #42".to_string(),
        },
        &script[..],
        Some(PAT),
    )
    .await;

    assert_eq!(
        run.asked,
        vec![("origin-cut-provider".to_string(), run.repo_dir.clone())],
        "the git it ran: {:?}",
        run.ran
    );
    let fetch = run
        .requests
        .iter()
        .find(|r| r.args.iter().any(|a| a == "fetch"))
        .unwrap_or_else(|| panic!("no fetch was issued: {:?}", run.ran));
    assert!(
        fetch.env.iter().any(|(_, v)| v == PAT),
        "the fetch must carry the injected token in its environment: {:?}",
        fetch.env
    );
    assert!(fetch.args.iter().all(|a| !a.contains(PAT)));
}

/// The bootstrap's git must be the *opening* of the log: the driver it starts
/// runs afterwards and is not this test's subject.
fn assert_git_prefix(ran: &[String], expected: &[String]) {
    let argv: Vec<String> = ran
        .iter()
        .take(expected.len())
        .map(|c| c.split(' ').skip(3).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(argv, expected, "unexpected bootstrap git: {ran:?}");
}
