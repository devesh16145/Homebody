//! Belonging to an organisation's fleet: its rules come here, this computer's log goes there.
//!
//! Homebody keeps everything on the computer it runs on. A fleet is the one exception, and only
//! by joining: an organisation runs its own server (`homebody-fleet`, in this repository), and a
//! computer that joins it
//!
//! - takes the organisation's rules, which then come before the user's own. They arrive signed
//!   with the fleet's key, which is pinned when the computer joins, so a server that is
//!   impersonated later cannot change them;
//! - sends its log there, record by record, exactly as written. Each record is already signed
//!   with this computer's key and chained to the one before, so the server can check that
//!   nothing was changed or left out, and so can anyone it hands the log to.
//!
//! Nothing here runs unless the computer has joined. The words on both sides of the
//! conversation (the policy, the join answer) are defined in this file so the server and the
//! app cannot drift apart.

use crate::hostname;
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const POLICY_FORMAT: &str = "homebody-fleet-policy-v1";
pub const MAX_RULES: usize = 2000;
/// How many log records go in one upload.
pub const BATCH: usize = 500;

/// One rule set by the organisation. `app` is an app's id, or "*" for every app.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrgRule {
    pub app: String,
    pub host: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrgPolicy {
    pub format: String,
    /// Goes up by one with every change. A computer never goes back to a lower one.
    pub version: u64,
    pub updated_at: String,
    /// The organisation's name, shown to the user.
    pub name: String,
    /// Some(false): unknown places are blocked without asking. None: the user decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<bool>,
    /// Protection cannot be switched to Watch or Off, per app or as a whole.
    #[serde(default)]
    pub lock_mode: bool,
    #[serde(default)]
    pub rules: Vec<OrgRule>,
}

impl OrgPolicy {
    /// Is this something a computer may apply? The error is a sentence for whoever wrote it.
    pub fn check(&self) -> Result<(), String> {
        if self.format != POLICY_FORMAT {
            return Err("This is not a Homebody fleet policy.".into());
        }
        if self.rules.len() > MAX_RULES {
            return Err(format!("A policy can hold at most {MAX_RULES} rules."));
        }
        if self.name.trim().is_empty() || self.name.len() > 80 || self.name.chars().any(|c| c.is_control()) {
            return Err("Give the organisation a short name.".into());
        }
        for r in &self.rules {
            let app_ok = r.app == "*" || (!r.app.is_empty() && r.app.len() <= 64 && r.app.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
            if !app_ok {
                return Err(format!("\"{}\" is not an app. Use an app's id, or * for every app.", r.app));
            }
            if !hostname::valid_pattern(&r.host) {
                return Err(format!("\"{}\" is not a place. Use a name like example.com or *.example.com.", r.host));
            }
        }
        Ok(())
    }

    /// The rules that apply to one app, with the names in their normal form.
    pub fn rules_for(&self, app_id: &str) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        for r in self.rules.iter().filter(|r| r.app == "*" || r.app == app_id) {
            let host = hostname::normalize_pattern(&r.host);
            match out.iter_mut().find(|(h, _)| *h == host) {
                // The same place twice: a block wins, as everywhere else.
                Some((_, enabled)) => *enabled = *enabled && r.enabled,
                None => out.push((host, r.enabled)),
            }
        }
        out
    }
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Check `{"policy": {...}, "signature": "..."}` against the fleet's pinned key.
pub fn verify_policy(signed: &Value, fleet_key_b64: &str) -> Result<OrgPolicy, String> {
    let body = signed.get("policy").ok_or("The server's answer holds no policy.")?;
    let sig = signed.get("signature").and_then(Value::as_str).ok_or("The policy is not signed.")?;
    let key = b64().decode(fleet_key_b64).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()).and_then(|k| VerifyingKey::from_bytes(&k).ok()).ok_or("The fleet's key on this computer is not valid.")?;
    let sig = b64().decode(sig).ok().and_then(|b| Signature::from_slice(&b).ok()).ok_or("The policy's signature is not valid.")?;
    key.verify(crate::ledger::canonical(body).as_bytes(), &sig).map_err(|_| "The policy was not signed by the fleet this computer joined, so it was not applied.".to_string())?;
    let policy: OrgPolicy = serde_json::from_value(body.clone()).map_err(|e| format!("The policy could not be read: {e}"))?;
    policy.check()?;
    Ok(policy)
}

