//! The engine: one object that owns the settings, the ledger, the prompts and the fence state,
//! decides every connection, and exposes the command surface in docs/ipc.md.
//!
//! Lock order, where more than one is held: `asks` → `ledger` → `feed`. `cfg` is never held
//! across another lock or an await.

use crate::catalog;
use crate::config::{AppConfig, Config, KeyConfig, TrustRule};
use crate::feed::{FeedIndex, Row};
use crate::fence::{self, Health};
use crate::fields;
use crate::hostname;
use crate::projects;
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
/// How many unanswered questions are kept for answering later.
const MISSED_KEPT: usize = 12;
const DEFAULT_WATCH_MINUTES: u64 = 60;
const MAX_WATCH_MINUTES: u64 = 7 * 24 * 60;
const REPEAT_WINDOW: Duration = Duration::from_secs(60);
/// Attempts to go around the gate tend to come in bursts on every start; fold them for longer.
const AROUND_WINDOW: Duration = Duration::from_secs(600);
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
    /// Questions that ran out of time, kept so they can still be answered (see `Status::missed`).
    missed: Mutex<Vec<Ask>>,
    /// The real keys Homebody keeps for apps (see `keys`).
    vault: Mutex<crate::keys::Vault>,
    grants: Mutex<HashMap<String, Instant>>,
    quiet: Mutex<HashMap<String, (Instant, Reason)>>,
    recent_denies: Mutex<HashMap<String, (Instant, u64)>>,
    /// Serialises refusals so two identical ones arriving together are counted on one row.
    deny_order: Mutex<()>,
    /// Gate ports this engine is listening on.
    gates: Mutex<HashSet<u16>>,
    /// What each app has sent to each trusted place lately: (app, rule) -> (when, bytes).
    sent: Mutex<HashMap<(String, String), std::collections::VecDeque<(Instant, u64)>>>,
    /// Fingerprints of the lines in the user's private folders, and when they were last read.
    docs: Mutex<Option<(Instant, Arc<crate::secrets::DocIndex>)>>,
    /// This computer's certificate authority for looking inside, opened on first use.
    inspector: Mutex<Option<Arc<crate::inspect::Inspector>>>,
    /// The program launchers call to reach us: the desktop app's own executable.
    opener: Mutex<Option<String>>,
    /// Process ids of windowed apps we opened (app id -> pid), so we can ask exactly those to quit.
    pids: Mutex<HashMap<String, u32>>,
    /// Repeats counted on a row in memory but not yet written down (row id -> how many).
    unsaved_repeats: Mutex<HashMap<u64, u64>>,
    /// Flipped at shutdown so background helpers (the system-log follower) stop too.
    stop: watch::Sender<bool>,
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

