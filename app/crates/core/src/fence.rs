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

/// The profile for a windowed app, which runs as you. One layer instead of three: there is no
/// private user and no packet filter behind it, so this profile is the whole fence.
///
/// - Network: this computer only (its own helpers, a local model), never another app's gate.
///   Its web traffic is pointed at its own gate, where the usual rules and questions apply.
/// - The ways a program running as you could step outside the profile are closed: handing a link
///   or file to another app, driving another app, planting something that starts at login, and
///   reaching Sandbox-ai's own settings, key and control socket.
///
/// It can still read and change your files, as it can when you open it normally.
fn window_profile(app: &AppConfig) -> String {
    let tag = |what: &str| format!("(with message \"{VIOLATION_TAG}{};{what}\")", app.id);
    let mut p = String::from("(version 1)\n(allow default)\n");
    p.push_str(&format!("(deny network* {})\n", tag("net")));
    p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
    for port in crate::catalog::GATE_PORTS.filter(|g| *g != app.proxy_port) {
        p.push_str(&format!("(deny network-outbound (remote ip \"localhost:{port}\") {})\n", tag("gate")));
    }
    p.push_str("(allow network-bind (local ip \"localhost:*\"))\n");
    p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
    // Windowed apps talk to their own helper processes over local sockets.
    p.push_str("(allow network-outbound (remote unix-socket))\n(allow network-bind (local unix-socket))\n(allow network-inbound (local unix-socket))\n");
    p.push_str(&format!("(deny mach-lookup (global-name \"com.apple.dnssd.service\") {})\n", tag("dns")));
    p.push_str(&format!("(deny mach-lookup (global-name \"com.apple.mDNSResponder\") {})\n", tag("dns")));
    p.push_str(&format!("(deny network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")) {})\n", tag("dns")));
    p.push_str(&format!("(deny lsopen {})\n", tag("open")));
    p.push_str(&format!("(deny appleevent-send {})\n", tag("event")));
    let safe = |path: &str| !path.is_empty() && !path.contains('"') && !path.contains('\\');
    let data = crate::paths::data_dir().to_string_lossy().into_owned();
    if safe(&data) {
        p.push_str(&format!("(deny file* (subpath \"{data}\"))\n"));
        p.push_str(&format!("(deny network-outbound (remote unix-socket (subpath \"{data}\")))\n"));
    }
    let home = real_home();
    if home.starts_with("/Users/") && safe(&home) {
        p.push_str(&format!("(deny file-write* (subpath \"{home}/Library/LaunchAgents\"))\n"));
    }
    p
}

/// What we need to know about a windowed app before wrapping it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    /// The program inside the .app that is actually started.
    pub exe: String,
    /// Built with Electron. Its own inner sandbox cannot start inside ours, so it is switched
    /// off, and its web traffic can be pointed at the gate.
    pub electron: bool,
}

/// Look inside a .app and say whether it can be protected. The error is a sentence for the user.
pub fn inspect_bundle(path: &str) -> Result<Bundle, String> {
    let app = std::path::Path::new(path);
    let name = app.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "This app".into());
    if !path.starts_with('/') || !path.ends_with(".app") || !app.is_dir() || path.contains('"') || path.contains('\\') || path.chars().any(|c| c.is_control()) {
        return Err("Choose an app, for example from your Applications folder.".into());
    }
    let plist = app.join("Contents/Info.plist");
    let exe_name = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleExecutable", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .ok_or_else(|| format!("{name} does not look like an app that can be started."))?;
    let exe = app.join("Contents/MacOS").join(&exe_name);
    if !exe.is_file() {
        return Err(format!("{name} does not look like an app that can be started."));
    }
    // An app that already lives in Apple's App Sandbox applies it at launch, and macOS refuses
    // a second sandbox on top of ours: the app would quit at once.
    let entitlements = std::process::Command::new("/usr/bin/codesign").args(["-d", "--entitlements", "-", "--xml"]).arg(app).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if app_sandboxed(&entitlements) {
        return Err(format!("{name} already runs in Apple's own sandbox, and macOS does not allow a second one around it. Apps like this need the Network Extension version of Sandbox-ai."));
    }
    Ok(Bundle { exe: exe.to_string_lossy().into_owned(), electron: app.join("Contents/Frameworks/Electron Framework.framework").exists() })
}

/// Whether a code-signing entitlements document switches the App Sandbox on.
fn app_sandboxed(entitlements_xml: &str) -> bool {
    let Some(at) = entitlements_xml.find("com.apple.security.app-sandbox") else { return false };
    let rest = &entitlements_xml[at..];
    let after_key = rest.find("</key>").map(|i| &rest[i + 6..]).unwrap_or("");
    after_key.trim_start().starts_with("<true")
}

