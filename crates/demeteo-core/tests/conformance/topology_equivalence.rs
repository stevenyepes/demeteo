//! Topology-equivalence conformance gate (C5,
//! `docs/EXECUTION_PARITY.md`).
//!
//! The by-construction guarantee at the top level: one workflow, run through
//! every transport, must yield an equivalent [`RunView`]. This is the
//! mechanism that keeps new step kinds / artifact types consistent across
//! local, desktop-over-SSH, and the headless runner without per-path bug
//! hunting (D4/D6).
//!
//! The three legs share:
//! * the **same workflow** ([`minimal_workflow`]) — one deterministic agent
//!   step that writes one declared artifact;
//! * the **same deterministic agent** — [`StubRuntime`](crate::adapters::agent::stub_runtime)
//!   (`agent_kind: "stub"`), gated on by `DEMETEO_STUB_AGENT`, so a run
//!   reaches a terminal state with no real LLM CLI and byte-identical output;
//! * the **same read model** — [`RunView`], reduced to a transport-independent
//!   [`RunViewSnapshot`] the legs are asserted equal on.
//!
//! Leg availability:
//! * **local** — always runs (`topology_local_leg`), and is the reference the
//!   other legs are compared against. No Docker.
//! * **ssh** — reuses the C2.2 loopback `sshd` container; gated behind the
//!   `ssh-conformance` feature + `DEMETEO_SSH_CONFORMANCE_*` env (see
//!   `execution_port.rs`).
//! * **runner** — a `demeteo-runner` container; gated behind the
//!   `topology-conformance` feature + `DEMETEO_RUNNER_CONFORMANCE_*` env.
//!
//! Because the SSH and runner legs need external infrastructure, the
//! cross-transport equivalence assertion lives behind those feature gates; a
//! Docker-less `cargo test` still exercises the local leg (which is what
//! proves the stub agent + workflow + RunView snapshot machinery itself).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

/// The declared artifact the single workflow step must produce. The
/// `@stub-write` directive path and the `last_write_to` capture path are the
/// *same string* so the declared-artifact resolver matches (see
/// `stub_runtime` docs).
const ARTIFACT_PATH: &str = "artifacts/topology-report.md";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// One deterministic agent step: write the declared artifact, end the turn.
/// The `@stub-write` line in the prompt drives [`StubRuntime`]; everything
/// else is a normal `agent`/`artifacts` step identical in shape to the
/// bundled `simple-task` plan step.
fn minimal_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Topology Conformance",
        "description": "Single deterministic agent step for the C5 topology gate.",
        "steps": [
            {
                "id": "s-report",
                "kind": "agent",
                "title": "Produce report",
                "agent_kind": "stub",
                "prompt_template": format!(
                    "Produce the topology conformance report.\n\n\
                     Feature description: {{{{feature_description}}}}\n\n\
                     @stub-write {ARTIFACT_PATH}\n"
                ),
                "capability": "artifacts",
                "allow_shell": true,
                "artifacts": [
                    {
                        "name": "topology-report",
                        "capture": { "kind": "last_write_to", "path": ARTIFACT_PATH },
                        "mode": "full"
                    }
                ],
                "on_failure": null,
                "max_iterations": 1
            }
        ]
    })
}

