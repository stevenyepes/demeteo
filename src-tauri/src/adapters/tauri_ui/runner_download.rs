//! Tauri-specific adapter for fetching the `demeteo-runner` release
//! asset. Lives here (not in `crates/demeteo-core/src/infrastructure/runner/`)
//! because it emits Tauri events to the frontend and owns the global
//! cancellation flag. Pure-logic counterparts (`runner::binary::release_cache_path`,
//! the asset name, the URL layout) live in the core crate.

use crate::error::AppError;
use crate::infrastructure::runner::binary::{release_cache_path, RUNNER_ASSET_NAME};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// GitHub repo the release assets are uploaded to. Mirrors CI in
/// `.github/workflows/build.yml`.
const GITHUB_REPO: &str = "stevenyepes/demeteo";

/// Appended to download-failure errors so a dev running an unpackaged
/// build (no matching published release) sees the right next step.
const DEV_FALLBACK_HINT: &str = " If you're running a local/dev build, run \
     `npm run build:runner` (or set DEMETEO_RUNNER_BIN to a Linux x86_64 build) — \
     Demeteo refuses to push a non-Linux binary to a remote machine.";

/// Emitted to the frontend as the binary streams in, so a several-tens-
/// of-MB download over a slow laptop connection shows real progress
/// instead of an indefinite spinner.
#[derive(Debug, Clone, Serialize)]
pub struct RunnerDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub const DOWNLOAD_PROGRESS_EVENT: &str = "runner-download-progress";

/// Successful download metadata, returned to the frontend so it can
/// pass `path` straight into `remote_enable_runs`.
#[derive(Debug, Serialize)]
pub struct DownloadedRunner {
    pub path: String,
    pub version: String,
}

/// Set by `cancel()`, checked between chunks by an in-flight
/// `download_release`. Only one download is ever driven from the UI
/// at a time, so a plain static is enough.
static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);

pub fn reset_cancel() {
    CANCEL_FLAG.store(false, Ordering::SeqCst);
}

pub fn cancel() {
    CANCEL_FLAG.store(true, Ordering::SeqCst);
}

fn is_cancelled() -> bool {
    CANCEL_FLAG.load(Ordering::SeqCst)
}

/// Fetch the version-matched release asset from GitHub to a laptop-
/// local cache, verifying its published checksum. Streams to a
/// `.partial` sibling of the cache path and renames into place only
/// once the checksum passes, so a cancelled or failed download never
/// leaves a corrupt file at the path the local-check treats as
/// trustworthy. Reads `version` from `app.package_info().version` so
/// the frontend doesn't have to know it.
#[tauri::command]
pub async fn remote_runner_download(app: AppHandle) -> Result<DownloadedRunner, AppError> {
    reset_cancel();
    let version = app.package_info().version.to_string();
    download_release(&app, &version).await
}

/// Tauri command — cancels whatever `remote_runner_download` is
/// currently in flight. No-op if nothing is downloading.
#[tauri::command]
pub fn remote_runner_download_cancel() {
    cancel();
}

async fn download_release(app: &AppHandle, version: &str) -> Result<DownloadedRunner, AppError> {
    let (base_url, sha_url) = urls_for(version);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::from(format!("failed to build HTTP client: {e}")))?;

    let expected_sha = fetch_checksum(&client, &sha_url).await?;
    let dest = release_cache_path(version);
    let partial = dest.with_extension("partial");
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| AppError::from(format!("failed to create {}: {}", dir.display(), e)))?;
    }

    let response = client
        .get(&base_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            AppError::from(format!(
                "couldn't download demeteo-runner {version} from {base_url}: {e}.{DEV_FALLBACK_HINT}"
            ))
        })?;

    stream_to_partial(&partial, response, app, version).await?;
    verify_and_finalize(&partial, &dest, &expected_sha, version).await?;

    Ok(DownloadedRunner {
        path: dest.display().to_string(),
        version: version.to_string(),
    })
}

fn urls_for(version: &str) -> (String, String) {
    let tag = if version.contains('-') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let base =
        format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/{RUNNER_ASSET_NAME}");
    (base.clone(), format!("{base}.sha256"))
}

async fn fetch_checksum(client: &reqwest::Client, sha_url: &str) -> Result<String, AppError> {
    let text = client
        .get(sha_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            AppError::from(format!(
                "couldn't download checksum from {sha_url}: {e}.{DEV_FALLBACK_HINT}"
            ))
        })?
        .text()
        .await
        .map_err(|e| AppError::from(format!("failed reading checksum: {e}")))?;
    text.split_whitespace()
        .next()
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::from(format!("checksum file at {sha_url} was empty")))
}

async fn stream_to_partial(
    partial: &PathBuf,
    response: reqwest::Response,
    app: &AppHandle,
    version: &str,
) -> Result<(), AppError> {
    let mut file = tokio::fs::File::create(partial)
        .await
        .map_err(|e| AppError::from(format!("failed to create {}: {}", partial.display(), e)))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let total = response.content_length();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if is_cancelled() {
            let _ = tokio::fs::remove_file(partial).await;
            return Err(AppError::from(format!(
                "download of demeteo-runner {version} was cancelled"
            )));
        }
        let chunk = chunk.map_err(|e| {
            let _ = std::fs::remove_file(partial);
            AppError::from(format!(
                "download of demeteo-runner {version} failed: {e}.{DEV_FALLBACK_HINT}"
            ))
        })?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
            let _ = tokio::fs::remove_file(partial).await;
            return Err(AppError::from(format!(
                "failed writing {}: {}",
                partial.display(),
                e
            )));
        }
        let _ = app.emit(
            DOWNLOAD_PROGRESS_EVENT,
            RunnerDownloadProgress { downloaded, total },
        );
    }
    drop(file);
    Ok(())
}

async fn verify_and_finalize(
    partial: &PathBuf,
    dest: &PathBuf,
    expected_sha: &str,
    version: &str,
) -> Result<(), AppError> {
    let bytes = tokio::fs::read(partial)
        .await
        .map_err(|e| AppError::from(format!("failed to re-read {}: {}", partial.display(), e)))?;
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha != expected_sha {
        let _ = tokio::fs::remove_file(partial).await;
        return Err(AppError::from(format!(
            "downloaded demeteo-runner {version} failed checksum verification \
             (expected {expected_sha}, got {actual_sha}) — try again, and if it \
             keeps happening this release's asset may be corrupt"
        )));
    }
    tokio::fs::rename(partial, dest).await.map_err(|e| {
        AppError::from(format!(
            "failed to finalize {} from {}: {}",
            dest.display(),
            partial.display(),
            e
        ))
    })?;
    Ok(())
}
