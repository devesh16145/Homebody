//! `homebody ci`: run an agent protected with nobody at the keyboard.
//!
//! For build pipelines and scripts. The same fence, gate, rules and signed log as the app; the
//! differences are the ones a machine needs:
//!
//! - nothing is ever asked: a place that is not trusted is refused at once;
//! - the places a run needs are named up front (`--trust`, `--rules`) and forgotten afterwards;
//! - the project is brought in as a copy and, with `--apply`, the changes are put back;
//! - the run ends with a summary and, with `--report`, a folder of signed evidence;
//! - the exit code is the command's own, or 3 when something was refused and `--fail-on-blocked`.

use homebody_core::{control, Engine};
use serde_json::{json, Value};
use std::time::Duration;

pub const HELP: &str = "homebody ci [options] <app> -- <command...>

Runs the command protected, without asking anything. Options:
  --project <folder>     bring this folder in as a copy and run there
  --apply                afterwards, put the changes back into the folder
  --keep-copy            leave the copy in place afterwards (it is deleted otherwise)
  --trust <place>        also trust this place for this run (repeatable)
  --rules <file>         also trust the places a file names: {\"trust\": [\"registry.npmjs.org\"]}
  --env <NAME>           hand this variable to the command (repeatable); nothing else is passed on
  --report <folder>      save the signed log, a table, the rules and a summary there
  --fail-on-blocked      exit with 3 if anything was refused

Before the first run on a machine:  homebody setup --print | sudo bash";

#[derive(Default)]
struct Opts {
    app: String,
    command: Vec<String>,
    project: Option<String>,
    apply: bool,
    keep_copy: bool,
    trust: Vec<String>,
    rules: Option<String>,
    env: Vec<String>,
    report: Option<String>,
    fail_on_blocked: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    let value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value."))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                o.command = args[i + 1..].to_vec();
                break;
            }
            "--project" => o.project = Some(value(&mut i, "--project")?),
            "--apply" => o.apply = true,
            "--keep-copy" => o.keep_copy = true,
            "--trust" => o.trust.push(value(&mut i, "--trust")?),
            "--rules" => o.rules = Some(value(&mut i, "--rules")?),
            "--env" => o.env.push(value(&mut i, "--env")?),
            "--report" => o.report = Some(value(&mut i, "--report")?),
            "--fail-on-blocked" => o.fail_on_blocked = true,
            x if x.starts_with("--") => return Err(format!("Unknown option: {x}")),
            x if o.app.is_empty() => o.app = x.to_string(),
            x => return Err(format!("Unexpected \"{x}\". Put the command after \"--\".")),
        }
        i += 1;
    }
    if o.app.is_empty() || o.command.is_empty() {
        return Err("Say which app and which command.".into());
    }
    if o.apply && o.project.is_none() {
        return Err("--apply needs --project.".into());
    }
    Ok(o)
}

