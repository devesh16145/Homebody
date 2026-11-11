//! `homebody`: the Homebody command line.
//!
//! Talks to the running app over its private control socket, or runs the engine itself with
//! `homebody daemon` when the app is not open. Same core, same rules, same log.

use homebody_core::{control, Engine};
use serde_json::{json, Value};
use std::os::unix::process::CommandExt;

const HELP: &str = "homebody: the Homebody command line

  homebody status                      what is protected and how it is going
  homebody feed [-n N]                 recent connections
  homebody watch                       follow events live
  homebody mode private|watch|off      change mode
  homebody lockdown on|off             block everything / release
  homebody asks                        questions waiting for an answer
  homebody allow <id> [--always]       answer a question: allow once, or always
  homebody block <id>                  answer a question: block
  homebody trust <app> <host>          always allow a destination for an app
  homebody untrust <app> <host>        block a destination for an app
  homebody trusted [app]               list trusted and blocked destinations
  homebody proof                       the proof statement
  homebody verify [file]               check the log (or an exported log file) has not been altered
  homebody export-log                  save the whole signed log to Downloads
  homebody mode watch [minutes]        Watch for a while (default 60; 0 = until switched off)
  homebody watched                     places contacted under Watch that are not decided yet
  homebody start|stop <app>            start or stop a protected model
  homebody terminal <app> [project]    open a protected terminal for an agent, optionally in a project
  homebody project list <app>          projects brought in for an agent
  homebody project add <app> <folder>  bring a folder in: the agent works on a copy
  homebody project changes <app> <name>   what the agent changed in its copy
  homebody project apply <app> <name> [--replace-mine]   put those changes into your folder
  homebody project remove <app> <name>    delete the copy (your folder is not touched)
  homebody run <app> -- <command...>   run a command protected, in this terminal
  homebody open [feed|proof|studio|glance]   show a window of the running app
  homebody inspect <app> on|off        look inside an app's connections to trusted places
  homebody secrets <app> on|off        while looking inside, stop requests that carry a secret
  homebody inside [app]                switches for addresses inside trusted places
  homebody ask on|off                  ask about new places, or block them without asking
  homebody limit <app> <place> <MB|off>   megabytes an app may send to a place per hour
  homebody private [add|remove <folder>]  folders whose contents must never be sent
  homebody summary [days]              what each app did lately, and what was stopped
  homebody timestamp                   have a public service vouch for when the log existed
  homebody update                      ask whether a newer version has been published
  homebody inside <app> <place> <path> on|off   allow or refuse one address inside a place
  homebody found                       AI apps on this computer that are not protected yet
  homebody open-protected <app>        open an app protected (what a launcher does)
  homebody check                       re-check protection now
  homebody setup                       one-time setup (asks for the administrator password)
  homebody daemon                      run the engine without the app (foreground)
  homebody call <command> [json]       send a raw command (see docs/ipc.md)

Apps: claude-code, ollama";

fn human_bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn print_event_line(e: &Value) {
    let verb = match s(e, "verb") {
        "allowed" => "Allowed",
        "blocked" => "Blocked",
        "asked" => "Asked  ",
        "seen" => "Seen   ",
        other => other,
    };
    let bytes = e.get("bytesOut").and_then(Value::as_u64).map(|b| format!("  {} out", human_bytes(b))).unwrap_or_default();
    let label = if s(e, "label") == s(e, "host") || s(e, "host").is_empty() { s(e, "label").to_string() } else { format!("{} ({})", s(e, "label"), s(e, "host")) };
    println!("{}  {}  {:<12} {}{}  — {}", s(e, "ts").get(11..19).unwrap_or(""), verb, s(e, "appName"), label, bytes, s(e, "explanation"));
}

