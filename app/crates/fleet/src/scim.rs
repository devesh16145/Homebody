//! Provisioning people from the identity provider (SCIM 2.0, the Users part).
//!
//! An identity provider that is told this server's address and a secret can add people, change
//! their role, and switch them off, so nobody has to be removed here by hand when they leave.
//! People made this way have no password here: they sign in through the provider.
//!
//! A person's role is read from `roles[0].value` ("admin", "auditor" or "viewer"); without one
//! they are a viewer. The last administrator is never switched off or demoted.

use crate::api::Fleet;
use crate::auth::valid_role;
use crate::http::{Request, Response};
use crate::store::{sha256_hex, Person};
use serde_json::{json, Value};

const USER: &str = "urn:ietf:params:scim:schemas:core:2.0:User";

fn scim_error(status: u16, detail: &str) -> Response {
    let mut r = Response::json(&json!({ "schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"], "status": status.to_string(), "detail": detail }));
    r.status = status;
    r.content_type = "application/scim+json";
    r
}

fn user(p: &Person) -> Value {
    json!({ "schemas": [USER], "id": p.name, "userName": p.name, "active": p.active, "roles": [{ "value": p.role, "primary": true }], "meta": { "resourceType": "User" } })
}

fn ok(v: Value, status: u16) -> Response {
    let mut r = Response::json(&v);
    r.status = status;
    r.content_type = "application/scim+json";
    r
}

fn role_in(v: &Value) -> Option<String> {
    v.get("roles").and_then(Value::as_array).and_then(|r| r.first()).and_then(|r| r.get("value").or(Some(r))).and_then(Value::as_str).map(str::to_lowercase)
}

/// Apply a wanted role and active state, refusing to leave the fleet without an administrator.
fn apply(people: &mut [Person], name: &str, role: Option<String>, active: Option<bool>) -> Result<Person, Response> {
    let admins = people.iter().filter(|p| p.role == "admin" && p.active).count();
    let p = people.iter_mut().find(|p| p.name == name).ok_or_else(|| scim_error(404, "No such user."))?;
    let (role, active) = (role.unwrap_or_else(|| p.role.clone()), active.unwrap_or(p.active));
    if !valid_role(&role) {
        return Err(scim_error(400, "The role is one of admin, auditor, viewer."));
    }
    if p.role == "admin" && p.active && (role != "admin" || !active) && admins <= 1 {
        return Err(scim_error(409, "This is the only administrator."));
    }
    p.role = role;
    p.active = active;
    Ok(p.clone())
}

pub fn handle(fleet: &Fleet, req: &Request, parts: &[&str]) -> Response {
    let Some(expected) = &fleet.scim_token_hash else { return scim_error(404, "Provisioning is not set up on this server.") };
    let given = req.header("authorization").and_then(|h| h.strip_prefix("Bearer ")).map(sha256_hex).unwrap_or_default();
    if !crate::store::same(&given, expected) {
        return scim_error(401, "The provisioning secret is missing or wrong.");
    }
    let store = &fleet.store;
    let body: Value = if req.body.is_empty() { Value::Null } else { serde_json::from_slice(&req.body).unwrap_or(Value::Null) };
    let done = |what: &str, p: &Person| {
        let _ = store.save_people();
        fleet.sessions.end_all(&p.name);
        store.audit("(identity provider)", what, json!({ "name": p.name, "role": p.role, "active": p.active }));
    };
    match (req.method.as_str(), parts) {
        ("GET", ["ServiceProviderConfig"]) => ok(json!({ "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"], "patch": { "supported": true }, "bulk": { "supported": false }, "filter": { "supported": true, "maxResults": 200 }, "changePassword": { "supported": false }, "sort": { "supported": false }, "etag": { "supported": false }, "authenticationSchemes": [{ "type": "oauthbearertoken", "name": "Bearer token", "description": "The provisioning secret" }] }), 200),
        ("GET", ["Users"]) => {
            // The one filter identity providers use: userName eq "someone".
            let wanted = req.query.get("filter").and_then(|f| f.split_once(" eq ")).filter(|(attr, _)| attr.trim().eq_ignore_ascii_case("userName")).map(|(_, v)| v.trim().trim_matches('"').to_lowercase());
            let list: Vec<Value> = store.people.lock().unwrap().iter().filter(|p| wanted.as_ref().map(|w| p.name.to_lowercase() == *w).unwrap_or(true)).map(user).collect();
            ok(json!({ "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"], "totalResults": list.len(), "startIndex": 1, "itemsPerPage": list.len(), "Resources": list }), 200)
        }
        ("POST", ["Users"]) => {
            let name: String = body.get("userName").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c)) {
                return scim_error(400, "userName can use letters, digits and . _ - @");
            }
            let role = role_in(&body).unwrap_or_else(|| "viewer".into());
            if !valid_role(&role) {
                return scim_error(400, "The role is one of admin, auditor, viewer.");
            }
            let person = Person { name: name.clone(), role, salt: String::new(), hash: String::new(), active: body.get("active").and_then(Value::as_bool).unwrap_or(true) };
            {
                let mut people = store.people.lock().unwrap();
                if people.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                    return scim_error(409, "There is already a user with that name.");
                }
                people.push(person.clone());
            }
            done("added a person", &person);
            ok(user(&person), 201)
        }
        ("GET", ["Users", id]) => match store.people.lock().unwrap().iter().find(|p| p.name == *id) {
            Some(p) => ok(user(p), 200),
            None => scim_error(404, "No such user."),
        },
        ("PUT", ["Users", id]) => {
            let result = apply(&mut store.people.lock().unwrap(), id, role_in(&body), body.get("active").and_then(Value::as_bool));
            match result {
                Ok(p) => {
                    done("changed a person", &p);
                    ok(user(&p), 200)
                }
                Err(r) => r,
            }
        }
        ("PATCH", ["Users", id]) => {
            let (mut role, mut active) = (None, None);
            for op in body.get("Operations").and_then(Value::as_array).into_iter().flatten() {
                if !op.get("op").and_then(Value::as_str).map(|o| o.eq_ignore_ascii_case("replace") || o.eq_ignore_ascii_case("add")).unwrap_or(false) {
                    continue;
                }
                let value = op.get("value").cloned().unwrap_or(Value::Null);
                let as_bool = |v: &Value| v.as_bool().or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true")));
                match op.get("path").and_then(Value::as_str) {
                    Some("active") => active = as_bool(&value),
                    Some(p) if p.starts_with("roles") => role = value.as_str().map(str::to_lowercase).or_else(|| role_in(&json!({ "roles": value }))),
                    _ => {
                        active = value.get("active").and_then(as_bool).or(active);
                        role = role_in(&value).or(role);
                    }
                }
            }
            let result = apply(&mut store.people.lock().unwrap(), id, role, active);
            match result {
                Ok(p) => {
                    done("changed a person", &p);
                    ok(user(&p), 200)
                }
                Err(r) => r,
            }
        }
        ("DELETE", ["Users", id]) => {
            let result = apply(&mut store.people.lock().unwrap(), id, None, Some(false));
            match result {
                Ok(p) => {
                    done("switched a person off", &p);
                    Response { status: 204, content_type: "application/scim+json", body: vec![], headers: vec![] }
                }
                Err(r) => r,
            }
        }
        _ => scim_error(404, "There is nothing at this address."),
    }
}
