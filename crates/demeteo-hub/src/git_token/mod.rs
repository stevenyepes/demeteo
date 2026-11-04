//! Git tokens for runs the Hub started on a runner: a GitHub App installation
//! token, minted per fetch for the one repository the start was signed for
//! (decision 71, `docs/HUB.md` §10).
//!
//! # Why a route and not the socket
//!
//! The runner asks over HTTPS, beside the attachment fetch, because the
//! socket is the wrong carrier for a secret twice over: it closes every
//! fifteen minutes by design, and every type on it is in
//! `demeteo-hub-protocol`, whose leak test refuses a `token` field. No type
//! here may move into that crate.
//!
//! # What is minted for whom
//!
//! Only for an instance paired as a runner, for a `start_feature` queued to
//! it that it **accepted** and that carries a runner part, and only while
//! the Feature it created for that start has not ended by its mirrored
//! events. The repository is the one in the stored, Hub-signed envelope and
//! never anything the request body says: the body names a request and a
//! Feature, and nothing else.
//!
//! # The binding rule
//!
//! The first fetch that would be served binds its `feature_id` to the
//! request, by a conditional `UPDATE` so that of two first fetches exactly
//! one wins; every later fetch must name the same Feature. A start is one
//! run, and a runner holding several cannot spend one start's tokens on
//! another.
//!
//! # The custody rule
//!
//! A minted token is handed to the caller and dropped. It is never written
//! to Postgres, a log line, a tracing field or `Debug` output, and the
//! response says `Cache-Control: no-store`. GitHub's error text is never
//! repeated either, in a response or a log.
//!
//! Dependencies run one way: this module uses `store`, `api`'s run fold and
//! the ports, and nothing imports it but `routes`.

pub mod fetch;
pub mod port;
pub mod postgres;
pub mod verdict;
