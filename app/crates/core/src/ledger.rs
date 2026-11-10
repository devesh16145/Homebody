//! The ledger: an append-only, hash-chained, signed record of every decision.
//!
//! One JSON object per line. Every record carries the hash of the record before it, its own
//! SHA-256 over a canonical form, and an Ed25519 signature of that hash made with a key that
//! belongs to this computer. Changing, removing or reordering any line breaks the chain, and
//! `verify` says where.
//!
//! Record kinds: `ask`, `answer`, `conn`, `end`, `mode`, `trust`, `lockdown`, `fence`, `start`.

use crate::model::ChainCheck;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Canonical JSON: object keys sorted, no whitespace. Written by hand so the result never depends
/// on how a JSON library happens to order maps.
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                write_canonical(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap()),
    }
}

fn hash_hex(body: &Map<String, Value>) -> String {
    let mut h = Sha256::new();
    h.update(canonical(&Value::Object(body.clone())).as_bytes());
    hex::encode(h.finalize())
}

pub struct Ledger {
    path: PathBuf,
    file: File,
    key: SigningKey,
    seq: u64,
    last_hash: String,
}

impl Ledger {
    /// Open (or create) the ledger and its key. Returns the ledger and every existing record, in
    /// order, so the caller can rebuild its view of history with the same code that handles live
    /// records.
    pub fn open(path: &Path, key_path: &Path) -> std::io::Result<(Ledger, Vec<Value>)> {
        let key = load_or_create_key(key_path)?;
        let mut records = Vec::new();
        let mut seq = 0u64;
        let mut last_hash = ZERO_HASH.to_string();
        let mut good_len = 0u64;
        let mut truncate_to: Option<u64> = None;

        if path.exists() {
            let f = File::open(path)?;
            let total = f.metadata()?.len();
            let mut reader = BufReader::new(f);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    break;
                }
                let complete = line.ends_with('\n');
                let parsed = serde_json::from_str::<Value>(line.trim_end()).ok().filter(|v| v.get("seq").and_then(Value::as_u64).is_some() && v.get("hash").and_then(Value::as_str).is_some());
                match parsed {
                    Some(v) if complete => {
                        seq = v["seq"].as_u64().unwrap();
                        last_hash = v["hash"].as_str().unwrap().to_string();
                        good_len += n as u64;
                        records.push(v);
                    }
                    _ => {
                        // A torn final line (crash mid-write, so no newline) is safe to drop: nothing
                        // was chained to it. Anything else is left in place for `verify` to report.
                        if !complete && good_len + n as u64 == total {
                            truncate_to = Some(good_len);
                        }
                        break;
                    }
                }
            }
        }

        let mut file = OpenOptions::new().create(true).read(true).write(true).open(path)?;
        if let Some(len) = truncate_to {
            file.set_len(len)?;
        }
        file.seek(SeekFrom::End(0))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok((Ledger { path: path.to_path_buf(), file, key, seq, last_hash }, records))
    }

    pub fn public_key_b64(&self) -> String {
        b64().encode(self.key.verifying_key().to_bytes())
    }

    pub fn head(&self) -> (u64, String) {
        (self.seq, self.last_hash.clone())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record. `fields` must not contain the reserved keys.
    pub fn append(&mut self, kind: &str, fields: Map<String, Value>) -> std::io::Result<Value> {
        let mut body = fields;
        body.insert("v".into(), Value::from(1));
        body.insert("seq".into(), Value::from(self.seq + 1));
        body.insert("ts".into(), Value::from(crate::now_iso()));
        body.insert("kind".into(), Value::from(kind));
        body.insert("prev".into(), Value::from(self.last_hash.clone()));
        let hash = hash_hex(&body);
        let sig = self.key.sign(&hex::decode(&hash).unwrap());
        body.insert("hash".into(), Value::from(hash.clone()));
        body.insert("sig".into(), Value::from(b64().encode(sig.to_bytes())));
        let record = Value::Object(body);
        let mut line = canonical(&record);
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.seq += 1;
        self.last_hash = hash;
        Ok(record)
    }

    /// Sign arbitrary bytes with the ledger key (proof exports).
    pub fn sign(&self, bytes: &[u8]) -> String {
        b64().encode(self.key.sign(bytes).to_bytes())
    }

    pub fn tail(&self, limit: usize) -> Vec<String> {
        let Ok(f) = File::open(&self.path) else { return vec![] };
        let lines: Vec<String> = BufReader::new(f).lines().map_while(Result::ok).collect();
        let start = lines.len().saturating_sub(limit);
        lines[start..].to_vec()
    }
}

