//! The user's settings: mode, protected apps and their trusted destinations.
//! Stored as one human-readable JSON file, written atomically.

use crate::catalog;
use crate::hostname;
use crate::model::Mode;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrustRule {
    pub host: String,
    /// true: trusted. false: blocked without asking.
    pub enabled: bool,
    /// "preset" or "you".
    pub source: String,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub icon: String,
    pub binary: String,
    pub os_user: String,
    pub proxy_port: u16,
    #[serde(default)]
    pub local_ports: Vec<u16>,
    #[serde(default)]
    pub serves_port: Option<u16>,
    #[serde(default)]
    pub loopback_all: bool,
    pub protected: bool,
    pub workspace: Option<String>,
    pub trust: Vec<TrustRule>,
}

impl AppConfig {
    /// The rule that decides this destination, if any. Exact rules win over wildcards, and among
    /// rules of the same kind a block wins over an allow, so adding a block can never be shadowed.
    pub fn rule_for(&self, host: &str) -> Option<&TrustRule> {
        let exact = self.trust.iter().filter(|r| !r.host.starts_with("*.") && r.host == host);
        if let Some(r) = pick_strictest(exact) {
            return Some(r);
        }
        pick_strictest(self.trust.iter().filter(|r| r.host.starts_with("*.") && hostname::matches(&r.host, host)))
    }
}

fn pick_strictest<'a>(rules: impl Iterator<Item = &'a TrustRule>) -> Option<&'a TrustRule> {
    let mut found: Option<&TrustRule> = None;
    for r in rules {
        match found {
            None => found = Some(r),
            Some(f) if f.enabled && !r.enabled => found = Some(r),
            _ => {}
        }
    }
    found
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub mode: Mode,
    pub mode_since: String,
    pub private_since: Option<String>,
    #[serde(default)]
    pub lockdown: bool,
    /// Ask about destinations that are neither trusted nor blocked. Off means block them quietly.
    #[serde(default = "yes")]
    pub ask: bool,
    pub apps: Vec<AppConfig>,
}

fn yes() -> bool {
    true
}

impl Config {
    pub fn fresh() -> Config {
        let now = crate::now_iso();
        let mut c = Config { version: 1, mode: Mode::Private, mode_since: now.clone(), private_since: Some(now), lockdown: false, ask: true, apps: vec![] };
        c.merge_presets();
        c
    }

