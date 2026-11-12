//! The Box: running an agent in a Linux virtual machine that has no network interface.
//!
//! The fence keeps a program on this Mac from reaching the network except through its gate.
//! The Box goes one step further for people who want it: the agent does not run on this Mac's
//! system at all. It runs in a small Linux machine (Apple's own virtualisation, no extra
//! software) that has
//!
//! - **no network interface**: there is nothing to send a packet on. Its one way out is a
//!   virtual socket to a helper on the Mac, which hands every connection to the app's gate;
//!   so the same rules, the same questions and the same signed log apply;
//! - **no name server**: names are looked up by the gate, outside;
//! - **one folder of yours**, at `/work`, and nothing else of this Mac.
//!
//! Every run starts from an instant copy of a prepared disk (`base.raw` → `run.raw`), so a run
//! cannot damage the next one, and "roll the Box back" is simply "start it again".
//!
//! macOS on Apple silicon only. One Box runs at a time. The disk is made with what macOS
//! ships and this program (see `boxdisk`); the app's window drives it through `status`,
//! which also says what a long step (set-up, putting an agent in) is doing right now.

use crate::config::AppConfig;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// The port on this Mac that commands for the Box go in through. Outside the range of gates.
pub const COMMAND_PORT: u16 = 47190;

struct Running {
    child: tokio::process::Child,
    app_id: String,
    share: String,
    started: String,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// What the kernel is started with. A console the helper records, no wait for a network that
/// is not there, and `arm64.nopauth`, without which this kernel does not start under Apple's
/// virtualisation on newer Macs.
const KERNEL_LINE: &str = "console=tty0 console=hvc0 consoleblank=0 systemd.mask=systemd-networkd-wait-online.service apparmor=0 arm64.nopauth";

/// A long step in progress (making the disk, putting an agent in), for the app's window.
struct Busy {
    what: &'static str,
    app_id: Option<String>,
    step: String,
    since: String,
}

static BUSY: Mutex<Option<Busy>> = Mutex::new(None);
/// How the last long step ended: `{ what, appId, ok, message, at }`.
static LAST: Mutex<Option<Value>> = Mutex::new(None);

/// Holds the "busy" mark for one long step; dropping it clears the mark however the step ends.
struct Working;

impl Working {
    fn begin(what: &'static str, app_id: Option<&str>, step: &str) -> Result<Working, String> {
        let mut busy = BUSY.lock().unwrap();
        if let Some(b) = busy.as_ref() {
            return Err(format!("The Box is busy ({}). Try again when it has finished.", b.step.to_lowercase()));
        }
        *busy = Some(Busy { what, app_id: app_id.map(str::to_string), step: step.to_string(), since: crate::now_iso() });
        *LAST.lock().unwrap() = None;
        Ok(Working)
    }

    fn step(&self, text: &str) {
        if let Some(b) = BUSY.lock().unwrap().as_mut() {
            b.step = text.to_string();
        }
    }

