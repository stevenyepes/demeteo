use std::sync::{Mutex, MutexGuard, PoisonError};

pub(crate) mod bootstrap;
pub(crate) mod execution_context;
pub(crate) mod feature_graph;
pub(crate) mod gate_presenter;
pub(crate) mod lifecycle;
pub(crate) mod replay;
pub(crate) mod startup_recovery;
pub(crate) mod step_executor;

/// Take one of the executor's in-memory lookup maps, surviving a poisoned lock.
///
/// `cancel_senders` and `gate_waiters` are registries — one entry per running
/// feature, one per step execution awaiting a decision. A panic elsewhere
/// while the lock is held leaves the map itself structurally fine, so
/// poisoning here is evidence that some other task died, not that this data is
/// bad. Propagating it would trade one task's panic for a Stop button that can
/// never fire again and a gate that can never be answered for the rest of the
/// process's life — which is the worse of the two failures by a wide margin.
fn lock_registry<T>(map: &Mutex<T>) -> MutexGuard<'_, T> {
    map.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Bootstrap phase vocabulary — `(id, label)`. Emitted as
/// [`DomainEvent::BootstrapProgress`](crate::ports::notification::DomainEvent::BootstrapProgress)
/// during [`StepExecutor::feature_start`](crate::ports::step_executor::StepExecutor::feature_start)
/// so the UI can animate an inline stepper. The frontend renders `label`
/// verbatim and orders rows by `id`, so this list is the single source of
/// truth for the feature-start sub-steps (the runner adds its own
/// clone-phase ids in `demeteo-runner`). Phases fire `running` →
/// `completed`, or `failed` with the error in `detail`.
pub(crate) mod bootstrap_phase {
    pub const PREPARING: (&str, &str) = ("preparing", "Loading project & workflow");
    pub const CONNECTING: (&str, &str) = ("connecting", "Connecting to machine");
    pub const VERIFYING_REPO: (&str, &str) = ("verifying_repo", "Verifying repository");
    pub const PREPARING_CONTEXT: (&str, &str) = ("preparing_context", "Preparing context & memory");
    pub const SYNCING_ORIGIN: (&str, &str) = ("syncing_origin", "Syncing with origin");
    pub const CREATING_BRANCH: (&str, &str) = ("creating_branch", "Creating feature branch");
    pub const HARNESS_PREFLIGHT: (&str, &str) = ("harness_preflight", "Checking project commands");
    pub const REGISTERING: (&str, &str) = ("registering", "Registering feature & steps");
    pub const STARTING_PIPELINE: (&str, &str) = ("starting_pipeline", "Starting pipeline");
}
