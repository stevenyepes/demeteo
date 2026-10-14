//! The instance socket: the one connection a paired desktop opens to the Hub,
//! carrying its run events and snapshots up and signed requests down.
//!
//! # What this proves, and what it does not
//!
//! An open socket proves one thing. Whoever opened it presented an access
//! token the Hub issued, unexpired and not retired, with a fresh proof made
//! by the device key that token is bound to. The tenant and the instance
//! every write is made for come from that token's row and from nowhere else.
//!
//! It does not prove that anything said on it is true. An event, a snapshot
//! and the scopes in a `hello` are the instance's report of itself, and a
//! stolen device key with its refresh token speaks as the instance
//! (`docs/HUB.md` §3). An `install_id` is a label the peer chose and a
//! `request_id` in a result is a value the peer chose; neither picks the row
//! a write lands on.
//!
//! Nor is the socket proof to the instance of anything. A request is trusted
//! for the signature in its envelope, checked against the key the instance
//! pinned at pairing, and not for the connection it arrived over (§6). The
//! scopes the Hub keeps are what the last `hello` said and it enforces none
//! of them: declining to queue a request for a scope that is off spares a
//! round trip, and the instance's own toggles decide (§7).
//!
//! # What is deliberately not stored
//!
//! **Costs and gate summaries.** The protocol types carry neither: a run
//! event is a kind from a closed list and three optional ids, and no kind
//! reports a cost. Holding only what those types carry therefore adds no
//! column for either, and a column added "for later" would be the free-text
//! field `RunEventPayload` exists to make impossible.
//!
//! **Frame text.** The mirror holds the parsed `RunEvent` serialised again,
//! never the text that arrived. What a row can hold is then bounded by the
//! type and not by what the parser happened to accept on the way in, and a
//! reader of the mirror meets one spelling of an event whatever the peer's
//! serialiser did with key order, whitespace or an absent optional id.
//!
//! # The frame rule
//!
//! A frame is JSON text holding one protocol struct and nothing around it.
//! The protocol crate has no envelope and no discriminator, so [`frame`]
//! tells the inbound messages apart by trying each of the four types and
//! requiring **exactly one** to parse. That is unambiguous only because all
//! four refuse unknown fields and no two share their required keys, which is
//! a property of the protocol crate that nothing there states. It is pinned
//! from outside: `each_protocol_fixture_parses_as_exactly_one_inbound_frame`
//! in `src-tauri/tests/hub_socket.rs` fails the day a fixture parses as two
//! types or as none, and more than one parsing is an error at run time and
//! never a choice of the first.
//!
//! **Rejected: a tagged frame enum in the protocol crate.** It is the cleaner
//! shape, and it changes the wire contract: every message gains a tag the
//! desktop client must write, in a crate both ends share and whose fixtures
//! are the contract. That is a decision about the protocol and not about how
//! the Hub reads it, so it is not taken here on the way to something else.
//! If it is taken, [`frame`] becomes one `from_str` and the fixture pin goes
//! with it.

pub mod connections;
pub mod cursor;
pub mod frame;
pub mod handler;
pub mod port;
pub mod postgres;
pub mod requests;
pub mod session;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

/// Instances connected to one `hub` process at once, over every tenant. A
/// default no decision stands behind, and measured against nothing: it is
/// [`MAX_PENDING`](crate::pairing::MAX_PENDING)'s number, carried here for
/// want of a better one. What it bounds is not the registry, whose entries
/// are small, but what each connection holds beside its entry: a task, a
/// socket and its buffers. At the bound an upgrade is answered
/// `503 too_many_connections` and no session is closed to make room
/// ([`Connections`](connections::Connections) says why).
pub const MAX_CONNECTIONS: usize = 16_384;

