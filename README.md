# Homebody

**Keep your AI on your computer, and prove it.**

A small menu-bar app for macOS that runs local AI apps and coding agents so they can reach only the places you trust. Anything else is blocked, or you are asked about it in plain words. Every decision goes into a signed record you can check and hand to someone else.

No virtual machine, no account, no telemetry, nothing hosted. Apache-2.0. Windows and Linux are planned; the core is portable Rust.

> Early software. It works end to end on the Mac it was built on, the app is not signed yet, and the list of [what it does not do](#what-it-does-not-do-yet) is real. Read that list before relying on it.

## What it does

- **Lets an agent use what it starts.** An agent can open a preview server it started on your computer, and nobody else's.
- **Fences each AI app.** A protected app runs as its own macOS user, under a launch-time sandbox profile, behind per-user packet filter rules. Three layers, each checked every 30 seconds; nothing protected starts unless all three hold.
- **One gate out.** Each protected app can reach the network only through its own local gate. The gate allows names you trust, refuses the rest, resolves names itself, refuses names that point back at your computer or home network, and checks that the name an app greets a server with is the name that was approved.
- **Asks in plain words.** "Ollama wants to download a model. Allow once, always allow, or block?" Thirty seconds without an answer means no.
- **Shows what tried to go around.** A program that connects directly or looks up a name by itself is refused by the fence, and that refusal shows up in Activity.
- **Lets agents work on your projects safely.** Bring a folder in and the agent works on a copy. You review what it changed and apply it to your own folder when you are happy.
- **Finds your AI apps and makes protected the easy way in.** Known AI apps on the computer are listed with one-click Protect, and each protected app gets a launcher of its own for the Dock, so opening the protected copy is as easy as opening the ordinary one.
- **Looks inside, if you ask it to.** Off by default. Switched on for an app, Homebody shows which addresses inside a trusted place the app asked for, and stops a trusted name being used as cover for another site. You can then switch off one address inside a place and keep the rest: usage logging off, the conversation on. It records paths and sizes, never content, and changes nothing about which certificates your Mac trusts.
- **Stops secrets leaving.** While it looks inside, a request carrying something that looks like a private key or an access token is stopped before the secret leaves, and you decide whether that one may go. Web socket messages are read the same way.
- **Keeps your private folders private.** Mark a folder and, while it looks inside, a request that repeats lines from a file in it is stopped.
- **Limits how much can go.** Each trusted place can have an hourly limit on what an app may send there, or be set to downloads only. This works for every app.
- **Watch mode.** Allow everything for an hour, then see exactly what your AI contacted and choose what to trust.
- **Proof.** An append-only, hash-chained, Ed25519-signed log, with a weekly summary per app. Export it as a folder (the log, a spreadsheet of activity, the rules in force) and anyone can check it with `sai verify <folder>`. A public time-stamping service can vouch for when it existed; only a fingerprint is sent.

Protected today: **Claude Code** and **Ollama** (both tested live, including a model download and GPU inference under the fence). Also recognised and run protected (not signed in), each seen to send everything through the gate: **Codex CLI, Gemini CLI, Aider and opencode**. Presets also exist for **llama.cpp, Cursor CLI and OpenClaw**, written from their documentation and not yet run here. Any other command-line program can be added with *Add app*. So can a windowed app, with [lighter protection](#what-it-does-not-do-yet): tested live with one Electron app (Obsidian), whose three start-up contacts came up as questions instead of going out.

## Try it

You need a Mac (built and tested on Apple silicon), Rust, Node and the Xcode Command Line Tools. Full Xcode is not needed.

```bash
cd app && npm install && scripts/make-dmg.sh
```

That builds the app and packs `app/target/release/bundle/dmg/Homebody-<version>-<arch>.dmg`. Open it and drag Homebody to Applications, or just open the app: it offers to move itself there.

The Homebody face appears in the menu bar and the window opens on the first-run steps. **Turn on protection** asks for your Mac password once: it creates the private users, loads the packet filter rules and installs a small watchdog that restores them after a restart. *What exactly it sets up* shows the exact script before you approve it, and can remove all of it again.

There is also a command line, `sai`, that drives the same core:

```bash
/Applications/Homebody.app/Contents/MacOS/sai status
```

## Check it yourself

```bash
cd app && cargo test -p sai-core
```

```bash
cd app && tests/bypass-corpus.sh
```

The second one is every way around the gate or the fence that we know of, as executable checks (81 at the time of writing). It needs the app running and setup done. If you find a way around that is not in it, that is the most valuable contribution you can make: see [SECURITY.md](SECURITY.md).

## What it does not do yet

- **Secrets are recognised by shape, and only some.** Private keys and a handful of token formats, in the content of requests. It catches an agent carelessly sending your `.env` file, not one set on getting a secret out. The same goes for private folders: exact lines only. Looking inside, which it depends on, is not available for Ollama or windowed apps.
- **Shared kernel.** A kernel exploit from inside a protected app is not contained. A virtual-machine tier is planned (`spike/box` is the experiment).
- **Agents work on a copy of your project,** not on the folder itself. That is deliberate, and it means changes reach your editor only when you apply them.
- **Windowed apps get one layer, not three.** A windowed app cannot run as another user, so it runs as you under the launch-time profile alone. Nothing it sends can skip the gate, and the obvious ways out for a program running as you are closed (handing a link to your browser, driving another app, planting a login item, touching Homebody's own files). But it can read your files, only the copy you open from Homebody is protected, its own inner sandbox has to be switched off, and an app that is deliberately trying to break out has other routes, for example writing into a folder another program syncs. Apps that live in Apple's App Sandbox cannot be wrapped at all. Full-strength protection for windowed apps needs the Network Extension, which needs an Apple Developer identity.
- **Not signed or notarized.** macOS will warn when you open a build from someone else. Building it yourself avoids that.
- **After a restart, protected apps wait for you.** The packet filter rules come back by themselves at boot (checked on a real restart: all 81 bypass cases passed straight afterwards), but a model server that was running protected is not started again until you start it, and Homebody itself opens at login only if you switch that on.
- **macOS only** for now, in English only, for one user account per Mac.
- **No automatic updates.** The app can check for a newer version when you ask; installing it is by hand until builds are signed.

`app/README.md` has the full, current list and the state of every part.

## What is in this repository

| Folder | What is in it |
|---|---|
| `app/` | The product: Rust core (`crates/core`), the `sai` command line (`crates/cli`), the Tauri shell (`src-tauri`) and the UI (`ui`, plain HTML/CSS/JS, no build step). Start with `app/README.md`; `app/docs/ipc.md` is the contract between core and UI. |
| `docs/` | Product spec, one-line feature spec, design principles. |
| `reports/`, `research_notes/` | The market research behind the product, with sources. |
| `spike/` | Earlier experiments that de-risked the design: the packet-filter fence, a Network Extension fence, the virtual-machine Box, a UI prototype. Superseded by `app/`, except the Box. |

## Contributing

Issues and pull requests are welcome; [CONTRIBUTING.md](CONTRIBUTING.md) explains how the code is laid out and what a good change looks like. Presets for more AI apps (which places they really need, observed on a real run) are especially useful.

## License

[Apache-2.0](LICENSE).
