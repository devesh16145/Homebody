//! Plain-language knowledge about destinations and apps.
//!
//! The catalog is what lets a prompt say "Ollama wants to download a model" instead of showing a
//! hostname. It is data, not logic: in the product it becomes a signed, community-maintained
//! preset repository. Entries here were observed in our own spikes (Ollama's startup calls, Claude
//! Code's API and sign-in hosts) or are the documented endpoints of the tools.

use crate::hostname;

pub struct Destination {
    pub pattern: &'static str,
    pub label: &'static str,
    pub purpose: &'static str,
    /// Other names that are part of the same user-visible action. Allowing this destination
    /// (once or always) also allows these, so one action is one prompt.
    pub companions: &'static [&'static str],
}

pub const DESTINATIONS: &[Destination] = &[
    // Anthropic / Claude
    Destination { pattern: "api.anthropic.com", label: "Anthropic API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "claude.ai", label: "Claude sign-in", purpose: "Signing in to your Claude account", companions: &["console.anthropic.com", "platform.claude.com"] },
    Destination { pattern: "console.anthropic.com", label: "Claude sign-in", purpose: "Signing in to your Claude account", companions: &[] },
    Destination { pattern: "platform.claude.com", label: "Claude sign-in", purpose: "Signing in to your Claude account", companions: &[] },
    Destination { pattern: "statsig.anthropic.com", label: "Anthropic usage statistics", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "*.statsig.com", label: "Statsig", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "*.sentry.io", label: "Sentry", purpose: "Crash reports", companions: &[] },
    Destination { pattern: "sentry.io", label: "Sentry", purpose: "Crash reports", companions: &[] },
    // Ollama
    Destination { pattern: "registry.ollama.ai", label: "Ollama model library", purpose: "Model download", companions: &["dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com"] },
    Destination { pattern: "dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com", label: "Ollama model storage", purpose: "Model download", companions: &[] },
    Destination { pattern: "ollama.com", label: "Ollama website", purpose: "Update check and model recommendations", companions: &[] },
    // Other model sources
    Destination { pattern: "huggingface.co", label: "Hugging Face", purpose: "Model download", companions: &["cdn-lfs.huggingface.co", "cdn-lfs.hf.co"] },
    Destination { pattern: "*.huggingface.co", label: "Hugging Face", purpose: "Model download", companions: &[] },
    Destination { pattern: "*.hf.co", label: "Hugging Face", purpose: "Model download", companions: &[] },
    Destination { pattern: "lmstudio.ai", label: "LM Studio website", purpose: "Update check", companions: &[] },
    // Other AI providers
    Destination { pattern: "api.openai.com", label: "OpenAI API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "generativelanguage.googleapis.com", label: "Google Gemini API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "auth.openai.com", label: "OpenAI sign-in", purpose: "Signing in to your OpenAI account", companions: &[] },
    Destination { pattern: "chatgpt.com", label: "ChatGPT", purpose: "Your AI provider (ChatGPT plan)", companions: &[] },
    Destination { pattern: "cloudcode-pa.googleapis.com", label: "Google Gemini (Google account)", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "accounts.google.com", label: "Google sign-in", purpose: "Signing in to your Google account", companions: &["oauth2.googleapis.com"] },
    Destination { pattern: "oauth2.googleapis.com", label: "Google sign-in", purpose: "Signing in to your Google account", companions: &[] },
    Destination { pattern: "play.googleapis.com", label: "Google usage statistics", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "openrouter.ai", label: "OpenRouter", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "api.mistral.ai", label: "Mistral API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "api.groq.com", label: "Groq API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "api.deepseek.com", label: "DeepSeek API", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "models.dev", label: "models.dev", purpose: "List of available models", companions: &[] },
    Destination { pattern: "*.posthog.com", label: "PostHog", purpose: "Usage statistics", companions: &[] },
    // Developer ecosystems
    Destination { pattern: "registry.npmjs.org", label: "npm registry", purpose: "Package downloads", companions: &[] },
    Destination { pattern: "pypi.org", label: "Python Package Index", purpose: "Package downloads", companions: &["files.pythonhosted.org"] },
    Destination { pattern: "files.pythonhosted.org", label: "Python Package Index", purpose: "Package downloads", companions: &[] },
    Destination { pattern: "crates.io", label: "Rust crates", purpose: "Package downloads", companions: &["static.crates.io", "index.crates.io"] },
    Destination { pattern: "static.crates.io", label: "Rust crates", purpose: "Package downloads", companions: &[] },
    Destination { pattern: "index.crates.io", label: "Rust crates", purpose: "Package downloads", companions: &[] },
    Destination { pattern: "github.com", label: "GitHub", purpose: "Code hosting", companions: &[] },
    Destination { pattern: "api.github.com", label: "GitHub", purpose: "Code hosting", companions: &[] },
    Destination { pattern: "raw.githubusercontent.com", label: "GitHub files", purpose: "Code hosting", companions: &[] },
    Destination { pattern: "objects.githubusercontent.com", label: "GitHub files", purpose: "Code hosting", companions: &[] },
    // Places data is commonly exfiltrated to in published attacks. Labelled so a prompt is honest.
    Destination { pattern: "webhook.site", label: "webhook.site", purpose: "A public drop box anyone can read", companions: &[] },
    Destination { pattern: "*.webhook.site", label: "webhook.site", purpose: "A public drop box anyone can read", companions: &[] },
    Destination { pattern: "pastebin.com", label: "Pastebin", purpose: "A public paste site", companions: &[] },
    Destination { pattern: "*.ngrok.io", label: "ngrok tunnel", purpose: "A tunnel to someone's computer", companions: &[] },
    Destination { pattern: "*.ngrok-free.app", label: "ngrok tunnel", purpose: "A tunnel to someone's computer", companions: &[] },
];

pub fn lookup(host: &str) -> Option<&'static Destination> {
    DESTINATIONS.iter().find(|d| !d.pattern.starts_with("*.") && d.pattern == host)
        .or_else(|| DESTINATIONS.iter().find(|d| hostname::matches(d.pattern, host)))
}

