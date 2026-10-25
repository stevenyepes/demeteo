pub mod adapters;
pub mod api;
pub mod attachments;
pub mod auth;
pub mod config;
pub mod pairing;
pub mod ports;
pub mod routes;
pub mod socket;
pub mod startup;
pub mod store;

/// The wire types, for a crate that depends on the Hub and not on
/// `demeteo-hub-protocol`. `src-tauri` is one: its tests drive the socket
/// through this crate and can name a message no other way.
pub use demeteo_hub_protocol as protocol;
