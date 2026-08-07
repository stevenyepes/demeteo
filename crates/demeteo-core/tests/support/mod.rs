//! Fixtures shared by test modules that live under different parents.
//!
//! Declared once from the crate root (`lib.rs`) rather than `#[path]`-included
//! by each caller: clippy's `duplicate_mod` rejects the same file compiled into
//! more than one module, and it is right to — three copies of a fixture are
//! three types the compiler will not let a helper cross between.

pub mod preflight_strategy;
