//! Looking inside traffic to places the user trusts.
//!
//! Without this, the gate sees a name, a size and nothing else: a trusted name can front for
//! another site inside the encryption, and "219 KB to your AI provider" could be anything.
//! With it switched on for an app, the gate ends the app's encrypted connection itself, opens
//! its own to the real server (checking that server's certificate the ordinary way), and reads
//! the request lines in between: which address inside the site, and how much was sent.
//!
//! What makes that possible is a certificate authority that exists only on this computer:
//!
//! - its key is made here, stays in the user's private data folder, and is never sent anywhere;
//! - only protected apps are told about it, through their environment. Nothing is added to the
//!   system's trusted certificates, so the rest of the computer is unaffected;
//! - an app that does not accept it simply fails to connect, and the Feed says so.
//!
//! What is recorded is the method and path of each request (never the query string, which can
//! carry keys) and sizes. Never bodies, never headers.

use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LEAF_LIFE: time::Duration = time::Duration::days(7);
const LEAF_REUSE: Duration = Duration::from_secs(3 * 24 * 3600);
/// The only protocol offered on both sides, so that request lines can be read as text.
const ALPN: &[u8] = b"http/1.1";

/// The local certificate authority, and the per-site certificates made from it.
pub struct Inspector {
    issuer: Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
    ca_pem: String,
    /// One key for every per-site certificate this run makes.
    leaf_key: KeyPair,
    servers: Mutex<HashMap<String, (Instant, Arc<rustls::ServerConfig>)>>,
    client: Arc<rustls::ClientConfig>,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Sandbox-ai local inspection");
    dn.push(DnType::OrganizationName, "Sandbox-ai, this computer only");
    p.distinguished_name = dn;
    // May sign certificates for sites, and nothing that could itself sign.
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let now = time::OffsetDateTime::now_utc();
    p.not_before = now - time::Duration::days(1);
    p.not_after = now + time::Duration::days(3650);
    p
}

impl Inspector {
    /// Load this computer's certificate authority, making it on first use.
    pub fn open(key_path: &Path, cert_path: &Path) -> Result<Inspector, String> {
        let fail = |what: &str, e: &dyn std::fmt::Display| format!("The local certificate {what}: {e}");
        let existing = std::fs::read_to_string(key_path).ok().zip(std::fs::read_to_string(cert_path).ok());
        let (ca_key, ca_pem) = match existing {
            Some((key_pem, cert_pem)) => (KeyPair::from_pem(&key_pem).map_err(|e| fail("key could not be read", &e))?, cert_pem),
            None => {
                let key = KeyPair::generate().map_err(|e| fail("key could not be made", &e))?;
                let cert = ca_params().self_signed(&key).map_err(|e| fail("could not be made", &e))?;
                write_private(key_path, key.serialize_pem().as_bytes()).map_err(|e| fail("key could not be saved", &e))?;
                std::fs::write(cert_path, cert.pem()).map_err(|e| fail("could not be saved", &e))?;
                (key, cert.pem())
            }
        };
        let ca_der = pem_to_der(&ca_pem).ok_or("The local certificate on disk is damaged.")?;
        // Signing only needs the authority's name and key, which never change.
        let issuer = Issuer::new(ca_params(), ca_key);
        let leaf_key = KeyPair::generate().map_err(|e| fail("key could not be made", &e))?;

        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut client = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        client.alpn_protocols = vec![ALPN.to_vec()];
        Ok(Inspector { issuer, ca_der, ca_pem, leaf_key, servers: Mutex::new(HashMap::new()), client: Arc::new(client) })
    }

    /// The authority's certificate, for handing to protected apps.
    pub fn ca_pem(&self) -> &str {
        &self.ca_pem
    }

    /// How to answer an app that asked for `host`: as that site, vouched for by our authority.
    pub fn server_config(&self, host: &str) -> Result<Arc<rustls::ServerConfig>, String> {
        if let Some((made, cfg)) = self.servers.lock().unwrap().get(host) {
            if made.elapsed() < LEAF_REUSE {
                return Ok(cfg.clone());
            }
        }
        let mut p = CertificateParams::new(vec![host.to_string()]).map_err(|e| e.to_string())?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host);
        p.distinguished_name = dn;
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.use_authority_key_identifier_extension = true;
        let now = time::OffsetDateTime::now_utc();
        p.not_before = now - time::Duration::hours(1);
        p.not_after = now + LEAF_LIFE;
        let leaf = p.signed_by(&self.leaf_key, &self.issuer).map_err(|e| e.to_string())?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.leaf_key.serialize_der()));
        let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(vec![leaf.der().clone(), self.ca_der.clone()], key)
            .map_err(|e| e.to_string())?;
        cfg.alpn_protocols = vec![ALPN.to_vec()];
        let cfg = Arc::new(cfg);
        self.servers.lock().unwrap().insert(host.to_string(), (Instant::now(), cfg.clone()));
        Ok(cfg)
    }

    /// How the gate itself connects to the real server: the ordinary public authorities, and
    /// never our own. If the real certificate does not check out, there is no connection.
    pub fn client_config(&self) -> Arc<rustls::ClientConfig> {
        self.client.clone()
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn pem_to_der(pem: &str) -> Option<CertificateDer<'static>> {
    use base64::Engine as _;
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    base64::engine::general_purpose::STANDARD.decode(body.trim()).ok().map(CertificateDer::from)
}

