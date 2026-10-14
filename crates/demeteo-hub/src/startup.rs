use crate::ports::KeyServiceError;
use crate::routes::AppState;
use axum::Router;
use std::future::Future;
use std::time::Duration;
use tokio::net::TcpListener;

/// What the startup probe of the key service means for the process. Nothing
/// here ends it: unseal is a manual operator step that can follow the Hub's
/// own start, and a crash-looping Hub would hide that step behind restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Ready,
    Sealed,
    Unavailable,
}

pub fn classify_key_probe<T>(probe: &Result<T, KeyServiceError>) -> KeyStatus {
    match probe {
        Ok(_) => KeyStatus::Ready,
        Err(KeyServiceError::Sealed) => KeyStatus::Sealed,
        Err(_) => KeyStatus::Unavailable,
    }
}

/// Serves `app` on `listener` until `stop` resolves, then asks every socket
/// session of `state` to end and returns once each has left, or once `drain`
/// has passed, whichever is first.
///
/// axum's graceful shutdown waits for the connections it serves, and an
/// upgraded socket is not one of them: `on_upgrade` runs the session in a
/// task of its own, and `serve` stops waiting at the hand-off. A process that
/// returned from here as soon as `serve` did would drop every session at its
/// next await, with no departure written and no close sent, and leave each
/// instance reading as present. So the wait is
/// [`Connections::drained`](crate::socket::connections::Connections::drained),
/// after `serve` has returned: no handler is running then, so no
/// registration can appear behind it.
///
/// A session still there at `drain` is left to be dropped with the process,
/// and only how many is logged.
pub async fn serve_until_stopped<F>(
    listener: TcpListener,
    app: Router,
    state: AppState,
    stop: F,
    drain: Duration,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let sessions = state.clone();
    // A socket does not end by itself, and a graceful shutdown waits for no
    // socket: unasked, each session would run until the process is gone.
    let stopping = async move {
        stop.await;
        sessions.connections().close_all();
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(stopping)
        .await?;
    let connections = state.connections();
    if tokio::time::timeout(drain, connections.drained())
        .await
        .is_err()
    {
        tracing::warn!(
            sessions = connections.alive(),
            ?drain,
            "stopping with socket sessions that had not left"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_seal_is_reported_as_sealed() {
        assert_eq!(classify_key_probe(&Ok(())), KeyStatus::Ready);
        assert_eq!(
            classify_key_probe::<()>(&Err(KeyServiceError::Sealed)),
            KeyStatus::Sealed
        );
        for err in [
            KeyServiceError::UnknownKey("k".into()),
            KeyServiceError::InvalidKeyName("a/b".into()),
            KeyServiceError::Transport("down".into()),
            KeyServiceError::Upstream {
                status: 403,
                detail: String::new(),
            },
        ] {
            assert_eq!(classify_key_probe::<()>(&Err(err)), KeyStatus::Unavailable);
        }
    }
}
