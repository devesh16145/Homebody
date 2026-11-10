//! OS-level enforcement on macOS, with no virtual machine.
//!
//! A protected app runs behind three independent layers:
//!
//! 1. **Its own OS user.** Ordinary file permissions keep it out of the real user's home folder.
//! 2. **A seatbelt profile** applied at launch: no network except the app's own local gate, no
//!    name lookups, nothing readable under the real user's home.
//! 3. **Packet filter rules** keyed on that OS user, kept loaded across restarts by a small root
//!    watchdog. They hold even if the seatbelt layer is ever removed from macOS.
//!
//! Layers 1 and 3 need one administrator approval ("setup"). The engine refuses to start anything
//! protected unless every layer checks out.

use crate::config::{AppConfig, Config};
use crate::model::{FenceState, SetupResult};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub const GROUP: &str = "_sai";
pub const SYSTEM_DIR: &str = "/Library/Application Support/SandboxAI";
pub const FILTER_STATUS: &str = "/var/run/sandboxai.fence";
pub const FILTER_ANCHOR: &str = "sandboxai";
pub const WORKSPACE_ROOT: &str = "/Users/Shared/SandboxAI";

#[derive(Clone, Debug, Default)]
pub struct Health {
    pub checked: bool,
    pub installed: bool,
    pub user_exists: bool,
    pub group_ok: bool,
    pub can_run: bool,
    pub seatbelt_ok: bool,
    pub filter_ok: bool,
    /// The installed packet filter rules are the ones this version of the app generates.
    pub rules_current: bool,
    pub workspace_ok: bool,
}

impl Health {
    pub fn layers_ok(&self) -> bool {
        self.user_exists && self.group_ok && self.can_run && self.seatbelt_ok && self.filter_ok && self.rules_current && self.workspace_ok
    }

    pub fn state(&self) -> FenceState {
        if !self.checked {
            FenceState::Checking
        } else if self.layers_ok() || (dev_allow_partial() && self.user_exists && self.can_run && self.seatbelt_ok) {
            FenceState::Up
        } else if !self.user_exists {
            FenceState::NotInstalled
        } else {
            FenceState::Down
        }
    }

    /// Why setup is needed, in plain words. Empty when everything is in place.
    pub fn reasons(&self, app_name: &str) -> Vec<String> {
        let mut r = Vec::new();
        if !self.checked || self.state() == FenceState::Up {
            return r;
        }
        if !self.user_exists {
            r.push(format!("{app_name} does not have its private space yet."));
            return r;
        }
        if !self.filter_ok {
            r.push("Protection was not restored after the last restart.".to_string());
        } else if !self.rules_current {
            r.push("Protection needs a quick update for this version of Sandbox-ai.".to_string());
        }
        if !self.group_ok {
            r.push(format!("{app_name}'s private space needs an update so it cannot look into your home folder."));
        }
        if !self.can_run {
            r.push(format!("Sandbox-ai is not yet allowed to start {app_name} in its private space."));
        } else if !self.seatbelt_ok {
            r.push(format!("The launch-time protection for {app_name} could not be confirmed."));
        }
        if !self.workspace_ok {
            r.push(format!("{app_name}'s workspace folder is missing."));
        }
        r
    }
}

/// Development only: treat the fence as up when the user and seatbelt layers pass but the packet
/// filter layer is not installed. Never honoured in release builds.
pub fn dev_allow_partial() -> bool {
    cfg!(debug_assertions) && std::env::var("SAI_DEV_ALLOW_PARTIAL").map(|v| v == "1").unwrap_or(false)
}

pub fn real_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "nobody".into())
}

pub fn real_home() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "/var/empty".into())
}

const VIOLATION_TAG: &str = "sai-app=";

/// One refused attempt to reach the network without going through the gate.
#[derive(Debug, PartialEq, Clone)]
pub struct Violation {
    pub app: String,
    /// Name of the program that tried, as the system reports it.
    pub process: String,
    /// "direct" (straight to an address), "lookup" (asked the system to resolve a name),
    /// "listen" (accept connections from other computers), "gate" (another app's gate), "other".
    pub what: &'static str,
    /// The port, for "direct" and "gate". macOS does not report the address.
    pub port: u16,
    /// How many attempts this report stands for (macOS folds rapid duplicates into one).
    pub count: u64,
}

