# For an independent security review

No independent review of Homebody has been done yet. This folder is what a reviewer needs to start: what the product claims, what is in scope, where the security-relevant code is, how to rebuild the binary under review, and how to re-run every check the project runs itself.

It is written by the project. A reviewer should treat every statement here as a claim to test, not as a finding.

## The claims

1. **A protected app can send to the network only through its gate.** Direct connections, name lookups and other apps' gates are refused by the operating system, not by the app's cooperation.
2. **The gate lets through only what the rules allow**, judged by the name the app is really talking to: no raw addresses, no names that resolve to this computer or the local network, no greeting one name and reaching another.
3. **A protected app cannot change its own rules, answer its own questions, or read or alter the log.**
4. **The log shows tampering.** A changed, removed or reordered record breaks the chain, and anyone can check that without the computer it was written on.
5. **A protected app cannot read the user's home folder.**
6. **An organisation's server can only add rules and receive logs.** It cannot forge another fleet's rules, and it cannot alter a computer's log without that showing.

What is *not* claimed is listed in SECURITY.md under "What is already known and stated" and shown in the app on the Report screen: content hidden inside allowed requests in forms Homebody does not recognise, timing channels, a kernel flaw (outside the Box), and anything a windowed app does to the user's own files.

## Scope, by risk

| Part | Where | Why it matters |
|---|---|---|
| The gate: request parsing, name canonicalisation, TLS greeting check | `app/crates/core/src/proxy.rs`, `hostname.rs` | Every earlier bypass in this class of product was a parsing bug here |
| Looking inside: local certificate authority, request line and body scanning | `inspect.rs`, `secrets.rs` | Terminates TLS; a mistake could weaken connections or leak the key |
| The fence on macOS: profile, packet filter rules, the root setup script and watchdog | `fence.rs` | Runs as root once; generates rules from settings |
| The fence on Linux | `fence_linux.rs` | The same, with nftables and systemd |
| The fence on Windows | `fence_windows.rs`, `cli/src/contain.rs` | Hand-written calls into Windows; a named pipe with an access list |
| The log | `ledger.rs` | The evidence |
| The fleet server | `app/crates/fleet/` | The only part that listens on a network |
| Fleet membership in the app | `fleet.rs`, and "org" in `config.rs`, `engine.rs` | Lets a server change local rules |
| The Box | `boxvm.rs`, `app/box/` | A second boundary; its helper bridges guest and gate |
| The control socket | `control.rs` | Whoever can write to it controls Homebody |

Out of scope: the review boards and mock data under `app/ui/` (not shipped behaviour), and `spike/` (experiments).

## Reproduce the binary

```bash
cd app && scripts/repro-check.sh
```

Builds the engine twice from scratch in two different folders and compares the results. `scripts/repro-build.sh` alone prints the SHA-256 to compare with a published one. Verified on macOS (Apple silicon) and on Linux (x86-64). The desktop app bundle is not reproducible yet: bundling adds timestamps, and signing depends on who distributes it.

## Re-run the checks

| Check | Command | Needs |
|---|---|---|
| Unit tests (engine) | `cd app && cargo test -p homebody-core` | nothing |
| Unit and whole-server tests (fleet) | `cargo test -p homebody-fleet` | `tar`, `curl`; `openssl` for the sign-in test |
| Bypass corpus, 81 cases | `SAI=$PWD/target/release/homebody tests/bypass-corpus.sh` | macOS with setup done and the engine running |
| The fence on Linux, 41 cases | `tests/linux-fence.sh` | a throwaway Linux machine with sudo |
| The fence on Windows | `tests/windows-fence.sh` (Git Bash) | Windows 11 |
| The Box, 21 cases | `tests/box.sh` | Apple silicon Mac with the Box set up |
| The Cedar export against Cedar's own engine | `cargo run --manifest-path tools/cedar-check/Cargo.toml` | downloads the Cedar library |
| The editor extension | `node editors/vscode/test/run.js` | Node |

Every bypass found so far and what fixed it is in [docs/advisories](../advisories/README.md).

## What the project already knows is weak

Stated here so a reviewer does not spend time rediscovering it, and can check whether it is worse than described.

- **macOS windowed apps and the browser profile run as the user** under one layer (the launch-time profile), with their own inner sandbox switched off. Their network is fenced; their access to the user's files is not (the browser profile's is: it cannot read the home folder apart from Downloads).
- **In every Electron app tried, one helper process connects directly on port 443** and is refused by the profile. Why it does not use the gate is not understood.
- **Linux: rules removed by an administrator are restored within 20 seconds, not instantly.**
- **Linux: no second layer beside the firewall rules and the private user.** macOS has a launch-time profile as well.
- **Windows: a protected program cannot be stopped from Homebody**, and the relay's port is chosen a moment before it is used.
- **The fleet server's console allows inline script** (its page is one self-contained file). Its content security policy forbids everything else.
- **Secrets are recognised by shape.** A secret in a shape Homebody does not know is not caught.
- **The Box's guest image is stock Debian** with AppArmor switched off in the guest and pointer authentication disabled, both to work around guest crashes seen under Apple's hypervisor.

## Reporting

See SECURITY.md. Findings are published as advisories with the fix; nothing is patched silently.
