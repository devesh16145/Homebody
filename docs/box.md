# The Box: an agent in a machine with no network

The fence keeps a program on your Mac from reaching the network except through its gate. The Box goes one step further for people who want it: the agent does not run on your Mac's system at all. It runs in a small Linux machine, using the virtualisation that is part of macOS, that has

- **no network interface.** There is nothing to send a packet on. Its one way out is a private channel to a helper on the Mac, which hands every connection to the app's gate. So the same rules, the same questions and the same signed log apply, and everything shows in Activity under the agent's name.
- **no name server.** Names are looked up by the gate, outside.
- **one folder of yours**, at `/work`, and nothing else of the Mac.

Every start is from an instant copy of a prepared disk. Whatever a run does to the machine is gone at the next start; only the shared folder is yours and stays.

Apple silicon Macs only.

```bash
homebody box setup                 # once: makes the disk (downloads Debian, about 350 MB)
homebody box start claude-code     # the Box sees only that agent's folder
homebody box start claude-code my-project   # or only one project you brought in
homebody box run -- <command...>   # run something inside it
homebody box stop                  # stops it and throws its disk away
homebody box                       # how it is
```

Starting takes about four seconds.

## What it is good for

The fence's limit is that a protected program shares the Mac's kernel: a kernel flaw could let it out. The Box puts a second kernel and a hardware boundary in between. It is the answer for running an agent you trust less, or code an agent fetched.

## What this first version does not do

- **It needs Homebrew's `e2fsprogs` to make the disk** (`brew install e2fsprogs`), because the image's start-up line is edited before its first start. A ready-made disk published with Homebody's releases will remove that; until then the Box is for people comfortable with Homebrew.
- **Agents are not installed in it for you.** The prepared disk is plain Debian with Python. Installing an agent means starting the Box, installing it there through the gate, and doing so again after the next start, because each start is a fresh copy. Keeping installed agents in the prepared disk is the next piece of work.
- **One Box at a time**, started from the command line. There is no button for it in the app yet.
- **API keys are typed inside it** like anywhere else. Keeping keys outside and adding them at the gate is not built.
- **No window.** It is for command-line agents.

## What has been run

`app/tests/box.sh`, 21 cases, on one Apple M4 Mac: no interface but loopback, direct connections (IPv4, IPv6, UDP) have nowhere to go, no name lookup, the host answers on the gate's channel only, trusted places reached and untrusted ones refused and shown in Activity, none of the Mac's folders visible, files written to the shared folder arrive as yours, and a new start does not have the last run's files.

An earlier experiment on the same Mac saw the Linux guest crash under heavy memory pressure (17 GB of swap in use). It has been stable in every run since, but it has not been run for hours at a time.
