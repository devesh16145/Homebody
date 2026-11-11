//! Proves the Cedar export says what Homebody does.
//!
//! 1. The exported policies parse, and validate against the exported schema, with Cedar's own
//!    library.
//! 2. For many apps, places and requests, Cedar's answer is compared with Homebody's own rule
//!    engine. Any difference is a bug in the translation and fails the run.
//!
//!   cargo run --manifest-path tools/cedar-check/Cargo.toml            the built-in rules + stress cases
//!   cargo run --manifest-path tools/cedar-check/Cargo.toml -- <settings.json>   also your own rules

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request, Schema, ValidationMode, Validator};
use homebody_core::config::{Config, InsideRule, TrustRule};
use std::str::FromStr;

fn rule(host: &str, enabled: bool) -> TrustRule {
    TrustRule { host: host.into(), enabled, source: "you".into(), recommended: false, limit_mb: None }
}

fn inside(host: &str, path: &str, enabled: bool, method: Option<&str>) -> InsideRule {
    InsideRule { host: host.into(), path: path.into(), enabled, source: "you".into(), method: method.map(str::to_string) }
}

/// Rules chosen to hit every corner of the precedence order.
fn stress() -> Config {
    let mut cfg = Config::fresh();
    let app = cfg.app_mut("claude-code").expect("built-in app");
    app.trust = vec![
        rule("api.example.com", true),
        rule("*.example.com", false),
        rule("deep.api.example.com", true),
        rule("both.example.com", true),
        rule("both.example.com", false),
        rule("*.open.org", true),
        rule("closed.open.org", false),
        rule("*.inner.open.org", false),
        rule("ok.inner.open.org", true),
        rule("plain.net", true),
        rule("*.wild-only.io", true),
    ];
    app.inside = vec![
        inside("plain.net", "/api", false, None),
        inside("plain.net", "/api/public", true, None),
        inside("plain.net", "/api/public/secret", false, None),
        inside("plain.net", "/upload", false, Some("POST")),
        inside("plain.net", "/upload", true, Some("GET")),
        inside("plain.net", "/dir/", false, None),
        inside("plain.net", "/Mixed/Case", false, None),
        inside("api.example.com", "/v1/files", false, None),
        inside("api.example.com", "/v1/files", true, Some("GET")),
    ];
    cfg
}

const HOSTS: &[&str] = &[
    "api.example.com", "example.com", "x.example.com", "deep.api.example.com", "a.deep.api.example.com", "both.example.com", "evilexample.com",
    "open.org", "a.open.org", "closed.open.org", "x.closed.open.org", "inner.open.org", "ok.inner.open.org", "no.inner.open.org",
    "plain.net", "a.plain.net", "wild-only.io", "a.wild-only.io", "a.b.wild-only.io", "unknown.dev",
    "api.anthropic.com", "statsig.anthropic.com", "claude.ai", "registry.ollama.ai", "a.sentry.io", "sentry.io", "chatgpt.com", "api.openai.com",
];
const PATHS: &[&str] = &[
    "/", "/api", "/api/", "/api/x", "/apix", "/api/public", "/api/public/y", "/api/public/secret", "/api/public/secret/z", "/api/publicity",
    "/upload", "/upload/a", "/uploads", "/dir", "/dir/", "/dir/a", "/mixed/case", "/mixed/case/a", "/v1/files", "/v1/files/1", "/v1/messages", "/api/event_logging", "/api/event_logging/v2/batch",
];
const METHODS: &[&str] = &["GET", "POST", "PUT"];

