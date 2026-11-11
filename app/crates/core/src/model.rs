//! The types in docs/ipc.md, exactly. If you change one, change the contract first.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Private,
    Watch,
    Off,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Private => "private",
            Mode::Watch => "watch",
            Mode::Off => "off",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "private" => Some(Mode::Private),
            "watch" => Some(Mode::Watch),
            "off" => Some(Mode::Off),
            _ => None,
        }
    }
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Indicator {
    Private,
    Watching,
    Asking,
    Blocked,
    Off,
    Attention,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FenceState {
    Up,
    Down,
    NotInstalled,
    Checking,
}

#[derive(Serialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub allowed: u64,
    pub blocked: u64,
    pub asked: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub version: String,
    pub mode: Mode,
    pub mode_since: String,
    pub private_since: Option<String>,
    pub lockdown: bool,
    pub fence: FenceState,
    pub setup_needed: bool,
    pub setup_reasons: Vec<String>,
    pub indicator: Indicator,
    pub sentence: String,
    pub today: Counters,
    pub apps: Vec<AppStatus>,
    pub pending: Vec<Ask>,
    /// Present while Watch is on.
    pub watch: Option<WatchStatus>,
    /// Present when a finished Watch has results waiting to be reviewed.
    pub watch_review: Option<WatchStatus>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub since: String,
    /// When Watch ends by itself (while on), or when it ended (for a review). None: no timer.
    pub until: Option<String>,
    /// Places contacted that are neither trusted nor blocked yet.
    pub places: u64,
}

/// One place an app contacted during Watch that has no rule yet.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WatchItem {
    pub app_id: String,
    pub app_name: String,
    pub app_icon: String,
    pub host: String,
    pub label: String,
    pub purpose: String,
    pub times: u64,
    pub bytes_out: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WatchReview {
    pub since: String,
    pub until: Option<String>,
    /// Watch is still on: this is what has been seen so far.
    pub active: bool,
    pub items: Vec<WatchItem>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub icon: String,
    pub installed: bool,
    pub protected: bool,
    pub fence: FenceState,
    pub running: bool,
    pub can_start: bool,
    pub workspace: Option<String>,
    pub today: Counters,
    /// Added by the user ("protect your own app"); can be removed.
    pub custom: bool,
    /// Installed, but in a way a protected app cannot use. A sentence for the user.
    pub problem: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Allowed,
    Blocked,
    Asked,
    Seen,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Trusted,
    NotTrusted,
    YouAllowedOnce,
    YouAlwaysAllow,
    YouBlocked,
    NoAnswer,
    Watching,
    ProtectionOff,
    Lockdown,
    Malformed,
    LocalAddress,
    /// Tried to reach the network without going through the gate; refused by the fence.
    AroundGate,
}

impl Reason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reason::Trusted => "trusted",
            Reason::NotTrusted => "not_trusted",
            Reason::YouAllowedOnce => "you_allowed_once",
            Reason::YouAlwaysAllow => "you_always_allow",
            Reason::YouBlocked => "you_blocked",
            Reason::NoAnswer => "no_answer",
            Reason::Watching => "watching",
            Reason::ProtectionOff => "protection_off",
            Reason::Lockdown => "lockdown",
            Reason::Malformed => "malformed",
            Reason::LocalAddress => "local_address",
            Reason::AroundGate => "around_gate",
        }
    }
    pub fn parse(s: &str) -> Option<Reason> {
        Some(match s {
            "trusted" => Reason::Trusted,
            "not_trusted" => Reason::NotTrusted,
            "you_allowed_once" => Reason::YouAllowedOnce,
            "you_always_allow" => Reason::YouAlwaysAllow,
            "you_blocked" => Reason::YouBlocked,
            "no_answer" => Reason::NoAnswer,
            "watching" => Reason::Watching,
            "protection_off" => Reason::ProtectionOff,
            "lockdown" => Reason::Lockdown,
            "malformed" => Reason::Malformed,
            "local_address" => Reason::LocalAddress,
            "around_gate" => Reason::AroundGate,
            _ => return None,
        })
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FeedEvent {
    pub id: u64,
    pub ts: String,
    pub app_id: String,
    pub app_name: String,
    pub app_icon: String,
    pub host: String,
    pub port: u16,
    pub label: String,
    pub purpose: String,
    pub verb: Verb,
    pub reason: Reason,
    pub explanation: String,
    pub open: bool,
    pub bytes_out: Option<u64>,
    pub bytes_in: Option<u64>,
    pub duration_ms: Option<u64>,
    pub trusted: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub id: String,
    pub app_id: String,
    pub app_name: String,
    pub app_icon: String,
    pub host: String,
    pub port: u16,
    pub label: String,
    pub purpose: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub expires_at: String,
    pub waiting: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrustEntry {
    pub app_id: String,
    pub host: String,
    pub label: String,
    pub purpose: String,
    pub enabled: bool,
    pub source: String,
    pub recommended: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChainCheck {
    pub ok: bool,
    pub count: u64,
    pub checked_at: String,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Proof {
    pub mode: Mode,
    pub private_since: Option<String>,
    pub streak_days: u64,
    pub streak_text: String,
    pub apps_protected: u32,
    pub app_names: Vec<String>,
    pub bytes_out_today: u64,
    pub untrusted_bytes_since_private: u64,
    pub blocked_total: u64,
    pub allowed_total: u64,
    pub chain: Option<ChainCheck>,
    pub public_key: String,
    pub statement: String,
    pub covers: String,
    pub cannot_see: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SetupResult {
    pub ok: bool,
    pub message: String,
}

/// What the user can answer to a prompt, plus the timeout the core applies itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Once,
    Always,
    Block,
    Timeout,
}

impl Choice {
    pub fn as_str(&self) -> &'static str {
        match self {
            Choice::Once => "once",
            Choice::Always => "always",
            Choice::Block => "block",
            Choice::Timeout => "timeout",
        }
    }
    pub fn parse(s: &str) -> Option<Choice> {
        match s {
            "once" => Some(Choice::Once),
            "always" => Some(Choice::Always),
            "block" => Some(Choice::Block),
            "timeout" => Some(Choice::Timeout),
            _ => None,
        }
    }
}
