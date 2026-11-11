//! A local control socket, so the `sai` command line and scripts can drive a running engine.
//!
//! The socket lives in the user's private data folder (mode 0700), which protected apps cannot
//! enter, so a protected app cannot use it to approve its own requests. One JSON object per line:
//!
//! request:  `{"cmd": "set_mode", "args": {"mode": "private"}}`
//! response: `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`
//! `{"cmd": "subscribe"}` turns the connection into a stream of `{"event": {...}}` lines.

#![cfg(unix)]

use crate::engine::Engine;
use crate::paths;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub async fn is_running() -> bool {
    UnixStream::connect(paths::control_socket()).await.is_ok()
}

/// Serve until the process ends. Fails if another engine already owns the socket.
pub async fn serve(engine: Engine) -> std::io::Result<()> {
    let path = paths::control_socket();
    if is_running().await {
        return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "Homebody is already running."));
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| std::io::Error::new(e.kind(), format!("control socket at {}: {e}", path.display())))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    loop {
        let (stream, _) = listener.accept().await?;
        let engine = engine.clone();
        tokio::spawn(async move {
            let _ = client(engine, stream).await;
        });
    }
}

async fn client(engine: Engine, stream: UnixStream) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            write.write_all(b"{\"ok\":false,\"error\":\"not JSON\"}\n").await?;
            continue;
        };
        let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or("").to_string();
        if cmd == "subscribe" {
            let mut rx = engine.subscribe();
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let mut out = serde_json::to_vec(&json!({ "event": ev })).unwrap_or_default();
                        out.push(b'\n');
                        if write.write_all(&out).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return Ok(()),
                }
            }
        }
        let args = req.get("args").cloned().unwrap_or(Value::Null);
        let resp = match engine.dispatch(&cmd, args).await {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) => json!({ "ok": false, "error": error }),
        };
        let mut out = serde_json::to_vec(&resp).unwrap_or_default();
        out.push(b'\n');
        write.write_all(&out).await?;
    }
    Ok(())
}

/// Send one command to the running engine.
pub async fn call(cmd: &str, args: Value) -> Result<Value, String> {
    let stream = UnixStream::connect(paths::control_socket()).await.map_err(|_| "Homebody is not running. Open the app, or run `sai daemon`.".to_string())?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_vec(&json!({ "cmd": cmd, "args": args })).unwrap_or_default();
    line.push(b'\n');
    write.write_all(&line).await.map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(read).lines();
    let resp = lines.next_line().await.map_err(|e| e.to_string())?.ok_or("No answer from Homebody.")?;
    let v: Value = serde_json::from_str(&resp).map_err(|e| e.to_string())?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(v.get("error").and_then(Value::as_str).unwrap_or("Unknown error.").to_string())
    }
}

/// Stream events from the running engine to `on_event` until the connection ends.
pub async fn subscribe(mut on_event: impl FnMut(Value)) -> Result<(), String> {
    let stream = UnixStream::connect(paths::control_socket()).await.map_err(|_| "Homebody is not running.".to_string())?;
    let (read, mut write) = stream.into_split();
    write.write_all(b"{\"cmd\":\"subscribe\"}\n").await.map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if let Some(ev) = v.get("event") {
                on_event(ev.clone());
            }
        }
    }
    Ok(())
}
