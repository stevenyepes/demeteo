//! Shared primitives used across layers. Kept deliberately tiny so
//! every layer can depend on it without circular-import risks.
//!
//! The single POSIX shell-escape function lives here — earlier versions
//! of the codebase had three separate copies (`paths::shell_escape_posix`,
//! `adapters/merge::shell_escape`, `commands/feature_lifecycle::shell_escape`)
//! which all drifted apart. They have been consolidated here.
//!
//! Time and ID helpers are split out from `paths.rs` so callers that
//! need time without path math don't pull in path utilities.
//!
//! `paths.rs` re-exports the old function names to keep the rest of
//! the codebase compiling during the migration. Those re-exports will
//! be removed in Phase F.

pub mod ids;
pub mod shell;
pub mod time;

// `paths.rs` lives at the crate root for now (Phase A migration).
// During Phase D it will move here as a sibling.
