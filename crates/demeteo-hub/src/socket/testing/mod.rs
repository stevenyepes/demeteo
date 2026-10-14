//! The scripted instance a session test plays against, the channel pair
//! that stands where the WebSocket does, and the rows that stand where the
//! store does.
//!
//! # This code panics, deliberately
//!
//! A [`FakeInstance`] is a script of what the Hub must send, in order, and
//! every departure from it is a panic: a frame other than the next one
//! expected, an expected frame that does not come, and a frame the Hub sent
//! that the script never read. A `Result` here would be unwrapped at every
//! call site, and one left unread would turn a refusal into a pass.
//!
//! A [`MemoryHub`] panics on a call it has no answer to, for the reason on
//! it.
//!
//! Checking only the frames a script reads is not enough. A session that
//! delivers a request twice, or delivers one that had expired, does it
//! *after* the script got the frames it asked for and stopped looking, so
//! every assertion the test made still holds. That is why [`finish`] does not
//! return until the Hub end has gone away and refuses anything left in
//! between, and why dropping a fake without finishing it makes the same
//! refusal of whatever had already arrived.
//!
//! # What is real and what is not
//!
//! No clock is paused here: the workspace builds `tokio` without `test-util`.
//! The wait for an expected frame is real time, bounded by [`PATIENCE`], and
//! only a script that is about to fail spends it.
//!
//! A ping is answered by the channel and never reaches the script, as a
//! WebSocket client answers one below the application. A script that counted
//! pings would be asserting a timer.
//!
//! Nothing here names an origin or a path. This module is compiled outside
//! `#[cfg(test)]`, so `tests/no_literals.rs` scans each of its files like any
//! other source file.
//!
//! [`finish`]: FakeInstance::finish

mod instance;
mod memory_hub;
mod transport;

pub use instance::{FakeInstance, PATIENCE};
pub use memory_hub::{InstanceView, MemoryHub, MemoryStore, MirrorRow, RequestView, SnapshotRow};
pub use transport::ChannelTransport;
