# Linux and Windows

Homebody started on macOS. The engine (rules, gate, signed log, the command line) is the same code on all three systems. The fence, the part that makes going around the gate impossible, is built from what each system has.

| | macOS | Linux | Windows |
|---|---|---|---|
| A space of its own for each app | a hidden user | a system user | an app container |
| No network except the gate | launch-time profile + packet filter rules per user | firewall (nftables) rules per user | the app container has no network capability; a relay inside it reaches the gate through a pipe only it may open |
| No name lookups around the gate | profile closes the resolver | firewall closes port 53; the resolver's local sockets are closed to the protected group | no network capability, so none |
| Your files | home folder closed by profile | home folder closed to the protected group | your user folder is not readable from an app container |
| Needs an administrator | once, for setup | once, for setup | never |
| Kept in place across restarts | a small root service | a systemd service | Windows itself |
| The desktop app | yes | not yet: command line only | not yet: command line only |
| Windowed apps, browser profile, the Box | yes | no | no |
| Looking inside connections | yes | yes (not yet run there) | no |

## Linux

```bash
homebody daemon &                      # the engine; there is no window yet
homebody setup --print | sudo bash     # once
homebody run claude-code -- claude     # an agent, protected, in this terminal
homebody asks                          # questions waiting; answer with: homebody allow <id> [--always] / homebody block <id>
```

Setup needs `nft`, `setfacl`, `sudo`, `setpriv`, `useradd` and `systemctl`, and says which package is missing. It creates a group and one system user per installed agent, firewall rules for those users, a service that keeps the rules loaded, and one `sudoers` line so you can start things as those users. It adds one entry to your home folder's access list (the protected group may not enter) and changes nothing else about it. `homebody setup --print-remove | sudo bash` removes everything again.

A protected program is started with "no new privileges", so `sudo`, `su` and other set-user-ID programs give it nothing.

Not on Linux yet: agents reaching servers they started themselves (only the listed local ports, such as a local model's, are open), and the window.

**Checked** on a GitHub-hosted Ubuntu machine on every push to the work branch (`app/tests/linux-fence.sh`, 41 cases, each refusal preceded by the same thing succeeding without the fence): setup from nothing, twice; direct connections over IPv4, IPv6, UDP and ping; lookups through the system, from a program, to the local resolver's port and by asking the resolver service; another app's gate; the home folder; `sudo`; the workspace; headless runs; rules removed by hand coming back within 20 seconds; refused attempts reaching Activity; removal. The gate half of the bypass corpus (29 cases) runs there too.

**Not checked:** other distributions (Fedora, Arch, Debian), a desktop session, a real agent signed in.

## Windows

```bash
homebody daemon                        # the engine, in one terminal
homebody run my-agent -- my-agent      # in another: the program runs in its app container
```

No setup step and no administrator. Each protected app gets an app container of its own the first time it runs. Folders it may use (a home of its own and its workspace, under `C:\ProgramData\Homebody`) are opened to that container with `icacls`.

Not on Windows yet: interactive terminals (a protected program's output is passed on, but it gets no console of its own, so a full-screen agent will not draw), programs installed inside your user folder (an app container cannot read them; install for the whole machine), stopping a protected program from Homebody, looking inside connections, and the window.

**Checked** on a GitHub-hosted Windows machine (`app/tests/windows-fence.sh`): see the table in [app/README.md](../app/README.md) for the current count. **Not checked:** a real agent, Windows 10, an ordinary (non-administrator) account.
