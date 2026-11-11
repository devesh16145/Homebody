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
