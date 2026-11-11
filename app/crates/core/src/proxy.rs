//! The gate: a small local HTTP proxy, one listener per protected app.
//!
//! A protected app can reach nothing but its own gate. The gate reads where the app wants to go,
//! asks the engine, resolves the name itself (the app has no name lookups), refuses anything that
//! resolves to this computer or the local network, and relays bytes while counting them.
//!
//! Two request shapes are understood: `CONNECT host:port` (encrypted traffic, passed through
//! untouched) and absolute-form plain HTTP (`GET http://host/path`). For encrypted traffic the
//! gate checks that the name in the TLS hello is the name that was approved, which closes the
//! simplest disguise: asking for a trusted name and then greeting the server as another.

use crate::engine::{Decision, Engine};
use crate::hostname;
use crate::model::Reason;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const HEAD_LIMIT: usize = 64 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await
}

const REFUSAL_PAUSE: Duration = Duration::from_millis(250);

/// Serve one gate. Which app the gate belongs to is looked up for every connection, so a gate
/// keeps working (or goes quiet) as apps are added and removed while the engine runs.
pub async fn serve(engine: Engine, port: u16, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // A gate that belongs to no app answers nobody.
                let Some(a) = engine.app_for_gate(port) else { continue };
                let e = engine.clone();
                tokio::spawn(async move {
                    let _ = handle(e, a, stream).await;
                });
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub connect: bool,
    pub host: String,
    pub port: u16,
    /// For plain HTTP: the request head rewritten for the origin server.
    pub forward: Vec<u8>,
}

/// Parse a proxy request head. Anything unusual is an error: the gate is not lenient.
pub(crate) fn parse_request(head: &str) -> Result<Request, &'static str> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or("empty request")?;
    let mut parts = request_line.split(' ');
    let (method, target, version) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if parts.next().is_some() || method.is_empty() || target.is_empty() || !version.starts_with("HTTP/1.") {
        return Err("bad request line");
    }
    if !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("bad method");
    }
    if method == "CONNECT" {
        let (host, port) = split_host_port(target, 443)?;
        return Ok(Request { connect: true, host, port, forward: Vec::new() });
    }
    let rest = target.strip_prefix("http://").ok_or("only CONNECT and http:// requests are possible")?;
    let cut = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let (host, port) = split_host_port(authority, 80)?;
    let path = if tail.is_empty() { "/".to_string() } else if tail.starts_with('/') { tail.to_string() } else { format!("/{tail}") };
    // Rebuild the head for the origin: our own Host (so a trusted name cannot front for another),
    // no proxy headers, one request per connection.
    let mut out = format!("{method} {path} {version}\r\nHost: {}\r\n", if port == 80 { host.clone() } else { format!("{host}:{port}") });
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if name == "host" || name == "connection" || name == "keep-alive" || name.starts_with("proxy-") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    Ok(Request { connect: false, host, port, forward: out.into_bytes() })
}

/// Split `host:port`. The host is returned exactly as given: judging it is the engine's job, so
/// that every odd name is recorded rather than silently dropped here.
fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16), &'static str> {
    if authority.is_empty() {
        return Err("no destination");
    }
    if authority.starts_with('[') {
        // IPv6 literal: hand the whole thing over so it is recorded and refused as an address.
        return Ok((authority.to_string(), default_port));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => {
            let port: u16 = p.parse().map_err(|_| "bad port")?;
            if port == 0 {
                return Err("bad port");
            }
            Ok((h.to_string(), port))
        }
        Some(_) => Ok((authority.to_string(), default_port)),
        None => Ok((authority.to_string(), default_port)),
    }
}

