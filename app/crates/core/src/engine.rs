//! The engine: one object that owns the settings, the ledger, the prompts and the fence state,
//! decides every connection, and exposes the command surface in docs/ipc.md.
//!
//! Lock order, where more than one is held: `asks` → `ledger` → `feed`. `cfg` is never held
//! across another lock or an await.

use crate::catalog;
use crate::config::{AppConfig, Config, TrustRule};
use crate::feed::{FeedIndex, Row};
use crate::fence::{self, Health};
use crate::fields;
use crate::hostname;
use crate::ledger::{self, Ledger};
use crate::model::*;
use crate::{now_iso, parse_iso, paths};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch, Notify};

pub const ASK_TIMEOUT: Duration = Duration::from_secs(30);
const ONCE_GRANT: Duration = Duration::from_secs(5 * 60);
const QUIET_AFTER_NO: Duration = Duration::from_secs(10 * 60);
const REPEAT_WINDOW: Duration = Duration::from_secs(60);
const BLOCKED_INDICATOR: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    Status(Status),
    Feed(FeedEvent),
    Ask(Ask),
    AskResolved { id: String, choice: String },
    Navigate { tab: String },
    /// Ask the shell to show a window: "main", "glance" or "prompt".
    Show { window: String, tab: Option<String> },
}

/// The outcome of asking "may this app reach this destination?".
#[derive(Clone, Debug)]
pub struct Decision {
    pub allow: bool,
    pub reason: Reason,
    /// The prompt this decision came from, if any.
    pub ask: Option<u64>,
    /// The canonical destination name, when the name was acceptable.
    pub host: Option<String>,
    /// Plain words for the refused app to show its user.
    pub message: String,
}

struct Pending {
    ask: Ask,
    seq: u64,
    tx: watch::Sender<Option<Choice>>,
}

struct Inner {
    cfg: Mutex<Config>,
    ledger: Mutex<Ledger>,
    feed: Mutex<FeedIndex>,
    asks: Mutex<HashMap<String, Pending>>,
    grants: Mutex<HashMap<String, Instant>>,
    quiet: Mutex<HashMap<String, (Instant, Reason)>>,
    recent_denies: Mutex<HashMap<String, (Instant, u64)>>,
    /// Serialises refusals so two identical ones arriving together are counted on one row.
    deny_order: Mutex<()>,
    health: Mutex<HashMap<String, Health>>,
    running: Mutex<HashSet<String>>,
    listen_errors: Mutex<Vec<String>>,
    last_blocked: Mutex<Option<Instant>>,
    chain: Mutex<Option<ChainCheck>>,
    tx: broadcast::Sender<Event>,
    dirty: Notify,
    /// Bumped by the panic button: every open connection watches it and drops when it changes.
    cut: watch::Sender<u64>,
}

#[derive(Clone)]
pub struct Engine(Arc<Inner>);

fn key(app: &str, host: &str) -> String {
    format!("{app}|{host}")
}

