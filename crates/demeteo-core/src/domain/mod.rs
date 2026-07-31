//! Policy: what *should* happen, given what an adapter observed.
//!
//! Everything here is synchronous and total. It takes an observation and
//! returns a decision; it performs nothing. `domain/` has **no `async fn`
//! anywhere in it**, and that is what keeps the boundary honest — a decision
//! spelled inside an `async fn` that also does I/O is only reachable from a
//! test that stands up the ports the I/O needs, so in practice it stops being
//! tested at all. Keeping the decision here makes it reachable from a unit
//! test with no port doubles, and leaves the adapter holding only the
//! choreography. See AGENTS.md §3, "Where a decision is allowed to live".
//!
//! Submodules should cite this rule rather than restating it.

pub mod action;
pub mod agent_event;
pub mod agent_session;
pub mod app_view;
pub mod artifact;
pub(crate) mod artifact_capture;
pub mod artifact_contract;
pub mod attachment;
pub mod bootstrap;
pub(crate) mod command_step;
pub mod ecosystem;
pub mod expr;
pub(crate) mod finalize;
pub(crate) mod gate;
pub mod harness_attribution;
pub mod harness_baseline;
pub mod harness_delta;
pub mod harness_failure;
pub mod harness_fingerprint;
pub mod harness_outcome;
pub mod harness_preflight;
pub mod harness_remediation;
pub mod harness_triage;
pub mod ids;
pub mod intercept;
pub mod memory;
pub(crate) mod merge_status;
pub mod models;
pub mod permission;
pub mod prompt_budget;
pub mod prompt_context;
pub mod restart_reconcile;
pub mod rework;
pub mod run_control;
pub mod run_spec;
pub mod sequence;
pub mod staged_deliverable;
pub mod step_boundary;
pub mod text;
pub mod usage;
pub mod verifier;
pub mod workflow_graph;
pub mod workflow_overrides;
