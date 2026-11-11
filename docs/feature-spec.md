# Feature spec: Homebody (working name)

One line per feature. Tags: **[MVP]** macOS v0.1 · **[v0.2]** · **[v0.3]** · **[v1.0]** · **[v1.x]** · **[Win]** / **[Linux]** platform release. Everything is free and open source; untagged lines ship in the release named by their nearest tag above.

## 1. Everyday experience

- **Private mode switch** — one menu-bar toggle that puts every detected AI app and agent into default-deny; green when nothing has left, amber when something was asked, red when something was blocked. [MVP]
- **Status glance** — menu-bar icon shows live state and the count of bytes that left in the last hour, zero being the goal. [MVP]
- **Plain-language alerts** — "LM Studio wants to send 2 MB to huggingface.co to download a model. Allow once, always, or block?" with no hostnames or ports unless the user expands. [MVP]
- **Auto-detect AI apps** — finds installed local-AI apps and agents (Ollama, LM Studio, Jan, OpenClaw, Claude Code, Codex, Gemini CLI) and offers to protect them in one click. [MVP]
- **Guided first run** — three screens: what we protect, grant the system extension, pick Private mode or Watch mode; under two minutes. [MVP]
- **Watch mode** — allow everything for a chosen period while recording, then show "here is what your AI actually contacted" and propose a policy. [MVP]
- **Proof view** — a single screen that says "Since Tuesday, nothing left your Mac from Ollama and Jan" with a shareable signed proof file. [MVP]
- **Trusted destinations list** — a readable list (model downloads, update servers, your chosen AI provider) the user can edit with toggles, not text. [MVP]
- **Panic button** — one click cuts all AI egress immediately and freezes any running Box. [MVP]
- **Family-friendly explanations** — every destination labeled with what it is for ("model download", "crash reports", "login") from a maintained public catalog. [v0.2]
- **Weekly summary** — a notification or email-free local report: what tried to leave, what was blocked, what changed. [v0.2]
- **Household profiles** — separate policies for different macOS user accounts on the same machine. [v0.3]

## 2. Core protection: Fence (native apps, no VM)

- **Per-process egress control** — allow or deny each outbound connection by app or process, via the macOS Network Extension content filter. [MVP]
- **Default deny for protected apps** — a protected app can reach only trusted destinations; everything else is blocked and logged. [MVP]
- **Ask-and-remember** — unknown destinations prompt once and remember the answer per app. [MVP]
- **Local model daemon fence** — Ollama or llama.cpp keeps serving local apps on localhost while its internet access is denied. [MVP]
- **Telemetry off by default** — vendor analytics, crash reporting and update pings are denied unless the user flips them on, per app. [MVP]
- **Cloud-model guard** — when a local chat app offers cloud models, the hop to the provider is blocked or asked, so "local" stays local. [MVP]
- **Update-only allowance** — approve model and app downloads from verified sources without opening general internet access. [v0.2]
- **Fence any app** — opt in to protect non-AI apps the same way, Little Snitch style. [v0.2]
- **Browser AI mode** — fence a chosen browser profile used for AI web apps, with a destination catalog for the big providers. [v0.3]
- **Windows Fence** — same features via a user-mode Windows Filtering Platform filter, no kernel driver. [Win]
- **Linux Fence** — same features via nftables rules keyed on user or cgroup, with an optional eBPF attribution helper. [Linux]

## 3. Core protection: Box (agents, VM)

- **Warm VM** — one lightweight Linux VM boots at app launch via Virtualization.framework, suspends to disk on quit, resumes in about a second. [MVP]
- **Instant sandboxes** — each agent run is a container inside the warm VM, starting in milliseconds. [MVP]
- **No network interface** — the VM has no NIC; the only route out is a vsock channel to the host proxy. [MVP]
- **DNS in the proxy** — the guest cannot resolve names itself, so DNS tunneling and resolver side doors are closed by construction. [MVP]
- **Project-only mounts** — only the chosen folder is visible inside; home dotfiles, keychains and credential paths are never mounted. [MVP]
- **Run any agent** — `Run in Box` for Claude Code, Codex CLI and Gemini CLI from the app or `sai run <agent>` from the CLI. [MVP]
- **MCP servers inside the Box** — the agent's MCP servers run under the same policy, closing the gap every vendor sandbox leaves. [MVP]
- **Credential injection at the boundary** — API keys never enter the guest; the proxy swaps sentinels for real keys on approved hosts only. [MVP]
- **Memory ballooning** — the VM returns idle RAM to the host; default ceiling 4 GB, configurable. [MVP]
- **Snapshot and rollback** — snapshot the project before a run and roll back destructive changes in one click. [v0.3]
- **Per-sandbox VMs** — a dedicated VM per sandbox for stronger separation between projects. [v0.3]
- **OpenClaw and Cursor CLI presets** — run personal agents and Cursor's CLI agent in the Box. [v0.2]
- **GPU inside the Box on Linux** — NVIDIA passthrough so the model can live inside the boundary. [Linux] [v1.x]
- **Windows Box** — Hyper-V VM with Hyper-V sockets as the channel. [Win]

## 4. Policy

