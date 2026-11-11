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
    Destination { pattern: "files.openai.com", label: "OpenAI files", purpose: "Files from your AI provider", companions: &[] },
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
    Destination { pattern: "opencode.ai", label: "opencode", purpose: "Account and hosted models", companions: &[] },
    Destination { pattern: "*.opencode.ai", label: "opencode hosted models", purpose: "An AI provider run by opencode", companions: &[] },
    Destination { pattern: "models.dev", label: "models.dev", purpose: "List of available models", companions: &[] },
    Destination { pattern: "*.i.posthog.com", label: "PostHog", purpose: "Usage statistics", companions: &[] },
    // Seen on real runs of windowed apps (Cursor, Antigravity, Comfy Desktop), November 2026.
    Destination { pattern: "metrics.cursor.sh", label: "Cursor usage statistics", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "*.cursor.sh", label: "Cursor", purpose: "Your AI provider", companions: &[] },
    Destination { pattern: "*.cursor.com", label: "Cursor", purpose: "Updates and account", companions: &[] },
    Destination { pattern: "marketplace.cursorapi.com", label: "Cursor extensions", purpose: "Extension downloads", companions: &[] },
    Destination { pattern: "*.comfy.org", label: "Comfy", purpose: "App files and updates", companions: &[] },
    Destination { pattern: "*.todesktop.com", label: "ToDesktop", purpose: "App updates", companions: &[] },
    Destination { pattern: "browser-intake-us5-datadoghq.com", label: "Datadog", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "*.datadoghq.com", label: "Datadog", purpose: "Usage statistics", companions: &[] },
    Destination { pattern: "storage.googleapis.com", label: "Google Cloud Storage", purpose: "File downloads", companions: &[] },
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

/// A website where an AI is used, and the places a browser needs to show it and sign in.
///
/// These are a starting set, read from each site with a fresh browser profile. A site may need
/// another place later (after sign-in, or when it changes); that place asks like any other.
pub struct AiSite {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    /// (place, what it is for)
    pub hosts: &'static [(&'static str, &'static str)],
}

const PERSON_CHECK: (&str, &str) = ("challenges.cloudflare.com", "The check that you are a person");

pub const AI_SITES: &[AiSite] = &[
    AiSite { id: "chatgpt", name: "ChatGPT", url: "https://chatgpt.com/", hosts: &[
        ("chatgpt.com", "The site itself"), ("*.chatgpt.com", "The site itself"), ("*.openai.com", "Sign-in and the AI itself"),
        ("*.oaistatic.com", "Its scripts and images"), ("*.oaiusercontent.com", "Files and images in your chats"), PERSON_CHECK,
    ] },
    AiSite { id: "claude", name: "Claude", url: "https://claude.ai/", hosts: &[
        ("claude.ai", "The site itself"), ("*.claude.ai", "The site itself"), ("*.anthropic.com", "Sign-in and the AI itself"),
        ("*.claudeusercontent.com", "Files and images in your chats"), PERSON_CHECK, ("*.hcaptcha.com", "The check that you are a person"),
    ] },
    AiSite { id: "gemini", name: "Gemini", url: "https://gemini.google.com/", hosts: &[
        ("gemini.google.com", "The site itself"), ("accounts.google.com", "Google sign-in"), ("www.google.com", "Google's shared pages"),
        ("*.gstatic.com", "Its scripts and images"), ("*.googleusercontent.com", "Files and images in your chats"),
        ("*.googleapis.com", "Google services the site calls"), ("ogs.google.com", "The Google account menu"), ("apis.google.com", "Google services the site calls"),
        ("waa-pa.clients6.google.com", "The check that you are a person"),
    ] },
    AiSite { id: "copilot", name: "Copilot", url: "https://copilot.microsoft.com/", hosts: &[
        ("copilot.microsoft.com", "The site itself"), ("*.copilot.microsoft.com", "The site itself"), ("copilot.com", "The site itself"), ("*.copilot.com", "The site itself"),
        ("login.microsoftonline.com", "Microsoft sign-in"),
        ("login.live.com", "Microsoft sign-in"), ("*.msauth.net", "Microsoft sign-in"), ("*.msftauth.net", "Microsoft sign-in"), ("*.bing.com", "Search and images in answers"),
    ] },
    AiSite { id: "perplexity", name: "Perplexity", url: "https://www.perplexity.ai/", hosts: &[
        ("www.perplexity.ai", "The site itself"), ("*.perplexity.ai", "The site itself"), PERSON_CHECK,
    ] },
    AiSite { id: "mistral", name: "Le Chat", url: "https://chat.mistral.ai/", hosts: &[
        ("chat.mistral.ai", "The site itself"), ("*.mistral.ai", "Sign-in and the AI itself"),
    ] },
    AiSite { id: "deepseek", name: "DeepSeek", url: "https://chat.deepseek.com/", hosts: &[
        ("chat.deepseek.com", "The site itself"), ("*.deepseek.com", "Sign-in and the AI itself"), ("*.awswaf.com", "The check that you are a person"),
    ] },
    AiSite { id: "grok", name: "Grok", url: "https://grok.com/", hosts: &[
        ("grok.com", "The site itself"), ("*.grok.com", "The site itself"), ("*.x.ai", "Sign-in and the AI itself"),
    ] },
];

