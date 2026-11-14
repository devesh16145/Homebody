//! Keys that stay home.
//!
//! An agent needs an API key to work, and a key in an agent's hands is a key a planted
//! instruction can ask it to send somewhere. So Homebody keeps the key. The app is handed a
//! stand-in of the same shape; when a request to a place the key is meant for passes the gate,
//! the stand-in in its headers is replaced by the real key. The real key never enters the app's
//! private space, its environment, or anything it can read.
//!
//! A stand-in seen in a request to any other place is refused and written down by name: that
//! is the app trying to send your key somewhere it was not meant to go.
//!
//! This only works for an app whose connections Homebody looks inside, because the swap happens
//! inside the encrypted connection. Real keys are kept in one file in Homebody's own folder,
//! readable only by you; they are never written to the log, the settings or an export.

use base64::Engine as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What is to happen to a key's stand-in on one connection.
#[derive(Clone, Debug, PartialEq)]
pub enum Fate {
    /// This place is one the key is for: the real key goes in.
    Here(Vec<u8>),
    /// The key is not for this place.
    Elsewhere,
    /// The key is for this place, but Homebody no longer has it.
    Lost,
}

/// One key, as the gate needs it for one request.
#[derive(Clone)]
pub struct Swap {
    /// The name the app knows it by (the environment variable).
    pub name: String,
    pub stand_in: Vec<u8>,
    pub fate: Fate,
}

/// Why a request carrying a stand-in was not let through. Each carries the key's name.
#[derive(Debug, PartialEq, Clone)]
pub enum Misuse {
    /// Bound for a place the key is not for.
    Elsewhere(String),
    /// On its way to the right place, but not where a key belongs in a request: in the
    /// address, or in a header that is not for credentials. A server may repeat those back
    /// (in an error page, a redirect), which would hand the real key to the app.
    Misplaced(String),
    /// Homebody no longer has the real key.
    Lost(String),
}

/// The names of keys Homebody knows, and where each is meant to go.
const KNOWN: &[(&str, &str, &[&str])] = &[
    ("ANTHROPIC_API_KEY", "Anthropic", &["api.anthropic.com"]),
    ("OPENAI_API_KEY", "OpenAI", &["api.openai.com"]),
    ("GEMINI_API_KEY", "Google Gemini", &["generativelanguage.googleapis.com"]),
    ("GOOGLE_API_KEY", "Google Gemini", &["generativelanguage.googleapis.com"]),
    ("OPENROUTER_API_KEY", "OpenRouter", &["openrouter.ai"]),
    ("MISTRAL_API_KEY", "Mistral", &["api.mistral.ai"]),
    ("GROQ_API_KEY", "Groq", &["api.groq.com"]),
    ("DEEPSEEK_API_KEY", "DeepSeek", &["api.deepseek.com"]),
    ("XAI_API_KEY", "xAI", &["api.x.ai"]),
    ("HF_TOKEN", "Hugging Face", &["huggingface.co"]),
    ("GITHUB_TOKEN", "GitHub", &["api.github.com", "github.com"]),
    ("GH_TOKEN", "GitHub", &["api.github.com", "github.com"]),
];

/// Where a key of this name is meant to go, when Homebody knows the name.
pub fn usual_places(name: &str) -> Vec<String> {
    KNOWN.iter().find(|(n, _, _)| *n == name).map(|(_, _, hosts)| hosts.iter().map(|h| h.to_string()).collect()).unwrap_or_default()
}

/// Whose key this is, in plain words: "Anthropic".
pub fn maker(name: &str) -> Option<&'static str> {
    KNOWN.iter().find(|(n, _, _)| *n == name).map(|(_, maker, _)| *maker)
}

pub fn known_names() -> Vec<&'static str> {
    KNOWN.iter().map(|(n, _, _)| *n).collect()
}

/// A name an environment variable can have.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// A key is one run of ordinary characters. Anything else (a space, a line break) could be used
/// to add lines to a request, and is not a key anyone issues.
pub fn valid_value(value: &str) -> Result<(), String> {
    if value.len() < 8 {
        return Err("That is too short to be a key.".into());
    }
    if value.len() > 512 {
        return Err("That is too long to be a key.".into());
    }
    if !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("A key is one run of letters, digits and signs, with no spaces or line breaks.".into());
    }
    Ok(())
}