/// The server name in a TLS ClientHello, if `buf` holds one complete hello with that extension.
/// `Err` means "this is not a TLS hello we understand".
pub(crate) fn client_hello_sni(buf: &[u8]) -> Result<Option<String>, ()> {
    // Record header: type 22 (handshake), version, length.
    if buf.len() < 5 || buf[0] != 22 || buf[1] != 3 {
        return Err(());
    }
    let rec_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
    let rec = buf.get(5..5 + rec_len).ok_or(())?;
    // Handshake header: type 1 (ClientHello), 3-byte length.
    if rec.len() < 4 || rec[0] != 1 {
        return Err(());
    }
    let hs_len = ((rec[1] as usize) << 16) | ((rec[2] as usize) << 8) | rec[3] as usize;
    let body = rec.get(4..4 + hs_len).ok_or(())?;
    let mut p = 2 + 32; // version + random
    let take = |p: &mut usize, n: usize| -> Result<&[u8], ()> {
        let s = body.get(*p..*p + n).ok_or(())?;
        *p += n;
        Ok(s)
    };
    let sid = *body.get(p).ok_or(())? as usize;
    p += 1;
    take(&mut p, sid)?;
    let cs = u16::from_be_bytes(take(&mut p, 2)?.try_into().unwrap()) as usize;
    take(&mut p, cs)?;
    let cm = *body.get(p).ok_or(())? as usize;
    p += 1;
    take(&mut p, cm)?;
    if p == body.len() {
        return Ok(None);
    }
    let ext_total = u16::from_be_bytes(take(&mut p, 2)?.try_into().unwrap()) as usize;
    let exts = take(&mut p, ext_total)?;
    let mut q = 0;
    while q + 4 <= exts.len() {
        let typ = u16::from_be_bytes([exts[q], exts[q + 1]]);
        let len = u16::from_be_bytes([exts[q + 2], exts[q + 3]]) as usize;
        let data = exts.get(q + 4..q + 4 + len).ok_or(())?;
        q += 4 + len;
        if typ == 0 {
            // server_name: list length, then entries of (type, length, name). Type 0 is a host name.
            if data.len() < 5 || data[2] != 0 {
                return Err(());
            }
            let n = u16::from_be_bytes([data[3], data[4]]) as usize;
            let name = data.get(5..5 + n).ok_or(())?;
            return std::str::from_utf8(name).map(|s| Some(s.to_string())).map_err(|_| ());
        }
    }
    Ok(None)
}

async fn respond(client: &mut TcpStream, code: u16, text: &str) {
    let reason = match code {
        403 => "Forbidden",
        502 => "Bad Gateway",
        _ => "Bad Request",
    };
    let body = format!("{text}\n");
    let msg = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = client.write_all(msg.as_bytes()).await;
    let _ = client.shutdown().await;
}

async fn read_head(client: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    loop {
        let n = client.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = std::str::from_utf8(&buf[..pos]).ok()?.to_string();
            return Some((head, buf[pos + 4..].to_vec()));
        }
        if buf.len() > HEAD_LIMIT {
            return None;
        }
    }
}

/// Resolve on the host and connect to the first public address that answers.
/// `Err(true)` means the name points only at this computer or the local network.
async fn connect_public(host: &str, port: u16) -> Result<(TcpStream, SocketAddr), bool> {
    let addrs: Vec<SocketAddr> = match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host, port))).await {
        Ok(Ok(a)) => a.collect(),
        _ => return Err(false),
    };
    if addrs.is_empty() {
        return Err(false);
    }
    let public: Vec<SocketAddr> = addrs.iter().copied().filter(|a| hostname::is_public(a.ip())).collect();
    if public.is_empty() {
        return Err(true);
    }
    for addr in public {
        if let Ok(Ok(s)) = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            let _ = s.set_nodelay(true);
            return Ok((s, addr));
        }
    }
    Err(false)
}