impl Engine {
    /// Load settings, open the ledger, rebuild history, start the gates and the background tasks.
    /// Must be called inside a Tokio runtime.
    pub async fn start() -> Result<Engine, String> {
        paths::ensure_data_dir().map_err(|e| format!("Could not create the data folder: {e}"))?;
        let cfg_path = paths::config_file();
        let existed = cfg_path.exists();
        let cfg = Config::load(&cfg_path);
        let (ledger, records) = Ledger::open(&paths::ledger_file(), &paths::ledger_key()).map_err(|e| format!("Could not open the log: {e}"))?;
        let mut feed = FeedIndex::default();
        for r in &records {
            feed.apply(r);
        }
        feed.close_stale();
        let (tx, _) = broadcast::channel(1024);
        let engine = Engine(Arc::new(Inner {
            cfg: Mutex::new(cfg),
            ledger: Mutex::new(ledger),
            feed: Mutex::new(feed),
            asks: Mutex::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
            quiet: Mutex::new(HashMap::new()),
            recent_denies: Mutex::new(HashMap::new()),
            deny_order: Mutex::new(()),
            health: Mutex::new(HashMap::new()),
            running: Mutex::new(HashSet::new()),
            listen_errors: Mutex::new(Vec::new()),
            last_blocked: Mutex::new(None),
            chain: Mutex::new(None),
            tx,
            dirty: Notify::new(),
            cut: watch::channel(0).0,
        }));
        if !existed {
            engine.save_cfg();
            engine.append("mode", fields! { "from" => Value::Null, "to" => "private", "why" => "first run" });
        }

        // One gate per app.
        let apps: Vec<(String, u16)> = engine.0.cfg.lock().unwrap().apps.iter().map(|a| (a.id.clone(), a.proxy_port)).collect();
        for (id, port) in apps {
            match crate::proxy::bind(port).await {
                Ok(listener) => {
                    tokio::spawn(crate::proxy::serve(engine.clone(), id, listener));
                }
                Err(e) => engine.0.listen_errors.lock().unwrap().push(format!("Port {port} is in use by another program ({e}).")),
            }
        }

        // Status emitter: coalesces bursts, and re-checks on a slow tick so time-based state
        // (the "blocked" indicator fading) is noticed.
        let e = engine.clone();
        tokio::spawn(async move {
            let mut last = String::new();
            loop {
                tokio::select! {
                    _ = e.0.dirty.notified() => { tokio::time::sleep(Duration::from_millis(150)).await; }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                let s = e.status();
                let ser = serde_json::to_string(&s).unwrap_or_default();
                if ser != last {
                    last = ser;
                    let _ = e.0.tx.send(Event::Status(s));
                }
            }
        });

        // Fence health: now, then every half minute.
        let e = engine.clone();
        tokio::spawn(async move {
            loop {
                e.check_fence().await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        Ok(engine)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.tx.subscribe()
    }

    pub fn emit(&self, e: Event) {
        let _ = self.0.tx.send(e);
    }

    fn touch(&self) {
        self.0.dirty.notify_one();
    }

    /// Open connections hold one of these and stop when it changes.
    pub fn cut_signal(&self) -> watch::Receiver<u64> {
        self.0.cut.subscribe()
    }

    pub(crate) fn record_refusal(&self, app_id: &str, host: &str, port: u16, reason: Reason, detail: &str, other: &str) {
        self.record_deny_detail(app_id, host, port, reason, detail, other);
    }

    fn save_cfg(&self) {
        let cfg = self.0.cfg.lock().unwrap().clone();
        if let Err(e) = cfg.save(&paths::config_file()) {
            eprintln!("sandbox-ai: could not save settings: {e}");
        }
    }

    /// Write a ledger record, fold it into the Feed, and tell the UI about the row it touched.
    fn append(&self, kind: &str, fields: Map<String, Value>) -> Option<Value> {
        let rec = match self.0.ledger.lock().unwrap().append(kind, fields) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sandbox-ai: could not write to the log: {e}");
                return None;
            }
        };
        let changed = self.0.feed.lock().unwrap().apply(&rec);
        if let Some(id) = changed {
            if let Some(ev) = self.feed_event(id) {
                let _ = self.0.tx.send(Event::Feed(ev));
            }
        }
        self.touch();
        Some(rec)
    }

    // -----------------------------------------------------------------------------------------
    // Decisions
    // -----------------------------------------------------------------------------------------

    /// Decide one connection. May wait up to 30 seconds for the user to answer a prompt.
    pub async fn decide_conn(&self, app_id: &str, raw_host: &str, port: u16) -> Decision {
        let deny = |reason: Reason, host: Option<String>, message: &str| Decision { allow: false, reason, ask: None, host, message: message.to_string() };

        let Some(host) = hostname::canonical(raw_host) else {
            let shown: String = raw_host.chars().filter(|c| !c.is_control()).take(120).collect();
            self.record_deny(app_id, &shown, port, Reason::Malformed);
            return deny(Reason::Malformed, None, "Blocked by Sandbox-ai: this is not an ordinary destination name.");
        };
        if port != 443 && port != 80 {
            self.record_deny_detail(app_id, &host, port, Reason::Malformed, "port", "");
            return deny(Reason::Malformed, Some(host), "Blocked by Sandbox-ai: only ordinary web connections are possible from a protected app.");
        }

        let found = {
            let cfg = self.0.cfg.lock().unwrap();
            cfg.app(app_id).map(|a| (cfg.mode, cfg.lockdown, cfg.ask, a.clone(), a.rule_for(&host).cloned()))
        };
        let Some((mode, lockdown, ask_enabled, app, rule)) = found else {
            self.record_deny(app_id, &host, port, Reason::NotTrusted);
            return deny(Reason::NotTrusted, Some(host), "Blocked by Sandbox-ai.");
        };
        let (label, _) = catalog::describe(&host);
        let refused = format!("Blocked by Sandbox-ai: {} is not a trusted destination for {}.", label, app.name);
        let allow = |reason: Reason, ask: Option<u64>| Decision { allow: true, reason, ask, host: Some(host.clone()), message: String::new() };

        if lockdown {
            self.record_deny(app_id, &host, port, Reason::Lockdown);
            return deny(Reason::Lockdown, Some(host), "Blocked by Sandbox-ai: everything is blocked right now.");
        }
        match mode {
            Mode::Off => return allow(Reason::ProtectionOff, None),
            Mode::Watch => return allow(Reason::Watching, None),
            Mode::Private => {}
        }
        match rule {
            Some(r) if r.enabled => return allow(Reason::Trusted, None),
            Some(r) => {
                let reason = if r.source == "you" { Reason::YouBlocked } else { Reason::NotTrusted };
                self.record_deny(app_id, &host, port, reason);
                return deny(reason, Some(host), &refused);
            }
            None => {}
        }

        let k = key(app_id, &host);
        if self.0.grants.lock().unwrap().get(&k).map(|until| *until > Instant::now()).unwrap_or(false) {
            return allow(Reason::YouAllowedOnce, None);
        }
        let quiet = self.0.quiet.lock().unwrap().get(&k).filter(|(until, _)| *until > Instant::now()).map(|(_, r)| *r);
        if let Some(reason) = quiet {
            self.record_deny(app_id, &host, port, reason);
            return deny(reason, Some(host), &refused);
        }
        if !ask_enabled {
            self.record_deny(app_id, &host, port, Reason::NotTrusted);
            return deny(Reason::NotTrusted, Some(host), &refused);
        }

        let (choice, seq) = self.ask(&app, &host, port).await;
        match choice {
            Choice::Once => allow(Reason::YouAllowedOnce, Some(seq)),
            Choice::Always => allow(Reason::YouAlwaysAllow, Some(seq)),
            Choice::Block => deny(Reason::YouBlocked, Some(host), &refused),
            Choice::Timeout => deny(Reason::NoAnswer, Some(host), &refused),
        }
    }

    /// Record a refusal. Identical refusals within a minute are counted on the first one's row
    /// rather than flooding the Feed (some apps retry every second).
    fn record_deny(&self, app_id: &str, host: &str, port: u16, reason: Reason) {
        self.record_deny_detail(app_id, host, port, reason, "", "");
    }

    fn record_deny_detail(&self, app_id: &str, host: &str, port: u16, reason: Reason, detail: &str, other: &str) {
        *self.0.last_blocked.lock().unwrap() = Some(Instant::now());
        let _in_order = self.0.deny_order.lock().unwrap();
        let k = format!("{app_id}|{host}|{port}|{}|{detail}|{other}", reason.as_str());
        let repeat_of = {
            let mut recent = self.0.recent_denies.lock().unwrap();
            recent.retain(|_, (t, _)| t.elapsed() < REPEAT_WINDOW);
            recent.get(&k).map(|(_, id)| *id)
        };
        if let Some(id) = repeat_of {
            if self.0.feed.lock().unwrap().add_repeat(id) {
                if let Some(ev) = self.feed_event(id) {
                    let _ = self.0.tx.send(Event::Feed(ev));
                }
                self.touch();
                return;
            }
        }
        let mut f = fields! { "app" => app_id, "host" => host, "port" => port, "decision" => "deny", "reason" => reason.as_str() };
        if !detail.is_empty() {
            f.insert("detail".into(), json!(detail));
        }
        if !other.is_empty() {
            f.insert("other".into(), json!(other));
        }
        if let Some(rec) = self.append("conn", f) {
            if let Some(seq) = rec.get("seq").and_then(Value::as_u64) {
                self.0.recent_denies.lock().unwrap().insert(k, (Instant::now(), seq));
            }
        }
    }

    /// Called by the gate once an allowed connection is actually established (or failed to be).
    pub fn record_conn(&self, app_id: &str, host: &str, ip: &str, port: u16, d: &Decision) -> Option<u64> {
        let mut f = fields! { "app" => app_id, "host" => host, "ip" => ip, "port" => port, "decision" => "allow", "reason" => d.reason.as_str() };
        if let Some(a) = d.ask {
            f.insert("ask".into(), json!(a));
        }
        self.append("conn", f).and_then(|r| r.get("seq").and_then(Value::as_u64))
    }

    pub fn record_end(&self, conn: u64, bytes_out: u64, bytes_in: u64, ms: u64, unreachable: bool) {
        let mut f = fields! { "conn" => conn, "out" => bytes_out, "in" => bytes_in, "ms" => ms };
        if unreachable {
            f.insert("unreachable".into(), json!(true));
        }
        self.append("end", f);
    }

    /// A name that passed the rules but points at this computer or the local network.
    pub fn record_local_address(&self, app_id: &str, host: &str, port: u16) {
        self.record_deny(app_id, host, port, Reason::LocalAddress);
    }

    async fn ask(&self, app: &AppConfig, host: &str, port: u16) -> (Choice, u64) {
        let k = key(&app.id, host);
        let (mut rx, seq, created) = {
            let mut asks = self.0.asks.lock().unwrap();
            if let Some(p) = asks.get_mut(&k) {
                p.ask.waiting += 1;
                (p.tx.subscribe(), p.seq, None)
            } else {
                let rec = self.append("ask", fields! { "app" => app.id, "host" => host, "port" => port });
                let seq = rec.as_ref().and_then(|r| r.get("seq")).and_then(Value::as_u64).unwrap_or(0);
                let (label, purpose) = catalog::describe(host);
                let now = chrono::Utc::now();
                let title = if purpose == "Model download" { format!("{} wants to download a model", app.name) } else { format!("{} wants to reach {}", app.name, label) };
                let body = match (purpose.as_str(), label == host) {
                    ("Model download", _) => format!("From {label} ({host}). Allow?"),
                    ("", _) => "Sandbox-ai does not know this place. Allow?".to_string(),
                    (p, true) => format!("{p}. Allow?"),
                    (p, false) => format!("{p} ({host}). Allow?"),
                };
                let ask = Ask {
                    id: seq.to_string(),
                    app_id: app.id.clone(),
                    app_name: app.name.clone(),
                    app_icon: app.icon.clone(),
                    host: host.to_string(),
                    port,
                    label,
                    purpose,
                    title,
                    body,
                    created_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    expires_at: (now + chrono::Duration::from_std(ASK_TIMEOUT).unwrap()).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    waiting: 1,
                };
                let (tx, rx) = watch::channel(None);
                asks.insert(k.clone(), Pending { ask: ask.clone(), seq, tx });
                (rx, seq, Some(ask))
            }
        };
        match created {
            Some(ask) => {
                let _ = self.0.tx.send(Event::Ask(ask.clone()));
                let e = self.clone();
                let id = ask.id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ASK_TIMEOUT).await;
                    let _ = e.resolve_ask(&id, Choice::Timeout);
                });
            }
            None => {
                let updated = self.0.asks.lock().unwrap().get(&k).map(|p| p.ask.clone());
                if let Some(a) = updated {
                    let _ = self.0.tx.send(Event::Ask(a));
                }
            }
        }
        self.touch();
        loop {
            if let Some(c) = *rx.borrow() {
                return (c, seq);
            }
            if rx.changed().await.is_err() {
                return (Choice::Timeout, seq);
            }
        }
    }

    /// Answer a prompt (or expire it).
    pub fn resolve_ask(&self, id: &str, choice: Choice) -> Result<(), String> {
        let p = {
            let mut asks = self.0.asks.lock().unwrap();
            let k = asks.iter().find(|(_, p)| p.ask.id == id).map(|(k, _)| k.clone());
            k.and_then(|k| asks.remove(&k))
        }
        .ok_or_else(|| "That question has already been answered.".to_string())?;

        self.append("answer", fields! { "ask" => p.seq, "choice" => choice.as_str(), "waiting" => p.ask.waiting });
        let app = p.ask.app_id.clone();
        let host = p.ask.host.clone();
        match choice {
            Choice::Always => {
                let _ = self.set_trust_inner(&app, &host, true, "prompt");
                for c in catalog::companions(&host) {
                    let _ = self.set_trust_inner(&app, c, true, "prompt");
                }
            }
            Choice::Once => {
                let until = Instant::now() + ONCE_GRANT;
                let mut g = self.0.grants.lock().unwrap();
                g.insert(key(&app, &host), until);
                for c in catalog::companions(&host) {
                    g.insert(key(&app, c), until);
                }
            }
            Choice::Block => {
                let _ = self.set_trust_inner(&app, &host, false, "prompt");
                *self.0.last_blocked.lock().unwrap() = Some(Instant::now());
            }
            Choice::Timeout => {
                self.0.quiet.lock().unwrap().insert(key(&app, &host), (Instant::now() + QUIET_AFTER_NO, Reason::NoAnswer));
                *self.0.last_blocked.lock().unwrap() = Some(Instant::now());
            }
        }
        let _ = self.0.tx.send(Event::AskResolved { id: id.to_string(), choice: choice.as_str().to_string() });
        let _ = p.tx.send(Some(choice));
        self.touch();
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Trusted destinations
    // -----------------------------------------------------------------------------------------

    fn set_trust_inner(&self, app_id: &str, pattern: &str, enabled: bool, via: &str) -> Result<(), String> {
        if !hostname::valid_pattern(pattern) {
            return Err("That is not a destination name. Use a name like example.com or *.example.com.".into());
        }
        let pattern = hostname::normalize_pattern(pattern);
        let changed = {
            let mut cfg = self.0.cfg.lock().unwrap();
            let app = cfg.app_mut(app_id).ok_or_else(|| "Unknown app.".to_string())?;
            match app.trust.iter_mut().find(|r| r.host == pattern) {
                Some(r) if r.enabled == enabled => false,
                Some(r) => {
                    r.enabled = enabled;
                    true
                }
                None => {
                    app.trust.push(TrustRule { host: pattern.clone(), enabled, source: "you".into(), recommended: false });
                    true
                }
            }
        };
        if changed {
            self.save_cfg();
            self.append("trust", fields! { "app" => app_id, "host" => pattern, "enabled" => enabled, "via" => via });
        }
        // A fresh decision replaces any remembered answer for matching destinations.
        let prefix = format!("{app_id}|");
        let matches = |k: &String| k.strip_prefix(&prefix).map(|h| h == pattern || hostname::matches(&pattern, h)).unwrap_or(false);
        self.0.quiet.lock().unwrap().retain(|k, _| !matches(k));
        self.0.grants.lock().unwrap().retain(|k, _| !matches(k));
        self.0.recent_denies.lock().unwrap().retain(|k, _| !k.starts_with(&prefix));
        self.touch();
        Ok(())
    }

    pub fn trust_list(&self, app_id: Option<&str>) -> Vec<TrustEntry> {
        let cfg = self.0.cfg.lock().unwrap();
        let mut out = Vec::new();
        for a in cfg.apps.iter().filter(|a| app_id.map(|id| id == a.id).unwrap_or(true)) {
            for r in &a.trust {
                let (label, purpose) = catalog::describe_pattern(&r.host);
                out.push(TrustEntry { app_id: a.id.clone(), host: r.host.clone(), label, purpose, enabled: r.enabled, source: r.source.clone(), recommended: r.recommended });
            }
        }
        out
    }

    fn remove_trust(&self, app_id: &str, pattern: &str) -> Result<(), String> {
        let pattern = hostname::normalize_pattern(pattern);
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let app = cfg.app_mut(app_id).ok_or_else(|| "Unknown app.".to_string())?;
            let before = app.trust.len();
            app.trust.retain(|r| !(r.host == pattern && r.source == "you"));
            if app.trust.len() == before {
                return Err("Only destinations you added can be removed. Built-in ones can be switched off.".into());
            }
        }
        self.save_cfg();
        self.append("trust", fields! { "app" => app_id, "host" => pattern, "removed" => true, "via" => "settings" });
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Mode
    // -----------------------------------------------------------------------------------------

    fn set_mode(&self, mode: Mode) {
        let from = {
            let mut cfg = self.0.cfg.lock().unwrap();
            if cfg.mode == mode {
                return;
            }
            let from = cfg.mode;
            let now = now_iso();
            cfg.mode = mode;
            cfg.mode_since = now.clone();
            cfg.private_since = if mode == Mode::Private { Some(now) } else { None };
            from
        };
        self.save_cfg();
        self.append("mode", fields! { "from" => from.as_str(), "to" => mode.as_str() });
        if mode != Mode::Private {
            self.cancel_asks();
        }
    }

    fn set_lockdown(&self, on: bool) {
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            if cfg.lockdown == on {
                return;
            }
            cfg.lockdown = on;
        }
        self.save_cfg();
        self.append("lockdown", fields! { "on" => on });
        if on {
            self.cancel_asks();
            self.0.grants.lock().unwrap().clear();
            self.0.cut.send_modify(|v| *v += 1);
        }
    }

    /// Pending prompts make no sense once the mode changes: answer them the safe way.
    fn cancel_asks(&self) {
        let ids: Vec<String> = self.0.asks.lock().unwrap().values().map(|p| p.ask.id.clone()).collect();
        for id in ids {
            let _ = self.resolve_ask(&id, Choice::Timeout);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------------------------

    fn explain(row: &Row, app_name: &str, label: &str) -> String {
        if row.verb == Verb::Asked {
            return "Waiting for your answer.".into();
        }
        let mut s = match row.reason {
            Reason::Trusted => format!("{label} is on {app_name}'s trusted list."),
            Reason::YouAllowedOnce => "You allowed this once.".into(),
            Reason::YouAlwaysAllow => "You chose to always allow this.".into(),
            Reason::YouBlocked => "You blocked this.".into(),
            Reason::NotTrusted => format!("{label} is switched off for {app_name}, so it was blocked without asking."),
            Reason::NoAnswer => "Nobody answered within 30 seconds, so it was blocked.".into(),
            Reason::Watching => "Watch mode is on, so this was allowed and recorded.".into(),
            Reason::ProtectionOff => "Protection is off, so this was allowed and recorded.".into(),
            Reason::Lockdown => "Everything was blocked at the time.".into(),
            Reason::Malformed => match row.detail.as_str() {
                "port" => format!("Protected apps can only make ordinary web connections, so port {} was blocked.", row.port),
                "greeting" => format!("{app_name} asked for {label} and then tried to reach {} instead, so the connection was cut.", row.other),
                "no_name" => format!("{app_name} asked for {label} but would not say who it was greeting, so the connection was cut."),
                "not_encrypted" => format!("{app_name} asked for {label} but did not start an ordinary encrypted connection, so it was cut."),
                _ => "This was not an ordinary destination name, so it was blocked.".into(),
            },
            Reason::LocalAddress => "This name points at your own computer or network, so it was blocked.".into(),
        };
        if row.unreachable {
            s.push_str(" The destination did not answer.");
        }
        if row.repeats > 0 {
            s.push_str(&format!(" Tried {} more time{} within a minute.", row.repeats, if row.repeats == 1 { "" } else { "s" }));
        }
        s
    }

    fn to_event(row: &Row, cfg: &Config) -> FeedEvent {
        let app = cfg.app(&row.app);
        let app_name = app.map(|a| a.name.clone()).unwrap_or_else(|| row.app.clone());
        let (label, purpose) = catalog::describe(&row.host);
        let trusted = app.and_then(|a| a.rule_for(&row.host)).map(|r| r.enabled).unwrap_or(false);
        FeedEvent {
            id: row.id,
            ts: row.ts.clone(),
            app_id: row.app.clone(),
            app_icon: app.map(|a| a.icon.clone()).unwrap_or_else(|| "generic".into()),
            explanation: Self::explain(row, &app_name, &label),
            app_name,
            host: row.host.clone(),
            port: row.port,
            label,
            purpose,
            verb: row.verb,
            reason: row.reason,
            open: row.open,
            bytes_out: row.bytes_out,
            bytes_in: row.bytes_in,
            duration_ms: row.duration_ms,
            trusted,
        }
    }

    fn feed_event(&self, id: u64) -> Option<FeedEvent> {
        let row = self.0.feed.lock().unwrap().get(id).cloned()?;
        let cfg = self.0.cfg.lock().unwrap();
        Some(Self::to_event(&row, &cfg))
    }

    pub fn feed(&self, limit: usize, before_id: Option<u64>, app_id: Option<&str>, verb: Option<Verb>) -> Vec<FeedEvent> {
        let rows: Vec<Row> = {
            let feed = self.0.feed.lock().unwrap();
            feed.rows().iter().rev().filter(|r| before_id.map(|b| r.id < b).unwrap_or(true)).filter(|r| app_id.map(|a| a == r.app).unwrap_or(true)).filter(|r| verb.map(|v| v == r.verb).unwrap_or(true)).take(limit).cloned().collect()
        };
        let cfg = self.0.cfg.lock().unwrap();
        rows.iter().map(|r| Self::to_event(r, &cfg)).collect()
    }

    fn local_midnight_utc() -> chrono::DateTime<chrono::Utc> {
        let now = chrono::Local::now();
        now.date_naive().and_hms_opt(0, 0, 0).and_then(|n| n.and_local_timezone(chrono::Local).earliest()).map(|d| d.with_timezone(&chrono::Utc)).unwrap_or_else(|| now.with_timezone(&chrono::Utc))
    }

    fn count(row: &Row, c: &mut Counters) {
        match row.verb {
            Verb::Allowed | Verb::Seen => c.allowed += 1,
            Verb::Blocked => c.blocked += 1 + row.repeats,
            Verb::Asked => {}
        }
        if row.asked {
            c.asked += 1;
        }
        c.bytes_out += row.bytes_out.unwrap_or(0);
        c.bytes_in += row.bytes_in.unwrap_or(0);
    }

    pub fn status(&self) -> Status {
        let cfg = self.0.cfg.lock().unwrap().clone();
        let health = self.0.health.lock().unwrap().clone();
        let running = self.0.running.lock().unwrap().clone();
        let mut pending: Vec<Ask> = self.0.asks.lock().unwrap().values().map(|p| p.ask.clone()).collect();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let midnight = Self::local_midnight_utc().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let watching = cfg.mode == Mode::Watch;
        let floor = if watching && cfg.mode_since < midnight { cfg.mode_since.clone() } else { midnight.clone() };
        let mut today = Counters::default();
        let mut per_app: HashMap<String, Counters> = HashMap::new();
        let mut watch_hosts: HashSet<String> = HashSet::new();
        {
            // Timestamps share one fixed format, so comparing the strings compares the times.
            let feed = self.0.feed.lock().unwrap();
            for row in feed.rows().iter().rev() {
                if row.ts < floor {
                    break;
                }
                if watching && row.verb == Verb::Seen && row.ts >= cfg.mode_since {
                    watch_hosts.insert(row.host.clone());
                }
                if row.ts >= midnight {
                    Self::count(row, &mut today);
                    Self::count(row, per_app.entry(row.app.clone()).or_default());
                }
            }
        }

        let mut apps = Vec::new();
        let mut reasons: Vec<String> = self.0.listen_errors.lock().unwrap().clone();
        let mut states: Vec<FenceState> = Vec::new();
        for a in &cfg.apps {
            let h = health.get(&a.id).cloned().unwrap_or_default();
            let state = h.state();
            let installed = if h.checked { h.installed } else { catalog::find_binary(&a.binary).is_some() };
            if a.protected && installed {
                states.push(state);
                for r in h.reasons(&a.name) {
                    if !reasons.contains(&r) {
                        reasons.push(r);
                    }
                }
            }
            apps.push(AppStatus {
                id: a.id.clone(),
                name: a.name.clone(),
                kind: a.kind.clone(),
                icon: a.icon.clone(),
                installed,
                protected: a.protected,
                fence: state,
                running: running.contains(&a.id),
                can_start: installed && a.protected && state == FenceState::Up,
                workspace: a.workspace.clone(),
                today: per_app.get(&a.id).copied().unwrap_or_default(),
            });
        }
        let fence = if states.iter().any(|s| *s == FenceState::Checking) {
            FenceState::Checking
        } else if states.iter().all(|s| *s == FenceState::Up) {
            FenceState::Up
        } else if states.iter().all(|s| *s == FenceState::NotInstalled) {
            FenceState::NotInstalled
        } else {
            FenceState::Down
        };
        let setup_needed = matches!(fence, FenceState::Down | FenceState::NotInstalled) || !self.0.listen_errors.lock().unwrap().is_empty();
        if fence == FenceState::NotInstalled {
            reasons = vec!["Sandbox-ai has not been set up on this computer yet.".into()];
        }

        let recently_blocked = self.0.last_blocked.lock().unwrap().map(|t| t.elapsed() < BLOCKED_INDICATOR).unwrap_or(false);
        let indicator = if setup_needed {
            Indicator::Attention
        } else if !pending.is_empty() {
            Indicator::Asking
        } else if cfg.lockdown || (recently_blocked && cfg.mode == Mode::Private) {
            Indicator::Blocked
        } else {
            match cfg.mode {
                Mode::Private => Indicator::Private,
                Mode::Watch => Indicator::Watching,
                Mode::Off => Indicator::Off,
            }
        };
        let plural = |n: u64, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        let sentence = if setup_needed {
            reasons.first().cloned().unwrap_or_else(|| "Protection needs your attention.".into())
        } else if cfg.lockdown {
            "Everything is blocked.".into()
        } else if let Some(a) = pending.first() {
            format!("{} is asking to reach {}.", a.app_name, a.label)
        } else {
            match cfg.mode {
                Mode::Private if today.blocked > 0 => format!("Private. {} blocked today.", plural(today.blocked, "attempt", "attempts")),
                Mode::Private if today.allowed > 0 => "Private. Only trusted places were contacted today.".into(),
                Mode::Private => "Private. Nothing has tried to leave today.".into(),
                Mode::Watch => format!("Watching. {} seen.", plural(watch_hosts.len() as u64, "place", "places")),
                Mode::Off => "Protection is off.".into(),
            }
        };

        Status {
            version: crate::VERSION.to_string(),
            mode: cfg.mode,
            mode_since: cfg.mode_since.clone(),
            private_since: cfg.private_since.clone(),
            lockdown: cfg.lockdown,
            fence,
            setup_needed,
            setup_reasons: reasons,
            indicator,
            sentence,
            today,
            apps,
            pending,
        }
    }

    pub fn proof(&self) -> Proof {
        let cfg = self.0.cfg.lock().unwrap().clone();
        let status = self.status();
        let since = cfg.private_since.clone().filter(|_| cfg.mode == Mode::Private);
        let (mut blocked, mut allowed, mut untrusted) = (0u64, 0u64, 0u64);
        if let Some(s) = &since {
            let feed = self.0.feed.lock().unwrap();
            for row in feed.rows().iter().rev().take_while(|r| r.ts >= *s) {
                match row.verb {
                    Verb::Blocked => blocked += 1 + row.repeats,
                    Verb::Allowed => allowed += 1,
                    Verb::Seen => {
                        allowed += 1;
                        untrusted += row.bytes_out.unwrap_or(0);
                    }
                    Verb::Asked => {}
                }
            }
        }
        let elapsed = since.as_deref().and_then(parse_iso).map(|t| (chrono::Utc::now() - t).num_seconds().max(0) as u64).unwrap_or(0);
        let unit = |n: u64, u: &str| format!("{n} {u}{}", if n == 1 { "" } else { "s" });
        let streak_text = match elapsed {
            s if s >= 86_400 => unit(s / 86_400, "day"),
            s if s >= 3_600 => unit(s / 3_600, "hour"),
            s if s >= 60 => unit(s / 60, "minute"),
            _ => "Just now".to_string(),
        };
        let protected: Vec<String> = status.apps.iter().filter(|a| a.protected && a.installed && a.fence == FenceState::Up).map(|a| a.name.clone()).collect();
        let names = match protected.len() {
            0 => "your protected apps".to_string(),
            1 => protected[0].clone(),
            _ => format!("{} and {}", protected[..protected.len() - 1].join(", "), protected[protected.len() - 1]),
        };
        let statement = match (&since, status.setup_needed) {
            (_, true) => "Protection is not fully in place right now, so there is nothing to prove yet.".to_string(),
            (None, _) => "Private mode is not on, so there is nothing to prove right now.".to_string(),
            (Some(s), _) => {
                let date = parse_iso(s).map(|t| t.with_timezone(&chrono::Local).format("%-d %b %Y, %H:%M").to_string()).unwrap_or_default();
                if untrusted == 0 {
                    format!("Since {date}, {names} sent nothing to any place you have not allowed.")
                } else {
                    format!("Since {date}, {names} sent {untrusted} bytes to places that are not on a trusted list.")
                }
            }
        };
        Proof {
            mode: cfg.mode,
            private_since: since,
            streak_days: elapsed / 86_400,
            streak_text,
            apps_protected: protected.len() as u32,
            app_names: protected,
            bytes_out_today: status.today.bytes_out,
            untrusted_bytes_since_private: untrusted,
            blocked_total: blocked,
            allowed_total: allowed,
            chain: self.0.chain.lock().unwrap().clone(),
            public_key: self.0.ledger.lock().unwrap().public_key_b64(),
            statement,
            covers: "Every connection your protected apps made or tried to make: which app, where it was going, whether it was allowed, and how much was sent. Every entry is signed by this computer and chained to the one before it, so nothing can be removed or changed without the check failing.".into(),
            cannot_see: "What was inside the data sent to places you trust: if you trust your AI provider, what you type goes there. Apps you have not protected. Anything done by someone who already controls this computer. Very slow hidden signals, such as timing, are not measured in this version.".into(),
        }
    }

    pub fn verify_chain(&self) -> ChainCheck {
        let (path, pk) = {
            let l = self.0.ledger.lock().unwrap();
            (l.path().to_path_buf(), l.public_key_b64())
        };
        let check = ledger::verify(&path, &pk);
        *self.0.chain.lock().unwrap() = Some(check.clone());
        self.touch();
        check
    }

    fn export_proof(&self) -> Result<String, String> {
        let check = self.verify_chain();
        let proof = self.proof();
        let (count, head) = self.0.ledger.lock().unwrap().head();
        let body = json!({
            "format": "sandboxai-proof-v1",
            "generatedAt": now_iso(),
            "statement": proof.statement,
            "privateSince": proof.private_since,
            "apps": proof.app_names,
            "blockedAttempts": proof.blocked_total,
            "allowedConnections": proof.allowed_total,
            "untrustedBytes": proof.untrusted_bytes_since_private,
            "log": { "records": count, "head": head, "verified": check.ok, "error": check.error },
            "publicKey": proof.public_key,
            "covers": proof.covers,
            "cannotSee": proof.cannot_see,
        });
        let signature = self.0.ledger.lock().unwrap().sign(ledger::canonical(&body).as_bytes());
        let file = json!({ "proof": body, "signature": signature, "how_to_check": "The signature is Ed25519 over the canonical JSON (sorted keys, no spaces) of \"proof\", made with \"publicKey\"." });
        let name = format!("Sandbox-ai proof {}.json", chrono::Local::now().format("%Y-%m-%d %H.%M"));
        let path = paths::downloads_dir().join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("The proof could not be saved: {e}"))?;
        Ok(path.to_string_lossy().into_owned())
    }

    // -----------------------------------------------------------------------------------------
    // Fence and apps
    // -----------------------------------------------------------------------------------------

    pub async fn check_fence(&self) {
        let (apps, expected_rules) = {
            let cfg = self.0.cfg.lock().unwrap();
            (cfg.apps.clone(), fence::filter_rules(&cfg))
        };
        for a in &apps {
            let h = fence::probe(a, &expected_rules).await;
            // A service we did not start may already be answering (for example after an app restart).
            if let Some(p) = a.serves_port {
                let up = fence::is_port_open(p).await;
                let mut running = self.0.running.lock().unwrap();
                if !up {
                    running.remove(&a.id);
                }
            }
            self.0.health.lock().unwrap().insert(a.id.clone(), h);
        }
        self.touch();
    }

    fn app_if_ready(&self, app_id: &str) -> Result<AppConfig, String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or_else(|| "Unknown app.".to_string())?;
        if !app.protected {
            return Err(format!("{} is not protected. Switch protection on for it first.", app.name));
        }
        let h = self.0.health.lock().unwrap().get(app_id).cloned().unwrap_or_default();
        if h.state() != FenceState::Up {
            return Err(h.reasons(&app.name).first().cloned().unwrap_or_else(|| "Protection is still being checked. Try again in a moment.".into()));
        }
        Ok(app)
    }

    async fn app_start(&self, app_id: &str) -> Result<(), String> {
        let app = self.app_if_ready(app_id)?;
        if self.0.running.lock().unwrap().contains(app_id) {
            return Ok(());
        }
        if let Some(p) = app.serves_port {
            if fence::is_port_open(p).await {
                return Err(format!("Something is already running on this computer where {} would be. Quit the other {} first, so only the protected one runs.", app.name, app.name));
            }
        }
        let mut child = fence::start_managed(&app).await?;
        self.0.running.lock().unwrap().insert(app_id.to_string());
        self.append("start", fields! { "app" => app_id, "action" => "start" });
        let e = self.clone();
        let id = app_id.to_string();
        tokio::spawn(async move {
            let _ = child.wait().await;
            e.0.running.lock().unwrap().remove(&id);
            e.append("start", fields! { "app" => id, "action" => "stop" });
        });
        Ok(())
    }

    async fn app_stop(&self, app_id: &str) -> Result<(), String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or_else(|| "Unknown app.".to_string())?;
        fence::stop_all(&app).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.touch();
        Ok(())
    }

    /// Stop everything we started. Called when the app quits.
    pub async fn shutdown(&self) {
        let running: Vec<String> = self.0.running.lock().unwrap().iter().cloned().collect();
        for id in running {
            let _ = self.app_stop(&id).await;
        }
    }

    // -----------------------------------------------------------------------------------------
    // The command surface (docs/ipc.md). One implementation for the app, the CLI and scripts.
    // -----------------------------------------------------------------------------------------

    pub async fn dispatch(&self, cmd: &str, args: Value) -> Result<Value, String> {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let req = |k: &str| s(k).ok_or_else(|| format!("Missing \"{k}\"."));
        let b = |k: &str| args.get(k).and_then(Value::as_bool).ok_or_else(|| format!("Missing \"{k}\"."));
        let ok = |v: &dyn erased::Ser| v.to_value();
        match cmd {
            "get_status" => ok(&self.status()),
            "set_mode" => {
                let mode = Mode::parse(&req("mode")?).ok_or("Unknown mode.")?;
                self.set_mode(mode);
                ok(&self.status())
            }
            "set_lockdown" => {
                self.set_lockdown(b("on")?);
                ok(&self.status())
            }
            "get_feed" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200).min(2000) as usize;
                let verb = match s("verb").as_deref() {
                    Some("allowed") => Some(Verb::Allowed),
                    Some("blocked") => Some(Verb::Blocked),
                    Some("asked") => Some(Verb::Asked),
                    Some("seen") => Some(Verb::Seen),
                    _ => None,
                };
                ok(&self.feed(limit, args.get("beforeId").and_then(Value::as_u64), s("appId").as_deref(), verb))
            }
            "decide" => {
                let choice = Choice::parse(&req("choice")?).filter(|c| *c != Choice::Timeout).ok_or("Unknown answer.")?;
                self.resolve_ask(&req("askId")?, choice)?;
                Ok(Value::Null)
            }
            "get_trust" => ok(&self.trust_list(s("appId").as_deref())),
            "set_trust" => {
                let app = req("appId")?;
                let (host, enabled) = (req("host")?, b("enabled")?);
                self.set_trust_inner(&app, &host, enabled, "settings")?;
                // A question about this very destination is answered by this choice.
                let pattern = hostname::normalize_pattern(&host);
                let waiting: Vec<String> = self.0.asks.lock().unwrap().values().filter(|p| p.ask.app_id == app && (p.ask.host == pattern || hostname::matches(&pattern, &p.ask.host))).map(|p| p.ask.id.clone()).collect();
                for id in waiting {
                    let _ = self.resolve_ask(&id, if enabled { Choice::Always } else { Choice::Block });
                }
                ok(&self.trust_list(Some(&app)))
            }
            "remove_trust" => {
                let app = req("appId")?;
                self.remove_trust(&app, &req("host")?)?;
                ok(&self.trust_list(Some(&app)))
            }
            "get_proof" => ok(&self.proof()),
            "verify_chain" => {
                let e = self.clone();
                let check = tokio::task::spawn_blocking(move || e.verify_chain()).await.map_err(|e| e.to_string())?;
                ok(&check)
            }
            "export_proof" => {
                let e = self.clone();
                let path = tokio::task::spawn_blocking(move || e.export_proof()).await.map_err(|e| e.to_string())??;
                if args.get("reveal").and_then(Value::as_bool).unwrap_or(true) {
                    fence::reveal(&path, true).await;
                }
                Ok(json!({ "path": path }))
            }
            "get_receipts" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200).min(5000) as usize;
                ok(&self.0.ledger.lock().unwrap().tail(limit))
            }
            // Exactly what the administrator approves during setup, so it can be read first.
            "get_setup_script" => Ok(Value::String(fence::setup_script(&self.0.cfg.lock().unwrap(), &fence::real_user()))),
            "get_policy_text" => Ok(Value::String(serde_json::to_string_pretty(&*self.0.cfg.lock().unwrap()).unwrap_or_default())),
            "run_setup" => {
                let cfg = self.0.cfg.lock().unwrap().clone();
                let mut r = fence::run_setup(&cfg).await;
                self.check_fence().await;
                if r.ok {
                    let st = self.status();
                    if st.setup_needed {
                        r = SetupResult { ok: false, message: st.setup_reasons.first().cloned().unwrap_or_else(|| "Setup ran, but protection could not be confirmed.".into()) };
                    } else {
                        self.append("fence", fields! { "event" => "setup" });
                        r.message = "Protection is on.".into();
                    }
                }
                ok(&r)
            }
            "remove_setup" => {
                self.shutdown().await;
                let cfg = self.0.cfg.lock().unwrap().clone();
                let r = fence::remove_setup(&cfg).await;
                self.check_fence().await;
                if r.ok {
                    self.append("fence", fields! { "event" => "removed" });
                }
                ok(&r)
            }
            "check_fence" => {
                self.check_fence().await;
                ok(&self.status())
            }
            "app_set_protected" => {
                let id = req("appId")?;
                let on = b("protected")?;
                {
                    let mut cfg = self.0.cfg.lock().unwrap();
                    cfg.app_mut(&id).ok_or("Unknown app.")?.protected = on;
                }
                self.save_cfg();
                if !on {
                    let _ = self.app_stop(&id).await;
                }
                ok(&self.status())
            }
            "app_start" => {
                self.app_start(&req("appId")?).await?;
                tokio::time::sleep(Duration::from_millis(300)).await;
                ok(&self.status())
            }
            "app_stop" => {
                self.app_stop(&req("appId")?).await?;
                ok(&self.status())
            }
            "app_open_terminal" => {
                let app = self.app_if_ready(&req("appId")?)?;
                fence::open_terminal(&app).await?;
                Ok(Value::Null)
            }
            "app_open_workspace" => {
                let app = self.0.cfg.lock().unwrap().app(&req("appId")?).cloned().ok_or("Unknown app.")?;
                let w = app.workspace.ok_or("This app has no workspace folder.")?;
                if !std::path::Path::new(&w).is_dir() {
                    return Err("The workspace folder does not exist yet. Finish setup first.".into());
                }
                fence::reveal(&w, false).await;
                Ok(Value::Null)
            }
            // For the CLI and scripts: bring a window forward in the running app.
            "show" => {
                let window = s("window").unwrap_or_else(|| "main".into());
                if !matches!(window.as_str(), "main" | "glance" | "prompt") {
                    return Err("Unknown window.".into());
                }
                let _ = self.0.tx.send(Event::Show { window, tab: s("tab") });
                Ok(Value::Null)
            }
            // For the CLI: the exact command line that runs something protected, only if the fence is up.
            "fenced_argv" => {
                let app = self.app_if_ready(&req("appId")?)?;
                let program = req("program")?;
                let program = if program.contains('/') { program } else { catalog::find_binary(&program).unwrap_or(program) };
                let rest: Vec<String> = args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                ok(&fence::fenced_argv(&app, &program, &rest, &[], s("cwd").as_deref()))
            }
            other => Err(format!("Unknown command: {other}")),
        }
    }
}

/// Tiny helper so `dispatch` can turn any serialisable value into a `Result<Value, String>`.
mod erased {
    use serde_json::Value;
    pub trait Ser {
        fn to_value(&self) -> Result<Value, String>;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_value(&self) -> Result<Value, String> {
            serde_json::to_value(self).map_err(|e| e.to_string())
        }
    }
}
