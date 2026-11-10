# Box spike (macOS)

Goal, from `docs/product-spec.md`: a Linux VM with no network interface, a vsock channel to a host proxy that resolves DNS and enforces a hostname allowlist, a shared project folder, and an agent running inside. Success means cold start under three seconds, an agent completing a task, and a call to an untrusted host denied and logged.

Status 2026-11-09, macOS 26.5.1, Apple M4, Command Line Tools only:

| Piece | Where | State |
|---|---|---|
| VM host: EFI boot, no NIC, vsock, virtiofs share, console, optional display window | `Sources/main.swift` | Built, ad-hoc signed, **booted Debian 13 repeatedly**. |
| Host allowlist proxy with signed receipts | `Sources/Proxy.swift` | Tested standalone with curl and **from inside the guest**. |
| Guest forwarder + cloud-init seed + self-test + Claude Code install | `guest/` | **Works** through the self-test; Node/Claude Code install started but the Debian 13 kernel corrupted the filesystem mid-install. |
| Image prep (console, no network wait, AppArmor off) | `prep-image.sh` | Works; idempotent; replays the journal on an hdiutil block device first. |
| Guest image | `fetch-image.sh` | Debian genericcloud arm64, release-selectable (12 default, 13 tested), pristine copy kept, `reset` clones it instantly. |

### Results so far

| Measure | Result |
|---|---|
| Cold boot to login prompt, configured image, 2 vCPU 2 GiB | **3.0 s** wall clock from `vm.start` (3.03 s and 3.05 s on two images), including GRUB's 1 s menu timeout, which `GRUB_TIMEOUT=0` would remove; guest at multi-user by 2.1 s uptime |
| First boot with cloud-init doing all setup (growpart, update-grub, SSH keys, self-test) | 7.4 to 11 s to login prompt |
| Host process memory while the guest idles | about 70 MB RSS for sai-box (guest RAM is mapped separately) |
| Network interfaces in guest | `lo` only |
| Direct connection from guest, bypassing proxy | fails: no route, no resolver |
| DNS from guest | fails: no resolver |
| `https://webhook.site` via proxy | 403, tunnel refused, receipt written |
| `http://webhook.site` via proxy | 403 with body "Blocked by Sandbox-ai: webhook.site is not a trusted destination" |
| `http://deb.debian.org` via proxy | 200 |
| `apt-get update` via proxy | success, 13 s on the pristine image |
| Receipts | every decision recorded, chain verified (180 receipts across both spikes) |
| Stability | with `arm64.nopauth`: self-test and vsock stress pass, no kernel warnings; without it, pointer-authentication faults within 1–5 minutes on both Debian 12 and 13 kernels |
| Claude Code | 2.1.319 installed inside the guest via apt (Node 18) and npm through the proxy |

### Guest instability: root cause found, pointer authentication

The first working image crashed within 17 to 47 seconds on every boot. Part of that was self-inflicted (debugfs edits against an unreplayed journal; fixed, see the rules below). But pristine images crashed too, on Debian 13's 6.12 kernel and on Debian 12's 6.1 kernel alike, with one vCPU or two, with or without the memory balloon or the shared folder:

| Uptime | Process | Symptom |
|---|---|---|
| 67 s | `jbd2/vda1-8` | jumped to a NULL function pointer during fsync |
| 62 s, 182 s | kernel mm | `BUG: Bad rss-counter state`, `non-zero pgtables_bytes on freeing mm` |
| 180 s | `mandb` | `ext4_lookup: iget: checksum invalid`, journal aborted |
| 208 s | userspace | `Oops ... Code: bad PC value` |
| 316 s | `kworker` in `__switch_to` | **`Oops - FPAC`** at an `autiasp` instruction |

The last one names the cause. FPAC is an arm64 **pointer authentication** failure: the kernel signs return addresses and function pointers with per-task keys, and a failed check at `__switch_to` means the keys the guest expects are not the keys it finds after a context switch. Every other symptom follows from that: an authenticated pointer that fails the check becomes a garbage or NULL address, so you get NULL jumps, bad PCs and corrupted accounting in random subsystems. The host is macOS 26.5.1 Virtualization.framework on an Apple M4.

Fix: boot the guest with `arm64.nopauth`, which disables pointer authentication in the guest kernel. With it, the same image passed the self-test, streamed three 11 MB downloads through vsock and the proxy at about 150 Mbit/s each, and showed **zero kernel warnings** past the point where every earlier run had failed. `prep-image.sh` and the cloud-init GRUB drop-in both carry the parameter now.

Update, later the same night: `arm64.nopauth` was not the whole story. A guest with it still wedged after 6.5 minutes (an "undefined instruction" oops in kernel code, then RCU stalls in `__switch_to`). At that moment the host had **17.4 GB of its 18 GB swap in use**, 6.7 GB compressed and about 160 MB free on a 16 GB Mac. Stale or corrupted guest pages under that kind of host paging explain every signature seen, PAC included. Mitigation for the spike: a 1 GB guest and fewer host apps. For the product: pin or at least size guest memory conservatively, watch host memory pressure, and treat guest kernel faults as a host-health signal rather than a guest bug.

Open question for the product: whether this is a macOS 26.5.1 regression, an M4-specific issue, host memory pressure alone, or a combination. Lima and Docker run Linux guests on Apple silicon daily, so a broad breakage would be known; our own guest image should carry `arm64.nopauth` until the cause is pinned down, and the bypass corpus should include a pointer-authentication stress test.