async fn handle(engine: Engine, app_id: String, mut client: TcpStream) -> std::io::Result<()> {
    let _ = client.set_nodelay(true);
    let Ok(Some((head, leftover))) = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client)).await else { return Ok(()) };
    let req = match parse_request(&head) {
        Ok(r) => r,
        Err(why) => {
            respond(&mut client, 400, &format!("Sandbox-ai could not understand this request ({why}).")).await;
            return Ok(());
        }
    };

    let decision: Decision = engine.decide_conn(&app_id, &req.host, req.port).await;
    if !decision.allow {
        // Some apps retry a refused request in a tight loop (thousands of times a minute).
        // A short pause before saying no costs a real user nothing and takes the heat out of it.
        tokio::time::sleep(REFUSAL_PAUSE).await;
        respond(&mut client, 403, &decision.message).await;
        return Ok(());
    }
    let host = decision.host.clone().unwrap_or_else(|| req.host.clone());

    let started = Instant::now();
    let (mut upstream, addr) = match connect_public(&host, req.port).await {
        Ok(x) => x,
        Err(true) => {
            engine.record_local_address(&app_id, &host, req.port);
            respond(&mut client, 403, "Blocked by Sandbox-ai: this name points at your own computer or network.").await;
            return Ok(());
        }
        Err(false) => {
            if let Some(conn) = engine.record_conn(&app_id, &host, "", req.port, &decision) {
                engine.record_end(conn, 0, 0, started.elapsed().as_millis() as u64, true);
            }
            respond(&mut client, 502, &format!("Sandbox-ai allowed this, but {host} did not answer.")).await;
            return Ok(());
        }
    };

    let mut first: Vec<u8> = Vec::new();
    if req.connect {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        first = leftover;
        if req.port == 443 {
            // The name the app greets the server with must be the name that was approved.
            match tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut client, &mut first)).await {
                Ok(Ok(Some(sni))) if hostname::canonical(&sni).as_deref() == Some(host.as_str()) => {}
                Ok(Ok(Some(sni))) => {
                    let shown: String = sni.chars().filter(|c| !c.is_control()).take(80).collect();
                    engine.record_refusal(&app_id, &host, req.port, Reason::Malformed, "greeting", &shown);
                    return Ok(());
                }
                Ok(Ok(None)) => {
                    engine.record_refusal(&app_id, &host, req.port, Reason::Malformed, "no_name", "");
                    return Ok(());
                }
                _ => {
                    engine.record_refusal(&app_id, &host, req.port, Reason::Malformed, "not_encrypted", "");
                    return Ok(());
                }
            }
        }
    } else {
        first.extend_from_slice(&req.forward);
        first.extend_from_slice(&leftover);
    }

    if req.connect && req.port == 443 {
        if let Some(inspector) = engine.inspector_for(&app_id) {
            inspected(&engine, &app_id, &host, &decision, inspector, client, first, upstream, addr, started).await;
            return Ok(());
        }
    }

    let conn = engine.record_conn(&app_id, &host, &addr.ip().to_string(), req.port, &decision);
    let (out, inn) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    if !first.is_empty() {
        upstream.write_all(&first).await?;
        out.fetch_add(first.len() as u64, Ordering::Relaxed);
    }
    relay(&engine, client, upstream, out.clone(), inn.clone()).await;
    if let Some(conn) = conn {
        engine.record_end(conn, out.load(Ordering::Relaxed), inn.load(Ordering::Relaxed), started.elapsed().as_millis() as u64, false);
    }
    Ok(())
}

/// A stream that first gives back bytes already read from it. The app's first message was read
/// to check the name it greets the server with; the encryption layer needs to see it again.
struct Rewound {
    first: Vec<u8>,
    at: usize,
    inner: TcpStream,
}

