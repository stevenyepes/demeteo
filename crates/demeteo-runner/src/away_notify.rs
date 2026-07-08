//! "Runner-push while away" notification channel
//! (docs/REMOTE_EXECUTION_PLAN.md M6.3, design §8). Fired from `run.rs`
//! at terminal/actionable points in a run's lifecycle — failed, parked,
//! needs-credentials, PR-ready — independent of whether a laptop is
//! currently connected: "the moment a run reaches a terminal/actionable
//! state, the runner fires ... so the user learns before reopening."
//!
//! Configured by the `DEMETEO_NOTIFY_WEBHOOK_URL` environment variable
//! on the runner host. The laptop UI (MachinesView → EnvModal's
//! "Notification webhook" field, `Machine.notify_webhook_url`) sets
//! this per-machine and injects it into the systemd unit's environment
//! at install time (`remote_enable_runs`) — no manual shell env var
//! needed. `DEMETEO_NOTIFY_WEBHOOK_URL` is a plain HTTP(S) endpoint that
//! receives a `{"text": "..."}` JSON body — the same minimal shape
//! Slack incoming webhooks and ntfy.sh both accept, so either can be
//! pointed at without a format flag. Email is a bigger lift (SMTP
//! creds, MIME) and is left for a follow-up; unset means silent no-op,
//! same as the M1-M5 behavior this replaces.

use async_trait::async_trait;
use demeteo_core::shared::secret_scrub::scrub_secrets;

#[async_trait]
pub trait AwayNotifier: Send + Sync {
    /// Best-effort — implementations swallow their own errors. A
    /// broken webhook must never fail the run it's reporting on.
    async fn notify(&self, title: &str, body: &str);
}

pub struct NoopAwayNotifier;

#[async_trait]
impl AwayNotifier for NoopAwayNotifier {
    async fn notify(&self, _title: &str, _body: &str) {}
}

pub struct WebhookAwayNotifier {
    url: String,
    client: reqwest::Client,
}

impl WebhookAwayNotifier {
    /// `None` when `DEMETEO_NOTIFY_WEBHOOK_URL` is unset/empty — the
    /// caller falls back to [`NoopAwayNotifier`] in that case.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("DEMETEO_NOTIFY_WEBHOOK_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        Some(Self {
            url,
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl AwayNotifier for WebhookAwayNotifier {
    async fn notify(&self, title: &str, body: &str) {
        // Secret scrubbing (M7.2, §6): the body is often a stringified
        // foreign error (a failed clone/push/PR call) that could echo a
        // credential-bearing URL — scrub before it leaves the host for a
        // webhook we don't control.
        let text = scrub_secrets(&format!("{}\n{}", title, body)).into_owned();
        let result = self
            .client
            .post(&self.url)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await;
        if let Err(e) = result {
            eprintln!("[demeteo-runner] away notification failed: {}", e);
        }
    }
}
