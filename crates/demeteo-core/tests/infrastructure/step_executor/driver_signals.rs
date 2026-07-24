//! Tests for the three helpers extracted out of `driver.rs` into
//! `driver/signals.rs`, `driver/status.rs`, and `driver/publish.rs`.
//!
//! **Status: deferred — testing seams not yet trivial.**
//!
//! Each of these methods depends on at least one collaborator that has no
//! in-memory test double wired into the test harness:
//!
//! * `ExecutionDriver::capture_signal` (signals.rs) calls
//!   `self.features.get(&self.f_id)` and `self.signals.enqueue(...)`. The
//!   `MemorySignalsPort` trait has no test fake here, and constructing a real
//!   `ExecutionDriver` requires a `GitOpsHelper`, a `DriverRegistry`, an
//!   `AgentRegistry`, an `AgentExecutionPort`, etc. — a fixture the mirror
//!   tests under `crates/demeteo-core/tests/infrastructure/step_executor/driver.rs`
//!   don't assemble. The `pub(crate)` bump suggested in the extraction plan
//!   would *unblock* a future test; that decision is left to the integrator.
//!
//! * `ExecutionDriver::ensure_feature_running` (status.rs) is `fn` (private),
//!   and the only way to reach it is through `start_execution_with_ctx` — a
//!   full-featured run loop that requires `machine_id`, a target dir, an
//!   agent registry, etc. The short-straw pathway would be a focused unit
//!   test against a fake `FeatureRepository` + `NotificationPort`, which
//!   would require promoting `ensure_feature_running` to `pub(crate)`.
//!
//! * `ExecutionDriver::auto_publish_pr` (publish.rs) is `async fn` (private),
//!   depends on `self.mr_publisher: Option<Arc<dyn MrPublisher>>`, and
//!   needs the same fake-or-fakeable `MrPublisher` trait impl that the
//!   publish UI uses. Same visibility / fixture problem as the other two.
//!
//! Once the integrator promotes these helpers to `pub(crate)` and we have
//! minimal in-memory doubles for `MemorySignalsPort` and `MrPublisher`,
//! the three tests below can be filled in. The seam is documented here so
//! the next person who touches this file knows what to build first.
#![allow(unused_imports, dead_code)]

// Placeholder so the file is at least non-empty and the clippy/fmt passes
// during the structural-decomposition PR. Replace with real assertions once
// the seams are unblocked.
#[test]
fn seam_documented() {
    // No-op. See the module-level doc for what each helper needs to become
    // testable.
}