    fn end(self, result: &Result<String, String>) {
        let (what, app_id) = BUSY.lock().unwrap().as_ref().map(|b| (b.what, b.app_id.clone())).unwrap_or(("", None));
        let (ok, message) = match result {
            Ok(m) => (true, m.clone()),
            Err(m) => (false, m.clone()),
        };
        *LAST.lock().unwrap() = Some(json!({ "what": what, "appId": app_id, "ok": ok, "message": message, "at": crate::now_iso() }));
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        *BUSY.lock().unwrap() = None;
    }
}

/// A long step that was refused before it began (so never marked itself busy) still has an
/// ending to show.
pub fn record_failure(what: &str, app_id: Option<&str>, message: &str) {
    let mut last = LAST.lock().unwrap();
    if last.as_ref().and_then(|l| l["message"].as_str()) != Some(message) {
        *last = Some(json!({ "what": what, "appId": app_id, "ok": false, "message": message, "at": crate::now_iso() }));
    }
}

/// Forget how the last long step ended (the window has shown it).
pub fn dismiss_last() {
    *LAST.lock().unwrap() = None;
}

/// The agents that have been put into the prepared disk, and the version each reported.
pub fn installed() -> serde_json::Map<String, Value> {
    std::fs::read(dir().join("installed.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn set_installed(app_id: &str, version: &str) {
    let mut all = installed();
    all.insert(app_id.to_string(), json!(version));
    let _ = std::fs::write(dir().join("installed.json"), serde_json::to_vec(&all).unwrap_or_default());
}

pub fn dir() -> PathBuf {
    crate::paths::data_dir().join("box")
}

fn key() -> PathBuf {
    dir().join("box_ed25519")
}

/// The helper that runs the machine: next to this program in the app, or where `box/build.sh`
/// puts it in a source checkout.
pub fn helper() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HOMEBODY_BOX_HELPER") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?;
    [here.join("homebody-box"), here.join("../box/homebody-box"), here.join("../../box/homebody-box")].into_iter().find(|p| p.is_file())
}

/// The scripts that make the disk: in the app's resources, or in a source checkout.
pub fn image_script() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HOMEBODY_BOX_SCRIPTS") {
        return Some(PathBuf::from(p).join("image.sh")).filter(|p| p.is_file());
    }
    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?;
    [here.join("../Resources/box/image.sh"), here.join("../../box/image.sh"), here.join("../../../box/image.sh")].into_iter().find(|p| p.is_file())
}

pub fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

pub fn prepared() -> bool {
    dir().join("base.raw").is_file() && key().is_file() && dir().join("prepared").is_file()
}

pub fn status() -> Value {
    let mut running = RUNNING.lock().unwrap();
    // A machine that stopped by itself is no longer running.
    if let Some(r) = running.as_mut() {
        if !matches!(r.child.try_wait(), Ok(None)) {
            *running = None;
        }
    }
    let busy = BUSY.lock().unwrap().as_ref().map(|b| {
        // While Debian is coming down, how much has arrived.
        let arrived = if b.what == "setup" { std::fs::metadata(dir().join(DOWNLOAD)).ok().map(|m| m.len() / 1_048_576) } else { None };
        json!({ "what": b.what, "appId": b.app_id, "step": b.step, "since": b.since, "downloadedMb": arrived, "downloadMb": DOWNLOAD_MB })
    });
    json!({
        "supported": supported(),
        "helper": helper().is_some(),
        "prepared": prepared(),
        "running": running.is_some(),
        "appId": running.as_ref().map(|r| r.app_id.clone()),
        "share": running.as_ref().map(|r| r.share.clone()),
        "since": running.as_ref().map(|r| r.started.clone()),
        "busy": busy,
        "last": LAST.lock().unwrap().clone(),
        "installed": installed(),
        "downloadMb": DOWNLOAD_MB,
    })
}

/// The file Debian's image arrives in (the name `box/image.sh` gives it), and about how big it is.
const DOWNLOAD: &str = "debian-12-genericcloud-arm64.tar.xz";
const DOWNLOAD_MB: u64 = 200;

