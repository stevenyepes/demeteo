use demeteo_hub::adapters::openbao::{http_client, OpenBaoKeys};
use demeteo_hub::config::Config;
use demeteo_hub::ports::KeyService;
use demeteo_hub::startup::{classify_key_probe, KeyStatus};
use demeteo_hub::{routes, store};
use std::process::ExitCode;
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
    let keys = OpenBaoKeys::new(
        http,
        &config.openbao_addr,
        &config.transit_mount,
        &config.openbao_token,
    )
    .map_err(|e| e.to_string())?;

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|e| format!("cannot bind {}: {e}", config.bind))?;
    tracing::info!(addr = %config.bind, "listening");
    // Spawned after bind, never awaited: a black-holed OpenBao would otherwise
    // hold /healthz back for up to OPENBAO_TOTAL_TIMEOUT.
    tokio::spawn(probe_key_service(keys, config.transit_key.clone()));
    axum::serve(listener, routes::router(&config.web_dir))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| e.to_string())
}

async fn probe_key_service(keys: OpenBaoKeys, transit_key: String) {
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