/// The command line that opens a windowed app protected: as you, under its profile, with its
/// web traffic pointed at its gate.
pub fn window_argv(app: &AppConfig) -> Result<Vec<String>, String> {
    let bundle = inspect_bundle(&app.binary)?;
    let gate = format!("http://127.0.0.1:{}", app.proxy_port);
    let mut v: Vec<String> = vec!["/usr/bin/sandbox-exec".into(), "-p".into(), seatbelt_profile(app), "/usr/bin/env".into()];
    for k in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy", "ALL_PROXY"] {
        v.push(format!("{k}={gate}"));
    }
    // Parts of these apps are plain Node programs, which ignore the app's own gate setting
    // unless told to read it from the environment.
    v.extend(["NO_PROXY=localhost,127.0.0.1".to_string(), "no_proxy=localhost,127.0.0.1".to_string(), "NODE_USE_ENV_PROXY=1".to_string(), "SANDBOX_AI=1".to_string(), bundle.exe]);
    if bundle.electron {
        v.extend(["--no-sandbox".to_string(), format!("--proxy-server={gate}"), "--proxy-bypass-list=<-loopback>;127.0.0.1;localhost".to_string()]);
    }
    Ok(v)
}

/// Whether the app is already open (protected or not). Windowed apps usually keep to one copy:
/// a second start just hands over to the first, which may be an unprotected one.
pub async fn window_running(app: &AppConfig) -> bool {
    let Ok(bundle) = inspect_bundle(&app.binary) else { return false };
    run(&["/usr/bin/pgrep", "-f", &format!("^{}", regex_escape(&bundle.exe))], Duration::from_secs(4)).await.map(|r| r.0).unwrap_or(false)
}

fn regex_escape(s: &str) -> String {
    s.chars().flat_map(|c| if c.is_ascii_alphanumeric() || c == '/' || c == ' ' || c == '-' || c == '_' { vec![c] } else { vec!['\\', c] }).collect()
}

/// Written by setup after it has tested, on this computer, that an agent can reach a server it
/// started itself and is refused at anyone else's. "own" when that held.
pub const LOCAL_SERVERS_FILE: &str = "/Library/Application Support/SandboxAI/local-servers";

/// Whether agents are allowed to reach servers they started themselves on this computer.
pub fn own_servers() -> bool {
    std::fs::read_to_string(LOCAL_SERVERS_FILE).map(|t| t.trim() == "own").unwrap_or(false)
}

/// The packet filter anchor holding the ports this agent is serving on right now. The watchdog
/// keeps it current (see `WATCHDOG`).
#[cfg(test)]
fn own_anchor(os_user: &str) -> String {
    format!("own_{}", os_user.trim_start_matches("_sai"))
}

/// An app that should reach its own local servers and nobody else's: every command-line agent.
fn wants_own_servers(app: &AppConfig) -> bool {
    !app.windowed && !app.loopback_all
}