/// The openings providers put on their keys to say what kind of key it is. They are public and
/// the same for everyone, so a stand-in may carry one. Nothing else of a key is ever copied:
/// a key with no opening from this list gets a stand-in that shares no character with it by
/// design. Longest first.
const OPENINGS: &[&str] = &[
    "sk-ant-api03-", "sk-ant-oat01-", "sk-ant-", "sk-proj-", "sk-svcacct-", "sk-or-v1-", "sk-", "github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "hf_", "gsk_", "xai-", "AIza", "pplx-", "r8_", "fw_", "csk-", "nvapi-",
];

/// How much of the start of this key is a public opening (0 when it has none, or when what
/// would be left is too short to be unguessable).
fn opening(key: &str) -> usize {
    OPENINGS.iter().find(|o| key.starts_with(**o) && key.len() - o.len() >= 16).map(|o| o.len()).unwrap_or(0)
}

/// A stand-in for a key: the same length and the same opening (such as "sk-ant-api03-"), so a
/// program that checks what its key looks like is satisfied, and then letters that say what it
/// is to anyone who reads it.
pub fn stand_in(real: &str) -> String {
    use rand::Rng;
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const MARK: &str = "HOMEBODYstandin";
    let opening = opening(real);
    let mut out = String::with_capacity(real.len());
    out.push_str(&real[..opening]);
    let rest = real.len() - opening;
    // Always at least 20 random letters, so a stand-in cannot be guessed or collide.
    if rest >= MARK.len() + 20 {
        out.push_str(MARK);
    }
    let mut rng = rand::rngs::OsRng;
    while out.len() < real.len() {
        out.push(LETTERS[rng.gen_range(0..LETTERS.len())] as char);
    }
    out
}

/// Whether a stand-in made for `old` can go on standing in for `new`: the same length and the
/// same opening.
pub fn stand_in_fits(stand_in: &str, old: &str, new: &str) -> bool {
    stand_in.len() == new.len() && old.len() == new.len() && opening(old) == opening(new) && old[..opening(old)] == new[..opening(new)]
}

/// The last few characters, for showing which key this is without showing it. Nothing for a
/// short key, where four characters would be a real part of it.
pub fn hint(real: &str) -> String {
    if real.chars().count() < 20 {
        return String::new();
    }
    let tail: String = real.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{tail}")
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    let (mut n, mut rest) = (0, hay);
    while let Some(at) = find(rest, needle) {
        n += 1;
        rest = &rest[at + needle.len()..];
    }
    n
}

fn trim(v: &[u8]) -> &[u8] {
    let is_space = |b: &u8| *b == b' ' || *b == b'\t';
    let start = v.iter().position(|b| !is_space(b)).unwrap_or(v.len());
    let end = v.iter().rposition(|b| !is_space(b)).map(|i| i + 1).unwrap_or(start);
    &v[start..end]
}

/// Headers whose whole value is a key.
const KEY_HEADERS: &[&[u8]] = &[b"x-api-key", b"x-goog-api-key", b"api-key"];

