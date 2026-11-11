//! Every address the server answers, in one place.
//!
//! `/v1/…` is for joined computers (they show their device secret). `/api/…` is for people
//! signed in to the console (a session cookie, and a role that allows the thing asked).
//! Anything that changes something must also carry the header `X-Homebody: console`, which a
//! page on another site cannot add, so a signed-in person cannot be tricked into a change.

use crate::auth::{self, allows, Sessions};
use crate::http::{Request, Response};
use crate::store::{random_token, sha256_hex, Device, JoinCode, Store};
use crate::tar;
use homebody_core::fleet::{OrgPolicy, OrgRule, POLICY_FORMAT};
use homebody_core::now_iso;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Fleet {
    pub store: Store,
    pub sessions: Sessions,
    /// The address computers and browsers reach this server at, for links it hands out.
    pub public_url: String,
    pub secure: bool,
    pub sso: Option<crate::sso::Sso>,
    pub scim_token_hash: Option<String>,
}

const CONSOLE: &str = include_str!("console.html");

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn short(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn fail(e: impl ToString) -> Response {
    Response::error(500, &e.to_string())
}

pub async fn handle(fleet: Arc<Fleet>, req: Request) -> Response {
    let path = req.path.clone();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (req.method.as_str(), parts.as_slice()) {
        ("GET", [""]) => Response::bytes("text/html; charset=utf-8", CONSOLE.as_bytes().to_vec()).with("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src data:; form-action 'self'; base-uri 'none'".into()),
        ("GET", ["health"]) => Response::json(&json!({ "ok": true })),
        (_, ["v1", ..]) => device_api(&fleet, &req, &parts[1..]),
        (_, ["auth", ..]) => crate::sso::handle(&fleet, &req, &parts[1..]).await,
        (_, ["scim", "v2", ..]) => crate::scim::handle(&fleet, &req, &parts[2..]),
        (_, ["api", ..]) => console_api(&fleet, &req, &parts[1..]),
        _ => Response::error(404, "There is nothing at this address."),
    }
}

// ---------------------------------------------------------------------------------------------
// Computers
// ---------------------------------------------------------------------------------------------

fn device_api(fleet: &Fleet, req: &Request, parts: &[&str]) -> Response {
    let store = &fleet.store;
    if let ("POST", ["join"]) = (req.method.as_str(), parts) {
        return join(fleet, req);
    }
    let Some(id) = auth::device(store, req.header("authorization")) else {
        return Response::error(401, "This computer is not known to this fleet. It may have been removed; join again with a new code.");
    };
    match (req.method.as_str(), parts) {
        ("GET", ["policy"]) => {
            let signed = store.signed_policy();
            let version = signed["policy"]["version"].as_u64().unwrap_or(0);
            let snapshot = {
                let mut devices = store.devices.lock().unwrap();
                devices.get_mut(&id).map(|d| {
                    d.policy_version = version;
                    d.last_seen = Some(now_iso());
                    d.clone()
                })
            };
            if let Some(d) = snapshot {
                let _ = store.save_device(&d);
            }
            Response::json(&signed)
        }
        ("GET", ["head"]) => {
            let devices = store.devices.lock().unwrap();
            let d = &devices[&id];
            Response::json(&json!({ "seq": d.head_seq, "hash": d.head_hash }))
        }
        ("POST", ["records"]) => {
            let body = match req.json() {
                Ok(b) => b,
                Err(r) => return r,
            };
            let after = body.get("after").and_then(Value::as_u64).unwrap_or(u64::MAX);
            let lines: Vec<String> = body.get("records").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
            match store.append_records(&id, after, &lines) {
                Ok(seq) => Response::json(&json!({ "seq": seq })),
                Err(e) => Response::error(409, &e),
            }
        }
        _ => Response::error(404, "There is nothing at this address."),
    }
}

fn join(fleet: &Fleet, req: &Request) -> Response {
    let store = &fleet.store;
    let body = match req.json() {
        Ok(b) => b,
        Err(r) => return r,
    };
    let code_hash = sha256_hex(s(&body, "code").trim());
    let key = s(&body, "publicKey").to_string();
    use base64::Engine as _;
    if crate::store::b64().decode(&key).map(|k| k.len() != 32).unwrap_or(true) {
        return Response::error(400, "The computer did not say which key signs its log.");
    }
    let now = now_iso();
    let team = {
        let mut codes = store.codes.lock().unwrap();
        let Some(c) = codes.iter_mut().find(|c| c.hash == code_hash && c.uses_left > 0 && c.expires_at > now) else {
            return Response::error(403, "That code is not valid, has been used up, or has expired. Ask your administrator for a new one.");
        };
        c.uses_left -= 1;
        c.team.clone()
    };
    let _ = store.save_codes();
    let secret = random_token(32);
    let device = Device {
        id: random_token(9),
        name: short(s(&body, "deviceName"), 80),
        user: short(s(&body, "user"), 80),
        os: short(s(&body, "os"), 20),
        version: short(s(&body, "version"), 20),
        team,
        public_key: key,
        secret_hash: sha256_hex(&secret),
        joined_at: now.clone(),
        last_seen: Some(now),
        head_seq: 0,
        head_hash: homebody_core::ledger::ZERO_HASH.into(),
        base_seq: 0,
        base_hash: homebody_core::ledger::ZERO_HASH.into(),
        policy_version: 0,
    };
    if let Err(e) = store.save_device(&device) {
        return fail(e);
    }
    store.audit("(a computer)", "joined", json!({ "device": device.id, "name": device.name, "user": device.user, "team": device.team }));
    let reply = json!({ "deviceId": device.id, "secret": secret, "fleetKey": store.public_key(), "org": store.settings.lock().unwrap().name });
    store.devices.lock().unwrap().insert(device.id.clone(), device);
    Response::json(&reply)
}

// ---------------------------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------------------------

fn session_cookie(fleet: &Fleet, secret: &str, max_age: u64) -> String {
    format!("hb_session={secret}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}", if fleet.secure { "; Secure" } else { "" })
}

pub fn signed_in(fleet: &Fleet, secret: &str) -> Response {
    Response::json(&json!({ "ok": true })).with("Set-Cookie", session_cookie(fleet, secret, 12 * 3600))
}

fn console_api(fleet: &Fleet, req: &Request, parts: &[&str]) -> Response {
    let store = &fleet.store;
    let changing = req.method != "GET";
    if changing && req.header("x-homebody") != Some("console") {
        return Response::error(403, "This request did not come from the console.");
    }
    if let ("POST", ["login"]) = (req.method.as_str(), parts) {
        let body = match req.json() {
            Ok(b) => b,
            Err(r) => return r,
        };
        return match fleet.sessions.sign_in(store, s(&body, "name"), s(&body, "password")) {
            Ok(secret) => {
                store.audit(s(&body, "name"), "signed in", json!({}));
                signed_in(fleet, &secret)
            }
            Err(e) => Response::error(401, &e),
        };
    }
    if let ("GET", ["sso"]) = (req.method.as_str(), parts) {
        return Response::json(&json!({ "enabled": fleet.sso.is_some(), "name": fleet.sso.as_ref().map(|s| s.label.clone()) }));
    }
    let cookie = req.cookie("hb_session").unwrap_or_default();
    let Some(me) = fleet.sessions.person(store, &cookie) else {
        return Response::error(401, "Sign in first.");
    };
    let need = |role: &str| -> Result<(), Response> {
        if allows(&me.role, role) {
            Ok(())
        } else {
            Err(Response::error(403, &format!("Your role ({}) does not allow this.", me.role)))
        }
    };
    macro_rules! need {
        ($role:expr) => {
            if let Err(r) = need($role) {
                return r;
            }
        };
    }
    macro_rules! body {
        () => {
            match req.json() {
                Ok(b) => b,
                Err(r) => return r,
            }
        };
    }
    match (req.method.as_str(), parts) {
        ("POST", ["logout"]) => {
            fleet.sessions.sign_out(&cookie);
            Response::json(&json!({ "ok": true })).with("Set-Cookie", session_cookie(fleet, "", 0))
        }
        ("GET", ["me"]) => Response::json(&json!({ "name": me.name, "role": me.role, "org": store.settings.lock().unwrap().name, "fleetKey": store.public_key(), "url": fleet.public_url })),

        // Computers: every role.
        ("GET", ["devices"]) => {
            need!("viewer");
            let devices: Vec<Device> = store.devices.lock().unwrap().values().cloned().collect();
            let version = store.policy.lock().unwrap().version;
            let mut out: Vec<Value> = devices
                .iter()
                .map(|d| {
                    let check = store.verify_device(d);
                    json!({ "id": d.id, "name": d.name, "user": d.user, "os": d.os, "version": d.version, "team": d.team, "joinedAt": d.joined_at, "lastSeen": d.last_seen,
                        "records": d.head_seq, "kept": check.as_ref().ok(), "intact": check.is_ok(), "problem": check.err(), "rulesCurrent": d.policy_version == version, "key": d.public_key })
                })
                .collect();
            out.sort_by(|a, b| s(a, "name").to_lowercase().cmp(&s(b, "name").to_lowercase()));
            Response::json(&json!(out))
        }
        ("DELETE", ["devices", id]) => {
            need!("admin");
            let name = store.devices.lock().unwrap().get(*id).map(|d| d.name.clone());
            match name {
                Some(n) => {
                    if let Err(e) = store.remove_device(id) {
                        return fail(e);
                    }
                    store.audit(&me.name, "removed a computer", json!({ "device": id, "name": n }));
                    Response::json(&json!({ "ok": true }))
                }
                None => Response::error(404, "Unknown computer."),
            }
        }

        // What the computers' apps did: auditors and administrators.
        ("GET", ["activity"]) => {
            need!("auditor");
            Response::json(&json!(activity(store, req.query.get("device").map(String::as_str), req.query.get("q").map(String::as_str), req.query.get("show").map(String::as_str), None, 500)))
        }
        ("GET", ["policy"]) => {
            need!("auditor");
            Response::json(&serde_json::to_value(&*store.policy.lock().unwrap()).unwrap_or(Value::Null))
        }
        ("PUT", ["policy"]) => {
            need!("admin");
            let b = body!();
            let rules: Vec<OrgRule> = match serde_json::from_value(b.get("rules").cloned().unwrap_or(json!([]))) {
                Ok(r) => r,
                Err(_) => return Response::error(400, "Each rule needs an app, a place, and whether it is allowed."),
            };
            let (old_version, name) = (store.policy.lock().unwrap().version, store.settings.lock().unwrap().name.clone());
            let next = OrgPolicy { format: POLICY_FORMAT.into(), version: old_version + 1, updated_at: now_iso(), name, ask: b.get("ask").and_then(Value::as_bool), lock_mode: b.get("lockMode").and_then(Value::as_bool).unwrap_or(false), rules };
            if let Err(e) = next.check() {
                return Response::error(400, &e);
            }
            *store.policy.lock().unwrap() = next.clone();
            if let Err(e) = store.save_policy() {
                return fail(e);
            }
            store.audit(&me.name, "changed the rules", json!({ "version": next.version, "rules": next.rules.len(), "ask": next.ask, "lockMode": next.lock_mode }));
            Response::json(&serde_json::to_value(&next).unwrap_or(Value::Null))
        }
        ("GET", ["export"]) => {
            need!("auditor");
            let (from, to, team) = (req.query.get("from").cloned().unwrap_or_default(), req.query.get("to").cloned().unwrap_or_default(), req.query.get("team").cloned().unwrap_or_default());
            match export(store, &from, &to, &team) {
                Ok(bytes) => {
                    store.audit(&me.name, "exported evidence", json!({ "from": from, "to": to, "team": team, "bytes": bytes.len() }));
                    Response::bytes("application/x-tar", bytes).with("Content-Disposition", format!("attachment; filename=\"homebody-evidence-{}.tar\"", chrono::Utc::now().format("%Y-%m-%d")))
                }
                Err(e) => Response::error(400, &e),
            }
        }

        // Administration.
        ("GET", ["codes"]) => {
            need!("admin");
            let now = now_iso();
            let codes: Vec<Value> = store.codes.lock().unwrap().iter().map(|c| json!({ "id": c.id, "team": c.team, "createdAt": c.created_at, "expiresAt": c.expires_at, "usesLeft": c.uses_left, "live": c.uses_left > 0 && c.expires_at > now })).collect();
            Response::json(&json!(codes))
        }
        ("POST", ["codes"]) => {
            need!("admin");
            let b = body!();
            let team = short(s(&b, "team"), 40);
            if !team.chars().all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c)) {
                return Response::error(400, "A team's name can use letters, digits, spaces and . _ -");
            }
            let uses = b.get("uses").and_then(Value::as_u64).unwrap_or(1).clamp(1, 10_000);
            let days = b.get("days").and_then(Value::as_u64).unwrap_or(7).clamp(1, 365);
            let code = random_token(18);
            let entry = JoinCode { id: random_token(6), hash: sha256_hex(&code), team: team.clone(), created_at: now_iso(), expires_at: (chrono::Utc::now() + chrono::Duration::days(days as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true), uses_left: uses };
            let id = entry.id.clone();
            store.codes.lock().unwrap().push(entry);
            if let Err(e) = store.save_codes() {
                return fail(e);
            }
            store.audit(&me.name, "made a joining code", json!({ "id": id, "team": team, "uses": uses, "days": days }));
            Response::json(&json!({ "id": id, "code": code, "url": fleet.public_url }))
        }
        ("DELETE", ["codes", id]) => {
            need!("admin");
            store.codes.lock().unwrap().retain(|c| c.id != *id);
            let _ = store.save_codes();
            store.audit(&me.name, "withdrew a joining code", json!({ "id": id }));
            Response::json(&json!({ "ok": true }))
        }
        ("GET", ["mdm-profile"]) => {
            need!("admin");
            let code = req.query.get("code").cloned().unwrap_or_default();
            if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Response::error(400, "Make a joining code first and pass it here.");
            }
            store.audit(&me.name, "made a management profile", json!({}));
            let org = store.settings.lock().unwrap().name.clone();
            Response::bytes("application/x-apple-aspen-config", mdm_profile(&org, &fleet.public_url, &code).into_bytes()).with("Content-Disposition", "attachment; filename=\"homebody-fleet.mobileconfig\"".into())
        }
        ("GET", ["people"]) => {
            need!("admin");
            let people: Vec<Value> = store.people.lock().unwrap().iter().map(|p| json!({ "name": p.name, "role": p.role, "active": p.active })).collect();
            Response::json(&json!(people))
        }
        ("POST", ["people"]) => {
            need!("admin");
            let b = body!();
            let person = match auth::new_person(s(&b, "name"), s(&b, "role"), s(&b, "password")) {
                Ok(p) => p,
                Err(e) => return Response::error(400, &e),
            };
            {
                let mut people = store.people.lock().unwrap();
                if people.iter().any(|p| p.name.eq_ignore_ascii_case(&person.name)) {
                    return Response::error(409, "There is already someone with that name.");
                }
                people.push(person.clone());
            }
            let _ = store.save_people();
            store.audit(&me.name, "added a person", json!({ "name": person.name, "role": person.role }));
            Response::json(&json!({ "ok": true }))
        }
        ("PUT", ["people", name]) => {
            need!("admin");
            let b = body!();
            let mut changed = json!({ "name": name });
            {
                let mut people = store.people.lock().unwrap();
                let admins = people.iter().filter(|p| p.role == "admin" && p.active).count();
                let Some(p) = people.iter_mut().find(|p| p.name == *name) else { return Response::error(404, "Unknown person.") };
                let (role, active) = (b.get("role").and_then(Value::as_str).unwrap_or(&p.role).to_string(), b.get("active").and_then(Value::as_bool).unwrap_or(p.active));
                if !auth::valid_role(&role) {
                    return Response::error(400, "The role is one of admin, auditor, viewer.");
                }
                // Never leave the fleet with nobody who can administer it.
                if p.role == "admin" && p.active && (role != "admin" || !active) && admins <= 1 {
                    return Response::error(409, "This is the only administrator. Make someone else an administrator first.");
                }
                if let Some(pw) = b.get("password").and_then(Value::as_str) {
                    match auth::new_person(&p.name, &role, pw) {
                        Ok(fresh) => {
                            p.salt = fresh.salt;
                            p.hash = fresh.hash;
                            changed["password"] = json!(true);
                        }
                        Err(e) => return Response::error(400, &e),
                    }
                }
                p.role = role.clone();
                p.active = active;
                changed["role"] = json!(role);
                changed["active"] = json!(active);
            }
            let _ = store.save_people();
            fleet.sessions.end_all(name);
            store.audit(&me.name, "changed a person", changed);
            Response::json(&json!({ "ok": true }))
        }
        ("GET", ["settings"]) => {
            need!("admin");
            let st = store.settings.lock().unwrap().clone();
            Response::json(&json!({ "name": st.name, "retentionDays": st.retention_days, "sso": fleet.sso.is_some(), "scim": fleet.scim_token_hash.is_some() }))
        }
        ("PUT", ["settings"]) => {
            need!("admin");
            let b = body!();
            let name = short(s(&b, "name").trim(), 80);
            if name.is_empty() {
                return Response::error(400, "Give the organisation a name.");
            }
            let days = b.get("retentionDays").and_then(Value::as_u64).unwrap_or(0).min(3650);
            {
                let mut st = store.settings.lock().unwrap();
                st.name = name.clone();
                st.retention_days = days;
            }
            // The name is part of the rules computers show, so it is a new version of them.
            {
                let mut p = store.policy.lock().unwrap();
                if p.name != name {
                    p.name = name.clone();
                    p.version += 1;
                    p.updated_at = now_iso();
                }
            }
            let _ = store.save_settings();
            let _ = store.save_policy();
            store.audit(&me.name, "changed the settings", json!({ "name": name, "retentionDays": days }));
            Response::json(&json!({ "ok": true }))
        }
        ("GET", ["audit"]) => {
            need!("auditor");
            Response::bytes("application/x-ndjson", store.audit_export()).with("Content-Disposition", "attachment; filename=\"homebody-fleet-console-log.jsonl\"".into())
        }
        _ => Response::error(404, "There is nothing at this address."),
    }
}