/// What this computer remembers about the fleet it joined. Kept in the data folder, private.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    pub url: String,
    pub device_id: String,
    pub secret: String,
    /// The fleet's public key, pinned at joining.
    pub fleet_key: String,
    pub org: String,
    /// A certificate file to trust for the server, when it does not use a public one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
    pub joined_at: String,
    /// The last record the server confirmed.
    #[serde(default)]
    pub sent_seq: u64,
    #[serde(default)]
    pub last_sync: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

pub fn membership_file() -> PathBuf {
    crate::paths::data_dir().join("fleet.json")
}

impl Membership {
    pub fn load() -> Option<Membership> {
        serde_json::from_slice(&std::fs::read(membership_file()).ok()?).ok()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = membership_file();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(tmp, path)
    }

    pub fn forget() {
        let _ = std::fs::remove_file(membership_file());
    }
}

/// A fleet server's address, cleaned. It has to be encrypted unless it is this computer itself.
pub fn clean_url(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/').to_string();
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).ok_or("The server's address starts with https://")?;
    let host = rest.split(['/', ':']).next().unwrap_or("");
    let local = host == "localhost" || host == "127.0.0.1";
    if url.starts_with("http://") && !local {
        return Err("The server's address has to start with https:// so that what is sent to it is encrypted.".into());
    }
    if rest.is_empty() || rest.contains(char::is_whitespace) || rest.contains(['"', '\'', '\\', '@']) || url.len() > 300 {
        return Err("That does not look like a server's address.".into());
    }
    Ok(url)
}

/// The host of a cleaned address, for the log.
pub fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("").to_string()
}

/// Settings an administrator pushed to this Mac with a management profile: where the fleet is
/// and a code to join it with. Read through `plutil`, which is on every Mac.
pub fn managed_join() -> Option<(String, String)> {
    let path = std::env::var("HOMEBODY_MANAGED_PREFS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/Library/Managed Preferences/app.homebody.mac.plist"));
    read_managed(&path)
}

fn read_managed(path: &Path) -> Option<(String, String)> {
    if !path.is_file() {
        return None;
    }
    let out = std::process::Command::new("/usr/bin/plutil").args(["-convert", "json", "-o", "-"]).arg(path).output().ok().filter(|o| o.status.success())?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let url = clean_url(v.get("FleetURL")?.as_str()?).ok()?;
    let token = v.get("JoinCode")?.as_str()?.trim().to_string();
    (!token.is_empty() && token.len() <= 200).then_some((url, token))
}

