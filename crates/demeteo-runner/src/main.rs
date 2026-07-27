//! Headless runner (docs/REMOTE_EXECUTION.md M1/M3):
//!
//! ```text
//! demeteo-runner submit <spec.json>   # one-shot, in-process (M1)
//! demeteo-runner serve                # long-lived daemon + control RPC (M3)
//! ```
//!
//! Both build the same engine the desktop app uses
//! (`demeteo_core::composition::build_core_context`) with
//! `ExecutionMode::LocalOnly` (no nested SSH — the runner *is* the
//! machine, docs/REMOTE_EXECUTION.md §3) and a `RunEventBridge`
//! `NotificationPort` that mirrors the engine's live `DomainEvent` stream
//! into the run event log (`notify_bridge`).
//! `run` holds the actual "bootstrap project, run workflow, push branch"
//! pipeline (R3), shared by both entry points.
//!
//! `serve` is the long-lived process a systemd `--user` unit (M2.1)
//! supervises: it starts the control-RPC listener (M3.1) and keeps the
//! engine's background tasks (scheduler, MR monitor, memory worker,
//! gate/driver resume) running indefinitely.

mod away_notify;
mod credentials;
mod git_askpass;
mod notify_bridge;
mod reconcile;
mod rpc;
mod run;
mod services;

use away_notify::{AwayNotifier, NoopAwayNotifier, WebhookAwayNotifier};
use credentials::CredentialStore;
use demeteo_core::composition::{build_core_context, CoreConfig, ExecutionMode};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use notify_bridge::RunEventBridge;
use services::RunnerServices;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Version reported by `--version`. CI sets `DEMETEO_RUNNER_VERSION` at
/// build time to the exact same version string the desktop app reports
/// (`app.package_info().version`, e.g. `"0.1.0-45"` nightly or `"0.2.0"`
/// stable) so the laptop can compare its own version against a locally
/// cached runner build with a plain string equality check. Local `cargo
/// build` runs (no CI env var set) fall back to the crate's own version.
const VERSION: &str = match option_env!("DEMETEO_RUNNER_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

fn runner_data_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("demeteo-runner")
}

/// Create the runner's data dir with `0700` perms **before** anything is
/// created inside it (the SQLite DB, the askpass helper, and — critically
/// — the `0600` control socket).
///
/// M3.1's whole authz model is "no other local user can reach the control
/// socket". `rpc::serve` binds the socket and *then* chmods it to `0600`,
/// a narrow TOCTOU window where the socket briefly carries umask perms.
/// An owner-only parent directory closes that window structurally — no
/// other uid can traverse into the dir to reach the socket at all,
/// whatever the socket's own mode is mid-bind — and also keeps the run
/// event log / spec DB unreadable by other local users (defence in depth
/// on top of the M7.2 secret scrubbing, §6).
fn ensure_private_data_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut perms = std::fs::metadata(dir)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o700);
    std::fs::set_permissions(dir, perms)
}

fn print_usage() {
    eprintln!("usage: demeteo-runner serve | demeteo-runner submit <spec.json> | demeteo-runner --version");
}