- **Presets** — curated, versioned, signed allowlists for agents (Claude Code, Codex, Gemini CLI, OpenClaw) and ecosystems (npm, PyPI, cargo, Go, Homebrew, GitHub git). [MVP]
- **Preset transparency** — every preset shows in plain words what it opens and why, with a public changelog and a diff on upgrade. [MVP]
- **Three modes** — Strict (deny unknown), Ask (prompt unknown), Watch (allow and record). [MVP]
- **Readable policy file** — one YAML file per project or app, version-controllable, editable in the app or a text editor. [MVP]
- **Fail-closed compiler** — empty or ambiguous policies are errors, never allow-all; hostnames canonicalized and fuzzed at parse time. [MVP]
- **Rule attribution** — every decision names the rule that produced it. [MVP]
- **Path and method rules** — allow only specific API paths on an allowed host, for example messages but not file uploads. [v0.2]
- **Content rules** — block outbound bodies that contain secrets, key files or chosen document contents. [v0.2]
- **Byte budgets** — cap how much data may leave to a host per hour as a covert-channel brake. [v1.x]
- **Cedar export** — export policy in a formal language for enterprise review. [v1.0]

## 5. Inspection (Pro)

- **TLS inspection in the Box** — our CA is baked into the guest image, so allowed-host traffic can be inspected without breaking tools. [v0.2]
- **Secret blocking** — pattern and entropy scan for cloud keys, tokens, private keys and JWTs in outbound bodies. [v0.2]
- **Document fingerprinting** — mark folders as private; outbound bodies containing their content are blocked. [v0.3]
- **Inspection for native apps** — optional TLS inspection for fenced apps that honor the system trust store. [v0.3]
- **Honest limits page** — the app shows what inspection cannot see (client-side encryption, timing) rather than implying totality. [v0.2]

## 6. Audit and proof

- **Signed receipts** — every allow, deny and ask becomes a hash-chained, Ed25519-signed record with process, destination, bytes and rule. [MVP]
- **Live feed** — scrolling connection view with filters by app, decision and time. [MVP]
- **Verify** — one click or `sai log verify` checks the chain for tampering. [MVP]
- **Export** — signed JSONL or CSV bundle with the governing policy and public key. [MVP]
- **Zero-egress certificate** — a signed statement that a named app sent nothing over a period, suitable to share. [MVP]
- **Local-only logs** — logs never leave the machine unless the user exports them or joins a Team. [MVP]
- **Countersigned evidence** — optional RFC 3161 timestamp from a public timestamping authority for stronger provenance. [v0.3]
- **Compliance mapping** — exports mapped to NIST SP 800-53 agent overlays and OWASP Agentic Top 10 controls. [v1.0]

## 7. Trust and security engineering

- **Open-source engine** — proxy, policy, log and VM lifecycle under Apache-2.0, never relicensed. [MVP]
- **Public bypass corpus** — 40+ bypass tests (null-byte hostnames, empty allowlists, DNS side doors, domain fronting, redirects, WebSockets) run in CI and published. [MVP]
- **Advisory on every fix** — each fixed bypass gets a CVE or advisory and a changelog line; no silent patches. [MVP]
- **Reproducible builds** — engine binaries reproducible from source; app signed and notarized. [v0.2]
- **Disclosure policy** — published security.txt and 90-day coordinated disclosure. [MVP]
- **Self-logging** — the app's own update checks and preset fetches pass through the same proxy and appear in the same log, off by default. [MVP]
- **Third-party audit** — independent proxy audit published before v1.0. [v1.0]

## 8. Developer and automation

- **CLI** — `sai` for init, run, policy, log and model commands, sharing the app's core. [MVP]
- **Agent launchers** — `sai run claude`, `sai run codex`, `sai run gemini` with flags passed through. [MVP]
- **Per-project policy** — policy file checked into the repo, auto-applied when the project is opened. [MVP]
- **Local API** — localhost socket for IDEs and scripts to start sandboxes and query the log. [v0.2]
- **CI mode** — headless Box for running agents in pipelines with the same policy and receipts. [v0.3]
- **Editor integration** — VS Code and JetBrains status indicator and policy editor. [v0.3]

## 9. Fleet (self-hosted, v1.0)

- **Policy distribution** — signed policies pushed to every install from a self-hosted fleet server.
- **Fleet audit console** — receipts from all installs, searchable, with retention controls.
- **SSO and SCIM** — identity-provider login and automatic provisioning.
- **Role-based access** — who may edit policy, who may view logs, who may export.
- **MDM profiles** — generated Jamf and Kandji profiles that pre-approve the system extension and content filter.
- **Evidence export** — audit-ready bundles per period and per team.

## 10. App platform and quality

- **Lightweight shell** — Tauri 2 on the system webview; installer under 20 MB, idle memory under 100 MB without a VM. [MVP]
- **Fast start** — app ready in under one second; first Box ready in under three seconds cold, under one second warm. [MVP]
- **Low overhead** — under 2 ms added latency for allow decisions, under 10 ms with inspection. [MVP]
- **Offline first** — everything works with no internet; nothing requires an account. [MVP]
- **No login** — no sign-up or account, ever. [MVP]
- **Auto-update** — signed delta updates, off by default, logged when on. [v0.2]
- **Accessibility and localization** — VoiceOver support and translations for the top ten languages, since the mainstream user is not English-first. [v0.3]
- **Windows release** — Fence and Box on Windows 11, Hyper-V and user-mode WFP, Authenticode-signed. [Win]
- **Linux release** — Fence and Box on major distributions via KVM and nftables, signed packages and AppImage. [Linux]
