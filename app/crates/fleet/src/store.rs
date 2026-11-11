//! What the server keeps, in plain files in one folder, so it can be read, backed up and moved
//! with ordinary tools.
//!
//! ```text
//! fleet.key            the fleet's signing key (never leaves this folder)
//! settings.json        the organisation's name, how long logs are kept
//! policy.json          the rules, with their version
//! people.json          who may sign in to the console, and as what
//! codes.json           codes computers join with
//! devices/<id>/device.json   one joined computer
//! devices/<id>/log.jsonl     its log, exactly as it sent it, checked on arrival
//! audit.jsonl          what people did in the console: its own signed, chained log
//! ```

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use homebody_core::fleet::{OrgPolicy, POLICY_FORMAT};
use homebody_core::ledger::{self, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn random_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Compare two secrets' digests without the time taken saying where they differ.
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(s.as_bytes()))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub name: String,
    /// Records older than this many days are removed from the server. 0: kept for ever.
    #[serde(default)]
    pub retention_days: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    /// "admin" (everything), "auditor" (read logs and rules, export), "viewer" (computers only).
    pub role: String,
    pub salt: String,
    pub hash: String,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JoinCode {
    pub id: String,
    /// SHA-256 of the code. The code itself is shown once, when it is made.
    pub hash: String,
    #[serde(default)]
    pub team: String,
    pub created_at: String,
    pub expires_at: String,
    /// How many more computers may join with it.
    pub uses_left: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub user: String,
    pub os: String,
    pub version: String,
    #[serde(default)]
    pub team: String,
    pub public_key: String,
    pub secret_hash: String,
    pub joined_at: String,
    #[serde(default)]
    pub last_seen: Option<String>,
    /// The last record held: its number and hash. The next piece has to continue from here.
    #[serde(default)]
    pub head_seq: u64,
    #[serde(default = "zero_hash")]
    pub head_hash: String,
    /// The record before the first one still kept (after old records were removed).
    #[serde(default)]
    pub base_seq: u64,
    #[serde(default = "zero_hash")]
    pub base_hash: String,
    /// The version of the rules it last fetched.
    #[serde(default)]
    pub policy_version: u64,
}

fn zero_hash() -> String {
    ledger::ZERO_HASH.to_string()
}

pub struct Store {
    pub dir: PathBuf,
    key: SigningKey,
    pub settings: Mutex<Settings>,
    pub policy: Mutex<OrgPolicy>,
    pub people: Mutex<Vec<Person>>,
    pub codes: Mutex<Vec<JoinCode>>,
    pub devices: Mutex<HashMap<String, Device>>,
    audit: Mutex<Ledger>,
    /// The last full check of each computer's stored log, and the record it had reached then.
    checked: Mutex<HashMap<String, (u64, u64, Result<u64, String>)>>,
}

/// The most one computer's log may take on this server. A joined computer cannot fill the disk.
pub const DEVICE_LOG_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default())?;
    private(&tmp);
    std::fs::rename(tmp, path)
}