/// (label, purpose). Unknown destinations are labelled with their own name and no purpose.
pub fn describe(host: &str) -> (String, String) {
    match lookup(host) {
        Some(d) => (d.label.to_string(), d.purpose.to_string()),
        None => (host.to_string(), String::new()),
    }
}

/// Describe a rule pattern (which may be a wildcard) rather than a concrete name.
pub fn describe_pattern(pattern: &str) -> (String, String) {
    if let Some(d) = DESTINATIONS.iter().find(|d| d.pattern == pattern) {
        return (d.label.to_string(), d.purpose.to_string());
    }
    match pattern.strip_prefix("*.") {
        Some(rest) => match lookup(rest) {
            Some(d) => (d.label.to_string(), d.purpose.to_string()),
            None => (pattern.to_string(), String::new()),
        },
        None => describe(pattern),
    }
}

pub fn companions(host: &str) -> &'static [&'static str] {
    lookup(host).map(|d| d.companions).unwrap_or(&[])
}

pub struct PresetTrust {
    pub host: &'static str,
    /// true: trusted out of the box. false: blocked quietly out of the box (known telemetry).
    pub enabled: bool,
    /// The app does not work without it.
    pub recommended: bool,
}

pub struct AppPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub icon: &'static str,
    /// Executable to look for.
    pub binary: &'static str,
    /// Private OS user this app runs as when protected.
    pub os_user: &'static str,
    pub proxy_port: u16,
    /// Loopback ports this app may connect to besides its own gate (for example a local model).
    pub local_ports: &'static [u16],
    /// Loopback port this app serves on, if it is a local service.
    pub serves_port: Option<u16>,
    /// The app talks to its own helper processes over arbitrary loopback ports (model servers do).
    /// It may then use any loopback port except other apps' gates.
    pub loopback_all: bool,
    pub trust: &'static [PresetTrust],
}

/// Ports reserved for gates. An app is never allowed to connect to a gate other than its own.
pub const GATE_PORTS: std::ops::RangeInclusive<u16> = 47110..=47139;

/// A local model server on its usual port. Every agent may reach it, so an agent and a local
/// model can work together with nothing leaving the computer.
pub const LOCAL_MODEL_PORTS: &[u16] = &[11434];

pub const APPS: &[AppPreset] = &[
    AppPreset {
        id: "claude-code",
        name: "Claude Code",
        kind: "agent",
        icon: "claude",
        binary: "claude",
        os_user: "_saiagent",
        proxy_port: 47110,
        local_ports: &[11434],
        serves_port: None,
        loopback_all: false,
        trust: &[
            PresetTrust { host: "api.anthropic.com", enabled: true, recommended: true },
            PresetTrust { host: "claude.ai", enabled: true, recommended: true },
            PresetTrust { host: "console.anthropic.com", enabled: true, recommended: true },
            PresetTrust { host: "platform.claude.com", enabled: true, recommended: true },
            PresetTrust { host: "statsig.anthropic.com", enabled: false, recommended: false },
            PresetTrust { host: "*.statsig.com", enabled: false, recommended: false },
            PresetTrust { host: "*.sentry.io", enabled: false, recommended: false },
            PresetTrust { host: "sentry.io", enabled: false, recommended: false },
        ],
    },
    AppPreset {
        id: "ollama",
        name: "Ollama",
        kind: "model",
        icon: "ollama",
        binary: "ollama",
        os_user: "_saiollama",
        proxy_port: 47111,
        local_ports: &[],
        serves_port: Some(11434),
        loopback_all: true,
        trust: &[
            // Model downloads are deliberately not listed: the first one asks, in plain words.
            PresetTrust { host: "ollama.com", enabled: false, recommended: false },
        ],
    },
    // The four below follow each tool's documentation. They have not been run on the computer
    // this was written on, so their lists are deliberately short: the provider and its sign-in.
    // Anything else the tool contacts asks first.
    AppPreset {
        id: "codex",
        name: "Codex CLI",
        kind: "agent",
        icon: "generic",
        binary: "codex",
        os_user: "_saicodex",
        proxy_port: 47112,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        trust: &[
            PresetTrust { host: "api.openai.com", enabled: true, recommended: true },
            PresetTrust { host: "auth.openai.com", enabled: true, recommended: true },
            PresetTrust { host: "chatgpt.com", enabled: true, recommended: true },
        ],
    },
    AppPreset {
        id: "gemini",
        name: "Gemini CLI",
        kind: "agent",
        icon: "generic",
        binary: "gemini",
        os_user: "_saigemini",
        proxy_port: 47113,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        trust: &[
            PresetTrust { host: "generativelanguage.googleapis.com", enabled: true, recommended: true },
            PresetTrust { host: "cloudcode-pa.googleapis.com", enabled: true, recommended: true },
            PresetTrust { host: "accounts.google.com", enabled: true, recommended: true },
            PresetTrust { host: "oauth2.googleapis.com", enabled: true, recommended: true },
            PresetTrust { host: "play.googleapis.com", enabled: false, recommended: false },
        ],
    },
    AppPreset {
        id: "aider",
        name: "Aider",
        kind: "agent",
        icon: "generic",
        binary: "aider",
        os_user: "_saiaider",
        proxy_port: 47114,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        // Aider works with whichever provider you give it a key for, so none is assumed.
        trust: &[PresetTrust { host: "*.posthog.com", enabled: false, recommended: false }],
    },
    AppPreset {
        id: "opencode",
        name: "opencode",
        kind: "agent",
        icon: "generic",
        binary: "opencode",
        os_user: "_saiopencode",
        proxy_port: 47115,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        trust: &[],
    },
];

