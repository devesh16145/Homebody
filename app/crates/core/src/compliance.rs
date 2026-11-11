//! Where Homebody's evidence fits in two public control catalogues.
//!
//! Organisations that have to show how they control AI tools usually do it against a catalogue
//! of controls. This table says, control by control, what Homebody does about it and which file
//! in an export shows it. It goes into every export as `compliance.json` and is the same table
//! as `docs/compliance.md` (a test keeps the two in step).
//!
//! It is the project's own mapping, made in good faith and not reviewed by an auditor. "partly"
//! and "no" are used wherever they are true: a mapping that claims everything is worth nothing.
//!
//! - NIST SP 800-53 Rev. 5 base controls. NIST's overlays for AI agents (the COSAiS project)
//!   were still being written when this was made; the mapping will follow them once published.
//! - OWASP Top 10 for Agentic Applications (2026), ASI01 to ASI10.

use serde_json::{json, Value};

pub struct Control {
    pub catalogue: &'static str,
    pub id: &'static str,
    pub name: &'static str,
    /// "yes": Homebody does this for protected apps. "partly": it helps, and the note says
    /// where it stops. "no": out of its reach, listed so nobody assumes otherwise.
    pub coverage: &'static str,
    pub how: &'static str,
    /// Files in an export that show it.
    pub evidence: &'static [&'static str],
}

const NIST: &str = "NIST SP 800-53 Rev. 5";
const OWASP: &str = "OWASP Top 10 for Agentic Applications 2026";

