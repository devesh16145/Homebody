# Changes

No version has been released yet. This file starts with the first release; until then the git history is the record, and [docs/advisories](docs/advisories/README.md) lists every way around that was found and fixed.

Each release will list, in this order: what is new, what changed in the built-in rules (the same notes the app shows after an update), and what was fixed, with security fixes named and linked to their advisory.

## Not released yet

New

- The app is called Homebody now, with a new look: a face peeking over the windowsill, in the menu bar and the Dock.
- A new main window organised around your apps: a sidebar with each app, a big Open button, and one list per app of the places it can reach, each with its own switch.
- The menu-bar panel shows a tile per app: click it to open that app protected.
- The question pop-up, Activity and Report have the new look: a countdown ring, a card per day, the cat at the top of Report.
- Your AI apps show their makers' own icons when those Mac apps are installed (Claude for Claude Code, Cursor for the Cursor CLI).
- The Homebody lettering (Fredoka) ships with the app; nothing is loaded from the web.
- The cat moves: it peeks up, ducks when everything is blocked, hops when a question arrives, and blinks.
- Everything behind the scenes is called Homebody too: the protection service, its rules, the private users (`_hb…`), the data and workspace folders, and the command line (`homebody`). An existing setup is moved over in one approval: users are renamed in place, so sign-ins and project copies stay.
- Web socket messages are read while looking inside, and examined for secrets like any other request.
- A switch inside a trusted place can apply to one kind of request (refuse sending, allow reading).
- Private folders: a request that repeats lines from a file in a folder you marked is stopped.
- Hourly limits on what an app may send to a trusted place, including "Downloads only".
- "Ask me about new places" can be switched off.
- First run in three steps, ending with a choice between Private and Watch for an hour.
- Proof shows the last seven days per app, and can add an independent timestamp.
- The log exports as a folder an auditor can use: signed log, activity as a spreadsheet, rules in force, timestamps.
- A project can suggest the places it needs in `.homebody.json`; nothing is trusted until you tick it.
- Check for a newer version, only when you ask.
- AI websites: a separate profile of Chrome, Brave, Edge, Chromium or Vivaldi that can reach only the AI sites you switch on (ChatGPT, Claude, Gemini, Copilot, Perplexity, Le Chat, DeepSeek, Grok). Pages in it cannot read your home folder, apart from Downloads.
- The Box: run an agent in a small Linux machine that has no network interface at all. Its only way out is the same gate. Apple silicon, early. It has a card on the agent's page (set up, put the agent in, open, stop) and its disk is made without Homebrew.
- Linux: the engine and the command line, with a fence made of a private user per app and firewall rules. No window yet.
- Windows: the engine and the command line, with a fence made of app containers. No setup, no administrator. No window yet.
- For build pipelines: `homebody ci` runs an agent with nobody at the keyboard and ends with a signed report; a GitHub action sets it up.
- For organisations: `homebody-fleet`, a server you run yourself. Joined computers take its signed rules and send it their logs. A web console with roles, sign-in through your identity provider, provisioning, evidence bundles, and a management profile for Macs.
- For reviewers: the rules as Cedar policies (`homebody policy`), a mapping of the evidence to NIST SP 800-53 and OWASP's list for AI agents, and an engine that rebuilds to the same file bit for bit.
- Editor plugins: VS Code and Cursor, and JetBrains editors. A status indicator, Homebody's questions, an agent opened protected on a copy of the open project. Each has a test that runs it inside the real editor.
- Keys that stay home: Homebody can keep an API key for an agent. The agent is handed a stand-in; the real key is put into its requests at the gate, only for the place it is for. A stand-in sent anywhere else is stopped and named in Activity. See `docs/keys.md`.
- Packs: one switch each for npm, Python, Rust, Go, Ruby, Java, Homebrew, Docker Hub and GitHub. Package sources are downloads-only.
- Test it: on an app's page, Homebody tries to get out as that app and shows what happened to each attempt. `homebody test <app>` does the same.
- Eight more agents are recognised: GitHub Copilot CLI, Goose, Amp, Qwen Code, Crush, Kiro CLI, Cline CLI and Droid. For each, only what its maker says it cannot work without is trusted, and its usage statistics are switched off.
- In the Box: Look inside works, and a key Homebody keeps reaches the Box only as a stand-in. The real key never enters the machine.
- Throw away what an agent did to its copy of a project and start over from your folder, in one click.
- With looking inside on, a second request can no longer be hidden inside the first (SAI-2026-006).
- An hourly limit can no longer be exceeded by many connections sending at once, and the first bytes of a connection count (SAI-2026-007).
- A link planted in an agent's workspace can no longer send a copy of your project somewhere else (SAI-2026-008).
- `homebody verify` checks a shared proof file as well as an exported log.
- The app is sealed as a whole when it is built. A download of an unsealed app is shown by macOS as "damaged", with no way to open it; a sealed one gets the ordinary warning for apps without an Apple signature.
- Every release build is now tried on a second Mac that has never had Homebody: installed from the disk image, set up, attacked with the whole bypass corpus, and removed again.
- A question you did not see is not lost. When nobody answers in time the place is still blocked, but the question stays on screen with its answers until you deal with it. The question window follows you to whichever desktop and screen you are on, and makes a soft sound (Settings can switch that off).
- A protected app can no longer put a macOS password or keychain prompt on your screen (SAI-2026-005). Signing in to Claude Code under Homebody no longer stops at "Keychain Not Found".
- A protected program never outlives its terminal. Closing an agent's terminal in an editor used to leave the agent running unseen until the editor quit; `homebody run` now stays beside the program and ends it.

Built-in rules

- llama.cpp, Cursor CLI and OpenClaw are recognised. Nothing is trusted for them until you say so.

Fixed

- [SAI-2026-004](docs/advisories/README.md): setup could be tricked by a protected app into creating a file the app owns, anywhere on the computer. Run setup once more after updating ("Check now" in Settings will ask).
- Linux: anything started as a protected user gets a terminal of its own, so it cannot type into yours.

- The gate recorded a connection that a browser opened ahead of need and never used as a refused "not an ordinary encrypted connection". A connection that sends nothing is no longer recorded.

- [SAI-2026-003](docs/advisories/README.md): with looking inside on, a request that only claimed to switch to a web socket made the gate stop reading the rest of the connection.
- An address that is switched off could be reached by respelling it (escapes, capitals, doubled slashes). See [SAI-2026-002](docs/advisories/README.md).
