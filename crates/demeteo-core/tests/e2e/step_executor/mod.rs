//! End-to-end tests for the step executor, split along the file's own natural
//! groups. [`harness`] holds the doubles and the wiring every other file
//! builds on.

mod bootstrap;
mod gate_decide;
mod harness;
mod launch_resolution;
mod origin_cut;
mod remote_mirror;
mod retry_guards;
mod sync_base;
