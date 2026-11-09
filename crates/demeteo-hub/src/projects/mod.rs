//! Hub Projects: the repositories a tenant keeps on the Hub itself, which a
//! dispatch to a runner names. They are the Hub's own and never synced with a
//! desktop's, which supersedes decision 65's resolution of a runner's
//! repository through a source desktop's snapshot. A dispatch to a desktop
//! still names that desktop's own Projects.
//!
//! A Hub Project holds the repository's identity (provider, host and
//! `owner/repo`) and the run-shape defaults a runner dispatch fills in, and
//! nothing else of the repository: no path, no URL with userinfo and no
//! secret (`AGENTS.md` §2). [`verdict`] says what a project may hold; the
//! store keeps one tenant's rows, unique per host and repository.

pub mod handlers;
pub mod port;
pub mod postgres;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod verdict;