fn private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl Store {
    /// Open the folder, making what is missing. The second value is true on the very first start.
    pub fn open(dir: &Path, name: &str) -> std::io::Result<(Store, bool)> {
        std::fs::create_dir_all(dir.join("devices"))?;
        private(dir);
        let key_path = dir.join("fleet.key");
        let key = match std::fs::read(&key_path).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) {
            Some(k) => SigningKey::from_bytes(&k),
            None => {
                let k = SigningKey::generate(&mut rand::rngs::OsRng);
                std::fs::write(&key_path, k.to_bytes())?;
                private(&key_path);
                k
            }
        };
        let first = !dir.join("settings.json").exists();
        let settings: Settings = read_json(&dir.join("settings.json")).unwrap_or(Settings { name: name.to_string(), retention_days: 0 });
        let policy: OrgPolicy = read_json(&dir.join("policy.json")).unwrap_or(OrgPolicy { format: POLICY_FORMAT.into(), version: 1, updated_at: homebody_core::now_iso(), name: settings.name.clone(), ask: None, lock_mode: false, rules: vec![] });
        let people: Vec<Person> = read_json(&dir.join("people.json")).unwrap_or_default();
        let codes: Vec<JoinCode> = read_json(&dir.join("codes.json")).unwrap_or_default();
        let mut devices = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir.join("devices")) {
            for e in entries.flatten() {
                if let Some(d) = read_json::<Device>(&e.path().join("device.json")) {
                    devices.insert(d.id.clone(), d);
                }
            }
        }
        let (audit, _) = Ledger::open(&dir.join("audit.jsonl"), &dir.join("audit.key"))?;
        private(&dir.join("audit.key"));
        let store = Store { dir: dir.to_path_buf(), key, settings: Mutex::new(settings), policy: Mutex::new(policy), people: Mutex::new(people), codes: Mutex::new(codes), devices: Mutex::new(devices), audit: Mutex::new(audit), checked: Mutex::new(HashMap::new()) };
        if first {
            store.save_settings()?;
            store.save_policy()?;
        }
        Ok((store, first))
    }

    pub fn public_key(&self) -> String {
        b64().encode(self.key.verifying_key().to_bytes())
    }

    /// The rules as computers receive them: the policy and the fleet's signature over it.
    pub fn signed_policy(&self) -> Value {
        let body = serde_json::to_value(&*self.policy.lock().unwrap()).unwrap_or(Value::Null);
        let signature = b64().encode(self.key.sign(ledger::canonical(&body).as_bytes()).to_bytes());
        json!({ "policy": body, "signature": signature })
    }

    pub fn save_settings(&self) -> std::io::Result<()> {
        write_json(&self.dir.join("settings.json"), &*self.settings.lock().unwrap())
    }
    pub fn save_policy(&self) -> std::io::Result<()> {
        write_json(&self.dir.join("policy.json"), &*self.policy.lock().unwrap())
    }
    pub fn save_people(&self) -> std::io::Result<()> {
        write_json(&self.dir.join("people.json"), &*self.people.lock().unwrap())
    }
    pub fn save_codes(&self) -> std::io::Result<()> {
        write_json(&self.dir.join("codes.json"), &*self.codes.lock().unwrap())
    }

    pub fn device_dir(&self, id: &str) -> PathBuf {
        self.dir.join("devices").join(id)
    }

    pub fn save_device(&self, d: &Device) -> std::io::Result<()> {
        let dir = self.device_dir(&d.id);
        std::fs::create_dir_all(&dir)?;
        write_json(&dir.join("device.json"), d)
    }

    pub fn remove_device(&self, id: &str) -> std::io::Result<()> {
        self.devices.lock().unwrap().remove(id);
        std::fs::remove_dir_all(self.device_dir(id))
    }

    /// Write down something a person did in the console.
    pub fn audit(&self, who: &str, what: &str, detail: Value) {
        let mut fields = serde_json::Map::new();
        fields.insert("who".into(), json!(who));
        fields.insert("what".into(), json!(what));
        fields.insert("detail".into(), detail);
        let _ = self.audit.lock().unwrap().append("console", fields);
    }

    pub fn audit_export(&self) -> Vec<u8> {
        self.audit.lock().unwrap().export().unwrap_or_default()
    }

    /// Take the next piece of a computer's log. Every record must be signed by that computer
    /// and continue exactly from the last one held; otherwise nothing of the piece is kept.
    pub fn append_records(&self, device_id: &str, after: u64, lines: &[String]) -> Result<u64, String> {
        let mut devices = self.devices.lock().unwrap();
        let d = devices.get_mut(device_id).ok_or("Unknown computer.")?;
        if after != d.head_seq {
            return Err(format!("This server holds records up to {} for this computer; send what follows that.", d.head_seq));
        }
        if lines.is_empty() {
            return Ok(d.head_seq);
        }
        if lines.len() > homebody_core::fleet::BATCH * 2 || lines.iter().any(|l| l.contains('\n') || l.len() > 64 * 1024) {
            return Err("That is not a piece of a log.".into());
        }
        let (seq, hash) = ledger::verify_continuation(lines, &d.public_key, d.head_seq, &d.head_hash)?;
        let dir = self.device_dir(device_id);
        let held = std::fs::metadata(dir.join("log.jsonl")).map(|m| m.len()).unwrap_or(0);
        if held + lines.iter().map(|l| l.len() as u64 + 1).sum::<u64>() > DEVICE_LOG_LIMIT {
            return Err("This computer's log has reached the most this server keeps for one computer. Ask the administrator to set a shorter keeping period.".into());
        }
        let mut text = lines.join("\n");
        text.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("log.jsonl")).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        d.head_seq = seq;
        d.head_hash = hash;
        d.last_seen = Some(homebody_core::now_iso());
        let snapshot = d.clone();
        drop(devices);
        self.save_device(&snapshot).map_err(|e| e.to_string())?;
        Ok(seq)
    }

    /// A computer's stored log, as lines.
    pub fn log_lines(&self, device_id: &str) -> Vec<String> {
        std::fs::read_to_string(self.device_dir(device_id).join("log.jsonl")).map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()).unwrap_or_default()
    }

    /// Check a stored log again from the first record still kept. Used for the console's
    /// "intact" mark and before anything is exported.
    pub fn verify_device(&self, d: &Device) -> Result<u64, String> {
        let lines = self.log_lines(&d.id);
        let (seq, hash) = ledger::verify_continuation(&lines, &d.public_key, d.base_seq, &d.base_hash)?;
        if seq != d.head_seq || hash != d.head_hash {
            return Err("The stored log does not end where this computer's record says it should.".into());
        }
        Ok(lines.len() as u64)
    }

    /// The same check, remembered until the log grows or is cut: the console asks often, and
    /// reading a long log again every time would let one busy computer slow everyone down.
    pub fn verified(&self, d: &Device) -> Result<u64, String> {
        if let Some((head, base, result)) = self.checked.lock().unwrap().get(&d.id) {
            if *head == d.head_seq && *base == d.base_seq {
                return result.clone();
            }
        }
        let result = self.verify_device(d);
        self.checked.lock().unwrap().insert(d.id.clone(), (d.head_seq, d.base_seq, result.clone()));
        result
    }

    /// Remove records older than the retention period. The hash of the last record removed is
    /// kept, so what remains can still be checked link by link.
    pub fn apply_retention(&self) -> usize {
        let days = self.settings.lock().unwrap().retention_days;
        if days == 0 {
            return 0;
        }
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let ids: Vec<String> = self.devices.lock().unwrap().keys().cloned().collect();
        let mut removed = 0;
        for id in ids {
            let lines = self.log_lines(&id);
            let old = lines.iter().take_while(|l| serde_json::from_str::<Value>(l).ok().and_then(|v| v.get("ts").and_then(Value::as_str).map(|t| t < cutoff.as_str())).unwrap_or(false)).count();
            if old == 0 {
                continue;
            }
            let Some(last) = serde_json::from_str::<Value>(&lines[old - 1]).ok() else { continue };
            let (Some(seq), Some(hash)) = (last.get("seq").and_then(Value::as_u64), last.get("hash").and_then(Value::as_str)) else { continue };
            let mut rest = lines[old..].join("\n");
            if !rest.is_empty() {
                rest.push('\n');
            }
            let path = self.device_dir(&id).join("log.jsonl");
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, rest).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                continue;
            }
            let snapshot = {
                let mut devices = self.devices.lock().unwrap();
                let Some(d) = devices.get_mut(&id) else { continue };
                d.base_seq = seq;
                d.base_hash = hash.to_string();
                d.clone()
            };
            let _ = self.save_device(&snapshot);
            removed += old;
        }
        removed
    }
}
