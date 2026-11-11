# Compliance mapping

Organisations that have to show how they control AI tools usually do it against a catalogue of controls. This page says, control by control, what Homebody does about it and which file in an export shows it.

Read this first:

- **This is the project's own mapping.** It was made in good faith from the text of the catalogues. No auditor has reviewed it, and it is not a certification of anything.
- **"partly" and "no" mean what they say.** Each row says where Homebody stops. Homebody covers the apps it protects on the computers it is installed on, and nothing else.
- **The evidence is in the export.** *Report → Export for an auditor* (or `homebody export-log`) makes a folder. The last column names the files in it. The same table is in that folder as `compliance.json`.

## Catalogues

- **NIST SP 800-53 Rev. 5**, base controls. NIST is writing overlays of these controls for AI agents (the COSAiS project, single-agent and multi-agent use cases). They were not published when this page was written; the mapping will follow them when they are.
- **OWASP Top 10 for Agentic Applications (2026)**, ASI01 to ASI10.

## What is in an export

| File | What it is |
|---|---|
| `log.jsonl` | Every decision, exactly as written: hash-chained, each record signed. Check it anywhere with `homebody verify log.jsonl`. |
| `activity.csv` | The same events as a table for a spreadsheet. Convenient; not signed. |
| `rules.json` | The rules in force when the export was made. |
| `policy/` | The same rules as Cedar policies, a formal language reviewers can check with their own tools. |
| `compliance.json` | This mapping, for programs. |
| `timestamps/` | If asked for: statements by a public time-stamping service that the log existed at a given time. |

## NIST SP 800-53 Rev. 5

| Control | Coverage | What Homebody does | Evidence in an export |
|---|---|---|---|
| **AC-3** Access Enforcement | yes | Each protected app runs as a private user of its own with no access to your home folder; it can write only in its workspace. | `rules.json` |
| **AC-4** Information Flow Enforcement | yes | A protected app can send only to places that are trusted for it. The decision is made at its gate and enforced again by the operating system, so going around the gate is refused too. | `rules.json`, `policy/policy.cedar`, `log.jsonl` |
| **AC-4(4)** Flow Control of Encrypted Information | partly | With Look inside on, encrypted connections to trusted places are opened at the gate, checked and re-encrypted; a connection that cannot be checked is cut. Off by default, and not available for apps that take their trusted certificates from the system. | `log.jsonl`, `activity.csv` |
| **AC-6** Least Privilege | yes | Protected apps run without administrator rights, without your environment (keys, tokens), and on Linux without the ability to gain privileges. | `rules.json` |
| **AU-2** Event Logging | yes | Every connection decision, every question and answer, and every change to the rules, the mode or the setup is logged. | `log.jsonl` |
| **AU-3** Content of Audit Records | yes | Each record says which app, which place and port, what was decided and why, how much was sent and received, and when. | `log.jsonl`, `activity.csv` |
| **AU-8** Time Stamps | yes | Records carry UTC times to the millisecond. An independent time-stamping service can vouch for when the log existed. | `log.jsonl`, `timestamps/` |
| **AU-9** Protection of Audit Information | yes | Records are chained by hash and each is signed (Ed25519), so a changed, removed or reordered record is detected. Protected apps cannot reach the log or its key. | `log.jsonl` |
| **AU-9(3)** Cryptographic Protection | yes | Hash chain and per-record signatures; anyone can check them offline with `homebody verify`. | `log.jsonl` |
| **AU-10** Non-repudiation | partly | Records are signed with a key that never leaves the computer, and an outside time stamp fixes when they existed. The key belongs to the computer's user, not to a hardware device. | `log.jsonl`, `timestamps/` |
| **AU-11** Audit Record Retention | partly | The log is kept on the computer until its owner removes it. A fleet server keeps what devices send for a period its administrator sets. | `log.jsonl` |
| **AU-12** Audit Record Generation | yes | Records are written by the engine at the moment of each decision, for every protected app. | `log.jsonl` |
| **CM-7** Least Functionality | yes | Usage statistics, crash reports and update checks of protected apps are switched off unless someone switches them on, per app. | `rules.json`, `policy/policy.cedar` |
| **IR-4** Incident Handling | partly | One switch blocks everything for every protected app and cuts open connections. What happened before is in the log. Homebody does not run an incident process for you. | `log.jsonl` |
| **SC-7** Boundary Protection | yes | Each protected app has one managed way out, its gate. Name lookups go through the gate too. | `rules.json`, `log.jsonl` |
| **SC-7(5)** Deny by Default — Allow by Exception | yes | A place with no rule is refused (after asking, when asking is on). Empty rules never mean allow. | `policy/policy.cedar`, `log.jsonl` |
| **SC-7(8)** Route Traffic to Authenticated Proxy Servers | partly | All of a protected app's traffic goes through its own gate, which knows the app by the private user it runs as. The gate does not ask the app to sign in. | `rules.json` |
| **SC-7(10)** Prevent Exfiltration | partly | Sending is limited to trusted places, optionally to an hourly amount, and with Look inside on to requests that carry no recognised secret and no lines from folders marked private. Data hidden inside an allowed request in a form Homebody does not recognise is not caught. | `log.jsonl`, `rules.json` |
| **SI-4** System Monitoring | yes | Every connection of a protected app is seen and shown live, including attempts the operating system refused. | `log.jsonl`, `activity.csv` |
| **SI-4(4)** Inbound and Outbound Communications Traffic | partly | Outbound traffic of protected apps is monitored. Inbound connections from other computers to a protected app are refused, not analysed. | `log.jsonl` |
| **SI-4(18)** Analyze Traffic and Covert Exfiltration | partly | Hourly send limits act as a brake, and requests are scanned for secrets and private text while Look inside is on. Timing channels and client-side encryption are not seen; the Report screen says so. | `log.jsonl` |
| **SI-7** Software, Firmware, and Information Integrity | partly | The engine can be rebuilt from source to the same file (scripts/repro-check.sh), and the log detects its own alteration. The desktop app's signature depends on who distributes it. | `log.jsonl` |