fn ssh_base(user: &str) -> Vec<String> {
    // -F /dev/null: none of your own ssh settings apply (a "forward my agent to every host"
    // line would otherwise hand the Box your keys).
    let mut v: Vec<String> = ["/usr/bin/ssh", "-F", "/dev/null", "-o", "ForwardAgent=no", "-o", "ForwardX11=no", "-i"].iter().map(|s| s.to_string()).collect();
    v.push(key().to_string_lossy().into_owned());
    v.extend(["-p", &COMMAND_PORT.to_string(), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5", "-o", "IdentitiesOnly=yes", "-o", "ServerAliveInterval=20", "-o", "ServerAliveCountMax=6"].iter().map(|s| s.to_string()));
    v.push(format!("{user}@127.0.0.1"));
    v
}

async fn run(argv: Vec<String>, timeout: Duration) -> Option<(bool, String)> {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output()).await.ok()?.ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

/// Run something in the guest as root (setup only) or as the agent.
async fn inside(user: &str, script: &str, timeout: Duration) -> Option<(bool, String)> {
    let mut argv = ssh_base(user);
    argv.push(script.to_string());
    run(argv, timeout).await
}

async fn wait_reachable(seconds: u64) -> bool {
    for _ in 0..seconds {
        if let Some((true, _)) = inside("agent", "true", Duration::from_secs(6)).await {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn spawn_helper(disk: &str, seed: Option<&str>, gate_port: u16, share: Option<&str>) -> Result<tokio::process::Child, String> {
    let helper = helper().ok_or("The Box's helper program is missing from this copy of Homebody.")?;
    let d = dir();
    let mut cmd = tokio::process::Command::new(helper);
    cmd.arg("run").arg("--disk").arg(disk).arg("--state").arg(d.join("state")).arg("--gate-port").arg(gate_port.to_string()).arg("--ssh-port").arg(COMMAND_PORT.to_string()).arg("--log").arg(d.join("console.log"));
    if let Some(s) = seed {
        cmd.arg("--seed").arg(s);
    }
    if let Some(s) = share {
        cmd.arg("--share").arg(s);
    }
    let log = std::fs::File::create(d.join("helper.log")).map_err(|e| e.to_string())?;
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::from(log)).kill_on_drop(true);
    cmd.spawn().map_err(|e| format!("The Box could not be started: {e}"))
}

async fn shut_down(mut child: tokio::process::Child) {
    // Ask the guest to stop so its disk is left whole; the helper exits when it has.
    let _ = inside("root", "sync; systemctl poweroff", Duration::from_secs(8)).await;
    if tokio::time::timeout(Duration::from_secs(15), child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

/// Make the Box's disk: fetch Debian's image (or use one already here), prepare it, start it
/// once so the guest configures itself, and shut it down cleanly. Long: minutes on first use.
/// `gate_port` is the gate the guest's set-up traffic goes through.
pub async fn setup(gate_port: u16, existing_image: Option<&str>) -> Result<String, String> {
    if !supported() {
        return Err("The Box needs a Mac with Apple silicon.".into());
    }
    if RUNNING.lock().unwrap().is_some() {
        return Err("Stop the Box first.".into());
    }
    let working = Working::begin("setup", None, "Getting ready")?;
    let result = setup_steps(&working, gate_port, existing_image).await;
    working.end(&result);
    result
}

async fn setup_steps(working: &Working, gate_port: u16, existing_image: Option<&str>) -> Result<String, String> {
    let script = image_script().ok_or("The Box's set-up scripts are missing from this copy of Homebody.")?;
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let step = |args: Vec<String>, minutes: u64| {
        let script = script.clone();
        let d = d.clone();
        async move {
            let mut argv = vec!["/bin/bash".to_string(), script.to_string_lossy().into_owned(), d.to_string_lossy().into_owned()];
            argv.extend(args);
            match run(argv, Duration::from_secs(minutes * 60)).await {
                Some((true, _)) => Ok(()),
                Some((false, out)) => Err(out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("The Box's disk could not be made.").trim().to_string()),
                None => Err("Making the Box's disk took too long.".to_string()),
            }
        }
    };
    let _ = std::fs::remove_file(d.join("prepared"));
    let _ = std::fs::remove_file(d.join("installed.json"));
    match existing_image {
        Some(img) => step(vec!["use".into(), img.to_string()], 5).await?,
        None => {
            if !d.join("pristine.raw").is_file() {
                working.step("Downloading Debian");
            }
            step(vec!["fetch".into()], 30).await?
        }
    }
    working.step("Preparing the disk");
    step(vec!["seed".into()], 2).await?;
    // A fresh base from the untouched image (an instant copy), with its start-up line set.
    let (pristine, base) = (d.join("pristine.raw"), d.join("base.raw"));
    let _ = std::fs::remove_file(&base);
    let cloned = std::process::Command::new("/bin/cp").arg("-c").arg(&pristine).arg(&base).status().map(|s| s.success()).unwrap_or(false);
    if !cloned {
        std::fs::copy(&pristine, &base).map_err(|e| format!("The Box's disk could not be copied: {e}"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    let image = base.clone();
    tokio::task::spawn_blocking(move || crate::boxdisk::prepare(&image, KERNEL_LINE)).await.map_err(|e| e.to_string())??;
    working.step("Starting it for the first time");
    // First start: the guest reads its instructions and sets itself up (a few seconds).
    let (base, seed) = (d.join("base.raw").to_string_lossy().into_owned(), d.join("seed.iso").to_string_lossy().into_owned());
    let _ = std::fs::remove_dir_all(d.join("state"));
    let child = spawn_helper(&base, Some(&seed), gate_port, None).await?;
    let ready = wait_reachable(90).await;
    let checked = if ready { inside("root", "test -e /etc/cloud/cloud-init.disabled && systemctl is-active homebody-forward && ip -br link | grep -vc '^lo '", Duration::from_secs(10)).await } else { None };
    shut_down(child).await;
    match checked {
        // "ip … | grep -vc '^lo '" prints the number of interfaces that are not loopback: 0, and grep exits 1.
        Some((_, out)) if out.contains("active") && out.lines().last().map(str::trim) == Some("0") => {
            std::fs::write(d.join("prepared"), crate::now_iso()).map_err(|e| e.to_string())?;
            Ok("The Box is ready.".into())
        }
        Some((_, out)) => Err(format!("The Box started but did not set itself up as expected: {}", out.lines().last().unwrap_or("").trim())),
        None => Err("The Box did not answer after its first start. Its console is in the box folder (console.log).".into()),
    }
}

/// How an agent gets into the Box's prepared disk: Debian packages, one npm package, the command
/// that shows it worked, and the places the installation itself has to reach.
pub struct Recipe {
    pub apt: &'static [&'static str],
    pub npm: &'static str,
    pub check: &'static str,
    /// The command that starts the agent inside the Box.
    pub program: &'static str,
    pub hosts: &'static [&'static str],
}

const NODE: &[&str] = &["nodejs", "npm", "git", "ca-certificates"];
const PACKAGE_PLACES: &[&str] = &["deb.debian.org", "security.debian.org", "registry.npmjs.org"];

pub fn recipe(app_id: &str) -> Option<Recipe> {
    let npm = match app_id {
        "claude-code" => ("@anthropic-ai/claude-code", "claude --version", "claude"),
        "codex" => ("@openai/codex", "codex --version", "codex"),
        "gemini" => ("@google/gemini-cli", "gemini --version", "gemini"),
        _ => return None,
    };
    Some(Recipe { apt: NODE, npm: npm.0, check: npm.1, program: npm.2, hosts: PACKAGE_PLACES })
}

/// Install an agent into the prepared disk, so every later start of the Box has it. The Box is
/// started on the prepared disk itself for this (not on a copy), everything is fetched through
/// the given gate, and it is shut down cleanly afterwards. Minutes.
pub async fn install(gate_port: u16, app_id: &str, name: &str, r: &Recipe) -> Result<String, String> {
    if !prepared() {
        return Err("The Box has not been set up yet.".into());
    }
    if RUNNING.lock().unwrap().is_some() {
        return Err("Stop the Box first.".into());
    }
    let working = Working::begin("install", Some(app_id), "Starting the Box")?;
    let result = install_steps(&working, gate_port, name, r).await;
    if let Ok(version) = &result {
        set_installed(app_id, version);
    }
    working.end(&result);
    result
}

async fn install_steps(working: &Working, gate_port: u16, name: &str, r: &Recipe) -> Result<String, String> {
    let base = dir().join("base.raw").to_string_lossy().into_owned();
    let child = spawn_helper(&base, None, gate_port, None).await?;
    if !wait_reachable(60).await {
        shut_down(child).await;
        return Err("The Box did not answer after starting.".into());
    }
    let script = format!(
        "set -e; export DEBIAN_FRONTEND=noninteractive; . /etc/profile.d/homebody-proxy.sh; apt-get update -q >/dev/null; apt-get install -y -q --no-install-recommends {} >/dev/null; npm install -g --no-fund --no-audit --loglevel=error {} >/dev/null; apt-get clean; {}",
        r.apt.join(" "),
        sh_quote(r.npm),
        r.check
    );
    working.step(&format!("Installing {name}"));
    let out = inside("root", &script, Duration::from_secs(25 * 60)).await;
    working.step("Shutting the Box down");
    shut_down(child).await;
    match out {
        Some((true, text)) => Ok(text.lines().last().unwrap_or("installed").trim().to_string()),
        Some((false, text)) => Err(format!("The installation did not finish: {}", text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no reason given").trim())),
        None => Err("The installation took too long.".into()),
    }
}

/// Start the Box for one app, with one folder of yours at /work. Starts from a fresh copy of
/// the prepared disk every time.
pub async fn start(app: &AppConfig, share: &str) -> Result<(), String> {
    if !prepared() {
        return Err("The Box has not been set up yet.".into());
    }
    if BUSY.lock().unwrap().is_some() {
        return Err("The Box is busy. Try again when it has finished.".into());
    }
    if !std::path::Path::new(share).is_dir() {
        return Err("The folder to share with the Box does not exist.".into());
    }
    if let Some(r) = RUNNING.lock().unwrap().as_ref() {
        return Err(format!("The Box is already running for {}. Stop it first.", r.app_id));
    }
    let d = dir();
    let (base, disk) = (d.join("base.raw"), d.join("run.raw"));
    let _ = std::fs::remove_file(&disk);
    // An instant copy on this Mac's disk format: no space is used until the copy changes.
    let cloned = std::process::Command::new("/bin/cp").arg("-c").arg(&base).arg(&disk).status().map(|s| s.success()).unwrap_or(false);
    if !cloned {
        return Err("The Box's disk could not be copied for this run.".into());
    }
    let child = spawn_helper(&disk.to_string_lossy(), None, app.proxy_port, Some(share)).await?;
    *RUNNING.lock().unwrap() = Some(Running { child, app_id: app.id.clone(), share: share.to_string(), started: crate::now_iso() });
    if !wait_reachable(60).await {
        stop().await;
        return Err("The Box did not answer after starting. Its console is in the box folder (console.log).".into());
    }
    // The shared folder, mounted where the agent works.
    let _ = inside("root", "mountpoint -q /work || mount -t virtiofs work /work", Duration::from_secs(10)).await;
    Ok(())
}

pub async fn stop() {
    let running = RUNNING.lock().unwrap().take();
    if let Some(r) = running {
        shut_down(r.child).await;
        let _ = std::fs::remove_file(dir().join("run.raw"));
    }
}

/// The command line that runs something in the running Box, as the agent, in /work, with a
/// terminal when there is one.
pub fn argv(program: &str, args: &[String], interactive: bool) -> Result<Vec<String>, String> {
    if RUNNING.lock().unwrap().is_none() {
        return Err("The Box is not running.".into());
    }
    let mut v = ssh_base("agent");
    if interactive {
        v.insert(1, "-t".into());
        // (before the options above; ssh takes flags in any order)
    }
    let command = std::iter::once(program.to_string()).chain(args.iter().cloned()).map(|a| sh_quote(&a)).collect::<Vec<_>>().join(" ");
    v.push(format!("cd /work 2>/dev/null; . /etc/profile.d/homebody-proxy.sh; exec {command}"));
    Ok(v)
}

/// What a Terminal window runs to put you in the running Box with the agent: `argv` is the
/// command that reaches into the Box (see `argv`).
pub fn terminal_script(name: &str, share: &str, argv: &[String]) -> String {
    format!(
        "#!/bin/zsh\ncd /\nclear\nprintf '\\033[1m%s, in the Box\\033[0m\\n' {name}\nprintf 'A sealed Linux machine: no network but its gate, and of this Mac only:\\n  %s\\n\\n' {share}\n{cmd}\nprintf '\\n\\033[2mSession ended. The Box is still running; stop it from Homebody.\\033[0m\\n'\n",
        name = sh_quote(name),
        share = sh_quote(share),
        cmd = argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "),
    )
}

/// Open a Terminal window in the running Box, running the agent.
pub async fn open_terminal(app: &AppConfig, program: &str) -> Result<(), String> {
    let share = RUNNING.lock().unwrap().as_ref().map(|r| r.share.clone()).ok_or("The Box is not running.")?;
    let run_dir = crate::paths::data_dir().join("run");
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
    let script = run_dir.join(format!("box-{}.command", app.id));
    std::fs::write(&script, terminal_script(&app.name, &share, &argv(program, &[], true)?)).map_err(|e| e.to_string())?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700));
    }
    let path = script.to_string_lossy().into_owned();
    match run(vec!["/usr/bin/open".into(), "-a".into(), "Terminal".into(), path], Duration::from_secs(10)).await {
        Some((true, _)) => Ok(()),
        Some((false, out)) => Err(format!("Terminal could not be opened: {}", out.trim())),
        None => Err("Terminal could not be opened.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_for_the_box_is_quoted_so_it_arrives_as_typed() {
        *RUNNING.lock().unwrap() = None;
        assert!(argv("true", &[], false).is_err(), "nothing runs in a Box that is not running");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        let base = ssh_base("agent");
        assert!(base.contains(&"agent@127.0.0.1".to_string()) && base.contains(&COMMAND_PORT.to_string()));
        assert!(!crate::catalog::GATE_PORTS.contains(&COMMAND_PORT), "the command port is not a gate");
    }

    #[test]
    fn the_terminal_script_is_valid_and_quotes_what_it_is_given() {
        let mut argv = ssh_base("agent");
        argv.insert(1, "-t".into());
        argv.push("cd /work; exec 'claude'".into());
        let script = terminal_script("Bob's agent", "/Users/Shared/Home body/x", &argv);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.command");
        std::fs::write(&p, &script).unwrap();
        assert!(std::process::Command::new("/bin/sh").arg("-n").arg(&p).status().unwrap().success(), "syntax error in:\n{script}");
        assert!(script.contains("'Bob'\\''s agent'") && script.contains("'/Users/Shared/Home body/x'"));
        assert!(script.contains("'-F' '/dev/null'") && script.contains("'ForwardAgent=no'"), "none of your own ssh settings apply");
    }

    #[test]
    fn one_long_step_at_a_time_and_how_it_ended_is_kept() {
        let first = Working::begin("setup", None, "Getting ready").unwrap();
        first.step("Downloading Debian");
        let st = status();
        assert_eq!(st["busy"]["step"], "Downloading Debian");
        assert_eq!(st["busy"]["what"], "setup");
        let second = Working::begin("install", Some("claude-code"), "Starting the Box");
        assert!(second.err().unwrap().contains("busy"), "a second long step is refused while one runs");
        first.end(&Err("No room on the disk.".into()));
        let st = status();
        assert!(st["busy"].is_null(), "the mark is cleared however the step ends");
        assert_eq!(st["last"]["ok"], false);
        assert_eq!(st["last"]["message"], "No room on the disk.");
        assert_eq!(st["last"]["what"], "setup");
        // A step refused before it began is recorded too, once.
        dismiss_last();
        record_failure("install", Some("codex"), "Stop the Box first.");
        assert_eq!(status()["last"]["appId"], "codex");
        // Starting another step clears the old ending.
        let next = Working::begin("install", Some("codex"), "Starting the Box").unwrap();
        assert!(status()["last"].is_null());
        drop(next);
        assert!(status()["busy"].is_null());
    }
}
