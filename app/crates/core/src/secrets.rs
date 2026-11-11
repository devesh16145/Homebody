//! Recognising secrets in what an app is about to send.
//!
//! An agent that has read a `.env` file or a private key will include it in what it sends to
//! its AI provider without a second thought. While an app's connections are being looked inside
//! (see `inspect`), the content of each request passes through here, and a request carrying
//! something that looks like a secret is stopped before the secret has left.
//!
//! Two rules shape this:
//!
//! - **Only shapes that are almost never anything else.** A private key block, and tokens with a
//!   maker's prefix and a fixed form. A false alarm stops an agent mid-task, so "looks a bit
//!   like a password" is not enough. This catches careless leaks, not a determined one: a
//!   secret that has been encoded, split up or reworded is not recognised.
//! - **Nothing is kept.** The content is looked at as it passes and forgotten. What is written
//!   down is the kind of secret and a fingerprint that cannot be turned back into it, so the
//!   user can say "let this one through" without the secret being stored anywhere.

use sha2::{Digest, Sha256};

/// How many of the most recent bytes are held back while more arrive. Longer than any shape
/// below, so by the time a byte is let go, every secret it could be the start of has been seen
/// in full. Sending a secret in two halves therefore does not get its first half out.
pub const HOLD: usize = 512;

/// A secret found in what was about to be sent.
#[derive(Debug, PartialEq, Clone)]
pub struct Found {
    /// In plain words, with its article: "a GitHub token".
    pub kind: &'static str,
    /// A short fingerprint of it, particular to this computer. Not reversible.
    pub fingerprint: String,
}

struct Token {
    prefix: &'static str,
    /// Characters after the prefix that must follow for it to count.
    min: usize,
    /// Whether "-" and "_" may appear in the body of the token.
    dashes: bool,
    kind: &'static str,
}

/// Tokens with a maker's prefix. Longer prefixes first where one starts with another.
const TOKENS: &[Token] = &[
    Token { prefix: "github_pat_", min: 60, dashes: true, kind: "a GitHub token" },
    Token { prefix: "ghp_", min: 36, dashes: false, kind: "a GitHub token" },
    Token { prefix: "gho_", min: 36, dashes: false, kind: "a GitHub token" },
    Token { prefix: "ghu_", min: 36, dashes: false, kind: "a GitHub token" },
    Token { prefix: "ghs_", min: 36, dashes: false, kind: "a GitHub token" },
    Token { prefix: "ghr_", min: 36, dashes: false, kind: "a GitHub token" },
    Token { prefix: "xoxb-", min: 24, dashes: true, kind: "a Slack token" },
    Token { prefix: "xoxp-", min: 24, dashes: true, kind: "a Slack token" },
    Token { prefix: "xoxa-", min: 24, dashes: true, kind: "a Slack token" },
    Token { prefix: "xoxs-", min: 24, dashes: true, kind: "a Slack token" },
    Token { prefix: "sk_live_", min: 24, dashes: false, kind: "a Stripe key" },
    Token { prefix: "rk_live_", min: 24, dashes: false, kind: "a Stripe key" },
    Token { prefix: "sk-ant-", min: 40, dashes: true, kind: "an Anthropic API key" },
    Token { prefix: "sk-proj-", min: 40, dashes: true, kind: "an OpenAI API key" },
    Token { prefix: "AIza", min: 35, dashes: true, kind: "a Google API key" },
    Token { prefix: "AKIA", min: 16, dashes: false, kind: "an AWS access key" },
];

