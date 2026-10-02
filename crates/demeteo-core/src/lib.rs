pub mod adapters;
pub mod application;
pub mod composition;
pub mod credential_cache;
pub mod db;
pub mod domain;
pub mod error;
pub mod infrastructure;
pub mod paths;
pub mod ports;
pub mod shared;
pub mod ssh_util;
pub mod state;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support;

/// Scratch directories for tests, reachable from other crates' test builds
/// through the `test-support` feature. Rationale on the module itself.
#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/test_dir.rs"]
pub mod test_dir;