/// The one line of a request's start, with every stand-in that sits where a credential
/// belongs replaced by `with(key)`. Returns the line and how many replacements were made.
///
/// Where a credential belongs:
/// - the whole value of `x-api-key`, `x-goog-api-key` or `api-key`;
/// - `Authorization: Bearer <key>` or `Authorization: token <key>`;
/// - the name or the password of `Authorization: Basic <base64 of name:password>` (git sends
///   a token that way);
/// - in the request line, a query parameter that is exactly `key=<key>` (Google's API).
fn in_credential_places(line: &[u8], first: bool, stand_in: &[u8], real: &[u8]) -> (Vec<u8>, usize) {
    if first {
        // METHOD SP target SP version. Only the query of the target is looked at.
        let Some(q) = line.iter().position(|b| *b == b'?') else { return (line.to_vec(), 0) };
        let end = line[q..].iter().position(|b| *b == b' ' || *b == b'#').map(|i| q + i).unwrap_or(line.len());
        let mut wanted = b"key=".to_vec();
        wanted.extend_from_slice(stand_in);
        let mut out = line[..=q].to_vec();
        let mut n = 0;
        for (i, param) in line[q + 1..end].split(|b| *b == b'&').enumerate() {
            if i > 0 {
                out.push(b'&');
            }
            if param == wanted.as_slice() {
                out.extend_from_slice(b"key=");
                out.extend_from_slice(real);
                n += 1;
            } else {
                out.extend_from_slice(param);
            }
        }
        out.extend_from_slice(&line[end..]);
        return (out, n);
    }
    let Some(colon) = line.iter().position(|b| *b == b':') else { return (line.to_vec(), 0) };
    let name = line[..colon].to_ascii_lowercase();
    let value = trim(&line[colon + 1..]);
    let rebuilt = |new_value: &[u8]| {
        let mut out = line[..=colon].to_vec();
        out.push(b' ');
        out.extend_from_slice(new_value);
        out
    };
    if KEY_HEADERS.contains(&name.as_slice()) {
        return if value == stand_in { (rebuilt(real), 1) } else { (line.to_vec(), 0) };
    }
    if name == b"authorization" {
        let Some(space) = value.iter().position(|b| *b == b' ') else { return (line.to_vec(), 0) };
        let (scheme, token) = (value[..space].to_ascii_lowercase(), trim(&value[space..]));
        if (scheme == b"bearer" || scheme == b"token") && token == stand_in {
            let mut v = value[..space].to_vec();
            v.push(b' ');
            v.extend_from_slice(real);
            return (rebuilt(&v), 1);
        }
        if scheme == b"basic" {
            if let Ok(plain) = base64::engine::general_purpose::STANDARD.decode(token) {
                let at = plain.iter().position(|b| *b == b':').unwrap_or(plain.len());
                let (user, pass) = (&plain[..at], plain.get(at + 1..).unwrap_or(&[]));
                if user == stand_in || pass == stand_in {
                    let mut changed = if user == stand_in { real.to_vec() } else { user.to_vec() };
                    changed.push(b':');
                    changed.extend_from_slice(if pass == stand_in { real } else { pass });
                    let mut v = value[..space].to_vec();
                    v.push(b' ');
                    v.extend_from_slice(base64::engine::general_purpose::STANDARD.encode(&changed).as_bytes());
                    return (rebuilt(&v), 1);
                }
            }
        }
    }
    (line.to_vec(), 0)
}

/// Whether a line carries this stand-in anywhere at all: as it is, or inside a "Basic" header.
fn carries(line: &[u8], stand_in: &[u8]) -> bool {
    if find(line, stand_in).is_some() {
        return true;
    }
    let Some(colon) = line.iter().position(|b| *b == b':') else { return false };
    if !line[..colon].eq_ignore_ascii_case(b"authorization") && !line[..colon].eq_ignore_ascii_case(b"proxy-authorization") {
        return false;
    }
    // Any run of base64 letters in the value that unwraps to something holding the stand-in.
    line[colon + 1..].split(|b| *b == b' ' || *b == b'\t' || *b == b',').filter(|t| t.len() >= 8).any(|t| {
        [&base64::engine::general_purpose::STANDARD, &base64::engine::general_purpose::STANDARD_NO_PAD, &base64::engine::general_purpose::URL_SAFE, &base64::engine::general_purpose::URL_SAFE_NO_PAD]
            .iter()
            .any(|e| e.decode(t).is_ok_and(|plain| find(&plain, stand_in).is_some()))
    })
}