const KEY_START: &[u8] = b"-----BEGIN ";
const KEY_MARK: &[u8] = b"PRIVATE KEY-----";
/// How much of a private key after its first line goes into the fingerprint.
const KEY_BODY: usize = 32;

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The secret starting exactly at `at`, if there is one. `Ok(None)`: none. `Err(())`: there
/// might be, but more bytes are needed to tell.
fn secret_at(buf: &[u8], at: usize, at_end: bool) -> Result<Option<(&'static str, &[u8])>, ()> {
    let rest = &buf[at..];
    // A private key block of any kind: "-----BEGIN OPENSSH PRIVATE KEY-----" and its relatives.
    if rest.starts_with(KEY_START) {
        let window = &rest[..rest.len().min(KEY_START.len() + 40)];
        if let Some(mark) = window.windows(KEY_MARK.len()).position(|w| w == KEY_MARK) {
            let header_end = mark + KEY_MARK.len();
            let want = header_end + KEY_BODY;
            if rest.len() >= want {
                return Ok(Some(("a private key", &rest[..want])));
            }
            return if at_end { Ok(Some(("a private key", rest))) } else { Err(()) };
        }
        // The first line is not complete yet: it may still turn out to be a private key.
        if !at_end && window.len() < KEY_START.len() + 40 && !window.contains(&b'\n') {
            return Err(());
        }
        return Ok(None);
    }
    if !at_end && KEY_START.starts_with(rest) && !rest.is_empty() {
        return Err(());
    }
    // A token is a word of its own: "task_live_x" is not a Stripe key.
    if at > 0 && word(buf[at - 1]) {
        return Ok(None);
    }
    for t in TOKENS {
        let prefix = t.prefix.as_bytes();
        if rest.starts_with(prefix) {
            let body = &rest[prefix.len()..];
            let run = body.iter().take_while(|b| b.is_ascii_alphanumeric() || (t.dashes && (**b == b'-' || **b == b'_'))).count();
            if run >= t.min {
                return Ok(Some((t.kind, &rest[..prefix.len() + t.min])));
            }
            // Ran to the end of what we have: the rest of it may still be coming.
            return if !at_end && run == body.len() { Err(()) } else { Ok(None) };
        }
        if !at_end && !rest.is_empty() && prefix.starts_with(rest) {
            return Err(());
        }
    }
    Ok(None)
}

/// Looks at the content of one request as it passes.
pub struct Scanner {
    salt: Vec<u8>,
    /// The most recent bytes, kept so that a secret arriving in pieces is still seen whole.
    tail: Vec<u8>,
    /// Fingerprints the user has said may go.
    allowed: Box<dyn Fn(&str) -> bool + Send>,
}

impl Scanner {
    /// `salt` makes fingerprints particular to this computer.
    pub fn new(salt: &str, allowed: impl Fn(&str) -> bool + Send + 'static) -> Scanner {
        Scanner { salt: salt.as_bytes().to_vec(), tail: Vec::new(), allowed: Box::new(allowed) }
    }

    fn fingerprint(&self, secret: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(&self.salt);
        h.update(b"|");
        h.update(secret);
        hex::encode(&h.finalize()[..8])
    }

    fn search(&self, buf: &[u8], at_end: bool) -> Option<Found> {
        for at in 0..buf.len() {
            // Cheap first look: every shape starts with one of these.
            if !matches!(buf[at], b'-' | b'g' | b'x' | b's' | b'r' | b'A') {
                continue;
            }
            if let Ok(Some((kind, bytes))) = secret_at(buf, at, at_end) {
                let fingerprint = self.fingerprint(bytes);
                if !(self.allowed)(&fingerprint) {
                    return Some(Found { kind, fingerprint });
                }
            }
        }
        None
    }

    /// Take the next piece of content. Returns how many of the oldest bytes not yet let go may
    /// now be let go, or the secret that was found (in which case nothing more may go).
    pub fn push(&mut self, data: &[u8]) -> Result<usize, Found> {
        let had = self.tail.len();
        self.tail.extend_from_slice(data);
        if let Some(found) = self.search(&self.tail, false) {
            return Err(found);
        }
        let keep = self.tail.len().min(HOLD);
        let clear = self.tail.len() - keep;
        self.tail.drain(..clear);
        debug_assert!(clear <= had + data.len());
        Ok(clear)
    }

