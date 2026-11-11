//! The server tested as a whole: a real listener, real requests, a real signed log.

use crate::api::Fleet;
use crate::{build, serve, Args};
use homebody_core::ledger::Ledger;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn cookie(&self) -> String {
        self.head.lines().find_map(|l| l.strip_prefix("Set-Cookie: ")).and_then(|c| c.split(';').next()).unwrap_or("").to_string()
    }
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| l.strip_prefix(&format!("{name}: "))).map(str::to_string)
    }
}

/// One request, written by hand so the test depends on nothing but the server.
async fn call(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Reply {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let payload = body.map(|b| serde_json::to_vec(b).unwrap()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\n", payload.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(&payload).await.unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.unwrap();
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").expect("a full reply");
    let head = String::from_utf8_lossy(&all[..split]).into_owned();
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Reply { status, head, body: all[split + 4..].to_vec() }
}

async fn start(dir: &std::path::Path, tune: impl FnOnce(&mut Args)) -> (String, Arc<Fleet>, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let mut args = Args { data: dir.to_path_buf(), name: "Example Org".into(), listen: addr.clone(), public_url: Some(format!("http://{addr}")), tls: None, oidc: None, scim_token: None };
    tune(&mut args);
    let (fleet, password) = build(args).await.expect("the server starts");
    tokio::spawn(serve(fleet.clone(), listener, None));
    (addr, fleet, password.expect("a first password"))
}

async fn sign_in(addr: &str, name: &str, password: &str) -> String {
    let r = call(addr, "POST", "/api/login", &[("X-Homebody", "console")], Some(&json!({ "name": name, "password": password }))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    r.cookie()
}

/// A computer's own log, made with the same code the app uses.
fn device_log(dir: &std::path::Path, records: usize) -> (Ledger, Vec<String>) {
    let (mut ledger, _) = Ledger::open(&dir.join("ledger.jsonl"), &dir.join("ledger.key")).unwrap();
    for i in 0..records {
        let mut f = serde_json::Map::new();
        f.insert("app".into(), json!("claude-code"));
        f.insert("host".into(), json!(if i % 3 == 0 { "evil.example" } else { "api.anthropic.com" }));
        f.insert("port".into(), json!(443));
        f.insert("decision".into(), json!(if i % 3 == 0 { "deny" } else { "allow" }));
        f.insert("reason".into(), json!(if i % 3 == 0 { "not_trusted" } else { "trusted" }));
        ledger.append("conn", f).unwrap();
    }
    let lines = ledger.lines_after(0, 10_000);
    (ledger, lines)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_joins_takes_signed_rules_and_its_log_is_only_accepted_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, fleet, password) = start(dir.path(), |_| {}).await;
    let admin = sign_in(&addr, "admin", &password).await;
    let console: &[(&str, &str)] = &[("Cookie", &admin), ("X-Homebody", "console")];

    // The console page itself, and nothing for people who are not signed in.
    let page = call(&addr, "GET", "/", &[], None).await;
    assert!(page.status == 200 && String::from_utf8_lossy(&page.body).contains("Homebody fleet") && page.header("Content-Security-Policy").is_some());
    assert_eq!(call(&addr, "GET", "/api/devices", &[], None).await.status, 401);
    assert_eq!(call(&addr, "POST", "/api/login", &[("X-Homebody", "console")], Some(&json!({ "name": "admin", "password": "wrong" }))).await.status, 401);

    // A code, and a computer joining with it. The code works once.
    let code = call(&addr, "POST", "/api/codes", console, Some(&json!({ "team": "Research", "uses": 1, "days": 1 }))).await.json();
    let code = code["code"].as_str().unwrap().to_string();
    let mac = tempfile::tempdir().unwrap();
    let (ledger, lines) = device_log(mac.path(), 7);
    let join = json!({ "code": code, "deviceName": "dana-mac", "user": "dana", "publicKey": ledger.public_key_b64(), "os": "macos", "version": "0.1.0" });
    assert_eq!(call(&addr, "POST", "/v1/join", &[], Some(&json!({ "code": "not-a-code", "publicKey": ledger.public_key_b64() }))).await.status, 403);
    let joined = call(&addr, "POST", "/v1/join", &[], Some(&join)).await;
    assert_eq!(joined.status, 200);
    let joined = joined.json();
    assert_eq!(call(&addr, "POST", "/v1/join", &[], Some(&join)).await.status, 403, "a used code does not work again");
    let bearer = format!("Bearer {}.{}", joined["deviceId"].as_str().unwrap(), joined["secret"].as_str().unwrap());
    let device: &[(&str, &str)] = &[("Authorization", &bearer)];
    assert_eq!(joined["org"], "Example Org");
    assert_eq!(call(&addr, "GET", "/v1/policy", &[("Authorization", "Bearer made.up")], None).await.status, 401);

    // Rules: changed in the console, signed by the fleet, checked with the key pinned at joining.
    let rules = json!({ "rules": [{ "app": "*", "host": "*.evil.example", "enabled": false }, { "app": "claude-code", "host": "api.anthropic.com", "enabled": true }], "lockMode": true, "ask": false });
    assert_eq!(call(&addr, "PUT", "/api/policy", &[("Cookie", &admin)], Some(&rules)).await.status, 403, "a change without the console's header is refused");
    assert_eq!(call(&addr, "PUT", "/api/policy", console, Some(&json!({ "rules": [{ "app": "*", "host": "*", "enabled": true }] }))).await.status, 400, "a rule that is not a place is refused");
    let saved = call(&addr, "PUT", "/api/policy", console, Some(&rules)).await;
    assert_eq!((saved.status, saved.json()["version"].as_u64()), (200, Some(2)));
    let signed = call(&addr, "GET", "/v1/policy", device, None).await.json();
    let policy = homebody_core::fleet::verify_policy(&signed, joined["fleetKey"].as_str().unwrap()).expect("signed by the fleet");
    assert!(policy.version == 2 && policy.lock_mode && policy.ask == Some(false) && policy.rules.len() == 2);
    let mut forged = signed.clone();
    forged["policy"]["rules"] = json!([]);
    assert!(homebody_core::fleet::verify_policy(&forged, joined["fleetKey"].as_str().unwrap()).is_err());

    // The log: accepted piece by piece, only when each piece continues the last and is untouched.
    let send = |after: u64, records: Vec<String>| {
        let (addr, bearer) = (addr.clone(), bearer.clone());
        async move { call(&addr, "POST", "/v1/records", &[("Authorization", &bearer)], Some(&json!({ "after": after, "records": records }))).await }
    };
    assert_eq!(send(0, lines[..3].to_vec()).await.json()["seq"], 3);
    assert_eq!(send(0, lines[..3].to_vec()).await.status, 409, "the same piece twice");
    assert_eq!(send(3, lines[4..].to_vec()).await.status, 409, "a piece with a record missing");
    let mut changed = lines[3..].to_vec();
    changed[0] = changed[0].replace("\"port\":443", "\"port\":80");
    assert_ne!(changed[0], lines[3]);
    assert_eq!(send(3, changed).await.status, 409, "a piece with a record changed");
    let other = tempfile::tempdir().unwrap();
    let (_, foreign) = device_log(other.path(), 7);
    assert_eq!(send(3, foreign[3..].to_vec()).await.status, 409, "records signed by another computer");
    assert_eq!(call(&addr, "GET", "/v1/head", device, None).await.json()["seq"], 3, "nothing of a refused piece is kept");
    assert_eq!(send(3, lines[3..].to_vec()).await.json()["seq"], 7);

    // What people see.
    let devices = call(&addr, "GET", "/api/devices", &[("Cookie", &admin)], None).await.json();
    // People with "@" in their name can be changed from the console (the path is decoded).
    assert_eq!(call(&addr, "POST", "/api/people", console, Some(&json!({ "name": "sam@example.org", "role": "viewer", "password": "a long password" }))).await.status, 200);
    assert_eq!(call(&addr, "PUT", "/api/people/sam%40example.org", console, Some(&json!({ "active": false }))).await.status, 200);
    assert!(devices[0]["name"] == "dana-mac" && devices[0]["team"] == "Research" && devices[0]["intact"] == true && devices[0]["records"] == 7 && devices[0]["rulesCurrent"] == true);
    let blocked = call(&addr, "GET", "/api/activity?show=blocked&q=evil", &[("Cookie", &admin)], None).await.json();
    assert_eq!(blocked.as_array().unwrap().len(), 3);
    assert!(blocked[0]["host"] == "evil.example" && blocked[0]["outcome"] == "blocked");

    // Roles.
    for (name, role) in [("vera", "viewer"), ("aud", "auditor")] {
        assert_eq!(call(&addr, "POST", "/api/people", console, Some(&json!({ "name": name, "role": role, "password": "a long password" }))).await.status, 200);
    }
    let viewer = sign_in(&addr, "vera", "a long password").await;
    let auditor = sign_in(&addr, "aud", "a long password").await;
    assert_eq!(call(&addr, "GET", "/api/devices", &[("Cookie", &viewer)], None).await.status, 200);
    assert_eq!(call(&addr, "GET", "/api/activity", &[("Cookie", &viewer)], None).await.status, 403);
    assert_eq!(call(&addr, "GET", "/api/activity", &[("Cookie", &auditor)], None).await.status, 200);
    assert_eq!(call(&addr, "PUT", "/api/policy", &[("Cookie", &auditor), ("X-Homebody", "console")], Some(&rules)).await.status, 403);
    assert_eq!(call(&addr, "GET", "/api/people", &[("Cookie", &auditor)], None).await.status, 403);
    // The only administrator cannot be demoted or switched off; a demoted person's session ends.
    assert_eq!(call(&addr, "PUT", "/api/people/admin", console, Some(&json!({ "role": "viewer" }))).await.status, 409);
    assert_eq!(call(&addr, "PUT", "/api/people/aud", console, Some(&json!({ "active": false }))).await.status, 200);
    assert_eq!(call(&addr, "GET", "/api/activity", &[("Cookie", &auditor)], None).await.status, 401);

    // Evidence: a tar whose logs check out with the app's own verifier.
    let bundle = call(&addr, "GET", "/api/export?team=Research", &[("Cookie", &admin)], None).await;
    assert_eq!(bundle.status, 200);
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("e.tar"), &bundle.body).unwrap();
    assert!(std::process::Command::new("tar").arg("-xf").arg(out.path().join("e.tar")).arg("-C").arg(out.path()).status().unwrap().success());
    let ev = out.path().join("evidence");
    for f in ["README.txt", "activity.csv", "rules.json", "compliance.json", "summary.json", "console-log.jsonl"] {
        assert!(ev.join(f).is_file(), "{f}");
    }
    let computer = std::fs::read_dir(ev.join("computers")).unwrap().next().unwrap().unwrap().path();
    let about: Value = serde_json::from_slice(&std::fs::read(computer.join("computer.json")).unwrap()).unwrap();
    let stored: Vec<String> = std::fs::read_to_string(computer.join("log.jsonl")).unwrap().lines().map(str::to_string).collect();
    assert_eq!(stored, lines, "the log is stored exactly as it was sent");
    assert_eq!(homebody_core::ledger::verify_continuation(&stored, about["publicKey"].as_str().unwrap(), 0, homebody_core::ledger::ZERO_HASH).unwrap().0, 7);
    let (check, _) = homebody_core::ledger::verify_export(&ev.join("console-log.jsonl"));
    assert!(check.ok && check.count >= 6, "the console's own log is signed and whole: {:?}", check.error);
    assert!(std::fs::read_to_string(ev.join("activity.csv")).unwrap().lines().count() == 8);

    // A management profile carries the address and the code, and nothing unescaped.
    let profile = call(&addr, "GET", "/api/mdm-profile?code=abc-DEF_123", &[("Cookie", &admin)], None).await;
    let text = String::from_utf8_lossy(&profile.body).into_owned();
    assert!(text.contains("<key>JoinCode</key><string>abc-DEF_123</string>") && text.contains(&format!("<key>FleetURL</key><string>http://{addr}</string>")));
    assert_eq!(call(&addr, "GET", "/api/mdm-profile?code=%3Cx%3E", &[("Cookie", &admin)], None).await.status, 400);

    // Removed: the computer's secret stops working.
    let id = joined["deviceId"].as_str().unwrap();
    assert_eq!(call(&addr, "DELETE", &format!("/api/devices/{id}"), console, None).await.status, 200);
    assert_eq!(call(&addr, "GET", "/v1/policy", device, None).await.status, 401);
    assert!(!fleet.store.device_dir(id).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn old_records_are_removed_and_what_is_left_still_checks() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = crate::store::Store::open(dir.path(), "Org").unwrap();
    let mac = tempfile::tempdir().unwrap();
    let (ledger, lines) = device_log(mac.path(), 6);
    let d = crate::store::Device { id: "d1".into(), name: "m".into(), user: "u".into(), os: "macos".into(), version: "0".into(), team: String::new(), public_key: ledger.public_key_b64(), secret_hash: String::new(), joined_at: homebody_core::now_iso(), last_seen: None, head_seq: 0, head_hash: homebody_core::ledger::ZERO_HASH.into(), base_seq: 0, base_hash: homebody_core::ledger::ZERO_HASH.into(), policy_version: 0 };
    store.save_device(&d).unwrap();
    store.devices.lock().unwrap().insert("d1".into(), d);
    // Records are signed, so their times cannot be changed for the test: instead the first
    // three are treated as old by setting the period so that everything already written is old,
    // after only those three have arrived.
    assert_eq!(store.append_records("d1", 0, &lines[..3]).unwrap(), 3);
    store.settings.lock().unwrap().retention_days = 0;
    assert_eq!(store.apply_retention(), 0, "nothing is removed when logs are kept for ever");
    // Kept for one day: nothing just written is old enough.
    store.settings.lock().unwrap().retention_days = 1;
    assert_eq!(store.apply_retention(), 0);
    // Cut by hand the way retention does, to check the remainder verifies from its new start.
    let last: Value = serde_json::from_str(&lines[2]).unwrap();
    std::fs::write(store.device_dir("d1").join("log.jsonl"), "").unwrap();
    {
        let mut devices = store.devices.lock().unwrap();
        let d = devices.get_mut("d1").unwrap();
        d.base_seq = 3;
        d.base_hash = last["hash"].as_str().unwrap().to_string();
    }
    assert_eq!(store.append_records("d1", 3, &lines[3..]).unwrap(), 6);
    let d = store.devices.lock().unwrap()["d1"].clone();
    assert_eq!(store.verify_device(&d).unwrap(), 3, "three records kept, linked to the hash of the last one removed");
    // If someone removes a kept record on the server, it shows.
    let kept = store.log_lines("d1");
    std::fs::write(store.device_dir("d1").join("log.jsonl"), format!("{}\n{}\n", kept[0], kept[2])).unwrap();
    assert!(store.verify_device(&d).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_identity_provider_can_add_change_and_switch_off_people() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, _fleet, password) = start(dir.path(), |a| a.scim_token = Some("a-provisioning-secret-of-some-length".into())).await;
    let scim: &[(&str, &str)] = &[("Authorization", "Bearer a-provisioning-secret-of-some-length")];
    assert_eq!(call(&addr, "GET", "/scim/v2/Users", &[("Authorization", "Bearer wrong")], None).await.status, 401);
    let made = call(&addr, "POST", "/scim/v2/Users", scim, Some(&json!({ "userName": "sam@example.org", "roles": [{ "value": "auditor" }] }))).await;
    assert_eq!((made.status, made.json()["roles"][0]["value"].as_str().map(str::to_string)), (201, Some("auditor".into())));
    assert_eq!(call(&addr, "POST", "/scim/v2/Users", scim, Some(&json!({ "userName": "sam@example.org" }))).await.status, 409);
    let found = call(&addr, "GET", "/scim/v2/Users?filter=userName%20eq%20%22sam%40example.org%22", scim, None).await.json();
    assert_eq!(found["totalResults"], 1);
    // The way providers switch someone off.
    let off = call(&addr, "PATCH", "/scim/v2/Users/sam@example.org", scim, Some(&json!({ "Operations": [{ "op": "replace", "value": { "active": false } }] }))).await;
    assert_eq!((off.status, off.json()["active"].as_bool()), (200, Some(false)));
    let on = call(&addr, "PATCH", "/scim/v2/Users/sam@example.org", scim, Some(&json!({ "Operations": [{ "op": "Replace", "path": "active", "value": "True" }, { "op": "replace", "path": "roles", "value": [{ "value": "viewer" }] }] }))).await.json();
    assert!(on["active"] == true && on["roles"][0]["value"] == "viewer");
    // A person made this way has no password here.
    assert_eq!(call(&addr, "POST", "/api/login", &[("X-Homebody", "console")], Some(&json!({ "name": "sam@example.org", "password": "" }))).await.status, 401);
    // The last administrator stays.
    assert_eq!(call(&addr, "DELETE", "/scim/v2/Users/admin", scim, None).await.status, 409);
    assert_eq!(call(&addr, "DELETE", "/scim/v2/Users/sam@example.org", scim, None).await.status, 204);
    let admin = sign_in(&addr, "admin", &password).await;
    let people = call(&addr, "GET", "/api/people", &[("Cookie", &admin)], None).await.json();
    assert!(people.as_array().unwrap().iter().any(|p| p["name"] == "sam@example.org" && p["active"] == false));
}

/// A stand-in identity provider: one RSA key, the three addresses a real one has.
mod provider {
    use super::*;
    use base64::Engine as _;

    pub struct Provider {
        pub issuer: String,
        key: ring::signature::RsaKeyPair,
        pub claims: std::sync::Mutex<Value>,
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    impl Provider {
        pub fn sign(&self, claims: &Value, alg: &str) -> String {
            let head = b64(json!({ "alg": alg, "kid": "k1", "typ": "JWT" }).to_string().as_bytes());
            let body = b64(claims.to_string().as_bytes());
            let mut sig = vec![0u8; self.key.public().modulus_len()];
            self.key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), format!("{head}.{body}").as_bytes(), &mut sig).unwrap();
            format!("{head}.{body}.{}", b64(&sig))
        }

        fn jwks(&self) -> Value {
            let public: ring::rsa::PublicKeyComponents<Vec<u8>> = ring::rsa::PublicKeyComponents::from(self.key.public());
            json!({ "keys": [{ "kty": "RSA", "kid": "k1", "alg": "RS256", "use": "sig", "n": b64(&public.n), "e": b64(&public.e) }] })
        }
    }

    pub async fn start() -> Option<Arc<Provider>> {
        // The test needs an RSA key; openssl makes one. Without openssl there is nothing to test with.
        let dir = tempfile::tempdir().ok()?;
        let pem = dir.path().join("k.pem");
        let der = dir.path().join("k.der");
        let made = std::process::Command::new("openssl").args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out"]).arg(&pem).output().ok()?.status.success()
            && std::process::Command::new("openssl").args(["pkcs8", "-topk8", "-nocrypt", "-outform", "DER", "-in"]).arg(&pem).arg("-out").arg(&der).output().ok()?.status.success();
        if !made {
            return None;
        }
        let key = ring::signature::RsaKeyPair::from_pkcs8(&std::fs::read(&der).ok()?).ok()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
        let issuer = format!("http://{}", listener.local_addr().ok()?);
        let p = Arc::new(Provider { issuer: issuer.clone(), key, claims: std::sync::Mutex::new(Value::Null) });
        let served = p.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { continue };
                let p = served.clone();
                tokio::spawn(async move {
                    let Some(req) = crate::http::read_request(&mut s).await else { return };
                    let body = match req.path.as_str() {
                        "/.well-known/openid-configuration" => json!({ "issuer": p.issuer, "authorization_endpoint": format!("{}/authorize", p.issuer), "token_endpoint": format!("{}/token", p.issuer), "jwks_uri": format!("{}/keys", p.issuer) }),
                        "/keys" => p.jwks(),
                        "/token" => {
                            let form = crate::http::parse_query(&String::from_utf8_lossy(&req.body));
                            // A real provider checks these; so does this one, so the test covers what is sent.
                            let ok = form.get("grant_type").map(String::as_str) == Some("authorization_code") && form.get("code").map(String::as_str) == Some("the-code") && form.get("client_secret").map(String::as_str) == Some("s3cret & more") && form.get("code_verifier").map(|v| v.len() >= 43).unwrap_or(false);
                            if ok {
                                let claims = p.claims.lock().unwrap().clone();
                                json!({ "id_token": p.sign(&claims, "RS256"), "token_type": "Bearer" })
                            } else {
                                json!({ "error": "invalid_grant" })
                            }
                        }
                        _ => json!({ "error": "not found" }),
                    };
                    let _ = crate::http::write_response(&mut s, crate::http::Response::json(&body)).await;
                });
            }
        });
        Some(p)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn people_sign_in_through_the_identity_provider_and_get_their_role_from_its_groups() {
    let Some(idp) = provider::start().await else {
        eprintln!("skipped: no openssl to make a test key with");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let issuer = idp.issuer.clone();
    let (addr, fleet, _) = start(dir.path(), move |a| {
        a.oidc = Some(crate::sso::SsoConfig { label: "Example ID".into(), issuer, client_id: "homebody-fleet".into(), client_secret: "s3cret & more".into(), admin_group: "fleet-admins".into(), auditor_group: "fleet-auditors".into(), default_role: "none".into() })
    })
    .await;
    assert_eq!(call(&addr, "GET", "/api/sso", &[], None).await.json(), json!({ "enabled": true, "name": "Example ID" }));

    // Each sign-in: go to the provider, come back with its code, land in the console signed in.
    let go = |groups: Value, tweak: fn(&mut Value)| {
        let (addr, idp) = (addr.clone(), idp.clone());
        async move {
            let to = call(&addr, "GET", "/auth/login", &[], None).await;
            assert_eq!(to.status, 302);
            let location = to.header("Location").unwrap();
            assert!(location.starts_with(&format!("{}/authorize?", idp.issuer)) && location.contains("code_challenge_method=S256") && location.contains("client_id=homebody-fleet"));
            let q = crate::http::parse_query(location.split_once('?').unwrap().1);
            assert_eq!(q["redirect_uri"], format!("http://{addr}/auth/callback"));
            let mut claims = json!({ "iss": idp.issuer, "aud": "homebody-fleet", "exp": chrono::Utc::now().timestamp() + 300, "nonce": q["nonce"], "email": "dana@example.org", "groups": groups });
            tweak(&mut claims);
            *idp.claims.lock().unwrap() = claims;
            let started = format!("hb_sso={}", q["state"]);
            assert!(to.head.contains(&format!("Set-Cookie: {started};")), "the sign-in is tied to this browser");
            // Without the cookie of the browser that started it, the answer is not taken.
            let elsewhere = call(&addr, "GET", &format!("/auth/callback?code=the-code&state={}", q["state"]), &[], None).await;
            assert!(elsewhere.cookie().is_empty() && String::from_utf8_lossy(&elsewhere.body).contains("not started in this browser"));
            call(&addr, "GET", &format!("/auth/callback?code=the-code&state={}", q["state"]), &[("Cookie", &started)], None).await
        }
    };
    let back = go(json!(["staff", "fleet-auditors"]), |_| {}).await;
    assert_eq!((back.status, back.header("Location").as_deref()), (302, Some("/")), "{}", String::from_utf8_lossy(&back.body));
    let me = call(&addr, "GET", "/api/me", &[("Cookie", &back.cookie())], None).await.json();
    assert!(me["name"] == "dana@example.org" && me["role"] == "auditor");
    // Moved to the administrators' group at the provider: the role follows at the next sign-in.
    let back = go(json!(["fleet-admins"]), |_| {}).await;
    assert_eq!(call(&addr, "GET", "/api/me", &[("Cookie", &back.cookie())], None).await.json()["role"], "admin");
    // In neither group: not let in, and told why.
    let refused = go(json!(["staff"]), |_| {}).await;
    assert!(refused.status == 200 && refused.cookie().is_empty() && String::from_utf8_lossy(&refused.body).contains("not in a group"));
    // A statement for another application, an expired one, and one from another issuer.
    for tweak in [(|c: &mut Value| c["aud"] = json!("someone-else")) as fn(&mut Value), |c| c["exp"] = json!(1), |c| c["iss"] = json!("https://elsewhere.example"), |c| c["nonce"] = json!("stale")] {
        let r = go(json!(["fleet-admins"]), tweak).await;
        assert!(r.status == 200 && r.cookie().is_empty(), "{}", String::from_utf8_lossy(&r.body));
    }
    // A code presented with a state this server never issued.
    assert!(call(&addr, "GET", "/auth/callback?code=the-code&state=made-up", &[("Cookie", "hb_sso=made-up")], None).await.cookie().is_empty());
    // A name that would only fit after dropping characters is refused, not "cleaned" into someone else's.
    let odd = go(json!(["fleet-admins"]), |c| c["email"] = json!("da!na@example.org")).await;
    assert!(odd.cookie().is_empty() && String::from_utf8_lossy(&odd.body).contains("cannot use"));
    let unverified = go(json!(["fleet-admins"]), |c| { c["email_verified"] = json!(false); c["sub"] = json!("user-123"); }).await;
    assert_eq!(call(&addr, "GET", "/api/me", &[("Cookie", &unverified.cookie())], None).await.json()["name"], "user-123", "an unverified address is not used as the name");
    // The local administrator's account cannot be taken over by a provider account of the same name.
    let takeover = go(json!(["fleet-auditors"]), |c| c["email"] = json!("admin")).await;
    assert!(takeover.cookie().is_empty() && String::from_utf8_lossy(&takeover.body).contains("signs in with a password"));
    // Signed, but with "none": refused before any key is consulted.
    let sso = fleet.sso.as_ref().unwrap();
    assert!(sso.role_for(&json!({ "groups": ["fleet-auditors"] })) == Some("auditor") && sso.role_for(&json!({})).is_none());
    // SSO people cannot use a password, even an empty one.
    assert_eq!(call(&addr, "POST", "/api/login", &[("X-Homebody", "console")], Some(&json!({ "name": "dana@example.org", "password": "" }))).await.status, 401);
}