/// Whether version `a` ("0.2.0") is newer than `b`. Anything that is not three numbers is not.
fn newer_version(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> {
        let parts: Vec<u64> = v.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        (parts.len() == 3).then_some(parts)
    };
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

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
            missed: Mutex::new(Vec::new()),
            vault: Mutex::new(crate::keys::Vault::open(&paths::keys_file())),
            grants: Mutex::new(HashMap::new()),
            quiet: Mutex::new(HashMap::new()),
            recent_denies: Mutex::new(HashMap::new()),
            deny_order: Mutex::new(()),
            gates: Mutex::new(HashSet::new()),
            pids: Mutex::new(HashMap::new()),
            opener: Mutex::new(None),
            inspector: Mutex::new(None),
            sent: Mutex::new(HashMap::new()),
            docs: Mutex::new(None),
            unsaved_repeats: Mutex::new(HashMap::new()),
            stop: watch::channel(false).0,
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
        let ports: Vec<u16> = engine.0.cfg.lock().unwrap().apps.iter().map(|a| a.proxy_port).collect();
        for port in ports {
            engine.ensure_gate(port).await;
        }

        // Status emitter: coalesces bursts, and re-checks on a slow tick so time-based state
        // (the "blocked" indicator fading) is noticed.
        let e = engine.clone();
        tokio::spawn(async move {
            let mut last = String::new();
            let mut flushed = Instant::now();
            loop {
                tokio::select! {
                    _ = e.0.dirty.notified() => { tokio::time::sleep(Duration::from_millis(150)).await; }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                e.watch_tick();
                if flushed.elapsed() >= Duration::from_secs(60) {
                    flushed = Instant::now();
                    e.flush_repeats();
                }
                let s = e.status();
                let ser = serde_json::to_string(&s).unwrap_or_default();
                if ser != last {
                    last = ser;
                    let _ = e.0.tx.send(Event::Status(s));
                }
            }
        });

        // Attempts that go around the gate: the fence refuses them, the system log records the
        // refusal, and this brings it into the Feed. If the log cannot be followed (it needs an
        // administrator account) the fence still holds; the attempts are just not shown.
        let e = engine.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut stop = e.0.stop.subscribe();
            loop {
                if let Ok(mut child) = crate::fence::violation_stream() {
                    if let Some(out) = child.stdout.take() {
                        let mut lines = tokio::io::BufReader::new(out).lines();
                        loop {
                            tokio::select! {
                                line = lines.next_line() => match line {
                                    Ok(Some(line)) => {
                                        if let Some(v) = crate::fence::violation_in(&line) {
                                            e.record_around(&v);
                                        }
                                    }
                                    _ => break,
                                },
                                _ = stop.changed() => {
                                    let _ = child.kill().await;
                                    return;
                                }
                            }
                        }
                    }
                    let _ = child.kill().await;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(300)) => {}
                    _ = stop.changed() => return,
                }
            }
        });

        // An organisation's fleet, if this computer has joined one (or a management profile says
        // which to join): its rules in, the log out, once a minute.
        let e = engine.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if crate::fleet::Membership::load().is_none() {
                if let Some((url, code)) = crate::fleet::managed_join() {
                    let _ = e.fleet_join(&url, &code, None).await;
                }
            }
            let mut stop = e.0.stop.subscribe();
            loop {
                if crate::fleet::Membership::load().is_some() {
                    let _ = e.fleet_sync().await;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                    _ = stop.changed() => return,
                }
            }
        });

        // Linux: something for the health check to knock on (see `fence_linux::PROBE_PORT`).
        // Connections are accepted and closed; nothing is ever read from them.
        if cfg!(target_os = "linux") {
            #[cfg(unix)]
            if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", crate::fence_linux::PROBE_PORT)).await {
                tokio::spawn(async move {
                    loop {
                        let _ = listener.accept().await;
                    }
                });
            }
        }

        // Fence health: now, then every half minute.
        let e = engine.clone();
        tokio::spawn(async move {
            e.check_fence().await;
            // We have just started, so nothing in a private space was started by this run. What
            // is there with no terminal was left behind by a run that ended abruptly, and is
            // stopped. What still has a terminal has a person at it, and carries on: its gate
            // is back, on the same port (see `fence::stop_leftovers`).
            let apps = e.0.cfg.lock().unwrap().provisioned().apps;
            for app in apps {
                if e.0.health.lock().unwrap().get(&app.id).map(|h| h.can_run).unwrap_or(false) && !e.0.running.lock().unwrap().contains(&app.id) {
                    fence::stop_leftovers(&app).await;
                }
            }
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                e.check_fence().await;
            }
        });
        Ok(engine)
    }

    /// Tell the engine which program launchers should call (see `fence::write_launcher`).
    /// Launchers that already exist are rewritten, so they keep working after the app has moved.
    pub fn set_opener(&self, exe: &str) {
        *self.0.opener.lock().unwrap() = Some(exe.to_string());
        let apps = self.0.cfg.lock().unwrap().apps.clone();
        for app in apps {
            if fence::launcher_path(&app).map(|p| p.is_dir()).unwrap_or(false) {
                let _ = fence::write_launcher(&app, exe);
            }
        }
    }

    /// Where the desktop app is running from, and whether that is the Applications folder.
    fn install_info(&self) -> Value {
        let bundle = self.0.opener.lock().unwrap().as_deref().and_then(fence::bundle_of);
        json!({ "path": bundle, "installed": bundle.as_deref() == Some(fence::INSTALL_PATH), "canInstall": bundle.is_some() && bundle.as_deref() != Some(fence::INSTALL_PATH) })
    }

    /// Start the gate on this port unless it is already running.
    async fn ensure_gate(&self, port: u16) {
        if !self.0.gates.lock().unwrap().insert(port) {
            return;
        }
        match crate::proxy::bind(port).await {
            Ok(listener) => {
                tokio::spawn(crate::proxy::serve(self.clone(), port, listener));
                // On Windows a protected app reaches its gate through a pipe only its own
                // private space can open (see `fence_windows`).
                #[cfg(windows)]
                if let Some(app_id) = self.app_for_gate(port) {
                    let errors = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::fence_windows::serve_gate_pipe(app_id, port).await {
                            errors.0.listen_errors.lock().unwrap().push(e);
                        }
                    });
                }
            }
            Err(e) => {
                self.0.gates.lock().unwrap().remove(&port);
                self.0.listen_errors.lock().unwrap().push(format!("Port {port} is in use by another program ({e})."));
            }
        }
    }

    /// The app whose gate this is, if any.
    pub(crate) fn app_for_gate(&self, port: u16) -> Option<String> {
        self.0.cfg.lock().unwrap().apps.iter().find(|a| a.proxy_port == port).map(|a| a.id.clone())
    }

    /// Whether a new question should make a sound.
    pub fn ask_sound(&self) -> bool {
        self.0.cfg.lock().unwrap().ask_sound
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
            eprintln!("homebody: could not save settings: {e}");
        }
    }

    /// Write a ledger record, fold it into the Feed, and tell the UI about the row it touched.
    fn append(&self, kind: &str, fields: Map<String, Value>) -> Option<Value> {
        let rec = match self.0.ledger.lock().unwrap().append(kind, fields) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("homebody: could not write to the log: {e}");
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
            return deny(Reason::Malformed, None, "Blocked by Homebody: this is not an ordinary destination name.");
        };
        if port != 443 && port != 80 {
            self.record_deny_detail(app_id, &host, port, Reason::Malformed, "port", "");
            return deny(Reason::Malformed, Some(host), "Blocked by Homebody: only ordinary web connections are possible from a protected app.");
        }

        let found = {
            let cfg = self.0.cfg.lock().unwrap();
            cfg.app(app_id).map(|a| (cfg.mode, cfg.lockdown, cfg.ask, a.clone(), a.rule_for(&host).cloned()))
        };
        let Some((mode, lockdown, ask_enabled, app, rule)) = found else {
            self.record_deny(app_id, &host, port, Reason::NotTrusted);
            return deny(Reason::NotTrusted, Some(host), "Blocked by Homebody.");
        };
        let (label, _) = catalog::describe(&host);
        let refused = format!("Blocked by Homebody: {} is not a trusted destination for {}.", label, app.name);
        let allow = |reason: Reason, ask: Option<u64>| Decision { allow: true, reason, ask, host: Some(host.clone()), message: String::new() };

        if lockdown {
            self.record_deny(app_id, &host, port, Reason::Lockdown);
            return deny(Reason::Lockdown, Some(host), "Blocked by Homebody: everything is blocked right now.");
        }
        match mode {
            Mode::Off => return allow(Reason::ProtectionOff, None),
            Mode::Watch => return allow(Reason::Watching, None),
            Mode::Private => {}
        }
        match rule {
            Some(r) if r.enabled && self.over_limit(app_id, &host) => {
                self.record_deny_detail(app_id, &host, port, Reason::NotTrusted, "limit", &r.limit_mb.unwrap_or(0).to_string());
                return deny(Reason::NotTrusted, Some(host), &format!("Blocked by Homebody: {} has sent as much to {} this hour as its limit allows.", app.name, label));
            }
            Some(r) if r.enabled => return allow(Reason::Trusted, None),
            Some(r) if r.source == "org" => {
                self.record_deny_detail(app_id, &host, port, Reason::NotTrusted, "org", "");
                return deny(Reason::NotTrusted, Some(host), &format!("Blocked by Homebody: your organisation's rules do not allow {} for {}.", label, app.name));
            }
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
        let window = if reason == Reason::AroundGate { AROUND_WINDOW } else { REPEAT_WINDOW };
        let repeat_of = {
            let mut recent = self.0.recent_denies.lock().unwrap();
            recent.retain(|_, (t, _)| t.elapsed() < AROUND_WINDOW);
            recent.get(&k).filter(|(t, _)| t.elapsed() < window).map(|(_, id)| *id)
        };
        if let Some(id) = repeat_of {
            if self.0.feed.lock().unwrap().add_repeat(id) {
                *self.0.unsaved_repeats.lock().unwrap().entry(id).or_default() += 1;
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

    // Hourly limits
    // -----------------------------------------------------------------------------------------

    /// The trusted rule this destination falls under for this app, and its hourly limit in bytes.
    fn limit_for(&self, app_id: &str, host: &str) -> Option<(String, u64)> {
        let cfg = self.0.cfg.lock().unwrap();
        let rule = cfg.app(app_id)?.rule_for(host)?;
        Some((rule.host.clone(), rule.limit_mb? * 1_048_576))
    }

    fn sent_last_hour(&self, app_id: &str, rule: &str) -> u64 {
        let mut sent = self.0.sent.lock().unwrap();
        let Some(q) = sent.get_mut(&(app_id.to_string(), rule.to_string())) else { return 0 };
        while q.front().map(|(t, _)| t.elapsed() > Duration::from_secs(3600)).unwrap_or(false) {
            q.pop_front();
        }
        q.iter().map(|(_, n)| n).sum()
    }

    /// Whether an app may send this many more bytes to a place now. When it may, they are
    /// counted at once, before they are sent, so that many connections sending at the same
    /// moment cannot each slip in under the limit. When it may not, nothing is counted and
    /// nothing is to be sent.
    pub(crate) fn may_send(&self, app_id: &str, host: &str, bytes: u64) -> bool {
        let Some((rule, limit)) = self.limit_for(app_id, host) else { return true };
        let mut sent = self.0.sent.lock().unwrap();
        let q = sent.entry((app_id.to_string(), rule)).or_default();
        while q.front().map(|(t, _)| t.elapsed() > Duration::from_secs(3600)).unwrap_or(false) {
            q.pop_front();
        }
        if q.iter().map(|(_, n)| n).sum::<u64>() + bytes > limit {
            return false;
        }
        q.push_back((Instant::now(), bytes));
        true
    }

    fn over_limit(&self, app_id: &str, host: &str) -> bool {
        self.limit_for(app_id, host).map(|(rule, limit)| self.sent_last_hour(app_id, &rule) >= limit).unwrap_or(false)
    }

    fn set_limit(&self, app_id: &str, pattern: &str, mb: Option<u64>) -> Result<(), String> {
        if mb.is_some_and(|m| m == 0 || m > 1_000_000) {
            return Err("Give the limit in megabytes per hour, for example 5.".into());
        }
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let rule = cfg.app_mut(app_id).ok_or("Unknown app.")?.trust.iter_mut().find(|r| r.host == pattern).ok_or("That place is not on the list.")?;
            rule.limit_mb = mb;
        }
        self.save_cfg();
        self.append("trust", fields! { "app" => app_id, "host" => pattern, "limit_mb" => mb, "via" => "limit" });
        Ok(())
    }

    // Private folders
    // -----------------------------------------------------------------------------------------

    /// Fingerprints of the text in the user's private folders, read again when they are stale.
    fn private_docs(&self) -> Option<Arc<crate::secrets::DocIndex>> {
        let folders = self.0.cfg.lock().unwrap().private_folders.clone();
        if folders.is_empty() {
            return None;
        }
        let current = self.0.docs.lock().unwrap().clone();
        let fresh = current.as_ref().map(|(at, _)| at.elapsed() < Duration::from_secs(300)).unwrap_or(false);
        if !fresh {
            // Read in the background; this request is judged against what we have.
            let e = self.clone();
            tokio::task::spawn_blocking(move || e.read_private_folders());
        }
        current.map(|(_, index)| index)
    }

    fn read_private_folders(&self) -> Arc<crate::secrets::DocIndex> {
        let folders = self.0.cfg.lock().unwrap().private_folders.clone();
        let salt = self.0.ledger.lock().unwrap().public_key_b64();
        let index = Arc::new(crate::secrets::DocIndex::build(&salt, &folders));
        *self.0.docs.lock().unwrap() = Some((Instant::now(), index.clone()));
        index
    }

    fn private_folders(&self) -> Value {
        let folders = self.0.cfg.lock().unwrap().private_folders.clone();
        let index = self.0.docs.lock().unwrap().as_ref().map(|(_, i)| (i.files, i.len()));
        json!({ "folders": folders.iter().map(|f| json!({ "path": f, "exists": std::path::Path::new(f).is_dir() })).collect::<Vec<_>>(), "files": index.map(|i| i.0), "lines": index.map(|i| i.1) })
    }

    fn set_private_folder(&self, raw: &str, add: bool) -> Result<(), String> {
        if add {
            let path = std::fs::canonicalize(raw).map_err(|_| "That folder could not be found.".to_string())?;
            let home = dirs::home_dir().ok_or("Your home folder could not be found.")?;
            if !path.is_dir() || path == home || !path.starts_with(&home) {
                return Err("Choose a folder inside your home folder, not the home folder itself.".into());
            }
            let path = path.to_string_lossy().into_owned();
            let mut cfg = self.0.cfg.lock().unwrap();
            if cfg.private_folders.len() >= 20 {
                return Err("That is a lot of folders. Remove one first, or choose a folder that contains several.".into());
            }
            if !cfg.private_folders.contains(&path) {
                cfg.private_folders.push(path);
            }
        } else {
            self.0.cfg.lock().unwrap().private_folders.retain(|f| f != raw);
        }
        self.save_cfg();
        self.append("private", fields! { "action" => if add { "added" } else { "removed" } });
        self.read_private_folders();
        Ok(())
    }

    /// The certificate authority for looking inside, made on first use.
    fn inspector(&self) -> Result<Arc<crate::inspect::Inspector>, String> {
        let mut slot = self.0.inspector.lock().unwrap();
        if let Some(i) = slot.as_ref() {
            return Ok(i.clone());
        }
        let i = Arc::new(crate::inspect::Inspector::open(&paths::inspect_key(), &paths::inspect_cert())?);
        *slot = Some(i.clone());
        Ok(i)
    }

    /// Whether this app's connections are to be looked inside, and with what.
    pub(crate) fn inspector_for(&self, app_id: &str) -> Option<Arc<crate::inspect::Inspector>> {
        let on = self.0.cfg.lock().unwrap().app(app_id).map(|a| a.inspect && Self::cannot_inspect(a).is_none()).unwrap_or(false);
        // While an app is being put in the Box, the Box's package tools fetch through this
        // gate, and they are not told about our certificate. Those minutes are not looked inside.
        #[cfg(unix)]
        if on && crate::boxvm::installing(app_id) {
            return None;
        }
        if on { self.inspector().ok() } else { None }
    }

    /// What a program started in the Box for this app is told: our certificate when the app's
    /// connections are looked inside, and the stand-ins for its kept keys.
    #[cfg(unix)]
    fn box_extras(&self, app_id: &str) -> crate::boxvm::Extras {
        let Some(app) = self.0.cfg.lock().unwrap().app(app_id).cloned() else { return Default::default() };
        if !app.inspect || Self::cannot_inspect(&app).is_some() {
            return Default::default();
        }
        crate::boxvm::Extras { ca_pem: self.inspector().ok().map(|i| i.ca_pem().to_string()), env: app.keys.iter().map(|k| (k.name.clone(), k.stand_in.clone())).collect() }
    }

    /// Why this app's connections cannot be looked inside, if they cannot. Looking inside
    /// relies on telling the app about our certificate through its environment; an app that
    /// takes its trusted certificates from the Mac itself never hears about it.
    fn cannot_inspect(app: &AppConfig) -> Option<String> {
        if cfg!(windows) {
            Some("Looking inside connections is not available on Windows yet.".to_string())
        } else if app.windowed {
            Some(format!("{} is a windowed app and takes its trusted certificates from your Mac, which Homebody does not change. Its connections cannot be looked inside yet.", app.name))
        } else if app.kind == "model" {
            // Seen on a real run: Ollama refused the certificate.
            Some(format!("{} takes its trusted certificates from your Mac, which Homebody does not change. Its connections cannot be looked inside yet.", app.name))
        } else {
            None
        }
    }

    /// Switch looking inside on or off for one app. Switching on makes the certificate
    /// authority if needed and puts its certificate where protected apps can read it.
    fn set_inspect(&self, app_id: &str, on: bool) -> Result<(), String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or("Unknown app.")?;
        if on {
            if let Some(why) = Self::cannot_inspect(&app) {
                return Err(why);
            }
            let inspector = self.inspector()?;
            crate::inspect::publish(inspector.ca_pem(), &fence::system_roots_pem())?;
        }
        if !on && !app.keys.is_empty() {
            return Err(format!("Homebody keeps {} for {}, and can only put a key into a connection it looks inside. Remove the key first.", if app.keys.len() == 1 { "a key" } else { "keys" }, app.name));
        }
        if app.inspect != on {
            if let Some(a) = self.0.cfg.lock().unwrap().app_mut(app_id) {
                a.inspect = on;
            }
            self.save_cfg();
            self.append("app", fields! { "action" => if on { "look_inside_on" } else { "look_inside_off" }, "app" => app_id });
        }
        Ok(())
    }

    /// The recogniser for secrets in this app's requests, if that is switched on for it.
    pub(crate) fn secret_scanner(&self, app_id: &str, host: &str) -> Option<crate::secrets::Scanner> {
        if !self.0.cfg.lock().unwrap().app(app_id).map(|a| a.secrets).unwrap_or(false) {
            return None;
        }
        // Signing in means sending a login token to the place that issued it. That is not a
        // leak, and stopping it would stop the app signing in.
        if catalog::describe(host).1.starts_with("Signing in") {
            return None;
        }
        // Fingerprints are particular to this computer: salted with its own log key.
        let salt = self.0.ledger.lock().unwrap().public_key_b64();
        let (engine, app_id) = (self.clone(), app_id.to_string());
        // A stand-in for a key Homebody keeps has the shape of a key and is not one: an app
        // that repeats its stand-in in what it sends has given nothing away.
        let stand_ins: Vec<String> = self.0.cfg.lock().unwrap().app(&app_id).map(|a| a.keys.iter().map(|k| crate::secrets::fingerprint(&salt, k.stand_in.as_bytes())).collect()).unwrap_or_default();
        let scanner = crate::secrets::Scanner::new(&salt, move |fp| stand_ins.iter().any(|f| f == fp) || engine.0.cfg.lock().unwrap().app(&app_id).map(|a| a.allowed_secrets.iter().any(|f| f == fp)).unwrap_or(false));
        Some(match self.private_docs() {
            Some(docs) => scanner.with_docs(docs),
            None => scanner,
        })
    }

    /// What the gate needs to put this app's kept keys into a connection to `host`: every
    /// stand-in, and the real key only for those that may go to this place.
    pub(crate) fn key_swaps(&self, app_id: &str, host: &str) -> Vec<crate::keys::Swap> {
        let keys = match self.0.cfg.lock().unwrap().app(app_id) {
            Some(a) if !a.keys.is_empty() => a.keys.clone(),
            _ => return Vec::new(),
        };
        let vault = self.0.vault.lock().unwrap();
        keys.iter()
            .map(|k| crate::keys::Swap {
                name: k.name.clone(),
                stand_in: k.stand_in.as_bytes().to_vec(),
                fate: match (k.goes_to(host), vault.get(app_id, &k.name)) {
                    (false, _) => crate::keys::Fate::Elsewhere,
                    (true, Some(v)) => crate::keys::Fate::Here(v.as_bytes().to_vec()),
                    (true, None) => crate::keys::Fate::Lost,
                },
            })
            .collect()
    }

    /// Keep a key for an app: from now on the app is handed a stand-in, and the real key is put
    /// into its requests at the gate, for the named places only.
    fn set_key(&self, app_id: &str, name: &str, value: &str, hosts: Option<Vec<String>>) -> Result<(), String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or("Unknown app.")?;
        if !crate::keys::valid_name(name) {
            return Err("A key is known by the name the app reads it from, in capitals: ANTHROPIC_API_KEY, for example.".into());
        }
        // Checked against everything Homebody sets with Look inside on, which is how the app
        // will be run once it has a key.
        if fence::fenced_env(&AppConfig { keys: Vec::new(), inspect: true, ..app.clone() }).iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
            return Err(format!("\"{name}\" is a name Homebody itself sets for a protected program."));
        }
        let value = value.trim();
        crate::keys::valid_value(value)?;
        if let Some(why) = Self::cannot_inspect(&app) {
            return Err(format!("Homebody can only keep a key for an app whose connections it can look inside. {why}"));
        }
        let mut places: Vec<String> = Vec::new();
        for h in hosts.unwrap_or_else(|| crate::keys::usual_places(name)) {
            let h = hostname::normalize_pattern(h.trim());
            if h.is_empty() {
                continue;
            }
            // One exact place, never a family of them: "every name under example.com" can
            // include names that belong to someone else.
            if h.contains('*') || hostname::canonical(&h).as_deref() != Some(h.as_str()) {
                return Err(format!("Name the exact place the key is for, like api.example.com. \"{h}\" is not one."));
            }
            if !places.contains(&h) {
                places.push(h);
            }
        }
        if places.is_empty() {
            return Err("Say which place this key is for (api.example.com). It will be sent there and nowhere else.".into());
        }
        let existing = app.keys.iter().find(|k| k.name == name).cloned();
        // An open session holds the stand-in it was started with: keep it when the new key has
        // the same shape, so replacing a key does not cut a session off.
        let stand_in = match &existing {
            Some(k) if self.0.vault.lock().unwrap().get(app_id, name).is_some_and(|old| crate::keys::stand_in_fits(&k.stand_in, old, value)) => k.stand_in.clone(),
            _ => crate::keys::stand_in(value),
        };
        // Look inside first: if that cannot be switched on, no key is left behind unused.
        self.set_inspect(app_id, true)?;
        self.0.vault.lock().unwrap().set(app_id, name, value)?;
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let a = cfg.app_mut(app_id).ok_or("Unknown app.")?;
            a.keys.retain(|k| k.name != name);
            a.keys.push(KeyConfig { name: name.to_string(), stand_in, hosts: places.clone(), hint: crate::keys::hint(value), added: now_iso() });
            a.keys.sort_by(|x, y| x.name.cmp(&y.name));
        }
        self.save_cfg();
        // The name and the places are written down. The key, and its last characters, are not.
        self.append("app", fields! { "action" => if existing.is_some() { "key_replaced" } else { "key_kept" }, "app" => app_id, "key" => name, "hosts" => places });
        Ok(())
    }

    fn remove_key(&self, app_id: &str, name: &str) -> Result<(), String> {
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let a = cfg.app_mut(app_id).ok_or("Unknown app.")?;
            if !a.keys.iter().any(|k| k.name == name) {
                return Err("Homebody is not keeping a key of that name for this app.".into());
            }
            a.keys.retain(|k| k.name != name);
        }
        self.0.vault.lock().unwrap().remove(app_id, name)?;
        self.save_cfg();
        self.append("app", fields! { "action" => "key_removed", "app" => app_id, "key" => name });
        Ok(())
    }

    fn key_infos(app: &AppConfig) -> Vec<KeyInfo> {
        app.keys.iter().map(|k| KeyInfo { name: k.name.clone(), maker: crate::keys::maker(&k.name).unwrap_or("").to_string(), hint: k.hint.clone(), hosts: k.hosts.clone(), added: k.added.clone() }).collect()
    }

    /// Let one particular secret through for an app from now on, or stop doing so.
    fn allow_secret(&self, app_id: &str, fingerprint: &str, allowed: bool) -> Result<(), String> {
        if fingerprint.len() != 16 || !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("That is not a fingerprint from the Feed.".into());
        }
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let app = cfg.app_mut(app_id).ok_or("Unknown app.")?;
            app.allowed_secrets.retain(|f| f != fingerprint);
            if allowed {
                app.allowed_secrets.push(fingerprint.to_string());
            }
        }
        self.save_cfg();
        self.append("trust", fields! { "app" => app_id, "secret" => fingerprint, "enabled" => allowed, "via" => "secret" });
        self.touch();
        Ok(())
    }

    /// Whether a request for this address inside a trusted place may go (the user's switches).
    pub(crate) fn inside_allowed(&self, app_id: &str, host: &str, method: &str, path: &str) -> bool {
        self.0.cfg.lock().unwrap().app(app_id).map(|a| a.inside_allows(host, method, path)).unwrap_or(false)
    }

    pub fn inside_list(&self, app_id: Option<&str>) -> Vec<InsideEntry> {
        let cfg = self.0.cfg.lock().unwrap();
        let mut out = Vec::new();
        for a in cfg.apps.iter().filter(|a| app_id.map(|id| id == a.id).unwrap_or(true)) {
            for r in &a.inside {
                let (label, purpose) = catalog::describe_inside(&r.host, &r.path);
                out.push(InsideEntry { app_id: a.id.clone(), host: r.host.clone(), host_label: catalog::describe(&r.host).0, path: r.path.clone(), method: r.method.clone(), label, purpose, enabled: r.enabled, source: r.source.clone() });
            }
        }
        out
    }

    fn set_inside(&self, app_id: &str, host: &str, path: &str, method: Option<String>, enabled: Option<bool>) -> Result<(), String> {
        let host = hostname::canonical(host).ok_or("That is not a place name.")?;
        if !crate::config::valid_inside_path(path) {
            return Err("That is not an address inside a place. It should look like /v1/messages.".into());
        }
        let method = method.map(|m| m.to_ascii_uppercase());
        if method.as_deref().map(|m| !crate::config::valid_method(m)).unwrap_or(false) {
            return Err("That is not a kind of request. It should be a word like GET or POST.".into());
        }
        {
            let mut cfg = self.0.cfg.lock().unwrap();
            let app = cfg.app_mut(app_id).ok_or("Unknown app.")?;
            match (app.inside.iter().position(|r| r.host == host && r.path == path && r.method == method), enabled) {
                (Some(i), Some(on)) => app.inside[i].enabled = on,
                (None, Some(on)) => app.inside.push(crate::config::InsideRule { host: host.clone(), path: path.to_string(), enabled: on, source: "you".into(), method: method.clone() }),
                (Some(i), None) if app.inside[i].source == "you" => {
                    app.inside.remove(i);
                }
                (Some(_), None) => return Err("Built-in switches can be turned on or off, but not removed.".into()),
                (None, None) => {}
            }
        }
        self.save_cfg();
        self.append("trust", fields! { "app" => app_id, "host" => host.as_str(), "path" => path, "method" => method, "enabled" => enabled, "via" => "inside" });
        Ok(())
    }

    /// The end of a connection that was looked inside: what was asked for in it.
    pub fn record_end_inspected(&self, conn: u64, bytes_out: u64, bytes_in: u64, ms: u64, requests: &[(String, u64)], keys: &[String]) {
        let paths: Vec<Value> = requests.iter().map(|(what, n)| json!([what, n])).collect();
        let mut f = fields! { "conn" => conn, "out" => bytes_out, "in" => bytes_in, "ms" => ms, "paths" => paths };
        if !keys.is_empty() {
            // Which kept keys were put in, by name. Never a key.
            f.insert("keys".into(), json!(keys));
        }
        self.append("end", f);
    }

    pub fn record_end(&self, conn: u64, bytes_out: u64, bytes_in: u64, ms: u64, unreachable: bool) {
        let mut f = fields! { "conn" => conn, "out" => bytes_out, "in" => bytes_in, "ms" => ms };
        if unreachable {
            f.insert("unreachable".into(), json!(true));
        }
        self.append("end", f);
    }

    /// Write down the repeats counted since the last call, one record per row, so the counts
    /// survive a restart without a record for every retry of an app that hammers once a second.
    /// The Feed already shows them, so these records are not folded in again while running.
    fn flush_repeats(&self) {
        let pending: Vec<(u64, u64)> = self.0.unsaved_repeats.lock().unwrap().drain().collect();
        if pending.is_empty() {
            return;
        }
        let mut ledger = self.0.ledger.lock().unwrap();
        for (id, count) in pending {
            if let Err(e) = ledger.append("repeat", fields! { "of" => id, "count" => count }) {
                eprintln!("homebody: could not write to the log: {e}");
            }
        }
    }

    /// The fence refused something that did not go through the gate. See `fence::parse_violation`.
    pub fn record_around(&self, v: &crate::fence::Violation) {
        if self.0.cfg.lock().unwrap().app(&v.app).is_none() {
            return;
        }
        for _ in 0..v.count.min(20) {
            self.record_deny_detail(&v.app, "", v.port, Reason::AroundGate, v.what, &v.process);
        }
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
                    ("", _) => "Homebody does not know this place. Allow?".to_string(),
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
                // Blocked for now, but the question is not lost: a person looking at another
                // window, another desktop or away from the desk never saw it. It stays, still
                // answerable, until they deal with it.
                let mut missed = self.0.missed.lock().unwrap();
                missed.retain(|m| !(m.app_id == app && m.host == host));
                missed.push(p.ask.clone());
                if missed.len() > MISSED_KEPT {
                    let extra = missed.len() - MISSED_KEPT;
                    missed.drain(..extra);
                }
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
            let org = cfg.org.as_ref().map(|p| p.name.clone());
            let app = cfg.app_mut(app_id).ok_or_else(|| "Unknown app.".to_string())?;
            if app.org_rule_for(&pattern).is_some() {
                return Err(format!("{} decides this place, so it cannot be changed here.", org.unwrap_or_else(|| "Your organisation".into())));
            }
            match app.trust.iter_mut().find(|r| r.host == pattern && r.source != "org") {
                Some(r) if r.enabled == enabled => false,
                Some(r) => {
                    r.enabled = enabled;
                    true
                }
                None => {
                    app.trust.push(TrustRule { host: pattern.clone(), enabled, source: "you".into(), recommended: false, limit_mb: None });
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
        // Decided now, however it was decided: no longer a question that was missed.
        self.0.missed.lock().unwrap().retain(|m| !(m.app_id == app_id && (m.host == pattern || hostname::matches(&pattern, &m.host))));
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
                // Your own rule for a place the organisation has a rule for is kept, but it is
                // not in force and would only show the place twice.
                if r.source != "org" && a.trust.iter().any(|o| o.source == "org" && o.host == r.host) {
                    continue;
                }
                let (mut label, mut purpose) = catalog::describe_pattern(&r.host);
                // A place switched on with an AI site is named after the site and what it is for.
                if let (true, Some((site, why))) = (label == r.host, catalog::ai_site_for(&r.host).filter(|_| a.kind == "browser")) {
                    label = format!("{}: {}", site.name, why.to_lowercase());
                    purpose = why.to_string();
                } else if a.kind == "browser" && label == r.host && catalog::BROWSER_BACKGROUND.contains(&r.host.as_str()) {
                    purpose = "Updates, statistics, advertising or crash reports: not needed to use an AI site".to_string();
                }
                let sent_mb = r.limit_mb.map(|_| (self.sent_last_hour(&a.id, &r.host) as f64 / 1_048_576.0 * 10.0).round() / 10.0);
                let pack = (r.source == "pack").then(|| catalog::pack_for(&r.host, &a.packs).map(|p| p.id.to_string())).flatten();
                out.push(TrustEntry { app_id: a.id.clone(), pack, host: r.host.clone(), label, purpose, enabled: r.enabled, source: r.source.clone(), recommended: r.recommended, limit_mb: r.limit_mb, sent_mb });
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

    /// Switch mode. `minutes` applies to Watch: how long until it switches itself back to
    /// Private (0: until switched off; not given: an hour). Asking for Watch while it is already
    /// on only moves the end time.
    fn set_mode_for(&self, mode: Mode, minutes: Option<u64>, why: &str) {
        let now = now_iso();
        let until = match minutes.unwrap_or(DEFAULT_WATCH_MINUTES) {
            0 => None,
            m => Some((chrono::Utc::now() + chrono::Duration::minutes(m.min(MAX_WATCH_MINUTES) as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        };
        let from = {
            let mut cfg = self.0.cfg.lock().unwrap();
            if cfg.mode == mode {
                if mode == Mode::Watch && minutes.is_some() && cfg.watch_until != until {
                    cfg.watch_until = until;
                    drop(cfg);
                    self.save_cfg();
                    self.touch();
                }
                return;
            }
            let from = cfg.mode;
            if from == Mode::Watch {
                cfg.watch_review = Some(crate::config::WatchWindow { since: cfg.mode_since.clone(), until: now.clone() });
            }
            cfg.mode = mode;
            cfg.mode_since = now.clone();
            cfg.private_since = if mode == Mode::Private { Some(now) } else { None };
            cfg.watch_until = if mode == Mode::Watch { until } else { None };
            if mode == Mode::Watch {
                cfg.watch_review = None;
            }
            from
        };
        // A Watch in which nothing new was contacted leaves nothing to review.
        if from == Mode::Watch && self.watch_review().map(|r| r.items.is_empty()).unwrap_or(true) {
            self.0.cfg.lock().unwrap().watch_review = None;
        }
        self.save_cfg();
        self.append("mode", fields! { "from" => from.as_str(), "to" => mode.as_str(), "why" => why });
        if mode != Mode::Private {
            self.cancel_asks();
        }
    }

    /// Switch Watch back to Private once its time is up. Called on a slow tick.
    fn watch_tick(&self) {
        let due = {
            let cfg = self.0.cfg.lock().unwrap();
            cfg.mode == Mode::Watch && cfg.watch_until.as_deref().map(|u| u <= now_iso().as_str()).unwrap_or(false)
        };
        if due {
            self.set_mode_for(Mode::Private, None, "watch ended");
        }
    }

    /// Places contacted under Watch between two times that have no rule yet, busiest first.
    fn watch_items(&self, since: &str, until: Option<&str>) -> Vec<WatchItem> {
        let cfg = self.0.cfg.lock().unwrap().clone();
        let mut found: Vec<WatchItem> = Vec::new();
        let feed = self.0.feed.lock().unwrap();
        for row in feed.rows().iter().rev().take_while(|r| r.ts.as_str() >= since) {
            if row.reason != Reason::Watching || until.map(|u| row.ts.as_str() > u).unwrap_or(false) || !hostname::valid_pattern(&row.host) {
                continue;
            }
            let Some(app) = cfg.app(&row.app) else { continue };
            if app.rule_for(&row.host).is_some() {
                continue;
            }
            match found.iter_mut().find(|i| i.app_id == row.app && i.host == row.host) {
                Some(i) => {
                    i.times += 1;
                    i.bytes_out += row.bytes_out.unwrap_or(0);
                }
                None => {
                    let (label, purpose) = catalog::describe(&row.host);
                    found.push(WatchItem { app_id: row.app.clone(), app_name: app.name.clone(), app_icon: app.icon.clone(), host: row.host.clone(), label, purpose, times: 1, bytes_out: row.bytes_out.unwrap_or(0) });
                }
            }
        }
        found.sort_by(|a, b| a.app_name.cmp(&b.app_name).then(b.times.cmp(&a.times)).then(a.host.cmp(&b.host)));
        found
    }

    /// What Watch has seen: so far while it is on, otherwise the finished Watch awaiting review.
    pub fn watch_review(&self) -> Option<WatchReview> {
        let (mode, since, until, review) = {
            let cfg = self.0.cfg.lock().unwrap();
            (cfg.mode, cfg.mode_since.clone(), cfg.watch_until.clone(), cfg.watch_review.clone())
        };
        if mode == Mode::Watch {
            return Some(WatchReview { items: self.watch_items(&since, None), since, until, active: true });
        }
        let w = review?;
        Some(WatchReview { items: self.watch_items(&w.since, Some(&w.until)), since: w.since, until: Some(w.until), active: false })
    }

    /// Trust the chosen places and put the review away. Places left out stay undecided, so the
    /// app asks about them the first time they come up again.
    fn finish_watch_review(&self, trust: &[(String, String)]) -> Result<(), String> {
        let offered = self.watch_review().filter(|r| !r.active).map(|r| r.items).unwrap_or_default();
        for (app, host) in trust {
            if !offered.iter().any(|i| &i.app_id == app && &i.host == host) {
                return Err(format!("{host} was not among the places seen."));
            }
        }
        for (app, host) in trust {
            self.set_trust_inner(app, host, true, "watch")?;
        }
        self.0.cfg.lock().unwrap().watch_review = None;
        self.save_cfg();
        self.touch();
        Ok(())
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
            Reason::YouAllowedOnce if row.detail == "fleet" => "Homebody itself joined your organisation's fleet here. From now on it takes the organisation's rules from this server and sends it the log.".into(),
            Reason::YouAllowedOnce if row.detail == "self" => "Homebody itself contacted this place because you asked it to. It sent a fingerprint of the log, or asked whether there is a newer version; nothing from your apps.".into(),
            Reason::YouAllowedOnce => "You allowed this once.".into(),
            Reason::YouAlwaysAllow => "You chose to always allow this.".into(),
            Reason::YouBlocked => "You blocked this.".into(),
            Reason::NotTrusted if row.detail == "limit" => format!("{app_name} has sent as much to {label} this hour as the limit you set allows{}, so this was stopped. It can go on once the hour has moved on.", if row.other.is_empty() { String::new() } else { format!(" ({} MB)", row.other) }),
            Reason::NotTrusted if row.detail == "secret" => {
                let kind = row.other.split('|').next().unwrap_or("a secret");
                format!("{app_name} was about to send what looks like {kind} to {label}. Homebody stopped the request before it left.")
            }
            Reason::NotTrusted if row.detail == "key" => {
                let whose = crate::keys::maker(&row.other).map(|m| format!("your {m} key")).unwrap_or_else(|| format!("your key {}", row.other));
                format!("{app_name} tried to send {whose} to {label}, which is not a place that key is for. Homebody keeps that key, so {app_name} only ever had a stand-in, and the request was stopped before even that left.")
            }
            Reason::NotTrusted if row.detail == "key_misplaced" => {
                let whose = crate::keys::maker(&row.other).map(|m| format!("your {m} key")).unwrap_or_else(|| format!("your key {}", row.other));
                format!("{app_name} sent the stand-in for {whose} to {label}, but not as a credential: it was in the address or in another part of the request, where a server may repeat it back. Putting the real key there would hand it to {app_name}, so the request was stopped and nothing of it was sent.")
            }
            Reason::NotTrusted if row.detail == "key_lost" => format!("{app_name} used a key that Homebody is meant to keep ({}), but Homebody no longer has it. Add it again on {app_name}'s page. Nothing was sent.", row.other),
            Reason::NotTrusted if row.detail == "path" => {
                let (what, _) = catalog::describe_inside(&row.host, &row.other);
                let named = if what.is_empty() { row.other.clone() } else { format!("{what} ({})", row.other) };
                format!("{app_name} asked {label} for {named}, which is switched off, so that request was refused and nothing of it was sent.")
            }
            Reason::NotTrusted if row.detail == "org" => format!("Your organisation's rules do not allow {label} for {app_name}, so it was blocked."),
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
                "other_site" => format!("Inside its connection to {label}, {app_name} addressed a different site ({}). That is how a trusted name is used as cover, so the connection was cut.", row.other),
                "not_followed" => format!("{app_name} sent {} to {label}. Homebody could not follow it, so it was not let through.", if row.other.is_empty() { "something unusual" } else { row.other.as_str() }),
                "no_inspect" => format!("{app_name} did not accept Homebody's certificate for {label}, so nothing was sent. If this keeps happening, switch off \"Look inside\" for {app_name}: it does not support it."),
                "real_certificate" => format!("The real {label} did not show a certificate that checks out, so Homebody did not connect {app_name} to it."),
                _ => "This was not an ordinary destination name, so it was blocked.".into(),
            },
            Reason::LocalAddress => "This name points at your own computer or network, so it was blocked.".into(),
            Reason::AroundGate => {
                let who = if row.other.is_empty() { "A program" } else { row.other.as_str() };
                let tried = match row.detail.as_str() {
                    "direct" => format!("tried to connect straight to the internet on port {}, without going through the gate", row.port),
                    "lookup" => "tried to look up a name by itself instead of going through the gate".to_string(),
                    "listen" => "tried to accept connections from other computers".to_string(),
                    "gate" => "tried to use another app's gate".to_string(),
                    "open" => "tried to hand something to another app, such as a link to your browser. That is a way for data to leave without passing the gate".to_string(),
                    "control" => "tried to control another app".to_string(),
                    "prompt" => "asked macOS to put a password or keychain prompt on your screen".to_string(),
                    _ => "tried a kind of network access that protected apps do not get".to_string(),
                };
                let who = if who == app_name { who.to_string() } else { format!("{who}, started by {app_name},") };
                if row.detail == "prompt" {
                    // Not an attempt to send anything: usually a program trying to save its sign-in.
                    format!("{who} {tried}. Protected apps cannot do that. A program that wanted to save a sign-in keeps it in its own private folder instead.")
                } else {
                    format!("{who} {tried}. The fence refused it, so nothing left this computer.")
                }
            }
        };
        if row.unreachable {
            s.push_str(" The destination did not answer.");
        }
        if row.repeats > 0 {
            let within = if row.reason == Reason::AroundGate { "over the next ten minutes" } else { "within a minute" };
            s.push_str(&format!(" Tried {} more time{} {within}.", row.repeats, if row.repeats == 1 { "" } else { "s" }));
        }
        s
    }

    fn to_event(row: &Row, cfg: &Config) -> FeedEvent {
        let app = cfg.app(&row.app);
        // "homebody" is the app itself (update check, timestamps); "sandbox-ai" is the same, written
        // before the app was renamed and kept as it was in the signed log.
        let app_name = app.map(|a| a.name.clone()).unwrap_or_else(|| if row.app == "homebody" || row.app == "sandbox-ai" { "Homebody".to_string() } else { row.app.clone() });
        let (label, purpose) = if row.reason == Reason::AroundGate {
            let label = match row.detail.as_str() {
                "direct" => "Direct connection",
                "lookup" => "Name lookup",
                "listen" => "Incoming connections",
                "gate" => "Another app's gate",
                "open" => "Opening another app",
                "control" => "Controlling another app",
                "prompt" => "System prompt",
                _ => "Network access",
            };
            (label.to_string(), if row.detail == "prompt" { "Asked for a prompt on your screen" } else { "Went around the gate" }.to_string())
        } else {
            catalog::describe(&row.host)
        };
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
            secret: (row.detail == "secret").then(|| {
                let (kind, fingerprint) = row.other.split_once('|').unwrap_or((row.other.as_str(), ""));
                SecretSeen { kind: kind.to_string(), fingerprint: fingerprint.to_string(), allowed: app.map(|a| a.allowed_secrets.iter().any(|f| f == fingerprint)).unwrap_or(false) }
            }),
            keys: row.keys.clone(),
            key_stopped: (row.reason == Reason::NotTrusted && row.detail.starts_with("key")).then(|| row.other.clone()),
            requests: row.requests.as_ref().map(|r| {
                r.iter()
                    .map(|(what, times)| {
                        let path = what.split_once(' ').map(|(_, p)| p).filter(|p| p.starts_with('/')).unwrap_or("").to_string();
                        RequestKind { what: what.clone(), times: *times, label: catalog::describe_inside(&row.host, &path).0, allowed: app.map(|a| a.inside_allows(&row.host, what.split(' ').next().unwrap_or(""), &path)).unwrap_or(true), path }
                    })
                    .collect()
            }),
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
            // One blocked thing is one, however often the app retried it; the row says how often.
            Verb::Blocked => c.blocked += 1,
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
            let found = catalog::find_binary(&a.binary);
            let problem = found.as_deref().filter(|_| !a.windowed).and_then(|path| catalog::binary_problem(path, &a.name));
            let installed = if h.checked { h.installed } else { found.is_some() };
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
                can_start: installed && a.protected && state == FenceState::Up && problem.is_none(),
                command: a.binary.clone(),
                workspace: a.workspace.clone(),
                today: per_app.get(&a.id).copied().unwrap_or_default(),
                custom: a.custom,
                inspect: a.inspect && Self::cannot_inspect(a).is_none(),
                can_inspect: Self::cannot_inspect(a).is_none(),
                secrets: a.secrets,
                keys: Self::key_infos(a),
                problem,
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
            reasons = vec!["Homebody has not been set up on this computer yet.".into()];
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

        let review = self.watch_review();
        let places = review.as_ref().map(|r| r.items.len() as u64).unwrap_or(0);
        let watch = review.as_ref().filter(|r| r.active).map(|r| WatchStatus { since: r.since.clone(), until: r.until.clone(), places });
        let watch_review = review.as_ref().filter(|r| !r.active && places > 0).map(|r| WatchStatus { since: r.since.clone(), until: r.until.clone(), places });

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
            missed: self.0.missed.lock().unwrap().clone(),
            ask_sound: cfg.ask_sound,
            ask: cfg.ask,
            preset_notes: catalog::PRESET_NOTES.iter().filter(|(v, _)| *v > cfg.presets_seen).map(|(_, text)| text.to_string()).collect(),
            local_servers: fence::own_servers(),
            org: cfg.org.as_ref().map(|p| p.name.clone()),
            org_locked: cfg.mode_locked(),
            watch,
            watch_review,
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
                    Verb::Blocked => blocked += 1,
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
            "format": "homebody-proof-v1",
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
        let name = format!("Homebody proof {}.json", chrono::Local::now().format("%Y-%m-%d %H.%M"));
        let path = paths::downloads_dir().join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(&file).unwrap_or_default()).map_err(|e| format!("The proof could not be saved: {e}"))?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// The whole log, exactly as written, with one signed line in front saying what it is.
    /// `homebody verify <file>` (or anyone's own code) can check it without this computer.
    fn export_log(&self, into: Option<std::path::PathBuf>) -> Result<String, String> {
        // A pipeline names the folder itself (`homebody ci --report`); a person gets one in Downloads.
        let dir = into.unwrap_or_else(|| paths::downloads_dir().join(format!("Homebody export {}", chrono::Local::now().format("%Y-%m-%d %H.%M"))));
        let fail = |what: &str, e: std::io::Error| format!("The {what} could not be saved: {e}");
        std::fs::create_dir_all(&dir).map_err(|e| fail("export", e))?;
        // 1. The whole log, exactly as written, with a signed first line. This is the evidence.
        let log = self.0.ledger.lock().unwrap().export().map_err(|e| format!("The log could not be read: {e}"))?;
        std::fs::write(dir.join("log.jsonl"), log).map_err(|e| fail("log", e))?;
        // 2. The same events as a table, for reading in a spreadsheet. A convenience, not evidence.
        std::fs::write(dir.join("activity.csv"), self.feed_csv()).map_err(|e| fail("table", e))?;
        // 3. The rules that were in force when this was made.
        let rules = serde_json::to_vec_pretty(&*self.0.cfg.lock().unwrap()).unwrap_or_default();
        std::fs::write(dir.join("rules.json"), rules).map_err(|e| fail("rules", e))?;
        // 3b. The same rules in a formal language, for reviewers who want to check them with
        // their own tools.
        let _ = crate::policy::write(&self.0.cfg.lock().unwrap(), &dir.join("policy"));
        // 3c. Where this evidence fits in public control catalogues.
        let _ = std::fs::write(dir.join("compliance.json"), serde_json::to_vec_pretty(&crate::compliance::json_table()).unwrap_or_default());
        // 4. Independent timestamps, if any were asked for.
        let stamps = paths::data_dir().join("timestamps");
        if let Ok(entries) = std::fs::read_dir(&stamps) {
            let out = dir.join("timestamps");
            for e in entries.flatten() {
                let _ = std::fs::create_dir_all(&out);
                let _ = std::fs::copy(e.path(), out.join(e.file_name()));
            }
        }
        let readme = "Homebody export\n=================\n\nlog.jsonl      Every decision, exactly as it was written. The first line says what the file is and\n               carries the key to check it with; each following line is one record, linked to the one\n               before and signed. Check it on any computer with:  homebody verify log.jsonl\n               Compare the key it prints with the one on the Report screen to know whose log it is.\nactivity.csv   The same events as a table for a spreadsheet. Convenient to read; it is not signed.\nrules.json     The rules in force when this export was made.\ncompliance.json  Which controls in NIST SP 800-53 and OWASP's list for AI agents this evidence\n               speaks to, and how far. The project's own mapping, not an auditor's.\npolicy/        The same rules as Cedar policies, a formal language reviewers can check with their\n               own tools. See the README inside.\ntimestamps/    Present if independent timestamps were asked for. Each .tsr is a statement by a public\n               time-stamping service that a log ending in a given record existed at that time. Check\n               one with:  openssl ts -reply -in <file>.tsr -text\n";
        std::fs::write(dir.join("README.txt"), readme).map_err(|e| fail("export", e))?;
        Ok(dir.to_string_lossy().into_owned())
    }

    /// The Feed as a table. Fields that begin like a formula are made harmless for spreadsheets.
    fn feed_csv(&self) -> String {
        let field = |v: &str| {
            let v = if v.starts_with(['=', '+', '-', '@']) { format!("'{v}") } else { v.to_string() };
            if v.contains([',', '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v }
        };
        let mut out = String::from("time,app,result,place,name,port,reason,bytes_sent,bytes_received,milliseconds,requests_inside,explanation\n");
        let cfg = self.0.cfg.lock().unwrap().clone();
        let feed = self.0.feed.lock().unwrap();
        for row in feed.rows() {
            let e = Self::to_event(row, &cfg);
            let inside = e.requests.as_ref().map(|r| r.iter().map(|k| format!("{} x{}", k.what, k.times)).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            let n = |v: Option<u64>| v.map(|x| x.to_string()).unwrap_or_default();
            let cols = [e.ts.clone(), e.app_name.clone(), format!("{:?}", e.verb).to_lowercase(), e.host.clone(), e.label.clone(), e.port.to_string(), e.reason.as_str().to_string(), n(e.bytes_out), n(e.bytes_in), n(e.duration_ms), inside, e.explanation.clone()];
            out.push_str(&cols.iter().map(|c| field(c)).collect::<Vec<_>>().join(","));
            out.push('\n');
        }
        out
    }

    /// Ask a public time-stamping service to vouch that the log, as it stands, existed now.
    /// Only a fingerprint of the last record is sent: nothing from the log itself. The answer
    /// is kept and goes into exports. This is Homebody itself contacting a server, so it
    /// happens only when asked for, and it is written into the log like any other contact.
    async fn timestamp_log(&self) -> Result<Value, String> {
        const SERVICE: &str = "http://timestamp.digicert.com";
        let (count, head) = self.0.ledger.lock().unwrap().head();
        if count == 0 || head.len() != 64 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("There is nothing in the log to timestamp yet.".into());
        }
        let dir = paths::data_dir().join("timestamps");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stem = format!("record-{count}-{}", &head[..12]);
        let (query, reply) = (dir.join(format!("{stem}.tsq")), dir.join(format!("{stem}.tsr")));
        let run = |args: Vec<String>| async move {
            tokio::time::timeout(Duration::from_secs(30), tokio::process::Command::new(&args[0]).args(&args[1..]).output()).await.ok().and_then(|r| r.ok())
        };
        let made = run(vec!["/usr/bin/openssl".into(), "ts".into(), "-query".into(), "-digest".into(), head.clone(), "-sha256".into(), "-cert".into(), "-out".into(), query.to_string_lossy().into_owned()]).await;
        if !made.map(|o| o.status.success()).unwrap_or(false) {
            return Err("The timestamp request could not be prepared.".into());
        }
        let sent = run(vec!["/usr/bin/curl".into(), "-sS".into(), "--max-time".into(), "20".into(), "-H".into(), "Content-Type: application/timestamp-query".into(), "--data-binary".into(), format!("@{}", query.to_string_lossy()), "-o".into(), reply.to_string_lossy().into_owned(), SERVICE.into()]).await;
        self.append("conn", fields! { "app" => "homebody", "host" => "timestamp.digicert.com", "port" => 80, "decision" => "allow", "reason" => "you_allowed_once", "detail" => "self" });
        if !sent.map(|o| o.status.success()).unwrap_or(false) {
            let _ = std::fs::remove_file(&reply);
            return Err("The time-stamping service could not be reached.".into());
        }
        let shown = run(vec!["/usr/bin/openssl".into(), "ts".into(), "-reply".into(), "-in".into(), reply.to_string_lossy().into_owned(), "-text".into()]).await.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let granted = shown.contains("Status: Granted");
        // The answer must be about our fingerprint, not somebody else's.
        let about_ours = shown.lines().skip_while(|l| !l.contains("Message data")).skip(1).take(4).flat_map(|l| l.split_whitespace().skip(2).take_while(|w| !w.contains('|')).map(str::to_string).collect::<Vec<_>>()).collect::<String>().replace('-', "").to_lowercase().starts_with(&head[..32]);
        if !granted || !about_ours {
            let _ = std::fs::remove_file(&reply);
            return Err("The time-stamping service did not give a usable answer.".into());
        }
        let time = shown.lines().find_map(|l| l.trim().strip_prefix("Time stamp: ")).unwrap_or("").to_string();
        self.append("timestamp", fields! { "records" => count, "head" => head.as_str(), "service" => SERVICE, "time" => time.as_str() });
        Ok(json!({ "records": count, "time": time, "service": SERVICE, "file": reply.to_string_lossy() }))
    }

    /// Ask where the project is published whether there is a newer version. Only when the user
    /// asks: it is Homebody itself contacting a server, and it is written into the log.
    async fn check_update(&self) -> Result<Value, String> {
        // Where releases are published.
        const HOME: &str = "devesh16145/Homebody";
        let out = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new("/usr/bin/curl").args(["-sS", "--max-time", "10", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: homebody-update-check"]).arg(format!("https://api.github.com/repos/{HOME}/releases/latest")).output(),
        )
        .await
        .ok()
        .and_then(|r| r.ok());
        self.append("conn", fields! { "app" => "homebody", "host" => "api.github.com", "port" => 443, "decision" => "allow", "reason" => "you_allowed_once", "detail" => "self" });
        let Some(out) = out.filter(|o| o.status.success()) else { return Err("The place where Homebody is published could not be reached.".into()) };
        let reply: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let latest = reply.get("tag_name").and_then(Value::as_str).map(|t| t.trim_start_matches('v').to_string());
        let newer = latest.as_deref().map(|l| newer_version(l, crate::VERSION)).unwrap_or(false);
        Ok(json!({
            "current": crate::VERSION,
            "latest": latest,
            "newer": newer,
            "url": reply.get("html_url").and_then(Value::as_str),
            "message": match (&latest, newer) {
                (None, _) => "No version has been published yet.".to_string(),
                (Some(l), true) => format!("Version {l} is available. You have {}.", crate::VERSION),
                (Some(_), false) => format!("You have the newest version ({}).", crate::VERSION),
            },
        }))
    }

    /// The last so many days in a few numbers: what each app did, and what was stopped.
    fn summary(&self, days: u64) -> Value {
        let cfg = self.0.cfg.lock().unwrap().clone();
        let since = (chrono::Utc::now() - chrono::Duration::days(days.clamp(1, 90) as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        #[derive(Default)]
        struct Sum {
            allowed: u64,
            blocked: u64,
            bytes_out: u64,
            around: u64,
            secrets: u64,
            addresses: u64,
            places: HashMap<String, (u64, u64)>,
            blocked_places: HashMap<String, u64>,
        }
        let mut per: Vec<(String, Sum)> = Vec::new();
        let feed = self.0.feed.lock().unwrap();
        for row in feed.rows().iter().rev().take_while(|r| r.ts >= since) {
            let at = match per.iter().position(|(id, _)| *id == row.app) {
                Some(i) => i,
                None => {
                    per.push((row.app.clone(), Sum::default()));
                    per.len() - 1
                }
            };
            let s = &mut per[at].1;
            let label = if row.host.is_empty() { String::new() } else { catalog::describe(&row.host).0 };
            match row.verb {
                Verb::Allowed | Verb::Seen => {
                    s.allowed += 1;
                    s.bytes_out += row.bytes_out.unwrap_or(0);
                    let p = s.places.entry(label).or_default();
                    p.0 += 1;
                    p.1 += row.bytes_out.unwrap_or(0);
                }
                Verb::Blocked => {
                    s.blocked += 1;
                    match (row.reason, row.detail.as_str()) {
                        (Reason::AroundGate, _) => s.around += 1,
                        (_, "secret") => s.secrets += 1,
                        (_, "path") => s.addresses += 1,
                        _ => *s.blocked_places.entry(label).or_default() += 1,
                    }
                }
                Verb::Asked => {}
            }
        }
        let top = |m: &HashMap<String, u64>| {
            let mut v: Vec<(&String, &u64)> = m.iter().filter(|(k, _)| !k.is_empty()).collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            v.into_iter().take(5).map(|(k, n)| json!({ "name": k, "times": n })).collect::<Vec<_>>()
        };
        let apps: Vec<Value> = per
            .iter()
            .map(|(id, s)| {
                let mut places: Vec<(&String, &(u64, u64))> = s.places.iter().collect();
                places.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(b.1 .0.cmp(&a.1 .0)));
                json!({
                    "appId": id,
                    "appName": cfg.app(id).map(|a| a.name.clone()).unwrap_or_else(|| id.clone()),
                    "allowed": s.allowed, "blocked": s.blocked, "bytesOut": s.bytes_out,
                    "aroundGate": s.around, "secretsStopped": s.secrets, "addressesRefused": s.addresses,
                    "places": places.into_iter().take(5).map(|(k, (n, b))| json!({ "name": k, "times": n, "bytesOut": b })).collect::<Vec<_>>(),
                    "blockedPlaces": top(&s.blocked_places),
                })
            })
            .collect();
        json!({ "days": days.clamp(1, 90), "since": since, "apps": apps })
    }

    // -----------------------------------------------------------------------------------------
    // Fence and apps
    // -----------------------------------------------------------------------------------------

    fn org_name(&self) -> String {
        self.0.cfg.lock().unwrap().org.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Your organisation".into())
    }

    fn fleet_status(&self) -> Value {
        let cfg = self.0.cfg.lock().unwrap();
        let (records, _) = self.0.ledger.lock().unwrap().head();
        match crate::fleet::Membership::load() {
            Some(m) => json!({
                "joined": true, "org": m.org, "url": m.url, "deviceId": m.device_id, "joinedAt": m.joined_at,
                "lastSync": m.last_sync, "lastError": m.last_error, "sent": m.sent_seq, "waiting": records.saturating_sub(m.sent_seq),
                "policyVersion": cfg.org.as_ref().map(|p| p.version), "rules": cfg.org.as_ref().map(|p| p.rules.len()), "locked": cfg.mode_locked(),
            }),
            None => json!({ "joined": false }),
        }
    }

    /// Join an organisation's fleet with a code its administrator gave out.
    async fn fleet_join(&self, url: &str, code: &str, ca_file: Option<String>) -> Result<(), String> {
        use crate::fleet;
        if fleet::Membership::load().is_some() {
            return Err("This computer is already in a fleet. Leave it first.".into());
        }
        let url = fleet::clean_url(url)?;
        let code = code.trim();
        if code.is_empty() || code.len() > 200 {
            return Err("Type the code your administrator gave you.".into());
        }
        if let Some(ca) = &ca_file {
            if !std::path::Path::new(ca).is_file() {
                return Err("The certificate file could not be found.".into());
            }
        }
        let public_key = self.0.ledger.lock().unwrap().public_key_b64();
        let name = std::process::Command::new(if cfg!(windows) { "hostname.exe" } else { "/bin/hostname" }).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "computer".into());
        let body = json!({ "code": code, "deviceName": name, "user": fence::real_user(), "publicKey": public_key, "os": std::env::consts::OS, "version": crate::VERSION });
        self.append("conn", fields! { "app" => "homebody", "host" => fleet::host_of(&url), "port" => 443, "decision" => "allow", "reason" => "you_allowed_once", "detail" => "fleet" });
        let reply = fleet::request("POST", &format!("{url}/v1/join"), None, Some(&body), ca_file.as_deref()).await?;
        let field = |k: &str| reply.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| "The server's answer was not understood. Is that a Homebody fleet server?".to_string());
        let m = fleet::Membership { url, device_id: field("deviceId")?, secret: field("secret")?, fleet_key: field("fleetKey")?, org: field("org")?, ca_file, joined_at: now_iso(), sent_seq: 0, last_sync: None, last_error: None };
        m.save().map_err(|e| format!("Joining could not be saved: {e}"))?;
        self.append("fleet", fields! { "event" => "joined", "org" => m.org.as_str(), "server" => m.url.as_str(), "fleetKey" => m.fleet_key.as_str() });
        self.fleet_sync().await
    }

    /// Take the organisation's current rules and send it the records it does not have yet.
    pub async fn fleet_sync(&self) -> Result<(), String> {
        use crate::fleet;
        let Some(mut m) = fleet::Membership::load() else { return Err("This computer is not in a fleet.".into()) };
        let bearer = format!("{}.{}", m.device_id, m.secret);
        let result: Result<(), String> = async {
            // 1. Rules. Signed by the fleet's key, which was pinned when this computer joined.
            let signed = fleet::request("GET", &format!("{}/v1/policy", m.url), Some(&bearer), None, m.ca_file.as_deref()).await?;
            let policy = fleet::verify_policy(&signed, &m.fleet_key)?;
            let current = self.0.cfg.lock().unwrap().org.clone();
            match &current {
                Some(c) if c.version > policy.version => return Err("The server offered older rules than this computer already has, so they were not applied.".into()),
                Some(c) if *c == policy => {}
                _ => {
                    let (version, rules) = (policy.version, policy.rules.len());
                    // What the organisation trusts and blocks goes into this computer's own log
                    // in full, so a rule that opens a place is on record here and not only on
                    // the server that sent it.
                    let listed: Vec<Value> = policy.rules.iter().take(400).map(|r| json!({ "app": r.app, "host": r.host, "enabled": r.enabled })).collect();
                    let (ask, lock) = (policy.ask, policy.lock_mode);
                    self.0.cfg.lock().unwrap().apply_org(Some(policy));
                    self.save_cfg();
                    self.cancel_asks();
                    self.append("fleet", fields! { "event" => "rules", "org" => m.org.as_str(), "version" => version, "rules" => rules, "list" => Value::Array(listed), "ask" => ask, "lockMode" => lock });
                }
            }
            // 2. The log, from where the server says it stands, exactly as written.
            let head = fleet::request("GET", &format!("{}/v1/head", m.url), Some(&bearer), None, m.ca_file.as_deref()).await?;
            let mut sent = head.get("seq").and_then(Value::as_u64).unwrap_or(0);
            for _ in 0..40 {
                let lines = self.0.ledger.lock().unwrap().lines_after(sent, fleet::BATCH);
                if lines.is_empty() {
                    break;
                }
                let reply = fleet::request("POST", &format!("{}/v1/records", m.url), Some(&bearer), Some(&json!({ "after": sent, "records": lines })), m.ca_file.as_deref()).await?;
                let now = reply.get("seq").and_then(Value::as_u64).unwrap_or(sent);
                if now <= sent {
                    return Err("The server did not take the records.".into());
                }
                sent = now;
            }
            m.sent_seq = sent;
            Ok(())
        }
        .await;
        m.last_sync = Some(now_iso());
        m.last_error = result.as_ref().err().cloned();
        let _ = m.save();
        self.touch();
        result
    }

    pub async fn check_fence(&self) {
        let (apps, expected_rules) = {
            let cfg = self.0.cfg.lock().unwrap();
            (cfg.apps.clone(), fence::filter_rules(&cfg.provisioned(), fence::own_servers()))
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
            // A copy left behind by an earlier run of ours that ended abruptly is ours to stop.
            // Nothing else runs as this app's private user.
            if fence::is_port_open(p).await {
                fence::stop_all(&app).await;
                tokio::time::sleep(Duration::from_millis(1200)).await;
            }
            if fence::is_port_open(p).await {
                return Err(format!("Something is already running on this computer where {} would be. Quit the other {} first, so only the protected one runs.", app.name, app.name));
            }
        }
        if app.windowed && fence::window_running(&app).await {
            // A second start would only bring the open copy forward, and that copy may be unprotected.
            return Err(format!("{} is already open. Quit it first, then open it from here so the protected one is the only one running.", app.name));
        }
        let mut child = fence::start_managed(&app).await?;
        if app.windowed {
            // An app that cannot run this way quits at once; say so instead of showing it as open.
            if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(2500), child.wait()).await {
                return Err(format!("{} closed straight away. It may not be able to run protected this way yet.", app.name));
            }
            if let Some(pid) = child.id() {
                self.0.pids.lock().unwrap().insert(app_id.to_string(), pid);
            }
        }
        self.0.running.lock().unwrap().insert(app_id.to_string());
        self.append("start", fields! { "app" => app_id, "action" => "start" });
        let e = self.clone();
        let id = app_id.to_string();
        tokio::spawn(async move {
            let _ = child.wait().await;
            e.0.running.lock().unwrap().remove(&id);
            e.0.pids.lock().unwrap().remove(&id);
            e.append("start", fields! { "app" => id, "action" => "stop" });
        });
        Ok(())
    }

    async fn app_stop(&self, app_id: &str) -> Result<(), String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or_else(|| "Unknown app.".to_string())?;
        if app.windowed {
            let pid = self.0.pids.lock().unwrap().get(app_id).copied();
            if let Some(pid) = pid {
                fence::stop_pid(pid).await;
            }
        } else {
            fence::stop_all(&app).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.touch();
        Ok(())
    }

    /// Stop everything we started. Called when the app quits.
    pub async fn shutdown(&self) {
        #[cfg(unix)]
        crate::boxvm::stop().await;
        let running: Vec<String> = self.0.running.lock().unwrap().iter().cloned().collect();
        for id in running {
            let _ = self.app_stop(&id).await;
        }
        self.flush_repeats();
        let _ = self.0.stop.send(true);
        // Give the helpers a moment to stop what they started before the process exits.
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // -----------------------------------------------------------------------------------------
    // The command surface (docs/ipc.md). One implementation for the app, the CLI and scripts.
    // -----------------------------------------------------------------------------------------

    /// The Box as the window shows it: names beside ids, and, for one app, whether that agent
    /// can be put in and whether it is.
    #[cfg(unix)]
    fn box_status(&self, app_id: Option<&str>) -> Value {
        let mut st = crate::boxvm::status();
        let name = |id: Option<&str>| id.and_then(|id| self.0.cfg.lock().unwrap().app(id).map(|a| a.name.clone()));
        st["appName"] = json!(name(st["appId"].as_str()));
        if st["busy"].is_object() {
            st["busy"]["appName"] = json!(name(st["busy"]["appId"].as_str()));
        }
        if let Some(id) = app_id {
            st["agent"] = json!({ "id": id, "canInstall": crate::boxvm::recipe(id).is_some(), "installed": st["installed"].get(id).cloned().unwrap_or(Value::Null) });
        }
        st
    }

    /// Make the Box's disk. The guest's set-up traffic (none is needed for the first start)
    /// goes through this app's gate, so anything it did would be judged and logged like the app's own.
    #[cfg(unix)]
    async fn box_setup(&self, app_id: &str, image: Option<&str>) -> Result<String, String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or("Unknown app.")?;
        let message = crate::boxvm::setup(app.proxy_port, image).await?;
        self.append("box", fields! { "event" => "set up" });
        Ok(message)
    }

    /// Put an agent into the Box's prepared disk. The packages come through the agent's own
    /// gate; the places they come from are trusted for the length of the installation only.
    #[cfg(unix)]
    async fn box_install(&self, app_id: &str) -> Result<String, String> {
        let app = self.0.cfg.lock().unwrap().app(app_id).cloned().ok_or("Unknown app.")?;
        let recipe = crate::boxvm::recipe(&app.id).ok_or_else(|| format!("Homebody does not know how to install {} in the Box yet. Claude Code, Codex CLI and Gemini CLI can be.", app.name))?;
        let mut added = Vec::new();
        for host in recipe.hosts {
            if app.rule_for(host).is_none() && self.set_trust_inner(&app.id, host, true, "box install").is_ok() {
                added.push(*host);
            }
        }
        let result = crate::boxvm::install(app.proxy_port, &app.id, &app.name, &recipe).await;
        for host in added {
            let _ = self.remove_trust(&app.id, host);
        }
        let version = result?;
        self.append("box", fields! { "event" => "installed", "app" => app.id.as_str(), "version" => version.as_str() });
        Ok(version)
    }

    /// Start the Box for an app. What it sees of this Mac: a project's copy if one is named,
    /// else the app's own space.
    #[cfg(unix)]
    async fn dispatch_box_start(&self, app: &AppConfig, project: Option<String>) -> Result<(), String> {
        if !app.protected {
            return Err(format!("{} is not protected. Switch protection on for it first.", app.name));
        }
        let share = match project {
            Some(name) => {
                app.projects.iter().find(|p| p.name == name).ok_or("Unknown project.")?;
                projects::copy_dir(app, &name).filter(|d| d.is_dir()).ok_or("That project's copy is missing.")?.to_string_lossy().into_owned()
            }
            None => app.workspace.clone().filter(|w| std::path::Path::new(w).is_dir()).ok_or("This app has no folder to share with the Box. Finish setup first.")?,
        };
        crate::boxvm::start(app, &share).await?;
        self.append("box", fields! { "event" => "started", "app" => app.id.as_str(), "share" => share.as_str() });
        Ok(())
    }

    /// Run a long Box step without making the caller wait for it. Returns once the step has
    /// had a moment to mark itself busy; how it ends is in `box_status` (`last`).
    #[cfg(unix)]
    async fn box_in_background<F, Fut>(&self, what: &'static str, app_id: Option<String>, job: F)
    where
        F: FnOnce(Engine) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let me = self.clone();
        tokio::spawn(async move {
            let after = me.clone();
            if let Err(e) = job(me).await {
                // A step refused before it began has not recorded how it ended.
                crate::boxvm::record_failure(what, app_id.as_deref(), &e);
            }
            after.touch();
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    pub async fn dispatch(&self, cmd: &str, args: Value) -> Result<Value, String> {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let req = |k: &str| s(k).ok_or_else(|| format!("Missing \"{k}\"."));
        let b = |k: &str| args.get(k).and_then(Value::as_bool).ok_or_else(|| format!("Missing \"{k}\"."));
        let ok = |v: &dyn erased::Ser| v.to_value();
        match cmd {
            "get_status" => ok(&self.status()),
            "set_mode" => {
                let mode = Mode::parse(&req("mode")?).ok_or("Unknown mode.")?;
                if mode != Mode::Private && self.0.cfg.lock().unwrap().mode_locked() {
                    return Err(format!("{} keeps protection on for this computer.", self.org_name()));
                }
                self.set_mode_for(mode, args.get("minutes").and_then(Value::as_u64), "you");
                ok(&self.status())
            }
            "get_watch_review" => ok(&self.watch_review()),
            "finish_watch_review" => {
                let mut trust = Vec::new();
                for t in args.get("trust").and_then(Value::as_array).into_iter().flatten() {
                    let field = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_string).ok_or("Each place needs an app and a name.");
                    trust.push((field("appId")?, field("host")?));
                }
                self.finish_watch_review(&trust)?;
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
            "export_log" => {
                let e = self.clone();
                let into = match s("dir") {
                    Some(d) if std::path::Path::new(&d).is_absolute() => Some(std::path::PathBuf::from(d)),
                    Some(_) => return Err("The export folder must be a full path.".into()),
                    None => None,
                };
                let path = tokio::task::spawn_blocking(move || e.export_log(into)).await.map_err(|e| e.to_string())??;
                if args.get("reveal").and_then(Value::as_bool).unwrap_or(true) {
                    fence::reveal(&path, true).await;
                }
                Ok(json!({ "path": path }))
            }
            // The rules in a formal language (Cedar), as a folder next to the other exports.
            "export_policy" => {
                let dir = match s("dir") {
                    Some(d) if std::path::Path::new(&d).is_absolute() => std::path::PathBuf::from(d),
                    Some(_) => return Err("The export folder must be a full path.".into()),
                    None => paths::downloads_dir().join(format!("Homebody policy {}", chrono::Local::now().format("%Y-%m-%d %H.%M"))),
                };
                let cfg = self.0.cfg.lock().unwrap().clone();
                crate::policy::write(&cfg, &dir).map_err(|e| format!("The policy could not be saved: {e}"))?;
                let path = dir.to_string_lossy().into_owned();
                if args.get("reveal").and_then(Value::as_bool).unwrap_or(true) {
                    fence::reveal(&path, false).await;
                }
                Ok(json!({ "path": path }))
            }
            "get_receipts" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200).min(5000) as usize;
                ok(&self.0.ledger.lock().unwrap().tail(limit))
            }
            // Exactly what the administrator approves during setup, so it can be read first.
            "get_setup_script" => Ok(Value::String(fence::setup_script(&self.0.cfg.lock().unwrap().provisioned(), &fence::real_user()))),
            "get_policy_text" => Ok(Value::String(serde_json::to_string_pretty(&*self.0.cfg.lock().unwrap()).unwrap_or_default())),
            "run_setup" => {
                let cfg = self.0.cfg.lock().unwrap().clone();
                let mut r = fence::run_setup(&cfg.provisioned()).await;
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
            "app_add" => {
                let (id, port) = {
                    let mut cfg = self.0.cfg.lock().unwrap();
                    let id = cfg.add_custom(&req("name")?, &req("program")?)?;
                    let port = cfg.app(&id).map(|a| a.proxy_port).unwrap_or(0);
                    (id, port)
                };
                self.save_cfg();
                self.append("app", fields! { "action" => "added", "app" => id.as_str() });
                // Its gate starts now; its private space appears once setup is approved again.
                self.ensure_gate(port).await;
                // A windowed app is only protected when opened through us, so give it a launcher.
                let added = self.0.cfg.lock().unwrap().app(&id).cloned();
                let opener = self.0.opener.lock().unwrap().clone();
                if let (Some(app), Some(opener)) = (added.filter(|a| a.windowed), opener) {
                    let _ = fence::write_launcher(&app, &opener);
                }
                self.check_fence().await;
                ok(&self.status())
            }
            // A browser profile kept for AI sites: which browsers can have one, and which sites exist.
            "get_ai_sites" => {
                let cfg = self.0.cfg.lock().unwrap();
                let app = s("appId").and_then(|id| cfg.app(&id).cloned());
                let mut browsers = Vec::new();
                let mut dirs = vec![std::path::PathBuf::from("/Applications")];
                if let Some(home) = dirs::home_dir() {
                    dirs.push(home.join("Applications"));
                }
                for name in catalog::KNOWN_BROWSERS {
                    if let Some(path) = dirs.iter().map(|d| d.join(format!("{name}.app"))).find(|p| p.is_dir()) {
                        let path = path.to_string_lossy().into_owned();
                        let added = cfg.apps.iter().find(|a| a.kind == "browser" && a.binary == path).map(|a| a.id.clone());
                        browsers.push(json!({ "name": name, "path": path, "appId": added }));
                    }
                }
                let sites: Vec<Value> = catalog::AI_SITES
                    .iter()
                    .map(|site| json!({ "id": site.id, "name": site.name, "url": site.url, "on": app.as_ref().map(|a| a.sites.iter().any(|x| x == site.id)).unwrap_or(false), "places": site.hosts.iter().map(|(h, why)| json!({ "host": h, "purpose": why })).collect::<Vec<_>>() }))
                    .collect();
                Ok(json!({ "browsers": browsers, "sites": sites }))
            }
            "browser_add" => {
                let sites: Vec<String> = args.get("sites").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let (id, port) = {
                    let mut cfg = self.0.cfg.lock().unwrap();
                    let id = cfg.add_browser(&req("browser")?, &sites)?;
                    let port = cfg.app(&id).map(|a| a.proxy_port).unwrap_or(0);
                    (id, port)
                };
                self.save_cfg();
                self.append("app", fields! { "action" => "added", "app" => id.as_str(), "sites" => sites.join(",") });
                self.ensure_gate(port).await;
                let added = self.0.cfg.lock().unwrap().app(&id).cloned();
                let opener = self.0.opener.lock().unwrap().clone();
                if let (Some(app), Some(opener)) = (added, opener) {
                    let _ = fence::write_launcher(&app, &opener);
                }
                self.check_fence().await;
                let mut st = serde_json::to_value(self.status()).map_err(|e| e.to_string())?;
                st["addedId"] = json!(id);
                Ok(st)
            }
            // Packs: the places a package manager fetches from, as one switch (see catalog::PACKS).
            "get_packs" => {
                let id = req("appId")?;
                let app = self.0.cfg.lock().unwrap().app(&id).cloned().ok_or("Unknown app.")?;
                let list: Vec<Value> = if app.kind == "agent" {
                    catalog::PACKS.iter().map(|p| json!({ "id": p.id, "name": p.name, "what": p.what, "hosts": p.hosts, "downloadsOnly": p.downloads_only, "on": app.packs.iter().any(|x| x == p.id) })).collect()
                } else {
                    Vec::new()
                };
                ok(&list)
            }
            "set_pack" => {
                let (app, pack, on) = (req("appId")?, req("pack")?, b("on")?);
                self.0.cfg.lock().unwrap().set_pack(&app, &pack, on)?;
                self.save_cfg();
                self.append("trust", fields! { "app" => app.as_str(), "pack" => pack.as_str(), "enabled" => on, "via" => "pack" });
                self.touch();
                ok(&self.trust_list(Some(&app)))
            }
            "browser_set_site" => {
                let (app, site, on) = (req("appId")?, req("site")?, b("on")?);
                self.0.cfg.lock().unwrap().set_site(&app, &site, on)?;
                self.save_cfg();
                self.append("trust", fields! { "app" => app.as_str(), "site" => site.as_str(), "enabled" => on, "via" => "site" });
                self.touch();
                ok(&self.trust_list(Some(&app)))
            }
            // An organisation's fleet (see `fleet`): join, leave, what is going on.
            "fleet_status" => Ok(self.fleet_status()),
            "fleet_join" => {
                self.fleet_join(&req("url")?, &req("code")?, s("caFile")).await?;
                Ok(self.fleet_status())
            }
            "fleet_sync" => {
                self.fleet_sync().await?;
                Ok(self.fleet_status())
            }
            "fleet_leave" => {
                if crate::fleet::Membership::load().is_none() {
                    return Err("This computer is not in a fleet.".into());
                }
                // Leaving is written down first and sent, so the organisation's copy of the log
                // ends with it rather than just going quiet. Then the membership is forgotten.
                let org = self.0.cfg.lock().unwrap().org.as_ref().map(|p| p.name.clone());
                self.append("fleet", fields! { "event" => "left", "org" => org });
                let _ = self.fleet_sync().await;
                crate::fleet::Membership::forget();
                self.0.cfg.lock().unwrap().apply_org(None);
                self.save_cfg();
                self.touch();
                Ok(self.fleet_status())
            }
            // The Box: an agent in a Linux virtual machine with no network interface (see `boxvm`).
            #[cfg(unix)]
            "box_status" => Ok(self.box_status(s("appId").as_deref())),
            // Making the disk and putting an agent in take minutes. From the command line the
            // call waits for the end; the window passes `background` and follows `box_status`.
            #[cfg(unix)]
            "box_setup" => {
                let (app_id, image) = (s("appId").unwrap_or_else(|| "claude-code".into()), s("image"));
                self.0.cfg.lock().unwrap().app(&app_id).ok_or("Unknown app.")?;
                if args.get("background").and_then(Value::as_bool) == Some(true) {
                    self.box_in_background("setup", None, |me| async move { me.box_setup(&app_id, image.as_deref()).await.map(|_| ()) }).await;
                    return Ok(self.box_status(None));
                }
                let message = self.box_setup(&app_id, image.as_deref()).await?;
                Ok(json!({ "message": message, "status": crate::boxvm::status() }))
            }
            #[cfg(unix)]
            "box_install" => {
                let app_id = req("appId")?;
                let app = self.0.cfg.lock().unwrap().app(&app_id).cloned().ok_or("Unknown app.")?;
                crate::boxvm::recipe(&app.id).ok_or_else(|| format!("Homebody does not know how to install {} in the Box yet. Claude Code, Codex CLI and Gemini CLI can be.", app.name))?;
                if args.get("background").and_then(Value::as_bool) == Some(true) {
                    let id = app_id.clone();
                    self.box_in_background("install", Some(app_id.clone()), |me| async move { me.box_install(&id).await.map(|_| ()) }).await;
                    return Ok(self.box_status(Some(&app_id)));
                }
                let version = self.box_install(&app_id).await?;
                Ok(json!({ "message": format!("{} is in the Box: {version}", app.name) }))
            }
            // The window's one button: start the Box for this agent if it is not running, and
            // open a Terminal window in it with the agent.
            #[cfg(unix)]
            "box_open" => {
                let app = self.0.cfg.lock().unwrap().app(&req("appId")?).cloned().ok_or("Unknown app.")?;
                let recipe = crate::boxvm::recipe(&app.id).ok_or_else(|| format!("{} cannot be put in the Box yet.", app.name))?;
                if !crate::boxvm::installed().contains_key(&app.id) {
                    return Err(format!("{} is not in the Box yet.", app.name));
                }
                let now = crate::boxvm::status();
                match (now["running"].as_bool() == Some(true), now["appId"].as_str()) {
                    (true, Some(id)) if id == app.id => {}
                    (true, other) => {
                        let name = other.and_then(|id| self.0.cfg.lock().unwrap().app(id).map(|a| a.name.clone())).unwrap_or_else(|| "another app".into());
                        return Err(format!("The Box is running for {name}. Stop it first: one Box runs at a time."));
                    }
                    _ => {
                        self.dispatch_box_start(&app, s("project")).await?;
                    }
                }
                crate::boxvm::open_terminal(&app, recipe.program, &self.box_extras(&app.id)).await?;
                Ok(self.box_status(Some(&app.id)))
            }
            #[cfg(unix)]
            "box_dismiss" => {
                crate::boxvm::dismiss_last();
                Ok(self.box_status(s("appId").as_deref()))
            }
            #[cfg(unix)]
            "box_start" => {
                let app = self.0.cfg.lock().unwrap().app(&req("appId")?).cloned().ok_or("Unknown app.")?;
                self.dispatch_box_start(&app, s("project")).await?;
                Ok(crate::boxvm::status())
            }
            #[cfg(unix)]
            "box_stop" => {
                let was = crate::boxvm::status();
                crate::boxvm::stop().await;
                if was["running"].as_bool() == Some(true) {
                    self.append("box", fields! { "event" => "stopped", "app" => was["appId"].as_str().unwrap_or("") });
                }
                Ok(crate::boxvm::status())
            }
            #[cfg(unix)]
            "box_argv" => {
                let rest: Vec<String> = args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let extras = crate::boxvm::running_app().map(|id| self.box_extras(&id)).unwrap_or_default();
                ok(&crate::boxvm::argv(&req("program")?, &rest, args.get("interactive").and_then(Value::as_bool).unwrap_or(false), &extras)?)
            }
            "get_install" => Ok(self.install_info()),
            // Each protected app's own icon, from its maker's Mac app when installed: { appId: dataUrl }.
            "get_app_icons" => {
                let apps: Vec<_> = self.0.cfg.lock().unwrap().apps.iter().filter(|a| a.protected).cloned().collect();
                let dir = paths::icons_dir();
                let mut out = serde_json::Map::new();
                for a in apps {
                    if let Some(url) = crate::appicons::icon(&a, &dir).await {
                        out.insert(a.id.clone(), Value::String(url));
                    }
                }
                Ok(Value::Object(out))
            }
            // Copies the app into Applications. The shell then restarts from the new copy.
            "install_app" => {
                let bundle = self.0.opener.lock().unwrap().as_deref().and_then(fence::bundle_of).ok_or("Only the Homebody app can install itself.")?;
                let path = fence::install_to_applications(&bundle).await?;
                Ok(json!({ "path": path }))
            }
            "discover_apps" => {
                let cfg = self.0.cfg.lock().unwrap().clone();
                ok(&tokio::task::spawn_blocking(move || fence::discover(&cfg)).await.map_err(|e| e.to_string())?)
            }
            // Open an app the way its kind is opened: a window, a protected terminal, a service.
            "app_open" => {
                let id = req("appId")?;
                let app = self.app_if_ready(&id)?;
                if app.kind == "agent" {
                    fence::open_terminal(&app, None).await?;
                } else {
                    self.app_start(&id).await?;
                }
                Ok(Value::Null)
            }
            // A small app of its own for the Dock or Spotlight that opens this one protected.
            "app_launcher" => {
                let app = self.0.cfg.lock().unwrap().app(&req("appId")?).cloned().ok_or("Unknown app.")?;
                let opener = self.0.opener.lock().unwrap().clone().ok_or("Launchers are made by the Homebody app. Open it and try again.")?;
                let path = fence::write_launcher(&app, &opener)?;
                if args.get("reveal").and_then(Value::as_bool).unwrap_or(true) {
                    fence::reveal(&path, true).await;
                }
                Ok(json!({ "path": path }))
            }
            "app_remove" => {
                let id = req("appId")?;
                if self.0.running.lock().unwrap().contains(&id) {
                    return Err("Stop it first.".into());
                }
                let app = self.0.cfg.lock().unwrap().app(&id).cloned().ok_or("Unknown app.")?;
                self.0.cfg.lock().unwrap().remove_custom(&id)?;
                let _ = self.0.vault.lock().unwrap().remove_app(&id);
                self.save_cfg();
                fence::remove_launcher(&app);
                self.0.health.lock().unwrap().remove(&id);
                fence::stop_all(&app).await;
                self.append("app", fields! { "action" => "removed", "app" => id.as_str() });
                self.check_fence().await;
                ok(&self.status())
            }
            "set_limit" => {
                self.set_limit(&req("appId")?, &req("host")?, args.get("mb").and_then(Value::as_u64))?;
                ok(&self.trust_list(None))
            }
            "ack_presets" => {
                self.0.cfg.lock().unwrap().presets_seen = catalog::presets_version();
                self.save_cfg();
                ok(&self.status())
            }
            // A question nobody answered in time, answered now. "always" and "block" make a rule,
            // as they would have; "later" only clears it away, and the app is asked about again
            // the next time it tries.
            "missed_answer" => {
                let id = req("id")?;
                let ask = self.0.missed.lock().unwrap().iter().find(|m| m.id == id).cloned().ok_or("That question has already been dealt with.")?;
                match req("choice")?.as_str() {
                    "always" => {
                        self.set_trust_inner(&ask.app_id, &ask.host, true, "prompt, afterwards")?;
                        for c in catalog::companions(&ask.host) {
                            let _ = self.set_trust_inner(&ask.app_id, c, true, "prompt, afterwards");
                        }
                    }
                    "block" => self.set_trust_inner(&ask.app_id, &ask.host, false, "prompt, afterwards")?,
                    "later" => {
                        self.0.quiet.lock().unwrap().remove(&key(&ask.app_id, &ask.host));
                    }
                    _ => return Err("Choose always, block or later.".into()),
                }
                self.0.missed.lock().unwrap().retain(|m| m.id != id);
                self.touch();
                ok(&self.status())
            }
            "set_ask_sound" => {
                self.0.cfg.lock().unwrap().ask_sound = b("on")?;
                self.save_cfg();
                self.touch();
                ok(&self.status())
            }
            "set_ask" => {
                let on = b("on")?;
                if self.0.cfg.lock().unwrap().org.as_ref().and_then(|p| p.ask).is_some() {
                    return Err(format!("{} decides whether new places are asked about.", self.org_name()));
                }
                self.0.cfg.lock().unwrap().ask = on;
                self.save_cfg();
                self.append("mode", fields! { "ask" => on, "why" => "you" });
                if !on {
                    self.cancel_asks();
                }
                ok(&self.status())
            }
            "check_update" => self.check_update().await,
            "timestamp_log" => self.timestamp_log().await,
            "get_summary" => Ok(self.summary(args.get("days").and_then(Value::as_u64).unwrap_or(7))),
            "get_private_folders" => Ok(self.private_folders()),
            "add_private_folder" | "remove_private_folder" => {
                let (e, path, add) = (self.clone(), req("path")?, cmd == "add_private_folder");
                tokio::task::spawn_blocking(move || e.set_private_folder(&path, add)).await.map_err(|e| e.to_string())??;
                Ok(self.private_folders())
            }
            "get_inside_rules" => ok(&self.inside_list(s("appId").as_deref())),
            "set_inside_rule" => {
                self.set_inside(&req("appId")?, &req("host")?, &req("path")?, s("method"), Some(b("enabled")?))?;
                ok(&self.inside_list(None))
            }
            "remove_inside_rule" => {
                self.set_inside(&req("appId")?, &req("host")?, &req("path")?, s("method"), None)?;
                ok(&self.inside_list(None))
            }
            "app_set_secrets" => {
                let id = req("appId")?;
                let on = b("on")?;
                self.0.cfg.lock().unwrap().app_mut(&id).ok_or("Unknown app.")?.secrets = on;
                self.save_cfg();
                self.append("app", fields! { "action" => if on { "stop_secrets_on" } else { "stop_secrets_off" }, "app" => id.as_str() });
                ok(&self.status())
            }
            "allow_secret" => {
                self.allow_secret(&req("appId")?, &req("fingerprint")?, b("allowed")?)?;
                Ok(Value::Null)
            }
            "get_keys" => {
                let id = req("appId")?;
                let app = self.0.cfg.lock().unwrap().app(&id).cloned().ok_or("Unknown app.")?;
                let known: Vec<Value> = crate::keys::known_names().into_iter().map(|n| json!({ "name": n, "maker": crate::keys::maker(n).unwrap_or(""), "hosts": crate::keys::usual_places(n) })).collect();
                ok(&json!({ "keys": Self::key_infos(&app), "known": known, "canKeep": Self::cannot_inspect(&app).is_none(), "why": Self::cannot_inspect(&app) }))
            }
            "key_set" => {
                let hosts = args.get("hosts").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>()).filter(|h| !h.is_empty());
                self.set_key(&req("appId")?, &req("name")?, &req("value")?, hosts)?;
                ok(&self.status())
            }
            "key_remove" => {
                self.remove_key(&req("appId")?, &req("name")?)?;
                ok(&self.status())
            }
            "app_set_inspect" => {
                self.set_inspect(&req("appId")?, b("on")?)?;
                ok(&self.status())
            }
            "app_set_protected" => {
                let id = req("appId")?;
                let on = b("protected")?;
                if !on && self.0.cfg.lock().unwrap().mode_locked() {
                    return Err(format!("{} keeps protection on for this computer.", self.org_name()));
                }
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
                if app.windowed {
                    return Err(format!("{} is a windowed app. Open it instead.", app.name));
                }
                let dir = match s("project") {
                    Some(name) => {
                        app.projects.iter().find(|p| p.name == name).ok_or("Unknown project.")?;
                        let d = projects::copy_dir(&app, &name).filter(|d| d.is_dir()).ok_or("That project's copy is missing.")?;
                        Some(d.to_string_lossy().into_owned())
                    }
                    None => None,
                };
                fence::open_terminal(&app, dir.as_deref()).await?;
                Ok(Value::Null)
            }
            // Projects: bring a folder in for an agent, review what it changed, apply it back.
            "get_projects" => {
                let cfg = self.0.cfg.lock().unwrap();
                let app = cfg.app(&req("appId")?).ok_or("Unknown app.")?;
                ok(&app.projects.iter().map(|p| projects::info(app, p)).collect::<Vec<_>>())
            }
            "project_import" => {
                let app = self.app_if_ready(&req("appId")?)?;
                let ws = app.workspace.clone().ok_or("This app does not work on folders.")?;
                let source = projects::validate_source(&req("path")?, std::path::Path::new(&ws))?;
                if app.projects.iter().any(|p| std::path::Path::new(&p.source) == source) {
                    return Err("That folder has already been brought in. Remove its copy first to bring it in again.".into());
                }
                let (a, src) = (app.clone(), source.clone());
                let project = tokio::task::spawn_blocking(move || projects::import(&a, &src)).await.map_err(|e| e.to_string())??;
                {
                    let mut cfg = self.0.cfg.lock().unwrap();
                    cfg.app_mut(&app.id).ok_or("Unknown app.")?.projects.push(project.clone());
                }
                self.save_cfg();
                self.append("project", fields! { "app" => app.id, "action" => "brought_in", "name" => project.name, "source" => project.source });
                let cfg = self.0.cfg.lock().unwrap();
                let app = cfg.app(&app.id).ok_or("Unknown app.")?;
                let mut out = ok(&projects::info(app, &project))?;
                // Places the project's own file asks for and that are not decided yet. Read
                // from the user's folder only; shown, never applied by themselves.
                let suggested: Vec<Value> = projects::suggested_places(&source)
                    .into_iter()
                    .filter(|h| !app.trust.iter().any(|r| &r.host == h))
                    .map(|h| {
                        let (label, purpose) = catalog::describe_pattern(&h);
                        json!({ "host": h, "label": label, "purpose": purpose })
                    })
                    .collect();
                out["suggested"] = json!(suggested);
                Ok(out)
            }
            // Real attempts to get out, made as the app, and what came of each (see fence::leak_test).
            "leak_test" => {
                let app = self.app_if_ready(&req("appId")?)?;
                let cases = fence::leak_test(&app).await?;
                let count = |o: fence::Outcome| cases.iter().filter(|c| c.outcome == o).count();
                let (stopped, through, unknown) = (count(fence::Outcome::Stopped), count(fence::Outcome::Through), count(fence::Outcome::Unknown));
                self.append("app", fields! { "action" => "tested", "app" => app.id.as_str(), "stopped" => stopped, "through" => through, "unknown" => unknown });
                // "held": every attempt was seen to be stopped. Not true while any is unknown.
                ok(&json!({ "appId": app.id, "appName": app.name, "at": now_iso(), "held": through == 0 && unknown == 0 && stopped > 0, "through": through, "unknown": unknown, "cases": cases }))
            }
            "project_changes" | "project_apply" | "project_remove" | "project_restart" => {
                let app = self.0.cfg.lock().unwrap().app(&req("appId")?).cloned().ok_or("Unknown app.")?;
                let name = req("name")?;
                let project = app.projects.iter().find(|p| p.name == name).cloned().ok_or("Unknown project.")?;
                let copy = projects::copy_dir(&app, &name).ok_or("This app does not work on folders.")?;
                let (base, source) = (projects::base_dir(&app.id, &name), std::path::PathBuf::from(&project.source));
                match cmd {
                    "project_changes" => {
                        let set = tokio::task::spawn_blocking(move || projects::change_set(&copy, &base, &source)).await.map_err(|e| e.to_string())??;
                        ok(&set)
                    }
                    "project_apply" => {
                        let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
                        let result = tokio::task::spawn_blocking(move || projects::apply(&copy, &base, &source, overwrite)).await.map_err(|e| e.to_string())??;
                        self.append("project", fields! { "app" => app.id, "action" => "applied", "name" => name, "files" => result.applied, "left" => result.skipped.len() });
                        ok(&result)
                    }
                    // Start over: throw the copy away, with whatever the agent did to it, and
                    // bring your folder in again as it is now. Your own folder is not touched.
                    "project_restart" => {
                        if !source.is_dir() {
                            return Err(format!("Your folder {} is not there any more, so there is nothing to start over from.", project.source));
                        }
                        // The copy keeps its name: an open terminal, and the editor plugins, know
                        // it by that. So the name must be free again by the time it is brought in.
                        let (mut a, n, src) = (app.clone(), name.clone(), source.clone());
                        a.projects.retain(|p| p.name != name);
                        let fresh = tokio::task::spawn_blocking(move || {
                            projects::remove(&a, &n)?;
                            projects::import(&a, &src)
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                        let fresh = match fresh {
                            Ok(f) => f,
                            Err(e) => {
                                // The copy is gone and no new one could be made: say so, and do
                                // not go on listing a project that is not there.
                                if !projects::copy_dir(&app, &name).is_some_and(|c| c.exists()) {
                                    if let Some(a) = self.0.cfg.lock().unwrap().app_mut(&app.id) {
                                        a.projects.retain(|p| p.name != name);
                                    }
                                    self.save_cfg();
                                    self.append("project", fields! { "app" => app.id.as_str(), "action" => "removed", "name" => name.as_str() });
                                    return Err(format!("The copy was thrown away, but your folder could not be brought in again: {e} Bring it in again when that is sorted out; your own folder was not touched."));
                                }
                                return Err(e);
                            }
                        };
                        {
                            let mut cfg = self.0.cfg.lock().unwrap();
                            let a = cfg.app_mut(&app.id).ok_or("Unknown app.")?;
                            a.projects.retain(|p| p.name != name);
                            a.projects.push(fresh.clone());
                        }
                        self.save_cfg();
                        self.append("project", fields! { "app" => app.id.as_str(), "action" => "started_over", "name" => fresh.name.as_str() });
                        let cfg = self.0.cfg.lock().unwrap();
                        ok(&projects::info(cfg.app(&app.id).ok_or("Unknown app.")?, &fresh))
                    }
                    _ => {
                        let a = app.clone();
                        let n = name.clone();
                        tokio::task::spawn_blocking(move || projects::remove(&a, &n)).await.map_err(|e| e.to_string())??;
                        {
                            let mut cfg = self.0.cfg.lock().unwrap();
                            cfg.app_mut(&app.id).ok_or("Unknown app.")?.projects.retain(|p| p.name != name);
                        }
                        self.save_cfg();
                        self.append("project", fields! { "app" => app.id, "action" => "removed", "name" => name });
                        Ok(Value::Null)
                    }
                }
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
                // Variables a pipeline hands over on purpose (an API key, say). Nothing else of the
                // caller's environment reaches a protected program, and the ones that point it at
                // its gate cannot be replaced.
                let mut extra: Vec<(String, String)> = Vec::new();
                if let Some(env) = args.get("env").and_then(Value::as_object).filter(|e| !e.is_empty()) {
                    let reserved: Vec<String> = fence::fenced_env(&app).into_iter().map(|(k, _)| k.to_ascii_uppercase()).collect();
                    for (k, v) in env {
                        let valid = !k.is_empty() && k.len() <= 80 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit());
                        if !valid || reserved.contains(&k.to_ascii_uppercase()) {
                            return Err(format!("\"{k}\" cannot be handed to a protected program."));
                        }
                        extra.push((k.clone(), v.as_str().unwrap_or_default().to_string()));
                    }
                }
                // Handed-over variables may be secrets, and a command line can be read by every
                // user of the computer and is written down by sudo. They go into a file only you
                // and this app can read, in its workspace; the program reads it and deletes it.
                #[cfg(unix)]
                if !extra.is_empty() && !app.windowed {
                    let ws = app.workspace.clone().filter(|w| std::path::Path::new(w).is_dir()).ok_or_else(|| format!("{} has no folder of its own to hand variables over in.", app.name))?;
                    let file = std::path::Path::new(&ws).join(format!(".homebody-env-{:016x}", rand::random::<u64>()));
                    let body: String = extra.iter().map(|(k, v)| format!("export {k}='{}'\n", v.replace('\'', "'\\''"))).collect();
                    {
                        use std::io::Write;
                        use std::os::unix::fs::OpenOptionsExt;
                        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o660).open(&file).map_err(|e| format!("The variables could not be handed over: {e}"))?;
                        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
                    }
                    let mut wrapped = vec!["-c".to_string(), ". \"$1\"; rm -f \"$1\"; shift; exec \"$@\"".to_string(), "sh".to_string(), file.to_string_lossy().into_owned(), program];
                    wrapped.extend(rest);
                    return ok(&fence::fenced_argv(&app, "/bin/sh", &wrapped, &[], s("cwd").as_deref()));
                }
                ok(&fence::fenced_argv(&app, &program, &rest, &extra, s("cwd").as_deref()))
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
