# Security

Sandbox-ai exists to stop data leaving a computer, so a way around it is the most important kind of bug there is. Reports are very welcome.

## Reporting

Use GitHub's private reporting: the **Security** tab of this repository → **Report a vulnerability**. That reaches the maintainer without making the details public.

Please include what you ran, on which macOS version, and what got out or got read. A failing case for `app/tests/bypass-corpus.sh` is the ideal report.

This is a volunteer project with no paid support and no bounty. You will get an honest reply and credit in the fix if you want it.

## What counts

- A protected app reaches any network destination that is not trusted, or that you were not asked about.
- A protected app reads or writes files in your home folder, or in another protected app's space.
- A protected app answers its own prompt, changes its own rules, or reaches the control socket.
- The signed log can be changed, cut or reordered and still pass `sai verify`.
- Anything in settings, an app name or a project that ends up running as administrator during setup.
- Applying a project's changes writes outside the folder it was brought in from.

## What is already known and stated

These are limits of the current design, listed in the README, not new findings:

- Traffic to a destination you trust is not inspected. Data can leave inside it, and a trusted name can front for another site at the same provider.
- A kernel exploit from inside a protected app is not contained (no virtual machine yet).
- Local programs running as you, outside the fence, are trusted. They can change settings or talk to a gate.
- Windowed apps run as you behind one layer (the launch-time profile). They can read your files, and a windowed app that deliberately tries to break out has routes a separate user would close, such as writing into a folder another program syncs or runs. A way for a windowed app to reach the **network** directly, or to answer its own prompt, is still a finding.
- An agent's access to its own local servers rests on the packet filter alone, with a list refreshed twice a second. A way for an agent to reach a local server that is not its own is a finding.
- The build is not signed or notarized yet.
- Whether the packet filter rules come back after a restart has not been verified on a real restart. The app refuses to start protected apps when they are missing.

## Supported versions

Only the latest commit on `main`. There are no releases yet.
