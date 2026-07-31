//! What bounds one agent session: dollars, context window, identity.
//!
//! Three questions the driver asks before and after every agent turn — how
//! much may this turn spend, has the session outgrown the model's context
//! window, and is this the same session as the last step's. None of them is a
//! call: each takes what the driver already knows and returns an answer. They
//! used to be inherent items on `ExecutionDriver`, so reaching any of them
//! from a test meant standing up the twenty-odd port doubles the arithmetic
//! never reads — which is why the two budget tests shipped `#[ignore]`d and
//! `unimplemented!()`.
//!
//! Synchronous and total, per the [`domain`](crate::domain) rule. The driver
//! keeps the choreography: reading the live session's token count, killing it,
//! building the resume summary, spawning fresh.

pub mod budget;
pub mod context_window;
pub mod key;