/// Recompute every hash, check every link and every signature. Does not need the private key.
pub fn verify(path: &Path, public_key_b64: &str) -> ChainCheck {
    let checked_at = crate::now_iso();
    let fail = |count: u64, msg: String| ChainCheck { ok: false, count, checked_at: checked_at.clone(), error: Some(msg) };

    let key_bytes: [u8; 32] = match b64().decode(public_key_b64).ok().and_then(|b| b.try_into().ok()) {
        Some(k) => k,
        None => return fail(0, "The public key is not valid.".into()),
    };
    let Ok(vk) = VerifyingKey::from_bytes(&key_bytes) else { return fail(0, "The public key is not valid.".into()) };
    let f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return ChainCheck { ok: true, count: 0, checked_at, error: None },
    };

    let mut prev = ZERO_HASH.to_string();
    let mut expected_seq = 1u64;
    let mut count = 0u64;
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let n = i + 1;
        let Ok(line) = line else { return fail(count, format!("Line {n} could not be read.")) };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&line) else { return fail(count, format!("Line {n} is not a valid record.")) };
        let (Some(hash), Some(sig)) = (obj.remove("hash").and_then(|v| v.as_str().map(str::to_string)), obj.remove("sig").and_then(|v| v.as_str().map(str::to_string))) else {
            return fail(count, format!("Record on line {n} has no hash or signature."));
        };
        let seq = obj.get("seq").and_then(Value::as_u64).unwrap_or(0);
        if seq != expected_seq {
            return fail(count, format!("Record {expected_seq} is missing or out of order (found {seq} on line {n})."));
        }
        if obj.get("prev").and_then(Value::as_str) != Some(prev.as_str()) {
            return fail(count, format!("Record {seq} does not follow the record before it: something was removed or changed."));
        }
        if hash_hex(&obj) != hash {
            return fail(count, format!("Record {seq} was changed after it was written."));
        }
        let sig_ok = b64().decode(&sig).ok().and_then(|b| Signature::from_slice(&b).ok()).zip(hex::decode(&hash).ok()).map(|(s, h)| vk.verify(&h, &s).is_ok()).unwrap_or(false);
        if !sig_ok {
            return fail(count, format!("Record {seq} was not signed by this computer."));
        }
        prev = hash;
        expected_seq += 1;
        count += 1;
    }
    ChainCheck { ok: true, count, checked_at, error: None }
}

