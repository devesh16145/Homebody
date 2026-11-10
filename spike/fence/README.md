# Fence spike (macOS)

Goal of the spike, from `docs/product-spec.md`: prove that a local model daemon (Ollama) can keep serving apps on this Mac while its own internet access is denied, with every attempt recorded as a signed, verifiable receipt.

Status on 2026-11-08, built and tested on macOS 26.5.1 with Command Line Tools only (no Xcode, no Apple Developer identity):

| Piece | Where | State |
|---|---|---|
| Signed, hash-chained receipt log and CLI | `fencelog/` | Built and tested end to end. Tamper detection confirmed. |
| Fence v0: pf rules + dedicated user + receipt watcher | `pf/` | **Run live on 2026-11-09. Works.** Internet and DNS denied for the fenced Ollama, localhost served, Metal GPU available to it, 37 signed receipts, chain verified, proof statement "Nothing left this machine from ollama." See section 2a. |
| Fence v1: Network Extension content filter + menu-bar app | `ne/` | Compiles, bundles and ad-hoc signs (512 KB). Host app UI runs. The extension cannot load until signed with an Apple Developer identity. |

## 1. fencelog

A single Swift binary (Foundation + CryptoKit, no dependencies) that owns the receipt format used by both fences.

```bash
cd spike/fence/fencelog && swift build -c release
```

```bash
.build/release/fencelog init
```

Each receipt is a JSON line: sequence, timestamp, process, destination, protocol, decision, rule, the previous receipt's hash, its own SHA-256 and an Ed25519 signature. `verify` recomputes everything; `proof --process ollama` prints a zero-egress statement; `export` writes a bundle with the public key so anyone can verify offline. Receipts live in `~/Library/Application Support/SandboxAI/` (override with `FENCELOG_DIR`).

Tested: init, append, ingest of simulated `tcpdump -i pflog0` lines, tail, verify, proof, export, and a byte flipped in receipt 2 made `verify` fail with "body hash does not match".

Known rough edge: a retransmitted TCP SYN produces a second receipt for the same connection.

## 2. Fence v0 (pf) — run this today

Ollama runs as a hidden dedicated user. pf drops everything that user sends except loopback and an explicit allow table, and logs each decision to `pflog0`, which `fencelog` turns into receipts. No entitlements, no kernel code, nothing persistent across reboot.

Three terminals:

```bash
sudo ./spike/fence/pf/fence.sh install
```

```bash
sudo ./spike/fence/pf/fence.sh run
```

```bash
sudo ./spike/fence/pf/fence.sh watch
```

Then:

```bash
sudo ./spike/fence/pf/fence.sh test
```

`test` checks four things: the host reaches Ollama on `127.0.0.1:11434`, the fenced user cannot reach `example.com`, the fenced user cannot resolve DNS, and the blocked attempts appear as receipts with a valid chain. It ends by printing the proof statement.

Model downloads are egress, so they are blocked. To pull a model:

```bash
sudo ./spike/fence/pf/fence.sh allow registry.ollama.ai ollama.com
```

then `ollama pull llama3.2` from a normal terminal, then `sudo ./fence.sh revoke`. This is the "ask" flow at shell fidelity. Allowances are IP-based and expire on revoke or reboot.

`uninstall` restores the stock `/etc/pf.conf` ruleset. `purge` also removes the user and its models.

`demo` does run, watch and test under a single sudo call and keeps serving until Ctrl-C. Prefer it: sudo caches credentials per terminal tab, so the three-terminal flow asks for the password three times.

### 2a. Live result, 2026-11-09, macOS 26.5.1, Apple M4

`sudo ./fence.sh install` created `_saiollama` (uid 590) and loaded five rules. `sudo ./fence.sh demo` then produced:

