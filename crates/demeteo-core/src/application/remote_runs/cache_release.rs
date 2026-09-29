//! Delivering `release_feature_cache` to the runner that owns a run, and
//! keeping what could not be delivered. See
//! [`crate::domain::runner_cache_release`].

use super::rpc::remote_rpc_via;
use crate::domain::runner_cache_release::{
    with_pending, worth_retrying, CacheReleaseReason, PendingRunnerRelease,
};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::remote_run_mirror::RunnerCachePort;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub struct RunnerCacheRpc {
    pub exec: Arc<dyn ExecutionPort>,
    pub app_settings: Arc<dyn AppSettingsRepository>,
}

#[async_trait]
impl RunnerCachePort for RunnerCacheRpc {
    async fn release_feature_cache(
        &self,
        machine_id: &str,
        run_id: &str,
        reason: CacheReleaseReason,
    ) -> Result<(), String> {
        remote_rpc_via(
            &*self.exec,
            &*self.app_settings,
            machine_id,
            "release_feature_cache",
            serde_json::json!({ "run_id": run_id, "reason": reason }),
        )
        .await
        .map(|_| ())
    }
}

const PENDING_KEY: &str = "pending_runner_cache_releases";

/// Serialises the queue's read-modify-write between the MR monitor's task, a
/// cleanup command and a reconcile. Never held across an `await`.
static PENDING_LOCK: Mutex<()> = Mutex::new(());

/// Send `entry`, and keep it for [`retry_pending_runner_releases`] when it
/// failed in a way a later try could fix. The failure is still returned: the
/// cache has not been released yet.
pub async fn release_on_runner(
    runner: &dyn RunnerCachePort,
    settings: &dyn AppSettingsRepository,
    entry: PendingRunnerRelease,
) -> Result<(), String> {
    let error = match runner
        .release_feature_cache(&entry.machine_id, &entry.run_id, entry.reason)
        .await
    {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !worth_retrying(&error) {
        return Err(error);
    }
    match update_pending(settings, |queue| with_pending(queue, entry)) {
        Ok(()) => Err(format!("{error} (will retry on the next reconcile)")),
        Err(queue_error) => Err(format!(
            "{error}; could not keep it to retry: {queue_error}"
        )),
    }
}

/// Re-send every release a runner could not take earlier, dropping each one
/// that lands or that the runner will never accept.
pub async fn retry_pending_runner_releases(
    runner: &dyn RunnerCachePort,
    settings: &dyn AppSettingsRepository,
) -> Result<(), String> {
    let queue = {
        let _lock = PENDING_LOCK.lock().map_err(|e| e.to_string())?;
        read_pending(settings)?
    };
    for entry in queue {
        let done = match runner
            .release_feature_cache(&entry.machine_id, &entry.run_id, entry.reason)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                eprintln!(
                    "[RunnerCache] release for run {} on {} still pending: {error}",
                    entry.run_id, entry.machine_id
                );
                !worth_retrying(&error)
            }
        };
        if done {
            update_pending(settings, |queue| {
                queue.into_iter().filter(|p| *p != entry).collect()
            })?;
        }
    }
    Ok(())
}

pub fn pending_runner_releases(
    settings: &dyn AppSettingsRepository,
) -> Result<Vec<PendingRunnerRelease>, String> {
    let _lock = PENDING_LOCK.lock().map_err(|e| e.to_string())?;
    read_pending(settings)
}

fn read_pending(settings: &dyn AppSettingsRepository) -> Result<Vec<PendingRunnerRelease>, String> {
    match settings.app_setting_get(PENDING_KEY)? {
        Some(raw) if !raw.is_empty() => serde_json::from_str(&raw)
            .map_err(|e| format!("unreadable pending runner cache releases: {e}")),
        _ => Ok(Vec::new()),
    }
}

fn update_pending(
    settings: &dyn AppSettingsRepository,
    change: impl FnOnce(Vec<PendingRunnerRelease>) -> Vec<PendingRunnerRelease>,
) -> Result<(), String> {
    let _lock = PENDING_LOCK.lock().map_err(|e| e.to_string())?;
    let queue = change(read_pending(settings)?);
    let raw = serde_json::to_string(&queue).map_err(|e| e.to_string())?;
    settings.app_setting_set(PENDING_KEY, &raw)
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/cache_release.rs"]
mod tests;