    /// Add apps and preset destinations that this build knows about and the file does not.
    /// Never changes anything the user has touched.
    pub fn merge_presets(&mut self) -> bool {
        let mut changed = false;
        for p in catalog::APPS {
            let app = match self.apps.iter_mut().find(|a| a.id == p.id) {
                Some(a) => a,
                None => {
                    self.apps.push(AppConfig {
                        id: p.id.into(),
                        name: p.name.into(),
                        kind: p.kind.into(),
                        icon: p.icon.into(),
                        binary: p.binary.into(),
                        os_user: p.os_user.into(),
                        proxy_port: p.proxy_port,
                        local_ports: p.local_ports.to_vec(),
                        serves_port: p.serves_port,
                        loopback_all: p.loopback_all,
                        protected: true,
                        workspace: if p.kind == "agent" { Some(format!("/Users/Shared/SandboxAI/{}", p.id)) } else { None },
                        trust: vec![],
                    });
                    changed = true;
                    self.apps.last_mut().unwrap()
                }
            };
            // How an app is fenced is ours to define, not a user setting: keep it in step with this build.
            if app.loopback_all != p.loopback_all || app.proxy_port != p.proxy_port || app.local_ports != p.local_ports || app.serves_port != p.serves_port || app.os_user != p.os_user || app.binary != p.binary {
                app.loopback_all = p.loopback_all;
                app.proxy_port = p.proxy_port;
                app.local_ports = p.local_ports.to_vec();
                app.serves_port = p.serves_port;
                app.os_user = p.os_user.into();
                app.binary = p.binary.into();
                changed = true;
            }
            for t in p.trust {
                if !app.trust.iter().any(|r| r.host == t.host) {
                    app.trust.push(TrustRule { host: t.host.into(), enabled: t.enabled, source: "preset".into(), recommended: t.recommended });
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn load(path: &Path) -> Config {
        match std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Config>(&b).ok()) {
            Some(mut c) => {
                c.merge_presets();
                c.sanitize();
                c
            }
            None => Config::fresh(),
        }
    }

    /// Drop anything in the file that is not a valid rule. A hand-edited settings file must never
    /// be able to introduce a pattern the matcher was not designed for.
    fn sanitize(&mut self) {
        for a in &mut self.apps {
            a.trust.retain(|r| hostname::valid_pattern(&r.host));
            for r in &mut a.trust {
                r.host = hostname::normalize_pattern(&r.host);
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path)
    }

    pub fn app(&self, id: &str) -> Option<&AppConfig> {
        self.apps.iter().find(|a| a.id == id)
    }

    pub fn app_mut(&mut self, id: &str) -> Option<&mut AppConfig> {
        self.apps.iter_mut().find(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(rules: &[(&str, bool)]) -> AppConfig {
        let mut c = Config::fresh();
        let mut a = c.apps.remove(0);
        a.trust = rules.iter().map(|(h, e)| TrustRule { host: h.to_string(), enabled: *e, source: "you".into(), recommended: false }).collect();
        a
    }

    #[test]
    fn exact_rule_beats_wildcard() {
        let a = app_with(&[("*.example.com", true), ("bad.example.com", false)]);
        assert!(!a.rule_for("bad.example.com").unwrap().enabled);
        assert!(a.rule_for("ok.example.com").unwrap().enabled);
        assert!(a.rule_for("example.com").is_none());
    }

    #[test]
    fn block_beats_allow_at_the_same_level() {
        let a = app_with(&[("*.example.com", true), ("*.cdn.example.com", false)]);
        assert!(!a.rule_for("x.cdn.example.com").unwrap().enabled);
        let b = app_with(&[("example.com", true), ("example.com", false)]);
        assert!(!b.rule_for("example.com").unwrap().enabled);
    }

    #[test]
    fn empty_rules_mean_nothing_is_trusted() {
        let a = app_with(&[]);
        assert!(a.rule_for("api.anthropic.com").is_none());
    }

    #[test]
    fn fresh_config_has_presets_and_starts_private() {
        let c = Config::fresh();
        assert_eq!(c.mode, Mode::Private);
        let claude = c.app("claude-code").unwrap();
        assert!(claude.rule_for("api.anthropic.com").unwrap().enabled);
        assert!(!claude.rule_for("statsig.anthropic.com").unwrap().enabled);
        assert!(!claude.rule_for("o1.ingest.sentry.io").unwrap().enabled);
        assert!(claude.rule_for("webhook.site").is_none());
        let ollama = c.app("ollama").unwrap();
        assert!(!ollama.rule_for("ollama.com").unwrap().enabled);
        assert!(ollama.rule_for("registry.ollama.ai").is_none());
    }

    #[test]
    fn invalid_patterns_in_a_file_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let mut c = Config::fresh();
        c.apps[0].trust.push(TrustRule { host: "*".into(), enabled: true, source: "you".into(), recommended: false });
        c.apps[0].trust.push(TrustRule { host: "127.0.0.1".into(), enabled: true, source: "you".into(), recommended: false });
        c.apps[0].trust.push(TrustRule { host: "*.com".into(), enabled: true, source: "you".into(), recommended: false });
        c.save(&p).unwrap();
        let loaded = Config::load(&p);
        assert!(loaded.apps[0].trust.iter().all(|r| hostname::valid_pattern(&r.host)));
        assert!(!loaded.apps[0].trust.iter().any(|r| r.host == "*" || r.host == "127.0.0.1" || r.host == "*.com"));
    }
}
