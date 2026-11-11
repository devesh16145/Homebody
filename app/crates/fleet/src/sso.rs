//! Signing in through the organisation's identity provider (OpenID Connect).
//!
//! The standard "authorization code" flow with PKCE: the browser is sent to the provider, comes
//! back with a one-time code, and this server exchanges the code for a signed statement of who
//! the person is. The statement's signature is checked against the provider's published keys,
//! and its issuer, audience, expiry and nonce against what this server expects. Which role the
//! person gets comes from the groups the provider says they are in.
//!
//! Only RS256 statements are accepted. Requests to the provider are made with the system's
//! `curl`; the client secret is handed to it on standard input, never on its command line.

use crate::api::Fleet;
use crate::http::{Request, Response};
use crate::store::{random_token, Person};
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Sso {
    pub label: String,
    pub issuer: String,
    pub client_id: String,
    client_secret: String,
    pub admin_group: String,
    pub auditor_group: String,
    /// "viewer", or "none": people in neither group may not sign in.
    pub default_role: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    /// state -> (nonce, code verifier, when)
    pending: Mutex<HashMap<String, (String, String, Instant)>>,
    keys: Mutex<Option<(Value, Instant)>>,
}

fn url_b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// An address at the provider: encrypted, unless it is on this computer (for tests).
fn acceptable(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

/// One request with curl. Everything, including any secret, goes in on standard input.
async fn fetch(url: &str, form: Option<&str>) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt;
    if !acceptable(url) {
        return Err("The identity provider's address is not encrypted.".into());
    }
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut config = format!("url = {}\nsilent\nshow-error\nmax-time = 15\nproto = \"=https,http\"\n", quote(url));
    if let Some(f) = form {
        config.push_str(&format!("data = {}\n", quote(f)));
    }
    let mut child = tokio::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" }).args(["-K", "-"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true).spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes()).await;
    }
    let out = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output()).await.map_err(|_| "The identity provider did not answer in time.".to_string())?.map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("The identity provider could not be reached: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "The identity provider's answer was not understood.".to_string())
}

pub struct SsoConfig {
    pub label: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub admin_group: String,
    pub auditor_group: String,
    pub default_role: String,
}

impl Sso {
    /// Read the provider's own description of itself and keep what is needed from it.
    pub async fn discover(c: SsoConfig) -> Result<Sso, String> {
        let issuer = c.issuer.trim_end_matches('/').to_string();
        let doc = fetch(&format!("{issuer}/.well-known/openid-configuration"), None).await?;
        let field = |k: &str| doc.get(k).and_then(Value::as_str).filter(|u| acceptable(u)).map(str::to_string).ok_or_else(|| format!("The identity provider's description has no usable {k}."));
        if doc.get("issuer").and_then(Value::as_str).map(|i| i.trim_end_matches('/')) != Some(issuer.as_str()) {
            return Err("The identity provider describes itself under a different address than the one given.".into());
        }
        Ok(Sso { label: c.label, issuer, client_id: c.client_id, client_secret: c.client_secret, admin_group: c.admin_group, auditor_group: c.auditor_group, default_role: c.default_role, authorization_endpoint: field("authorization_endpoint")?, token_endpoint: field("token_endpoint")?, jwks_uri: field("jwks_uri")?, pending: Mutex::new(HashMap::new()), keys: Mutex::new(None) })
    }

    async fn jwks(&self, refresh: bool) -> Result<Value, String> {
        if !refresh {
            if let Some((k, at)) = &*self.keys.lock().unwrap() {
                if at.elapsed() < Duration::from_secs(3600) {
                    return Ok(k.clone());
                }
            }
        }
        let k = fetch(&self.jwks_uri, None).await?;
        *self.keys.lock().unwrap() = Some((k.clone(), Instant::now()));
        Ok(k)
    }

