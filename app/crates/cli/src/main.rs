//! `sai` — the Sandbox-ai command line.
//!
//! Talks to the running app over its private control socket, or runs the engine itself with
//! `sai daemon` when the app is not open. Same core, same rules, same log.

use sai_core::{control, Engine};
use serde_json::{json, Value};
use std::os::unix::process::CommandExt;

const HELP: &str = "sai — Sandbox-ai command line

  sai status                      what is protected and how it is going
  sai feed [-n N]                 recent connections
  sai watch                       follow events live
  sai mode private|watch|off      change mode
  sai lockdown on|off             block everything / release
  sai asks                        questions waiting for an answer
  sai allow <id> [--always]       answer a question: allow once, or always
  sai block <id>                  answer a question: block
  sai trust <app> <host>          always allow a destination for an app
  sai untrust <app> <host>        block a destination for an app
  sai trusted [app]               list trusted and blocked destinations
  sai proof                       the proof statement
  sai verify                      check the log has not been altered
  sai start|stop <app>            start or stop a protected model
  sai terminal <app>              open a protected terminal for an agent
  sai run <app> -- <command...>   run a command protected, in this terminal
  sai open [feed|proof|studio|glance]   show a window of the running app
  sai check                       re-check protection now
  sai setup                       one-time setup (asks for the administrator password)
  sai daemon                      run the engine without the app (foreground)
  sai call <command> [json]       send a raw command (see docs/ipc.md)

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
    let label = if s(e, "label") == s(e, "host") { s(e, "host").to_string() } else { format!("{} ({})", s(e, "label"), s(e, "host")) };
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
            eprintln!("sai daemon running. Ctrl-C to stop.");
            let _ = tokio::signal::ctrl_c().await;
            engine.shutdown().await;
            let _ = std::fs::remove_file(sai_core::paths::control_socket());
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
        "mode" => print_status(&call("set_mode", json!({ "mode": a(1) })).await),
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
            call("app_open_terminal", json!({ "appId": a(1) })).await;
        }
        "open" => {
            let (window, tab) = match a(1) {
                "glance" => ("glance", None),
                "prompt" => ("prompt", None),
                "" | "main" => ("main", None),
                tab => ("main", Some(tab)),
            };
            call("show", json!({ "window": window, "tab": tab })).await;
        }
        "check" => print_status(&call("check_fence", Value::Null).await),
        "setup" => {
            let r = call("run_setup", Value::Null).await;
            println!("{}", s(&r, "message"));
        }
        "run" => {
            let Some(sep) = args.iter().position(|x| x == "--") else {
                eprintln!("usage: sai run <app> -- <command...>");
                std::process::exit(1);
            };
            let Some(program) = args.get(sep + 1) else {
                eprintln!("usage: sai run <app> -- <command...>");
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

fn print_trust(list: &Value) {
    for t in list.as_array().into_iter().flatten() {
        println!("{:<12} {:<8} {:<48} {}", s(t, "appId"), if t["enabled"].as_bool() == Some(true) { "trusted" } else { "blocked" }, s(t, "host"), s(t, "purpose"));
    }
}