pub fn ai_site(id: &str) -> Option<&'static AiSite> {
    AI_SITES.iter().find(|s| s.id == id)
}

/// The site (and purpose) a browser rule belongs to, for labelling it.
pub fn ai_site_for(pattern: &str) -> Option<(&'static AiSite, &'static str)> {
    AI_SITES.iter().find_map(|s| s.hosts.iter().find(|(h, _)| *h == pattern).map(|(_, why)| (s, *why)))
}

/// The browser maker's own background traffic: update checks, statistics, lists it downloads.
/// None of it is needed to use an AI site, so a browser profile starts with these switched off
/// instead of asking about each one. They can be switched on like any other place.
pub const BROWSER_BACKGROUND: &[&str] = &[
    "clients2.google.com", "update.googleapis.com", "optimizationguide-pa.googleapis.com", "safebrowsing.googleapis.com", "content-autofill.googleapis.com",
    "clientservices.googleapis.com", "*.gvt1.com", "*.gvt2.com", "android.clients.google.com",
    "go-updater.brave.com", "variations.brave.com", "*.bravesoftware.com", "p3a.brave.com",
    "edge.microsoft.com", "config.edge.skype.com", "msedge.api.cdp.microsoft.com", "*.vivaldi.com",
    // Statistics, advertising and crash reports that sites load: also not needed to use them.
    "www.googletagmanager.com", "*.google-analytics.com", "*.doubleclick.net", "play.google.com", "ogads-pa.clients6.google.com",
    "*.sentry.io", "static.cloudflareinsights.com", "cloudflareinsights.com", "*.datadoghq.com", "browser-intake-datadoghq.com",
    "cdn.segment.com", "api.segment.io", "*.amplitude.com", "*.mixpanel.com", "*.hotjar.com", "*.clarity.ms",
];

/// Browsers that can keep a separate profile pointed at a gate, by the name of the .app.
pub const KNOWN_BROWSERS: &[&str] = &["Google Chrome", "Brave Browser", "Microsoft Edge", "Chromium", "Vivaldi"];

pub fn companions(host: &str) -> &'static [&'static str] {
    lookup(host).map(|d| d.companions).unwrap_or(&[])
}

/// Plain words for addresses inside a known place, for when its connections are looked inside.
/// (place, start of the path, label, purpose). Observed on real runs, November 2026.
pub const INSIDE: &[(&str, &str, &str, &str)] = &[
    ("api.anthropic.com", "/v1/messages", "Conversation", "What you and the AI say"),
    ("api.anthropic.com", "/api/event_logging", "Event logging", "Usage statistics"),
    ("api.anthropic.com", "/mcp-registry", "List of tool servers", "Start-up check"),
    ("api.anthropic.com", "/api/claude_cli/bootstrap", "Start-up settings", "Start-up check"),
    ("api.openai.com", "/v1/responses", "Conversation", "What you and the AI say"),
    ("api.openai.com", "/v1/chat/completions", "Conversation", "What you and the AI say"),
    ("generativelanguage.googleapis.com", "/v1beta/models", "Conversation", "What you and the AI say"),
];