/// Enrich the process `PATH` from a login + interactive shell so agent
/// binaries installed via a developer tool-manager (mise/asdf/nvm) — whose
/// shims are activated in `~/.bashrc`, behind the interactive guard — are
/// resolvable both by the daemon's own PATH lookups
/// (`is_binary_on_local_path`, used by the M4.1 agent-readiness precondition)
/// and by every agent process it spawns (`Command::new(binary)` in the local
/// execution adapter).
///
/// This makes the runner self-sufficient regardless of how it was launched.
/// The systemd unit wraps `ExecStart` in `bash -lic` (belt), but a runner
/// started manually, by an older unit still on disk, or on a distro where an
/// interactive `ExecStart` misbehaves would otherwise inherit systemd's
/// minimal PATH and reject every run with "agent 'opencode' is not
/// installed/available". Mirrors the laptop-side
/// `ShellOptions::login_interactive` probe so both sides resolve the same
/// PATH ("available", "runnable", and "runner-launched" agree).
///
/// Called once at startup, before the tokio runtime is built, so the env
/// mutation happens while the process is still single-threaded. Best-effort:
/// any probe failure leaves `PATH` untouched.
fn enrich_path_from_login_shell() {
    let output = std::process::Command::new("bash")
        .args(["-lic", "printf %s \"$PATH\""])
        .output();
    let login_path = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(o) => {
            eprintln!(
                "[demeteo-runner] login-shell PATH probe exited {}; leaving PATH unchanged",
                o.status
            );
            return;
        }
        Err(e) => {
            eprintln!(
                "[demeteo-runner] login-shell PATH probe failed ({e}); leaving PATH unchanged"
            );
            return;
        }
    };
    if login_path.is_empty() {
        return;
    }
    // Union with the existing PATH: login-shell entries first (they carry the
    // tool-manager shims), then any pre-existing entries not already present,
    // so we never drop a path systemd or the environment supplied.
    let current = std::env::var("PATH").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut merged: Vec<&str> = Vec::new();
    for entry in login_path.split(':').chain(current.split(':')) {
        if !entry.is_empty() && seen.insert(entry) {
            merged.push(entry);
        }
    }
    let new_path = merged.join(":");
    if new_path != current {
        eprintln!(
            "[demeteo-runner] PATH enriched from login shell ({} entries)",
            merged.len()
        );
        // Safe: still single-threaded here (called before the tokio runtime
        // and any thread spawns in `main`).
        std::env::set_var("PATH", new_path);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // M7.1: the laptop's "Enable remote runs" flow probes this to
        // decide install (missing/unparseable output) vs. upgrade
        // (version mismatch against the running app's own version).
        Some("--version") | Some("-V") => {
            println!("demeteo-runner {}", VERSION);
        }
        Some("submit") => {
            let Some(spec_path) = args.get(2) else {
                print_usage();
                std::process::exit(2);
            };
            enrich_path_from_login_shell();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let exit_code = rt.block_on(submit(spec_path, rt.handle().clone()));
            std::process::exit(exit_code);
        }
        Some("serve") => {
            enrich_path_from_login_shell();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let handle = rt.handle().clone();
            let exit_code = rt.block_on(serve(handle));
            std::process::exit(exit_code);
        }
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}

async fn serve(runtime: tokio::runtime::Handle) -> i32 {
    let app_data_dir = runner_data_dir();
    eprintln!("[demeteo-runner] data dir: {}", app_data_dir.display());
    if let Err(e) = ensure_private_data_dir(&app_data_dir) {
        eprintln!("[demeteo-runner] failed to secure data dir (0700): {}", e);
        return 1;
    }

    // `build_core_context` already runs the engine's own restart
    // reconciliation for individual features/steps (`startup_watchdog` +
    // `resume_interrupted_features`) before returning. `reconcile_on_startup`
    // below is the M2.3 layer on top: it re-attaches runner_runs rows
    // (this daemon's own submit/status mirror) to whatever the engine
    // just resumed, under a bounded reboot-retry budget.
    // Bridge the engine's live `DomainEvent` stream into the run event log
    // so remote runs report per-step progress / retries / failures, not just
    // coarse lifecycle events. Wired *after* the context is built (it needs
    // the `run_events`/`runner_runs` ports the build constructs); until then
    // it drops events exactly as the old noop adapter did.
    let notify_bridge = Arc::new(RunEventBridge::new());
    let ctx = Arc::new(build_core_context(
        CoreConfig {
            app_data_dir: app_data_dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        notify_bridge.clone(),
        runtime,
    ));
    notify_bridge.wire(ctx.run_events.clone(), ctx.runner_runs.clone());
    let askpass_path = git_askpass::ensure_askpass_script(&app_data_dir)
        .unwrap_or_else(|e| panic!("failed to write askpass helper: {}", e));
    let away_notifier: Arc<dyn AwayNotifier> = match WebhookAwayNotifier::from_env() {
        Some(n) => Arc::new(n),
        None => Arc::new(NoopAwayNotifier),
    };
    let svc = Arc::new(RunnerServices {
        ctx: ctx.clone(),
        creds: Arc::new(CredentialStore::new()),
        askpass_path,
        away_notifier,
    });
    reconcile::reconcile_on_startup(svc.clone()).await;

    let socket_path = rpc::socket_path(&app_data_dir);
    let rpc_svc = svc.clone();
    let rpc_task = tokio::spawn(async move { rpc::serve(rpc_svc, socket_path).await });

    // M2.2: on SIGTERM (`systemctl --user stop`) or SIGINT (Ctrl-C),
    // mark every in-flight runner_runs row `interrupted` before exiting
    // so `list_runs`/`get_status` reflect reality immediately rather than
    // showing a stale `running` until the next restart's reconciliation
    // catches up. The underlying feature/step state is handled separately
    // by the engine's own crash-consistent (WAL-mode) SQLite — this is
    // only about the runner's own run-mirror table.
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[demeteo-runner] failed to install SIGTERM handler: {}", e);
            return 1;
        }
    };

    tokio::select! {
        res = rpc_task => {
            eprintln!("[demeteo-runner] RPC server task ended: {:?}", res);
            1
        }
        _ = sigterm.recv() => {
            eprintln!("[demeteo-runner] SIGTERM received, marking in-flight runs interrupted");
            if let Err(e) = ctx.runner_runs.mark_all_running_interrupted(paths::now_ms()) {
                eprintln!("[demeteo-runner] failed to mark runs interrupted: {}", e);
            }
            0
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("[demeteo-runner] SIGINT received, marking in-flight runs interrupted");
            if let Err(e) = ctx.runner_runs.mark_all_running_interrupted(paths::now_ms()) {
                eprintln!("[demeteo-runner] failed to mark runs interrupted: {}", e);
            }
            0
        }
    }
}

