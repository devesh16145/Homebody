# Security

Homebody exists to stop data leaving a computer, so a way around it is the most important kind of bug there is. Reports are very welcome.

## Reporting

Use GitHub's private reporting: the **Security** tab of this repository → **Report a vulnerability**. That reaches the maintainer without making the details public.

Please include what you ran, on which macOS version, and what got out or got read. A failing case for `app/tests/bypass-corpus.sh` is the ideal report.

This is a volunteer project with no paid support and no bounty. You will get an honest reply and credit in the fix if you want it.

## What counts

- A protected app reaches any network destination that is not trusted, or that you were not asked about.
- A protected app reads or writes files in your home folder, or in another protected app's space.
- A protected app answers its own prompt, changes its own rules, or reaches the control socket.
- A protected app sends more to a trusted place in an hour than the limit set for it, beyond the one piece that was being passed on when the limit was reached (a few kilobytes; up to 4 MB for an app with looking inside on and private folders in use, because a request is then held and passed whole).
- A file in a project (`.homebody.json`) gets a place trusted without the user ticking it.
- The signed log can be changed, cut or reordered and still pass `homebody verify`.
- Anything in settings, an app name or a project that ends up running as administrator during setup.
- With looking inside on: a request that reaches a site other than the approved one, a request the gate passed on without being able to follow it, anything after "?" or any content ending up in the log, the local certificate authority's key being readable by a protected app, any spelling of an address that is switched off reaching the real server, a secret of a recognised shape, sent as it is in the content of a request or of a web socket message, reaching the real server whole or in part, or anything at all reaching the real server on a connection the gate has stopped following.
- Applying a project's changes writes outside the folder it was brought in from.
- On Linux or Windows: any of the above for a protected app there. The fences are described in [docs/platforms.md](docs/platforms.md).
- In the Box: anything inside it reaches the network other than through the gate, sees a folder of the Mac other than the shared one, or leaves something behind that the next start still has.
- A browser profile kept for AI sites reads a file in your home folder outside Downloads, or reaches a place that is not switched on without asking.
- Fleet: a computer applies rules that the fleet it joined did not sign, or goes back to an older version of them; the server accepts log records that the computer they are filed under did not sign, or that do not continue its log; someone does something in the console their role does not allow, or without signing in; a page on another website makes a signed-in person's browser change something.
- `homebody ci` hands a protected run a variable of the job's environment that was not named with `--env`, or leaves a place trusted after the run.

## What is already known and stated

These are limits of the current design, listed in the README, not new findings:

- Traffic to a destination you trust is not examined for content. With looking inside off (the default), a trusted name can also front for another site at the same provider.
- A kernel exploit from inside a protected app is not contained (no virtual machine yet).
- Local programs running as you, outside the fence, are trusted. They can change settings or talk to a gate.
- Windowed apps run as you behind one layer (the launch-time profile). They can read your files, and a windowed app that deliberately tries to break out has routes a separate user would close, such as writing into a folder another program syncs or runs. A way for a windowed app to reach the **network** directly, or to answer its own prompt, is still a finding.
- An agent's access to its own local servers rests on the packet filter alone, with a list refreshed twice a second. A way for an agent to reach a local server that is not its own is a finding.
- Secrets are recognised by shape only. One that has been encoded, compressed or interleaved with other text is not recognised. That is a known limit, not a finding.
- Private folders are matched by exact lines of 40 characters or more, three in one request. Reworded, encoded, shortened or split text is not recognised. That is a known limit; three exact lines from an indexed file reaching the real server is a finding.
- An hourly limit on a trusted place bounds what is sent there; it does not stop sending below the limit, and it does not count what is received.
- What a server sends back over a web socket is not read.
- The build is not signed or notarized yet.
- An organisation's fleet server sees what joined computers' apps reached, and its rules come before the user's own, including rules that trust a place the user had blocked. That is what joining means and the app says so first; it is not a finding.
- Linux: firewall rules removed by an administrator come back within 20 seconds, not at once.
- Windows: a protected program gets no console of its own and cannot be stopped from Homebody.
- The fleet server's console page allows inline script (it is one self-contained file); everything else is forbidden by its content security policy.
- Protected apps are not started again by themselves after a restart. The packet filter rules are restored at boot (verified on one real restart, macOS 26.5); the app refuses to start protected apps whenever they are missing.

## When something is fixed

Every way around that is fixed is written up in [docs/advisories](docs/advisories/README.md): what it was, which versions had it, and the test that now covers it. Fixes are never made quietly. Reports are acknowledged within a week, and details are published once a fix is available or after 90 days, whichever comes first, unless you ask for longer.

## Supported versions

Only the latest commit on `main`. There are no releases yet.
