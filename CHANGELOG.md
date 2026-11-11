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

Built-in rules

- llama.cpp, Cursor CLI and OpenClaw are recognised. Nothing is trusted for them until you say so.

Fixed

- [SAI-2026-003](docs/advisories/README.md): with looking inside on, a request that only claimed to switch to a web socket made the gate stop reading the rest of the connection.
- An address that is switched off could be reached by respelling it (escapes, capitals, doubled slashes). See [SAI-2026-002](docs/advisories/README.md).