/// Go through the start of a request (its line and headers) for the stand-ins of kept keys.
///
/// A stand-in that sits exactly where a credential belongs, in a request to a place its key is
/// for, is replaced by the real key. A request that carries a stand-in in any other way is
/// refused whole: to the wrong place, or to the right place but somewhere a server might
/// repeat back to the app (the address, another header), or by a method that asks the server
/// to repeat the request. Returns what to send and the names of the keys put in.
///
/// A start with no stand-in in it is returned exactly as it came, byte for byte.
pub fn swap(head: &[u8], keys: &[Swap]) -> Result<(Vec<u8>, Vec<String>), Misuse> {
    if keys.is_empty() {
        return Ok((head.to_vec(), Vec::new()));
    }
    let lines = split_lines(head);
    let mut out: Vec<Vec<u8>> = lines.iter().map(|l| l.to_vec()).collect();
    let mut used: Vec<String> = Vec::new();
    for k in keys.iter().filter(|k| k.stand_in.len() >= 8) {
        if !lines.iter().any(|l| carries(l, &k.stand_in)) {
            continue;
        }
        let real = match &k.fate {
            Fate::Here(real) => real,
            Fate::Elsewhere => return Err(Misuse::Elsewhere(k.name.clone())),
            Fate::Lost => return Err(Misuse::Lost(k.name.clone())),
        };
        for (i, line) in lines.iter().enumerate() {
            if !carries(line, &k.stand_in) {
                continue;
            }
            // Replace against the line as the app wrote it, so one key's real value is never
            // searched for another key's stand-in.
            let (changed, n) = in_credential_places(&out[i], i == 0, &k.stand_in, real);
            // Every occurrence must have been one that was replaced.
            let raw = count(line, &k.stand_in);
            if n == 0 || (raw > 0 && raw != n) || find(&changed, &k.stand_in).is_some() {
                return Err(Misuse::Misplaced(k.name.clone()));
            }
            out[i] = changed;
        }
        used.push(k.name.clone());
    }
    if !used.is_empty() {
        // TRACE asks the server to send the request back, headers and all.
        let method = lines.first().map(|l| l.split(|b| *b == b' ').next().unwrap_or(&[]).to_ascii_uppercase()).unwrap_or_default();
        if method == b"TRACE" {
            return Err(Misuse::Misplaced(used.remove(0)));
        }
        return Ok((out.join(&b"\r\n"[..]), used));
    }
    Ok((head.to_vec(), used))
}

/// Lines of a request's start. They end in CR LF; the pair is the separator, never one of the
/// two characters alone.
fn split_lines(head: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = head;
    while let Some(at) = find(rest, b"\r\n") {
        out.push(&rest[..at]);
        rest = &rest[at + 2..];
    }
    out.push(rest);
    out
}

// ---------------------------------------------------------------------------------------------
// Where the real keys are kept
// ---------------------------------------------------------------------------------------------

/// The real keys, by app and name. One file in Homebody's own folder, readable only by you.
pub struct Vault {
    path: PathBuf,
    keys: BTreeMap<String, String>,
    /// The file is there but could not be read as what it should be. Nothing is written over
    /// it: doing so would throw away every key in it.
    unreadable: bool,
}

fn slot(app_id: &str, name: &str) -> String {
    format!("{app_id}/{name}")
}

