# Product spec and MVP scope: Homebody (working name)

Status: draft v0.3, 2026-11-08. Derived from market research that is not part of this repository; section references point there. v0.2 changes: delivered as a fast, lightweight desktop app (macOS first, then Windows and Linux); audience widened from developers to everyday local-AI users; two enforcement modes (Fence for native apps, Box for agents). v0.3 change: the whole product is open source with no paid tiers; pricing removed. The one-line feature list lives in `docs/feature-spec.md`; how those features are surfaced without making the app heavy or complex is in `docs/design-principles.md`.

## 0. Delivery model

A desktop app, not a CLI-first tool. The CLI remains for developers and automation, but the product people install is a menu-bar (macOS) or tray (Windows, Linux) app.

- Shell: Tauri 2 using the OS webview. Installer target under 20 MB, idle memory under 100 MB excluding any running VM.
- Core: one Rust crate for the proxy, policy engine, audit log and VM lifecycle, shared by the app, the CLI and the later self-hosted fleet server.
- Two enforcement modes behind one policy and one log:
  - Fence: per-process egress control for apps running natively on the host (LM Studio, Jan, Ollama, OpenClaw desktop, chat apps, browsers if the user opts in). On macOS this is a Network Extension content filter, the same mechanism Little Snitch uses. No VM, so instant and zero overhead. Fence is what everyday users touch first.
  - Box: a warm Linux VM for agents and anything the user does not trust to run on the host (coding agents, MCP servers, downloaded tools). One VM per machine booted at app launch, sandboxes as containers inside it, suspend-to-disk on quit, ballooned memory when idle.
- Platform order: macOS first (Apple silicon, macOS 14 and later), then Windows 11 via Hyper-V and the Windows Filtering Platform in user mode, then Linux via KVM and nftables. Windows is expected to lag by one or two releases because Hyper-V boots slower and Home-edition support must be verified.
- Signing: Apple notarization, Windows Authenticode via Azure Trusted Signing (no drivers, so this suffices), Linux signed packages and AppImage.

## 1. One-line

A neutral, open-source egress layer for AI agents and local models on a developer's own machine: every agent runs inside a free hypervisor boundary, every outbound byte passes through one hardened policy proxy, and every decision is written to a signed, tamper-evident log the user can verify and export.

Tagline candidate: "Little Snitch for AI agents. See and prove every byte that left your machine."

## 2. Why this and not "a sandbox"

The research found that VM isolation plus a hostname allowlist is now free from Anthropic, OpenAI, Cursor, Docker and Apple. What none of them ship, and what every 2025–2026 exfiltration incident exploited, is:

- Coverage. Vendor sandboxes cover their own agent's shell commands only. MCP servers, web fetch tools, hooks and the local model runner sit outside the boundary.
- Content. Allowlists are per-hostname. Anything reachable on an allowed host (a gist, a package registry, the model provider's own Files API) is an exfiltration channel the sandbox cannot see.
- Evidence. No mainstream tool produces a tamper-evident per-connection log. Enterprises cannot answer "what did the agent send, where?"
- Correctness. Every documented 2026 bypass was a policy or parser bug (null-byte hostnames, empty allowlist treated as allow-all, DNS resolving when connects are blocked), and vendors patched silently.

The product is an evidentiary instrument wrapped around a borrowed hypervisor. Isolation is a commodity we consume, not a moat we build.

## 3. Target user and job to be done

Two primary users, served by one app with two default profiles.

Everyday local-AI user (the long-term mainstream bet): someone who installed LM Studio, Ollama, Jan or a personal agent like OpenClaw because they wanted AI that keeps their documents, photos, health notes and messages on their own machine. They are not technical, they will not edit a policy file, and they cannot tell whether "runs locally" is true. Job: "Make sure my AI stays on my computer, tell me in plain words if anything tries to leave, and show me proof." As a free, open-source app this segment grows through trust and word of mouth.

Developer running agents (the near-term adoption and credibility engine): an individual who runs Claude Code, Codex CLI, Gemini CLI or OpenClaw on their own laptop, often with permission prompts disabled, who has read the home-directory-wipe stories and the SSH-key exfiltration writeups. They already pay for the agent and for two or three prosumer tools. Job: "Let me run agents unsupervised on my real projects, and be certain nothing I did not approve left this laptop, with proof I can show someone."

Secondary users served by the same app with different presets:
- Security or platform engineers at the developer's employer, who arrive when five to ten developers at one company have installed it and want central policy and a fleet audit log. Served by the self-hosted fleet server in v1.0.
- Researchers and red-teamers running untrusted models or agent evals. Design partners and contributors.

## 4. Product principles (non-negotiable)

1. Borrow the VM boundary. Use the platform's free hypervisor (Apple Virtualization.framework or `container` on macOS, KVM-based microVM on Linux, Hyper-V on Windows later). Never depend on `sandbox-exec` or any process-level sandbox as the primary boundary.
2. Default deny, every protocol. The guest has no network interface to the outside. The only path out is a vsock or unix socket to the host proxy. DNS resolves in the proxy, never in the guest.
3. Neutral across agents and models. One policy applies to Claude Code, Codex, Gemini CLI, OpenClaw, MCP servers and the inference daemon. We never privilege one vendor.
4. Evidence over promises. Every allow, deny and ask decision becomes a signed, hash-chained receipt. A `verify` command checks the chain offline.
5. Honest claims. Marketing never says "absolute" or "unbypassable." We publish a residual-channels list and a public bypass test corpus, and every fix for a bypass gets a CVE or advisory and a changelog entry. Silent patching is a firing offense.
6. Fully open source. Engine, app, presets, guest image and fleet server are Apache-2.0 from day one and never relicensed. No paid tiers, no feature gating, no account.
7. Strict mode must be usable. Default-deny breaks package installs, OAuth flows and telemetry. Curated ecosystem presets and an "ask" flow are core product, not polish.
8. Secrets never enter the guest. Credential paths are not mounted. API keys are injected at the proxy boundary as sentinel substitution.
9. Feature-rich but never feature-heavy. All features exist, about eight are visible by default; the rest live in Studio. Speed, size and silence have numeric budgets enforced in CI (see `docs/design-principles.md`).

## 5. Scope

In scope:

- Running CLI coding agents and their MCP servers inside an isolated guest with a project directory mounted.
- Policy-controlled egress with hostname, port, protocol and (Pro) L7 path and content rules.
- Signed per-connection audit log, local export and verification.
- Fencing a host-side local inference daemon (Ollama, llama.cpp, LM Studio server) so it can serve the guest but cannot reach the internet.
- Credential shielding and boundary injection.
- macOS and Linux CLI first. Windows later via Hyper-V.
- Fleet server (self-hosted, open source): policy distribution, fleet audit console, SSO/SCIM, MDM profiles, evidence export.

Also in scope (v0.2 change):

- Fence mode for native desktop AI apps: per-process egress control, plain-language alerts and a zero-egress proof for apps running on the host, without a VM.
- A simple, non-technical default experience: one Private mode switch, a status indicator, and alerts a non-developer can act on.

Out of scope (at least through v1.0):

- Running GUI apps inside the Box VM. Native apps are protected by Fence instead.
- Sandboxing the whole system or replacing a general-purpose firewall. We fence AI apps and agents the user chooses, with an opt-in to fence any app.
- Kernel drivers on any platform.
- Cloud-hosted sandboxes or any hosted service we operate. Everything runs on the user's machine or on a server they host.
- Running the model inside the guest on Apple silicon. Metal is unavailable to Linux guests; the model stays a fenced host daemon.
- A prompt-injection detector or LLM guardrail. We control what leaves; we do not judge prompts.

## 6. Architecture overview

Components, all local except the optional self-hosted fleet server:

1. CLI (`homebody` as working command name). Creates and manages sandboxes, applies policy, launches agents, tails and exports logs. Single static binary, Homebrew cask and DMG on macOS.
2. Guest image. A minimal Linux image we build and sign. Contains the agent runtimes (Node, Python, common toolchains), our guest agent, and our CA certificate baked into the system trust store and the Node, Python, Go, Rust and curl trust paths. The guest has no NIC. It has a vsock channel to the host.
3. Guest agent. Runs in the guest. Reports process metadata (pid, binary, cwd) for each outbound connection to the proxy so receipts carry "which process asked." Enforces that `/etc/resolv.conf` points at the proxy and that no other resolver exists. Provides the in-guest `ask` UI hook.
4. Egress proxy (host). Rust. Speaks HTTP CONNECT, SOCKS5 and a DNS-over-vsock resolver. Applies the compiled policy. Optionally terminates TLS using our CA for hosts marked `inspect`. Performs sentinel credential substitution. Emits receipts.
5. Policy engine. Declarative policy file compiled to a validated, canonical, signed bundle. Presets for ecosystems and agents. Three modes: `strict` (deny unmatched), `ask` (prompt on unmatched, remember answer), `audit` (allow all, log all; onboarding and "what does this agent actually call?" discovery).
6. Audit log. Append-only JSONL of receipts, each hash-chained to the previous and signed with a per-install Ed25519 key. `homebody log verify` checks the chain. `homebody log export` produces a signed bundle with the policy version that governed it.
7. Inference fence (host). Runs the local model daemon as a dedicated OS user with no outbound network permitted, and forwards a single localhost port into the guest over vsock. MVP implementation is a pf (macOS) or nftables (Linux) rule keyed on that user. v0.3 upgrades macOS to a Network Extension content filter for per-process attribution in the same audit log.
8. Fleet server (v1.0, self-hosted). Open-source service that distributes signed policy bundles, collects receipts from fleet installs, provides SSO/SCIM, RBAC, MDM profile generation and compliance evidence export.

Data flow: agent in guest opens a socket, guest networking routes it to the proxy over vsock, proxy resolves the name, matches policy, logs a receipt, and either connects out, inspects and forwards, prompts the user, or denies. Nothing in the guest can reach the host network stack except through that path.

Platform substrates:

| Platform | Guest substrate | Inference fence | Status |
|---|---|---|---|
| macOS 26+ Apple silicon | Apple `container` or our own Virtualization.framework VM with vsock | pf rule on dedicated user; NE content filter later | MVP |
| macOS 14–15 | Our own Virtualization.framework VM | Same | MVP if cheap, else v0.2 |
| Linux x86_64 and arm64 | libkrun or Firecracker microVM over KVM | nftables rule on dedicated user or cgroup; model may also run in-guest with NVIDIA passthrough later | v0.2 |
| Windows 11 | Hyper-V / WHP VM, no driver | WFP user-mode filter or run daemon inside the VM with WSL2 GPU | v1.x |

## 7. Policy model

Policy is a single file, human-readable, version-controlled with the project. Sketch:

```yaml
version: 1
mode: strict            # strict | ask | audit
include:
  - preset:claude-code   # api.anthropic.com, statsig, sentry (telemetry off by default, see below)
  - preset:npm
  - preset:pypi
  - preset:github-git    # github.com:443 git and api, not gists or releases uploads
rules:
  - allow: { host: "api.anthropic.com", port: 443, inspect: true,
             methods: [POST], paths: ["/v1/messages*"] }     # Files API not allowed
  - allow: { host: "registry.npmjs.org", port: 443 }
  - deny:  { host: "*.webhook.site" }                        # explicit, logged with reason
  - deny:  { any: true, reason: "default deny" }
content:
  block_secrets: true     # entropy + pattern scan of inspected bodies: AWS, GitHub, Slack, JWT, PEM
  block_paths: ["~/.ssh", "~/.aws", ".env"]   # refuse bodies containing their contents
secrets:
  ANTHROPIC_API_KEY: { inject_as: header.Authorization, for_host: "api.anthropic.com" }
mounts:
  - { host: ".", guest: "/work", mode: rw }
telemetry: deny          # vendor telemetry hosts in presets are off unless flipped
```

Rules of the model:

- The compiler rejects ambiguous or empty policies. An empty allowlist in `strict` is an error, never allow-all.
- Hostnames are matched after canonicalization: punycode, trailing dot, case, embedded nulls and control characters all rejected at parse time. The parser is fuzzed in CI.
- `inspect: true` requires a trusted CA, so it applies to guest traffic and to fenced host apps that honor the system trust store. The fenced inference daemon is allow or deny only.
- Presets are versioned, signed and published in a public repo with a changelog so users can see what a vendor preset actually opens. A preset diff is shown on upgrade.
- Every decision receipt cites the rule id that produced it.

## 8. Audit receipts

Each receipt contains: timestamp, sandbox id, policy bundle hash, guest process (pid, binary, argv hash, cwd) when the guest agent could attribute it, destination name and resolved IP, port, protocol, SNI, decision (allow, deny, ask-allow, ask-deny), rule id, bytes in and out, duration, inspection result (none, clean, blocked-secret with pattern class), previous receipt hash, signature.

Commands: `homebody log tail`, `homebody log query --deny --since 1h`, `homebody log verify`, `homebody log export --since <date> --format jsonl|csv`. Export includes the public key and the policy bundles referenced so a third party can verify without the install.

## 9. Threat model and published residual channels

Protects against: an agent, MCP server or installed dependency that is prompt-injected, malicious or buggy and attempts to read local secrets and send them anywhere; destructive filesystem actions outside the mounted project; vendor telemetry the user did not consent to; the local model runner phoning home.

Does not protect against, and the docs say so on the front page:

- Exfiltration to an allowed host that is not under inspection, or under inspection with client-side encryption or encoding we do not recognize.
- Low-bandwidth covert channels: timing, request counts, allowed-host URL paths under byte budgets we do not enforce in MVP.
- A compromise of the hypervisor itself. We inherit the substrate's escape risk and track its CVEs.
- A compromised host. The proxy and keys live on the host; a host-level attacker owns the log.
- The content of the conversation sent to the model provider. If the policy allows the model API, the model sees the project. We can log and inspect it; we cannot make it private.
- Writes inside the mounted project directory. Snapshots and rollback are a v0.3 feature.

We maintain a public bypass corpus as integration tests: null-byte and CRLF hostnames, IPv6 literals and mapped addresses, empty and malformed allowlists, DNS resolution with blocked connect, domain fronting with mismatched SNI and Host, redirects to denied hosts, WebSocket upgrades, HTTP/2 and HTTP/3 fallbacks, proxy-unaware tools attempting raw sockets, and guest attempts to edit policy or resolver files. Every published bypass of a competitor is added within a week.

## 10. Non-functional requirements

- Startup: sandbox cold start under 3 seconds on an M-series Mac, warm under 1 second. Agents are interactive; latency kills adoption.
- Proxy overhead: under 2 ms added latency for allow decisions without inspection, under 10 ms with inspection on typical API payloads.
- Inference: host daemon serves the guest at native Metal or CUDA speed. No measurable regression versus running the agent on the host.
- Footprint: guest image under 1 GB, memory configurable, default 4 GB.
- UX: one command to wrap any supported agent, for example `homebody run claude` or `homebody run codex -- --full-auto`. First run offers `audit` mode for ten minutes, then proposes a policy from what it saw.
- Security engineering: parser fuzzing in CI, a published security policy with a 90-day disclosure window, signed releases with reproducible builds for the engine, third-party audit of the proxy before v1.0.
- Privacy of the product itself: the CLI makes no network calls except update checks and preset fetches, both off by default and both logged through the same proxy. We are the first entry in our own audit log.

## 11. MVP definition (v0.1, macOS)

v0.2 amendment: the MVP ships as the desktop app with Fence mode for native AI apps alongside the Box items below. Fence MVP: detect installed local-AI apps (Ollama, LM Studio, Jan, OpenClaw), one-click Private mode that denies their egress except user-approved destinations, plain-language alerts, and a "nothing left" proof view backed by the same signed log. Fence requires the Network Extension system extension, so the app installs to `/Applications` and is distributed directly, not via the Mac App Store. The `homebody model fence` CLI item below becomes the CLI face of Fence.

Ships when all of the following work on macOS Apple silicon (macOS 14 and later):

1. `homebody init` creates a policy from a preset list. `homebody run claude` and `homebody run codex` launch the agent in a guest with the current directory mounted.
2. Guest has no NIC. All egress through the host proxy. DNS in the proxy. Verified by the bypass corpus.
3. Policy modes `strict`, `ask` and `audit`. Presets: claude-code, codex, gemini-cli, npm, pypi, cargo, go, homebrew-in-guest, github-git. Telemetry hosts off by default.
4. Credential shielding: home dotfiles never mounted; `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` injected at the proxy via sentinel substitution.
5. MCP servers configured for the agent run inside the guest and are governed by the same policy. This is the headline coverage difference versus vendor sandboxes.
6. Signed, hash-chained receipts with `log tail`, `log query`, `log verify`, `log export`.
7. Inference fence: `homebody model fence ollama` runs Ollama as a dedicated user with pf denying its egress and forwards port 11434 into the guest. `homebody model verify` proves zero egress over a window by reading the pf log.
8. Public GitHub repo under Apache-2.0, notarized DMG, Homebrew cask, bypass corpus in CI, residual-channels page, security policy.

Explicitly not in v0.1: TLS inspection and content rules (v0.2), Linux (v0.2), filesystem snapshots and rollback (v0.3), Cursor and OpenClaw presets (v0.2), Windows (v1.x), fleet server (v1.0).

MVP success metrics at 90 days after launch:

| Metric | Target |
|---|---|
| Active installs (weekly, opt-in anonymous counter off by default, so use Homebrew analytics and GitHub) | 5,000 |
| Show HN launch | Front page, over 200 points |
| Bypass corpus | 40+ cases, zero known open bypasses older than 7 days |
| Inbound from a security or platform team citing a developer's install | 5 companies |
| Published advisories | Every fixed bypass has one |
| External contributors with merged PRs | 20 |
| Community presets contributed | 10 |

## 12. Roadmap after MVP

- v0.2: TLS inspection with baked CA, content rules and secret blocking, L7 path rules, Linux via libkrun or Firecracker, Cursor CLI, Gemini CLI and OpenClaw presets, policy diff on preset upgrade.
- v0.3: macOS Network Extension content filter for host-process attribution and fencing of LM Studio and Jan, project snapshots and rollback, `ask` flow with remembered answers and OAuth device-flow handling, third-party audit of the proxy.
- v1.0: self-hosted fleet server, signed policy distribution, fleet receipt collection, SSO/SCIM, RBAC, MDM profile generator (Jamf, Kandji) pre-approving the system extension, evidence export mapped to NIST SP 800-53 agent overlays and OWASP Agentic Top 10.
- v1.x: Windows via Hyper-V, NVIDIA in-guest inference on Linux, byte budgets per host as a covert-channel mitigation.

## 13. Open-source model

The product is free and open source in full. There are no tiers, no gated features, no accounts and no hosted service we run.

- License: Apache-2.0 for every component (engine, app, CLI, guest image build, presets, fleet server). Chosen over AGPL because many enterprises block AGPL and the product's value depends on being adopted inside companies. Never relicensed; this is written into the governance doc.
- Governance: a public roadmap, RFC process for policy-format and receipt-format changes, and a maintainers file. Presets are a separate repo so the community can contribute allowlists for new apps without touching the engine.
- Distribution: GitHub releases, Homebrew cask, notarized DMG; later winget and Linux packages. Build and signing pipelines are public so anyone can verify a release matches the source.
- Sustainability (not a priority now): if ever needed, options that do not compromise the open-source promise are GitHub Sponsors, foundation grants, and paid support or audits for organizations. None are planned.
- Trust mechanics that matter more because it is open: reproducible builds, signed releases, a public bypass corpus, and third-party audit of the proxy.

## 14. Launch plan

1. Build in public from the first commit. Weekly notes on bypass-corpus additions and on competitor sandbox issues we reproduce.
2. Pre-launch: five design partners, two of them researchers or red-teamers, one a security engineer at a company already using Claude Code at scale.
3. Launch: Show HN leading with the bypass corpus and the residual-channels page, not with features. Title the post around proof, for example "Show HN: An open-source egress layer for AI agents that logs and signs every connection, with our own bypass tests."
4. Distribution: GitHub, Homebrew cask, notarized DMG, docs site. No login required. No Mac App Store for the full product; the system extension and `/Applications` requirements make direct distribution the only path.
5. Content: a "what does Claude Code actually call?" audit-mode writeup for each major agent, published as preset changelogs. These rank for the searches the beachhead persona makes.

## 15. Key risks and mitigations

| Risk | Mitigation |
|---|---|
| Docker adds TLS inspection and audit to Docker Sandboxes | Ship inspection and signed receipts first; stay neutral and loginless; cover model runners and desktop agents Docker ignores |
| Apple changes Virtualization or `container` behavior | Keep our own Virtualization.framework path as fallback; never depend on `sandbox-exec` |
| Proxy parser bug becomes a public bypass | Fuzzing in CI, bypass corpus, 90-day disclosure policy, public advisory on every fix; treat as a marketing asset |
| Default-deny breaks workflows and users flip to audit mode permanently | Curated presets, ten-minute audit-to-policy onboarding, `ask` mode with memory, telemetry of nothing but we read GitHub issues daily |
| Mac GPU: no Metal in guests | Host inference daemon fenced by user-keyed pf rule, then Network Extension; document the daemon as an egress vector |
| Endpoint Security entitlement delayed or refused | Do not depend on it; content filter provides flow metadata with source audit token for attribution |
| Weak Show HN reception like other local tools | Lead with proof artifacts and credibility, not isolation; partner quotes from researchers |
| Maintainer burnout and unfunded security work on a fully free project | Keep scope tight, automate the bypass corpus, recruit co-maintainers early, keep sponsorship and grants as a fallback |

## 16. Open decisions

1. Guest substrate on macOS: decided in v0.2. Own Virtualization.framework VM via a small Swift helper, supporting macOS 14 and later, one warm VM per machine with containers inside.
1a. Desktop shell: decided in v0.2. Tauri 2 with a shared Rust core. Electron rejected on size and memory.
1b. Platform order: decided in v0.2. macOS, then Windows 11 (Hyper-V, user-mode WFP), then Linux (KVM, nftables).
2. Policy language: custom YAML schema as sketched, or Cedar. Recommendation: YAML schema for v0.1 with a stable compiled form, keep Cedar as an export target for enterprise policy review.
3. Working name and command. Decided: the app is Homebody and the command line is `homebody`.
4. Pricing and tiers: decided in v0.3. None. Fully open source under Apache-2.0.
5. Governance model: single maintainer with public RFCs at first, or a foundation from the start. Recommendation: single maintainer now, revisit at v1.0.