impl tokio::io::AsyncRead for Rewound {
    fn poll_read(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        if self.at < self.first.len() {
            let n = (self.first.len() - self.at).min(buf.remaining());
            let (from, to) = (self.at, self.at + n);
            buf.put_slice(&self.first[from..to]);
            self.at = to;
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for Rewound {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// One connection, looked inside. The app's encryption ends here with a certificate from the
/// local authority; ours to the real server starts here and is checked the ordinary way; and
/// in between the request lines are read (see `inspect::Sniffer`). Anything that cannot be
/// followed, or that names another site, ends the connection.
#[allow(clippy::too_many_arguments)]
async fn inspected(engine: &Engine, app_id: &str, host: &str, decision: &Decision, inspector: Arc<crate::inspect::Inspector>, client: TcpStream, first: Vec<u8>, upstream: TcpStream, addr: SocketAddr, started: Instant) {
    let Ok(server_config) = inspector.server_config(host) else { return };
    let Ok(name) = rustls::pki_types::ServerName::try_from(host.to_string()) else { return };
    // Our own connection first: if the real server's certificate does not check out, the app
    // must not be told everything is fine.
    let upstream = match tokio::time::timeout(HELLO_TIMEOUT, tokio_rustls::TlsConnector::from(inspector.client_config()).connect(name, upstream)).await {
        Ok(Ok(s)) => s,
        _ => {
            engine.record_refusal(app_id, host, 443, Reason::Malformed, "real_certificate", "");
            return;
        }
    };
    let client = match tokio::time::timeout(HELLO_TIMEOUT, tokio_rustls::TlsAcceptor::from(server_config).accept(Rewound { first, at: 0, inner: client })).await {
        Ok(Ok(s)) => s,
        _ => {
            engine.record_refusal(app_id, host, 443, Reason::Malformed, "no_inspect", "");
            return;
        }
    };

    let conn = engine.record_conn(app_id, host, &addr.ip().to_string(), 443, decision);
    let (out, inn) = (AtomicU64::new(0), AtomicU64::new(0));
    let sniffer = std::sync::Mutex::new(crate::inspect::Sniffer::new(host));
    let trouble: std::sync::Mutex<Option<crate::inspect::Trouble>> = std::sync::Mutex::new(None);
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = tokio::io::split(upstream);
    let up = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match cr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Read before it is passed on: what cannot be vouched for does not leave.
                    if let Err(t) = sniffer.lock().unwrap().feed(&buf[..n]) {
                        *trouble.lock().unwrap() = Some(t);
                        return true;
                    }
                    if uw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    out.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = uw.shutdown().await;
        false
    };
    let down = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match ur.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    inn.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = cw.shutdown().await;
    };
    tokio::pin!(up, down);
    let mut cut = engine.cut_signal();
    tokio::select! {
        stopped = &mut up => { if !stopped { let _ = tokio::time::timeout(Duration::from_secs(5), &mut down).await; } }
        _ = &mut down => { let _ = tokio::time::timeout(Duration::from_secs(2), &mut up).await; }
        _ = cut.changed() => {}
    }
    if let Some(conn) = conn {
        let summary = sniffer.lock().unwrap().summary();
        engine.record_end_inspected(conn, out.load(Ordering::Relaxed), inn.load(Ordering::Relaxed), started.elapsed().as_millis() as u64, &summary);
    }
    let found = trouble.lock().unwrap().take();
    match found {
        Some(crate::inspect::Trouble::OtherSite(other)) => engine.record_refusal(app_id, host, 443, Reason::Malformed, "other_site", &other),
        Some(crate::inspect::Trouble::NotUnderstood(what)) => engine.record_refusal(app_id, host, 443, Reason::Malformed, "not_followed", what),
        None => {}
    }
}

/// Read from the client until `buf` holds one complete TLS record, then return its server name.
async fn read_hello(client: &mut TcpStream, buf: &mut Vec<u8>) -> Result<Option<String>, ()> {
    let mut chunk = [0u8; 4096];
    loop {
        if buf.len() >= 5 {
            if buf[0] != 22 || buf[1] != 3 {
                return Err(());
            }
            let need = 5 + u16::from_be_bytes([buf[3], buf[4]]) as usize;
            if buf.len() >= need {
                return client_hello_sni(buf);
            }
            if need > 20 * 1024 {
                return Err(());
            }
        }
        let n = client.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn relay(engine: &Engine, client: TcpStream, upstream: TcpStream, out: Arc<AtomicU64>, inn: Arc<AtomicU64>) {
    let (mut cr, mut cw) = client.into_split();
    let (mut ur, mut uw) = upstream.into_split();
    let up = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match cr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if uw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    out.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = uw.shutdown().await;
    };
    let down = async {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match ur.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    inn.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = cw.shutdown().await;
    };
    tokio::pin!(up, down);
    let mut cut = engine.cut_signal();
    // Run both directions. When one finishes, give the other a moment to drain. The panic
    // button cuts everything at once.
    tokio::select! {
        _ = &mut up => { let _ = tokio::time::timeout(Duration::from_secs(5), &mut down).await; }
        _ = &mut down => { let _ = tokio::time::timeout(Duration::from_secs(2), &mut up).await; }
        _ = cut.changed() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connect() {
        let r = parse_request("CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443").unwrap();
        assert_eq!((r.connect, r.host.as_str(), r.port), (true, "api.anthropic.com", 443));
    }

    #[test]
    fn rewrites_plain_http_and_pins_the_host_header() {
        let r = parse_request("GET http://deb.debian.org/debian/Release?x=1 HTTP/1.1\r\nHost: evil.example.net\r\nProxy-Authorization: Basic abc\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\nUser-Agent: curl").unwrap();
        let fwd = String::from_utf8(r.forward).unwrap();
        assert!(fwd.starts_with("GET /debian/Release?x=1 HTTP/1.1\r\nHost: deb.debian.org\r\n"));
        assert!(!fwd.contains("evil.example.net"), "the app's own Host header must not survive");
        assert!(!fwd.to_lowercase().contains("proxy-"));
        assert!(fwd.ends_with("Connection: close\r\n\r\n"));
        assert!(fwd.contains("User-Agent: curl\r\n"));
    }

    #[test]
    fn hands_odd_destinations_to_the_engine_unchanged() {
        // The engine refuses these and records them; the parser must not hide them.
        assert_eq!(parse_request("CONNECT 127.0.0.1:443 HTTP/1.1").unwrap().host, "127.0.0.1");
        assert_eq!(parse_request("CONNECT [::1]:443 HTTP/1.1").unwrap().host, "[::1]:443");
        assert_eq!(parse_request("GET http://user@evil.com/ HTTP/1.1").unwrap().host, "user@evil.com");
        assert_eq!(parse_request("CONNECT trusted.com\0.evil.com:443 HTTP/1.1").unwrap().host, "trusted.com\0.evil.com");
        for h in ["127.0.0.1", "[::1]:443", "user@evil.com", "trusted.com\0.evil.com"] {
            assert_eq!(hostname::canonical(h), None);
        }
    }

    #[test]
    fn rejects_malformed_requests() {
        for bad in ["", "GET", "GET / HTTP/1.1", "GET https://example.com/ HTTP/1.1", "GET ftp://example.com/ HTTP/1.1", "CONNECT example.com:0 HTTP/1.1", "CONNECT example.com:99999 HTTP/1.1", "CONNECT example.com:443 HTTP/2", "get http://example.com/ HTTP/1.1", "CONNECT  example.com:443 HTTP/1.1", "CONNECT example.com:443 HTTP/1.1 extra"] {
            assert!(parse_request(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    /// A minimal but well-formed ClientHello carrying the given server name.
    fn hello(sni: Option<&str>) -> Vec<u8> {
        let mut exts = Vec::new();
        // An unrelated extension first, to exercise the walk.
        exts.extend_from_slice(&[0x00, 0x0b, 0x00, 0x02, 0x01, 0x00]);
        if let Some(name) = sni {
            let n = name.as_bytes();
            let mut sn = Vec::new();
            sn.extend_from_slice(&((n.len() + 3) as u16).to_be_bytes());
            sn.push(0);
            sn.extend_from_slice(&(n.len() as u16).to_be_bytes());
            sn.extend_from_slice(n);
            exts.extend_from_slice(&[0x00, 0x00]);
            exts.extend_from_slice(&(sn.len() as u16).to_be_bytes());
            exts.extend_from_slice(&sn);
        }
        let mut body = vec![0x03, 0x03];
        body.extend_from_slice(&[7u8; 32]);
        body.push(0); // session id
        body.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // one cipher suite
        body.extend_from_slice(&[0x01, 0x00]); // compression
        body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
        body.extend_from_slice(&exts);
        let mut hs = vec![0x01, 0, (body.len() >> 8) as u8, body.len() as u8];
        hs.extend_from_slice(&body);
        let mut rec = vec![22, 3, 1, (hs.len() >> 8) as u8, hs.len() as u8];
        rec.extend_from_slice(&hs);
        rec
    }

    #[test]
    fn reads_the_server_name_from_a_tls_hello() {
        assert_eq!(client_hello_sni(&hello(Some("api.anthropic.com"))), Ok(Some("api.anthropic.com".to_string())));
        assert_eq!(client_hello_sni(&hello(None)), Ok(None));
    }

    #[test]
    fn rejects_things_that_are_not_a_tls_hello() {
        assert!(client_hello_sni(b"GET / HTTP/1.1\r\n\r\n").is_err());
        assert!(client_hello_sni(b"SSH-2.0-OpenSSH_9.6\r\n").is_err());
        assert!(client_hello_sni(&[]).is_err());
        let mut truncated = hello(Some("example.com"));
        truncated.truncate(truncated.len() - 4);
        assert!(client_hello_sni(&truncated).is_err());
        let mut lying = hello(Some("example.com"));
        let n = lying.len();
        lying[n - 12] = 0xff; // corrupt a length inside the extension
        assert!(client_hello_sni(&lying).is_err() || client_hello_sni(&lying) != Ok(Some("example.com".into())));
    }
}