fn print_status(st: &Value) {
    println!("{}", s(st, "sentence"));
    println!("mode: {}   fence: {}   lockdown: {}", s(st, "mode"), s(st, "fence"), st.get("lockdown").and_then(Value::as_bool).unwrap_or(false));
    if st.get("setupNeeded").and_then(Value::as_bool).unwrap_or(false) {
        println!("setup needed:");
        for r in st.get("setupReasons").and_then(Value::as_array).into_iter().flatten() {
            println!("  - {}", r.as_str().unwrap_or(""));
        }
    }
    let t = &st["today"];
    println!("today: {} allowed, {} blocked, {} asked, {} sent", t["allowed"], t["blocked"], t["asked"], human_bytes(t["bytesOut"].as_u64().unwrap_or(0)));
    for a in st.get("apps").and_then(Value::as_array).into_iter().flatten() {
        println!("  {:<12} installed={} protected={} fence={} running={}", s(a, "name"), a["installed"], a["protected"], s(a, "fence"), a["running"]);
    }
    for a in st.get("pending").and_then(Value::as_array).into_iter().flatten() {
        println!("  ? [{}] {} — {}", s(a, "id"), s(a, "title"), s(a, "body"));
    }
}

async fn call(cmd: &str, args: Value) -> Value {
    match control::call(cmd, args).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    match a(0) {
        "" | "help" | "-h" | "--help" => println!("{HELP}"),
        "daemon" => {
            let engine = match Engine::start().await {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            let server = engine.clone();
            let control_task = tokio::spawn(async move { control::serve(server).await });
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if control_task.is_finished() {
                match control_task.await {
                    Ok(Err(e)) => eprintln!("Could not start: {e}"),
                    _ => eprintln!("Could not start the control socket."),
                }
                std::process::exit(1);
            }
            eprintln!("homebody daemon running. Ctrl-C to stop.");
            // Stop cleanly however we are asked to: Ctrl-C, or a plain kill, logout or shutdown.
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            engine.shutdown().await;
            let _ = std::fs::remove_file(homebody_core::paths::control_socket());
        }
        "status" => {
            let st = call("get_status", Value::Null).await;
            if args.iter().any(|x| x == "--json") {
                println!("{}", serde_json::to_string_pretty(&st).unwrap());
            } else {
                print_status(&st);
            }
        }
        "feed" => {
            let n = args.iter().position(|x| x == "-n").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok()).unwrap_or(20);
            let rows = call("get_feed", json!({ "limit": n })).await;
            for e in rows.as_array().into_iter().flatten().rev() {
                print_event_line(e);
            }
        }
        "watch" => {
            let r = control::subscribe(|ev| match s(&ev, "type") {
                "feed" => print_event_line(&ev["data"]),
                "ask" => println!("?  [{}] {} — {}", s(&ev["data"], "id"), s(&ev["data"], "title"), s(&ev["data"], "body")),
                "ask_resolved" => println!("=  [{}] answered: {}", s(&ev["data"], "id"), s(&ev["data"], "choice")),
                "status" => println!("·  {}", s(&ev["data"], "sentence")),
                _ => {}
            })
            .await;
            if let Err(e) = r {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        "mode" => {
            let minutes = a(2).parse::<u64>().ok();
            print_status(&call("set_mode", json!({ "mode": a(1), "minutes": minutes })).await)
        }
        "watched" => {
            let r = call("get_watch_review", Value::Null).await;
            let items = r["items"].as_array().cloned().unwrap_or_default();
            if r.is_null() || items.is_empty() {
                println!("Nothing new was contacted under Watch.");
            }
            for i in &items {
                println!("{:<12} {:<44} {} time(s), {} sent  {}", s(i, "appName"), s(i, "host"), i["times"], human_bytes(i["bytesOut"].as_u64().unwrap_or(0)), s(i, "purpose"));
            }
        }
        "lockdown" => print_status(&call("set_lockdown", json!({ "on": a(1) == "on" })).await),
        "asks" => {
            let st = call("get_status", Value::Null).await;
            let pending = st.get("pending").and_then(Value::as_array).cloned().unwrap_or_default();
            if pending.is_empty() {
                println!("Nothing is waiting for an answer.");
            }
            for p in pending {
                println!("[{}] {} — {}", s(&p, "id"), s(&p, "title"), s(&p, "body"));
            }
        }
        "allow" => {
            let choice = if args.iter().any(|x| x == "--always") { "always" } else { "once" };
            call("decide", json!({ "askId": a(1), "choice": choice })).await;
        }
        "block" => {
            call("decide", json!({ "askId": a(1), "choice": "block" })).await;
        }
        "trust" | "untrust" => {
            let list = call("set_trust", json!({ "appId": a(1), "host": a(2), "enabled": a(0) == "trust" })).await;
            print_trust(&list);
        }
        "trusted" => {
            let arg = if a(1).is_empty() { Value::Null } else { json!({ "appId": a(1) }) };
            print_trust(&call("get_trust", arg).await);
        }
        "proof" => {
            let p = call("get_proof", Value::Null).await;
            println!("{}\n", s(&p, "statement"));
            println!("private for: {}   blocked: {}   allowed: {}", s(&p, "streakText"), p["blockedTotal"], p["allowedTotal"]);
            println!("log key: {}", s(&p, "publicKey"));
        }
        "verify" if !a(1).is_empty() => {
            // A file exported with "Export": checked here, without the running app. Given the
            // export folder, the log inside it is the thing to check.
            let given = std::path::Path::new(a(1));
            let file = if given.is_dir() { given.join("log.jsonl") } else { given.to_path_buf() };
            let (c, key) = homebody_core::ledger::verify_export(&file);
            if let Some(k) = key {
                println!("signed with key: {k}");
            }
            if c.ok {
                println!("OK: {} records, chain intact, all signatures valid.", c.count);
                println!("Compare the key above with the one on the Report screen (or `homebody proof`) to know whose log this is.");
            } else {
                println!("FAILED after {} records: {}", c.count, c.error.unwrap_or_default());
                std::process::exit(2);
            }
        }
        "export-log" => {
            let r = call("export_log", json!({ "reveal": false })).await;
            println!("{}", s(&r, "path"));
        }
        "verify" => {
            let c = call("verify_chain", Value::Null).await;
            if c["ok"].as_bool() == Some(true) {
                println!("OK: {} records, chain intact, all signatures valid.", c["count"]);
            } else {
                println!("FAILED after {} records: {}", c["count"], s(&c, "error"));
                std::process::exit(2);
            }
        }
        "start" => print_status(&call("app_start", json!({ "appId": a(1) })).await),
        "stop" => print_status(&call("app_stop", json!({ "appId": a(1) })).await),
        "terminal" => {
            let project = if a(2).is_empty() { Value::Null } else { json!(a(2)) };
            call("app_open_terminal", json!({ "appId": a(1), "project": project })).await;
        }
        "project" => match a(1) {
            "list" | "" => {
                let app = if a(2).is_empty() { "claude-code" } else { a(2) };
                let list = call("get_projects", json!({ "appId": app })).await;
                let list = list.as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    println!("No projects brought in for {app}.");
                }
                for p in &list {
                    println!("{:<24} from {}\n{:<24} copy {}", s(p, "name"), s(p, "source"), "", s(p, "copy"));
                }
            }
            "add" => {
                let folder = std::fs::canonicalize(a(3)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| a(3).to_string());
                let p = call("project_import", json!({ "appId": a(2), "path": folder })).await;
                println!("Brought in as \"{}\". The agent works in {}", s(&p, "name"), s(&p, "copy"));
            }
            "changes" => print_changes(&call("project_changes", json!({ "appId": a(2), "name": a(3) })).await),
            "apply" => {
                let r = call("project_apply", json!({ "appId": a(2), "name": a(3), "overwrite": args.iter().any(|x| x == "--replace-mine") })).await;
                println!("{} applied to your folder.", r["applied"]);
                for path in r["skipped"].as_array().into_iter().flatten() {
                    println!("  left alone (you changed it too): {}", path.as_str().unwrap_or(""));
                }
            }
            "remove" => {
                call("project_remove", json!({ "appId": a(2), "name": a(3) })).await;
                println!("Copy removed. Your own folder was not touched.");
            }
            other => {
                eprintln!("Unknown project command: {other}\n\n{HELP}");
                std::process::exit(1);
            }
        },
        "open" => {
            let (window, tab) = match a(1) {
                "glance" => ("glance", None),
                "prompt" => ("prompt", None),
                "" | "main" => ("main", None),
                tab => ("main", Some(tab)),
            };
            call("show", json!({ "window": window, "tab": tab })).await;
        }
        "inspect" => {
            let st = call("app_set_inspect", json!({ "appId": a(1), "on": a(2) != "off" })).await;
            for app in st["apps"].as_array().into_iter().flatten() {
                if s(app, "id") == a(1) {
                    println!("{}: looking inside is {}", s(app, "name"), if app["inspect"].as_bool() == Some(true) { "on" } else { "off" });
                }
            }
        }
        "secrets" => {
            let st = call("app_set_secrets", json!({ "appId": a(1), "on": a(2) != "off" })).await;
            for app in st["apps"].as_array().into_iter().flatten() {
                if s(app, "id") == a(1) {
                    println!("{}: stopping secrets is {}", s(app, "name"), if app["secrets"].as_bool() == Some(true) { "on" } else { "off" });
                }
            }
        }
        "ask" => print_status(&call("set_ask", json!({ "on": a(1) != "off" })).await),
        "limit" => {
            let mb = a(3).parse::<u64>().ok();
            print_trust(&call("set_limit", json!({ "appId": a(1), "host": a(2), "mb": mb })).await);
        }
        "private" => {
            let r = match a(1) {
                "add" => call("add_private_folder", json!({ "path": std::fs::canonicalize(a(2)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| a(2).to_string()) })).await,
                "remove" => call("remove_private_folder", json!({ "path": a(2) })).await,
                _ => call("get_private_folders", Value::Null).await,
            };
            for f in r["folders"].as_array().into_iter().flatten() {
                println!("{}{}", s(f, "path"), if f["exists"].as_bool() == Some(true) { "" } else { "  (missing)" });
            }
            println!("{} files, {} lines of text recognised", r["files"], r["lines"]);
        }
        "timestamp" => {
            let r = call("timestamp_log", Value::Null).await;
            println!("Timestamped {} records at {} by {}\n{}", r["records"], s(&r, "time"), s(&r, "service"), s(&r, "file"));
        }
        "update" => println!("{}", s(&call("check_update", Value::Null).await, "message")),
        "summary" => {
            let r = call("get_summary", json!({ "days": a(1).parse::<u64>().unwrap_or(7) })).await;
            println!("The last {} days", r["days"]);
            for app in r["apps"].as_array().into_iter().flatten() {
                println!("\n{}: {} allowed, {} blocked, {} sent", s(app, "appName"), app["allowed"], app["blocked"], human_bytes(app["bytesOut"].as_u64().unwrap_or(0)));
                for p in app["places"].as_array().into_iter().flatten() {
                    println!("   reached {:<32} {} times, {} sent", s(p, "name"), p["times"], human_bytes(p["bytesOut"].as_u64().unwrap_or(0)));
                }
                for p in app["blockedPlaces"].as_array().into_iter().flatten() {
                    println!("   blocked {:<32} {} times", s(p, "name"), p["times"]);
                }
                let (g, sec, ad) = (app["aroundGate"].as_u64().unwrap_or(0), app["secretsStopped"].as_u64().unwrap_or(0), app["addressesRefused"].as_u64().unwrap_or(0));
                if g + sec + ad > 0 {
                    println!("   {g} went around the gate, {sec} carried a secret, {ad} were for an address that is switched off");
                }
            }
        }
        "inside" => {
            let list = if a(2).is_empty() {
                call("get_inside_rules", if a(1).is_empty() { Value::Null } else { json!({ "appId": a(1) }) }).await
            } else {
                call("set_inside_rule", json!({ "appId": a(1), "host": a(2), "path": a(3), "enabled": a(4) != "off" })).await
            };
            for r in list.as_array().into_iter().flatten() {
                println!("{:<12} {:<8} {}{}  {}", s(r, "appId"), if r["enabled"].as_bool() == Some(true) { "allowed" } else { "refused" }, s(r, "host"), s(r, "path"), s(r, "label"));
            }
        }
        "found" => {
            for f in call("discover_apps", Value::Null).await.as_array().into_iter().flatten() {
                let how = if f["canProtect"].as_bool() != Some(true) { "cannot be protected" } else if f["asks"].as_bool() == Some(true) { "can be protected" } else { "offline only" };
                println!("{:<16} {:<20} {}", s(f, "name"), how, s(f, "path"));
            }
        }
        "--open-protected" | "open-protected" => {
            call("app_open", json!({ "appId": a(1) })).await;
        }
        "check" => print_status(&call("check_fence", Value::Null).await),
        "setup" => {
            let r = call("run_setup", Value::Null).await;
            println!("{}", s(&r, "message"));
        }
        "run" => {
            let Some(sep) = args.iter().position(|x| x == "--") else {
                eprintln!("usage: homebody run <app> -- <command...>");
                std::process::exit(1);
            };
            let Some(program) = args.get(sep + 1) else {
                eprintln!("usage: homebody run <app> -- <command...>");
                std::process::exit(1);
            };
            let cwd = args.iter().position(|x| x == "--cwd").filter(|i| *i < sep).and_then(|i| args.get(i + 1)).cloned();
            let argv = call("fenced_argv", json!({ "appId": a(1), "program": program, "args": &args[sep + 2..], "cwd": cwd })).await;
            let argv: Vec<String> = argv.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
            let err = std::process::Command::new(&argv[0]).args(&argv[1..]).current_dir("/").exec();
            eprintln!("could not run: {err}");
            std::process::exit(1);
        }
        "call" => {
            let payload = if a(2).is_empty() { Value::Null } else { serde_json::from_str(a(2)).unwrap_or(Value::Null) };
            match control::call(a(1), payload).await {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("Unknown command: {other}\n\n{HELP}");
            std::process::exit(1);
        }
    }
}

fn print_changes(set: &Value) {
    let total = set["total"].as_u64().unwrap_or(0);
    if total == 0 {
        println!("No changes waiting.");
        return;
    }
    for c in set["items"].as_array().into_iter().flatten() {
        let link = c.get("link").and_then(Value::as_str).map(|l| format!("  -> {l}")).unwrap_or_default();
        let conflict = if c["conflict"].as_bool() == Some(true) { "  (you changed this too)" } else { "" };
        println!("{:<9} {}{}{}", s(c, "kind"), s(c, "path"), link, conflict);
    }
    if set["truncated"].as_bool() == Some(true) {
        println!("… showing the first {} of {total}", set["items"].as_array().map(Vec::len).unwrap_or(0));
    }
    println!("{total} waiting, {} also changed by you", set["conflicts"]);
}

fn print_trust(list: &Value) {
    for t in list.as_array().into_iter().flatten() {
        let limit = t["limitMb"].as_u64().map(|m| format!("  [at most {m} MB an hour, {} sent]", t["sentMb"])).unwrap_or_default();
        println!("{:<12} {:<8} {:<48} {}{limit}", s(t, "appId"), if t["enabled"].as_bool() == Some(true) { "trusted" } else { "blocked" }, s(t, "host"), s(t, "purpose"));
    }
}