fn check(name: &str, cfg: &Config) -> Result<(usize, usize), String> {
    let export = homebody_core::policy::cedar(cfg);
    let policies = PolicySet::from_str(&export.policies).map_err(|e| format!("{name}: the policies do not parse: {e}"))?;
    let (schema, warnings) = Schema::from_cedarschema_str(&export.schema).map_err(|e| format!("{name}: the schema does not parse: {e}"))?;
    for w in warnings {
        eprintln!("{name}: schema warning: {w}");
    }
    let result = Validator::new(schema.clone()).validate(&policies, ValidationMode::Strict);
    if !result.validation_passed() {
        return Err(format!("{name}: the policies do not fit the schema:\n{}", result.validation_errors().map(|e| format!("  {e}")).collect::<Vec<_>>().join("\n")));
    }
    let apps: serde_json::Value = serde_json::from_str(&export.entities).map_err(|e| e.to_string())?;
    let authorizer = Authorizer::new();
    let (mut asked, mut wrong) = (0, Vec::new());
    for app in cfg.apps.iter().filter(|a| a.protected) {
        let principal = EntityUid::from_str(&format!("Homebody::App::\"{}\"", app.id)).map_err(|e| e.to_string())?;
        for host in HOSTS {
            let mut list = apps.as_array().cloned().unwrap_or_default();
            list.push(serde_json::json!({ "uid": { "type": "Homebody::Place", "id": host }, "attrs": { "name": host }, "parents": [] }));
            let entities = Entities::from_json_value(serde_json::Value::Array(list), Some(&schema)).map_err(|e| format!("{name}: entities: {e}"))?;
            let resource = EntityUid::from_str(&format!("Homebody::Place::\"{host}\"")).map_err(|e| e.to_string())?;
            // Reaching the place.
            let action = EntityUid::from_str("Homebody::Action::\"connect\"").map_err(|e| e.to_string())?;
            let context = Context::from_json_value(serde_json::json!({ "port": 443 }), Some((&schema, &action))).map_err(|e| e.to_string())?;
            let request = Request::new(principal.clone(), action, resource.clone(), context, Some(&schema)).map_err(|e| e.to_string())?;
            let cedar = authorizer.is_authorized(&request, &policies, &entities).decision() == Decision::Allow;
            let ours = app.rule_for(host).map(|r| r.enabled).unwrap_or(false);
            asked += 1;
            if cedar != ours {
                wrong.push(format!("  {} -> {host}: Homebody says {}, Cedar says {}", app.id, if ours { "allow" } else { "refuse" }, if cedar { "allow" } else { "refuse" }));
            }
            // Addresses inside it.
            if app.inside.iter().any(|r| r.host == *host) {
                let action = EntityUid::from_str("Homebody::Action::\"request\"").map_err(|e| e.to_string())?;
                for path in PATHS {
                    for method in METHODS {
                        let context = Context::from_json_value(serde_json::json!({ "method": method, "path": path }), Some((&schema, &action))).map_err(|e| e.to_string())?;
                        let request = Request::new(principal.clone(), action.clone(), resource.clone(), context, Some(&schema)).map_err(|e| e.to_string())?;
                        let cedar = authorizer.is_authorized(&request, &policies, &entities).decision() == Decision::Allow;
                        let ours = app.inside_allows(host, method, path);
                        asked += 1;
                        if cedar != ours {
                            wrong.push(format!("  {} -> {method} {host}{path}: Homebody says {}, Cedar says {}", app.id, if ours { "allow" } else { "refuse" }, if cedar { "allow" } else { "refuse" }));
                        }
                    }
                }
            }
        }
    }
    if !wrong.is_empty() {
        return Err(format!("{name}: {} of {asked} answers differ:\n{}", wrong.len(), wrong.join("\n")));
    }
    Ok((policies.policies().count(), asked))
}

fn main() {
    let mut sets = vec![("built-in rules".to_string(), Config::fresh()), ("stress rules".to_string(), stress())];
    if let Some(path) = std::env::args().nth(1) {
        sets.push((path.clone(), Config::load(std::path::Path::new(&path))));
    }
    let mut failed = false;
    for (name, cfg) in &sets {
        match check(name, cfg) {
            Ok((policies, asked)) => println!("ok  {name}: {policies} policies parse and fit the schema; {asked} questions, Cedar and Homebody agree on every one"),
            Err(e) => {
                failed = true;
                eprintln!("FAILED  {e}");
            }
        }
    }
    std::process::exit(if failed { 1 } else { 0 });
}
