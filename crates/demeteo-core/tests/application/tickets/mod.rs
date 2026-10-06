// Tests extracted from `crates/demeteo-core/src/application/tickets/mod.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::{Arc, Mutex};

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{DiscoveryId, FeatureId, MachineId, ProjectId, TicketId};
use crate::domain::models::{Machine, Project};
use crate::domain::run_placement::RunPlacement;
use crate::domain::ticket_graph::TicketLane;
use crate::ports::remote_run_mirror::RemoteRunMirrorPort;

fn ticket(id: &str, seq: i64) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// §6.5 makes the recorded reason the whole content of a force start, so a row
/// carrying only whitespace must not read as one — the bypass would then reach
/// the agent's briefing with nothing to say for itself.
#[test]
fn a_blank_force_reason_is_not_a_force_start() {
    let mut t = ticket("t-1", 1);
    t.force_start_reason = Some("   ".to_string());
    assert!(!node_of(&t, None).force_started);

    t.force_start_reason = Some("no forge remote".to_string());
    assert!(node_of(&t, None).force_started);
}

/// The projection carries `mr_state` through verbatim; the derived layer owns
/// what the word means.
#[test]
fn the_projection_carries_the_forge_state_verbatim() {
    let mut t = ticket("t-1", 1);
    t.state = TicketState::Started;
    t.feature_id = Some(FeatureId::from("f-1".to_string()));
    let node = node_of(&t, Some("closed"));
    assert_eq!(node.state, TicketNodeState::Started);
    assert_eq!(node.mr_state.as_deref(), Some("closed"));
}

/// §8.4: a Ticket with a Feature owns a branch, a worktree and a PR that
/// outlive the plan, so the whole Discovery is held.
#[test]
fn a_discovery_with_a_started_ticket_refuses_deletion() {
    let mut started = ticket("t-2", 2);
    started.feature_id = Some(FeatureId::from("f-1".to_string()));
    let refusal = deletion_refusal(&[ticket("t-1", 1), started])
        .expect("a started ticket should hold the discovery");
    assert!(refusal.contains("#2"), "{refusal}");
    assert!(!refusal.contains("#1"), "{refusal}");
}

#[test]
fn a_discovery_of_unstarted_tickets_may_go() {
    assert!(deletion_refusal(&[ticket("t-1", 1), ticket("t-2", 2)]).is_none());
}

/// The two spellings of "has a Feature" are one answer, and either locks. A
/// row whose `state` this build could not name reads as `started` on purpose;
/// consulting `feature_id` alone would hand it back as editable.
#[test]
fn a_started_row_with_no_feature_still_locks() {
    let mut t = ticket("t-1", 1);
    t.state = TicketState::Started;
    assert!(is_locked(&t));

    let mut t = ticket("t-2", 2);
    t.state = TicketState::Dropped;
    assert!(!is_locked(&t));
}

/// A stored Discovery holding one stored ticket, in a project whose compute is
/// `remote_host` (local when `None`). The Discovery is opened against the
/// project's own host unless `discovery_machine` names another.
fn stored_board(
    tag: &str,
    remote_host: Option<&str>,
    discovery_machine: Option<&str>,
    ticket_machine: Option<&str>,
) -> (AppContext, Ticket) {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-ticket-board-{tag}"));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    for id in ["gpu-box", "build-host"] {
        ctx.machines
            .add(Machine {
                id: MachineId::from(id.to_string()),
                name: id.to_string(),
                host: format!("{id}.internal"),
                port: 22,
                username: "demeteo".to_string(),
                auth_type: "key".to_string(),
                key_path: None,
                agents: None,
                auto_approved_rules: None,
                use_login_shell: Some(false),
                setup_commands: None,
                notify_webhook_url: None,
            })
            .expect("the machine is stored");
    }
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "name fixture".to_string(),
            compute_type: if remote_host.is_some() {
                "remote"
            } else {
                "local"
            }
            .to_string(),
            remote_host: remote_host.map(|h| MachineId::from(h.to_string())),
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    let discovery = open_discovery(
        &ctx,
        NewDiscovery {
            project_id: project_id.as_str().to_string(),
            title: "placement".to_string(),
            agent_kind: "claude-code".to_string(),
            model: None,
            effort: None,
            machine_id: discovery_machine.map(str::to_string),
            staged_attachments: Vec::new(),
        },
    )
    .expect("the discovery opens");
    let mut t = ticket("t-1", 1);
    t.discovery_id = discovery.id;
    t.machine_id = ticket_machine.map(|m| MachineId::from(m.to_string()));
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    (ctx, t)
}

