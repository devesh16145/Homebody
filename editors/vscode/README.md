# Homebody for VS Code and Cursor

Homebody keeps your AI agents from sending anything anywhere you have not agreed to. This extension brings the parts you need while working into the editor. It talks only to the Homebody app running on this computer; it has no network code of its own.

- **Status bar**: Private, Watching, a question waiting, or everything blocked. Click it for the actions below.
- **Questions**: when an agent wants to reach a new place, the question appears here as well as in Homebody. Answer in either.
- **Open an agent protected, in this folder**: the agent works on a copy of the open folder in a terminal inside the editor. It cannot touch anything else of yours.
- **Review what the agent changed**: see its changes and put them back into your folder when you are happy.
- **Trust places this project asks for**: a project can list the places its tools need in `.homebody.json`. They are shown to you; nothing is trusted unless you tick it. The file is checked as you type.
- **Block everything now**.

## `.homebody.json`

```json
{ "trust": ["registry.npmjs.org", "*.example.com"] }
```

## Install

Build the package and install it from the file:

```bash
./package.sh
```

Then in the editor: Extensions → … → Install from VSIX, and choose `homebody-0.1.0.vsix`.

## Test

```bash
node test/run.js
```

Add `--live` to also ask the Homebody running on this computer for its status (read-only).

### Inside a real editor

```bash
test/in-editor/run.sh            # code if it is installed, else cursor; or name the command
```

This starts the editor itself with a throw-away profile and a throw-away folder, loads the extension into it, and runs every command against the Homebody running on this computer. A script stands in for the person and answers the pop-ups; everything else is real: the status bar, the terminal, the engine, the fence. It brings the folder in for one agent, opens that agent in the editor's terminal for a few seconds, has the agent change a file and takes the change back, answers one of Homebody's questions, blocks everything for a moment, and puts all of it back.

## What has been run

- `node test/run.js`: 17 tests with a stand-in editor.
- `test/in-editor/run.sh` in **Cursor 3.28 (VS Code 1.128)** on a Mac, 33 of 33, in the editor's ordinary window with a throw-away profile. Among them: the folder's own settings file names another program as "the homebody command" and is not obeyed; the agent runs as its private user and is gone within seconds of its terminal being closed; a wrong `.homebody.json` is marked as you type.
- Two faults were found this way and fixed. A folder opened through a link (`/tmp` on a Mac) was not recognised as already brought in. And closing the agent's terminal left the agent running unseen until the editor quit: an editor stops a terminal's program with "hang up", which Claude Code does not quit on. That one was in Homebody's command line, not here (`app/tests/terminal.py`).
- The packaged file was installed into Cursor on the same Mac. Cursor 3 opens in its agent window by default, and that window runs only Cursor's own built-in extensions; this one starts in the editor window. It has not been run in VS Code itself, on Linux or on Windows.