/// Places named in a rules file: `{"trust": ["a.example", "*.b.example"]}`.
fn places_in(file: &str) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("The rules file could not be read: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("The rules file is not valid JSON: {e}"))?;
    let list = v.get("trust").and_then(Value::as_array).ok_or("The rules file has no \"trust\" list.")?;
    list.iter().map(|h| h.as_str().map(str::to_string).ok_or_else(|| "Every entry in \"trust\" must be text.".to_string())).collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn human(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

/// One line per place the run touched.
#[derive(Default, Clone)]
struct Place {
    host: String,
    label: String,
    allowed: u64,
    refused: u64,
    sent: u64,
    why: String,
}

fn places(events: &[Value]) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    for e in events {
        let host = if s(e, "host").is_empty() { s(e, "label") } else { s(e, "host") };
        let i = match out.iter().position(|p| p.host == host) {
            Some(i) => i,
            None => {
                out.push(Place { host: host.to_string(), label: s(e, "label").to_string(), ..Default::default() });
                out.len() - 1
            }
        };
        match s(e, "verb") {
            "blocked" => {
                out[i].refused += 1;
                out[i].why = s(e, "explanation").to_string();
            }
            _ => out[i].allowed += 1,
        }
        out[i].sent += e.get("bytesOut").and_then(Value::as_u64).unwrap_or(0);
    }
    out.sort_by(|a, b| b.refused.cmp(&a.refused).then(a.host.cmp(&b.host)));
    out
}

fn markdown(app: &str, command: &[String], code: i32, list: &[Place], verified: &Value) -> String {
    let refused: u64 = list.iter().map(|p| p.refused).sum();
    let mut m = format!("### Homebody: {app}\n\n");
    m += &match refused {
        0 => "Nothing was refused. Everything that left went to a trusted place.\n\n".to_string(),
        n => format!("**{n} attempt{} to reach a place that is not trusted {} refused.**\n\n", if n == 1 { "" } else { "s" }, if n == 1 { "was" } else { "were" }),
    };
    if !list.is_empty() {
        m += "| Place | Allowed | Refused | Sent |\n|---|---:|---:|---:|\n";
        for p in list {
            m += &format!("| {} | {} | {} | {} |\n", p.host.replace('|', "\\|"), p.allowed, p.refused, human(p.sent));
        }
        m += "\n";
    }
    m += &format!("Command: `{}` (exit code {code}). Log: {} records, {}.\n", command.join(" ").replace('`', "'"), verified["count"], if verified["ok"].as_bool() == Some(true) { "chain intact" } else { "CHAIN BROKEN" });
    m
}

async fn call(cmd: &str, args: Value) -> Result<Value, String> {
    control::call(cmd, args).await
}

/// Wait for the first health check, then say whether this app can run protected here.
async fn ready(app: &str) -> Result<(), String> {
    for _ in 0..80 {
        let st = call("get_status", Value::Null).await?;
        let Some(a) = st["apps"].as_array().into_iter().flatten().find(|a| s(a, "id") == app) else {
            return Err(format!("Unknown app: {app}. `homebody status` lists them."));
        };
        match s(a, "fence") {
            "up" => return Ok(()),
            "checking" => tokio::time::sleep(Duration::from_millis(500)).await,
            _ => {
                let why: Vec<&str> = st["setupReasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let why = if why.is_empty() { s(a, "problem").to_string() } else { why.join(" ") };
                return Err(format!("{} cannot run protected on this machine yet. {why}\nRun once:  homebody setup --print | sudo bash", s(a, "name")));
            }
        }
    }
    Err("Protection could not be checked in time.".into())
}

/// Everything that has to be undone when the run ends, however it ends.
#[derive(Default)]
struct Undo {
    ask_was_on: bool,
    trusted: Vec<String>,
    engine: Option<Engine>,
}

async fn undo(app: &str, u: Undo) {
    for host in &u.trusted {
        let _ = call("remove_trust", json!({ "appId": app, "host": host })).await;
    }
    if u.ask_was_on {
        let _ = call("set_ask", json!({ "on": true })).await;
    }
    if let Some(engine) = u.engine {
        engine.shutdown().await;
        let _ = std::fs::remove_file(homebody_core::paths::control_socket());
    }
}

pub async fn run(args: &[String]) -> i32 {
    if args.is_empty() || args.iter().take_while(|a| *a != "--").any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return 0;
    }
    let o = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            return 2;
        }
    };
    let mut u = Undo::default();
    // Use the running app when there is one; otherwise be the engine for the length of this run.
    if !control::is_running().await {
        match crate::engine_here().await {
            Ok(e) => u.engine = Some(e),
            Err(e) => {
                eprintln!("{e}");
                return 4;
            }
        }
    }
    let code = match body(&o, &mut u).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("homebody: {e}");
            4
        }
    };
    undo(&o.app, u).await;
    code
}