/// Read one seatbelt refusal as macOS logs it, for example:
///
/// ```text
/// Sandbox: curl(40242) deny(1) network-outbound remote:*:443
/// sai-app=claude-code;net
/// ```
///
/// Returns nothing for refusals that do not carry our tag. The program name comes from the
/// program itself, so it is reduced to plain characters before it is shown anywhere.
pub fn parse_violation(message: &str) -> Option<Violation> {
    let mut lines = message.lines();
    let first = lines.next()?;
    let tag = lines.find_map(|l| l.trim().strip_prefix(VIOLATION_TAG))?;
    let (app, hint) = tag.split_once(';').unwrap_or((tag, "net"));
    if app.is_empty() || !app.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    // "3 duplicate reports for Sandbox: …" stands for that many more attempts.
    let (count, rest) = match first.split_once(" duplicate report") {
        Some((n, rest)) => (n.trim().parse::<u64>().unwrap_or(1).max(1), rest.split_once("Sandbox: ")?.1),
        None => (1, first.strip_prefix("Sandbox: ")?),
    };
    let (who, rest) = rest.split_once(" deny(")?;
    let process: String = who.rsplit_once('(').map(|(name, _)| name).unwrap_or(who).chars().filter(|c| c.is_ascii_alphanumeric() || "-_.+ ".contains(*c)).take(32).collect();
    let rest = rest.split_once(") ")?.1;
    let (op, target) = rest.split_once(' ').unwrap_or((rest, ""));
    let port = target.rsplit_once(':').and_then(|(_, p)| p.trim().parse::<u16>().ok()).unwrap_or(0);
    let what = match (hint, op) {
        ("dns", _) => "lookup",
        ("gate", _) => "gate",
        (_, "network-outbound") if target.contains("mDNSResponder") => "lookup",
        (_, "network-outbound") if target.starts_with('/') => "other",
        (_, "network-outbound") => "direct",
        (_, "network-bind") | (_, "network-inbound") => "listen",
        _ => "other",
    };
    Some(Violation { app: app.to_string(), process: if process.trim().is_empty() { "A program".into() } else { process.trim().to_string() }, what, port: if matches!(what, "direct" | "gate") { port } else { 0 }, count })
}

/// Follow the system log for refusals carrying our tag. Each line of the child's output is one
/// JSON record; hand `eventMessage` to [`parse_violation`]. Only records written by the kernel
/// are accepted, because any program can write to the log under any name.
pub fn violation_stream() -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new("/usr/bin/log")
        .args(["stream", "--style", "ndjson", "--predicate"])
        .arg(format!("process == \"kernel\" AND sender == \"Sandbox\" AND eventMessage CONTAINS \"{VIOLATION_TAG}\""))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// The refusal in one record of [`violation_stream`]'s output, if it is one of ours.
pub fn violation_in(line: &str) -> Option<Violation> {
    let rec: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if rec.get("processImagePath").and_then(|v| v.as_str()) != Some("/kernel") || rec.get("processID").and_then(|v| v.as_u64()) != Some(0) {
        return None;
    }
    parse_violation(rec.get("eventMessage")?.as_str()?)
}

/// Home folder of a protected app's OS user.
pub fn home_for(os_user: &str) -> String {
    format!("/var/sai/{}", os_user.trim_start_matches("_sai").trim_start_matches('_'))
}

/// The seatbelt profile for one app. Later rules win over earlier ones.
///
/// Every network refusal carries a tag naming the app and what was refused. macOS writes refusals
/// to the system log with that tag, which is how attempts to go around the gate reach the Feed
/// (see [`parse_violation`]).
pub fn seatbelt_profile(app: &AppConfig) -> String {
    let tag = |what: &str| format!("(with message \"{VIOLATION_TAG}{};{what}\")", app.id);
    let mut p = String::from("(version 1)\n(allow default)\n");
    // Network: nothing, then exactly the local gate and the listed local services.
    p.push_str(&format!("(deny network* {})\n", tag("net")));
    p.push_str(&format!("(allow network-outbound (remote ip \"localhost:{}\"))\n", app.proxy_port));
    for port in &app.local_ports {
        p.push_str(&format!("(allow network-outbound (remote ip \"localhost:{port}\"))\n"));
    }
    if app.loopback_all {
        // Model servers talk to their own helper processes over arbitrary local ports.
        // Any local port, then, except another app's gate.
        p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
        for port in crate::catalog::GATE_PORTS.filter(|g| *g != app.proxy_port) {
            p.push_str(&format!("(deny network-outbound (remote ip \"localhost:{port}\") {})\n", tag("gate")));
        }
    }
    // Serving on this computer only (a local model's port, an agent's dev server).
    p.push_str("(allow network-bind (local ip \"localhost:*\"))\n");
    p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
    // No name lookups: they are a way to send data out through the system resolver.
    p.push_str(&format!("(deny mach-lookup (global-name \"com.apple.dnssd.service\") {})\n", tag("dns")));
    p.push_str(&format!("(deny mach-lookup (global-name \"com.apple.mDNSResponder\") {})\n", tag("dns")));
    p.push_str(&format!("(deny network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")) {})\n", tag("dns")));
    // Nothing under the real user's home, whatever the file permissions say.
    let home = real_home();
    if home.starts_with("/Users/") && !home.contains('"') {
        p.push_str(&format!("(deny file-read* file-write* (subpath \"{home}\"))\n"));
    }
    p
}

