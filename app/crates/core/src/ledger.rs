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
    /// The whole log exactly as written, with one signed line in front saying what it is.
    /// See [`verify_export`]. Takes `&self`, so nothing can be appended while it is read.
    pub fn export(&self) -> std::io::Result<Vec<u8>> {
        let (count, head) = self.head();
        let mut header = serde_json::json!({
            "format": EXPORT_FORMAT,
            "exportedAt": crate::now_iso(),
            "publicKey": self.public_key_b64(),
            "records": count,
            "head": head,
            "howToCheck": "Each following line is one record. hash = SHA-256 of the record's canonical JSON (sorted keys, no spaces) without hash and sig; prev = the hash of the record before; sig = Ed25519 over the hash bytes, made with publicKey. This line is signed the same way over its canonical JSON without sig.",
        });
        let sig = self.sign(canonical(&header).as_bytes());
        header["sig"] = Value::String(sig);
        let mut out = serde_json::to_vec(&header).unwrap_or_default();
        out.push(b'\n');
        out.extend(std::fs::read(self.path())?);
        Ok(out)
    }

    pub fn sign(&self, bytes: &[u8]) -> String {
        b64().encode(self.key.sign(bytes).to_bytes())
    }

    /// Up to `limit` records after record number `seq`, exactly as written.
    pub fn lines_after(&self, seq: u64, limit: usize) -> Vec<String> {
        let Ok(f) = File::open(&self.path) else { return vec![] };
        BufReader::new(f).lines().map_while(Result::ok).filter(|l| !l.trim().is_empty()).skip(seq as usize).take(limit).collect()
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
    verify_lines(BufReader::new(f), &vk, 0, checked_at)
}

/// The first line of an exported log: says what the file is and carries the key to check it with.
pub const EXPORT_FORMAT: &str = "homebody-log-v1";

/// Check a log exported with "Export" (a header line, then the records exactly as written).
/// The key is read from the file itself, so a pass means "unchanged since it was signed by the
/// holder of this key"; compare the key with the one on the Proof screen to know whose it is.
/// The public half of the log key kept in this file, if there is one. Reads; never creates.
pub fn public_key_of(key_path: &Path) -> Option<String> {
    let bytes: [u8; 32] = std::fs::read(key_path).ok()?.try_into().ok()?;
    Some(b64().encode(SigningKey::from_bytes(&bytes).verifying_key().to_bytes()))
}

/// Check a proof file (the one "Share proof" saves): that its statement was signed by the key
/// it names and has not been changed since. Returns the proof itself. This says who made the
/// statement, not that it is true; the exported log is what backs it.
pub fn verify_proof(file: &Value) -> Result<Value, String> {
    let proof = file.get("proof").filter(|p| p.is_object()).ok_or("This is not a Homebody proof file.")?;
    if proof.get("format").and_then(Value::as_str) != Some("homebody-proof-v1") {
        return Err("This proof is in a form this version does not know.".into());
    }
    let key = proof.get("publicKey").and_then(Value::as_str).ok_or("The proof does not name the key that signed it.")?;
    let key: [u8; 32] = b64().decode(key).ok().and_then(|b| b.try_into().ok()).ok_or("The key in the proof is not a key.")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "The key in the proof is not a key.".to_string())?;
    let sig = file.get("signature").and_then(Value::as_str).ok_or("The proof is not signed.")?;
    let sig: [u8; 64] = b64().decode(sig).ok().and_then(|b| b.try_into().ok()).ok_or("The signature is not a signature.")?;
    key.verify(canonical(proof).as_bytes(), &Signature::from_bytes(&sig)).map_err(|_| "The statement was changed after it was signed, or was signed by a different key.".to_string())?;
    Ok(proof.clone())
}

pub fn verify_export(path: &Path) -> (ChainCheck, Option<String>) {
    let checked_at = crate::now_iso();
    let fail = |msg: &str| (ChainCheck { ok: false, count: 0, checked_at: checked_at.clone(), error: Some(msg.to_string()) }, None);
    let Ok(f) = File::open(path) else { return fail("The file could not be opened.") };
    let mut reader = BufReader::new(f);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return fail("The file could not be read.");
    }
    let header: Value = serde_json::from_str(first.trim()).unwrap_or(Value::Null);
    // Exports made before the app was called Homebody carry the old name; the format is the same.
    if !matches!(header.get("format").and_then(Value::as_str), Some(f) if f == EXPORT_FORMAT || f == "sandboxai-log-v1") {
        return fail("This is not a Homebody log export.");
    }
    let Some(key) = header.get("publicKey").and_then(Value::as_str).map(str::to_string) else { return fail("The export does not say which key signed it.") };
    let Some(vk) = b64().decode(&key).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).and_then(|k| VerifyingKey::from_bytes(&k).ok()) else { return fail("The public key is not valid.") };
    // The header is signed too, so the record count it promises cannot be edited to hide a cut.
    let mut unsigned = header.clone();
    let sig = unsigned.as_object_mut().and_then(|o| o.remove("sig")).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let header_ok = b64().decode(&sig).ok().and_then(|b| Signature::from_slice(&b).ok()).map(|sg| vk.verify(canonical(&unsigned).as_bytes(), &sg).is_ok()).unwrap_or(false);
    if !header_ok {
        return (ChainCheck { ok: false, count: 0, checked_at, error: Some("The first line of the export was changed after it was written.".into()) }, Some(key));
    }
    let mut check = verify_lines(reader, &vk, 1, checked_at);
    if check.ok && header.get("records").and_then(Value::as_u64) != Some(check.count) {
        check.ok = false;
        check.error = Some(format!("The export says it holds {} records but {} were found: part of it is missing.", header["records"], check.count));
    }
    (check, Some(key))
}