pub const CONTROLS: &[Control] = &[
    Control { catalogue: NIST, id: "AC-3", name: "Access Enforcement", coverage: "yes", how: "Each protected app runs as a private user of its own with no access to your home folder; it can write only in its workspace.", evidence: &["rules.json"] },
    Control { catalogue: NIST, id: "AC-4", name: "Information Flow Enforcement", coverage: "yes", how: "A protected app can send only to places that are trusted for it. The decision is made at its gate and enforced again by the operating system, so going around the gate is refused too.", evidence: &["rules.json", "policy/policy.cedar", "log.jsonl"] },
    Control { catalogue: NIST, id: "AC-4(4)", name: "Flow Control of Encrypted Information", coverage: "partly", how: "With Look inside on, encrypted connections to trusted places are opened at the gate, checked and re-encrypted; a connection that cannot be checked is cut. Off by default, and not available for apps that take their trusted certificates from the system.", evidence: &["log.jsonl", "activity.csv"] },
    Control { catalogue: NIST, id: "AC-6", name: "Least Privilege", coverage: "yes", how: "Protected apps run without administrator rights, without your environment (keys, tokens), and on Linux without the ability to gain privileges.", evidence: &["rules.json"] },
    Control { catalogue: NIST, id: "AU-2", name: "Event Logging", coverage: "yes", how: "Every connection decision, every question and answer, and every change to the rules, the mode or the setup is logged.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "AU-3", name: "Content of Audit Records", coverage: "yes", how: "Each record says which app, which place and port, what was decided and why, how much was sent and received, and when.", evidence: &["log.jsonl", "activity.csv"] },
    Control { catalogue: NIST, id: "AU-8", name: "Time Stamps", coverage: "yes", how: "Records carry UTC times to the millisecond. An independent time-stamping service can vouch for when the log existed.", evidence: &["log.jsonl", "timestamps/"] },
    Control { catalogue: NIST, id: "AU-9", name: "Protection of Audit Information", coverage: "yes", how: "Records are chained by hash and each is signed (Ed25519), so a changed, removed or reordered record is detected. Protected apps cannot reach the log or its key.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "AU-9(3)", name: "Cryptographic Protection", coverage: "yes", how: "Hash chain and per-record signatures; anyone can check them offline with `homebody verify`.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "AU-10", name: "Non-repudiation", coverage: "partly", how: "Records are signed with a key that never leaves the computer, and an outside time stamp fixes when they existed. The key belongs to the computer's user, not to a hardware device.", evidence: &["log.jsonl", "timestamps/"] },
    Control { catalogue: NIST, id: "AU-11", name: "Audit Record Retention", coverage: "partly", how: "The log is kept on the computer until its owner removes it. A fleet server keeps what devices send for a period its administrator sets.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "AU-12", name: "Audit Record Generation", coverage: "yes", how: "Records are written by the engine at the moment of each decision, for every protected app.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "CM-7", name: "Least Functionality", coverage: "yes", how: "Usage statistics, crash reports and update checks of protected apps are switched off unless someone switches them on, per app.", evidence: &["rules.json", "policy/policy.cedar"] },
    Control { catalogue: NIST, id: "IR-4", name: "Incident Handling", coverage: "partly", how: "One switch blocks everything for every protected app and cuts open connections. What happened before is in the log. Homebody does not run an incident process for you.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "SC-7", name: "Boundary Protection", coverage: "yes", how: "Each protected app has one managed way out, its gate. Name lookups go through the gate too.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: NIST, id: "SC-7(5)", name: "Deny by Default — Allow by Exception", coverage: "yes", how: "A place with no rule is refused (after asking, when asking is on). Empty rules never mean allow.", evidence: &["policy/policy.cedar", "log.jsonl"] },
    Control { catalogue: NIST, id: "SC-7(8)", name: "Route Traffic to Authenticated Proxy Servers", coverage: "partly", how: "All of a protected app's traffic goes through its own gate, which knows the app by the private user it runs as. The gate does not ask the app to sign in.", evidence: &["rules.json"] },
    Control { catalogue: NIST, id: "SC-7(10)", name: "Prevent Exfiltration", coverage: "partly", how: "Sending is limited to trusted places, optionally to an hourly amount, and with Look inside on to requests that carry no recognised secret and no lines from folders marked private. Data hidden inside an allowed request in a form Homebody does not recognise is not caught.", evidence: &["log.jsonl", "rules.json"] },
    Control { catalogue: NIST, id: "SI-4", name: "System Monitoring", coverage: "yes", how: "Every connection of a protected app is seen and shown live, including attempts the operating system refused.", evidence: &["log.jsonl", "activity.csv"] },
    Control { catalogue: NIST, id: "SI-4(4)", name: "Inbound and Outbound Communications Traffic", coverage: "partly", how: "Outbound traffic of protected apps is monitored. Inbound connections from other computers to a protected app are refused, not analysed.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "SI-4(18)", name: "Analyze Traffic and Covert Exfiltration", coverage: "partly", how: "Hourly send limits act as a brake, and requests are scanned for secrets and private text while Look inside is on. Timing channels and client-side encryption are not seen; the Report screen says so.", evidence: &["log.jsonl"] },
    Control { catalogue: NIST, id: "SI-7", name: "Software, Firmware, and Information Integrity", coverage: "partly", how: "The engine can be rebuilt from source to the same file (scripts/repro-check.sh), and the log detects its own alteration. The desktop app's signature depends on who distributes it.", evidence: &["log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI01", name: "Agent Goal Hijack", coverage: "partly", how: "Homebody does not stop an agent being talked into something. It limits what a hijacked agent can do with it: it reaches only trusted places and cannot read your home folder.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI02", name: "Tool Misuse and Exploitation", coverage: "partly", how: "Whatever tool an agent runs, runs inside the same fence: the same places, the same folders. Tools that only act on this computer are not restricted beyond the agent's own folders.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI03", name: "Identity and Privilege Abuse", coverage: "partly", how: "An agent starts with none of your credentials: a private user, an empty environment, no access to your keys or browser sessions. Credentials you give it yourself are its to use.", evidence: &["rules.json"] },
    Control { catalogue: OWASP, id: "ASI04", name: "Agentic Supply Chain Vulnerabilities", coverage: "partly", how: "Tool servers and packages an agent installs run under the agent's fence. Package sources must be trusted explicitly, and a project's own list of places is only ever shown to you, never applied by itself. Homebody does not inspect what a package does.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI05", name: "Unexpected Code Execution", coverage: "partly", how: "Code an agent runs unexpectedly runs as the agent: no home folder, no network beyond the gate, no administrator rights. The shared kernel is the limit; the virtual-machine tier is the answer to that.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI06", name: "Memory and Context Poisoning", coverage: "no", how: "What an agent remembers or reads is outside Homebody's reach. Only the consequences are limited, as for ASI01.", evidence: &[] },
    Control { catalogue: OWASP, id: "ASI07", name: "Insecure Inter-Agent Communication", coverage: "partly", how: "Protected apps cannot use each other's gates, and agents cannot reach each other's servers on this computer. Agents that talk through a place both are trusted for are not separated.", evidence: &["rules.json"] },
    Control { catalogue: OWASP, id: "ASI08", name: "Cascading Failures", coverage: "partly", how: "Each app is fenced by itself, sending can be limited per hour, and one switch stops everything. Homebody does not watch for runaway behaviour by itself.", evidence: &["rules.json", "log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI09", name: "Human-Agent Trust Exploitation", coverage: "partly", how: "Questions are written by Homebody, not by the agent, in plain words; no answer means no. A protected app cannot answer its own questions or change its rules.", evidence: &["log.jsonl"] },
    Control { catalogue: OWASP, id: "ASI10", name: "Rogue Agents", coverage: "partly", how: "Refused by default, logged, and stoppable with one switch; attempts to go around the gate are refused by the operating system and shown. Detecting that an agent has gone rogue is left to the person reading the log.", evidence: &["log.jsonl", "activity.csv"] },
];

/// The table as it goes into an export.
pub fn json_table() -> Value {
    json!({
        "format": "homebody-compliance-v1",
        "note": "The project's own mapping of Homebody's evidence to public control catalogues. Not reviewed by an auditor. Coverage is one of yes, partly, no; read \"how\" for where it stops.",
        "controls": CONTROLS.iter().map(|c| json!({ "catalogue": c.catalogue, "id": c.id, "name": c.name, "coverage": c.coverage, "how": c.how, "evidence": c.evidence })).collect::<Vec<_>>(),
    })
}

/// The table as Markdown, for the documentation.
pub fn markdown() -> String {
    let mut m = String::new();
    for cat in [NIST, OWASP] {
        m += &format!("## {cat}\n\n| Control | Coverage | What Homebody does | Evidence in an export |\n|---|---|---|---|\n");
        for c in CONTROLS.iter().filter(|c| c.catalogue == cat) {
            let ev = if c.evidence.is_empty() { "—".to_string() } else { c.evidence.iter().map(|e| format!("`{e}`")).collect::<Vec<_>>().join(", ") };
            m += &format!("| **{}** {} | {} | {} | {} |\n", c.id, c.name, c.coverage, c.how, ev);
        }
        m.push('\n');
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_is_named_once_and_says_how() {
        let mut seen = std::collections::HashSet::new();
        for c in CONTROLS {
            assert!(seen.insert((c.catalogue, c.id)), "{} twice", c.id);
            assert!(["yes", "partly", "no"].contains(&c.coverage), "{}", c.id);
            assert!(c.how.len() > 40 && !c.how.contains('|'), "{}", c.id);
            assert!(c.coverage == "no" || !c.evidence.is_empty(), "{} claims coverage with no evidence", c.id);
        }
        assert_eq!(CONTROLS.iter().filter(|c| c.catalogue == OWASP).count(), 10);
    }

    #[test]
    fn the_documentation_carries_the_same_table() {
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../docs/compliance.md")).expect("docs/compliance.md");
        // A checkout on Windows may have turned the line endings; the words are what is compared.
        assert!(doc.replace("\r\n", "\n").contains(&markdown()), "docs/compliance.md is out of step: regenerate the tables from compliance::markdown()");
    }
}