/// Rows for the console's Activity view (and the spreadsheet in an export), newest first.
pub fn activity(store: &Store, device: Option<&str>, q: Option<&str>, show: Option<&str>, period: Option<(&str, &str)>, limit: usize) -> Vec<Value> {
    let devices: Vec<Device> = store.devices.lock().unwrap().values().filter(|d| device.map(|id| id.is_empty() || id == d.id).unwrap_or(true)).cloned().collect();
    let q = q.unwrap_or("").trim().to_lowercase();
    let mut rows: Vec<Value> = Vec::new();
    for d in &devices {
        let lines = store.log_lines(&d.id);
        let start = lines.len().saturating_sub(20_000);
        for line in &lines[start..] {
            let Ok(r) = serde_json::from_str::<Value>(line) else { continue };
            let (kind, ts) = (s(&r, "kind"), s(&r, "ts"));
            if let Some((from, to)) = period {
                if (!from.is_empty() && ts < from) || (!to.is_empty() && ts > to) {
                    continue;
                }
            }
            let (what, outcome) = match kind {
                "conn" if s(&r, "detail") == "self" => continue,
                "conn" if s(&r, "decision") == "allow" => ("connection", "allowed"),
                "conn" => ("connection", "blocked"),
                "ask" => ("question", "asked"),
                "trust" | "mode" | "lockdown" | "fleet" | "app" | "fence" => (kind, "change"),
                _ => continue,
            };
            let wanted = match show.unwrap_or("") {
                "blocked" => outcome == "blocked",
                "allowed" => outcome == "allowed",
                "changes" => outcome == "change",
                _ => true,
            };
            let text = format!("{} {} {}", s(&r, "app"), s(&r, "host"), d.name).to_lowercase();
            if !wanted || (!q.is_empty() && !text.contains(&q)) {
                continue;
            }
            let detail = match kind {
                "trust" => format!("{} {}", if r.get("removed").is_some() { "rule removed:" } else if r["enabled"].as_bool() == Some(true) { "trusted:" } else { "blocked:" }, if s(&r, "host").is_empty() { s(&r, "site") } else { s(&r, "host") }),
                "mode" => format!("mode: {}", if s(&r, "to").is_empty() { "asking changed" } else { s(&r, "to") }),
                "lockdown" => "everything blocked switched".to_string(),
                "fleet" => format!("fleet: {}", s(&r, "event")),
                "app" => format!("app {}", s(&r, "action")),
                "fence" => format!("protection: {}", s(&r, "event")),
                _ => s(&r, "reason").replace('_', " "),
            };
            rows.push(json!({ "device": d.id, "deviceName": d.name, "user": d.user, "team": d.team, "ts": ts, "seq": r["seq"], "what": what, "outcome": outcome, "app": s(&r, "app"), "host": s(&r, "host"), "port": r["port"], "detail": detail }));
        }
    }
    rows.sort_by(|a, b| s(b, "ts").cmp(s(a, "ts")));
    rows.truncate(limit);
    rows
}

