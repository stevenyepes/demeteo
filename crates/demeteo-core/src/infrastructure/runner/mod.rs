//! Provisioning + supervision of `demeteo-runner`, the headless control
//! binary that runs on each remote machine. Pure-logic submodules only —
//! `binary` (locate + arch + version), `install` (systemd unit
//! rendering), `status` (remote state probe over SSH). The Tauri
//! download path lives in `src-tauri/src/adapters/tauri_ui/runner_download.rs`
//! because it emits Tauri events and manages a cancellation flag.

pub mod binary;
pub mod install;
pub mod status;