/// Environment for anything that runs protected. Starts from nothing: the real user's environment
/// (tokens, keys, paths) must not leak into a protected process.
pub fn fenced_env(app: &AppConfig) -> Vec<(String, String)> {
    let home = home_for(&app.os_user);
    let gate = format!("http://127.0.0.1:{}", app.proxy_port);
    let mut env: Vec<(String, String)> = vec![
        ("HOME".into(), home.clone()),
        ("USER".into(), app.os_user.clone()),
        ("LOGNAME".into(), app.os_user.clone()),
        ("SHELL".into(), "/bin/zsh".into()),
        ("PATH".into(), "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("TERM".into(), std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into())),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("HTTPS_PROXY".into(), gate.clone()),
        ("HTTP_PROXY".into(), gate.clone()),
        ("https_proxy".into(), gate.clone()),
        ("http_proxy".into(), gate.clone()),
        ("ALL_PROXY".into(), gate),
        ("NO_PROXY".into(), "localhost,127.0.0.1".into()),
        ("no_proxy".into(), "localhost,127.0.0.1".into()),
        ("SANDBOX_AI".into(), "1".into()),
    ];
    match app.id.as_str() {
        "ollama" => {
            env.push(("OLLAMA_HOST".into(), format!("127.0.0.1:{}", app.serves_port.unwrap_or(11434))));
            env.push(("OLLAMA_MODELS".into(), format!("{home}/models")));
        }
        "claude-code" => {
            env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
        }
        _ => {}
    }
    env
}

/// The full command line that runs `program` protected: as the app's OS user, under its seatbelt
/// profile, with a clean environment, in `cwd`.
pub fn fenced_argv(app: &AppConfig, program: &str, args: &[String], extra_env: &[(String, String)], cwd: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = vec!["/usr/bin/sudo".into(), "-n".into(), "-u".into(), app.os_user.clone(), "-H".into(), "/usr/bin/sandbox-exec".into(), "-p".into(), seatbelt_profile(app), "/usr/bin/env".into(), "-i".into()];
    for (k, val) in fenced_env(app).into_iter().chain(extra_env.iter().cloned()) {
        v.push(format!("{k}={val}"));
    }
    let home = home_for(&app.os_user);
    let dir = cwd.map(str::to_string).or_else(|| app.workspace.clone()).unwrap_or(home);
    v.extend(["/bin/sh".into(), "-c".into(), "cd \"$1\" && shift && exec \"$@\"".into(), "sh".into(), dir, program.to_string()]);
    v.extend(args.iter().cloned());
    v
}

async fn run(argv: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..]).current_dir("/").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output()).await.ok()?.ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

async fn run_owned(argv: Vec<String>, timeout: Duration) -> Option<(bool, String)> {
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    run(&refs, timeout).await
}

/// Check every layer for one app. Cheap enough to run every half minute.
/// The health check deliberately tries to go around the gate, to see that it is refused. Those
/// attempts are ours, not the app's, so they must not carry the tag that puts refusals in the Feed.
fn untagged(mut argv: Vec<String>) -> Vec<String> {
    for a in &mut argv {
        if a.contains(VIOLATION_TAG) {
            *a = a.replace(VIOLATION_TAG, "sai-check=");
        }
    }
    argv
}

pub async fn probe(app: &AppConfig, expected_rules: &str) -> Health {
    let t = Duration::from_secs(6);
    let mut h = Health { checked: true, ..Default::default() };
    h.installed = crate::catalog::find_binary(&app.binary).is_some();
    h.user_exists = run(&["/usr/bin/id", "-u", &app.os_user], t).await.map(|r| r.0).unwrap_or(false);
    if !h.user_exists {
        return h;
    }
    h.group_ok = run(&["/usr/bin/id", "-gn", &app.os_user], t).await.map(|(ok, out)| ok && out.trim() == GROUP).unwrap_or(false);
    h.can_run = run(&["/usr/bin/sudo", "-n", "-u", &app.os_user, "/usr/bin/true"], t).await.map(|r| r.0).unwrap_or(false);
    h.workspace_ok = match &app.workspace {
        Some(w) => std::path::Path::new(w).is_dir(),
        None => true,
    };
    if h.can_run {
        // Positive control: the profile compiles and a harmless command runs under it.
        let control = run_owned(fenced_argv(app, "/usr/bin/true", &[], &[], Some("/")), t).await.map(|r| r.0).unwrap_or(false);
        // The real test: a direct connection to the internet must be refused by the profile itself,
        // which reads the same online or offline.
        let direct = run_owned(untagged(fenced_argv(app, "/usr/bin/nc", &["-v".into(), "-z".into(), "-G".into(), "2".into(), "1.1.1.1".into(), "443".into()], &[], Some("/"))), t).await;
        let refused = matches!(&direct, Some((false, out)) if out.contains("not permitted"));
        // And a name lookup must fail.
        let lookup = run_owned(untagged(fenced_argv(app, "/usr/bin/dscacheutil", &["-q".into(), "host".into(), "-a".into(), "name".into(), "example.com".into()], &[], Some("/"))), t).await;
        let no_lookup = matches!(&lookup, Some((_, out)) if !out.contains("ip_address") && !out.contains("ipv6_address"));
        h.seatbelt_ok = control && refused && no_lookup;
    }
    h.filter_ok = filter_status_ok(&app.os_user);
    h.rules_current = std::fs::read_to_string(format!("/etc/pf.anchors/{FILTER_ANCHOR}")).map(|installed| installed.trim() == expected_rules.trim()).unwrap_or(false);
    h
}

/// The watchdog writes `ok <epoch> user1,user2` every 20 s.
fn filter_status_ok(os_user: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(FILTER_STATUS) else { return false };
    let mut parts = text.split_whitespace();
    let (Some(state), Some(epoch), Some(users)) = (parts.next(), parts.next(), parts.next()) else { return false };
    let fresh = epoch.parse::<i64>().map(|e| (chrono::Utc::now().timestamp() - e).abs() <= 90).unwrap_or(false);
    state == "ok" && fresh && users.split(',').any(|u| u == os_user)
}

// ---------------------------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------------------------