// ---------------------------------------------------------------------------------------------
// Handing the certificate to protected apps
// ---------------------------------------------------------------------------------------------

/// Where protected apps can read the certificate. They cannot enter the user's home, so it
/// lives in the shared folder, in a subfolder that belongs to the user.
pub fn public_dir() -> PathBuf {
    PathBuf::from(format!("/Users/Shared/SandboxAI-certificates/{}", crate::fence::real_user()))
}

/// Only our authority: for programs that add to what they already trust.
pub fn public_ca_path() -> PathBuf {
    public_dir().join("sandbox-ai-ca.pem")
}

/// The system's authorities plus ours: for programs that replace what they trust with one file.
pub fn public_bundle_path() -> PathBuf {
    public_dir().join("bundle.pem")
}

/// Put the certificate where protected apps can read it. Refuses a folder that is not ours:
/// the shared folder is writable by everyone, so someone else could have made it first.
pub fn publish(ca_pem: &str, system_roots_pem: &str) -> Result<(), String> {
    let dir = public_dir();
    let fail = |e: std::io::Error| format!("The certificate could not be shared with protected apps: {e}");
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
        owned_by_us(parent)?;
    }
    std::fs::create_dir_all(&dir).map_err(fail)?;
    owned_by_us(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir.parent(), Some(dir.as_path())].into_iter().flatten() {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).map_err(fail)?;
        }
    }
    std::fs::write(public_ca_path(), ca_pem).map_err(fail)?;
    std::fs::write(public_bundle_path(), format!("{system_roots_pem}\n{ca_pem}")).map_err(fail)?;
    Ok(())
}

fn owned_by_us(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        let me = std::fs::metadata(crate::paths::data_dir()).map(|m| m.uid()).map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.uid() != me {
            return Err(format!("{} exists and is not yours, so the certificate was not put there.", dir.display()));
        }
    }
    let _ = dir;
    Ok(())
}

/// The settings that tell a program about the certificate. Different programs read different
/// ones; a program that reads none of them will refuse the connection, and the Feed says so.
pub fn env() -> Vec<(String, String)> {
    let (ca, bundle) = (public_ca_path().to_string_lossy().into_owned(), public_bundle_path().to_string_lossy().into_owned());
    vec![
        ("NODE_EXTRA_CA_CERTS".into(), ca),
        ("SSL_CERT_FILE".into(), bundle.clone()),
        ("REQUESTS_CA_BUNDLE".into(), bundle.clone()),
        ("CURL_CA_BUNDLE".into(), bundle.clone()),
        ("GIT_SSL_CAINFO".into(), bundle),
    ]
}

// ---------------------------------------------------------------------------------------------
// Reading request lines
// ---------------------------------------------------------------------------------------------

/// Why an inspected connection was cut.
#[derive(Debug, PartialEq, Clone)]
pub enum Trouble {
    /// The request named a different site than the one that was approved.
    OtherSite(String),
    /// Not something we can follow safely, so it is not let through.
    NotUnderstood(&'static str),
    /// A request for an address inside the site that the user has switched off.
    Refused(String),
    /// The content of a request held something that looks like a secret.
    Secret(crate::secrets::Found),
}

const HEAD_LIMIT: usize = 64 * 1024;
const MAX_KINDS: usize = 12;

enum State {
    Head,
    Body(u64),
    ChunkSize,
    ChunkData(u64),
    ChunkEnd,
    Trailers,
    /// After an upgrade (a web socket, say) the bytes are no longer requests.
    Opaque,
}

/// Follows the requests an app sends on one connection, without keeping any of their content.
/// Fed every byte the app sends, in order, and says which bytes may be passed on. The start of
/// each request (its line and headers) is held back until it is complete and has been checked,
/// so nothing of a request that is refused ever leaves. Everything it does not understand is
/// an error, because bytes we cannot follow are bytes we cannot vouch for.
pub struct Sniffer {
    host: String,
    state: State,
    buf: Vec<u8>,
    /// "METHOD /path" -> how many times.
    seen: Vec<(String, u64)>,
    pub requests: u64,
    /// Whether a request for this path (no query) may go. The user's switches for addresses
    /// inside a trusted place.
    allow: Box<dyn Fn(&str) -> bool + Send>,
    /// Looks at the content of requests for secrets, when that is switched on.
    scanner: Option<crate::secrets::Scanner>,
    /// Bytes waiting to be passed on, in order, while the scanner still holds back the content
    /// before them. Each piece is marked as content (true) or as the framing around it.
    waiting: std::collections::VecDeque<(Vec<u8>, bool)>,
    /// How many waiting content bytes the scanner has cleared.
    cleared: usize,
}

impl Sniffer {
    /// Also look at the content of each request for secrets (see `secrets`).
    pub fn scanning(mut self, scanner: crate::secrets::Scanner) -> Sniffer {
        self.scanner = Some(scanner);
        self
    }