/// One request to the fleet server, made with the system's `curl` like the app's other own
/// requests. The secret goes in a private file, never on the command line where other programs
/// on the computer could read it.
pub async fn request(method: &str, url: &str, bearer: Option<&str>, body: Option<&Value>, ca_file: Option<&str>) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt;
    let run = crate::paths::ensure_data_dir().map_err(|e| e.to_string())?.join("run");
    std::fs::create_dir_all(&run).map_err(|e| e.to_string())?;
    let header_file = run.join(format!("fleet-{}.headers", rand::random::<u64>()));
    let mut headers = String::from("Content-Type: application/json\nUser-Agent: homebody-fleet-client\n");
    if let Some(b) = bearer {
        headers.push_str(&format!("Authorization: Bearer {b}\n"));
    }
    std::fs::write(&header_file, headers).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&header_file, std::fs::Permissions::from_mode(0o600));
    }
    let curl = if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" };
    let mut cmd = tokio::process::Command::new(curl);
    cmd.args(["-sS", "--max-time", "30", "--proto", "=https,http", "-X", method, "-H"]).arg(format!("@{}", header_file.display())).args(["-w", "\n%{http_code}"]);
    if let Some(ca) = ca_file {
        cmd.arg("--cacert").arg(ca);
    }
    if body.is_some() {
        cmd.args(["--data-binary", "@-"]);
    }
    cmd.arg(url).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let result = async {
        let mut child = cmd.spawn().map_err(|e| format!("The request could not be made: {e}"))?;
        if let (Some(mut stdin), Some(b)) = (child.stdin.take(), body) {
            let bytes = serde_json::to_vec(b).unwrap_or_default();
            let _ = stdin.write_all(&bytes).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(40), child.wait_with_output()).await.map_err(|_| "The server did not answer in time.".to_string())?.map_err(|e| e.to_string())?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            return Err(format!("The server could not be reached. {}", why.lines().last().unwrap_or("").trim_start_matches("curl: ").trim()));
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').unwrap_or((&text, "0"));
        let reply: Value = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
        if code.trim().starts_with('2') {
            Ok(reply)
        } else {
            Err(reply.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("The server answered with an error ({}).", code.trim())))
        }
    }
    .await;
    let _ = std::fs::remove_file(&header_file);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    fn policy(rules: Vec<OrgRule>) -> OrgPolicy {
        OrgPolicy { format: POLICY_FORMAT.into(), version: 3, updated_at: crate::now_iso(), name: "Example Org".into(), ask: Some(false), lock_mode: true, rules }
    }

    fn rule(app: &str, host: &str, enabled: bool) -> OrgRule {
        OrgRule { app: app.into(), host: host.into(), enabled }
    }

    #[test]
    fn a_policy_is_applied_only_when_the_pinned_key_signed_exactly_it() {
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        let pinned = b64().encode(key.verifying_key().to_bytes());
        let body = serde_json::to_value(policy(vec![rule("*", "*.example.com", false)])).unwrap();
        let sig = b64().encode(key.sign(crate::ledger::canonical(&body).as_bytes()).to_bytes());
        let signed = json!({ "policy": body, "signature": sig });
        assert_eq!(verify_policy(&signed, &pinned).unwrap().version, 3);
        // One character changed after signing.
        let mut tampered = signed.clone();
        tampered["policy"]["rules"][0]["enabled"] = json!(true);
        assert!(verify_policy(&tampered, &pinned).unwrap_err().contains("not signed by the fleet"));
        // Signed, but by somebody else.
        let other = b64().encode(SigningKey::generate(&mut rand::rngs::OsRng).verifying_key().to_bytes());
        assert!(verify_policy(&signed, &other).is_err());
        assert!(verify_policy(&json!({ "policy": body }), &pinned).is_err());
    }

    #[test]
    fn a_policy_with_something_that_is_not_a_place_or_an_app_is_refused() {
        assert!(policy(vec![rule("*", "example.com", true), rule("claude-code", "*.example.org", false)]).check().is_ok());
        assert!(policy(vec![rule("*", "*", true)]).check().is_err());
        assert!(policy(vec![rule("*", "*.com", true)]).check().is_err());
        assert!(policy(vec![rule("Claude Code", "example.com", true)]).check().is_err());
        assert!(policy(vec![rule("a;b", "example.com", true)]).check().is_err());
        let mut p = policy(vec![]);
        p.format = "something-else".into();
        assert!(p.check().is_err());
    }

    #[test]
    fn rules_for_every_app_and_for_one_app_are_merged_and_a_block_wins() {
        let p = policy(vec![rule("*", "Example.COM", true), rule("claude-code", "example.com", false), rule("ollama", "models.example.com", true)]);
        assert_eq!(p.rules_for("claude-code"), vec![("example.com".to_string(), false)]);
        assert_eq!(p.rules_for("ollama"), vec![("example.com".to_string(), true), ("models.example.com".to_string(), true)]);
    }

    #[test]
    fn the_server_address_has_to_be_encrypted_unless_it_is_this_computer() {
        assert_eq!(clean_url(" https://fleet.example.com/ ").unwrap(), "https://fleet.example.com");
        assert_eq!(clean_url("http://127.0.0.1:8470").unwrap(), "http://127.0.0.1:8470");
        assert!(clean_url("http://fleet.example.com").is_err());
        assert!(clean_url("fleet.example.com").is_err());
        assert!(clean_url("https://user@fleet.example.com").is_err());
        assert_eq!(host_of("https://fleet.example.com:8443/x"), "fleet.example.com");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_management_profile_says_where_to_join() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.homebody.mac.plist");
        std::fs::write(&p, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>FleetURL</key><string>https://fleet.example.com</string><key>JoinCode</key><string>abc-123</string></dict></plist>\n").unwrap();
        assert_eq!(read_managed(&p), Some(("https://fleet.example.com".to_string(), "abc-123".to_string())));
        assert_eq!(read_managed(&dir.path().join("missing.plist")), None);
        std::fs::write(&p, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>FleetURL</key><string>http://fleet.example.com</string><key>JoinCode</key><string>x</string></dict></plist>\n").unwrap();
        assert_eq!(read_managed(&p), None, "an unencrypted address is not followed");
    }
}
