//! The rules in a formal language, for people who review policy for a living.
//!
//! Homebody's rules are switches in an app. An organisation that has to sign off on them wants
//! something it can read line by line, diff, and check with its own tools. This writes the rules
//! in force as [Cedar](https://www.cedarpolicy.com) policies: a small open language with a public
//! specification, where everything is refused unless a `permit` allows it and a `forbid` always
//! wins over a `permit`.
//!
//! The export is a faithful translation, not a second source of truth: Homebody keeps deciding
//! with its own rules. `tools/cedar-check` proves the translation by asking Cedar's own engine
//! the same questions and comparing the answers.
//!
//! How the two line up:
//!
//! | Homebody                                   | Cedar                                              |
//! |--------------------------------------------|----------------------------------------------------|
//! | a protected app                            | `Homebody::App::"claude-code"`                     |
//! | a place (a name on the internet)           | `Homebody::Place::"api.anthropic.com"`, `.name`    |
//! | reaching a place                           | action `connect`                                   |
//! | an address inside a place (Look inside)    | action `request`, `context.method`, `context.path` |
//! | a trusted place                            | `permit`                                           |
//! | a blocked place                            | `forbid`                                           |
//! | a place with no rule                       | no policy: refused (Homebody may ask you first)    |
//!
//! An exact name beats a wildcard in Homebody. Cedar has no such order, so a wildcard `forbid`
//! carries an `unless` naming the exact places that are trusted beneath it. An organisation's
//! rules come before the person's own; so each of the person's own rules carries an `unless`
//! that steps aside wherever an organisation rule covers the place.

use crate::config::{AppConfig, Config, InsideRule, TrustRule};

pub struct CedarExport {
    pub policies: String,
    pub schema: String,
    pub entities: String,
}

pub const SCHEMA: &str = r#"// What Homebody's exported policies talk about.
namespace Homebody {
  // An app that runs protected. `kind` is "agent", "model", "window" or "browser".
  entity App = {
    name: String,
    kind: String,
    protected: Bool,
  };

  // A place on the internet. The entity's id and `name` are both the host name, lower case.
  entity Place = {
    name: String,
  };

  // Opening a connection to a place.
  action connect appliesTo {
    principal: [App],
    resource: [Place],
    context: { port: Long },
  };

  // One request inside a connection that is being looked inside. `path` is lower case and has
  // nothing after "?"; `method` is upper case.
  action request appliesTo {
    principal: [App],
    resource: [Place],
    context: { method: String, path: String },
  };
}
"#;