async fn submit(spec_path: &str, runtime: tokio::runtime::Handle) -> i32 {
    let spec_json = match std::fs::read_to_string(spec_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[demeteo-runner] could not read {}: {}", spec_path, e);
            return 1;
        }
    };
    let spec: RunSpec = match serde_json::from_str(&spec_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[demeteo-runner] invalid run spec: {}", e);
            return 1;
        }
    };

    let app_data_dir = runner_data_dir();
    eprintln!("[demeteo-runner] data dir: {}", app_data_dir.display());
    if let Err(e) = ensure_private_data_dir(&app_data_dir) {
        eprintln!("[demeteo-runner] failed to secure data dir (0700): {}", e);
        return 1;
    }

    let notify_bridge = Arc::new(RunEventBridge::new());
    let ctx = Arc::new(build_core_context(
        CoreConfig {
            app_data_dir: app_data_dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        notify_bridge.clone(),
        runtime,
    ));
    notify_bridge.wire(ctx.run_events.clone(), ctx.runner_runs.clone());
    let askpass_path = git_askpass::ensure_askpass_script(&app_data_dir)
        .unwrap_or_else(|e| panic!("failed to write askpass helper: {}", e));
    let creds = Arc::new(CredentialStore::new());

    // No RPC client submitted this, so there's no laptop-generated
    // run_id (M3.2) to key the event log by — mint one so `submit`'s run
    // still gets one (`stream_events` can tail it just like an RPC run).
    let run_id = format!("cli-{}", paths::new_id());

    // No `inject_credentials` RPC call is coming either (no laptop in
    // the loop for a one-shot CLI submit) — bridge the env var into the
    // in-memory store for just this run so the same askpass path serves
    // both entry points.
    if let Ok(pat) = std::env::var(run::GIT_PAT_ENV) {
        creds.insert(&run_id, pat);
    }

    let away_notifier: Arc<dyn AwayNotifier> = match WebhookAwayNotifier::from_env() {
        Some(n) => Arc::new(n),
        None => Arc::new(NoopAwayNotifier),
    };
    let svc = Arc::new(RunnerServices {
        ctx,
        creds,
        askpass_path,
        away_notifier,
    });
    match run::execute_run(&svc, &run_id, &spec).await {
        Ok(outcome) if outcome.pushed_branch.is_some() => 0,
        Ok(outcome) => {
            eprintln!(
                "[demeteo-runner] run ended in non-success state: {}",
                outcome.status
            );
            1
        }
        Err(e) => {
            eprintln!("[demeteo-runner] run failed: {}", e);
            1
        }
    }
}
