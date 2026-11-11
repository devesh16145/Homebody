//! Sandbox-ai core.
//!
//! Everything the product does that is not pixels lives here, so the desktop app, the CLI and
//! (later) the fleet server share one implementation:
//!
//! - [`hostname`]: what counts as a valid destination name, and how names match rules
//! - [`catalog`]:  plain-language labels for known destinations, and presets for known apps
//! - [`config`]:   the user's settings: mode, protected apps, trusted destinations
//! - [`ledger`]:   the append-only, hash-chained, signed record of every decision
//! - [`proxy`]:    the local gate each protected app must go through
//! - [`engine`]:   the state machine tying it together, and the command surface in docs/ipc.md
//! - [`inspect`]:  looking inside traffic to trusted places: a local certificate authority, request lines
//! - [`secrets`]:  recognising private keys and access tokens in what an app is about to send
//! - [`projects`]: bring a folder in for an agent to work on, review, apply back
//! - [`fence`]:    OS-level enforcement: private users, seatbelt profile, packet filter, launchers
//! - [`control`]:  a local socket so the CLI and scripts can drive a running engine

pub mod catalog;
pub mod config;
pub mod control;
pub mod engine;
pub mod feed;
pub mod fence;
pub mod hostname;
pub mod inspect;
pub mod ledger;
pub mod model;
pub mod paths;
pub mod projects;
pub mod proxy;
pub mod secrets;

pub use engine::{Engine, Event};
pub use model::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current time as an ISO 8601 UTC string with milliseconds, the only time format we store or emit.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn parse_iso(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&chrono::Utc))
}
