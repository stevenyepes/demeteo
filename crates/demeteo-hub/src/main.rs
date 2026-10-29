use demeteo_hub::adapters::openbao::{http_client, OpenBaoKeys};
use demeteo_hub::attachments::postgres::PostgresAttachments;
use demeteo_hub::attachments::sweep;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::config::Config;
use demeteo_hub::ports::KeyService;
use demeteo_hub::socket::SHUTDOWN_DRAIN_SECS;
use demeteo_hub::startup::{classify_key_probe, serve_until_stopped, KeyStatus};
use demeteo_hub::{routes, store};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const OPENBAO_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OPENBAO_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = Config::from_env().map_err(|e| e.to_string())?;
    tracing::info!(?config, "loaded configuration");

    let db = store::connect(&config.database_url)
        .await
        .map_err(|e| e.to_string())?;
    store::migrate(&db).await.map_err(|e| e.to_string())?;

    let http =
        http_client(OPENBAO_CONNECT_TIMEOUT, OPENBAO_TOTAL_TIMEOUT).map_err(|e| e.to_string())?;
    let keys: Arc<dyn KeyService> = Arc::new(
        OpenBaoKeys::new(
            http,
            &config.openbao_addr,
            &config.transit_mount,
            &config.openbao_token,
        )
        .map_err(|e| e.to_string())?,
    );

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|e| format!("cannot bind {}: {e}", config.bind))?;
    tracing::info!(addr = %config.bind, "listening");
    // Spawned after bind, never awaited: a black-holed OpenBao would otherwise
    // hold /healthz back for up to OPENBAO_TOTAL_TIMEOUT.
    tokio::spawn(probe_key_service(
        Arc::clone(&keys),
        config.transit_key.clone(),
    ));
    let attachments = PostgresAttachments::new(db.clone());
    let sweep_secs = config.attachments.sweep_secs;
    tokio::spawn(async move { sweep::run(&attachments, sweep_secs).await });
    let rp = RelyingParty {
        rp_id: config.rp_id,
        origin: config.origin,
    };
    let state = routes::AppState::new(
        db,
        rp,
        keys,
        config.signing_key,
        config.transit_key,
        config.attachments,
    )
    .map_err(|e| e.to_string())?;
    let app = routes::router(state.clone(), &config.web_dir);
    serve_until_stopped(
        listener,
        app,
        state,
        shutdown_signal(),
        Duration::from_secs(SHUTDOWN_DRAIN_SECS),
    )
    .await
    .map_err(|e| e.to_string())
}

async fn probe_key_service(keys: Arc<dyn KeyService>, transit_key: String) {
    let probe = keys.encrypt(&transit_key, b"startup-probe").await;
    match classify_key_probe(&probe) {
        KeyStatus::Ready => tracing::info!("key service is ready"),
        KeyStatus::Sealed => {
            tracing::warn!("OpenBao is sealed; serving until an operator unseals it")
        }
        KeyStatus::Unavailable => {
            tracing::warn!(error = ?probe.err(), "key service is not usable yet; serving anyway")
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