    /// Where to send the browser, and what to remember until it comes back.
    /// Returns (where to send the browser, the state to remember in that browser).
    fn begin(&self, redirect: &str) -> (String, String) {
        use sha2::Digest;
        let (state, nonce, verifier) = (random_token(24), random_token(24), random_token(48));
        let challenge = url_b64().encode(sha2::Sha256::digest(verifier.as_bytes()));
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, (_, _, at)| at.elapsed() < Duration::from_secs(600));
        // Anyone can start a sign-in; only so many are remembered at once.
        if pending.len() >= 2000 {
            pending.clear();
        }
        pending.insert(state.clone(), (nonce.clone(), verifier, Instant::now()));
        (format!("{}{}response_type=code&client_id={}&redirect_uri={}&scope={}&state={state}&nonce={nonce}&code_challenge={challenge}&code_challenge_method=S256", self.authorization_endpoint, if self.authorization_endpoint.contains('?') { "&" } else { "?" }, enc(&self.client_id), enc(redirect), enc("openid profile email groups")), state)
    }

    /// Check a signed statement from the provider. Returns its claims.
    async fn verify(&self, token: &str, nonce: &str) -> Result<Value, String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("The identity provider's statement is not in the expected form.".into());
        }
        let decode = |p: &str| url_b64().decode(p.trim_end_matches('=')).map_err(|_| "The identity provider's statement could not be read.".to_string());
        let header: Value = serde_json::from_slice(&decode(parts[0])?).map_err(|e| e.to_string())?;
        if header.get("alg").and_then(Value::as_str) != Some("RS256") {
            return Err("The identity provider signed its statement in a way this server does not accept (only RS256).".into());
        }
        let kid = header.get("kid").and_then(Value::as_str);
        let signature = decode(parts[2])?;
        let signed = format!("{}.{}", parts[0], parts[1]);
        let check = |keys: &Value| -> bool {
            keys.get("keys").and_then(Value::as_array).into_iter().flatten().filter(|k| k.get("kty").and_then(Value::as_str) == Some("RSA") && (kid.is_none() || k.get("kid").and_then(Value::as_str) == kid)).any(|k| {
                let part = |n: &str| k.get(n).and_then(Value::as_str).and_then(|v| url_b64().decode(v.trim_end_matches('=')).ok());
                match (part("n"), part("e")) {
                    (Some(n), Some(e)) => ring::signature::RsaPublicKeyComponents { n: &n, e: &e }.verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature).is_ok(),
                    _ => false,
                }
            })
        };
        // The provider may have changed its keys since they were last read: look once more.
        if !check(&self.jwks(false).await?) && !check(&self.jwks(true).await?) {
            return Err("The statement was not signed by the identity provider's published keys.".into());
        }
        let claims: Value = serde_json::from_slice(&decode(parts[1])?).map_err(|e| e.to_string())?;
        let aud_ok = match claims.get("aud") {
            Some(Value::String(a)) => *a == self.client_id,
            Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(self.client_id.as_str())),
            _ => false,
        };
        let now = chrono::Utc::now().timestamp();
        if claims.get("iss").and_then(Value::as_str).map(|i| i.trim_end_matches('/')) != Some(self.issuer.as_str()) {
            return Err("The statement names a different identity provider.".into());
        }
        if !aud_ok {
            return Err("The statement was made for a different application.".into());
        }
        if claims.get("exp").and_then(Value::as_i64).map(|e| e <= now).unwrap_or(true) {
            return Err("The statement has expired.".into());
        }
        if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return Err("The statement does not belong to this sign-in.".into());
        }
        Ok(claims)
    }

    /// The role a person gets from the groups the provider lists for them.
    pub fn role_for(&self, claims: &Value) -> Option<&'static str> {
        let groups: Vec<&str> = claims.get("groups").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        if !self.admin_group.is_empty() && groups.contains(&self.admin_group.as_str()) {
            Some("admin")
        } else if !self.auditor_group.is_empty() && groups.contains(&self.auditor_group.as_str()) {
            Some("auditor")
        } else if self.default_role == "viewer" {
            Some("viewer")
        } else {
            None
        }
    }
}

fn page(message: &str) -> Response {
    let safe = message.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    Response::bytes("text/html; charset=utf-8", format!("<!doctype html><meta charset=utf-8><title>Homebody fleet</title><body style=\"font:16px system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\"><h1 style=\"font-size:1.3rem\">Sign-in did not finish</h1><p>{safe}</p><p><a href=\"/\">Back to the console</a></p>").into_bytes())
}

