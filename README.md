# Homebody

**Keep your AI on your computer, and prove it.**

A small menu-bar app for macOS that runs local AI apps and coding agents so they can reach only the places you trust. Anything else is blocked, or you are asked about it in plain words. Every decision goes into a signed record you can check and hand to someone else.

No account, no telemetry, nothing hosted. Apache-2.0. The engine and command line also run on Linux and Windows, each with a fence of its own (no window there yet).

> Early software. It works end to end on the Mac it was built on, the app is not signed yet, and the list of [what it does not do](#what-it-does-not-do-yet) is real. Read that list before relying on it.

## What it does

- **Lets an agent use what it starts.** An agent can open a preview server it started on your computer, and nobody else's.
- **Covers everything the agent starts.** MCP servers, hooks, language servers, a helper run in the background: whatever a protected agent launches runs as the same private user under the same rules. An agent's own sandbox covers its shell commands and leaves those outside.
- **Fences each AI app.** A protected app runs as its own macOS user, under a launch-time sandbox profile, behind per-user packet filter rules. Three layers, each checked every 30 seconds; nothing protected starts unless all three hold.
- **One gate out.** Each protected app can reach the network only through its own local gate. The gate allows names you trust, refuses the rest, resolves names itself, refuses names that point back at your computer or home network, and checks that the name an app greets a server with is the name that was approved.
- **Asks in plain words.** "Ollama wants to download a model. Allow once, always allow, or block?" Thirty seconds without an answer means no.
- **Shows what tried to go around.** A program that connects directly or looks up a name by itself is refused by the fence, and that refusal shows up in Activity.
- **Lets agents work on your projects safely.** Bring a folder in and the agent works on a copy. You review what it changed and apply it to your own folder when you are happy.
- **Finds your AI apps and makes protected the easy way in.** Known AI apps on the computer are listed with one-click Protect, and each protected app gets a launcher of its own for the Dock, so opening the protected copy is as easy as opening the ordinary one.
- **Looks inside, if you ask it to.** Off by default. Switched on for an app, Homebody shows which addresses inside a trusted place the app asked for, and stops a trusted name being used as cover for another site. You can then switch off one address inside a place and keep the rest: usage logging off, the conversation on. It records paths and sizes, never content, and changes nothing about which certificates your Mac trusts.
- **Stops secrets leaving.** While it looks inside, a request carrying something that looks like a private key or an access token is stopped before the secret leaves, and you decide whether that one may go. Web socket messages are read the same way.
- **Keeps your private folders private.** Mark a folder and, while it looks inside, a request that repeats lines from a file in it is stopped.
- **Keeps your API keys out of the agent's hands.** Homebody holds the key and hands the agent a stand-in. The real key is put in at the gate, only for the place it is for; a stand-in sent anywhere else is stopped and named in Activity. [docs/keys.md](docs/keys.md)
- **One switch for what agents usually need.** npm, Python, Rust, Go, Ruby, Java, Homebrew, Docker Hub and GitHub each come as a pack. The package sources are downloads-only: almost nothing may be sent to them, so a registry cannot be used to carry your work out.
- **Lets you see it hold.** *Test it* on an app's page tries to get out as that app (straight to the internet, around the gate, into your files) and shows what happened to each attempt.
- **Limits how much can go.** Each trusted place can have an hourly limit on what an app may send there, or be set to downloads only. This works for every app.
- **Watch mode.** Allow everything for an hour, then see exactly what your AI contacted and choose what to trust.
- **Report.** An append-only, hash-chained, Ed25519-signed log, with a weekly summary per app. Export it as a folder (the log, a spreadsheet of activity, the rules in force) and anyone can check it with `homebody verify <folder>`. A public time-stamping service can vouch for when it existed; only a fingerprint is sent.

Newer, and less tried than the list above (each has its own page saying exactly what has been run):

- **AI websites.** A separate profile of Chrome, Brave, Edge, Chromium or Vivaldi that can reach only the AI sites you switch on, and cannot read your home folder apart from Downloads. Add app → AI websites.
- **The Box.** Run an agent in a small Linux machine with no network interface at all; its only way out is the same gate. Apple silicon, command line. [docs/box.md](docs/box.md)
- **Linux and Windows.** The engine and command line, with a fence built from each system's own parts. [docs/platforms.md](docs/platforms.md)
- **Build pipelines.** `homebody ci` runs an agent with nobody at the keyboard and ends with a signed report. [docs/ci.md](docs/ci.md)
- **Organisations.** A fleet server you run yourself: your rules on every joined computer, their signed logs in one place, roles, single sign-on. [docs/fleet.md](docs/fleet.md)
- **For reviewers.** The rules as Cedar policies, the evidence mapped to NIST SP 800-53 and OWASP's list for AI agents ([docs/compliance.md](docs/compliance.md)), an engine that rebuilds to the same file bit for bit, and a pack for an independent review ([docs/audit](docs/audit/README.md)). No independent review has been done yet.
- **In your editor.** Plugins for VS Code and Cursor ([editors/vscode](editors/vscode/README.md)) and for JetBrains editors ([editors/jetbrains](editors/jetbrains/README.md)).

Protected today: **Claude Code** (a real signed-in session, start to finish) and **Ollama** (a model download and GPU inference under the fence). Also recognised and run protected (not signed in), each seen to send everything through the gate: **Codex CLI, Gemini CLI, Aider and opencode**. Presets also exist for **GitHub Copilot CLI, Goose, Amp, Qwen Code, Crush, Kiro CLI, Cline CLI, Droid, llama.cpp, Cursor CLI and OpenClaw**, written from their makers' documentation and not yet run here. Any other command-line program can be added with *Add app*. So can a windowed app, with [lighter protection](#what-it-does-not-do-yet): tested live with one Electron app (Obsidian), whose three start-up contacts came up as questions instead of going out.

## Try it

You need a Mac (built and tested on Apple silicon), Rust, Node and the Xcode Command Line Tools. Full Xcode is not needed.

```bash
cd app && npm install && scripts/make-dmg.sh
```

That builds the app and packs `app/target/release/bundle/dmg/Homebody-<version>-<arch>.dmg`. Open it and drag Homebody to Applications, or just open the app: it offers to move itself there.

The Homebody face appears in the menu bar and the window opens on the first-run steps. **Turn on protection** asks for your Mac password once: it creates the private users, loads the packet filter rules and installs a small watchdog that restores them after a restart. *What exactly it sets up* shows the exact script before you approve it, and can remove all of it again.

There is also a command line, `homebody`, that drives the same core:

```bash
/Applications/Homebody.app/Contents/MacOS/homebody status
```

## Check it yourself

```bash
cd app && cargo test -p homebody-core
```

```bash
cd app && tests/bypass-corpus.sh
```

The second one is every way around the gate or the fence that we know of, as executable checks (105 at the time of writing). It needs the app running and setup done. If you find a way around that is not in it, that is the most valuable contribution you can make: see [SECURITY.md](SECURITY.md).

## What it does not do yet

- **Secrets are recognised by shape, and only some.** Private keys and a handful of token formats, in the content of requests. It catches an agent carelessly sending your `.env` file, not one set on getting a secret out. The same goes for private folders: exact lines only. Looking inside, which it depends on, is not available for Ollama or windowed apps.
- **Shared kernel.** A kernel exploit from inside a protected app is not contained by the fence. The Box (a Linux machine with no network interface, [docs/box.md](docs/box.md)) is the answer; it is early: one Box at a time, command-line agents only.
- **Agents work on a copy of your project,** not on the folder itself. That is deliberate, and it means changes reach your editor only when you apply them.
- **Windowed apps get one layer, not three.** A windowed app cannot run as another user, so it runs as you under the launch-time profile alone. Nothing it sends can skip the gate, and the obvious ways out for a program running as you are closed (handing a link to your browser, driving another app, planting a login item, touching Homebody's own files). But it can read your files, only the copy you open from Homebody is protected, its own inner sandbox has to be switched off, and an app that is deliberately trying to break out has other routes, for example writing into a folder another program syncs. Apps that live in Apple's App Sandbox cannot be wrapped at all. Full-strength protection for windowed apps needs the Network Extension, which needs an Apple Developer identity.
- **Not signed or notarized.** A build you download from someone else will not open on a double-click: macOS says it cannot check who made it, and you have to allow it once in System Settings → Privacy & Security ("Open Anyway"). The release build is checked to be sealed as a whole, which is what keeps that message from being the dead-end "is damaged" one; the step itself has not been clicked through on a download yet. Building it yourself avoids all of this.
- **After a restart, protected apps wait for you.** The packet filter rules come back by themselves at boot (checked on a real restart: all 81 bypass cases passed straight afterwards), but a model server that was running protected is not started again until you start it, and Homebody itself opens at login only if you switch that on.
- **macOS only** for now, in English only, for one user account per Mac.
- **No automatic updates.** The app can check for a newer version when you ask; installing it is by hand until builds are signed.

`app/README.md` has the full, current list and the state of every part.

## What is in this repository

| Folder | What is in it |
|---|---|
| `app/` | The product: Rust core (`crates/core`), the `homebody` command line (`crates/cli`), the Tauri shell (`src-tauri`) and the UI (`ui`, plain HTML/CSS/JS, no build step). Start with `app/README.md`; `app/docs/ipc.md` is the contract between core and UI. |
| `docs/` | Product spec, one-line feature spec, design principles. |
| `reports/`, `research_notes/` | The market research behind the product, with sources. |
| `spike/` | Earlier experiments that de-risked the design: the packet-filter fence, a Network Extension fence, the virtual-machine Box, a UI prototype. Superseded by `app/`, except the Box. |

## Contributing

Issues and pull requests are welcome; [CONTRIBUTING.md](CONTRIBUTING.md) explains how the code is laid out and what a good change looks like. Presets for more AI apps (which places they really need, observed on a real run) are especially useful.

## License

[Apache-2.0](LICENSE).

The app includes the Fredoka typeface by The Fredoka Project Authors, under the [SIL Open Font License 1.1](app/ui/fonts/OFL.txt). The icons of other makers' apps are read from those apps on your own Mac and are not part of this repository.