fn load_or_create_key(path: &Path) -> std::io::Result<SigningKey> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Ok(SigningKey::from_bytes(&arr));
        }
    }
    let key = SigningKey::generate(&mut rand::rngs::OsRng);
    std::fs::write(path, key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

/// Convenience for building record fields.
#[macro_export]
macro_rules! fields {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m = serde_json::Map::new();
        $( m.insert($k.to_string(), serde_json::json!($v)); )*
        m
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> (Ledger, Vec<Value>) {
        Ledger::open(&dir.join("ledger.jsonl"), &dir.join("ledger.key")).unwrap()
    }

    #[test]
    fn canonical_sorts_keys_at_every_level() {
        let v: Value = serde_json::from_str(r#"{"b":1,"a":{"z":[{"y":1,"x":2}],"c":"s"}}"#).unwrap();
        assert_eq!(canonical(&v), r#"{"a":{"c":"s","z":[{"x":2,"y":1}]},"b":1}"#);
    }

    #[test]
    fn append_then_verify() {
        let dir = tempfile::tempdir().unwrap();
        let (mut l, existing) = open(dir.path());
        assert!(existing.is_empty());
        for i in 0..5 {
            l.append("conn", fields! { "app" => "ollama", "host" => "ollama.com", "n" => i }).unwrap();
        }
        let check = verify(l.path(), &l.public_key_b64());
        assert!(check.ok, "{:?}", check.error);
        assert_eq!(check.count, 5);
    }

    #[test]
    fn reopening_continues_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let pk;
        {
            let (mut l, _) = open(dir.path());
            l.append("mode", fields! { "to" => "private" }).unwrap();
            l.append("mode", fields! { "to" => "watch" }).unwrap();
            pk = l.public_key_b64();
        }
        let (mut l, existing) = open(dir.path());
        assert_eq!(existing.len(), 2);
        assert_eq!(l.public_key_b64(), pk);
        let r = l.append("mode", fields! { "to" => "private" }).unwrap();
        assert_eq!(r["seq"], 3);
        assert!(verify(l.path(), &pk).ok);
    }

    fn tamper(path: &Path, f: impl Fn(Vec<String>) -> Vec<String>) {
        let lines: Vec<String> = std::fs::read_to_string(path).unwrap().lines().map(str::to_string).collect();
        std::fs::write(path, f(lines).join("\n") + "\n").unwrap();
    }

    fn three_records() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let (mut l, _) = open(dir.path());
        l.append("conn", fields! { "host" => "a.example.com", "decision" => "deny" }).unwrap();
        l.append("conn", fields! { "host" => "b.example.com", "decision" => "deny" }).unwrap();
        l.append("conn", fields! { "host" => "c.example.com", "decision" => "deny" }).unwrap();
        let (p, k) = (l.path().to_path_buf(), l.public_key_b64());
        (dir, p, k)
    }

    #[test]
    fn detects_an_edited_record() {
        let (_d, path, pk) = three_records();
        tamper(&path, |mut l| {
            l[1] = l[1].replace("\"deny\"", "\"allow\"");
            l
        });
        let c = verify(&path, &pk);
        assert!(!c.ok);
        assert!(c.error.unwrap().contains("Record 2 was changed"));
    }

    #[test]
    fn detects_a_removed_record() {
        let (_d, path, pk) = three_records();
        tamper(&path, |mut l| {
            l.remove(1);
            l
        });
        assert!(!verify(&path, &pk).ok);
    }

    #[test]
    fn detects_reordering_and_truncation_from_the_front() {
        let (_d, path, pk) = three_records();
        tamper(&path, |mut l| {
            l.swap(0, 1);
            l
        });
        assert!(!verify(&path, &pk).ok);
        let (_d2, path2, pk2) = three_records();
        tamper(&path2, |mut l| {
            l.remove(0);
            l
        });
        assert!(!verify(&path2, &pk2).ok);
    }

    #[test]
    fn detects_a_record_signed_by_another_key() {
        let (_d, path, _pk) = three_records();
        let other = tempfile::tempdir().unwrap();
        let (l2, _) = open(other.path());
        let c = verify(&path, &l2.public_key_b64());
        assert!(!c.ok);
        assert!(c.error.unwrap().contains("not signed by this computer"));
    }

    #[test]
    fn a_torn_last_line_is_dropped_on_open() {
        let (_d, path, pk) = three_records();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"seq\":4,\"kind\":\"conn\",\"ha").unwrap();
        drop(f);
        let (mut l, existing) = Ledger::open(&path, &path.with_file_name("ledger.key")).unwrap();
        assert_eq!(existing.len(), 3);
        l.append("mode", fields! { "to" => "private" }).unwrap();
        let c = verify(&path, &pk);
        assert!(c.ok, "{:?}", c.error);
        assert_eq!(c.count, 4);
    }
}