/// Start a protected background service (a local model). Output goes to a log file.
pub async fn start_managed(app: &AppConfig) -> Result<tokio::process::Child, String> {
    let binary = crate::catalog::find_binary(&app.binary).ok_or_else(|| format!("{} is not installed on this computer.", app.name))?;
    let args: Vec<String> = match app.id.as_str() {
        "ollama" => vec!["serve".into()],
        _ => vec![],
    };
    let argv = fenced_argv(app, &binary, &args, &[], Some(&home_for(&app.os_user)));
    let logs = crate::paths::logs_dir();
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{}.log", app.id))).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    Command::new(&argv[0]).args(&argv[1..]).current_dir("/").stdin(Stdio::null()).stdout(Stdio::from(log)).stderr(Stdio::from(err)).spawn().map_err(|e| format!("{} could not be started: {e}", app.name))
}

/// Stop everything running as this app's OS user. We cannot signal another user's processes
/// directly, so the signal is sent as that user.
pub async fn stop_all(app: &AppConfig) {
    let _ = run(&["/usr/bin/sudo", "-n", "-u", &app.os_user, "/bin/sh", "-c", "kill -TERM -1"], Duration::from_secs(5)).await;
}

pub async fn is_port_open(port: u16) -> bool {
    tokio::time::timeout(Duration::from_millis(400), tokio::net::TcpStream::connect(("127.0.0.1", port))).await.map(|r| r.is_ok()).unwrap_or(false)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The script a protected terminal runs: a short banner, then the agent, fenced.
pub fn terminal_script(app: &AppConfig, binary: &str, dir: Option<&str>) -> String {
    let argv = fenced_argv(app, binary, &[], &[], dir);
    let workspace = dir.map(str::to_string).or_else(|| app.workspace.clone()).unwrap_or_default();
    format!(
        "#!/bin/zsh\ncd /\nclear\nprintf '\\033[1m%s, protected by Sandbox-ai\\033[0m\\n' {name}\nprintf 'It can only reach places you trust, and can only write in:\\n  %s\\n\\n' {ws}\n{cmd}\nprintf '\\n\\033[2mProtected session ended. You can close this window.\\033[0m\\n'\n",
        name = sh_quote(&app.name),
        ws = sh_quote(&workspace),
        cmd = argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "),
    )
}

/// Open a Terminal window running the agent protected, in its workspace.
pub async fn open_terminal(app: &AppConfig, dir: Option<&str>) -> Result<(), String> {
    let binary = crate::catalog::find_binary(&app.binary).ok_or_else(|| format!("{} is not installed on this computer.", app.name))?;
    let run_dir = crate::paths::data_dir().join("run");
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
    let script = run_dir.join(format!("{}.command", app.id));
    std::fs::write(&script, terminal_script(app, &binary, dir)).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700));
    }
    let path = script.to_string_lossy().into_owned();
    match run(&["/usr/bin/open", "-a", "Terminal", &path], Duration::from_secs(10)).await {
        Some((true, _)) => Ok(()),
        Some((false, out)) => Err(format!("Terminal could not be opened: {}", out.trim())),
        None => Err("Terminal could not be opened.".into()),
    }
}

pub async fn reveal(path: &str, select: bool) {
    let args: Vec<&str> = if select { vec!["/usr/bin/open", "-R", path] } else { vec!["/usr/bin/open", path] };
    let _ = run(&args, Duration::from_secs(10)).await;
}

// ---------------------------------------------------------------------------------------------
// One-time setup (administrator approval)
// ---------------------------------------------------------------------------------------------

/// Packet filter rules for every protected app. Loopback is limited to the ports each app needs.
pub fn filter_rules(cfg: &Config) -> String {
    let mut s = String::from("# Sandbox-ai packet filter rules. Generated by the app; changes are overwritten.\n");
    for app in &cfg.apps {
        let u = &app.os_user;
        let mut out_ports: Vec<u16> = vec![app.proxy_port];
        out_ports.extend(&app.local_ports);
        let ports = out_ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
        s.push_str(&format!("\n# {}\n", app.name));
        s.push_str(&format!("pass out quick on lo0 proto tcp from any to any port {{ {ports} }} user {u}\n"));
        if app.loopback_all {
            let (lo, hi) = (crate::catalog::GATE_PORTS.start(), crate::catalog::GATE_PORTS.end());
            s.push_str(&format!("block return out quick on lo0 proto tcp from any to any port {lo}:{hi} user {u}\n"));
            s.push_str(&format!("pass out quick on lo0 proto tcp from any to any user {u}\n"));
        }
        // Loopback is judged twice, leaving and arriving. Arrivals at this user's own sockets are
        // fine: its service port, an agent's dev server, a model server's helper processes.
        s.push_str(&format!("pass in quick on lo0 proto tcp from any to any user {u}\n"));
        s.push_str(&format!("block return out quick user {u}\n"));
        s.push_str(&format!("block return in quick user {u}\n"));
    }
    s
}

const WATCHDOG: &str = r#"#!/bin/bash
# Sandbox-ai fence watchdog. Runs as root from launchd and keeps the packet filter rules loaded,
# including after restarts and after anything else reloads the system rules.
ANCHOR=sandboxai
RULES=/etc/pf.anchors/sandboxai
CONF="/Library/Application Support/SandboxAI/pf.conf"
STATUS=/var/run/sandboxai.fence
USERS="@USERS@"