    /// Pass these bytes on, or queue them behind content the scanner is still holding back.
    fn emit(&mut self, bytes: &[u8], content: bool, pass: &mut Vec<u8>) -> Result<(), Trouble> {
        let Some(scanner) = self.scanner.as_mut() else {
            pass.extend_from_slice(bytes);
            return Ok(());
        };
        if content {
            self.cleared += scanner.push(bytes).map_err(Trouble::Secret)?;
            self.waiting.push_back((bytes.to_vec(), true));
        } else if self.waiting.is_empty() {
            pass.extend_from_slice(bytes);
            return Ok(());
        } else {
            self.waiting.push_back((bytes.to_vec(), false));
        }
        self.release(pass);
        Ok(())
    }

    /// The content of the current request has ended: what was held back is judged and let go.
    fn content_ended(&mut self, pass: &mut Vec<u8>) -> Result<(), Trouble> {
        if let Some(scanner) = self.scanner.as_mut() {
            self.cleared += scanner.finish().map_err(Trouble::Secret)?;
            self.release(pass);
        }
        Ok(())
    }

    fn release(&mut self, pass: &mut Vec<u8>) {
        while let Some((bytes, content)) = self.waiting.front_mut() {
            if !*content {
                pass.extend_from_slice(bytes);
            } else if self.cleared >= bytes.len() {
                self.cleared -= bytes.len();
                pass.extend_from_slice(bytes);
            } else {
                // Part of this piece is cleared; the rest stays.
                let n = self.cleared;
                pass.extend_from_slice(&bytes[..n]);
                bytes.drain(..n);
                self.cleared = 0;
                return;
            }
            self.waiting.pop_front();
        }
    }

    pub fn new(host: &str) -> Sniffer {
        Sniffer::with_rules(host, |_| true)
    }

    pub fn with_rules(host: &str, allow: impl Fn(&str) -> bool + Send + 'static) -> Sniffer {
        Sniffer { host: host.to_ascii_lowercase(), state: State::Head, buf: Vec::new(), seen: Vec::new(), requests: 0, allow: Box::new(allow), scanner: None, waiting: Default::default(), cleared: 0 }
    }

