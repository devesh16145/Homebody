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

/// A folder brought in for an agent to work on. See `projects`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    /// Folder name inside the workspace.
    pub name: String,
    /// Your original folder.
    pub source: String,
    pub imported_at: String,
}

/// A stretch of Watch mode: from when it was switched on to when it ended.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchWindow {
    pub since: String,
    pub until: String,
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
    /// Added by the user rather than known to this build.
    #[serde(default)]
    pub custom: bool,
    /// A windowed app. It cannot run as another user, so it runs as you under the launch-time
    /// profile alone: its network is fenced, its access to your files is not. `binary` is the
    /// path of the .app, and `os_user` is a placeholder that is never created or used.
    #[serde(default)]
    pub windowed: bool,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
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

fn valid_os_user(u: &str) -> bool {
    u.strip_prefix("_sai").map(|r| !r.is_empty() && r.len() <= 16 && r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())).unwrap_or(false)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_workspace(w: &str) -> bool {
    w.strip_prefix("/Users/Shared/SandboxAI/").map(valid_id).unwrap_or(false)
}

/// A display name with nothing in it but letters, digits, spaces and a little punctuation.
fn clean_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| c.is_alphanumeric() || " -_.+".contains(*c)).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty() && cleaned.chars().count() <= 40 && cleaned.chars().any(|c| c.is_alphanumeric())).then_some(cleaned)
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
    /// When Watch switches itself back to Private. None: it stays on until switched off.
    #[serde(default)]
    pub watch_until: Option<String>,
    /// The last finished Watch whose results have not been reviewed yet.
    #[serde(default)]
    pub watch_review: Option<WatchWindow>,
    pub apps: Vec<AppConfig>,
}

fn yes() -> bool {
    true
}