## OWASP Top 10 for Agentic Applications 2026

| Control | Coverage | What Homebody does | Evidence in an export |
|---|---|---|---|
| **ASI01** Agent Goal Hijack | partly | Homebody does not stop an agent being talked into something. It limits what a hijacked agent can do with it: it reaches only trusted places and cannot read your home folder. | `rules.json`, `log.jsonl` |
| **ASI02** Tool Misuse and Exploitation | partly | Whatever tool an agent runs, runs inside the same fence: the same places, the same folders. Tools that only act on this computer are not restricted beyond the agent's own folders. | `rules.json`, `log.jsonl` |
| **ASI03** Identity and Privilege Abuse | partly | An agent starts with none of your credentials: a private user, an empty environment, no access to your keys or browser sessions. Credentials you give it yourself are its to use. | `rules.json` |
| **ASI04** Agentic Supply Chain Vulnerabilities | partly | Tool servers and packages an agent installs run under the agent's fence. Package sources must be trusted explicitly, and a project's own list of places is only ever shown to you, never applied by itself. Homebody does not inspect what a package does. | `rules.json`, `log.jsonl` |
| **ASI05** Unexpected Code Execution | partly | Code an agent runs unexpectedly runs as the agent: no home folder, no network beyond the gate, no administrator rights. The shared kernel is the limit; the virtual-machine tier is the answer to that. | `rules.json`, `log.jsonl` |
| **ASI06** Memory and Context Poisoning | no | What an agent remembers or reads is outside Homebody's reach. Only the consequences are limited, as for ASI01. | — |
| **ASI07** Insecure Inter-Agent Communication | partly | Protected apps cannot use each other's gates, and agents cannot reach each other's servers on this computer. Agents that talk through a place both are trusted for are not separated. | `rules.json` |
| **ASI08** Cascading Failures | partly | Each app is fenced by itself, sending can be limited per hour, and one switch stops everything. Homebody does not watch for runaway behaviour by itself. | `rules.json`, `log.jsonl` |
| **ASI09** Human-Agent Trust Exploitation | partly | Questions are written by Homebody, not by the agent, in plain words; no answer means no. A protected app cannot answer its own questions or change its rules. | `log.jsonl` |
| **ASI10** Rogue Agents | partly | Refused by default, logged, and stoppable with one switch; attempts to go around the gate are refused by the operating system and shown. Detecting that an agent has gone rogue is left to the person reading the log. | `log.jsonl`, `activity.csv` |

## Keeping this page honest

The tables above are generated from `app/crates/core/src/compliance.rs`:

```bash
cargo run -p homebody-core --example compliance_md
```

A test fails when this page and that file disagree, so the page cannot drift from what the app exports.