/// Whether `path` is `prefix` or lies under it, by whole path segments.
pub fn path_under(path: &str, prefix: &str) -> bool {
    path == prefix || prefix.ends_with('/') && path.starts_with(prefix) || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

/// (label, purpose) for an address inside a place; empty when we have no words for it.
pub fn describe_inside(host: &str, path: &str) -> (String, String) {
    INSIDE.iter().filter(|(h, p, _, _)| *h == host && path_under(path, p)).max_by_key(|(_, p, _, _)| p.len()).map(|(_, _, l, u)| (l.to_string(), u.to_string())).unwrap_or_default()
}

pub struct PresetInside {
    pub host: &'static str,
    /// Start of the path, matched by whole segments.
    pub path: &'static str,
    /// false: switched off out of the box (known usage statistics), like the places above.
    pub enabled: bool,
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
    /// Switches for addresses inside trusted places. They only act while the app's connections
    /// are being looked inside.
    pub inside: &'static [PresetInside],
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
        os_user: "_hbagent",
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
        // Seen on a real run: 225 KB of event logging beside a 49 KB conversation request.
        inside: &[PresetInside { host: "api.anthropic.com", path: "/api/event_logging", enabled: false }],
    },
    AppPreset {
        id: "ollama",
        name: "Ollama",
        kind: "model",
        icon: "ollama",
        binary: "ollama",
        os_user: "_hbollama",
        proxy_port: 47111,
        local_ports: &[],
        serves_port: Some(11434),
        loopback_all: true,
        trust: &[
            // Model downloads are deliberately not listed: the first one asks, in plain words.
            PresetTrust { host: "ollama.com", enabled: false, recommended: false },
        ],
        inside: &[],
    },
    // The four below started from each tool's documentation and were then run protected, not
    // signed in, on 11 Nov 2026. Seen: Codex reached api.openai.com and chatgpt.com and asked for
    // files.openai.com and GitHub (update check); Gemini reached its API and tried
    // play.googleapis.com (usage statistics, blocked here); Aider asked for its provider, for
    // raw.githubusercontent.com (model list) and pypi.org (update check). All of it went through
    // the gate. opencode talks to a local server of its own, so it needs "own local servers";
    // with that on it ran, and asked for opencode.ai and models.opencode.ai (its hosted models).
    // The lists stay short on purpose: the provider and its sign-in. Anything else asks first.
    AppPreset {
        id: "codex",
        name: "Codex CLI",
        kind: "agent",
        icon: "generic",
        binary: "codex",
        os_user: "_hbcodex",
        proxy_port: 47112,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        trust: &[
            PresetTrust { host: "api.openai.com", enabled: true, recommended: true },
            PresetTrust { host: "auth.openai.com", enabled: true, recommended: true },
            PresetTrust { host: "chatgpt.com", enabled: true, recommended: true },
        ],
        inside: &[],
    },
    AppPreset {
        id: "gemini",
        name: "Gemini CLI",
        kind: "agent",
        icon: "generic",
        binary: "gemini",
        os_user: "_hbgemini",
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
        inside: &[],
    },
    AppPreset {
        id: "aider",
        name: "Aider",
        kind: "agent",
        icon: "generic",
        binary: "aider",
        os_user: "_hbaider",
        proxy_port: 47114,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        // Aider works with whichever provider you give it a key for, so none is assumed.
        trust: &[PresetTrust { host: "*.i.posthog.com", enabled: false, recommended: false }],
        inside: &[],
    },
    AppPreset {
        id: "opencode",
        name: "opencode",
        kind: "agent",
        icon: "generic",
        binary: "opencode",
        os_user: "_hbopencode",
        proxy_port: 47115,
        local_ports: LOCAL_MODEL_PORTS,
        serves_port: None,
        loopback_all: false,
        trust: &[],
        inside: &[],
    },
    // Recognised by name only. Nothing is trusted for them out of the box: whatever they
    // contact asks first. None has been run on the computer this was written on.
    AppPreset { id: "llama-cpp", name: "llama.cpp", kind: "agent", icon: "generic", binary: "llama-server", os_user: "_hbllama", proxy_port: 47116, local_ports: LOCAL_MODEL_PORTS, serves_port: None, loopback_all: false, trust: &[], inside: &[] },
    AppPreset { id: "cursor-agent", name: "Cursor CLI", kind: "agent", icon: "generic", binary: "cursor-agent", os_user: "_hbcursor", proxy_port: 47117, local_ports: LOCAL_MODEL_PORTS, serves_port: None, loopback_all: false, trust: &[], inside: &[] },
    AppPreset { id: "openclaw", name: "OpenClaw", kind: "agent", icon: "generic", binary: "openclaw", os_user: "_hbopenclaw", proxy_port: 47118, local_ports: LOCAL_MODEL_PORTS, serves_port: None, loopback_all: false, trust: &[], inside: &[] },
];