ensure() {
  pfctl -s info 2>/dev/null | grep -q '^Status: Enabled' || pfctl -E >/dev/null 2>&1
  if ! pfctl -sr 2>/dev/null | grep -q "^anchor \"$ANCHOR\""; then
    { cat /etc/pf.conf; printf '\nanchor "%s"\nload anchor "%s" from "%s"\n' "$ANCHOR" "$ANCHOR" "$RULES"; } > "$CONF"
    pfctl -q -f "$CONF" 2>"$STATUS.err"
  fi
  want=$(grep -c '^block' "$RULES" 2>/dev/null); want=${want:-0}
  have=$(pfctl -a "$ANCHOR" -sr 2>/dev/null | grep -c '^block')
  if [ "$have" -lt "$want" ]; then
    pfctl -q -a "$ANCHOR" -f "$RULES" 2>>"$STATUS.err"
    have=$(pfctl -a "$ANCHOR" -sr 2>/dev/null | grep -c '^block')
  fi
  state=down
  if [ "$want" -gt 0 ] && [ "$have" -ge "$want" ] \
     && pfctl -s info 2>/dev/null | grep -q '^Status: Enabled' \
     && pfctl -sr 2>/dev/null | grep -q "^anchor \"$ANCHOR\""; then
    state=ok
  fi
  echo "$state $(date +%s) $USERS" > "$STATUS.tmp" && chmod 644 "$STATUS.tmp" && mv "$STATUS.tmp" "$STATUS"
}

if [ "${1:-}" = "--once" ]; then ensure; cat "$STATUS"; exit 0; fi
while true; do ensure; sleep 20; done
"#;

const DAEMON_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key><string>com.sandboxai.fence</string>
	<key>ProgramArguments</key>
	<array><string>/bin/bash</string><string>/Library/Application Support/SandboxAI/fence-watchdog.sh</string></array>
	<key>RunAtLoad</key><true/>
	<key>KeepAlive</key><true/>
</dict>
</plist>
"#;