    /// The content has ended. Returns how many bytes may now be let go (all that were held).
    pub fn finish(&mut self) -> Result<usize, Found> {
        if let Some(found) = self.search(&self.tail, true) {
            return Err(found);
        }
        let clear = self.tail.len();
        self.tail.clear();
        Ok(clear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built at run time, so that no file in this repository holds something token-shaped.
    fn github() -> String {
        format!("gh{}_{}", "p", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8")
    }
    fn private_key() -> String {
        format!("-----BEGIN {} PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACD\n-----END OPENSSH PRIVATE KEY-----\n", "OPENSSH")
    }

    /// Feed in pieces; return how much was let go, and what was found.
    fn run(content: &[u8], step: usize, allowed: &'static [&'static str]) -> (usize, Option<Found>) {
        let mut s = Scanner::new("this-computer", move |fp| allowed.contains(&fp));
        let mut gone = 0;
        for piece in content.chunks(step.max(1)) {
            match s.push(piece) {
                Ok(n) => gone += n,
                Err(f) => return (gone, Some(f)),
            }
        }
        match s.finish() {
            Ok(n) => (gone + n, None),
            Err(f) => (gone, Some(f)),
        }
    }

    #[test]
    fn secrets_are_found_however_the_content_arrives_and_none_of_one_gets_out() {
        let cases: Vec<(String, &str)> = vec![
            (github(), "a GitHub token"),
            (private_key(), "a private key"),
            (format!("xox{}-{}", "b", "1234567890-0987654321-AbCdEfGhIjKl"), "a Slack token"),
            (format!("sk_{}_{}", "live", "4eC39HqLyjWDarjtT1zdp7dcABCD"), "a Stripe key"),
            (format!("sk-{}-{}", "ant", "api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-abcdef"), "an Anthropic API key"),
            (format!("AK{}{}", "IA", "IOSFODNN7EXAMPLE"), "an AWS access key"),
            (format!("AI{}{}", "za", "SyA-1234567890abcdefghijklmnopqrstuvw"), "a Google API key"),
        ];
        for (secret, kind) in &cases {
            // In the middle of an ordinary request body, as an agent would send it.
            let before = format!("{{\"messages\":[{{\"role\":\"user\",\"content\":\"{}here is my config: KEY=", "x".repeat(700));
            let content = format!("{before}{secret}\\nDEBUG=1\"}}]}}");
            for step in [1, 2, 5, 17, 100, 511, 512, 513, 4096] {
                let (gone, found) = run(content.as_bytes(), step, &[]);
                let found = found.unwrap_or_else(|| panic!("{kind}, step {step}: not found"));
                assert_eq!(found.kind, *kind, "step {step}");
                assert!(gone <= before.len(), "{kind}, step {step}: {gone} bytes had gone, the secret starts at {}", before.len());
            }
        }
    }

    #[test]
    fn the_same_secret_has_the_same_fingerprint_and_can_be_let_through() {
        let content = format!("token: {} end", github());
        let a = run(content.as_bytes(), 3, &[]).1.unwrap();
        let b = run(content.as_bytes(), 4096, &[]).1.unwrap();
        assert_eq!(a.fingerprint, b.fingerprint, "however it was cut up");
        assert_eq!(a.fingerprint.len(), 16);
        assert!(!content.contains(&a.fingerprint));
        // Another computer gives it another fingerprint.
        let mut other = Scanner::new("another-computer", |_| false);
        assert_ne!(other.push(content.as_bytes()).unwrap_err().fingerprint, a.fingerprint);
        // Let through by its fingerprint: everything goes, and a different secret is still caught.
        let fp: &'static str = Box::leak(a.fingerprint.clone().into_boxed_str());
        let allowed: &'static [&'static str] = Box::leak(vec![fp].into_boxed_slice());
        assert_eq!(run(content.as_bytes(), 7, allowed), (content.len(), None));
        let two = format!("{content} and {}", private_key());
        assert_eq!(run(two.as_bytes(), 7, allowed).1.unwrap().kind, "a private key");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let harmless = [
            "the task_live_updates flag and risk_live_data are fine".to_string(),
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----".to_string(),
            "-----BEGIN PUBLIC KEY-----\nMFkw\n-----END PUBLIC KEY-----".to_string(),
            format!("gh{}_tooshort", "p"),
            "see ghp_ in the docs, and the sk-ant- prefix".to_string(),
            "AKIA is how AWS keys start; MAKIAVELLI0123456789AB is a word".to_string(),
            "a dashed line -----BEGIN and nothing more".to_string(),
            "x".repeat(5000),
            String::new(),
        ];
        for text in &harmless {
            for step in [1, 9, 4096] {
                assert_eq!(run(text.as_bytes(), step, &[]), (text.len(), None), "{}", &text[..text.len().min(40)]);
            }
        }
    }

    #[test]
    fn a_secret_right_at_the_end_or_cut_short_is_still_caught() {
        // The content ends with the first line of a private key and nothing after it.
        let header = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");
        assert_eq!(run(header.as_bytes(), 4, &[]).1.unwrap().kind, "a private key");
        // A token as the very last thing.
        assert!(run(format!("x={}", github()).as_bytes(), 6, &[]).1.is_some());
        // Bytes are let go only once they are HOLD behind, and all of them at the end.
        let mut s = Scanner::new("c", |_| false);
        assert_eq!(s.push(&[b'a'; 100]), Ok(0));
        assert_eq!(s.push(&[b'a'; HOLD]), Ok(100));
        assert_eq!(s.finish(), Ok(HOLD));
    }
}