/// Gate ports from here up are handed to apps the user adds.
pub const FIRST_CUSTOM_PORT: u16 = 47120;

pub fn app_preset(id: &str) -> Option<&'static AppPreset> {
    APPS.iter().find(|a| a.id == id)
}

/// Why a protected app could not run this program even though it exists, in plain words.
/// Protected apps cannot enter your home folder, so a program installed there is out of reach.
pub fn binary_problem(path: &str, app_name: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
    if real.starts_with(&home) || std::path::Path::new(path).starts_with(&home) {
        return Some(format!("{app_name} is installed inside your home folder ({path}), where protected apps cannot go. Install it for the whole computer instead, for example with Homebrew or \"npm install -g\"."));
    }
    None
}

/// Find an executable the way a login shell on this platform would. A full path is used as is.
pub fn find_binary(name: &str) -> Option<String> {
    if name.starts_with('/') {
        let p = std::path::Path::new(name);
        // A windowed app is a folder whose name ends in ".app".
        return (p.is_file() || (name.ends_with(".app") && p.is_dir())).then(|| name.to_string());
    }
    let mut dirs: Vec<String> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into(), "/usr/bin".into(), "/bin".into()];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin").to_string_lossy().into_owned());
        dirs.push(home.join(".claude/local").to_string_lossy().into_owned());
    }
    if let Ok(path) = std::env::var("PATH") {
        for p in path.split(':') {
            if !p.is_empty() && !dirs.iter().any(|d| d == p) {
                dirs.push(p.to_string());
            }
        }
    }
    for d in dirs {
        let candidate = std::path::Path::new(&d).join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_pattern_is_valid() {
        for d in DESTINATIONS {
            assert!(hostname::valid_pattern(d.pattern), "{}", d.pattern);
            for c in d.companions {
                assert!(hostname::canonical(c).is_some(), "{c}");
            }
        }
        for a in APPS {
            for t in a.trust {
                assert!(hostname::valid_pattern(t.host), "{}", t.host);
            }
        }
    }

    #[test]
    fn exact_entries_win_over_wildcards() {
        assert_eq!(describe("api.anthropic.com").0, "Anthropic API");
        assert_eq!(describe("o123.ingest.sentry.io").1, "Crash reports");
        assert_eq!(describe("unknown.example.org"), ("unknown.example.org".to_string(), String::new()));
    }

    #[test]
    fn app_ports_are_unique() {
        let mut ports: Vec<u16> = APPS.iter().map(|a| a.proxy_port).collect();
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), APPS.len());
        assert!(APPS.iter().all(|a| GATE_PORTS.contains(&a.proxy_port)));
        assert!(APPS.iter().all(|a| a.proxy_port < FIRST_CUSTOM_PORT), "preset ports stay below the range for added apps");
        let mut users: Vec<&str> = APPS.iter().map(|a| a.os_user).collect();
        users.sort();
        users.dedup();
        assert_eq!(users.len(), APPS.len(), "every app has its own private user");
    }

    #[test]
    fn a_program_inside_the_home_folder_is_reported() {
        let home = dirs::home_dir().unwrap();
        let inside = home.join(".local/bin/aider").to_string_lossy().into_owned();
        assert!(binary_problem(&inside, "Aider").unwrap().contains("inside your home folder"));
        assert!(binary_problem("/opt/homebrew/bin/aider", "Aider").is_none());
        assert_eq!(find_binary("/bin/sh").as_deref(), Some("/bin/sh"));
        assert!(find_binary("/no/such/program").is_none());
    }
}
