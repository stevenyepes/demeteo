//! The gate's policy, with no I/O in it.
//!
//! A gate performs almost nothing itself: it parks, and then it reads one
//! recorded answer. Its hard parts are two decisions over that answer — what a
//! decision string means, and where a `redirect` lands — and both used to be
//! spelled inside `adapters/step_executor/steps/gate.rs`, in `&mut self`
//! methods that were also writing `step_executions` rows, emitting events and
//! mutating the driver's retry context.
//!
//! Everything here is synchronous and total: it takes what the adapter
//! observed and returns what should happen. `domain/` has no `async fn`
//! anywhere in it, which is what keeps that boundary honest.
//!
//! The adapter keeps the choreography: parking on the waiter, capturing the
//! memory signal, resetting the redirect target's durable state, and mapping
//! the verdict onto the executor's own `StepOutcome`.

pub(crate) mod redirect;