impl Config {
    pub fn fresh() -> Config {
        let now = crate::now_iso();
        let mut c = Config { version: 1, mode: Mode::Private, mode_since: now.clone(), private_since: Some(now), lockdown: false, ask: true, watch_until: None, watch_review: None, apps: vec![] };
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
                        custom: false,
                        windowed: false,
                        projects: vec![],
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
        // App entries end up in a script an administrator approves and in packet filter rules, so
        // nothing from a hand-edited file may carry characters that mean something there.
        self.apps.retain(|a| valid_os_user(&a.os_user) && valid_id(&a.id) && catalog::GATE_PORTS.contains(&a.proxy_port) && a.workspace.as_deref().map(valid_workspace).unwrap_or(true));
        let mut seen: Vec<(String, String, u16)> = Vec::new();
        self.apps.retain(|a| {
            let clash = seen.iter().any(|(id, user, port)| *id == a.id || *user == a.os_user || *port == a.proxy_port);
            seen.push((a.id.clone(), a.os_user.clone(), a.proxy_port));
            !clash
        });
        for a in &mut self.apps {
            a.name = clean_name(&a.name).unwrap_or_else(|| a.id.clone());
            if a.custom {
                // What an added app may reach on this computer is ours to define, as for presets.
                a.kind = if a.windowed { "window" } else { "agent" }.into();
                a.local_ports = if a.windowed { vec![] } else { catalog::LOCAL_MODEL_PORTS.to_vec() };
                a.serves_port = None;
                a.loopback_all = a.windowed;
                if a.windowed {
                    a.workspace = None;
                    a.projects.clear();
                }
            } else {
                a.windowed = false;
            }
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

    /// Protect a command-line program this build has no preset for. It gets what every agent
    /// gets: a private user, a gate of its own, and a workspace folder. The one-time setup has
    /// to be approved again before it can run, because the private user does not exist yet.
    pub fn add_custom(&mut self, name: &str, program: &str) -> Result<String, String> {
        let name = clean_name(name).ok_or("Give the app a short name using letters and numbers.")?;
        if self.apps.iter().any(|a| a.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("There is already an app called {name}."));
        }
        let program = program.trim();
        if program.is_empty() || program.chars().any(|c| c.is_control()) || program.len() > 512 {
            return Err("Choose the program to run.".into());
        }
        if !program.starts_with('/') && !program.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c)) {
            return Err("Give the program's name (like aider) or choose its file.".into());
        }
        let found = catalog::find_binary(program).ok_or_else(|| format!("{program} was not found on this computer."))?;
        let windowed = found.ends_with(".app");
        if windowed {
            // Fails with a sentence for the user when the app cannot be wrapped.
            crate::fence::inspect_bundle(&found)?;
        } else if let Some(problem) = catalog::binary_problem(&found, &name) {
            return Err(problem);
        }
        let port = (catalog::FIRST_CUSTOM_PORT..=*catalog::GATE_PORTS.end()).find(|p| !self.apps.iter().any(|a| a.proxy_port == *p)).ok_or("No more apps can be added.")?;
        let user_prefix = if windowed { "_saiwin" } else { "_saiapp" };
        let n = (1..100).find(|n| !self.apps.iter().any(|a| a.os_user == format!("{user_prefix}{n}"))).ok_or("No more apps can be added.")?;
        let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        let base = if slug.is_empty() { format!("app-{n}") } else { format!("my-{slug}") };
        let id = (0..100).map(|i| if i == 0 { base.clone() } else { format!("{base}-{i}") }).find(|id| self.app(id).is_none()).ok_or("No more apps can be added.")?;
        self.apps.push(AppConfig {
            id: id.clone(),
            name,
            kind: if windowed { "window" } else { "agent" }.into(),
            icon: "generic".into(),
            binary: program.to_string(),
            os_user: format!("{user_prefix}{n}"),
            proxy_port: port,
            local_ports: if windowed { vec![] } else { catalog::LOCAL_MODEL_PORTS.to_vec() },
            serves_port: None,
            loopback_all: windowed,
            protected: true,
            workspace: if windowed { None } else { Some(format!("/Users/Shared/SandboxAI/{id}")) },
            custom: true,
            windowed,
            projects: vec![],
            trust: vec![],
        });
        Ok(id)
    }

    /// Forget an app the user added. Its private user and workspace stay on disk until setup is
    /// removed; they are inert without the entry.
    pub fn remove_custom(&mut self, id: &str) -> Result<(), String> {
        match self.app(id) {
            None => Err("Unknown app.".into()),
            Some(a) if !a.custom => Err(format!("{} is built in. Switch its protection off instead.", a.name)),
            Some(_) => {
                self.apps.retain(|a| a.id != id);
                Ok(())
            }
        }
    }

    /// The apps that get a private space on this computer: the ones that are installed, and
    /// the ones the user added. A recognised app that is not installed gets nothing until it is;
    /// installing it later asks for one more approval of setup.
    pub fn provisioned(&self) -> Config {
        let mut c = self.clone();
        c.apps.retain(|a| !a.windowed && (a.custom || catalog::find_binary(&a.binary).is_some()));
        c
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
    fn adding_your_own_app_gives_it_a_user_a_gate_and_a_workspace() {
        let mut c = Config::fresh();
        let id = c.add_custom("My Script!", "/bin/sh").unwrap();
        let a = c.app(&id).unwrap().clone();
        assert_eq!((a.id.as_str(), a.name.as_str(), a.os_user.as_str(), a.proxy_port), ("my-my-script", "My Script", "_saiapp1", catalog::FIRST_CUSTOM_PORT));
        assert_eq!(a.workspace.as_deref(), Some("/Users/Shared/SandboxAI/my-my-script"));
        assert!(a.custom && a.protected && a.trust.is_empty() && a.kind == "agent");
        // A second one gets the next user and port; names must differ.
        assert!(c.add_custom("my script", "/bin/sh").unwrap_err().contains("already"));
        let id2 = c.add_custom("Other", "sh").unwrap();
        let b = c.app(&id2).unwrap();
        assert_eq!((b.os_user.as_str(), b.proxy_port), ("_saiapp2", catalog::FIRST_CUSTOM_PORT + 1));
        // Presets cannot be removed, added apps can.
        assert!(c.remove_custom("ollama").is_err());
        c.remove_custom(&id).unwrap();
        assert!(c.app(&id).is_none());
        // Nothing that is not a program, and nothing with odd characters.
        assert!(c.add_custom("X", "/no/such/program").is_err());
        assert!(c.add_custom("X", "sh; rm -rf /").is_err());
        assert!(c.add_custom("\n\n", "/bin/sh").is_err());
        let home = dirs::home_dir().unwrap().join("sai-test-not-there").to_string_lossy().into_owned();
        assert!(c.add_custom("X", &home).is_err());
    }

    #[test]
    fn a_hand_edited_file_cannot_smuggle_text_into_setup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let mut c = Config::fresh();
        let id = c.add_custom("Mine", "/bin/sh").unwrap();
        let n = c.apps.len();
        // An app whose user name would add a line to the administrator's permission file.
        let mut evil = c.app(&id).unwrap().clone();
        evil.id = "evil".into();
        evil.os_user = "_saix ALL=(ALL) NOPASSWD: ALL\nroot".into();
        evil.proxy_port = 47130;
        c.apps.push(evil);
        // One whose name would add packet filter rules, one pointing its workspace elsewhere,
        // and one reusing another app's gate.
        let mut named = c.app(&id).unwrap().clone();
        named.id = "named".into();
        named.os_user = "_sainamed".into();
        named.proxy_port = 47131;
        named.name = "Nice\npass out quick all".into();
        named.local_ports = vec![22, 5432];
        named.loopback_all = true;
        c.apps.push(named);
        let mut elsewhere = c.app(&id).unwrap().clone();
        elsewhere.id = "elsewhere".into();
        elsewhere.os_user = "_saielse".into();
        elsewhere.proxy_port = 47132;
        elsewhere.workspace = Some("/Users/Shared/SandboxAI/../../etc".into());
        c.apps.push(elsewhere);
        let mut clash = c.app(&id).unwrap().clone();
        clash.id = "clash".into();
        clash.os_user = "_saiclash".into();
        clash.proxy_port = 47110;
        c.apps.push(clash);
        c.save(&p).unwrap();

        let loaded = Config::load(&p);
        assert_eq!(loaded.apps.len(), n + 1, "only the renamed one survives, cleaned");
        assert!(loaded.app("evil").is_none() && loaded.app("elsewhere").is_none() && loaded.app("clash").is_none());
        let named = loaded.app("named").unwrap();
        assert_eq!(named.name, "Nicepass out quick all");
        assert!(!named.loopback_all && named.local_ports == catalog::LOCAL_MODEL_PORTS);
        let rules = crate::fence::filter_rules(&loaded, true) + &crate::fence::filter_rules(&loaded, false);
        assert!(rules.lines().all(|l| l.is_empty() || l.starts_with('#') || l.starts_with("pass ") || l.starts_with("block ") || l == "anchor \"*\""));
        assert!(!rules.lines().any(|l| l.trim() == "pass out quick all"), "the name stays inside one comment line");
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
