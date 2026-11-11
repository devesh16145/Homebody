# Sandbox-ai (working name)

Keep your AI on your computer, and prove it.

A menu-bar app for macOS (Windows and Linux to follow) that runs local AI apps and agents so they can reach only the places you trust, asks you in plain words about anything else, and keeps a signed record of every decision. No virtual machine. Apache-2.0, no accounts, no telemetry, nothing hosted.

Status: 11 Nov 2026. Everything in the table runs end to end on this Mac in the release app, installed in `/Applications`. Verified live: all three fence layers, Ollama protected with GPU access, model download and inference under the fence, projects, go-arounds in the Feed, Watch, log export, clean stop, windowed apps, launchers, installing into Applications, recovery after the app is force-killed. Not yet verified: that protection is restored by the watchdog after a restart; the open-at-login switch across a real login; the release workflow (never run); signing and notarization (no Apple Developer identity yet).

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
| CLI `sai`: status, feed, watch, mode, watched, asks, allow/block, trust, limit, ask, inspect, secrets, inside, private, proof, summary, timestamp, verify, export-log, project, found, update, run, terminal, open, daemon | Working |
| Protected apps | Claude Code (agent) and Ollama (local models): tested live. Codex CLI, Gemini CLI, Aider, opencode: recognised when installed, each with its own private user and gate; their short trusted lists follow the tools' documentation and have not been run here |
| Protect another app: any command-line program gets its own private user, gate, workspace and an empty Trusted list (Studio → Apps) | Built, unit-tested; needs one more approval of setup per added app |
| Windowed apps (lighter protection): added from Studio → Apps, opened protected as you under the launch-time profile, web traffic pointed at the app's own gate; handing things to other apps, driving other apps, login items and Sandbox-ai's own files are closed to it | Working for Electron apps, tested live with Obsidian: start-up contacts arrived at the gate as questions, direct connections and lookups were refused and shown. Not tried with LM Studio, Jan or Comfy, or with a non-Electron app (those would be offline rather than asked) |
| Found on this Mac: known windowed AI apps in the Applications folders are listed in Studio → Apps with one-click Protect, or with the reason they cannot be protected | Working; on this Mac it found Antigravity, Bionic, Claude, Comfy Desktop, Cursor (can be asked), ChatGPT and Hermes (offline only) |
| Launchers: every protected app can have "<Name> (protected)" in `~/Applications/Sandbox-ai`, for the Dock or Spotlight. Clicking it opens the app protected, starting Sandbox-ai first if it is not running | Working, both cases tested live. The launcher points at where Sandbox-ai is now; if the app is moved, use Show launcher again |
| Installing: a disk image (`scripts/make-dmg.sh`, 3 MB) with the app and the `sai` command-line tool inside it; the app offers to move itself into Applications and reopens from there; launchers and the login entry follow the move | Working, tested live on this Mac. The image is unsigned, so macOS warns on other Macs |
| Recovery: a protected app left running by a run that ended abruptly is stopped at the next start, instead of blocking it | Working, tested by force-killing the app with Ollama running |
| Own local servers: an agent may connect to servers it started itself on this computer and to nobody else's. The watchdog lists, twice a second while an agent runs, the ports that agent is listening on and opens exactly those to it; for those ports, arrivals at anyone else's socket are refused, so a port that changes hands is not reachable in between. Setup tests this on the computer (own server reachable, someone else's refused) and falls back to the stricter rules if it does not hold | Working, tested live: own server reached on the first try; your server, another agent's server and a port that changed hands all refused. Two things to know. With it on, the launch-time profile allows any local port except other gates, so this one property rests on the packet filter alone. And a refused local connection now times out instead of failing at once |
| Opening the app shows its window (also when it is already running in the menu bar); a start at login or through a launcher stays quiet | Built, not yet seen by a person |
| Every run of setup keeps a record of what it did as administrator (`logs/setup.sh.last.log`) | Working; it is what explained the failed first design |
| Looking inside (off by default, per app): the gate ends the app's encrypted connection with a certificate from an authority that exists only on this computer, opens its own to the real server and checks that server's certificate the ordinary way, and reads the request lines in between. The Feed shows which paths were asked for and how often. The site named inside a request must be the approved one, which stops a trusted name being used as cover for another site; requests that cannot be followed (two lengths, unusual encodings, a tunnel inside) are not let through. Only method and path are recorded, never what follows "?", headers or content. Nothing is added to the Mac's trusted certificates: only protected apps are told, through their environment | Working, tested live with Claude Code, Codex, Gemini, Aider and opencode (each accepted it), seven bypass cases. Not available for Ollama or windowed apps: they take trusted certificates from the Mac itself. HTTP/1.1 only is offered while looking inside |
| Switches inside a place: with looking inside on, one address inside a trusted place can be refused while the rest keeps working ("Refuse" in the Feed, or Studio → Trusted). Each request is held back until it has been checked, so nothing of a refused request leaves, not even its first line. An address is judged as written and as a forgiving server would read it (escapes, capitals, dot segments, doubled slashes, parameters), so respelling it does not get around the switch. Claude Code's event logging comes switched off | Working: 13 bypass cases, tested live with direct requests (the refused address gets a 403 from Sandbox-ai, its neighbours and the conversation address reach the real server). Not yet seen: Claude Code itself being refused its event logging, because it did not try to send any in the runs after the switch existed |
| Stop secrets leaving (on whenever looking inside is on): the content of each request is examined as it passes for private key blocks and for access tokens with a maker's prefix (GitHub, Slack, Stripe, Anthropic, OpenAI project keys, Google API keys, AWS access key ids). A request carrying one is stopped and the app is told why. The most recent 512 bytes of content are always held back, so a secret sent in pieces does not get its first half out. The Feed shows the kind and offers "Let this one through", which applies to that one secret for that one app. The secret is never stored: only a fingerprint particular to this computer | Working: 10 unit tests on the recogniser and the hold-back, 7 bypass cases, and the whole flow run live with made-up tokens |
| Web sockets are followed too (with looking inside on): a switch to a web socket is believed only once the real server has agreed to it, and what the app then sends is read message by message and examined for secrets like any other request. Compression is not offered, so the content stays readable | Working: tested live against a public echo server and with unit tests on every message size; one bypass case (a pretend switch, see advisory SAI-2026-003) |
| Switches for one kind of request: an address inside a trusted place can be refused for sending (`POST`) while reading (`GET`) stays allowed | Working, one bypass case |
| Private folders (Studio → Trusted): text from files in folders you mark is never sent. With looking inside on, a request that repeats three or more long lines from one of those files is stopped, like a secret | Working, tested live with a real folder. Exact lines only: reworded, encoded or shortened text is not recognised |
| Hourly limits (Studio → Trusted): each trusted place can be given a limit on how much an app may send there per hour, or "Downloads only". Counted by the gate, so it works for every app, including Ollama and windowed apps | Working, tested live: the connection is cut at the limit and new ones are refused until the hour moves on |
| "Ask me about new places" can be switched off: anything not trusted is then refused quietly | Working |
| When the built-in rules change with a new version, Studio → Trusted says what changed. Nothing new is ever trusted by itself | Working |
| First run in three steps: what it does, one approval, then a choice between Private and "Watch for an hour" | Working in the preview; the approval step itself was run for real in earlier builds |
| Proof → the last seven days: per app, what was reached, what was blocked, what was stopped | Working, computed from the signed log |
| Independent timestamp (Proof): a public time-stamping service vouches for when the log existed. Only a fingerprint of the log is sent | Working, tested live with DigiCert's service; the answer is kept and exported with the log |
| Export for an auditor: a folder with the signed log, the Feed as a spreadsheet, the rules in force, timestamps and a read-me. `sai verify <folder>` checks it on any computer | Working, verified after export; a changed log fails |
| Projects can suggest places: a `.sandbox-ai.json` in a folder you bring in lists the places the project needs (a package registry, say). You are shown the list and tick what to trust | Working. The file is only ever a suggestion |
| Update check (Studio → Advanced): only when you click, and shown in the Feed. Nothing is downloaded | Working; answers "No version has been published yet" because none has |
| More presets: llama.cpp, Cursor CLI, OpenClaw | Rules written from their documentation; none of the three was installed and run here |
| A program installed inside your home folder is detected and explained instead of failing silently | Working |
| Open at login (Studio → Advanced) | Built; not yet tried across a real login |

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
    inspect.rs       looking inside: the local certificate authority, reading request lines
    secrets.rs       recognising private keys and access tokens in what is about to be sent
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

Sizes in this build: 90 unit tests, 81 bypass cases, a 7.2 MB app including the command-line tool (the encryption libraries for looking inside added about 2 MB), a 4 MB disk image.

Measured on this Mac (Apple silicon, release build, 11 Nov 2026): ready 0.4 seconds after starting; the gate adds about 0.3 ms to opening a connection; looking inside adds about 9 ms to a whole request; the main process holds 104 MB when idle with one model server protected (the target was under 100; the window's own web view processes come on top while the window is open).

## Verify it yourself

```bash
cargo test -p sai-core
```

```bash
tests/bypass-corpus.sh
```

The corpus needs the app (or `sai daemon`) running. The gate half runs anywhere; the fence half runs commands as the protected user and needs setup done.

## Real apps, run protected (11 Nov 2026)

Each was opened protected for about 20 seconds with every prompt answered "block". All four started, stayed up, and quit cleanly.

| App | What it asked for on start-up | Notes |
|---|---|---|
| Cursor | Its API servers (`api2`–`api5.cursor.sh`, `api.origin.cursor.com`), `metrics.cursor.sh`, `downloads.cursor.com`, `marketplace.cursorapi.com`, and a tool server configured by the user | Five direct connections on port 443 by a helper process were refused |
| Antigravity | `oauth2.googleapis.com`, `play.googleapis.com` (usage statistics), its auto-updater | Its language server tried a direct connection and a name lookup; refused |
| Comfy Desktop | `desktop-assets.comfy.org`, `media.comfy.org`, `storage.googleapis.com`, `github.com`, Datadog, the ToDesktop updater | Retried blocked downloads more than 8,000 times a minute, which is why refusals now pause for a quarter of a second. Its Python side tried name lookups; refused |
| Bionic | Nothing through the gate | One name lookup, refused |

The four command-line agents were installed and run the same way, not signed in (a fake key where one was needed to get as far as the network):

| Agent | Result | What was seen |
|---|---|---|
| Codex CLI 0.167.1 | Works through the gate | Reached `api.openai.com` and `chatgpt.com` (trusted; OpenAI answered "not signed in"); asked for `files.openai.com` and GitHub. Nothing went around |
| Gemini CLI 0.68.0 | Works through the gate | Reached its API (trusted; Google answered "bad key"); `play.googleapis.com` (usage statistics) was blocked by the preset |
| Aider 0.86.2 | Works through the gate | Asked for its provider, `raw.githubusercontent.com` (model list) and `pypi.org` (update check) |
| opencode 2.2.7 | Works through the gate, once own local servers are on | It talks to a local server of its own. Asked for `opencode.ai` and `models.opencode.ai`: by default it sends prompts to a model hosted by opencode |

opencode exposed a gap that affected every agent: one that starts a preview server could not open it. That is what "own local servers" in the table above fixes. The first design for it (marking an agent's local traffic as it leaves and recognising the mark where it arrives) failed its own test here: macOS drops the mark in between. The watchdog's port list replaced it.

What this shows and does not show: the apps run, their start-up traffic is visible and under your control, and what goes around the gate is refused. It does not show that each app is fully usable once you trust its places: in every Electron app one helper still tries to connect directly, so some feature of each is cut off, and we have not found which.

## What looking inside showed on the first run

Claude Code, started once with a made-up key, sent these to `api.anthropic.com`, the one name on its trusted list:

| Request | Sent |
|---|---|
| `POST /api/event_logging/v2/batch` | 225 KB |
| `POST /v1/messages` (the conversation itself) | 49 KB |
| `POST /api/eval/sdk-…`, `GET /api/claude_cli/bootstrap`, `GET /mcp-registry/v0/servers`, a few more | a few hundred bytes each |

So the "219 KB to Anthropic at start-up" noted in the first build was event logging riding on the trusted name, not the conversation. Sandbox-ai now shows the difference and can refuse the one while allowing the other: that address comes switched off for Claude Code whenever looking inside is on.

## What it does not do yet, stated plainly

- **Agents work on a copy of your project, not on the folder itself.** This is deliberate: nothing an agent does can damage your work until you apply it. The costs: changes are not live in your editor until applied, the agent's own commits stay in the copy (its changes arrive in your folder as ordinary edits), and files you edit after bringing a project in do not reach the copy. Sharing a folder in place is possible but was left out for now, because granting an agent a path into your home also exposes neighbouring files to it by file permissions alone.
- **A link an agent creates is applied as a link.** The review shows where every new link points; read that line before applying.
- **Secrets are recognised by shape, and only some.** Private key blocks and a handful of token formats. Not passwords, not secrets without a tell-tale form, and not one that has been encoded, compressed, split with other text in between or reworded. It catches an agent carelessly sending your `.env` file; it does not stop an agent set on getting a secret out. Only the content of requests is examined, not addresses or headers.
- **Web sockets are examined only in one direction and only uncompressed.** What the app sends is read; what comes back is not. An app that insists on compressed messages will find the connection refused rather than passed unread.
- **Private folders match exact lines.** Lines of 40 characters or more, three of them in one request. Short files, reworded text, or text the app has encoded are not recognised, and files over the index's size are skipped. It is there to catch an agent pasting your notes into a request, not to stop a determined one.
- **Hourly limits count what is sent, per trusted place, per app.** They do not limit downloads, and an app can still send up to the limit.
- **No translations yet.** Every sentence is in English.
- **One user per Mac.** A second account on the same Mac would need its own setup; that has not been built or tried.
- **A stopped secret can stall a conversation.** If the secret is in an agent's conversation so far, every later request carries it and is stopped too, until you let that one through or start a fresh conversation.
- **With looking inside off (the default) none of this acts.** The Feed shows only the place and the size, and the switches inside a place and the check for secrets do nothing.
- **A refused address closes its connection.** The app is told "403, switched off" and the connection ends; an app that had other requests waiting on the same connection has to reconnect. Refusing an address an app depends on can stop the app working: the switch is yours to turn back on.
- **Attempts that go around the gate are shown without the address.** macOS reports the program and the port, not where it was trying to go. Attempts to read your files are stopped by ordinary file permissions before the fence sees them, so those are not shown at all. The Network Extension version will show both.
- **A trusted name can front for another site only where looking inside is off.** The greeting check stops the simple version everywhere; the full check (the site named inside each request) needs looking inside, which Ollama and windowed apps cannot use.
- **Shared kernel.** A kernel exploit from inside a protected app is not contained. The virtual-machine tier (see `../spike/box`) is the answer and is planned.
- **The setup script runs from a file in your account** for the moments between writing it and approving it. A signed privileged helper replaces this once the project has an Apple Developer identity.
- **Windowed apps get one layer.** They run as you under the launch-time profile, with no private user and no packet filter behind it. They can read your files; only the copy opened from Sandbox-ai is protected; their inner sandbox is switched off (macOS refuses a sandbox inside a sandbox); links they try to open in your browser are refused and shown. Apps in Apple's App Sandbox cannot be wrapped. An app deliberately trying to break out has routes left. The Network Extension is the full answer.
- **macOS only.** The core is portable Rust; `fence.rs` and the control socket are the platform-specific parts.
- **Unsigned, and no automatic updates.** Gatekeeper will warn when the app is opened on another Mac until it is signed and notarized; `.github/workflows/release.yml` is ready to do both once the Apple secrets exist. Studio → Advanced can check for a newer version when you ask, but updating still means downloading the newer image yourself: installing an update automatically needs a signed build, so that the app can tell a real update from a forged one.

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
