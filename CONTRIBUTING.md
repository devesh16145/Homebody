# Contributing

Thank you for looking. This project is small and young, so a short note about what you plan to change (an issue is enough) saves both of us time before a large pull request.

## Getting set up

You need a Mac (built and tested on Apple silicon), Rust, Node, and the Xcode Command Line Tools.

```bash
cd app && npm install
```

```bash
cd app && cargo test -p sai-core
```

The UI needs no build step and no app: serve `app/ui` with any static file server and open `main.html`, `glance.html` or `dev.html`. In a plain browser it runs against an in-page mock of the same contract the app uses, with scenarios (`?scenario=private`, `watching`, `asking`, `setup`, …).

## How the code is laid out

- `app/crates/core` is everything that is not pixels: the gate, the rules, the ask flow, the signed log, the fence, projects. The app, the command line and scripts all go through one function, `Engine::dispatch`.
- `app/docs/ipc.md` is the contract between core and UI. Change it in the same pull request as the code, and keep the mock in `app/ui/js/mock.js` in step.
- `app/src-tauri` is a deliberately thin shell: windows, the menu-bar icon, one command, one event channel.
- `app/ui` is plain HTML, CSS and ES modules.

## What a good change looks like

- **Security first, and say what you did not check.** If a change touches the gate, the fence, name matching or the log, add a test for the way it could go wrong. New ways around belong in `app/tests/bypass-corpus.sh`.
- **Anything that reaches the administrator's setup script or the packet filter rules must be validated.** Names, user names, ports and paths from settings are checked in `config.rs`; follow that pattern.
- **Plain words in everyday screens.** Feed, Glance, Proof and prompts never say proxy, allowlist, TLS, sandbox or firewall. Those words may appear only in Studio → Advanced. `docs/design-principles.md` has the reasoning.
- **Small and fast.** No UI framework, no new dependency without a reason. The app is about 4 MB; please help keep it that way.
- **Presets come from observation.** A preset for an AI app should list the places it was seen to need on a real run, with a note of how you checked. Known telemetry goes in switched off.

## Reporting a security problem

Please do not open a public issue for something that lets a protected app reach the network, read your files or alter the log. See [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contribution is licensed under Apache-2.0, the license of this repository.