/// The built-in rules have a version, and every change to them a line here, newest last, so
/// that after an update the app can say what changed instead of changing rules silently.
/// The rules ship inside the app, so they are covered by whatever vouches for the app itself.
pub const PRESET_NOTES: &[(u32, &str)] = &[
    (1, "Claude Code and Ollama: the AI provider and sign-in are trusted; usage statistics and crash reports are switched off."),
    (2, "Codex CLI, Gemini CLI, Aider and opencode are recognised. For each, only its AI provider and sign-in are trusted."),
    (3, "Claude Code: event logging inside Anthropic's address is switched off. It acts while Look inside is on."),
    (4, "llama.cpp, Cursor CLI and OpenClaw are recognised. Nothing is trusted for them until you say so."),
];

pub fn presets_version() -> u32 {
    PRESET_NOTES.last().map(|(v, _)| *v).unwrap_or(0)
}

/// Windowed AI apps worth pointing out when they are installed, by the name of the .app.
/// Only a hint for "found on this computer": anything else can still be added by hand.
pub const KNOWN_WINDOWED: &[&str] = &[
    "LM Studio", "Jan", "Ollama", "Msty", "GPT4All", "AnythingLLM", "Open WebUI", "Comfy Desktop", "ComfyUI", "Draw Things", "DiffusionBee",
    "ChatGPT", "Claude", "Gemini", "Copilot", "Perplexity", "DeepSeek", "Poe", "Cherry Studio", "Chatbox", "BoltAI", "Hermes", "Bionic",
    "Cursor", "Windsurf", "Antigravity", "Kiro", "Trae", "Void", "Codex", "Goose",
];

pub fn is_known_windowed(app_name: &str) -> bool {
    KNOWN_WINDOWED.iter().any(|k| k.eq_ignore_ascii_case(app_name.trim()))
}

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
    fn browser_background_places_are_valid_and_no_site_depends_on_one() {
        for b in BROWSER_BACKGROUND {
            assert!(hostname::valid_pattern(b), "{b}");
            for s in AI_SITES {
                assert!(!s.hosts.iter().any(|(h, _)| h == b), "{b} is needed by {}", s.id);
            }
        }
    }

    #[test]
    fn every_ai_site_names_valid_places_and_a_secure_address() {
        let mut ids = std::collections::HashSet::new();
        for s in AI_SITES {
            assert!(ids.insert(s.id), "{} twice", s.id);
            assert!(s.url.starts_with("https://") && !s.hosts.is_empty(), "{}", s.id);
            for (h, why) in s.hosts {
                assert!(hostname::valid_pattern(h) && !why.is_empty(), "{h}");
            }
            // The address the browser opens must itself be among the places switched on.
            let host = s.url.trim_start_matches("https://").trim_end_matches('/');
            assert!(s.hosts.iter().any(|(h, _)| hostname::matches(h, host)), "{} cannot reach its own address", s.id);
        }
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
    fn addresses_inside_a_place_are_matched_by_whole_segments() {
        assert!(path_under("/api/event_logging", "/api/event_logging"));
        assert!(path_under("/api/event_logging/v2/batch", "/api/event_logging"));
        assert!(!path_under("/api/event_logging_other", "/api/event_logging"));
        assert!(!path_under("/api", "/api/event_logging"));
        assert!(path_under("/anything", "/"));
        assert_eq!(describe_inside("api.anthropic.com", "/api/event_logging/v2/batch").0, "Event logging");
        assert_eq!(describe_inside("api.anthropic.com", "/unknown"), (String::new(), String::new()));
        assert_eq!(describe_inside("example.com", "/v1/messages"), (String::new(), String::new()));
        for a in APPS {
            for r in a.inside {
                assert!(r.path.starts_with('/') && hostname::canonical(r.host).is_some(), "{} {}", r.host, r.path);
                assert!(a.trust.iter().any(|t| t.host == r.host && t.enabled), "{}: a switch inside a place that is not trusted", a.id);
            }
        }
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