/// Check records that continue a log from a known point: the record numbered `after_seq` had
/// the hash `after_hash` (zero and [`ZERO_HASH`] for the very beginning). This is how a fleet
/// server takes a computer's log piece by piece without ever trusting a piece it cannot link.
/// Returns the number and hash of the last record checked.
pub fn verify_continuation(lines: &[String], public_key_b64: &str, after_seq: u64, after_hash: &str) -> Result<(u64, String), String> {
    let vk = b64().decode(public_key_b64).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).and_then(|k| VerifyingKey::from_bytes(&k).ok()).ok_or("The public key is not valid.")?;
    let text = lines.join("\n");
    let check = verify_from(text.as_bytes(), &vk, 0, crate::now_iso(), after_hash.to_string(), after_seq + 1);
    match check.error {
        Some(e) => Err(e),
        None => {
            let last: Value = lines.iter().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str(l).ok()).unwrap_or(Value::Null);
            match (last.get("seq").and_then(Value::as_u64), last.get("hash").and_then(Value::as_str)) {
                (Some(seq), Some(hash)) => Ok((seq, hash.to_string())),
                _ => Ok((after_seq, after_hash.to_string())),
            }
        }
    }
}

fn verify_lines(reader: impl BufRead, vk: &VerifyingKey, line_offset: usize, checked_at: String) -> ChainCheck {
    verify_from(reader, vk, line_offset, checked_at, ZERO_HASH.to_string(), 1)
}

fn verify_from(reader: impl BufRead, vk: &VerifyingKey, line_offset: usize, checked_at: String, mut prev: String, mut expected_seq: u64) -> ChainCheck {
    let fail = |count: u64, msg: String| ChainCheck { ok: false, count, checked_at: checked_at.clone(), error: Some(msg) };
    let mut count = 0u64;
    for (i, line) in reader.lines().enumerate() {
        let n = i + 1 + line_offset;
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

    #[test]
    fn a_proof_file_is_believed_only_as_it_was_signed() {
        let dir = tempfile::tempdir().unwrap();
        let (ledger, _) = Ledger::open(&dir.path().join("l.jsonl"), &dir.path().join("l.key")).unwrap();
        let body = serde_json::json!({ "format": "homebody-proof-v1", "generatedAt": "2026-11-15T00:00:00Z", "statement": "Since Tuesday, Ollama sent nothing.", "publicKey": ledger.public_key_b64() });
        let file = serde_json::json!({ "proof": body, "signature": ledger.sign(canonical(&body).as_bytes()) });
        assert_eq!(verify_proof(&file).unwrap()["statement"], "Since Tuesday, Ollama sent nothing.");
        // One word changed.
        let mut forged = file.clone();
        forged["proof"]["statement"] = serde_json::json!("Since Monday, Ollama sent nothing.");
        assert!(verify_proof(&forged).is_err());
        // Signed by someone else, naming their own key: it checks out, as theirs. Naming ours: it does not.
        let (other, _) = Ledger::open(&dir.path().join("o.jsonl"), &dir.path().join("o.key")).unwrap();
        let mut theirs = file.clone();
        theirs["signature"] = serde_json::json!(other.sign(canonical(&body).as_bytes()));
        assert!(verify_proof(&theirs).is_err());
        assert!(verify_proof(&serde_json::json!({ "something": "else" })).is_err());
    }


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

    fn exported(dir: &Path) -> PathBuf {
        let (mut l, _) = open(dir);
        for i in 0..3 {
            l.append("conn", fields! { "host" => format!("h{i}.example.com") }).unwrap();
        }
        let out = dir.join("export.jsonl");
        std::fs::write(&out, l.export().unwrap()).unwrap();
        out
    }

    #[test]
    fn an_exported_log_checks_out_on_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let out = exported(dir.path());
        let (check, key) = verify_export(&out);
        assert!(check.ok, "{:?}", check.error);
        assert_eq!(check.count, 3);
        assert!(key.is_some());
    }

    #[test]
    fn an_exported_log_that_was_cut_or_edited_fails() {
        let dir = tempfile::tempdir().unwrap();
        let out = exported(dir.path());
        let original = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = original.lines().collect();

        // The last record removed: every remaining record is still valid, but the count is not.
        std::fs::write(&out, lines[..3].join("\n") + "\n").unwrap();
        let (check, _) = verify_export(&out);
        assert!(!check.ok && check.error.unwrap().contains("missing"));

        // The same cut, with the header's count edited to match: the header's signature fails.
        let header = lines[0].replace("\"records\":3", "\"records\":2");
        assert_ne!(header, lines[0]);
        std::fs::write(&out, format!("{header}\n{}\n{}\n", lines[1], lines[2])).unwrap();
        let (check, _) = verify_export(&out);
        assert!(!check.ok && check.error.unwrap().contains("first line"));

        // A record edited.
        std::fs::write(&out, original.replace("h1.example.com", "h9.example.com")).unwrap();
        assert!(!verify_export(&out).0.ok);

        // Not an export at all.
        std::fs::write(&out, lines[1..].join("\n")).unwrap();
        assert!(verify_export(&out).0.error.unwrap().contains("not a Homebody log export"));
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