impl Vault {
    pub fn open(path: &Path) -> Vault {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(keys) => Vault { path: path.to_path_buf(), keys, unreadable: false },
                Err(_) => Vault { path: path.to_path_buf(), keys: BTreeMap::new(), unreadable: true },
            },
            Err(e) => Vault { path: path.to_path_buf(), keys: BTreeMap::new(), unreadable: e.kind() != std::io::ErrorKind::NotFound },
        }
    }

    pub fn get(&self, app_id: &str, name: &str) -> Option<&str> {
        self.keys.get(&slot(app_id, name)).map(String::as_str)
    }

    pub fn set(&mut self, app_id: &str, name: &str, value: &str) -> Result<(), String> {
        self.keys.insert(slot(app_id, name), value.to_string());
        self.save()
    }

    pub fn remove(&mut self, app_id: &str, name: &str) -> Result<(), String> {
        if self.keys.remove(&slot(app_id, name)).is_some() {
            self.save()?;
        }
        Ok(())
    }

    /// Forget every key kept for an app (it was removed from the list).
    pub fn remove_app(&mut self, app_id: &str) -> Result<(), String> {
        let before = self.keys.len();
        let prefix = format!("{app_id}/");
        self.keys.retain(|k, _| !k.starts_with(&prefix));
        if self.keys.len() != before {
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        if self.unreadable {
            return Err(format!("The file Homebody keeps keys in ({}) could not be read, so nothing was changed. Move it aside and add your keys again.", self.path.display()));
        }
        let text = serde_json::to_vec_pretty(&self.keys).map_err(|e| e.to_string())?;
        // Written under another name first and created private, so the keys are never in a
        // file others could read, not even for a moment.
        let tmp = self.path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut f = options.open(&tmp).map_err(|e| format!("Could not save the key: {e}"))?;
        f.write_all(&text).map_err(|e| format!("Could not save the key: {e}"))?;
        drop(f);
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("Could not save the key: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "sk-ant-STANDIN0000";
    const R: &str = "sk-ant-REALREAL11";

    fn here(name: &str, stand_in: &str, real: &str) -> Swap {
        Swap { name: name.into(), stand_in: stand_in.as_bytes().to_vec(), fate: Fate::Here(real.as_bytes().to_vec()) }
    }
    fn elsewhere(name: &str, stand_in: &str) -> Swap {
        Swap { name: name.into(), stand_in: stand_in.as_bytes().to_vec(), fate: Fate::Elsewhere }
    }
    fn swapped(head: &str) -> Result<String, Misuse> {
        swap(head.as_bytes(), &[here("ANTHROPIC_API_KEY", S, R)]).map(|(out, _)| String::from_utf8(out).unwrap())
    }
    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn a_stand_in_keeps_the_shape_and_none_of_the_key() {
        let real = format!("sk-ant-api03-{}", "Zq9".repeat(30));
        let s = stand_in(&real);
        assert_eq!(s.len(), real.len());
        assert!(s.starts_with("sk-ant-api03-HOMEBODYstandin"), "{s}");
        assert!(!s.contains("Zq9Zq9"));
        assert_ne!(stand_in(&real), s, "two stand-ins for one key differ");
        // Only an opening that is public is carried over. A key that merely has dashes in it
        // (here, one shaped like a serial number) gives nothing of itself to its stand-in.
        let serial = "3f2a1b4c-9d8e-4f70-a1b2-c3d4e5f60718";
        for _ in 0..50 {
            let s = stand_in(serial);
            assert_eq!(s.len(), serial.len());
            assert!(!s.starts_with("3f2a") && !s.contains("9d8e") && !s.contains('-'), "{s}");
        }
        // (Put together here so that no file holds something shaped like a real key.)
        let google = format!("AI{}{}", "za", "Sy-made-up-0123456789-made-up-012345");
        assert!(stand_in(&google).starts_with("AIza") && !stand_in(&google).contains("made-up"));
        let short = "abcd-123";
        let s = stand_in(short);
        assert_eq!(s.len(), short.len());
        assert_ne!(s, short);
        // A stand-in is kept when a key is replaced by one of the same kind and length only.
        let (a, b) = (format!("sk-ant-api03-{}", "a".repeat(40)), format!("sk-ant-api03-{}", "b".repeat(40)));
        assert!(stand_in_fits(&stand_in(&a), &a, &b));
        assert!(!stand_in_fits(&stand_in(&a), &a, &format!("sk-proj-{}", "b".repeat(45))));
    }

    #[test]
    fn the_real_key_goes_in_where_a_credential_belongs() {
        // The whole value of a key header, whatever the spelling of its name.
        assert_eq!(swapped(&format!("POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\nx-api-key: {S}\r\nContent-Length: 2")).unwrap(), format!("POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\nx-api-key: {R}\r\nContent-Length: 2"));
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nX-Api-Key:{S}")).unwrap(), format!("GET / HTTP/1.1\r\nX-Api-Key: {R}"));
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nx-goog-api-key: {S}")).unwrap(), format!("GET / HTTP/1.1\r\nx-goog-api-key: {R}"));
        // A bearer token, and GitHub's older "token" form.
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nAuthorization: Bearer {S}")).unwrap(), format!("GET / HTTP/1.1\r\nAuthorization: Bearer {R}"));
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nauthorization: token {S}")).unwrap(), format!("GET / HTTP/1.1\r\nauthorization: token {R}"));
        // A name-and-password header, as git sends it, with the key in either seat.
        assert_eq!(swapped(&format!("GET /r.git/info/refs HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("x-access-token:{S}")))).unwrap(), format!("GET /r.git/info/refs HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("x-access-token:{R}"))));
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("{S}:x-oauth-basic")))).unwrap(), format!("GET / HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("{R}:x-oauth-basic"))));
        // Google's API takes the key as a parameter named exactly "key".
        assert_eq!(swapped(&format!("GET /v1beta/models?alt=json&key={S}&pageSize=5 HTTP/1.1\r\nHost: g")).unwrap(), format!("GET /v1beta/models?alt=json&key={R}&pageSize=5 HTTP/1.1\r\nHost: g"));
        // The names of the keys put in are reported, once each.
        let (_, used) = swap(format!("GET /?key={S} HTTP/1.1\r\nx-api-key: {S}").as_bytes(), &[here("ANTHROPIC_API_KEY", S, R)]).unwrap();
        assert_eq!(used, vec!["ANTHROPIC_API_KEY"]);
    }

    #[test]
    fn a_request_with_no_stand_in_is_passed_on_byte_for_byte() {
        for head in ["GET / HTTP/1.1\r\nHost: api.anthropic.com", "GET / HTTP/1.1\r\nX-Odd:\t value \r\nAuthorization: Basic !!!notbase64", "GET /?key=other HTTP/1.1\r\nx-api-key: sk-ant-something-else"] {
            assert_eq!(swap(head.as_bytes(), &[here("ANTHROPIC_API_KEY", S, R)]).unwrap(), (head.as_bytes().to_vec(), vec![]));
        }
        // Bytes that are not text, and a header that only looks like the Basic kind.
        let odd = b"GET / HTTP/1.1\r\nAuthorization: \xff basic \r\nX: \xfe\xff".to_vec();
        assert_eq!(swap(&odd, &[here("ANTHROPIC_API_KEY", S, R)]).unwrap().0, odd);
    }

    #[test]
    fn a_stand_in_anywhere_a_server_might_repeat_back_is_refused() {
        let misplaced = Err(Misuse::Misplaced("ANTHROPIC_API_KEY".into()));
        // In the address: an error page that quotes the address would hand the key to the app.
        assert_eq!(swapped(&format!("GET /v1/{S} HTTP/1.1\r\nHost: api.anthropic.com")), misplaced);
        assert_eq!(swapped(&format!("GET /v1/models?{S}=1 HTTP/1.1\r\nHost: h")), misplaced);
        assert_eq!(swapped(&format!("GET /v1/models?x={S} HTTP/1.1\r\nHost: h")), misplaced);
        assert_eq!(swapped(&format!("GET /v1/models?key=a{S} HTTP/1.1\r\nHost: h")), misplaced);
        assert_eq!(swapped(&format!("GET /v1/models?key={S}&again={S} HTTP/1.1\r\nHost: h")), misplaced);
        // In a header that is not for credentials, or that a server answers with.
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nOrigin: https://{S}")), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nX-Note: {S}")), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\n{S}: 1")), misplaced);
        // In a credential header but not as the whole credential.
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nx-api-key: {S}, {S}")), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nAuthorization: Bearer x{S}")), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nAuthorization: Digest username=\"{S}\"")), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("user:pre{S}")))), misplaced);
        assert_eq!(swapped(&format!("GET / HTTP/1.1\r\nProxy-Authorization: Basic {}", b64(&format!("user:{S}")))), misplaced);
        // Properly placed, but with a method that asks the server to send the request back.
        assert_eq!(swapped(&format!("TRACE / HTTP/1.1\r\nx-api-key: {S}")), misplaced);
        // One good place does not excuse a bad one.
        assert_eq!(swapped(&format!("GET /{S} HTTP/1.1\r\nx-api-key: {S}")), misplaced);
    }

    #[test]
    fn a_stand_in_bound_for_another_place_is_named_and_stopped() {
        let keys = [elsewhere("ANTHROPIC_API_KEY", S), here("OPENAI_API_KEY", "sk-proj-STANDIN11", "sk-proj-REALREAL1")];
        for head in [format!("POST /collect HTTP/1.1\r\nHost: api.openai.com\r\nX-Stolen: {S}"), format!("GET / HTTP/1.1\r\nx-api-key: {S}"), format!("GET /?k={S} HTTP/1.1"), format!("GET / HTTP/1.1\r\nAuthorization: Basic {}", b64(&format!("x:{S}"))), format!("GET / HTTP/1.1\r\nAuthorization: Basic  {}", b64(&format!("x:{S}")).trim_end_matches('='))] {
            assert_eq!(swap(head.as_bytes(), &keys), Err(Misuse::Elsewhere("ANTHROPIC_API_KEY".to_string())), "{head}");
        }
        // The key that does belong here still works on its own.
        let (out, used) = swap(b"POST /v1/chat HTTP/1.1\r\nAuthorization: Bearer sk-proj-STANDIN11", &keys).unwrap();
        assert!(find(&out, b"sk-proj-REALREAL1").is_some());
        assert_eq!(used, vec!["OPENAI_API_KEY"]);
        // A key Homebody no longer has is said to be lost, not misused.
        let lost = [Swap { name: "X_KEY".into(), stand_in: S.as_bytes().to_vec(), fate: Fate::Lost }];
        assert_eq!(swap(format!("GET / HTTP/1.1\r\nx-api-key: {S}").as_bytes(), &lost), Err(Misuse::Lost("X_KEY".into())));
    }

    #[test]
    fn only_ordinary_keys_and_names_are_taken() {
        assert!(valid_value("sk-ant-api03-abcDEF123_-").is_ok());
        assert!(valid_value("short").is_err());
        assert!(valid_value("has a space in it").is_err());
        assert!(valid_value("line\r\nX-Injected: 1").is_err());
        assert!(valid_value(&"k".repeat(600)).is_err());
        assert!(valid_name("ANTHROPIC_API_KEY") && valid_name("MY_KEY_2"));
        assert!(!valid_name("lower") && !valid_name("2FAST") && !valid_name("A B") && !valid_name(""));
    }

    #[test]
    fn keys_are_kept_in_a_file_only_you_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut v = Vault::open(&path);
        v.set("claude-code", "ANTHROPIC_API_KEY", "sk-ant-REALREAL11").unwrap();
        v.set("codex", "OPENAI_API_KEY", "sk-proj-REALREAL1").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = Vault::open(&path);
        assert_eq!(again.get("claude-code", "ANTHROPIC_API_KEY"), Some("sk-ant-REALREAL11"));
        assert_eq!(again.get("codex", "ANTHROPIC_API_KEY"), None);
        v.remove_app("claude-code").unwrap();
        assert_eq!(Vault::open(&path).get("claude-code", "ANTHROPIC_API_KEY"), None);
        assert_eq!(Vault::open(&path).get("codex", "OPENAI_API_KEY"), Some("sk-proj-REALREAL1"));
    }

    #[test]
    fn a_key_file_that_cannot_be_read_is_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        std::fs::write(&path, b"{ this is not what it should be").unwrap();
        let mut v = Vault::open(&path);
        assert_eq!(v.get("claude-code", "ANTHROPIC_API_KEY"), None);
        assert!(v.set("claude-code", "ANTHROPIC_API_KEY", "sk-ant-REALREAL11").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ this is not what it should be");
        // A short key shows nothing of itself; a long one its last four characters.
        assert_eq!(hint("abcd-1234"), "");
        assert_eq!(hint("sk-ant-api03-0123456789abcdefWXYZ"), "…WXYZ");
    }

    #[test]
    fn the_usual_places_for_known_keys() {
        assert_eq!(usual_places("ANTHROPIC_API_KEY"), vec!["api.anthropic.com"]);
        assert!(usual_places("SOMETHING_ELSE").is_empty());
        assert_eq!(maker("OPENAI_API_KEY"), Some("OpenAI"));
    }
}