    /// What was asked for, most frequent first.
    pub fn summary(&self) -> Vec<(String, u64)> {
        let mut v = self.seen.clone();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Take the next bytes from the app. Bytes that may be passed on are added to `pass`, in
    /// order. On an error, `pass` holds what came before the trouble and nothing of it.
    pub fn feed(&mut self, mut data: &[u8], pass: &mut Vec<u8>) -> Result<(), Trouble> {
        while !data.is_empty() {
            match self.state {
                State::Opaque => {
                    pass.extend_from_slice(data);
                    return Ok(());
                }
                State::Body(left) | State::ChunkData(left) => {
                    let take = (left.min(data.len() as u64)) as usize;
                    self.emit(&data[..take], true, pass)?;
                    data = &data[take..];
                    let left = left - take as u64;
                    let ended = matches!((&self.state, left), (State::Body(_), 0));
                    self.state = match (&self.state, left) {
                        (State::Body(_), 0) => State::Head,
                        (State::Body(_), n) => State::Body(n),
                        (_, 0) => State::ChunkEnd,
                        (_, n) => State::ChunkData(n),
                    };
                    if ended {
                        self.content_ended(pass)?;
                    }
                }
                State::Head | State::ChunkSize | State::ChunkEnd | State::Trailers => {
                    // Line-shaped parts: held back until the part is complete and checked.
                    let Some(used) = self.take_until_complete(data, pass)? else { return Ok(()) };
                    data = &data[used..];
                }
            }
        }
        Ok(())
    }

    /// Add bytes to the buffer until the current line-shaped part is complete, then act on it.
    /// Returns how many bytes of `data` were used, or None if all of it was and more is needed.
    fn take_until_complete(&mut self, data: &[u8], pass: &mut Vec<u8>) -> Result<Option<usize>, Trouble> {
        let end: &[u8] = if matches!(self.state, State::Head) { b"\r\n\r\n" } else { b"\r\n" };
        let before = self.buf.len();
        self.buf.extend_from_slice(data);
        // Only look where the ending could newly be, plus a little overlap.
        let from = before.saturating_sub(end.len() - 1);
        let Some(at) = self.buf[from..].windows(end.len()).position(|w| w == end).map(|p| from + p) else {
            if self.buf.len() > HEAD_LIMIT {
                return Err(Trouble::NotUnderstood("a request line or header that never ends"));
            }
            return Ok(None);
        };
        let part: Vec<u8> = self.buf[..at].to_vec();
        let used = at + end.len() - before;
        self.buf.clear();
        match self.state {
            State::Head => self.on_head(&part)?,
            State::ChunkSize => {
                let line = std::str::from_utf8(&part).map_err(|_| Trouble::NotUnderstood("a chunk size that is not text"))?;
                let hex = line.split(';').next().unwrap_or("").trim();
                let size = u64::from_str_radix(hex, 16).map_err(|_| Trouble::NotUnderstood("a chunk size that is not a number"))?;
                if size == 0 {
                    // The last chunk: the content is complete.
                    self.content_ended(pass)?;
                }
                self.state = if size == 0 { State::Trailers } else { State::ChunkData(size) };
            }
            State::ChunkEnd => {
                if !part.is_empty() {
                    return Err(Trouble::NotUnderstood("a chunk that does not end where it said"));
                }
                self.state = State::ChunkSize;
            }
            State::Trailers => {
                // An empty line ends the request; anything else is one more trailer line.
                if part.is_empty() {
                    self.state = State::Head;
                }
            }
            _ => unreachable!("only line-shaped states get here"),
        }
        // Checked and understood: this part may go, in its place behind anything still held.
        let mut framing = part;
        framing.extend_from_slice(end);
        self.emit(&framing, false, pass)?;
        Ok(Some(used))
    }

    fn on_head(&mut self, head: &[u8]) -> Result<(), Trouble> {
        let mut text = head.to_vec();
        text.extend_from_slice(b"\r\n\r\n");
        let mut headers = [httparse::EMPTY_HEADER; 96];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&text) {
            Ok(httparse::Status::Complete(_)) => {}
            _ => return Err(Trouble::NotUnderstood("a request that is not ordinary HTTP")),
        }
        let method = req.method.unwrap_or("").to_string();
        let target = req.path.unwrap_or("");
        if method.is_empty() || method.eq_ignore_ascii_case("CONNECT") {
            return Err(Trouble::NotUnderstood("a tunnel inside the connection"));
        }
        let (mut host, mut length, mut chunked, mut upgrade) = (None::<String>, None::<u64>, false, false);
        for h in req.headers.iter() {
            let value = std::str::from_utf8(h.value).map_err(|_| Trouble::NotUnderstood("a header that is not text"))?.trim();
            if h.name.eq_ignore_ascii_case("host") {
                if host.is_some() {
                    return Err(Trouble::NotUnderstood("two Host headers"));
                }
                host = Some(value.to_ascii_lowercase());
            } else if h.name.eq_ignore_ascii_case("content-length") {
                let n = value.parse::<u64>().map_err(|_| Trouble::NotUnderstood("a length that is not a number"))?;
                if length.is_some_and(|l| l != n) {
                    return Err(Trouble::NotUnderstood("two different lengths"));
                }
                length = Some(n);
            } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
                if !value.eq_ignore_ascii_case("chunked") {
                    return Err(Trouble::NotUnderstood("an unusual transfer encoding"));
                }
                chunked = true;
            } else if h.name.eq_ignore_ascii_case("upgrade") {
                upgrade = true;
            }
        }
        // The site named inside must be the site that was approved. This is what stops a
        // trusted name being used as cover for another one at the same provider.
        let named = host.ok_or(Trouble::NotUnderstood("a request that does not say which site it is for"))?;
        let bare = named.strip_suffix(":443").unwrap_or(&named);
        if bare != self.host {
            return Err(Trouble::OtherSite(named.chars().filter(|c| !c.is_control()).take(80).collect()));
        }
        // A full address in the request line must name the same site too.
        if let Some(rest) = target.strip_prefix("https://").or_else(|| target.strip_prefix("http://")) {
            let authority = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
            if authority.strip_suffix(":443").unwrap_or(&authority) != self.host {
                return Err(Trouble::OtherSite(authority.chars().filter(|c| !c.is_control()).take(80).collect()));
            }
        }
        if chunked && length.is_some() {
            return Err(Trouble::NotUnderstood("both a length and chunks"));
        }
        let path = clean_path(target);
        // Judged as written and as a server would most generously read it, so that another
        // spelling of a switched-off address is refused too.
        if !(self.allow)(&path) || !(self.allow)(&rule_path(target)) {
            return Err(Trouble::Refused(path));
        }
        self.note(&method, &path);
        self.state = if upgrade {
            State::Opaque
        } else if chunked {
            State::ChunkSize
        } else {
            match length.unwrap_or(0) {
                0 => State::Head,
                n => State::Body(n),
            }
        };
        Ok(())
    }

