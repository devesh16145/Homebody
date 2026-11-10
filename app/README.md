# Sandbox-ai (working name)

Keep your AI on your computer, and prove it.

A menu-bar app for macOS (Windows and Linux to follow) that runs local AI apps and agents so they can reach only the places you trust, asks you in plain words about anything else, and keeps a signed record of every decision. No virtual machine. Apache-2.0, no accounts, no telemetry, nothing hosted.

Status: second build, 10 Nov 2026 ("usable every day"). Everything below runs end to end on this Mac in the release app: all three fence layers installed and verified live, Ollama protected with GPU access, a model download and inference under the fence. New in this build and tested live: bringing a project folder in for an agent and applying its changes back, attempts that go around the gate shown in the Feed, Watch with a timer and a review of what was contacted, export of the whole signed log, and a clean stop when the app is killed. Not yet verified: that protection is restored by the watchdog after a restart, and the folder chooser button in the real app window (the command behind it is tested; the button was only exercised in the browser preview).

## What works today

| Area | State |
|---|---|
| The gate: one local proxy per protected app, name rules, port rules, DNS resolved by the gate, refusal of names that point at this computer or the local network | Working, 29 bypass cases pass |
| TLS greeting check: the name an app greets the server with must be the name that was approved | Working |
| Ask flow: Allow once (5 min), Always allow, Block, 30 s timeout that blocks, one question per destination, companion destinations so one action is one question | Working |
| Modes: Private, Watch, Off, and the "Block everything" panic control that also cuts open connections | Working |
| Watch: allows everything for a set time (an hour by default), returns to Private by itself, then shows the places contacted that have no rule yet so you can trust the ones you want | Working, tested live with a one-minute timer |
| Projects: bring a folder in for an agent. It works on a copy inside its workspace (instant, copy-on-write); you review added, changed and deleted files and apply them to your own folder. Files you changed in the meantime are left alone unless you say otherwise. `.git` is never touched | Working, 12 unit tests and a live run as the protected user |
| Around the gate: a protected program that tries to connect directly, look up a name itself, or listen for other computers is refused by the fence, and the refusal appears in the Feed with the program's name | Working, read from the system log; needs an administrator account |
| Ledger: append-only, hash-chained, Ed25519-signed; verify; signed proof export; export of the whole log as one file that `sai verify <file>` checks on any computer | Working, tamper and cut-file cases tested |
| Fence, layer 1: each protected app runs as its own macOS user, in a group of its own | Working: your home folder is closed to them by file permissions alone |
| Fence, layer 2: seatbelt profile at launch (no network except the app's gate, no name lookups, nothing readable under your home) | Working, 17 bypass cases pass as the protected user |
| Fence, layer 3: packet filter rules per user, restored at boot by a root watchdog | Working, verified with the seatbelt layer removed: internet blocked, only the app's own gate reachable. Not yet verified across a restart |
| Health probes every 30 s; nothing protected starts unless every layer checks out | Working |
| App: menu-bar icon with six states, Glance popover, prompt banner, main window (Feed, Proof, Studio), first-run setup screen | Working |
| CLI `sai`: status, feed, watch, mode, watched, asks, allow/block, trust, proof, verify, export-log, project, run, terminal, open, daemon | Working |
| Protected apps | Claude Code (agent), Ollama (local models) |

## Run it

Open the app:

```bash
open "target/release/bundle/macos/Sandbox-ai.app"
```

A shield appears in the menu bar and the main window opens on the setup screen. Click **Turn on protection** and approve with your Mac password once. Setup creates a private space for each protected app, loads the packet filter rules, installs the watchdog that restores them after restarts, and allows the app to start things in those spaces. Studio → Advanced can remove all of it again.

Then:

- Studio → Apps → Claude Code → **Open protected terminal** runs Claude Code fenced. Its files live in `/Users/Shared/SandboxAI/claude-code`.
- Studio → Apps → Claude Code → **Bring in a project…** copies one of your folders into that workspace. **Open terminal** on the project starts Claude Code there; **Review** shows what it changed; **Apply** puts the changes into your own folder.
- Studio → Apps → Ollama → **Start** runs the model server fenced on `127.0.0.1:11434`, where your other apps find it as usual. Quit any Ollama you already have running first.
- The first model download or unknown destination shows a prompt at the top right of the screen.

Command line, same core and same log:

```bash
./target/release/sai status
```

```bash
./target/release/sai run claude-code -- claude
```

Build from source: `cargo build` for the core and CLI, `npx tauri build` for the app (needs Rust, Node, and the Xcode Command Line Tools; full Xcode is not required).

## How it is built

```
app/
  crates/core/     sai-core: everything that is not pixels
    hostname.rs      what a valid destination name is; how names match rules
    catalog.rs       plain-language labels for destinations; app presets
    config.rs        settings: mode, protected apps, trusted destinations
    ledger.rs        hash-chained signed log, verification
    feed.rs          ledger records folded into Feed rows (same code live and at startup)
    proxy.rs         the gate: request parsing, TLS greeting check, relay with byte counts
    engine.rs        decisions, ask flow, status, proof, the command surface
    fence.rs         OS users, seatbelt profile, packet filter rules, setup script, launchers
    projects.rs      bring a folder in for an agent, compare, apply back
    control.rs       private control socket for the CLI
  crates/cli/      sai
  src-tauri/       thin shell: windows, menu-bar icon, one command, one event channel
  ui/              plain HTML, CSS and ES modules; no framework, no build step
  docs/ipc.md      the contract between core and UI
  tests/bypass-corpus.sh   every known way around the gate or fence, as executable checks
```

The UI calls one command, `sai`, with `{cmd, args}`; the core's `dispatch` serves the app, the CLI and scripts alike. Opened in a plain browser, the UI runs against an in-page mock of the same contract (`ui/dev.html` is a review harness with scenarios).

Sizes in this build: 58 unit tests, 46 bypass cases, a 4.2 MB app.

## Verify it yourself

```bash
cargo test -p sai-core
```

```bash
tests/bypass-corpus.sh
```

The corpus needs the app (or `sai daemon`) running. The gate half runs anywhere; the fence half runs commands as the protected user and needs setup done.

## What it does not do yet, stated plainly

- **Agents work on a copy of your project, not on the folder itself.** This is deliberate: nothing an agent does can damage your work until you apply it. The costs: changes are not live in your editor until applied, the agent's own commits stay in the copy (its changes arrive in your folder as ordinary edits), and files you edit after bringing a project in do not reach the copy. Sharing a folder in place is possible but was left out for now, because granting an agent a path into your home also exposes neighbouring files to it by file permissions alone.
- **A link an agent creates is applied as a link.** The review shows where every new link points; read that line before applying.
- **It cannot see inside encrypted traffic to a trusted destination.** Observed on this Mac: Claude Code, not signed in, sent 219 KB to `api.anthropic.com` in one connection at startup. The Feed shows the size and destination; what was in it is not visible without inspection, which is a later feature.
- **Attempts that go around the gate are shown without the address.** macOS reports the program and the port, not where it was trying to go. Attempts to read your files are stopped by ordinary file permissions before the fence sees them, so those are not shown at all. The Network Extension version will show both.
- **A trusted name can still front for another site on the same hosting provider** inside encrypted traffic. The greeting check stops the simple version; the full fix is inspection.
- **Shared kernel.** A kernel exploit from inside a protected app is not contained. The virtual-machine tier (see `../spike/box`) is the answer and is planned.
- **The setup script runs from a file in your account** for the moments between writing it and approving it. A signed privileged helper replaces this once the project has an Apple Developer identity.
- **macOS only.** The core is portable Rust; `fence.rs` and the control socket are the platform-specific parts.
- **Unsigned.** Gatekeeper will warn when the app is opened on another Mac until it is signed and notarized.

## Lessons from the first real setup run

Two things only a real administrator run revealed, both now handled in `fence.rs`:

- macOS refuses to rewrite a user's home-folder attribute, even for root, unless the calling app holds a privacy permission. Setup writes user attributes only when they differ and never depends on that one.
- Folders inside any user's home (Mail, app containers) are protected even from root. Setup changes only the top of each private home folder.

- On loopback the packet filter judges a connection twice, leaving and arriving. A model server reaches its own helper process over a random local port, so arrivals at a protected user's own sockets must be allowed or inference hangs.

Setup failures now name the step and command that stopped. The app also compares the installed rules with the ones the running version generates and asks for a quick update when they differ.

## Development notes

- `SAI_DATA_DIR` moves all state (settings, ledger, key, control socket) to another folder. Keep the path short: macOS limits socket paths to 104 bytes.
- `SAI_DEV_ALLOW_PARTIAL=1` (debug builds only) treats the fence as up when the user and seatbelt layers pass but the packet filter is not installed. It exists so the app can be developed before setup has been run. Release builds ignore it.
- Icons are drawn by `src-tauri/icons/make-icons.swift`.