fn csv_field(v: &str) -> String {
    let v = if v.starts_with(['=', '+', '-', '@']) { format!("'{v}") } else { v.to_string() };
    format!("\"{}\"", v.replace('"', "\"\""))
}

/// An evidence bundle: every computer's log exactly as it was sent (checked again just now),
/// the rules with the fleet's signature, a table of the period, and the console's own log.
fn export(store: &Store, from: &str, to: &str, team: &str) -> Result<Vec<u8>, String> {
    let day = |d: &str| d.is_empty() || (d.len() == 10 && d.chars().all(|c| c.is_ascii_digit() || c == '-'));
    if !day(from) || !day(to) {
        return Err("Dates look like 2026-11-30.".into());
    }
    let to_end = if to.is_empty() { String::new() } else { format!("{to}T23:59:59.999Z") };
    let now = chrono::Utc::now().timestamp() as u64;
    let devices: Vec<Device> = store.devices.lock().unwrap().values().filter(|d| team.is_empty() || d.team == team).cloned().collect();
    let mut t = Vec::new();
    let mut summary = Vec::new();
    for d in &devices {
        let check = store.verify_device(d);
        let folder: String = format!("{}-{}", d.name, d.id).chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).take(60).collect();
        let lines = store.log_lines(&d.id);
        let mut log = lines.join("\n");
        log.push('\n');
        tar::add(&mut t, &format!("evidence/computers/{folder}/log.jsonl"), log.as_bytes(), now)?;
        let about = json!({ "name": d.name, "user": d.user, "team": d.team, "os": d.os, "publicKey": d.public_key, "joinedAt": d.joined_at, "lastSeen": d.last_seen,
            "afterSeq": d.base_seq, "afterHash": d.base_hash, "headSeq": d.head_seq, "headHash": d.head_hash, "checkedAt": now_iso(), "intact": check.is_ok(), "problem": check.as_ref().err() });
        tar::add(&mut t, &format!("evidence/computers/{folder}/computer.json"), &serde_json::to_vec_pretty(&about).unwrap_or_default(), now)?;
        summary.push(json!({ "computer": d.name, "user": d.user, "team": d.team, "folder": folder, "records": lines.len(), "intact": check.is_ok() }));
    }
    let mut csv = String::from("time,computer,user,team,app,place,port,outcome,detail\n");
    for r in activity(store, None, None, None, Some((from, &to_end)), usize::MAX).iter().rev() {
        csv += &[s(r, "ts"), s(r, "deviceName"), s(r, "user"), s(r, "team"), s(r, "app"), s(r, "host"), &r["port"].as_u64().map(|p| p.to_string()).unwrap_or_default(), s(r, "outcome"), s(r, "detail")].iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",");
        csv.push('\n');
    }
    tar::add(&mut t, "evidence/activity.csv", csv.as_bytes(), now)?;
    tar::add(&mut t, "evidence/rules.json", &serde_json::to_vec_pretty(&store.signed_policy()).unwrap_or_default(), now)?;
    tar::add(&mut t, "evidence/compliance.json", &serde_json::to_vec_pretty(&homebody_core::compliance::json_table()).unwrap_or_default(), now)?;
    tar::add(&mut t, "evidence/console-log.jsonl", &store.audit_export(), now)?;
    let about = json!({ "organisation": store.settings.lock().unwrap().name, "fleetKey": store.public_key(), "madeAt": now_iso(), "from": from, "to": to, "team": team, "computers": summary });
    tar::add(&mut t, "evidence/summary.json", &serde_json::to_vec_pretty(&about).unwrap_or_default(), now)?;
    let readme = "Homebody fleet evidence\n=======================\n\ncomputers/<name>/log.jsonl      One computer's log, exactly as it sent it: each record is signed with that\n                                computer's key and chained to the one before.\ncomputers/<name>/computer.json  Whose log it is, the key that signed it, and where the kept part starts.\n                                Check a log on any computer with:  homebody verify computers/<name>\nactivity.csv                    The chosen period as a table, for a spreadsheet. Convenient; not signed.\nrules.json                      The organisation's rules with the fleet's signature over them.\ncompliance.json                 Which controls in NIST SP 800-53 and OWASP's list for AI agents this\n                                evidence speaks to. The project's own mapping, not an auditor's.\nconsole-log.jsonl               What people did in the console (who changed the rules, who exported),\n                                as its own signed log. Check it with:  homebody verify console-log.jsonl\nsummary.json                    What is in this bundle.\n";
    tar::add(&mut t, "evidence/README.txt", readme.as_bytes(), now)?;
    tar::finish(&mut t);
    Ok(t)
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn uuid() -> String {
    let h = hex::encode(rand::random::<[u8; 16]>()).to_uppercase();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// A configuration profile for Jamf, Kandji or any other Mac management system. It gives
/// Homebody on a managed Mac the fleet's address and a code, and Homebody joins by itself.
pub fn mdm_profile(org: &str, url: &str, code: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>PayloadContent</key>
	<array>
		<dict>
			<key>PayloadType</key><string>app.homebody.mac</string>
			<key>PayloadIdentifier</key><string>app.homebody.mac.fleet</string>
			<key>PayloadUUID</key><string>{}</string>
			<key>PayloadVersion</key><integer>1</integer>
			<key>PayloadDisplayName</key><string>Homebody fleet</string>
			<key>FleetURL</key><string>{}</string>
			<key>JoinCode</key><string>{}</string>
		</dict>
	</array>
	<key>PayloadDisplayName</key><string>Homebody: join {}</string>
	<key>PayloadDescription</key><string>Tells Homebody on this Mac which fleet to join. The fleet's rules then apply, and Homebody's log is sent to the fleet's server.</string>
	<key>PayloadIdentifier</key><string>app.homebody.fleet-join</string>
	<key>PayloadOrganization</key><string>{}</string>
	<key>PayloadScope</key><string>System</string>
	<key>PayloadType</key><string>Configuration</string>
	<key>PayloadUUID</key><string>{}</string>
	<key>PayloadVersion</key><integer>1</integer>
</dict>
</plist>
"#,
        uuid(),
        xml(url),
        xml(code),
        xml(org),
        xml(org),
        uuid()
    )
}