/// The certificate authorities this computer trusts out of the box, as text. Programs that take
/// one file of trusted authorities get these plus ours (see `inspect::publish`).
pub fn system_roots_pem() -> String {
    std::process::Command::new("/usr/bin/security")
        .args(["find-certificate", "-a", "-p", "/System/Library/Keychains/SystemRootCertificates.keychain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Where the desktop app would live once installed.
pub const INSTALL_PATH: &str = "/Applications/Sandbox-ai.app";

/// The .app this executable is part of, if it is part of one.
pub fn bundle_of(exe: &str) -> Option<String> {
    std::path::Path::new(exe).ancestors().find(|a| a.extension().map(|e| e == "app").unwrap_or(false)).map(|a| a.to_string_lossy().into_owned())
}

/// Copy the app into the Applications folder, replacing an older copy. The copy we are running
/// from is left where it is. Returns the new location.
pub async fn install_to_applications(bundle: &str) -> Result<String, String> {
    if bundle == INSTALL_PATH {
        return Ok(INSTALL_PATH.to_string());
    }
    if !std::path::Path::new(bundle).join("Contents/MacOS").is_dir() {
        return Err("Sandbox-ai could not find its own app to copy.".into());
    }
    // Copy next to the destination first, then swap, so a failed copy never leaves half an app.
    let staging = format!("{INSTALL_PATH}.installing");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    match run(&["/usr/bin/ditto", bundle, &staging], Duration::from_secs(120)).await {
        Some((true, _)) => {}
        Some((false, out)) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(format!("Sandbox-ai could not be copied into Applications: {}", out.trim()));
        }
        None => return Err("Copying Sandbox-ai into Applications took too long.".into()),
    }
    let old = format!("{INSTALL_PATH}.old");
    let _ = tokio::fs::remove_dir_all(&old).await;
    let had = tokio::fs::rename(INSTALL_PATH, &old).await.is_ok();
    if let Err(e) = tokio::fs::rename(&staging, INSTALL_PATH).await {
        if had {
            let _ = tokio::fs::rename(&old, INSTALL_PATH).await;
        }
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(format!("Sandbox-ai could not be put into Applications: {e}"));
    }
    let _ = tokio::fs::remove_dir_all(&old).await;
    Ok(INSTALL_PATH.to_string())
}

/// Known windowed AI apps installed in the usual places and not on the list yet.
pub fn discover(cfg: &Config) -> Vec<crate::model::FoundApp> {
    let mut dirs = vec![std::path::PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else { continue };
            if ext != "app" || !crate::catalog::is_known_windowed(stem) {
                continue;
            }
            let path_s = path.to_string_lossy().into_owned();
            if cfg.apps.iter().any(|a| a.binary == path_s) || found.iter().any(|f: &crate::model::FoundApp| f.name.eq_ignore_ascii_case(stem)) {
                continue;
            }
            found.push(match inspect_bundle(&path_s) {
                Ok(b) if b.electron => crate::model::FoundApp { name: stem.to_string(), path: path_s, can_protect: true, asks: true, note: "Everything it sends will go through the gate, so you are asked.".into() },
                Ok(_) => crate::model::FoundApp { name: stem.to_string(), path: path_s, can_protect: true, asks: false, note: "Can only be run cut off from the internet: this kind of app cannot be pointed at the gate yet.".into() },
                Err(why) => crate::model::FoundApp { name: stem.to_string(), path: path_s, can_protect: false, asks: false, note: why },
            });
        }
    }
    found.sort_by(|a, b| b.can_protect.cmp(&a.can_protect).then(b.asks.cmp(&a.asks)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    found
}

/// Where an app's launcher lives: a small app of its own that you can keep in the Dock or find
/// with Spotlight, so that opening the protected copy is as easy as opening the ordinary one.
pub fn launcher_path(app: &AppConfig) -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join("Applications").join("Sandbox-ai").join(format!("{} (protected).app", app.name)))
}

/// Write the launcher. `opener` is the program that knows how to ask the running Sandbox-ai to
/// open an app (this app's own executable); the launcher runs `opener --open-protected <id>`.
pub fn write_launcher(app: &AppConfig, opener: &str) -> Result<String, String> {
    let path = launcher_path(app).ok_or("Your home folder could not be found.")?;
    let fail = |e: std::io::Error| format!("The launcher could not be created: {e}");
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(path.join("Contents/MacOS")).map_err(fail)?;
    std::fs::create_dir_all(path.join("Contents/Resources")).map_err(fail)?;
    // Borrow the original app's icon, so the launcher is recognisable in the Dock.
    let mut icon_line = String::new();
    if app.windowed {
        let bundle = std::path::Path::new(&app.binary);
        let icon = std::process::Command::new("/usr/bin/plutil").args(["-extract", "CFBundleIconFile", "raw", "-o", "-"]).arg(bundle.join("Contents/Info.plist")).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|n| !n.is_empty() && !n.contains('/'));
        if let Some(name) = icon {
            let file = if name.ends_with(".icns") { name } else { format!("{name}.icns") };
            if std::fs::copy(bundle.join("Contents/Resources").join(&file), path.join("Contents/Resources/icon.icns")).is_ok() {
                icon_line = "  <key>CFBundleIconFile</key><string>icon</string>\n".into();
            }
        }
    }
    let xml = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n  <key>CFBundleName</key><string>{name} (protected)</string>\n  <key>CFBundleDisplayName</key><string>{name} (protected)</string>\n  <key>CFBundleIdentifier</key><string>com.sandboxai.launcher.{id}</string>\n  <key>CFBundleExecutable</key><string>launcher</string>\n  <key>CFBundlePackageType</key><string>APPL</string>\n  <key>CFBundleVersion</key><string>1</string>\n  <key>LSUIElement</key><true/>\n{icon_line}</dict></plist>\n",
        name = xml(&app.name),
        id = xml(&app.id),
    );
    std::fs::write(path.join("Contents/Info.plist"), plist).map_err(fail)?;
    // When Sandbox-ai is not running yet it is started the ordinary way, as its own app, and
    // told what to open. When it is running, the request is handed over directly.
    let bundle = std::path::Path::new(opener).ancestors().find(|a| a.extension().map(|e| e == "app").unwrap_or(false)).map(|a| a.to_string_lossy().into_owned());
    let start = match &bundle {
        Some(b) => format!("exec /usr/bin/open -g -a {} --args --open-protected {}", sh_quote(b), sh_quote(&app.id)),
        None => format!("exec \"$OPENER\" --open-protected {}", sh_quote(&app.id)),
    };
    let script = format!(
        "#!/bin/sh\n# Opens {name} protected by Sandbox-ai. Created by Sandbox-ai; safe to delete.\nOPENER={opener}\nif [ -x \"$OPENER\" ]; then\n  if /usr/bin/pgrep -qf \"^$OPENER\"; then exec \"$OPENER\" --open-protected {id}; fi\n  {start}\nfi\n/usr/bin/osascript -e 'display alert \"Sandbox-ai was not found\" message \"Open Sandbox-ai and use Show launcher again to refresh this shortcut.\"' >/dev/null 2>&1\n",
        name = app.name,
        opener = sh_quote(opener),
        id = sh_quote(&app.id),
    );
    let exe = path.join("Contents/MacOS/launcher");
    std::fs::write(&exe, script).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(fail)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

