//! Homebody core.
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
//! - [`compliance`]: where the evidence fits in NIST SP 800-53 and OWASP's agentic list
//! - [`policy`]:   the rules in a formal language (Cedar), for review outside the app
//! - [`projects`]: bring a folder in for an agent to work on, review, apply back
//! - [`fence`]:    OS-level enforcement: private users, seatbelt profile, packet filter, launchers
//! - `fence_linux`: the same fence built from Linux's tools: system users, nftables, a systemd service
//! - [`fleet`]:    belonging to an organisation's fleet: its rules come here, the log goes there
//! - `boxvm`:      the Box: an agent in a Linux virtual machine with no network interface (macOS)
//! - [`control`]:  a local socket so the CLI and scripts can drive a running engine
//! - [`appicons`]: each app's own icon, from its maker's Mac app

pub mod appicons;
pub mod boxdisk;
#[cfg(unix)]
pub mod boxvm;
pub mod catalog;
pub mod compliance;
pub mod config;
pub mod control;
pub mod engine;
pub mod feed;
pub mod fence;
pub mod fleet;
#[cfg(unix)]
pub mod fence_linux;
#[cfg(windows)]
pub mod fence_windows;
pub mod hostname;
pub mod inspect;
pub mod keys;
pub mod ledger;
pub mod model;
pub mod paths;
pub mod policy;
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