async fn body(o: &Opts, u: &mut Undo) -> Result<i32, String> {
    ready(&o.app).await?;

    // Never ask: there is nobody to answer.
    let st = call("get_status", Value::Null).await?;
    if st["ask"].as_bool() == Some(true) {
        call("set_ask", json!({ "on": false })).await?;
        u.ask_was_on = true;
    }

    // Places for this run only.
    let mut wanted = o.trust.clone();
    if let Some(file) = &o.rules {
        wanted.extend(places_in(file)?);
    }
    let known = call("get_trust", json!({ "appId": o.app })).await?;
    for host in wanted {
        if known.as_array().into_iter().flatten().any(|t| s(t, "host").eq_ignore_ascii_case(&host)) {
            continue; // already decided on this machine, one way or the other: leave it alone
        }
        call("set_trust", json!({ "appId": o.app, "host": host, "enabled": true })).await.map_err(|e| format!("\"{host}\" cannot be trusted: {e}"))?;
        u.trusted.push(host);
    }

    // The project, as a copy.
    let mut project: Option<(String, String)> = None; // (name, copy)
    if let Some(folder) = &o.project {
        let source = std::fs::canonicalize(folder).map_err(|_| format!("The project folder {folder} could not be found."))?;
        let source = source.to_string_lossy().into_owned();
        let existing = call("get_projects", json!({ "appId": o.app })).await?;
        for p in existing.as_array().into_iter().flatten().filter(|p| s(p, "source") == source) {
            call("project_remove", json!({ "appId": o.app, "name": s(p, "name") })).await?;
        }
        let info = call("project_import", json!({ "appId": o.app, "path": source })).await?;
        project = Some((s(&info, "name").to_string(), s(&info, "copy").to_string()));
    }

    // Where the log stood before the run.
    let before = call("get_feed", json!({ "limit": 1 })).await?;
    let marker = before.as_array().and_then(|a| a.first()).and_then(|e| e["id"].as_u64()).unwrap_or(0);

    let mut env = serde_json::Map::new();
    for name in &o.env {
        let value = std::env::var(name).map_err(|_| format!("--env {name}: that variable is not set."))?;
        env.insert(name.clone(), Value::String(value));
    }
    let argv = call("fenced_argv", json!({ "appId": o.app, "program": o.command[0], "args": &o.command[1..], "cwd": project.as_ref().map(|p| p.1.clone()), "env": env })).await?;
    let argv: Vec<String> = argv.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
    let mut child = tokio::process::Command::new(&argv[0]).args(&argv[1..]).spawn().map_err(|e| format!("The command could not be started: {e}"))?;
    let code = tokio::select! {
        status = child.wait() => status.ok().and_then(|s| s.code()).unwrap_or(1),
        signal = crate::stop_signal() => { let _ = call("app_stop", json!({ "appId": o.app })).await; signal }
    };
    // Connections are closed out in the log a moment after the program ends.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let rows = call("get_feed", json!({ "limit": 2000, "appId": o.app })).await?;
    let mut events: Vec<Value> = rows.as_array().cloned().unwrap_or_default().into_iter().filter(|e| e["id"].as_u64().unwrap_or(0) > marker).collect();
    events.reverse();
    let list = places(&events);
    let refused: u64 = list.iter().map(|p| p.refused).sum();
    let verified = call("verify_chain", Value::Null).await?;

    if let (Some((name, _)), true) = (&project, o.apply) {
        let r = call("project_apply", json!({ "appId": o.app, "name": name, "overwrite": false })).await?;
        let left = r["skipped"].as_array().map(Vec::len).unwrap_or(0);
        eprintln!("homebody: {} changed file(s) put back into the project{}.", r["applied"], if left > 0 { format!(", {left} left alone because the project changed meanwhile") } else { String::new() });
    }

    if let (Some((name, _)), false) = (&project, o.keep_copy) {
        let _ = call("project_remove", json!({ "appId": o.app, "name": name })).await;
    }

    let md = markdown(&o.app, &o.command, code, &list, &verified);
    eprintln!("\n── Homebody ─────────────────────────────────────────");
    match refused {
        0 => eprintln!("Nothing was refused."),
        n => eprintln!("{n} attempt(s) to reach a place that is not trusted were refused:"),
    }
    for p in &list {
        if p.refused > 0 {
            eprintln!("  refused  {:<40} ×{}  {}", p.host, p.refused, p.why);
        } else {
            eprintln!("  allowed  {:<40} ×{}  {} sent", p.host, p.allowed, human(p.sent));
        }
    }
    if let Some(dir) = &o.report {
        let dir = std::path::absolute(dir).map_err(|e| format!("The report folder is not usable: {e}"))?;
        let r = call("export_log", json!({ "reveal": false, "dir": dir.to_string_lossy() })).await?;
        let summary = json!({
            "app": o.app, "command": o.command, "exitCode": code, "refused": refused,
            "places": list.iter().map(|p| json!({ "place": p.host, "label": p.label, "allowed": p.allowed, "refused": p.refused, "bytesSent": p.sent })).collect::<Vec<_>>(),
            "log": verified,
        });
        std::fs::write(dir.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap_or_default()).map_err(|e| format!("The summary could not be saved: {e}"))?;
        std::fs::write(dir.join("summary.md"), &md).map_err(|e| format!("The summary could not be saved: {e}"))?;
        eprintln!("Evidence saved to {}", s(&r, "path"));
    }
    // GitHub shows this file on the run's page.
    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
            use std::io::Write;
            let _ = f.write_all(md.as_bytes());
        }
    }
    Ok(if code == 0 && refused > 0 && o.fail_on_blocked { 3 } else { code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_come_before_the_double_dash_and_the_command_after() {
        let o = parse(&v(&["--trust", "pypi.org", "--project", ".", "--apply", "--fail-on-blocked", "claude-code", "--", "claude", "-p", "--trust"])).unwrap();
        assert_eq!((o.app.as_str(), o.trust, o.apply, o.fail_on_blocked), ("claude-code", v(&["pypi.org"]), true, true));
        assert_eq!(o.command, v(&["claude", "-p", "--trust"]));
    }

    #[test]
    fn a_run_without_an_app_or_a_command_is_refused() {
        assert!(parse(&v(&["claude-code"])).is_err());
        assert!(parse(&v(&["--", "true"])).is_err());
        assert!(parse(&v(&["--apply", "a", "--", "true"])).is_err());
        assert!(parse(&v(&["--nope", "a", "--", "true"])).is_err());
    }

    #[test]
    fn the_summary_puts_refusals_first_and_counts_them() {
        let events = vec![
            json!({ "host": "api.anthropic.com", "label": "Anthropic", "verb": "allowed", "bytesOut": 2048 }),
            json!({ "host": "evil.example", "label": "evil.example", "verb": "blocked", "explanation": "not trusted" }),
            json!({ "host": "evil.example", "label": "evil.example", "verb": "blocked", "explanation": "not trusted" }),
        ];
        let list = places(&events);
        assert_eq!((list[0].host.as_str(), list[0].refused), ("evil.example", 2));
        let md = markdown("claude-code", &v(&["claude"]), 0, &list, &json!({ "ok": true, "count": 9 }));
        assert!(md.contains("**2 attempts"));
        assert!(md.contains("| api.anthropic.com | 1 | 0 | 2.0 KB |"));
    }
}