/// A transport-independent reduction of a run's `RunView` — the shape the
/// three legs must agree on. Deliberately excludes anything that legitimately
/// varies by transport/run (ids, timestamps, absolute cache paths, cost
/// magnitude): what must match is the *step set*, each step's terminal
/// status, and each declared artifact's body.
#[derive(Debug, PartialEq, Eq)]
struct RunViewSnapshot {
    terminal_status: String,
    steps: Vec<StepView>,
    /// `(step_id, declared-artifact-basename, body)` for every materialized
    /// declared artifact, sorted for order-independent comparison.
    artifacts: Vec<(String, String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
struct StepView {
    step_id: String,
    kind: String,
    status: String,
}

/// Initialize a real local git repo at the project's expected repo dir so
/// `bootstrap_project` detects it and skips its (network) clone path — the
/// same "already cloned" shortcut the runner's `pre_clone_with_askpass`
/// relies on.
fn init_local_repo(workspace_dir: &Path, project_id: &str, repo_path: &str) {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, repo_path);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# topology conformance fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

/// Drive a freshly-started feature to a terminal state, returning its final
/// status. Fails the test if it doesn't settle within the timeout.
async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    const MAX_WAIT: Duration = Duration::from_secs(60);
    let started = Instant::now();
    loop {
        let feature = ctx
            .features
            .get(feature_id)
            .expect("feature read")
            .expect("feature exists");
        if matches!(
            feature.status.as_str(),
            "completed" | "awaiting_mr" | "failed" | "interrupted"
        ) {
            return feature.status;
        }
        if started.elapsed() > MAX_WAIT {
            // Dump step diagnostics so a hang/failure is legible.
            let steps = ctx
                .features
                .steps_for_feature(feature_id)
                .unwrap_or_default();
            panic!(
                "feature {} did not reach a terminal state in {:?}; last status {}, steps: {:#?}",
                feature_id.as_str(),
                MAX_WAIT,
                feature.status,
                steps
                    .iter()
                    .map(|s| (
                        s.step_id.0.clone(),
                        s.status.clone(),
                        s.error_message.clone()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Reduce a terminal run to its transport-independent [`RunViewSnapshot`],
/// reading everything through `RunView` (the single display seam).
async fn snapshot(
    ctx: &AppContext,
    feature_id: &FeatureId,
    terminal_status: String,
    machine_id: &str,
) -> RunViewSnapshot {
    let steps = ctx.run_view.steps(feature_id).expect("steps");
    let mut step_views = Vec::new();
    let mut artifacts = Vec::new();
    for s in &steps {
        step_views.push(StepView {
            step_id: s.step_id.0.clone(),
            kind: s.step_kind.clone(),
            status: s.status.clone(),
        });
        for path in &s.artifact_paths {
            let body = ctx
                .run_view
                .artifact_body(machine_id, path)
                .await
                .unwrap_or_else(|e| format!("<unreadable: {e}>"));
            let base = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_string();
            artifacts.push((s.step_id.0.clone(), base, body));
        }
    }
    step_views.sort_by(|a, b| a.step_id.cmp(&b.step_id));
    artifacts.sort();
    RunViewSnapshot {
        terminal_status,
        steps: step_views,
        artifacts,
    }
}

const REPO_PATH: &str = "demeteo/topology";
const PROVIDER_ID: &str = "topo-provider";

/// A unique app-data dir for one engine instance.
///
/// The counter is what makes it unique; the timestamp only makes it readable.
/// `SystemTime::now()` resolves to the microsecond on macOS, so two tests
/// entering here in the same microsecond used to get the *same* path — and
/// since each leg deletes its dir when it finishes, whichever ended first took
/// the other's database and artifacts with it. Linux resolves to the true
/// nanosecond, so this only ever reproduced off-CI, where the parity gates are
/// run by hand.
fn fresh_app_data_dir(tag: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "demeteo-topology-{tag}-{}-{seq}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create app data dir");
    dir
}

/// Seed a git repo at the project's expected repo dir so `bootstrap_project`
/// detects it and skips its (network) clone — the same "already cloned"
/// shortcut every offline path relies on. Local seeding uses `std::fs` +
/// `git`; remote seeding runs the identical git sequence over the execution
/// port so the fixture lands on the target host.
async fn seed_repo(
    ctx: &AppContext,
    project_id: &str,
    compute_type: &str,
    machine_id: Option<&str>,
) {
    match machine_id {
        None => init_local_repo(&ctx.workspace_dir, project_id, REPO_PATH),
        Some(mid) => {
            let dir = paths::repo_target_dir(
                &ctx.exec,
                compute_type,
                Some(mid),
                project_id,
                REPO_PATH,
                None,
            )
            .await
            .expect("resolve remote repo dir");
            let d = paths::shell_escape_posix(&dir.to_string_lossy());
            let script = format!(
                "set -e; mkdir -p {d}; cd {d}; git init -b main >/dev/null; \
                 git config user.email demeteo@local; git config user.name demeteo; \
                 printf '# topology conformance fixture\\n' > README.md; \
                 git add -A; \
                 git -c user.email=demeteo@local -c user.name=demeteo commit -m seed >/dev/null"
            );
            ctx.exec
                .run_command(mid, &script)
                .await
                .expect("seed remote repo");
        }
    }
}

/// Register the provider, create the project, seed its repo, bootstrap,
/// ingest [`minimal_workflow`], drive one feature to a terminal state, and
/// return the transport-independent snapshot. Shared by every leg — the only
/// difference between transports is `(compute_type, machine_id)` and how the
/// engine was composed. Declared-artifact bodies are always read from the
/// engine host's local `FsArtifactStore` (`machine_id = "local"`), so that
/// read is transport-independent by construction.
async fn run_leg(
    ctx: &AppContext,
    compute_type: &str,
    machine_id: Option<&str>,
) -> RunViewSnapshot {
    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(PROVIDER_ID),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .expect("register provider");

    let project = projects::create(
        ctx,
        projects::ProjectConfig {
            name: "topology".to_string(),
            compute_type: compute_type.to_string(),
            remote_host: machine_id.map(str::to_string),
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    seed_repo(ctx, project.id.as_str(), compute_type, machine_id).await;
    bootstrap::bootstrap_project(ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &minimal_workflow()).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(FeatureLaunch {
            project_id: project.id.0.clone(),
            workflow_id: workflow_id.0.clone(),
            title: "Topology Feature".to_string(),
            description: "Produce a deterministic topology conformance report.".to_string(),
            agent_kind: Some("stub".to_string()),
            ..Default::default()
        })
        .await
        .expect("feature_start");

    let status = poll_terminal(ctx, &feature.id).await;
    for s in ctx
        .features
        .steps_for_feature(&feature.id)
        .unwrap_or_default()
    {
        eprintln!(
            "[topology] step {} status={} error={:?} artifacts={:?}",
            s.step_id.0, s.status, s.error_message, s.artifact_paths
        );
    }
    snapshot(ctx, &feature.id, status, "local").await
}

/// The reference leg: `minimal_workflow` on a locally-executing engine
/// (`ExecutionMode::LocalOnly`). No Docker.
async fn run_local_leg() -> RunViewSnapshot {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = fresh_app_data_dir("local");
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    let snap = run_leg(&ctx, "local", None).await;
    let _ = std::fs::remove_dir_all(&tmp);
    snap
}

/// The local leg alone: proves the deterministic stub agent drives the shared
/// engine to a terminal `completed` state and materializes the declared
/// artifact through `RunView`. This is the reference every containerized leg
/// is compared against, and it runs with no Docker.
#[tokio::test]
async fn topology_local_leg_produces_expected_runview() {
    let snap = run_local_leg().await;

    assert!(
        matches!(snap.terminal_status.as_str(), "completed" | "awaiting_mr"),
        "local leg must reach a success terminal; got {snap:#?}"
    );
    assert_eq!(
        snap.steps,
        vec![StepView {
            step_id: "s-report".to_string(),
            kind: "agent".to_string(),
            status: "completed".to_string(),
        }],
        "the single agent step must complete",
    );
    assert_eq!(
        snap.artifacts.len(),
        1,
        "exactly one declared artifact must be materialized; got {:#?}",
        snap.artifacts
    );
    let (step_id, base, body) = &snap.artifacts[0];
    assert_eq!(step_id, "s-report");
    assert_eq!(base, "topology-report.md");
    assert!(
        body.contains("stub artifact") && body.contains(ARTIFACT_PATH),
        "artifact body must be the deterministic stub body; got: {body:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// C5 — the same workflow, over desktop-over-SSH, must yield an equivalent
// RunView. Reuses the C2.2 loopback sshd container and its
// `DEMETEO_SSH_CONFORMANCE_*` env; gated behind `ssh-conformance` so a
// Docker-less `cargo test` still runs the local leg above.
// ─────────────────────────────────────────────────────────────────────────

/// The SSH leg: the *same* engine, composed with `ExecutionMode::Router` and
/// a single machine pointing at the loopback sshd container, so every git /
/// file / agent operation crosses the SSH boundary. The worktree lives on the
/// container; the declared-artifact body is still materialized into the
/// engine host's local `FsArtifactStore`, which is why the snapshot matches
/// the local leg exactly.
#[cfg(feature = "ssh-conformance")]
async fn run_ssh_leg() -> RunViewSnapshot {
    use crate::domain::ids::MachineId;
    use crate::domain::models::Machine;

    std::env::set_var(STUB_AGENT_ENV, "1");

    let host =
        std::env::var("DEMETEO_SSH_CONFORMANCE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_num: i32 = std::env::var("DEMETEO_SSH_CONFORMANCE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2222);
    let user =
        std::env::var("DEMETEO_SSH_CONFORMANCE_USER").unwrap_or_else(|_| "demeteo".to_string());
    let password = std::env::var("DEMETEO_SSH_CONFORMANCE_PASSWORD")
        .expect("DEMETEO_SSH_CONFORMANCE_PASSWORD must be set for the ssh leg");

    let tmp = fresh_app_data_dir("ssh");
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::Router,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );

    let machine_id = "ssh-topology";
    // Seed the in-process credential cache so the SSH adapter's password
    // lookup never reaches the OS keyring (mirrors the C2.2 leg).
    crate::credential_cache::set(&format!("machine_{machine_id}"), &password);
    ctx.machines
        .add(Machine {
            id: MachineId(machine_id.to_string()),
            name: "ssh-topology".to_string(),
            host,
            port: port_num,
            username: user,
            auth_type: "password".to_string(),
            key_path: None,
            agents: None,
            auto_approved_rules: None,
            use_login_shell: Some(false),
            setup_commands: None,
            notify_webhook_url: None,
        })
        .expect("register machine");

    let snap = run_leg(&ctx, machine_id, Some(machine_id)).await;
    let _ = std::fs::remove_dir_all(&tmp);
    snap
}

/// The cross-transport gate: local and desktop-over-SSH must render the same
/// `RunView` for the same workflow. Removing the C1/C2 adapter parity (e.g.
/// dropping the login-shell/cwd honoring) is what would make this diverge.
#[cfg(feature = "ssh-conformance")]
#[tokio::test]
async fn topology_local_matches_ssh() {
    let local = run_local_leg().await;
    let ssh = run_ssh_leg().await;
    assert_eq!(
        local, ssh,
        "local and SSH transports must produce an equivalent RunView",
    );
}