fn only_view(ctx: &AppContext, t: &Ticket) -> TicketView {
    let mut views = board(ctx, &t.discovery_id)
        .expect("the board reads")
        .tickets;
    assert_eq!(views.len(), 1);
    views.remove(0)
}

#[tokio::test]
async fn a_ticket_on_a_local_discovery_inherits_local() {
    let (ctx, t) = stored_board("local", None, None, None);
    let view = only_view(&ctx, &t);
    assert_eq!(view.placement.placement, RunPlacement::Local);
    assert!(view.placement.inherited);
    assert_eq!(
        resolve_ticket_placement(&ctx, &t, None).expect("the placement resolves"),
        view.placement
    );
}

#[tokio::test]
async fn a_stored_machine_is_detached_and_not_inherited() {
    let (ctx, t) = stored_board("stored", None, None, Some("gpu-box"));
    let view = only_view(&ctx, &t);
    assert_eq!(
        view.placement.placement,
        RunPlacement::Detached {
            machine_id: MachineId::from("gpu-box".to_string())
        }
    );
    assert!(!view.placement.inherited);
}

/// `domain/run_placement.rs` keeps a remote project's own host attached.
#[tokio::test]
async fn a_remote_projects_own_host_resolves_local() {
    let (ctx, t) = stored_board("own-host", Some("build-host"), None, None);
    let view = only_view(&ctx, &t);
    assert_eq!(view.placement.placement, RunPlacement::Local);
    assert!(view.placement.inherited);
}

#[tokio::test]
async fn a_discovery_on_another_machine_defaults_detached_and_an_override_wins() {
    let (ctx, t) = stored_board("elsewhere", None, Some("gpu-box"), None);
    assert_eq!(
        only_view(&ctx, &t).placement.placement,
        RunPlacement::Detached {
            machine_id: MachineId::from("gpu-box".to_string())
        }
    );
    let overridden =
        resolve_ticket_placement(&ctx, &t, Some(&MachineId::from("local".to_string())))
            .expect("the placement resolves");
    assert_eq!(overridden.placement, RunPlacement::Local);
    assert!(!overridden.inherited);
}

/// Every ticket here holds its own choice, so the only place the Discovery's
/// default can still be read from is the board itself.
#[tokio::test]
async fn the_board_carries_the_discovery_default_when_no_ticket_inherits() {
    let (ctx, t) = stored_board("default-elsewhere", None, Some("gpu-box"), Some("local"));
    let read = board(&ctx, &t.discovery_id).expect("the board reads");
    assert!(!read.tickets[0].placement.inherited);
    assert_eq!(
        read.discovery_default,
        RunPlacement::Detached {
            machine_id: MachineId::from("gpu-box".to_string())
        }
    );

    let (ctx, t) = stored_board("default-local", None, None, Some("gpu-box"));
    assert_eq!(
        board(&ctx, &t.discovery_id)
            .expect("the board reads")
            .discovery_default,
        RunPlacement::Local
    );

    let (ctx, t) = stored_board(
        "default-own-host",
        Some("build-host"),
        None,
        Some("gpu-box"),
    );
    assert_eq!(
        board(&ctx, &t.discovery_id)
            .expect("the board reads")
            .discovery_default,
        RunPlacement::Local
    );
}

/// A Local run executes on the project's compute, which is not where the
/// Discovery was opened when its interview ran on another machine.
#[tokio::test]
async fn the_board_names_the_host_a_local_run_executes_on() {
    let (ctx, t) = stored_board("local-host-desktop", None, Some("gpu-box"), None);
    let read = board(&ctx, &t.discovery_id).expect("the board reads");
    assert_eq!(read.local_host, MachineId::from("local".to_string()));

    let (ctx, t) = stored_board(
        "local-host-remote",
        Some("build-host"),
        Some("gpu-box"),
        None,
    );
    let read = board(&ctx, &t.discovery_id).expect("the board reads");
    assert_eq!(read.local_host, MachineId::from("build-host".to_string()));
}

