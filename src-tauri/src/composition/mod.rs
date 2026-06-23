//! Composition root. The single function `build_app_context` is the
//! only place in the codebase that constructs concrete adapters and
//! wires them into the `AppContext` dependency bag. Extracted from
//! `lib.rs::run` so that adding a new port requires touching one
//! function instead of the bootstrap.
//!
//! During Phase A this module re-exports `AppContext` so other modules
//! can find the type. The actual construction happens inline in
//! `lib.rs::run` until Phase B is complete; after Phase B the body of
//! `run` shrinks to a single `composition::build_app_context(app)?`
//! call.

pub use crate::state::AppContext;