Rules that fell out of this, now encoded in the scripts:
- Replay the journal before editing an image: e2fsck on the partition's block device from `hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage`, not on a `?offset=` file and not on the raw character device.
- Never create symlinks with debugfs. The kernel command line does every job that needed one.
- Prefer shutting the guest down cleanly. A killed VM leaves a dirty journal; the prep script handles it, but data written in the last seconds is lost.
- `apparmor=0` stays for the spike (the AppArmor oops was pointer authentication too); the product ships its own guest image.
- Debian 12's initramfs lacks the virtio console module, so `console=tty0` is added as well; kernel messages reach the display window and the virtio console takes over once the root filesystem is up.

## How it fits together

```
 guest (no NIC)                              host (macOS)
 ┌──────────────────────────┐                ┌───────────────────────────────┐
 │ apt / curl / node        │                │ sai-box                       │
 │   http_proxy=127.0.0.1:3128 ─► sai-forward.py ─ vsock 2:3128 ─► ProxyConnection │
 │                          │                │   canonicalise host           │
 │ /work  ◄── virtiofs "sai" ─────────────── │   allowlist → allow / deny    │
 └──────────────────────────┘                │   DNS + connect on host       │
                                             │   receipt → fencelog chain    │
                                             └───────────────────────────────┘
```

The guest cannot resolve names or open sockets to the internet: there is no interface. Its only way out is HTTP-proxy semantics over vsock, so every destination is a hostname the host sees, decides on, and records.

## Proxy test without a VM

```bash
./build/sai-box proxy --port 3129 --allow example.com --allow "*.debian.org"
```

Results on 2026-11-09 with `curl -x http://127.0.0.1:3129`:

| Request | Result | Rule |
|---|---|---|
| `https://example.com/` | 200 | allow:example.com |
| `https://webhook.site/abc` | 403, tunnel refused | default-deny |
| `http://deb.debian.org/.../Release` | 200 | allow:*.debian.org |
| `http://93.184.216.34/` | 403 | deny:malformed-host (IP literals bypass allowlists, so they are refused) |
| `https://api.anthropic.com/` | 403 | default-deny |
| `http://security.debian.org/...` | 200 | allow:*.debian.org |

Six receipts, chain intact, all signatures valid. Denied requests record `-` for the IP because resolution never happens for them.

## Running the VM

```bash
./fetch-image.sh
```

```bash
./prep-image.sh
```

Reset the working image to pristine at any time with `./fetch-image.sh reset`, then run `./prep-image.sh` again. Shut the guest down cleanly (`poweroff` at the console) or let `prep-image.sh` replay the journal before editing the image.

```bash
./guest/make-seed.sh
```

```bash
./build/sai-box run --disk build/image/disk.raw --seed build/seed.iso --share "$PWD/../.." --allow "*.debian.org" --allow registry.npmjs.org --allow nodejs.org --allow api.anthropic.com
```

First boot runs cloud-init from the seed: sets root password `box` for the serial console, installs the forwarder as a systemd service, points apt and the shell environment at `127.0.0.1:3128`, mounts the shared folder at `/work`, masks the network wait, and prints `sai-ready` on the console. `sai-box` reports the time from start to the login prompt. Ctrl-] stops the VM.

Then inside the guest:

```
apt-get update                       # goes through the proxy; allowed
curl -sS https://webhook.site/x      # 403 from the proxy; denied and logged on the host
```

## Claude Code inside the Box: what was proven

With the allowlist `*.debian.org, registry.npmjs.org, api.anthropic.com, claude.ai, console.anthropic.com, platform.claude.com`:

- Claude Code 2.1.319 installed inside the guest via apt (Node 18) and npm, entirely through the proxy.
- Run from the Box Console web page with an API key, `claude -p` connected to `api.anthropic.com` four times (allowed receipts) and nothing else left the guest. The exfiltration button's `curl https://webhook.site/…` was refused with 403 and logged.
- The task itself did not complete: the guest filesystem went read-only at 139 s after a disk read returned an inode with a bad checksum, on a host that was thrashing (17 GB of 18 GB swap in use, ~150 GB swapped in and ~160 GB out since boot). See the instability section; the Box's egress design is not what failed.

## Box Console (web UI)

`python3 spike/box/ui/server.py` serves http://127.0.0.1:8787: Box status, trusted destinations, a live feed of receipts, a Claude Code runner (API key kept in memory, passed to the guest over the vsock SSH tunnel only), an exfiltration button and a receipt-chain verify. Stdlib only. It is the first working sketch of the product's Feed and Proof surfaces on real data.

## SSH into the Box (no NIC involved)

`sai-box run --ssh-port 2222 ...` tunnels `127.0.0.1:2222` on the host to vsock port 22 in the guest, where a tiny Python forwarder hands the stream to sshd on loopback. cloud-init installs a key pair made only for this Box (`build/box_ed25519`). Then:

```bash
./spike/box/box-ssh.sh
```

gives a root shell inside the guest, or `./box-ssh.sh 'command'` runs one command. SSH login shells inherit the proxy environment, so `curl` and `apt` inside the session go through the allowlist exactly like everything else. This is the host→guest command channel the product needs; it replaces typing into the serial console, which did not accept input from the app's terminal panel.

## What is deliberately not in the spike

- No TLS inspection: CONNECT is passthrough. Content rules come with the product's baked-in CA.
- No warm VM, suspend or ballooning tuning. Cold boot time is measured as-is.
- No GPU or model inside the guest. On Apple silicon the model stays a fenced host daemon (see the Fence spike).
- The guest root password and console login are spike conveniences, not product behaviour.
