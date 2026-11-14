# The Box: an agent in a machine with no network

The fence keeps a program on your Mac from reaching the network except through its gate. The Box goes one step further for people who want it: the agent does not run on your Mac's system at all. It runs in a small Linux machine, using the virtualisation that is part of macOS, that has

- **no network interface.** There is nothing to send a packet on. Its one way out is a private channel to a helper on the Mac, which hands every connection to the app's gate. So the same rules, the same questions and the same signed log apply, and everything shows in Activity under the agent's name.
- **no name server.** Names are looked up by the gate, outside.
- **one folder of yours**, at `/work`, and nothing else of the Mac.

Every start is from an instant copy of a prepared disk. Whatever a run does to the machine is gone at the next start; only the shared folder is yours and stays.

Apple silicon Macs only.

## In the app

On the page of an agent that can go in the Box (Claude Code, Codex CLI, Gemini CLI) there is a card called **The Box**. It always shows the one next step:

1. **Set up the Box.** Once. Downloads Debian from Debian (about 200 MB), checks it against Debian's published checksum, prepares the disk and starts it once. A few minutes, mostly the download; about 3 GB of disk when an agent is in it.
2. **Put it in.** Once per agent. Installs the agent into the prepared disk, fetching everything through that agent's gate. About two minutes.
3. **Open in the Box.** Starts the Box (about four seconds) and opens a Terminal window inside it with the agent running. **Stop** throws the Box's disk away; the shared folder is kept.

The two long steps run in the background and say what they are doing; you can leave the page. The Box sees the agent's own folder (`/Users/Shared/Homebody/<agent>`), which is where projects you bring in for it live.

## From the command line

```bash
homebody box setup                 # once: makes the disk (downloads Debian, about 200 MB)
homebody box install claude-code   # once: puts the agent into the disk
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

- **Three agents can be put into it:** `homebody box install claude-code` (or `codex`, or `gemini`) installs the agent into the prepared disk, fetching everything through that agent's gate, so every later start has it. Only Claude Code has been installed this way so far. Other agents are installed by hand inside a running Box and are gone at the next start.
- **One Box at a time.** Starting it for a second agent means stopping it for the first.
- **The card opens the agent's whole folder.** Starting the Box on a single project is command line only (`homebody box start <agent> <project>`).
- **API keys are typed inside it** like anywhere else. Keeping keys outside and adding them at the gate is not built.
- **No window inside it.** It is for command-line agents; the app opens a Terminal window into it.
- **Set-up needs the internet once** and cannot be resumed: a download that is interrupted starts again.

## How the disk is made, and why no extra software is needed

Debian's image has to be started with a few settings: a console the helper can record, no two-minute wait for a network that is not there, and one (`arm64.nopauth`) without which this kernel does not start under Apple's virtualisation on newer Macs. Those settings live in a file inside the image's Linux filesystem, which macOS cannot read. The first version edited that file with Homebrew's `e2fsprogs`.

Now Homebody does it itself (`app/crates/core/src/boxdisk.rs`). The image's small start-up partition is in a format every system reads, and holds a three-line file that only says "go and read the real configuration". Homebody replaces that file, in place, with one that starts the newest kernel in `/boot` directly with the Box's settings, and falls back to the image's own configuration if anything does not fit. Two things are read from the image (its partition table and the Linux filesystem's identifier) and one cluster plus one length field are written. Nothing is mounted. It refuses an image whose start-up file is not the one Debian ships.

The rest uses what macOS ships: `curl`, `shasum`, `tar`, `ssh-keygen`, `hdiutil`.

## What has been run

`app/tests/box.sh`, 29 cases, on one Apple M4 Mac (the last seven added with Look inside and kept keys, see below): no interface but loopback, direct connections (IPv4, IPv6, UDP) have nowhere to go, no name lookup, the host answers on the gate's channel only, trusted places reached and untrusted ones refused and shown in Activity, none of the Mac's folders visible, files written to the shared folder arrive as yours, a command that stays quiet for a while is not cut off, and a new start does not have the last run's files. Run again, 22 of 22, on a disk made the new way (no Homebrew), through the installed app.

The app's path was run against the installed app's engine with the same calls the card makes: set-up in the background (8 seconds with Debian already downloaded; the download itself was not repeated), Claude Code put in through its gate (two minutes, version 2.1.322), then "Open in the Box", which started the Box and opened a Terminal window with `claude` running inside it. The kernel's start-up line inside carried every setting. Claude Code was not signed in or used for work there. The card itself was checked in a browser preview against simulated data, in every state, in light and dark and at a narrow width; it was not clicked inside the real app's window. Codex CLI and Gemini CLI have recipes but have not been put in. The disk preparation has unit tests against a synthetic image and was run on Debian's real image; it has been run on one Debian image (12, kernel 6.1.0-54), on one Mac.

An earlier experiment on the same Mac saw the Linux guest crash under heavy memory pressure (17 GB of swap in use). It has been stable in every run since, but it has not been run for hours at a time.

## Looking inside, and keys that stay out of the Box

When Look inside is on for an agent, a program started in the Box (from the app, or with `homebody box run`) is told about Homebody's certificate the same way a protected program on the Mac is, so its connections to trusted places can be looked inside: single addresses switched off, secrets stopped, and so on.

A key Homebody keeps for the agent ([keys.md](keys.md)) reaches the Box only as a stand-in. The real key is put in at the gate, on the Mac. It is never in the machine: not in its environment, not on its disk.

While an agent is being put in the Box, its package tools fetch through the gate without being looked inside; they are not told about the certificate.

Checked in `app/tests/box.sh`: a program in the Box accepts the certificate and what it asked for shows in Activity; the Box holds a stand-in and the key is nowhere in it; a request with the stand-in goes to the place the key is for and is stopped anywhere else.
