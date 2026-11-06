//! Git hosts a tenant connects from the Hub's own pages, and the minter that
//! reads them (decision 71, amended 2026-11-06; `docs/HUB.md` §6.5).
//!
//! # Connecting a GitHub App
//!
//! GitHub's App manifest flow: the Hub hands the browser a manifest naming
//! the two permissions a run needs, GitHub creates the App under the user's
//! account and redirects back with a one-hour code, and the Hub exchanges the
//! code for the App's id, slug and private key. The key goes straight into
//! Transit as `demeteo-hub-gh-<app id>` and is dropped; the client and
//! webhook secrets in the same answer are never read. What is kept is
//! [`GitHostRow`](crate::store::git_hosts::GitHostRow): no secret at all.
//!
//! That exchange is the one moment the key is in the Hub's memory. Signing
//! still happens in Transit, so a Hub compromised later can use the key, as
//! before, but never copy it.
//!
//! # Which App mints
//!
//! [`minter::GithubApps`] serves every tenant: per fetch it reads the
//! start's tenant's Apps on the repository's host beside the one configured
//! by environment, and mints with the single one installed on the
//! repository. Connecting or removing an App needs no restart.

pub mod handlers;
pub mod manifest;
pub mod minter;
pub mod port;
pub mod postgres;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