| Check | Result | Note |
|---|---|---|
| Host reaches Ollama on 127.0.0.1:11434 | Pass (after a fix) | The first run failed because the test fired 3 s after start while Ollama spent ~11 s timing out on calls to ollama.com and discovering the GPU. `demo` now waits for the port. The endpoint answered `{"version":"0.40.15"}` once up. |
| Fenced user reaching example.com | Pass, blocked | IPv4 and IPv6 attempts both dropped and logged. |
| Fenced user resolving DNS at 1.1.1.1 | Pass, blocked | Logged as a denied UDP/53 receipt. |
| Receipts logged and chain valid | Pass | 23 receipts at test time, 37 two minutes later, all verified. |

Findings worth carrying into the product:

- Ollama 0.40 phones home on startup even with no models installed: `https://ollama.com/api/tags` for a "cloud cache" and `/api/experimental/model-recommendations`, retried roughly every second while failing, then backed off to five minutes. All attempts were denied. This is exactly the traffic an everyday user cannot see today, and a strong default for the Feed's first event.
- The fenced daemon still saw the Apple M4 GPU through Metal (`library=Metal ... total="11.8 GiB"`). Fencing the host daemon costs no inference performance, which validates the Mac architecture in the product spec.
- The receipt watcher attributed 12 multicast packets logged by Apple's own pf anchors to Ollama and mis-parsed their addresses, because pflog carries every logged rule on the machine. Fixed: `ingest-pflog` now accepts only lines from our anchor (`--anchor fence`). The 12 bad receipts remain in this machine's log, since the log is append-only; delete `~/Library/Application Support/SandboxAI/receipts.jsonl` to start clean.
- Each denied connection attempt yields two receipts (two sockets, or SYN plus retransmit). Harmless for the spike; the product should coalesce by 5-tuple within a window.

What v0 cannot do: attribute connections to a process other than by uid, see hostnames (pf sees IPs; DNS is blocked so the daemon never resolves), or inspect content. It proves the enforcement and evidence model, not the product UX.

## 3. Fence v1 (Network Extension) — ready for a Developer identity

The product-grade fence: a content-filter system extension sees every outbound flow with its process audit token, consults `policy.json`, returns allow or drop, and writes receipts into the shared app-group container. A menu-bar app installs the extension, toggles Private mode, and shows activity.

```bash
cd spike/fence/ne && ./build.sh check
```

```bash
cd spike/fence/ne && ./build.sh
```

Plain `swiftc`, no Xcode project. Output is `build/Fence.app` with the extension embedded at `Contents/Library/SystemExtensions/`. An ad-hoc build also produces `build/FenceUI.app`, an app-only variant without the restricted entitlement, which is the only ad-hoc variant macOS will launch; use it to test the menu-bar UI.

Verified here: both targets type-check and link; the bundle layout, Info.plists and entitlements are correct (`codesign -d --entitlements`); `FenceUI.app` launches from the terminal and from the launcher, creates the app-group container and writes a default `policy.json` into it.

Group-container note: the shared-paths code probes the group container for writability and falls back to `~/Library/Application Support/SandboxAI-Fence/` if macOS refuses the write, because non-Team-ID-prefixed group ids from unsigned apps are not guaranteed access on recent macOS. With a real `TEAM_ID` the group id becomes `<TEAMID>.com.sandboxai.fence` and the probe is a no-op.

Not verified here, and impossible on this Mac today: loading the extension. The content-filter entitlement must be backed by a provisioning profile from a paid Apple Developer account, and the extension must be signed with a certificate from that account. An ad-hoc signed app carrying that entitlement is refused by launchd (observed: "Launchd job spawn failed").

To make it loadable:

1. Join the Apple Developer Program. In Certificates, Identifiers & Profiles create two App IDs, `com.sandboxai.fence` and `com.sandboxai.fence.extension`, each with the Network Extensions capability (content filter) and an App Group `<TEAMID>.com.sandboxai.fence`.
2. Create a Developer ID Application certificate (for direct distribution) or a Mac Development certificate (for this machine only), and matching provisioning profiles for both App IDs. Save them as `ne/Resources/app.provisionprofile` and `ne/Resources/extension.provisionprofile`.
3. Build: `TEAM_ID=<TEAMID> SIGN_ID="Developer ID Application: <Name> (<TEAMID>)" ./build.sh`
4. Copy `build/Fence.app` to `/Applications` (macOS 26 refuses system extensions from anywhere else), launch it, choose Install Fence extension, approve it in System Settings → General → Login Items & Extensions → Network Extensions, then Turn Private mode on.
5. Run Ollama normally. Its outbound attempts appear in Show recent activity and in `~/Library/Group Containers/<TEAMID>.com.sandboxai.fence/Receipts/`.

Policy is `policy.json` in the group container: `mode` (strict or watch), `fencedProcesses` (substrings matched against the executable name or signing identifier), `allowHosts` (exact or `*.example.com`), `allowLocalhost`. The extension reloads it when the file changes.

## 3b. Agent fence, no VM (built and tested 2026-11-09)

`agent/agent-fence.sh` runs Claude Code on the Mac itself inside a fence, with no hypervisor:

- A hidden macOS user `_saiagent` (uid 591) with its own home at `/var/sai/agent`. Your home folder is private to your account, so the agent cannot read your SSH keys, cloud credentials or documents.
- One shared workspace, `/Users/Shared/sandbox-ai/work`, owned by the agent and writable by your account too. It is the only place the agent writes.
- pf anchor `com.sandboxai/agent`: the agent user may talk to loopback only; everything else is dropped and logged.
- On loopback, the Box spike's proxy in TCP mode (`sai-box proxy --label agent`) applies the hostname allowlist, resolves DNS, and writes signed receipts. Default allowlist: `api.anthropic.com claude.ai console.anthropic.com platform.claude.com registry.npmjs.org`.
- The Sandbox-ai Console (`spike/box/ui/server.py`, default agent mode) drives it: status, trusted list, live feed, a Claude Code runner (`claude -p` as the fenced user, key kept in memory), an exfiltration button and chain verify. The console needs `sudo ./agent-fence.sh allow-console`, which writes `/etc/sudoers.d/sandbox-ai-agent` allowing your account to run commands as `_saiagent` without a password. Remove it with `uninstall`.

Live result: `sudo ./agent-fence.sh test` passed 4/4 (direct internet blocked, DNS blocked, webhook.site refused with 403 via proxy, api.anthropic.com reachable via proxy); the console's exfiltration button produced a 403 and a `Blocked webhook.site:443 default-deny` receipt. Claude Code and Node run from the host's Homebrew install; nothing is installed for the fenced user.

What it does not give you compared with the Box: a shared kernel, so a kernel exploit is not contained; the agent can read world-readable files and see other processes; destructive commands are limited by Unix permissions, not a wall. For everyday users and most developers this is the right first tier, and it is the one that runs reliably on this Mac tonight.

Known cosmetic issue: the Ollama fence's receipt watcher, if still running from before the anchor filter was added, labels the agent's pf denials "ollama". Restart it (`fence.sh watch`) to pick up the `--anchor` filter.

## 4. What the spike has shown so far

- The receipt format and verification work and are small enough to live in both the extension and the CLI.
- A dedicated-user pf fence is viable with zero entitlements and is the right fallback for users who will not approve a system extension.
- The Network Extension path compiles without Xcode, which keeps the build reproducible and scriptable for CI.
- The real gate for the product-grade fence is the Developer account, not code.

## 5. Next steps

1. Done: pf fence run live, results in 2a. Still to try: `fence.sh allow registry.ollama.ai ollama.com`, pull a small model, revoke, and run a prompt to confirm inference under the fence.
2. Get the Developer identity, sign, load the extension, and confirm per-process attribution for an unsigned Homebrew binary (the `proc_pidpath` path) and for a signed app (the signing-identifier path).
3. Measure decision latency in `handleNewFlow` and the extension's memory footprint against the budgets in `docs/design-principles.md`.
4. Start the Box spike: Virtualization.framework VM, vsock to a host proxy, Claude Code inside.