pub async fn handle(fleet: &Fleet, req: &Request, parts: &[&str]) -> Response {
    let Some(sso) = &fleet.sso else { return Response::error(404, "Sign-in through an identity provider is not set up on this server.") };
    let redirect = format!("{}/auth/callback", fleet.public_url);
    match (req.method.as_str(), parts) {
        ("GET", ["login"]) => {
            // The sign-in is tied to the browser that started it: the answer is only taken
            // from a browser that carries this cookie, so a link cannot sign someone in as
            // somebody else.
            let (to, state) = sso.begin(&redirect);
            let cookie = format!("hb_sso={state}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=600{}", if fleet.secure { "; Secure" } else { "" });
            Response { status: 302, content_type: "text/plain", body: vec![], headers: vec![("Location".into(), to), ("Set-Cookie".into(), cookie)] }
        }
        ("GET", ["callback"]) => {
            if let Some(e) = req.query.get("error") {
                return page(&format!("The identity provider said: {}", e.chars().filter(|c| c.is_ascii_alphanumeric() || " _-".contains(*c)).take(80).collect::<String>()));
            }
            let (Some(code), Some(state)) = (req.query.get("code"), req.query.get("state")) else { return page("The identity provider's answer was incomplete.") };
            if req.cookie("hb_sso").as_deref() != Some(state.as_str()) {
                return page("This sign-in was not started in this browser. Start again from the console.");
            }
            let Some((nonce, verifier, at)) = sso.pending.lock().unwrap().remove(state) else { return page("This sign-in was not started here, or was already used. Start again.") };
            if at.elapsed() > Duration::from_secs(600) {
                return page("This sign-in took too long. Start again.");
            }
            let form = format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&client_secret={}&code_verifier={verifier}", enc(code), enc(&redirect), enc(&sso.client_id), enc(&sso.client_secret));
            let reply = match fetch(&sso.token_endpoint, Some(&form)).await {
                Ok(r) => r,
                Err(e) => return page(&e),
            };
            let Some(token) = reply.get("id_token").and_then(Value::as_str) else { return page("The identity provider did not say who you are.") };
            let claims = match sso.verify(token, &nonce).await {
                Ok(c) => c,
                Err(e) => return page(&e),
            };
            // An address the provider says is not verified is not a name to trust.
            let email = claims.get("email").and_then(Value::as_str).filter(|_| claims.get("email_verified").and_then(Value::as_bool) != Some(false));
            let name = email.or_else(|| claims.get("preferred_username").and_then(Value::as_str)).or_else(|| claims.get("sub").and_then(Value::as_str)).unwrap_or("").to_string();
            let Some(role) = sso.role_for(&claims) else { return page("You are signed in at the identity provider, but you are not in a group that may use this console.") };
            // The name is used exactly as given or not at all: dropping characters could turn
            // one person's name into another's.
            if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c)) {
                return page("The identity provider gave a name for you that this console cannot use (letters, digits and . _ - @ only).");
            }
            let store = &fleet.store;
            {
                let mut people = store.people.lock().unwrap();
                match people.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&name)) {
                    Some(p) if !p.active => return page("Your access to this console has been switched off."),
                    // Somebody who signs in here with a password is a different kind of account;
                    // the identity provider does not get to take it over or change its role.
                    Some(p) if !p.hash.is_empty() => return page("There is already an account with this name that signs in with a password. Ask an administrator to remove it first."),
                    Some(p) => p.role = role.to_string(),
                    // No password: this person can only come in through the identity provider.
                    None => people.push(Person { name: name.clone(), role: role.to_string(), salt: String::new(), hash: String::new(), active: true }),
                }
            }
            let _ = store.save_people();
            store.audit(&name, "signed in", json!({ "through": sso.label, "role": role }));
            let secret = fleet.sessions.start(&name);
            let mut r = crate::api::signed_in(fleet, &secret);
            r.status = 302;
            r.headers.push(("Location".into(), "/".into()));
            r
        }
        _ => Response::error(404, "There is nothing at this address."),
    }
}