/// The script the administrator approves. Idempotent: running it again repairs and updates.
pub fn setup_script(cfg: &Config, real_user: &str) -> String {
    let users: Vec<&str> = cfg.apps.iter().map(|a| a.os_user.as_str()).collect();
    let mut s = String::new();
    s.push_str("#!/bin/bash\n# Sandbox-ai one-time setup. Generated by the app. Safe to run again.\nset -Eeuo pipefail\n");
    s.push_str(&format!("REAL_USER={}\nGROUP={GROUP}\nSUPPORT={}\nWORKROOT={}\n", sh_quote(real_user), sh_quote(SYSTEM_DIR), sh_quote(WORKSPACE_ROOT)));
    s.push_str(r#"
free_id() { # first id in 590..599 then 500..589 used by neither a user nor a group
  local used; used="$( { dscl . -list /Users UniqueID; dscl . -list /Groups PrimaryGroupID; } | awk '{print $2}' | sort -n | uniq)"
  for i in $(seq 590 599) $(seq 500 589); do echo "$used" | grep -qx "$i" || { echo "$i"; return; }; done
  echo "no free id" >&2; exit 1
}

STEP="starting"
trap 'echo "Setup stopped at: $STEP (while running: $BASH_COMMAND)" >&2' ERR

STEP="system folders"
mkdir -p "$SUPPORT" "$WORKROOT"
chown root:wheel "$SUPPORT" "$WORKROOT"; chmod 755 "$SUPPORT" "$WORKROOT"

# 1. A group of its own, so protected apps are not members of "staff" and cannot look into home folders.
STEP="group for protected apps"
if ! dscl . -read "/Groups/$GROUP" >/dev/null 2>&1; then
  gid="$(free_id)"
  dscl . -create "/Groups/$GROUP"
  dscl . -create "/Groups/$GROUP" PrimaryGroupID "$gid"
  dscl . -create "/Groups/$GROUP" RealName "Sandbox-ai protected apps"
  dscl . -create "/Groups/$GROUP" Password '*'
fi
GID="$(dscl . -read "/Groups/$GROUP" PrimaryGroupID | awk '{print $2}')"

set_attr() { # user attribute value: write only when it differs, so a correct record is never touched
  local cur; cur="$(dscl . -read "/Users/$1" "$2" 2>/dev/null | awk 'NR==1{print $NF}')"
  [ "$cur" = "$3" ] || dscl . -create "/Users/$1" "$2" "$3"
}

ensure_user() { # name home
  local u="$1" home="$2"
  STEP="private space for $u"
  if ! dscl . -read "/Users/$u" >/dev/null 2>&1; then
    local uid; uid="$(free_id)"
    dscl . -create "/Users/$u"
    dscl . -create "/Users/$u" UniqueID "$uid"
    dscl . -create "/Users/$u" PrimaryGroupID "$GID"
    dscl . -create "/Users/$u" RealName "Sandbox-ai protected app"
    dscl . -create "/Users/$u" UserShell /bin/zsh
    dscl . -create "/Users/$u" IsHidden 1
    dscl . -passwd "/Users/$u" "$(head -c 32 /dev/urandom | base64)"
    # macOS guards the home-folder attribute separately and may refuse it. Nothing depends on it:
    # the app always tells a protected process where its home is.
    dscl . -create "/Users/$u" NFSHomeDirectory "$home" 2>/dev/null || true
  fi
  set_attr "$u" PrimaryGroupID "$GID"
  # Only the top of the home folder is changed. macOS protects some folders inside it (Mail, app
  # containers) even from root, and nothing inside is reachable once the top is private.
  mkdir -p "$home"
  chown "$u:$GROUP" "$home"
  chmod 700 "$home"
}

ensure_workspace() { # dir owner
  local d="$1" u="$2"
  STEP="workspace folder $d"
  local rights="read,write,execute,append,delete,delete_child,readattr,writeattr,readextattr,writeextattr,readsecurity,file_inherit,directory_inherit"
  mkdir -p "$d"
  chown "$u:$GROUP" "$d"; chmod 770 "$d"
  chmod -N "$d" 2>/dev/null || true
  chmod +a "user:$REAL_USER allow $rights" "$d"
  chmod +a "user:$u allow $rights" "$d"
  [ -f "$d/README.txt" ] || { printf 'This folder is the only place the protected app can write.\nPut the files you want it to work on here.\n' > "$d/README.txt"; chown "$u:$GROUP" "$d/README.txt"; chmod 660 "$d/README.txt"; }
}

"#);
    for app in &cfg.apps {
        s.push_str(&format!("ensure_user {} {}\n", sh_quote(&app.os_user), sh_quote(&home_for(&app.os_user))));
        if let Some(w) = &app.workspace {
            s.push_str(&format!("ensure_workspace {} {}\n", sh_quote(w), sh_quote(&app.os_user)));
        }
    }
    s.push_str("\n# 2. Packet filter rules, and a watchdog that keeps them loaded across restarts.\nSTEP=\"packet filter rules\"\nmkdir -p /etc/pf.anchors\n");
    s.push_str(&format!("cat > /etc/pf.anchors/{FILTER_ANCHOR} <<'SAI_EOF'\n{}SAI_EOF\n", filter_rules(cfg)));
    s.push_str(&format!("cat > \"$SUPPORT/fence-watchdog.sh\" <<'SAI_EOF'\n{}SAI_EOF\n", WATCHDOG.replace("@USERS@", &users.join(","))));
    s.push_str("chown root:wheel \"$SUPPORT/fence-watchdog.sh\" /etc/pf.anchors/sandboxai; chmod 755 \"$SUPPORT/fence-watchdog.sh\"; chmod 644 /etc/pf.anchors/sandboxai\n");
    s.push_str(&format!("cat > /Library/LaunchDaemons/com.sandboxai.fence.plist <<'SAI_EOF'\n{DAEMON_PLIST}SAI_EOF\n"));
    s.push_str(r#"chown root:wheel /Library/LaunchDaemons/com.sandboxai.fence.plist; chmod 644 /Library/LaunchDaemons/com.sandboxai.fence.plist
# Reload the rules now (the anchor may already hold an older set).
STEP="starting the watchdog"
pfctl -q -a sandboxai -F rules 2>/dev/null || true
launchctl bootout system/com.sandboxai.fence 2>/dev/null || true
sleep 1
launchctl enable system/com.sandboxai.fence 2>/dev/null || true
launchctl bootstrap system /Library/LaunchDaemons/com.sandboxai.fence.plist 2>/dev/null || launchctl kickstart -k system/com.sandboxai.fence

# 3. Let the app start things as the protected users without asking for a password each time.
STEP="permission to start protected apps"
"#);
    s.push_str(&format!("printf '%s ALL=({}) NOPASSWD: ALL\\n' \"$REAL_USER\" > /etc/sudoers.d/sandbox-ai.tmp\n", users.join(",")));
    s.push_str(r#"chmod 440 /etc/sudoers.d/sandbox-ai.tmp
if visudo -cf /etc/sudoers.d/sandbox-ai.tmp >/dev/null; then mv /etc/sudoers.d/sandbox-ai.tmp /etc/sudoers.d/sandbox-ai; else rm -f /etc/sudoers.d/sandbox-ai.tmp; echo "could not write the permission file" >&2; exit 1; fi

# 4. Leftovers from earlier experiments on this computer.
rm -f /etc/sudoers.d/sandbox-ai-agent /etc/pf.anchors/com.sandboxai.fence /etc/pf.anchors/com.sandboxai.agent /etc/pf.anchors/com.sandboxai.main.conf

# 5. Confirm.
STEP="confirming the rules are loaded"
state="$(/bin/bash "$SUPPORT/fence-watchdog.sh" --once)"
case "$state" in
  ok*) echo "$state" ;;
  *) echo "The packet filter rules did not load: $(tail -3 /var/run/sandboxai.fence.err 2>/dev/null | tr '\n' ' ')" >&2; false ;;
esac
"#);
    s
}

pub fn remove_script(cfg: &Config) -> String {
    let mut s = String::from("#!/bin/bash\n# Sandbox-ai: remove everything setup created. Workspace folders are kept.\nset -uo pipefail\n");
    s.push_str(r#"launchctl bootout system/com.sandboxai.fence 2>/dev/null || true
rm -f /Library/LaunchDaemons/com.sandboxai.fence.plist /etc/sudoers.d/sandbox-ai /etc/sudoers.d/sandbox-ai-agent
pfctl -q -a sandboxai -F all 2>/dev/null || true
pfctl -q -f /etc/pf.conf 2>/dev/null || true
rm -f /etc/pf.anchors/sandboxai /var/run/sandboxai.fence
rm -rf "/Library/Application Support/SandboxAI"
"#);
    for app in &cfg.apps {
        let u = sh_quote(&app.os_user);
        s.push_str(&format!("pkill -u {u} 2>/dev/null || true\ndscl . -delete /Users/{} 2>/dev/null || true\nrm -rf {}\n", app.os_user, sh_quote(&home_for(&app.os_user))));
    }
    s.push_str(&format!("dscl . -delete /Groups/{GROUP} 2>/dev/null || true\necho removed\n"));
    s
}

/// Run a script as root after the OS's own administrator prompt. We never see the password.
async fn run_as_admin(script: &str, name: &str, prompt: &str) -> SetupResult {
    let dir = match crate::paths::ensure_data_dir() {
        Ok(d) => d.join("run"),
        Err(e) => return SetupResult { ok: false, message: format!("Could not prepare setup: {e}") },
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    if let Err(e) = std::fs::write(&path, script) {
        return SetupResult { ok: false, message: format!("Could not prepare setup: {e}") };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let apple = format!("do shell script \"/bin/bash \" & quoted form of \"{}\" & \" 2>&1\" with prompt \"{}\" with administrator privileges", esc(&path.to_string_lossy()), esc(prompt));
    let out = run(&["/usr/bin/osascript", "-e", &apple], Duration::from_secs(300)).await;
    let _ = std::fs::remove_file(&path);
    match out {
        Some((true, _)) => SetupResult { ok: true, message: "Done.".into() },
        Some((false, text)) if text.contains("-128") || text.to_lowercase().contains("cancel") => SetupResult { ok: false, message: "You cancelled the approval, so nothing was changed.".into() },
        Some((false, text)) => {
            // osascript folds the script's output into one line; keep only our own closing sentence.
            let flat = text.replace('\n', " ");
            let line = match flat.find("Setup stopped at:") {
                Some(i) => flat[i..].trim().trim_end_matches(|c: char| c == ')' || c == '(' || c.is_ascii_digit() || c == ' ').to_string() + ")",
                None => flat.rsplit("execution error: ").next().unwrap_or(&flat).trim().chars().take(300).collect(),
            };
            SetupResult { ok: false, message: format!("Setup did not finish. {line}") }
        }
        None => SetupResult { ok: false, message: "Setup timed out waiting for approval.".into() },
    }
}

pub async fn run_setup(cfg: &Config) -> SetupResult {
    run_as_admin(&setup_script(cfg, &real_user()), "setup.sh", "Sandbox-ai needs your approval to turn on protection.").await
}

pub async fn remove_setup(cfg: &Config) -> SetupResult {
    run_as_admin(&remove_script(cfg), "remove.sh", "Sandbox-ai needs your approval to remove protection from this computer.").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_rules_block_every_protected_user_and_limit_loopback() {
        let cfg = Config::fresh();
        let rules = filter_rules(&cfg);
        for app in &cfg.apps {
            assert!(rules.contains(&format!("block return out quick user {}", app.os_user)));
            assert!(rules.contains(&format!("block return in quick user {}", app.os_user)));
            assert!(rules.contains(&app.proxy_port.to_string()));
        }
        assert!(!rules.contains("pass out quick on lo0 user"), "loopback must be limited to named ports");
        for app in &cfg.apps {
            assert!(rules.contains(&format!("pass in quick on lo0 proto tcp from any to any user {}", app.os_user)), "arrivals at the user's own loopback sockets must pass");
        }
    }

    #[test]
    fn refusals_are_read_from_the_system_log() {
        let v = parse_violation("Sandbox: curl(40242) deny(1) network-outbound remote:*:443\nsai-app=claude-code;net").unwrap();
        assert_eq!(v, Violation { app: "claude-code".into(), process: "curl".into(), what: "direct", port: 443, count: 1 });
        let v = parse_violation("Sandbox: node(7) deny(1) network-outbound /private/var/run/mDNSResponder\nsai-app=claude-code;dns").unwrap();
        assert_eq!((v.what, v.port), ("lookup", 0));
        let v = parse_violation("Sandbox: x(7) deny(1) mach-lookup com.apple.dnssd.service\nsai-app=ollama;dns").unwrap();
        assert_eq!((v.app.as_str(), v.what), ("ollama", "lookup"));
        let v = parse_violation("Sandbox: python3.12(9) deny(1) network-bind local:*:8080\nsai-app=claude-code;net").unwrap();
        assert_eq!((v.process.as_str(), v.what, v.port), ("python3.12", "listen", 0));
        let v = parse_violation("Sandbox: ollama(9) deny(1) network-outbound remote:*:47110\nsai-app=ollama;gate").unwrap();
        assert_eq!((v.what, v.port), ("gate", 47110));
        let v = parse_violation("4 duplicate reports for Sandbox: curl(1) deny(1) network-outbound remote:*:80\nsai-app=claude-code;net").unwrap();
        assert_eq!((v.count, v.port), (4, 80));
        let v = parse_violation("1 duplicate report for Sandbox: curl(1) deny(1) network-outbound remote:*:80\nsai-app=claude-code;net").unwrap();
        assert_eq!(v.count, 1);
    }

    #[test]
    fn refusals_without_our_tag_or_with_odd_names_are_handled() {
        assert!(parse_violation("Sandbox: curl(1) deny(1) network-outbound remote:*:443").is_none());
        assert!(parse_violation("Sandbox: curl(1) deny(1) network-outbound remote:*:443\nsai-app=../x;net").is_none());
        assert!(parse_violation("sai-app=claude-code;net").is_none());
        // A program chooses its own name; nothing but plain characters survives.
        let v = parse_violation("Sandbox: <b>evil\u{1b}[31m(1) deny(1) network-outbound remote:*:443\nsai-app=claude-code;net").unwrap();
        assert_eq!(v.process, "bevil31m");
        let long = format!("Sandbox: {}(1) deny(1) network-outbound remote:*:443\nsai-app=claude-code;net", "a".repeat(200));
        assert_eq!(parse_violation(&long).unwrap().process.len(), 32);
    }

    #[test]
    fn only_kernel_records_count() {
        let msg = "Sandbox: curl(1) deny(1) network-outbound remote:*:443\nsai-app=claude-code;net";
        let kernel = serde_json::json!({ "processImagePath": "/kernel", "processID": 0, "eventMessage": msg }).to_string();
        assert!(violation_in(&kernel).is_some());
        let forged = serde_json::json!({ "processImagePath": "/usr/bin/logger", "processID": 500, "eventMessage": msg }).to_string();
        assert!(violation_in(&forged).is_none());
        assert!(violation_in("Filtering the log data using …").is_none());
    }

    #[test]
    fn seatbelt_profile_denies_network_then_allows_only_the_gate() {
        let cfg = Config::fresh();
        let app = cfg.app("claude-code").unwrap();
        let p = seatbelt_profile(app);
        let deny = p.find("(deny network* ").unwrap();
        let allow = p.find(&format!("localhost:{}", app.proxy_port)).unwrap();
        assert!(deny < allow, "the allow must come after the deny so it wins");
        assert!(p.contains("com.apple.dnssd.service"));
        assert!(!p.contains("(allow network*)"));
    }

    #[test]
    fn a_model_server_may_use_loopback_but_never_another_gate() {
        let cfg = Config::fresh();
        let (ollama, claude) = (cfg.app("ollama").unwrap(), cfg.app("claude-code").unwrap());
        let p = seatbelt_profile(ollama);
        let allow_all = p.find("(allow network-outbound (remote ip \"localhost:*\"))").expect("loopback allowance");
        let deny_other = p.find(&format!("(deny network-outbound (remote ip \"localhost:{}\") ", claude.proxy_port)).expect("other gate denied");
        assert!(allow_all < deny_other, "the deny must come after the allow so it wins");
        assert!(!p.contains(&format!("(deny network-outbound (remote ip \"localhost:{}\")", ollama.proxy_port)), "its own gate stays reachable");
        // An agent gets no such allowance.
        assert!(!seatbelt_profile(claude).contains("localhost:*\"))\n(deny network-outbound (remote ip"));
        assert!(!seatbelt_profile(claude).contains("(allow network-outbound (remote ip \"localhost:*\"))"));
        let rules = filter_rules(&cfg);
        let block = rules.find(&format!("port 47110:47119 user {}", ollama.os_user)).expect("gate range blocked for the model server");
        let pass_all = rules.find(&format!("pass out quick on lo0 proto tcp from any to any user {}", ollama.os_user)).expect("loopback pass");
        assert!(block < pass_all, "the gate-range block must be evaluated first");
        assert!(!rules.contains(&format!("pass out quick on lo0 proto tcp from any to any user {}", claude.os_user)));
    }

    #[test]
    fn fenced_env_does_not_inherit_the_real_environment() {
        std::env::set_var("SAI_TEST_SECRET", "hunter2");
        let cfg = Config::fresh();
        let app = cfg.app("claude-code").unwrap();
        let argv = fenced_argv(app, "/usr/bin/true", &[], &[], None);
        assert!(argv.iter().any(|a| a == "-i"), "env must start empty");
        assert!(!argv.iter().any(|a| a.contains("hunter2")));
        assert!(argv.iter().any(|a| a == &format!("HTTPS_PROXY=http://127.0.0.1:{}", app.proxy_port)));
    }

    #[test]
    fn terminal_script_is_valid_zsh_and_runs_the_agent_fenced() {
        let cfg = Config::fresh();
        let app = cfg.app("claude-code").unwrap();
        let script = terminal_script(app, "/opt/homebrew/bin/claude", None);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.command");
        std::fs::write(&p, &script).unwrap();
        assert!(std::process::Command::new("/bin/zsh").arg("-n").arg(&p).status().unwrap().success(), "syntax error in:\n{script}");
        assert!(script.contains("/usr/bin/sandbox-exec"));
        assert!(script.contains("-u' '_saiagent'") || script.contains("'-u' '_saiagent'"));
        assert!(script.contains("(deny network*"));
    }

    #[test]
    fn setup_script_is_valid_bash() {
        let cfg = Config::fresh();
        for (name, script) in [("setup", setup_script(&cfg, "tester")), ("remove", remove_script(&cfg))] {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join(format!("{name}.sh"));
            std::fs::write(&p, script).unwrap();
            let ok = std::process::Command::new("/bin/bash").arg("-n").arg(&p).status().unwrap().success();
            assert!(ok, "{name} script has a syntax error");
        }
    }
}
