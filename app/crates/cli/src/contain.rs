//! Windows: the two small jobs behind `homebody run`.
//!
//! `__contain` runs *outside* the app's private space (as you). It makes sure the space and the
//! app's folders exist, starts the relay inside the space, starts the program inside the space,
//! waits for it, and stops the relay.
//!
//! `__relay` runs *inside* the space. It listens where the app expects its gate
//! (127.0.0.1:<port>, which inside an app container reaches only programs in the same
//! container) and passes every connection through the named pipe to the real gate outside.

use homebody_core::fence_windows::{self as win, Contained, ContainerSid};
use std::time::Duration;

fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub async fn contain(args: &[String]) -> i32 {
    let sep = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (opts, command) = (&args[..sep], args.get(sep + 1..).unwrap_or(&[]));
    let (Some(app), Some(port), Some(home), Some(cwd)) = (value(opts, "--app"), value(opts, "--port"), value(opts, "--home"), value(opts, "--cwd")) else {
        eprintln!("homebody: this command is used by `homebody run`.");
        return 2;
    };
    if command.is_empty() {
        eprintln!("homebody: nothing to run.");
        return 2;
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < opts.len() {
        if opts[i] == "--env" {
            if let Some((k, v)) = opts.get(i + 1).and_then(|kv| kv.split_once('=')) {
                env.push((k.to_string(), v.to_string()));
            }
            i += 1;
        }
        i += 1;
    }
    let run = || -> Result<i32, String> {
        let sid = ContainerSid::for_app(app)?;
        let sid_text = sid.text()?;
        win::ensure_dirs(home, value(opts, "--workspace"), &sid_text)?;
        // The relay is this same program, started inside the space, so the space has to be
        // allowed to read it.
        let me = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        win::grant(&me, &sid_text, false)?;
        let relay = Contained::spawn(&sid, &[me, "__relay".into(), port.to_string(), win::gate_pipe(app)], &win::base_env(home), home)?;
        // Give it a moment to start listening; the program's first connection needs it.
        std::thread::sleep(Duration::from_millis(500));
        let program = Contained::spawn(&sid, command, &env, cwd);
        let code = match program {
            Ok(p) => p.wait(),
            Err(e) => {
                relay.kill();
                return Err(e);
            }
        };
        relay.kill();
        Ok(code)
    };
    match tokio::task::spawn_blocking(run).await {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            eprintln!("homebody: {e}");
            4
        }
        Err(_) => 4,
    }
}

pub async fn relay(port: &str, pipe: &str) -> i32 {
    use tokio::net::windows::named_pipe::ClientOptions;
    let Ok(port) = port.parse::<u16>() else { return 2 };
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("homebody relay: could not listen on {port}: {e}");
            return 3;
        }
    };
    let pipe = pipe.to_string();
    loop {
        let Ok((mut conn, _)) = listener.accept().await else { continue };
        let pipe = pipe.clone();
        tokio::spawn(async move {
            // The pipe has one free end at a time; another is made the moment one is taken.
            for _ in 0..50 {
                match ClientOptions::new().open(&pipe) {
                    Ok(mut out) => {
                        let _ = tokio::io::copy_bidirectional(&mut conn, &mut out).await;
                        return;
                    }
                    Err(e) if e.raw_os_error() == Some(231) => tokio::time::sleep(Duration::from_millis(40)).await,
                    Err(e) => {
                        eprintln!("homebody relay: the gate could not be reached: {e}");
                        return;
                    }
                }
            }
        });
    }
}