/// The mirror is shown beside the Feature, never read for the lane: the
/// runner reporting `completed` does not land a ticket whose PR is unmerged.
#[tokio::test]
async fn the_ticket_features_mirror_row_is_exposed_without_moving_the_lane() {
    let (mut ctx, mut t) = stored_board("mirror", None, None, Some("gpu-box"));
    let project_id = ctx
        .discoveries
        .get(&t.discovery_id)
        .expect("the discovery reads")
        .expect("the discovery exists")
        .project_id;
    ctx.features
        .add(Feature {
            id: FeatureId::from("f-shadow".to_string()),
            project_id: project_id.clone(),
            workflow_id: None,
            workflow_version_id: None,
            title: "shadow".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("open".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .expect("the feature is stored");
    t.state = TicketState::Started;
    t.feature_id = Some(FeatureId::from("f-shadow".to_string()));
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is started");
    ctx.remote_run_mirror
        .upsert_submitted(
            "gpu-box",
            "run-1",
            Some(project_id.as_str()),
            Some("f-shadow"),
            "shadow",
            1,
        )
        .expect("the mirror row is stored");
    ctx.remote_run_mirror
        .update_status(
            "gpu-box",
            "run-1",
            "completed",
            None,
            Some("f-shadow"),
            None,
            None,
            0,
            2,
        )
        .expect("the mirror status is stored");
    ctx.remote_run_mirror
        .upsert_submitted("gpu-box", "run-other", None, Some("f-other"), "other", 3)
        .expect("an unrelated mirror row is stored");

    let mirror = ScopedOnlyMirror::wrap(&mut ctx);
    let view = only_view(&ctx, &t);
    assert_eq!(mirror.asked(), [vec!["f-shadow".to_string()]]);
    let feature = view.feature.expect("the started ticket has its feature");
    let remote = feature.remote.expect("the mirror row is exposed");
    assert_eq!(remote.machine_id, "gpu-box");
    assert_eq!(remote.run_id, "run-1");
    assert_eq!(remote.status, "completed");
    assert_eq!(view.standing.lane, TicketLane::InFlight);
}

/// `t` started as a Feature `f-started`, recorded as placed at `placement`
/// unless `placement` is `None` — the row a start before V61 left behind.
fn start_stored(ctx: &AppContext, t: &mut Ticket, placement: Option<&RunPlacement>) {
    let project_id = ctx
        .discoveries
        .get(&t.discovery_id)
        .expect("the discovery reads")
        .expect("the discovery exists")
        .project_id;
    ctx.features
        .add(Feature {
            id: FeatureId::from("f-started".to_string()),
            project_id,
            workflow_id: None,
            workflow_version_id: None,
            title: "started".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("open".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .expect("the feature is stored");
    let feature_id = FeatureId::from("f-started".to_string());
    match placement {
        Some(placement) => ctx
            .tickets
            .record_start(&t.id, &feature_id, placement, 1)
            .expect("the start is recorded"),
        None => {
            t.state = TicketState::Started;
            t.feature_id = Some(feature_id.clone());
            ctx.tickets
                .upsert_batch(std::slice::from_ref(t))
                .expect("the ticket is started");
        }
    }
    t.state = TicketState::Started;
    t.feature_id = Some(feature_id);
}

/// The degraded start `mirror_unrecorded` reports: the runner took the run,
/// the mirror never recorded it. The attempt is still where it went — not
/// "local" for want of a mirror row.
#[tokio::test]
async fn a_detached_attempt_with_no_mirror_row_is_still_placed_detached() {
    let (ctx, mut t) = stored_board("unmirrored", None, None, None);
    let gpu_box = RunPlacement::Detached {
        machine_id: MachineId::from("gpu-box".to_string()),
    };
    start_stored(&ctx, &mut t, Some(&gpu_box));

    let feature = only_view(&ctx, &t).feature.expect("the attempt is shown");
    assert!(feature.remote.is_none());
    assert_eq!(feature.placement, Some(gpu_box));
}

#[tokio::test]
async fn an_attempt_with_no_recorded_placement_is_unknown_not_local() {
    let (ctx, mut t) = stored_board("unplaced", None, None, None);
    start_stored(&ctx, &mut t, None);

    let feature = only_view(&ctx, &t).feature.expect("the attempt is shown");
    assert_eq!(feature.placement, None);
}

/// The mirror only feeds a display chip, so losing it must not blank a board
/// whose lanes were already derived from the Features.
#[tokio::test]
async fn a_failed_mirror_read_degrades_the_chip_not_the_board() {
    let (mut ctx, mut t) = stored_board("mirror-down", None, None, None);
    start_stored(&ctx, &mut t, Some(&RunPlacement::Local));
    let mirror = ScopedOnlyMirror::wrap(&mut ctx);
    mirror.fail();

    let feature = only_view(&ctx, &t).feature.expect("the attempt is shown");
    assert!(feature.remote.is_none());
    assert_eq!(feature.placement, Some(RunPlacement::Local));
}

#[tokio::test]
async fn a_board_with_no_attempts_reads_no_mirror_rows() {
    let (mut ctx, t) = stored_board("no-attempts", None, None, None);
    let mirror = ScopedOnlyMirror::wrap(&mut ctx);
    only_view(&ctx, &t);
    assert!(mirror.asked().is_empty());
}

/// The board renders without the project row; a launch, which needs the
/// project's compute to place the run, still refuses.
#[tokio::test]
async fn the_board_renders_from_the_discoverys_machine_without_its_project_row() {
    let (ctx, t) = stored_board("orphaned", Some("build-host"), Some("gpu-box"), None);
    let project_id = ctx
        .discoveries
        .get(&t.discovery_id)
        .expect("the discovery reads")
        .expect("the discovery exists")
        .project_id;
    // With foreign keys off the Discovery survives its project row instead
    // of cascading away with it.
    rusqlite::Connection::open(ctx.app_data_dir.join("demeteo.db"))
        .expect("the database opens")
        .execute_batch(&format!(
            "PRAGMA foreign_keys = OFF; DELETE FROM projects WHERE id = '{}';",
            project_id.as_str()
        ))
        .expect("the project row is deleted");
    assert!(ctx
        .projects
        .get_project(&project_id)
        .expect("the project reads")
        .is_none());

    let read = board(&ctx, &t.discovery_id).expect("the board reads");
    let gpu_box = RunPlacement::Detached {
        machine_id: MachineId::from("gpu-box".to_string()),
    };
    assert_eq!(read.discovery_default, gpu_box);
    assert_eq!(read.tickets[0].placement.placement, gpu_box);
    assert_eq!(read.local_host, MachineId::from("gpu-box".to_string()));
    assert!(read.tickets[0].placement.inherited);

    assert!(resolve_ticket_placement(&ctx, &t, None).is_err());
}

/// The context's own mirror, except that the unscoped `list()` panics — a
/// board read must ask only for its own attempts' features.
struct ScopedOnlyMirror {
    inner: Arc<dyn RemoteRunMirrorPort>,
    asked: Mutex<Vec<Vec<String>>>,
    failing: std::sync::atomic::AtomicBool,
}

impl ScopedOnlyMirror {
    fn wrap(ctx: &mut AppContext) -> Arc<Self> {
        let mirror = Arc::new(Self {
            inner: ctx.remote_run_mirror.clone(),
            asked: Mutex::new(Vec::new()),
            failing: std::sync::atomic::AtomicBool::new(false),
        });
        ctx.remote_run_mirror = mirror.clone();
        mirror
    }

    /// Every scoped read errors from here on.
    fn fail(&self) {
        self.failing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn asked(&self) -> Vec<Vec<String>> {
        self.asked.lock().expect("the call log locks").clone()
    }
}

impl RemoteRunMirrorPort for ScopedOnlyMirror {
    fn upsert_submitted(
        &self,
        machine_id: &str,
        run_id: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        title: &str,
        now: i64,
    ) -> Result<RemoteRunMirror, String> {
        self.inner
            .upsert_submitted(machine_id, run_id, project_id, feature_id, title, now)
    }
    fn update_status(
        &self,
        machine_id: &str,
        run_id: &str,
        status: &str,
        error: Option<&str>,
        feature_id: Option<&str>,
        pr_url: Option<&str>,
        pushed_branch: Option<&str>,
        last_offset: i64,
        now: i64,
    ) -> Result<(), String> {
        self.inner.update_status(
            machine_id,
            run_id,
            status,
            error,
            feature_id,
            pr_url,
            pushed_branch,
            last_offset,
            now,
        )
    }
    fn mark_notified(&self, machine_id: &str, run_id: &str, status: &str) -> Result<(), String> {
        self.inner.mark_notified(machine_id, run_id, status)
    }
    fn delete_for_feature(&self, feature_id: &str) -> Result<(), String> {
        self.inner.delete_for_feature(feature_id)
    }
    fn get(&self, machine_id: &str, run_id: &str) -> Result<Option<RemoteRunMirror>, String> {
        self.inner.get(machine_id, run_id)
    }
    fn list(&self) -> Result<Vec<RemoteRunMirror>, String> {
        panic!("a board read scanned every mirrored run")
    }
    fn list_for_features(&self, feature_ids: &[&str]) -> Result<Vec<RemoteRunMirror>, String> {
        self.asked
            .lock()
            .expect("the call log locks")
            .push(feature_ids.iter().map(|id| id.to_string()).collect());
        if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("database is locked".to_string());
        }
        self.inner.list_for_features(feature_ids)
    }
}