/// Bytes in the largest message a session reads, and in the largest frame of
/// one. A default no decision stands behind, and measured against nothing:
/// not against the snapshot of an installation with many Projects, which is
/// the one message with no bound of its own. An instance whose snapshot is
/// larger cannot deliver it: the transport reports the message as a
/// connection that failed, and the session ends without having read it.
///
/// Set on the upgrade and not left to axum, whose own bounds are 64 MiB a
/// message and 16 MiB a frame. Either is what one connection may make the
/// Hub buffer, and there are up to [`MAX_CONNECTIONS`] of them. Both bounds
/// take this one value: a frame is part of a message, so a frame bound above
/// it would never be the one that is reached.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Bytes in the longest `feature_id` a run event may carry. A default no
/// decision stands behind, and measured against nothing: chosen far below
/// what one entry of the mirror's primary-key index can hold (a B-tree entry
/// is limited to about a third of an 8 kB page) and far above the desktop's
/// `f-<16 hex>` ids. An id over it would fail at the store on every resend,
/// so [`frame::parse`] refuses it as the peer's violation. It is a bound the
/// desktop client, which is not built, has to share.
pub const MAX_FEATURE_ID_BYTES: usize = 256;

/// Seconds between the pings a session sends. A default no decision stands
/// behind, and measured against nothing: not against how soon anything
/// between an instance and the Hub drops an idle connection, and not against
/// what a ping costs a desktop that would otherwise sleep.
pub const PING_INTERVAL_SECS: u64 = 30;

/// Seconds a session goes on hearing nothing from its peer, of any kind,
/// before it takes the peer for gone. It is how long an instance whose
/// connection died without a close goes on reading as present. Carried as
/// [`PING_INTERVAL_SECS`] is, and tied to it by the one thing the assertion
/// below holds: at or under the interval, a peer with nothing to send is
/// dropped before the ping it would have answered.
pub const PONG_DEADLINE_SECS: u64 = 90;

const _: () = assert!(PONG_DEADLINE_SECS > PING_INTERVAL_SECS);

/// Seconds a stopping `hub` waits, once it has asked every session to end,
/// for each to write its departure and send its close. A default no decision
/// stands behind, and measured against nothing: not against how slow a
/// departure gets under load. What it has to stay under is the container's
/// stop grace, after which the process is killed and waits for nothing:
/// `deploy/hub/compose.yaml` sets no `stop_grace_period`, so that is
/// Compose's default of ten seconds. A session that outlasts it is dropped
/// as a kill would drop it, and its instance goes on reading as present.
pub const SHUTDOWN_DRAIN_SECS: u64 = 5;

/// Seconds a request stays deliverable after it was enqueued. A default no
/// decision stands behind, and measured against nothing: not against how
/// long a desktop is usually away, and not against how stale a "start this"
/// may get before running it is a surprise. It is signed into the envelope
/// as `expires_at`, so changing it changes no request already queued.
pub const REQUEST_TTL_SECS: u64 = 600;

/// Random bytes in a request's id. An instance refuses an id it has already
/// accepted (`docs/HUB.md` §6), so two requests of one instance must never
/// share one, and the primary key refuses the insert if they ever did. A
/// default no decision stands behind: 128 random bits, where a collision is
/// not something to plan for. It has to be the length of
/// [`RequestId`](crate::store::requests::RequestId), which the row is keyed
/// by, and minting an id does not compile when it is not.
pub const REQUEST_ID_BYTES: usize = 16;

/// The `kind` every snapshot row is written under, so that an instance has
/// one row and each snapshot it sends replaces it whole. `docs/HUB.md` §5
/// speaks of one snapshot per kind (`projects`, `workflows`) and the table is
/// keyed for that, but the protocol's
/// [`Snapshot`](demeteo_hub_protocol::Snapshot) is one struct carrying both,
/// with no kind on it to key by.
///
/// **Rejected: a row per field of the struct.** It gives the rows a division
/// the wire does not have, and a reader then has to put the parts back
/// together and take it on trust that they came from one frame.
pub const SNAPSHOT_KIND: &str = "full";
