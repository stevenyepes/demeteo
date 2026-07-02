import { invoke } from "@tauri-apps/api/core";

/**
 * Identity bundle returned by the backend on startup.
 *
 * Mirrors the Rust `AppInfo` struct (see `commands/app_session.rs`).
 * - `version` comes from `CARGO_PKG_VERSION` (always matches
 *   `tauri.conf.json`).
 * - `channel` is `"stable"` or `"nightly"`; selected at compile time via
 *   the `DEMETEO_RELEASE_CHANNEL` env var (defaults to `stable`).
 */
export interface AppInfo {
  version: string;
  channel: string;
}

/** Fetch the running binary's version + release channel. Used by the
 * About screen to render a dynamic title + a `STABLE` / `NIGHTLY` badge
 * instead of a hard-coded string. */
export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}