pub fn remove_launcher(app: &AppConfig) {
    if let Some(p) = launcher_path(app) {
        let _ = std::fs::remove_dir_all(&p);
        // The folder goes too once the last launcher has gone (this fails, harmlessly, otherwise).
        if let Some(dir) = p.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

const VIOLATION_TAG: &str = "sai-app=";

/// One refused attempt to reach the network without going through the gate.
#[derive(Debug, PartialEq, Clone)]
pub struct Violation {
    pub app: String,
    /// Name of the program that tried, as the system reports it.
    pub process: String,
    /// "direct" (straight to an address), "lookup" (asked the system to resolve a name),
    /// "listen" (accept connections from other computers), "gate" (another app's gate),
    /// "open" (hand something to another app), "control" (drive another app), "other".
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
        ("open", _) => "open",
        ("event", _) => "control",
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
    seatbelt_profile_for(app, own_servers())
}

/// `own_servers`: agents may connect to any port on this computer except other apps' gates.
/// The profile cannot tell whose server a port belongs to; the packet filter does that part
/// (see [`filter_rules`] and the watchdog), which is why this is only switched on after setup
/// has tested it.
pub fn seatbelt_profile_for(app: &AppConfig, own_servers: bool) -> String {
    if app.windowed {
        return window_profile(app);
    }
    let tag = |what: &str| format!("(with message \"{VIOLATION_TAG}{};{what}\")", app.id);
    let mut p = String::from("(version 1)\n(allow default)\n");
    // Network: nothing, then exactly the local gate and the listed local services.
    p.push_str(&format!("(deny network* {})\n", tag("net")));
    p.push_str(&format!("(allow network-outbound (remote ip \"localhost:{}\"))\n", app.proxy_port));
    for port in &app.local_ports {
        p.push_str(&format!("(allow network-outbound (remote ip \"localhost:{port}\"))\n"));
    }
    if app.loopback_all || (own_servers && wants_own_servers(app)) {
        // Model servers talk to their own helper processes over arbitrary local ports, and an
        // agent to the servers it starts. Any local port, then, except another app's gate.
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
        // Node programs read the gate from the environment only when asked to.
        ("NODE_USE_ENV_PROXY".into(), "1".into()),
        ("SANDBOX_AI".into(), "1".into()),
    ];
    if app.inspect && !app.windowed && app.kind != "model" {
        // Tell the app about the local certificate, or it would refuse every inspected connection.
        env.extend(crate::inspect::env());
    }
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
    if app.windowed {
        // As you, under the windowed app's profile, with the same pointers to its gate.
        let gate = format!("http://127.0.0.1:{}", app.proxy_port);
        let mut v: Vec<String> = vec!["/usr/bin/sandbox-exec".into(), "-p".into(), seatbelt_profile(app), "/usr/bin/env".into()];
        for k in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy", "ALL_PROXY"] {
            v.push(format!("{k}={gate}"));
        }
        v.extend(["NO_PROXY=localhost,127.0.0.1".to_string(), "no_proxy=localhost,127.0.0.1".to_string(), "SANDBOX_AI=1".to_string()]);
        v.extend(extra_env.iter().map(|(k, val)| format!("{k}={val}")));
        v.extend(["/bin/sh".into(), "-c".into(), "cd \"$1\" && shift && exec \"$@\"".into(), "sh".into(), cwd.unwrap_or("/").to_string(), program.to_string()]);
        v.extend(args.iter().cloned());
        return v;
    }
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
    if app.windowed {
        // One layer, so one thing to check: the profile compiles, runs a harmless command, and
        // refuses a direct connection and a name lookup. The other layers do not apply.
        let wrap = |program: &str, args: &[&str]| {
            let mut v: Vec<String> = vec!["/usr/bin/sandbox-exec".into(), "-p".into(), seatbelt_profile(app), program.into()];
            v.extend(args.iter().map(|a| a.to_string()));
            v
        };
        let control = run_owned(wrap("/usr/bin/true", &[]), t).await.map(|r| r.0).unwrap_or(false);
        let direct = run_owned(untagged(wrap("/usr/bin/nc", &["-v", "-z", "-G", "2", "1.1.1.1", "443"])), t).await;
        let refused = matches!(&direct, Some((false, out)) if out.contains("not permitted"));
        let lookup = run_owned(untagged(wrap("/usr/bin/dscacheutil", &["-q", "host", "-a", "name", "example.com"])), t).await;
        let no_lookup = matches!(&lookup, Some((_, out)) if !out.contains("ip_address") && !out.contains("ipv6_address"));
        h.seatbelt_ok = control && refused && no_lookup;
        (h.user_exists, h.group_ok, h.can_run, h.filter_ok, h.rules_current, h.workspace_ok) = (true, true, true, true, true, true);
        return h;
    }
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
    if app.windowed {
        let argv = window_argv(app)?;
        // Its output goes nowhere. It must not go to a file in our data folder: the profile
        // closes that folder to the app, and an app that cannot touch its own output quits.
        return Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("{} could not be opened: {e}", app.name));
    }
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
    if app.windowed {
        return; // runs as you; the engine stops the one process it started (see `stop_pid`)
    }
    let _ = run(&["/usr/bin/sudo", "-n", "-u", &app.os_user, "/bin/sh", "-c", "kill -TERM -1"], Duration::from_secs(5)).await;
}

