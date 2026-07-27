//! Composition root re-export. The actual `build_core_context` function
//! lives in `demeteo_core::composition` (docs/REMOTE_EXECUTION.md
//! M0.2) so both this Tauri app and the future headless runner share one
//! construction site. `lib.rs::run`'s Tauri `.setup()` closure calls it,
//! supplying `ExecutionMode::Router` and a `TauriNotificationAdapter`,
//! then adapts nothing further — the returned `AppContext` is managed
//! as-is.

pub use demeteo_core::composition::{build_core_context, CoreConfig, ExecutionMode};
pub use demeteo_core::state::AppContext;