/// A string literal in Cedar.
fn lit(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A `like` pattern that matches `prefix` followed by anything: every `*` of our own is escaped.
fn like_prefix(prefix: &str, tail: &str) -> String {
    format!("\"{}{tail}\"", prefix.replace('\\', "\\\\").replace('"', "\\\"").replace('*', "\\*"))
}

/// A name for a policy, unique within the export, made of plain characters.
fn policy_id(parts: &[&str]) -> String {
    parts.iter().map(|p| p.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect::<String>()).collect::<Vec<_>>().join(":")
}

fn app_ref(app: &AppConfig) -> String {
    format!("Homebody::App::{}", lit(&app.id))
}

/// The condition "this place is covered by `pattern`".
fn place_cond(pattern: &str) -> Option<String> {
    pattern.strip_prefix("*.").map(|suffix| format!("resource.name like \"*.{}\"", suffix.replace('\\', "\\\\").replace('"', "\\\"").replace('*', "\\*")))
}

/// Rules for reaching places, in Homebody's own order of precedence.
///
/// There are two tiers. Rules set by an organisation (source "org") are asked first; where one
/// of them covers a place, the person's own rules are not asked at all. In Cedar that becomes:
/// the organisation's rules as they are, and every one of the person's own rules with an
/// `unless` that steps aside wherever an organisation rule covers the place.
fn connect_policies(app: &AppConfig, out: &mut String) {
    let org: Vec<&TrustRule> = app.trust.iter().filter(|r| r.source == "org").collect();
    let own: Vec<&TrustRule> = app.trust.iter().filter(|r| r.source != "org").collect();
    let cover: Vec<String> = org.iter().map(|r| place_cond(&r.host).unwrap_or_else(|| format!("resource.name == {}", lit(&r.host)))).collect();
    tier(app, &org, "org-", &[], out);
    tier(app, &own, "", &cover, out);
}

/// One tier's rules: an exact name beats a wildcard, and a block beats an allow of the same kind.
/// `aside` lists conditions under which this whole tier does not apply.
fn tier(app: &AppConfig, rules: &[&TrustRule], prefix: &str, aside: &[String], out: &mut String) {
    let exact: Vec<&&TrustRule> = rules.iter().filter(|r| !r.host.starts_with("*.")).collect();
    // For one exact name, a block beats an allow.
    let decided = |host: &str| -> Option<bool> {
        let mut same = exact.iter().filter(|r| r.host == host).peekable();
        same.peek()?;
        Some(same.all(|r| r.enabled))
    };
    let unless = |more: &[String]| -> String {
        let all: Vec<&String> = more.iter().chain(aside.iter()).collect();
        if all.is_empty() {
            String::new()
        } else {
            format!("\nunless {{ {} }}", all.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(" || "))
        }
    };
    let mut seen: Vec<&str> = Vec::new();
    for r in &exact {
        if seen.contains(&r.host.as_str()) {
            continue;
        }
        seen.push(&r.host);
        let allowed = decided(&r.host).unwrap_or(false);
        let limit = rules.iter().find(|x| x.host == r.host).and_then(|x| x.limit_mb);
        out.push_str(&format!("@id({})\n", lit(&policy_id(&[&app.id, &format!("{prefix}place"), &r.host]))));
        if let (true, Some(mb)) = (allowed, limit) {
            out.push_str(&format!("@limit_mb_per_hour({})\n", lit(&mb.to_string())));
        }
        out.push_str(&format!("{}(\n  principal == {},\n  action == Homebody::Action::\"connect\",\n  resource == Homebody::Place::{}\n){};\n\n", if allowed { "permit" } else { "forbid" }, app_ref(app), lit(&r.host), unless(&[])));
    }
    let mut seen: Vec<(&str, bool)> = Vec::new();
    for r in rules.iter().filter(|r| r.host.starts_with("*.")) {
        if seen.contains(&(r.host.as_str(), r.enabled)) {
            continue;
        }
        seen.push((&r.host, r.enabled));
        let Some(cond) = place_cond(&r.host) else { continue };
        out.push_str(&format!("@id({})\n", lit(&policy_id(&[&app.id, &format!("{prefix}{}", if r.enabled { "places" } else { "places-blocked" }), &r.host]))));
        if let (true, Some(mb)) = (r.enabled, r.limit_mb) {
            out.push_str(&format!("@limit_mb_per_hour({})\n", lit(&mb.to_string())));
        }
        out.push_str(&format!("{}(\n  principal == {},\n  action == Homebody::Action::\"connect\",\n  resource\n)\nwhen {{ {cond} }}", if r.enabled { "permit" } else { "forbid" }, app_ref(app)));
        // An exact name in the same tier is decided by its own rule, not by a wildcard: a
        // blocked wildcard spares the exact names trusted beneath it, and a trusted wildcard
        // needs no sparing (a `forbid` for the exact name already wins).
        let mut spared: Vec<String> = Vec::new();
        if !r.enabled {
            for e in exact.iter().filter(|e| crate::hostname::matches(&r.host, &e.host) && decided(&e.host) == Some(true)) {
                let c = format!("resource.name == {}", lit(&e.host));
                if !spared.contains(&c) {
                    spared.push(c);
                }
            }
        }
        out.push_str(&unless(&spared));
        out.push_str(";\n\n");
    }
}

/// The condition "this request is covered by `r`".
fn inside_cond(r: &InsideRule) -> String {
    let p = r.path.to_lowercase();
    let path = if p.ends_with('/') {
        format!("context.path like {}", like_prefix(&p, "*"))
    } else {
        format!("(context.path == {} || context.path like {})", lit(&p), like_prefix(&p, "/*"))
    };
    match &r.method {
        Some(m) => format!("{path} && context.method == {}", lit(&m.to_ascii_uppercase())),
        None => path,
    }
}

/// Switches for addresses inside places. Everything inside a place is allowed unless a switch
/// is off; where switches overlap, the longest path decides, then the one that names a method.
fn request_policies(app: &AppConfig, out: &mut String) {
    out.push_str(&format!("@id({})\npermit(\n  principal == {},\n  action == Homebody::Action::\"request\",\n  resource\n);\n\n", lit(&policy_id(&[&app.id, "requests"])), app_ref(app)));
    let key = |r: &InsideRule| (r.path.len(), r.method.is_some());
    for (i, r) in app.inside.iter().enumerate().filter(|(_, r)| !r.enabled) {
        // Switches that are on and would be chosen over this one for a request both cover.
        let over: Vec<String> = app
            .inside
            .iter()
            .enumerate()
            .filter(|(j, o)| o.enabled && o.host == r.host && (key(o) > key(r) || (key(o) == key(r) && *j > i)))
            .map(|(_, o)| format!("({})", inside_cond(o)))
            .collect();
        out.push_str(&format!("@id({})\n", lit(&policy_id(&[&app.id, "inside", &r.host, &r.path, r.method.as_deref().unwrap_or("any"), &i.to_string()]))));
        out.push_str(&format!("forbid(\n  principal == {},\n  action == Homebody::Action::\"request\",\n  resource == Homebody::Place::{}\n)\nwhen {{ {} }}", app_ref(app), lit(&r.host), inside_cond(r)));
        if !over.is_empty() {
            out.push_str(&format!("\nunless {{ {} }}", over.join(" || ")));
        }
        out.push_str(";\n\n");
    }
}

/// The rules in force, as Cedar policies with their schema and the apps as entities.
pub fn cedar(cfg: &Config) -> CedarExport {
    let mut p = String::new();
    p.push_str("// Homebody's rules, written as Cedar policies. Generated; edit the rules in Homebody instead.\n");
    p.push_str(&format!("// Made {} by Homebody {}.\n//\n", crate::now_iso(), crate::VERSION));
    p.push_str("// Read it like this: an app may reach a place only if a `permit` says so, and a `forbid`\n// always wins. A place with no policy is refused");
    p.push_str(match (cfg.mode, cfg.ask) {
        (crate::model::Mode::Private, true) => "; Homebody asks the person at the computer\n// first, and their answer becomes a rule.\n",
        (crate::model::Mode::Private, false) => " without asking.\n",
        (crate::model::Mode::Watch, _) => ".\n// NOTE: when this was made Homebody was set to Watch, which records without refusing.\n",
        (crate::model::Mode::Off, _) => ".\n// NOTE: when this was made protection was switched off, so nothing was being enforced.\n",
    });
    if cfg.lockdown {
        p.push_str("// NOTE: when this was made everything was blocked (the lock was on), whatever the rules say.\n");
    }
    p.push('\n');
    let mut entities = Vec::new();
    for app in &cfg.apps {
        entities.push(serde_json::json!({
            "uid": { "type": "Homebody::App", "id": app.id },
            "attrs": { "name": app.name, "kind": app.kind, "protected": app.protected },
            "parents": [],
        }));
        if !app.protected {
            p.push_str(&format!("// {} is not protected: Homebody does not stand between it and the internet.\n\n", app.name));
            continue;
        }
        p.push_str(&format!("// ---- {} ----\n\n", app.name.replace('\n', " ")));
        connect_policies(app, &mut p);
        request_policies(app, &mut p);
    }
    CedarExport { policies: p, schema: SCHEMA.to_string(), entities: serde_json::to_string_pretty(&entities).unwrap_or_else(|_| "[]".into()) }
}

pub const README: &str = "Homebody policy export\n======================\n\npolicy.cedar          The rules in force, as Cedar policies (https://www.cedarpolicy.com).\nhomebody.cedarschema  What the policies talk about: apps, places, and the two actions.\nentities.json         The protected apps, as Cedar entities.\n\nCheck the policies against the schema with Cedar's own tool:\n  cedar validate --schema homebody.cedarschema --policies policy.cedar\n\nAsk a question, for example \"may Claude Code reach example.com?\":\n  cedar authorize --schema homebody.cedarschema --policies policy.cedar --entities entities.json \\\n    --principal 'Homebody::App::\"claude-code\"' --action 'Homebody::Action::\"connect\"' \\\n    --resource 'Homebody::Place::\"example.com\"' --context context.json\n(with a Place entity for the name added to entities.json and {\"port\": 443} in context.json)\n\nThis is a translation for review. Homebody itself keeps deciding with its own rules; the\ntranslation is checked against them by tools/cedar-check in Homebody's source.\n";

/// Write the export into `dir` (made if missing).
pub fn write(cfg: &Config, dir: &std::path::Path) -> std::io::Result<()> {
    let e = cedar(cfg);
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("policy.cedar"), e.policies)?;
    std::fs::write(dir.join("homebody.cedarschema"), e.schema)?;
    std::fs::write(dir.join("entities.json"), e.entities)?;
    std::fs::write(dir.join("README.txt"), README)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str, enabled: bool) -> TrustRule {
        TrustRule { host: host.into(), enabled, source: "you".into(), recommended: false, limit_mb: None }
    }

    fn app_with(trust: Vec<TrustRule>, inside: Vec<InsideRule>) -> Config {
        let mut cfg = Config::fresh();
        cfg.apps.truncate(1);
        cfg.apps[0].id = "ollama".into();
        cfg.apps[0].trust = trust;
        cfg.apps[0].inside = inside;
        cfg
    }

    #[test]
    fn a_trusted_place_is_a_permit_and_a_blocked_one_a_forbid() {
        let cfg = app_with(vec![rule("api.example.com", true), rule("stats.example.com", false)], vec![]);
        let p = cedar(&cfg).policies;
        assert!(p.contains("permit(\n  principal == Homebody::App::\"ollama\",\n  action == Homebody::Action::\"connect\",\n  resource == Homebody::Place::\"api.example.com\"\n);"), "{p}");
        assert!(p.contains("forbid(\n  principal == Homebody::App::\"ollama\",\n  action == Homebody::Action::\"connect\",\n  resource == Homebody::Place::\"stats.example.com\"\n);"));
    }

    #[test]
    fn a_blocked_wildcard_spares_the_exact_places_trusted_beneath_it() {
        let cfg = app_with(vec![rule("*.example.com", false), rule("api.example.com", true), rule("x.example.com", true), rule("x.example.com", false), rule("other.org", true)], vec![]);
        let p = cedar(&cfg).policies;
        assert!(p.contains("when { resource.name like \"*.example.com\" }\nunless { resource.name == \"api.example.com\" };"), "{p}");
        // The same exact name both trusted and blocked is blocked, so it is not spared.
        assert!(p.contains("forbid(\n  principal == Homebody::App::\"ollama\",\n  action == Homebody::Action::\"connect\",\n  resource == Homebody::Place::\"x.example.com\"\n);"));
        assert!(!p.contains("resource.name == \"x.example.com\""));
    }

    #[test]
    fn a_persons_own_rules_step_aside_where_the_organisation_has_one() {
        let org = |host: &str, enabled| TrustRule { host: host.into(), enabled, source: "org".into(), recommended: false, limit_mb: None };
        let cfg = app_with(vec![rule("api.example.com", true), rule("mine.dev", false), org("*.example.com", false), org("mine.dev", true)], vec![]);
        let p = cedar(&cfg).policies;
        // The organisation's rules as they are.
        assert!(p.contains("@id(\"ollama:org-places-blocked:_.example.com\")\nforbid(") && p.contains("@id(\"ollama:org-place:mine.dev\")\npermit("), "{p}");
        // The person's own allow and block both give way under the organisation's rules.
        let aside = "unless { resource.name like \"*.example.com\" || resource.name == \"mine.dev\" };";
        assert_eq!(p.matches(aside).count(), 2, "{p}");
    }

    #[test]
    fn switches_inside_a_place_follow_the_longest_path() {
        let inside = vec![
            InsideRule { host: "api.example.com".into(), path: "/api".into(), enabled: false, source: "you".into(), method: None },
            InsideRule { host: "api.example.com".into(), path: "/api/ok".into(), enabled: true, source: "you".into(), method: None },
            InsideRule { host: "api.example.com".into(), path: "/upload".into(), enabled: false, source: "you".into(), method: Some("POST".into()) },
        ];
        let p = cedar(&app_with(vec![rule("api.example.com", true)], inside)).policies;
        assert!(p.contains("when { (context.path == \"/api\" || context.path like \"/api/*\") }\nunless { ((context.path == \"/api/ok\" || context.path like \"/api/ok/*\")) };"), "{p}");
        assert!(p.contains("when { (context.path == \"/upload\" || context.path like \"/upload/*\") && context.method == \"POST\" };"));
    }

    #[test]
    fn nothing_in_a_name_or_path_can_break_out_of_its_quotes() {
        assert_eq!(lit("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(like_prefix("/a*b\"", "/*"), "\"/a\\*b\\\"/*\"");
        assert_eq!(policy_id(&["my app", "place", "a b\"c"]), "my_app:place:a_b_c");
    }

    #[test]
    fn an_unprotected_app_gets_no_policies_and_says_so() {
        let mut cfg = app_with(vec![rule("api.example.com", true)], vec![]);
        cfg.apps[0].protected = false;
        let e = cedar(&cfg);
        assert!(!e.policies.contains("permit(") && e.policies.contains("is not protected"));
        assert!(e.entities.contains("\"protected\": false"));
    }
}
