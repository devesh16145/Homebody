//! A local control socket, so the `homebody` command line and scripts can drive a running engine.
//!
//! The socket lives in the user's private data folder (mode 0700), which protected apps cannot
//! enter, so a protected app cannot use it to approve its own requests. One JSON object per line:
//!
//! request:  `{"cmd": "set_mode", "args": {"mode": "private"}}`
//! response: `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`
//! `{"cmd": "subscribe"}` turns the connection into a stream of `{"event": {...}}` lines.
//!
//! On Windows the same conversation runs over a named pipe that only its creator (and the system
//! and administrators) can write to; a program in an app container cannot open it at all.

use crate::engine::Engine;
use crate::paths;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[cfg(unix)]
mod os {
    use super::*;
    pub use tokio::net::UnixStream as Stream;
    use tokio::net::UnixListener;

    pub async fn connect() -> std::io::Result<Stream> {
        Stream::connect(paths::control_socket()).await
    }

    pub async fn serve(engine: Engine) -> std::io::Result<()> {
        let path = paths::control_socket();
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
}

#[cfg(windows)]
mod os {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    pub use tokio::net::windows::named_pipe::NamedPipeClient as Stream;

    /// One pipe per data folder, so a development copy and the real one do not meet.
    pub fn pipe_name() -> String {
        use sha2::Digest;
        let tag = hex::encode(&sha2::Sha256::digest(paths::data_dir().to_string_lossy().to_lowercase().as_bytes())[..6]);
        format!(r"\\.\pipe\homebody-control-{tag}")
    }

    pub async fn connect() -> std::io::Result<Stream> {
        ClientOptions::new().open(pipe_name())
    }

    pub async fn serve(engine: Engine) -> std::io::Result<()> {
        let name = pipe_name();
        let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
        loop {
            server.connect().await?;
            let stream = std::mem::replace(&mut server, ServerOptions::new().reject_remote_clients(true).create(&name)?);
            let engine = engine.clone();
            tokio::spawn(async move {
                let _ = client(engine, stream).await;
            });
        }
    }
}

pub async fn is_running() -> bool {
    os::connect().await.is_ok()
}

/// Serve until the process ends. Fails if another engine already owns the socket.
pub async fn serve(engine: Engine) -> std::io::Result<()> {
    if is_running().await {
        return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "Homebody is already running."));
    }
    os::serve(engine).await
}

async fn client<S: AsyncRead + AsyncWrite + Unpin>(engine: Engine, stream: S) -> std::io::Result<()> {
    let (read, mut write) = tokio::io::split(stream);
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
    let stream = os::connect().await.map_err(|_| "Homebody is not running. Open the app, or run `homebody daemon`.".to_string())?;
    let (read, mut write) = tokio::io::split(stream);
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
    let stream = os::connect().await.map_err(|_| "Homebody is not running.".to_string())?;
    let (read, mut write) = tokio::io::split(stream);
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