    fn note(&mut self, method: &str, shown: &str) {
        self.requests += 1;
        let method: String = method.chars().filter(|c| c.is_ascii_alphabetic()).take(12).collect::<String>().to_ascii_uppercase();
        let what = format!("{method} {shown}");
        let known = self.seen.iter().position(|(k, _)| *k == what);
        let key = match known {
            Some(_) => what,
            None if self.seen.iter().filter(|(k, _)| k != "other requests").count() < MAX_KINDS => what,
            None => "other requests".to_string(),
        };
        match self.seen.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => self.seen.push((key, 1)),
        }
    }
}

/// The path of a request as it is shown, matched against rules and written down: no site, and
/// nothing after "?" or "#", which can carry keys and personal data.
pub fn clean_path(target: &str) -> String {
    let path = target.split(['?', '#']).next().unwrap_or("");
    let path = path.strip_prefix("https://").or_else(|| path.strip_prefix("http://")).map(|r| r.find('/').map(|i| &r[i..]).unwrap_or("/")).unwrap_or(path);
    let shown: String = path.chars().filter(|c| c.is_ascii_graphic()).take(120).collect();
    if shown.is_empty() { "/".into() } else { shown }
}

/// The path of a request the way a forgiving server might read it, for matching against the
/// user's switches: escapes undone, capitals lowered, backslashes read as slashes, empty and
/// "." segments dropped, ".." applied, and anything after ";" in a segment ignored. Different
/// spellings of one address come out the same, so switching an address off cannot be dodged
/// by respelling it. Never shown or written down, and never shortened.
pub fn rule_path(target: &str) -> String {
    let path = target.split(['?', '#']).next().unwrap_or("");
    let path = path.strip_prefix("https://").or_else(|| path.strip_prefix("http://")).map(|r| r.find('/').map(|i| &r[i..]).unwrap_or("/")).unwrap_or(path);
    // Undo %XX escapes (once, as servers do).
    let bytes = path.as_bytes();
    let mut plain: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                plain.push(hi * 16 + lo);
                i += 3;
            }
            (b, _, _) => {
                plain.push(b);
                i += 1;
            }
        }
    }
    let text = String::from_utf8_lossy(&plain).replace('\\', "/").to_lowercase();
    let mut kept: Vec<&str> = Vec::new();
    for segment in text.split('/') {
        match segment.split(';').next().unwrap_or("") {
            "" | "." => {}
            ".." => {
                kept.pop();
            }
            name => kept.push(name),
        }
    }
    format!("/{}", kept.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn inspector(dir: &Path) -> Inspector {
        Inspector::open(&dir.join("ca.key"), &dir.join("ca.pem")).unwrap()
    }

    fn client_trusting(ca: Option<&Inspector>) -> tokio_rustls::TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        if let Some(i) = ca {
            roots.add(i.ca_der.clone()).unwrap();
        } else {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().unwrap().with_root_certificates(roots).with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    }

    async fn handshake(i: &Inspector, served_as: &str, asked_for: &str, trusting: bool) -> Result<String, String> {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let acceptor = tokio_rustls::TlsAcceptor::from(i.server_config(served_as).unwrap());
        let server = tokio::spawn(async move {
            let mut s = acceptor.accept(a).await.map_err(|e| e.to_string())?;
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            s.write_all(b"world").await.map_err(|e| e.to_string())?;
            s.shutdown().await.ok();
            Ok::<_, String>(String::from_utf8_lossy(&buf).into_owned())
        });
        let name = rustls::pki_types::ServerName::try_from(asked_for.to_string()).unwrap();
        let mut c = client_trusting(trusting.then_some(i)).connect(name, b).await.map_err(|e| e.to_string())?;
        c.write_all(b"hello").await.map_err(|e| e.to_string())?;
        let mut back = String::new();
        c.read_to_string(&mut back).await.ok();
        server.await.unwrap()?;
        Ok(back)
    }

    #[tokio::test]
    async fn a_program_that_trusts_the_local_authority_gets_through_and_others_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let i = inspector(dir.path());
        assert_eq!(handshake(&i, "api.example.com", "api.example.com", true).await.unwrap(), "world");
        // Not told about our authority: refused.
        assert!(handshake(&i, "api.example.com", "api.example.com", false).await.is_err());
        // A certificate for one site is no good for another.
        assert!(handshake(&i, "api.example.com", "evil.example.org", true).await.is_err());
    }

    #[tokio::test]
    async fn the_authority_is_made_once_and_kept_private() {
        let dir = tempfile::tempdir().unwrap();
        let first = inspector(dir.path()).ca_pem().to_string();
        let again = inspector(dir.path());
        assert_eq!(first, again.ca_pem(), "the same authority after a restart");
        // Certificates made after a restart still check out against the saved authority.
        assert_eq!(handshake(&again, "a.example.com", "a.example.com", true).await.unwrap(), "world");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.path().join("ca.key")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Another computer's authority is a different one.
        let other = tempfile::tempdir().unwrap();
        assert_ne!(first, inspector(other.path()).ca_pem());
        // The gate's own connections trust the public authorities and not ours.
        assert!(handshake(&again, "a.example.com", "a.example.com", false).await.is_err());
    }

    fn feed_all(host: &str, bytes: &[u8], step: usize) -> Result<Sniffer, Trouble> {
        let mut s = Sniffer::new(host);
        let mut passed = Vec::new();
        for piece in bytes.chunks(step.max(1)) {
            s.feed(piece, &mut passed)?;
        }
        assert_eq!(passed, bytes, "everything that was understood is passed on unchanged");
        Ok(s)
    }

    /// Feed in pieces and return what was passed on, and the trouble if there was any.
    fn passed(mut s: Sniffer, bytes: &[u8], step: usize) -> (Vec<u8>, Option<Trouble>) {
        let mut out = Vec::new();
        for piece in bytes.chunks(step.max(1)) {
            if let Err(t) = s.feed(piece, &mut out) {
                return (out, Some(t));
            }
        }
        (out, None)
    }

    #[test]
    fn nothing_of_a_refused_request_leaves_and_everything_before_it_does() {
        let good = b"POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 5\r\n\r\nhello";
        let logging = b"POST /api/event_logging/v2/batch?x=1 HTTP/1.1\r\nHost: api.example.com\r\nAuthorization: Bearer SECRET\r\nContent-Length: 9\r\n\r\ntelemetry";
        let wire = [good.as_slice(), logging.as_slice(), good.as_slice()].concat();
        for step in [1, 3, 8, 50, 4096] {
            let s = Sniffer::with_rules("api.example.com", |path| !path.starts_with("/api/event_logging"));
            let (out, trouble) = passed(s, &wire, step);
            assert_eq!(trouble, Some(Trouble::Refused("/api/event_logging/v2/batch".into())), "step {step}");
            assert_eq!(out, good, "step {step}: the request before it went, and not one byte of the refused one");
        }
        // The same holds for a request that names another site, and for one that cannot be followed.
        let fronted = b"POST /x HTTP/1.1\r\nHost: attacker.example.net\r\nCookie: SECRET\r\n\r\n";
        let smuggled = b"POST /x HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\nabc";
        for bad in [fronted.as_slice(), smuggled.as_slice()] {
            for step in [1, 7, 4096] {
                let (out, trouble) = passed(Sniffer::new("api.example.com"), &[good.as_slice(), bad].concat(), step);
                assert!(trouble.is_some());
                assert_eq!(out, good, "step {step}");
            }
        }
        // A request line is held back until its headers are complete.
        let mut s = Sniffer::new("api.example.com");
        let mut out = Vec::new();
        s.feed(b"POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\n", &mut out).unwrap();
        assert!(out.is_empty(), "nothing leaves before the request has been checked");
        s.feed(b"\r\n", &mut out).unwrap();
        assert!(out.starts_with(b"POST /v1/messages"));
    }

    #[test]
    fn another_spelling_of_a_switched_off_address_is_refused_too() {
        let off = |path: &str| !crate::catalog::path_under(path, "/api/event_logging");
        let padding = "a/".repeat(100);
        let spellings = [
            "/api/event_logging/v2/batch".to_string(),
            "/api//event_logging/v2/batch".into(),
            "/api/./event_logging/v2/batch".into(),
            "/x/../api/event_logging".into(),
            "/api/event_logging/../event_logging/v2".into(),
            "/API/Event_Logging/v2/batch".into(),
            "/api/%65vent_logging/v2/batch".into(),
            "/api%2Fevent_logging%2fv2".into(),
            "/api/event_logging;jsessionid=1/v2/batch".into(),
            "/api\\event_logging\\v2".into(),
            "/api/event_logging?x=/v1/messages".into(),
            "https://api.example.com/api/event_logging/v2/batch".into(),
            // Padded past the length at which a path is shortened for display.
            format!("/{padding}../{}api/event_logging/v2/batch", "../".repeat(99)),
        ];
        for spelled in &spellings {
            let wire = format!("POST {spelled} HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 4\r\n\r\ndata");
            let (out, trouble) = passed(Sniffer::with_rules("api.example.com", off), wire.as_bytes(), 9);
            assert!(matches!(trouble, Some(Trouble::Refused(_))), "{}", &spelled[..spelled.len().min(60)]);
            assert!(out.is_empty(), "nothing of it left: {}", &spelled[..spelled.len().min(60)]);
        }
        // Addresses that only look similar are not caught up in it.
        for fine in ["/api/event_logging_other", "/api/events", "/v1/messages", "/api/event_loggin/g"] {
            let wire = format!("GET {fine} HTTP/1.1\r\nHost: api.example.com\r\n\r\n");
            let (_, trouble) = passed(Sniffer::with_rules("api.example.com", off), wire.as_bytes(), 9);
            assert_eq!(trouble, None, "{fine}");
        }
        // Read either way it is under the switched-off address, so it is refused: a server
        // that does not tidy the path would take it as written.
        let odd = "GET /api/event_logging/../health HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        assert!(matches!(passed(Sniffer::with_rules("api.example.com", off), odd.as_bytes(), 9).1, Some(Trouble::Refused(_))));
        assert_eq!(rule_path("/A/b/../C//d/./e;x=1?q"), "/a/c/d/e");
        assert_eq!(rule_path("/../../.."), "/");
        assert_eq!(rule_path("/%zz/%4"), "/%zz/%4");
    }

    fn token() -> String {
        format!("gh{}_{}", "p", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8")
    }

    fn scanning(allowed: bool) -> Sniffer {
        Sniffer::new("api.example.com").scanning(crate::secrets::Scanner::new("this-computer", move |_| allowed))
    }

    #[test]
    fn a_request_carrying_a_secret_is_stopped_before_the_secret_leaves() {
        let filler = "x".repeat(1500);
        let body = format!("{{\"content\":\"{filler} my key is {} thanks {filler}\"}}", token());
        let plain = format!("POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        // The same content sent in chunks, with the secret split across two of them.
        let cut = body.find("gh").unwrap() + 9;
        let chunked = format!("POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n", cut, &body[..cut], body.len() - cut, &body[cut..]);
        for wire in [&plain, &chunked] {
            for step in [1, 13, 600, 4096] {
                let (out, trouble) = passed(scanning(false), wire.as_bytes(), step);
                assert!(matches!(&trouble, Some(Trouble::Secret(f)) if f.kind == "a GitHub token"), "step {step}: {trouble:?}");
                let sent = String::from_utf8_lossy(&out);
                assert!(!sent.contains("gh") || !sent.contains(&token()[..12]), "step {step}: part of the secret left");
                assert!(!sent.contains("thanks"), "step {step}: nothing after it left either");
            }
        }
    }

    #[test]
    fn with_scanning_on_everything_else_arrives_complete_and_in_order() {
        let small = b"POST /a HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 5\r\n\r\nhello";
        let big_body = "y".repeat(3000);
        let big = format!("POST /b HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: {}\r\n\r\n{big_body}", big_body.len());
        let chunked = b"POST /c HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\nX-T: 1\r\n\r\n";
        let get = b"GET /d HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        let wire = [small.as_slice(), big.as_bytes(), chunked.as_slice(), get.as_slice(), small.as_slice()].concat();
        for step in [1, 7, 512, 513, 4096, 100_000] {
            let (out, trouble) = passed(scanning(false), &wire, step);
            assert_eq!(trouble, None, "step {step}");
            assert!(out == wire, "step {step}: what arrives is exactly what was sent");
        }
        // A secret the user has let through goes like anything else.
        let body = format!("key {}", token());
        let with_secret = format!("POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let (out, trouble) = passed(scanning(true), with_secret.as_bytes(), 11);
        assert_eq!((out.as_slice(), trouble), (with_secret.as_bytes(), None));
        // Bytes are held back only within a request: a finished request has left completely.
        let mut s = scanning(false);
        let mut out = Vec::new();
        s.feed(small, &mut out).unwrap();
        assert_eq!(out, small);
    }

    #[test]
    fn paths_are_shown_without_what_follows_the_question_mark() {
        assert_eq!(clean_path("/v1/messages?beta=true&key=SECRET"), "/v1/messages");
        assert_eq!(clean_path("https://api.example.com/v1/x#frag"), "/v1/x");
        assert_eq!(clean_path("https://api.example.com"), "/");
        assert_eq!(clean_path(""), "/");
        assert_eq!(clean_path("/a b\u{7}c"), "/abc");
    }

    #[test]
    fn requests_are_followed_however_the_bytes_arrive() {
        let wire = b"POST /v1/messages?beta=true&key=SECRET HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 11\r\n\r\nhello world\
GET /v1/models HTTP/1.1\r\nhost: API.example.com:443\r\n\r\n\
POST /v1/messages HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nX-Trailer: 1\r\n\r\n\
GET /v1/models HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        for step in [1, 2, 3, 7, 16, 64, 4096] {
            let s = feed_all("api.example.com", wire, step).unwrap_or_else(|e| panic!("step {step}: {e:?}"));
            assert_eq!(s.requests, 4, "step {step}");
            assert_eq!(s.summary(), vec![("GET /v1/models".to_string(), 2), ("POST /v1/messages".to_string(), 2)], "step {step}");
        }
        // Nothing after "?" is ever kept.
        let s = feed_all("api.example.com", wire, 9).unwrap();
        assert!(!format!("{:?}", s.summary()).contains("SECRET"));
    }

    #[test]
    fn a_request_for_another_site_is_caught() {
        let fronted = b"POST /upload HTTP/1.1\r\nHost: attacker.example.net\r\nContent-Length: 4\r\n\r\ndata";
        assert_eq!(feed_all("api.example.com", fronted, 5).err(), Some(Trouble::OtherSite("attacker.example.net".into())));
        // Also when it hides behind an honest first request on the same connection.
        let second = [b"GET / HTTP/1.1\r\nHost: api.example.com\r\n\r\n".as_slice(), fronted].concat();
        assert!(matches!(feed_all("api.example.com", &second, 13), Err(Trouble::OtherSite(_))));
        // And when the request line carries a full address for somewhere else.
        let absolute = b"GET https://attacker.example.net/x HTTP/1.1\r\nHost: api.example.com\r\n\r\n";
        assert!(matches!(feed_all("api.example.com", absolute, 8), Err(Trouble::OtherSite(_))));
    }

    #[test]
    fn what_cannot_be_followed_is_not_let_through() {
        let cases: [&[u8]; 8] = [
            b"GET / HTTP/1.1\r\n\r\n",                                                              // no site named
            b"GET / HTTP/1.1\r\nHost: api.example.com\r\nHost: api.example.com\r\n\r\n",            // named twice
            b"POST / HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabc", // two lengths
            b"POST / HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n", // both framings
            b"POST / HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: gzip\r\n\r\n",        // unusual framing
            b"POST / HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n", // bad chunk size
            b"CONNECT evil.example.net:443 HTTP/1.1\r\nHost: api.example.com\r\n\r\n",              // a tunnel inside
            b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03 this is not text at all\r\n\r\n",        // not HTTP
        ];
        for (n, c) in cases.iter().enumerate() {
            assert!(matches!(feed_all("api.example.com", c, 6), Err(Trouble::NotUnderstood(_))), "case {n}");
        }
        // A header that never ends is cut off rather than buffered forever.
        let endless = [b"GET / HTTP/1.1\r\nHost: api.example.com\r\nX: ".to_vec(), vec![b'a'; HEAD_LIMIT + 10]].concat();
        assert!(matches!(feed_all("api.example.com", &endless, 4096), Err(Trouble::NotUnderstood(_))));
    }

    #[test]
    fn after_an_upgrade_the_rest_is_left_alone_and_many_kinds_are_capped() {
        let ws = b"GET /v1/responses HTTP/1.1\r\nHost: api.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\x81\x05hello this is not a request line\r\n\r\n";
        let s = feed_all("api.example.com", ws, 11).unwrap();
        assert_eq!((s.requests, s.summary()), (1, vec![("GET /v1/responses".to_string(), 1)]));
        let mut many = Vec::new();
        for n in 0..40 {
            many.extend_from_slice(format!("GET /item/{n} HTTP/1.1\r\nHost: api.example.com\r\n\r\n").as_bytes());
        }
        let s = feed_all("api.example.com", &many, 50).unwrap();
        assert_eq!(s.requests, 40);
        assert_eq!(s.summary().len(), MAX_KINDS + 1);
        assert_eq!(s.summary()[0], ("other requests".to_string(), 40 - MAX_KINDS as u64));
    }
}