/// Ask one process we started to quit.
pub async fn stop_pid(pid: u32) {
    let _ = run(&["/bin/kill", "-TERM", &pid.to_string()], Duration::from_secs(4)).await;
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
pub fn filter_rules(cfg: &Config, own_servers: bool) -> String {
    let mut s = String::from("# Sandbox-ai packet filter rules. Generated by the app; changes are overwritten.\n");
    let (lo, hi) = (crate::catalog::GATE_PORTS.start(), crate::catalog::GATE_PORTS.end());
    if own_servers && cfg.apps.iter().any(wants_own_servers) {
        // One small rule set per agent, kept current by the watchdog: the ports that agent is
        // serving on right now are opened to it, and for exactly those ports arrivals at anyone
        // else's socket are refused, so a port that changes hands cannot be reached in between.
        // Looked at first, so no later rule can decide these packets differently.
        s.push_str("\n# Agents reach the servers they started themselves, and nobody else's\nanchor \"*\"\n");
    }
    for app in cfg.apps.iter().filter(|a| !a.windowed) {
        let u = &app.os_user;
        let mut out_ports: Vec<u16> = vec![app.proxy_port];
        out_ports.extend(&app.local_ports);
        let ports = out_ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
        s.push_str(&format!("\n# {}\n", app.name));
        s.push_str(&format!("pass out quick on lo0 proto tcp from any to any port {{ {ports} }} user {u}\n"));
        if app.loopback_all {
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

/// The rules the watchdog loads for one agent while it is serving on `ports`. Kept here, next to
/// the rest, so the tests can check them; the watchdog writes the same two lines.
#[cfg(test)]
fn own_rules(os_user: &str, ports: &[u16]) -> String {
    let list = ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
    format!("pass out quick on lo0 proto tcp from any to any port {{ {list} }} user {os_user}\nblock return in quick on lo0 proto tcp from any to any port {{ {list} }} user != {os_user}\n")
}

const WATCHDOG: &str = r#"#!/bin/bash
# Sandbox-ai fence watchdog. Runs as root from launchd and keeps the packet filter rules loaded,
# including after restarts and after anything else reloads the system rules.
ANCHOR=sandboxai
RULES=/etc/pf.anchors/sandboxai
CONF="/Library/Application Support/SandboxAI/pf.conf"
STATUS=/var/run/sandboxai.fence
USERS="@USERS@"
OWN_USERS="@OWN@"
MODE="/Library/Application Support/SandboxAI/local-servers"

# Agents and the servers they start themselves. Twice a second, while any agent is running: list
# the ports each agent is listening on and load exactly those into that agent's own small anchor.
# Until a new server is listed (half a second or so) the agent is told "connection refused", which
# is what it would see from a server that is still starting.
own_tick() {
  [ -n "$OWN_USERS" ] || return 0
  [ "$(cat "$MODE" 2>/dev/null)" = own ] || return 0
  local csv listing="" u ports list
  csv="$(echo "$OWN_USERS" | tr ' ' ',')"
  if pgrep -u "$csv" >/dev/null 2>&1; then
    listing="$(lsof -a -nP -iTCP -sTCP:LISTEN -u "$csv" -F Ln 2>/dev/null)"
  fi
  for u in $OWN_USERS; do
    ports="$(printf '%s\n' "$listing" | awk -v u="$u" '/^L/ { cur = substr($0, 2) } /^n/ { if (cur == u) { n = $0; sub(/.*:/, "", n); if (n ~ /^[0-9]+$/) print n } }' | sort -un | tr '\n' ' ')"
    [ "$ports" = "$(cat "/var/run/sandboxai.own.$u" 2>/dev/null)" ] && continue
    if [ -z "$ports" ]; then
      pfctl -q -a "$ANCHOR/own_${u#_sai}" -F rules 2>/dev/null
    else
      list="$(printf '%s' "$ports" | sed 's/ *$//' | tr ' ' ',')"
      printf 'pass out quick on lo0 proto tcp from any to any port { %s } user %s\nblock return in quick on lo0 proto tcp from any to any port { %s } user != %s\n' "$list" "$u" "$list" "$u" | pfctl -q -a "$ANCHOR/own_${u#_sai}" -f - 2>>"$STATUS.err"
    fi
    printf '%s' "$ports" > "/var/run/sandboxai.own.$u"
  done
}

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
# Nothing from before this start can be trusted to be loaded still.
rm -f /var/run/sandboxai.own.* 2>/dev/null
n=0
while true; do
  if [ "$n" -eq 0 ]; then ensure; fi
  own_tick
  n=$(( (n + 1) % 40 ))
  sleep 0.5
done
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
    let users: Vec<&str> = cfg.apps.iter().filter(|a| !a.windowed).map(|a| a.os_user.as_str()).collect();
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
    for app in cfg.apps.iter().filter(|a| !a.windowed) {
        s.push_str(&format!("ensure_user {} {}\n", sh_quote(&app.os_user), sh_quote(&home_for(&app.os_user))));
        if let Some(w) = &app.workspace {
            s.push_str(&format!("ensure_workspace {} {}\n", sh_quote(w), sh_quote(&app.os_user)));
        }
    }
    s.push_str("\n# 2. Packet filter rules, and a watchdog that keeps them loaded across restarts.\nSTEP=\"packet filter rules\"\nmkdir -p /etc/pf.anchors\n");
    s.push_str(&format!(
        "write_rules() {{ # none | own\n  if [ \"$1\" = own ]; then\ncat > /etc/pf.anchors/{FILTER_ANCHOR} <<'SAI_EOF'\n{}SAI_EOF\n  else\ncat > /etc/pf.anchors/{FILTER_ANCHOR} <<'SAI_EOF'\n{}SAI_EOF\n  fi\n  chown root:wheel /etc/pf.anchors/{FILTER_ANCHOR}; chmod 644 /etc/pf.anchors/{FILTER_ANCHOR}\n}}\nwrite_rules none\n",
        filter_rules(cfg, true),
        filter_rules(cfg, false),
    ));
    let own_users: Vec<&str> = cfg.apps.iter().filter(|a| wants_own_servers(a)).map(|a| a.os_user.as_str()).collect();
    s.push_str(&format!("TEST_USER={}\nOWN_USERS={}\n", sh_quote(own_users.first().copied().unwrap_or("")), sh_quote(&own_users.join(" "))));
    s.push_str(&format!("cat > \"$SUPPORT/fence-watchdog.sh\" <<'SAI_EOF'\n{}SAI_EOF\n", WATCHDOG.replace("@USERS@", &users.join(",")).replace("@OWN@", &own_users.join(" "))));
    s.push_str("chown root:wheel \"$SUPPORT/fence-watchdog.sh\"; chmod 755 \"$SUPPORT/fence-watchdog.sh\"\n");
    s.push_str(&format!("cat > /Library/LaunchDaemons/com.sandboxai.fence.plist <<'SAI_EOF'\n{DAEMON_PLIST}SAI_EOF\n"));
    s.push_str(r#"chown root:wheel /Library/LaunchDaemons/com.sandboxai.fence.plist; chmod 644 /Library/LaunchDaemons/com.sandboxai.fence.plist
launchctl bootout system/com.sandboxai.fence 2>/dev/null || true
sleep 1

# Start from the rules that allow agents their own local servers when there are agents; whether
# they stay is decided by the test below. Anything left from an earlier run goes first.
forget_own() { for u in $OWN_USERS; do pfctl -q -a "sandboxai/own_${u#_sai}" -F rules 2>/dev/null || true; rm -f "/var/run/sandboxai.own.$u"; done; }
set_local() { # none | own
  write_rules "$1"
  printf '%s\n' "$1" > "$SUPPORT/local-servers"; chown root:wheel "$SUPPORT/local-servers"; chmod 644 "$SUPPORT/local-servers"
}
forget_own
if [ -n "$TEST_USER" ]; then set_local own; else set_local none; fi

# Reload the rules now (the anchor may already hold an older set).
STEP="starting the watchdog"
pfctl -q -a sandboxai -F rules 2>/dev/null || true
launchctl enable system/com.sandboxai.fence 2>/dev/null || true
launchctl bootstrap system /Library/LaunchDaemons/com.sandboxai.fence.plist 2>/dev/null || launchctl kickstart -k system/com.sandboxai.fence

# 2b. Can agents be allowed the servers they start themselves, and only those? Tested here, on
# this computer, with the real rules and the real watchdog, before it is relied on: an agent's
# own test server must become reachable to it, and somebody else's must stay refused.
if [ -n "$TEST_USER" ]; then
  STEP="checking whether agents can be allowed their own local servers"
  sleep 2
  own=no; other=no
  sudo -n -u "$TEST_USER" /usr/bin/nc -l 127.0.0.1 47991 >/dev/null 2>&1 & p1=$!
  /usr/bin/nc -l 127.0.0.1 47993 >/dev/null 2>&1 & p2=$!
  for i in 1 2 3 4 5 6 7 8; do
    sleep 0.5
    if sudo -n -u "$TEST_USER" /usr/bin/nc -z -G 2 127.0.0.1 47991 >/dev/null 2>&1; then own="yes after $i tries"; break; fi
  done
  if sudo -n -u "$TEST_USER" /usr/bin/nc -z -G 2 127.0.0.1 47993 >/dev/null 2>&1; then other=yes; fi
  echo "local servers: its own reachable: $own; somebody else's reachable: $other"
  { pfctl -a sandboxai -sr | head -3; pfctl -a "sandboxai/own_${TEST_USER#_sai}" -sr; } 2>/dev/null | sed 's/^/    /' || true
  # The ordinary "please stop" signal is ignored in this context (seen on a real run: setup
  # waited forever on a test server nobody had connected to), so the test servers are stopped
  # the way that cannot be ignored, and nothing waits on them.
  kill -KILL "$p1" "$p2" 2>/dev/null || true
  /usr/bin/pkill -KILL -u "$TEST_USER" -f "nc -l 127.0.0.1 47991" 2>/dev/null || true
  case "$own/$other" in
    yes*/no) echo "local servers: on" ;;
    *) echo "local servers: the test did not pass, staying with the stricter rules"
       set_local none; forget_own
       pfctl -q -a sandboxai -f /etc/pf.anchors/sandboxai 2>/dev/null || true ;;
  esac
fi

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
    for app in cfg.apps.iter().filter(|a| !a.windowed) {
        let u = sh_quote(&app.os_user);
        s.push_str(&format!("pkill -u {u} 2>/dev/null || true\ndscl . -delete /Users/{} 2>/dev/null || true\nrm -rf {}\n", app.os_user, sh_quote(&home_for(&app.os_user))));
    }
    // Private users left behind by apps that were added and later removed from the list.
    s.push_str("for u in $(dscl . -list /Users | grep '^_sai'); do\n  if dscl . -read \"/Users/$u\" RealName 2>/dev/null | grep -q 'Sandbox-ai protected app'; then pkill -u \"$u\" 2>/dev/null || true; dscl . -delete \"/Users/$u\" 2>/dev/null || true; rm -rf \"/var/sai/${u#_sai}\"; fi\ndone\n");
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
    // Keep what the last run said, success or not: it is the only record of what was done as
    // administrator, and the only way to see why a self-test went the way it did.
    if let Some((ok, text)) = &out {
        let logs = crate::paths::logs_dir();
        let _ = std::fs::create_dir_all(&logs);
        let _ = std::fs::write(logs.join(format!("{name}.last.log")), format!("{} {}\n{}\n", crate::now_iso(), if *ok { "finished" } else { "failed" }, text.replace('\r', "\n")));
    }
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
        let rules = filter_rules(&cfg, false);
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

    fn windowed(id: &str, bundle: &str) -> AppConfig {
        let mut c = Config::fresh();
        let mut a = c.apps.remove(0);
        a.id = id.into();
        a.name = "Window".into();
        a.kind = "window".into();
        a.binary = bundle.into();
        a.os_user = "_saiwin1".into();
        a.proxy_port = crate::catalog::FIRST_CUSTOM_PORT;
        a.custom = true;
        a.windowed = true;
        a.workspace = None;
        a
    }

    #[test]
    fn a_windowed_app_is_fenced_by_its_profile_alone() {
        let a = windowed("my-window", "/Applications/Nope.app");
        let p = seatbelt_profile(&a);
        let deny = p.find("(deny network* ").unwrap();
        let local = p.find("(allow network-outbound (remote ip \"localhost:*\"))").unwrap();
        assert!(deny < local, "local traffic is allowed after the deny, so it wins");
        // Every gate but its own is closed again after that.
        for port in crate::catalog::GATE_PORTS {
            let rule = format!("(deny network-outbound (remote ip \"localhost:{port}\")");
            assert_eq!(p.contains(&rule), port != a.proxy_port, "{port}");
            if port != a.proxy_port {
                assert!(p.find(&rule).unwrap() > local);
            }
        }
        // The ways out for a program running as you.
        for needle in ["(deny lsopen ", "(deny appleevent-send ", "Library/LaunchAgents", "(deny file* (subpath ", "mDNSResponder"] {
            assert!(p.contains(needle), "{needle}");
        }
        assert!(!p.contains("(allow network*)") && !p.contains("(allow network-outbound)\n"));
        // It never appears in anything an administrator approves or the packet filter loads.
        let mut cfg = Config::fresh();
        cfg.apps.push(a);
        assert!(!filter_rules(&cfg, false).contains("_saiwin1") && !filter_rules(&cfg, true).contains("_saiwin1"));
        assert!(!setup_script(&cfg, "tester").contains("_saiwin1"));
        assert!(!remove_script(&cfg).contains("_saiwin1 "));
        assert!(cfg.provisioned().app("my-window").is_none());
    }

    #[test]
    fn a_launcher_is_a_small_app_that_asks_sandbox_ai_to_open_the_real_one() {
        let a = windowed("my-window", "/Applications/Nope.app");
        let path = launcher_path(&a).unwrap();
        assert!(path.ends_with("Applications/Sandbox-ai/Window (protected).app"));
        let script_for = |opener: &str| {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("l.sh");
            let script = format!("#!/bin/sh\nOPENER={}\nif [ -x \"$OPENER\" ]; then exec \"$OPENER\" --open-protected {}; fi\n", sh_quote(opener), sh_quote(&a.id));
            std::fs::write(&p, &script).unwrap();
            assert!(std::process::Command::new("/bin/sh").arg("-n").arg(&p).status().unwrap().success());
            script
        };
        // A path with a quote and a space cannot break out of the script.
        let s = script_for("/Apps/It's here/Sandbox-ai.app/Contents/MacOS/sandbox-ai");
        assert!(s.contains("'/Apps/It'\\''s here/Sandbox-ai.app/Contents/MacOS/sandbox-ai'"));
    }

    #[test]
    fn agents_reach_their_own_local_servers_and_nobody_elses() {
        let cfg = Config::fresh();
        let (claude, ollama) = (cfg.app("claude-code").unwrap(), cfg.app("ollama").unwrap());
        let rules = filter_rules(&cfg, true);
        // The per-agent rule sets are looked at before anything else.
        let children = rules.find("anchor \"*\"").expect("child anchors");
        assert!(children < rules.find("\n# Claude Code").unwrap());
        // The static part for an agent is unchanged: its gate, the local model, nothing else.
        let strict = filter_rules(&cfg, false);
        assert_eq!(rules.replace("\n# Agents reach the servers they started themselves, and nobody else's\nanchor \"*\"\n", ""), strict);
        assert!(!strict.contains("anchor"));
        assert!(!strict.contains("pass out quick on lo0 proto tcp from any to any user _saiagent\n"));
        // What the watchdog loads for an agent serving on two ports: those ports, for that agent,
        // and for those ports nobody else's socket.
        let own = own_rules("_saiagent", &[3000, 5173]);
        assert!(own.contains("port { 3000, 5173 } user _saiagent\n") && own.contains("port { 3000, 5173 } user != _saiagent\n"));
        assert_eq!(own_anchor("_saiagent"), "own_agent");
        // The watchdog script writes the same two rules, and is told which users to do it for.
        assert!(WATCHDOG.contains("pass out quick on lo0 proto tcp from any to any port { %s } user %s") && WATCHDOG.contains("user != %s"));
        let script = setup_script(&cfg, "tester");
        let agents: Vec<&str> = cfg.apps.iter().filter(|a| a.kind == "agent").map(|a| a.os_user.as_str()).collect();
        assert!(agents.len() >= 5 && !agents.contains(&"_saiollama"));
        assert!(script.contains(&format!("OWN_USERS=\"{}\"", agents.join(" "))) && !script.contains("@OWN@"));
        // The launch-time profile follows the same switch, and never opens another app's gate.
        let p = seatbelt_profile_for(claude, true);
        let all = p.find("(allow network-outbound (remote ip \"localhost:*\"))").unwrap();
        assert!(p.find(&format!("(deny network-outbound (remote ip \"localhost:{}\")", ollama.proxy_port)).unwrap() > all);
        assert!(!p.contains(&format!("(deny network-outbound (remote ip \"localhost:{}\")", claude.proxy_port)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_packet_filter_accepts_both_rule_sets() {
        // pfctl -n only reads the rules; it loads nothing and needs no special rights.
        let mut cfg = Config::fresh();
        cfg.add_custom("Mine", "/bin/sh").unwrap();
        for own in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("rules");
            // The private users need not exist on the computer running the tests: the grammar
            // is what is being checked, so every one of them is replaced by a user that does.
            let mut rules = filter_rules(&cfg, own);
            for a in &cfg.apps {
                rules = rules.replace(&format!("user {}\n", a.os_user), "user root\n").replace(&format!("user {} ", a.os_user), "user root ");
            }
            if own {
                // Checked as one file: the child rules use the same grammar as the main ones.
                rules.push_str(&own_rules("root", &[3000, 5173]));
            }
            assert!(!rules.contains("user _sai"));
            std::fs::write(&p, rules).unwrap();
            let out = std::process::Command::new("/sbin/pfctl").arg("-n").arg("-f").arg(&p).output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success() && !err.contains("syntax error"), "own={own}: {err}");
        }
    }

    #[test]
    fn the_app_bundle_is_found_from_its_executable() {
        assert_eq!(bundle_of("/Applications/Sandbox-ai.app/Contents/MacOS/sandbox-ai").as_deref(), Some("/Applications/Sandbox-ai.app"));
        assert_eq!(bundle_of("/Volumes/Sandbox-ai/Sandbox-ai.app/Contents/MacOS/sandbox-ai").as_deref(), Some("/Volumes/Sandbox-ai/Sandbox-ai.app"));
        assert_eq!(bundle_of("/usr/local/bin/sai"), None);
    }

    #[test]
    fn known_windowed_apps_are_matched_by_name() {
        assert!(crate::catalog::is_known_windowed("LM Studio"));
        assert!(crate::catalog::is_known_windowed("lm studio"));
        assert!(!crate::catalog::is_known_windowed("TextEdit"));
        assert!(!crate::catalog::is_known_windowed("LM Studio Helper"));
    }

    #[test]
    fn apps_in_apples_sandbox_are_recognised() {
        assert!(app_sandboxed("<dict><key>com.apple.security.app-sandbox</key><true/></dict>"));
        assert!(app_sandboxed("<key>com.apple.security.app-sandbox</key>\n\t<true/>"));
        assert!(!app_sandboxed("<dict><key>com.apple.security.app-sandbox</key><false/></dict>"));
        assert!(!app_sandboxed("<dict><key>com.apple.security.cs.allow-jit</key><true/></dict>"));
        assert!(!app_sandboxed(""));
        assert!(inspect_bundle("/Applications/No Such App.app").is_err());
        assert!(inspect_bundle("/bin/sh").is_err());
        assert_eq!(regex_escape("/Applications/A (B).app/x"), "/Applications/A \\(B\\)\\.app/x");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_system_app_in_apples_sandbox_is_refused_with_a_reason() {
        // TextEdit ships with every Mac and lives in the App Sandbox.
        if std::path::Path::new("/System/Applications/TextEdit.app").is_dir() {
            let err = inspect_bundle("/System/Applications/TextEdit.app").unwrap_err();
            assert!(err.contains("Apple's own sandbox"), "{err}");
        }
    }

    #[test]
    fn refusals_are_read_from_the_system_log() {
        let v = parse_violation("Sandbox: Obsidian(7) deny(1) lsopen\nsai-app=my-window;open").unwrap();
        assert_eq!((v.what, v.port), ("open", 0));
        let v = parse_violation("Sandbox: Obsidian(7) deny(1) appleevent-send com.apple.finder\nsai-app=my-window;event").unwrap();
        assert_eq!(v.what, "control");
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
        let p = seatbelt_profile_for(app, false);
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
        let p = seatbelt_profile_for(ollama, false);
        let allow_all = p.find("(allow network-outbound (remote ip \"localhost:*\"))").expect("loopback allowance");
        let deny_other = p.find(&format!("(deny network-outbound (remote ip \"localhost:{}\") ", claude.proxy_port)).expect("other gate denied");
        assert!(allow_all < deny_other, "the deny must come after the allow so it wins");
        assert!(!p.contains(&format!("(deny network-outbound (remote ip \"localhost:{}\")", ollama.proxy_port)), "its own gate stays reachable");
        // An agent gets no such allowance.
        assert!(!seatbelt_profile_for(claude, false).contains("localhost:*\"))\n(deny network-outbound (remote ip"));
        assert!(!seatbelt_profile_for(claude, false).contains("(allow network-outbound (remote ip \"localhost:*\"))"));
        let rules = filter_rules(&cfg, false);
        let block = rules.find(&format!("port {}:{} user {}", crate::catalog::GATE_PORTS.start(), crate::catalog::GATE_PORTS.end(), ollama.os_user)).expect("gate range blocked for the model server");
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
