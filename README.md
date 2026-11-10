# Sandbox-ai (working name)

Keep your AI on your computer, and prove it. An open-source menu-bar app that lets local AI apps and agents reach only the places you trust, asks about anything else in plain words, and keeps a signed record.

| Folder | What is in it |
|---|---|
| `app/` | The product: Rust core, `sai` CLI, Tauri desktop shell, UI. Start with `app/README.md`. |
| `docs/` | Product spec, one-line feature spec, design principles. |
| `reports/` | Market research report. `research_notes/` holds the sourced notes behind it. |
| `spike/` | Earlier experiments that de-risked the design: the packet-filter fence, the Network Extension fence, the virtual-machine Box, a UI prototype. Superseded by `app/` except the Box, which is the planned stronger tier. |

Open the app:

```bash
open "app/target/release/bundle/macos/Sandbox-ai.app"
```
