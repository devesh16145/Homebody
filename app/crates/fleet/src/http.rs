//! Just enough HTTP for a small server: one request per connection, a size limit on everything.

use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const HEAD_LIMIT: usize = 16 * 1024;
pub const BODY_LIMIT: usize = 24 * 1024 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn json(&self) -> Result<serde_json::Value, Response> {
        serde_json::from_slice(&self.body).map_err(|_| Response::error(400, "The request was not valid JSON."))
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        self.header("cookie")?.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn json(v: &serde_json::Value) -> Response {
        Response { status: 200, content_type: "application/json", body: serde_json::to_vec(v).unwrap_or_default(), headers: vec![] }
    }

    pub fn error(status: u16, message: &str) -> Response {
        Response { status, content_type: "application/json", body: serde_json::to_vec(&serde_json::json!({ "error": message })).unwrap_or_default(), headers: vec![] }
    }

    pub fn bytes(content_type: &'static str, body: Vec<u8>) -> Response {
        Response { status: 200, content_type, body, headers: vec![] }
    }

    pub fn with(mut self, name: &str, value: String) -> Response {
        self.headers.push((name.to_string(), value));
        self
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi * 16 + lo);
                i += 3;
            }
            (b'+', _, _) => {
                out.push(b' ');
                i += 1;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).map(|(k, v)| (percent_decode(k), percent_decode(v))).collect()
}

/// Read one request. `None` for anything that is not a well-formed request within the limits.
pub async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Request> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > HEAD_LIMIT {
            return None;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let mut headers = [httparse::EMPTY_HEADER; 48];
    let mut req = httparse::Request::new(&mut headers);
    req.parse(&buf[..head_end]).ok()?;
    let method = req.method?.to_string();
    let target = req.path?.to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), parse_query(q))).unwrap_or((target.clone(), HashMap::new()));
    let headers: HashMap<String, String> = req.headers.iter().map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).trim().to_string())).collect();
    if headers.contains_key("transfer-encoding") {
        return None; // bodies come with a length, or not at all
    }
    let len: usize = headers.get("content-length").map(|v| v.parse().ok()).unwrap_or(Some(0))?;
    if len > BODY_LIMIT {
        return None;
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(Request { method, path, query, headers, body })
}

pub async fn write_response<S: AsyncWrite + Unpin>(stream: &mut S, r: Response) -> std::io::Result<()> {
    let reason = match r.status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n", r.status, r.content_type, r.body.len());
    for (k, v) in &r.headers {
        // Header values are ours, but never let a line break through.
        head.push_str(&format!("{k}: {}\r\n", v.replace(['\r', '\n'], " ")));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&r.body).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_is_read_with_its_query_headers_and_body() {
        let raw = b"POST /api/x?device=a%20b&q=1+2 HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nCookie: a=1; hb_session=abc\r\n\r\nbodyEXTRA".to_vec();
        let r = read_request(&mut &raw[..]).await.unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.body.as_slice()), ("POST", "/api/x", &b"body"[..]));
        assert_eq!((r.query["device"].as_str(), r.query["q"].as_str()), ("a b", "1 2"));
        assert_eq!(r.cookie("hb_session").as_deref(), Some("abc"));
    }

    #[tokio::test]
    async fn oversized_and_chunked_requests_are_refused() {
        let big = format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", BODY_LIMIT + 1);
        assert!(read_request(&mut big.as_bytes()).await.is_none());
        let chunked = b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert!(read_request(&mut &chunked[..]).await.is_none());
        let endless = vec![b'a'; HEAD_LIMIT + 5000];
        assert!(read_request(&mut &endless[..]).await.is_none());
    }

    #[test]
    fn percent_signs_that_are_not_escapes_are_kept() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
    }
}
