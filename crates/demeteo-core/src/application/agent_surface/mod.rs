//! The shared operation seam an external client — a headless caller outside
//! the desktop UI, and later a CLI — invokes over
//! [`crate::state::AppContext`], instead of that caller re-deriving what
//! [`crate::application::run_view::RunView`] and the repository ports
//! already know.
//!
//! Every operation here is a free function over `&AppContext`, matching
//! every other `application::*` module — there is no `AgentSurface` struct
//! or trait to construct. Reads live in [`reads`]; writes are a separate,
//! later addition to this seam.

pub use reads::*;
pub use writes::*;

mod reads;
mod writes;
