#!/bin/bash
# prep-image.sh — offline patch of the Debian cloud image for a VM with NO network interface and a
# virtio console. Uses e2fsprogs (brew install e2fsprogs); no root, no mounting. Idempotent.
#
# Procedure, in this order (the order matters, see "lessons" below):
#   1. e2fsck -fy        replay the ext4 journal and leave a clean filesystem
#   2. debugfs           remove stale entries from earlier attempts, then make sure every kernel line in
#                        /boot/grub/grub.cfg carries the parameters in EXTRA (writing a minimal grub.cfg
#                        if Debian's is missing)
#   3. e2fsck -fn        confirm the result is clean
#
# Why these kernel parameters:
#   console=hvc0   kernel messages and a login prompt go to the virtio console sai-box attaches; systemd's
#                  getty generator spawns serial-getty@hvc0 for it automatically
#   systemd.mask=systemd-networkd-wait-online.service
#                  cloud-init's network stage otherwise waits for a network that never comes, so its
#                  write_files/runcmd never run (observed: log ends at "Waiting on external services")
#   apparmor=0     Debian's 6.12 cloud kernel oopsed in AppArmor find_attach() (NULL deref, "reboot is
#                  needed") when apt-get spawned its sandboxed methods inside this VM. The product ships
#                  its own guest image; for the spike AppArmor is off in the guest.
#
# Lessons from the first attempts, kept here so nobody repeats them:
#   - Editing with debugfs while the filesystem has an unreplayed journal (VM killed, not shut down)
#     loses the edits at the next mount: the journal replay overwrites the metadata blocks debugfs
#     touched. Always e2fsck first.
#   - debugfs "symlink" entries were rejected by the kernel ("deleted inode referenced") for the same
#     reason. We create no symlinks; the kernel command line does every job.
set -euo pipefail
IMG="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/build/image/disk.raw}"
E2="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
DBG="$E2/debugfs"; FSCK="$E2/e2fsck"
[ -x "$DBG" ] && [ -x "$FSCK" ] || { echo "e2fsprogs not found; brew install e2fsprogs"; exit 1; }
[ -f "$IMG" ] || { echo "image not found: $IMG"; exit 1; }

read -r OFF PARTUUID PARTIDX <<<"$(python3 -I - "$IMG" <<'PYEOF'
import struct, sys, uuid
f=open(sys.argv[1],"rb"); f.seek(512); h=f.read(92)
assert h[:8]==b"EFI PART", "not a GPT image"
elba=struct.unpack_from("<Q",h,72)[0]; n=struct.unpack_from("<I",h,80)[0]; sz=struct.unpack_from("<I",h,84)[0]
f.seek(elba*512); best=None
for i in range(n):
    e=f.read(sz)
    if e[:16]==b"\0"*16: continue
    first,last=struct.unpack_from("<QQ",e,32); size=(last-first+1)*512
    if best is None or size>best[0]: best=(size,first,str(uuid.UUID(bytes_le=e[16:32])),i)
print(best[1]*512, best[2], best[3])
PYEOF
)"
FS="$IMG?offset=$OFF"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
EXTRA="console=tty0 console=hvc0 consoleblank=0 systemd.mask=systemd-networkd-wait-online.service apparmor=0 arm64.nopauth"
echo "root filesystem: partition $((PARTIDX+1)), byte offset $OFF, PARTUUID $PARTUUID"

echo "--- 1. journal replay + check"
# e2fsck on a "?offset=" file cannot finish a journal replay (it re-opens the file without the offset
# and fails with "Bad magic number"), and the raw char device rejects the superblock write. The
# buffered block device of an hdiutil-attached image works, so that is what we use when we can.
# Exit codes: 0 clean, 1 corrected, 2 corrected + reboot advised, >=4 uncorrected/fatal.
DEV="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$IMG" 2>/dev/null | awk '/^\/dev\/disk[0-9]+ /{print $1; exit}' || true)"
set +e
if [ -n "$DEV" ]; then
  out="$("$FSCK" -fy "${DEV}s$((PARTIDX+1))" 2>&1)"; rc=$?
  hdiutil detach "$DEV" >/dev/null 2>&1
else
  echo "hdiutil attach failed; falling back to file offset (journal replay may not complete)"
  out="$("$FSCK" -fy "$FS" 2>&1)"; rc=$?
fi
echo "$out" | tail -1
set -e
[ "$rc" -lt 4 ] || { echo "e2fsck failed with exit $rc; refusing to edit a damaged filesystem"; echo "$out" | tail -5; exit 1; }
[ "$rc" -eq 0 ] && echo "filesystem was clean" || echo "filesystem repaired (e2fsck exit $rc)"

echo "--- 2. edits"
for p in /boot/grub/grub.cfg /etc/systemd/system/systemd-networkd-wait-online.service \
         /etc/systemd/system/getty.target.wants/serial-getty@hvc0.service; do
  d="$(dirname "$p")"; b="$(basename "$p")"
  line="$("$DBG" -R "ls -l $d" "$FS" 2>/dev/null | grep -F " $b" || true)"
  if [ -n "$line" ] && echo "$line" | awk '{exit !($1==0)}'; then
    "$DBG" -w -R "unlink $p" "$FS" 2>/dev/null; echo "removed stale entry $p"
  fi
done

if "$DBG" -R "dump /boot/grub/grub.cfg $TMP/grub.cfg" "$FS" 2>/dev/null && [ -s "$TMP/grub.cfg" ]; then
  python3 -I - "$TMP/grub.cfg" "$TMP/grub.cfg.new" "$EXTRA" <<'PYEOF'
import re, sys
src, dst, extra = sys.argv[1], sys.argv[2], sys.argv[3].split()
out=[]; patched=0
for line in open(src):
    m = re.match(r"^(\s*linux\s+\S+.*?)\s*$", line)
    if m:
        body = m.group(1); have = body.split()
        add = [p for p in extra if p not in have]
        if add:
            body += " " + " ".join(add); patched += 1
        line = body + "\n"
    out.append(line)
open(dst,"w").write("".join(out))
print(f"patched {patched} kernel lines" if patched else "grub.cfg already carries all parameters")
PYEOF
  if ! cmp -s "$TMP/grub.cfg" "$TMP/grub.cfg.new"; then
    "$DBG" -w -R "rm /boot/grub/grub.cfg" "$FS" 2>/dev/null
    "$DBG" -w -R "write $TMP/grub.cfg.new /boot/grub/grub.cfg" "$FS" 2>/dev/null
  fi
else
  KERNEL="$("$DBG" -R "ls /boot" "$FS" 2>/dev/null | tr ' ' '\n' | grep -m1 '^vmlinuz-')"
  INITRD="$("$DBG" -R "ls /boot" "$FS" 2>/dev/null | tr ' ' '\n' | grep -m1 '^initrd.img-')"
  [ -n "$KERNEL" ] && [ -n "$INITRD" ] || { echo "kernel/initrd not found in /boot"; exit 1; }
  cat > "$TMP/grub.cfg.new" <<EOF
# Minimal grub.cfg written by Sandbox-ai prep-image.sh (Debian's was lost; update-grub regenerates it)
set timeout=1
insmod part_gpt
insmod ext2
set root=(hd0,gpt1)
menuentry 'Debian GNU/Linux (Sandbox-ai Box)' {
	linux	/boot/$KERNEL root=PARTUUID=$PARTUUID ro $EXTRA
	initrd	/boot/$INITRD
}
EOF
  echo "wrote minimal grub.cfg for $KERNEL"
  "$DBG" -w -R "write $TMP/grub.cfg.new /boot/grub/grub.cfg" "$FS" 2>/dev/null
fi

echo "--- 3. verify"
"$DBG" -R "cat /boot/grub/grub.cfg" "$FS" 2>/dev/null | grep -m1 -E "^[[:space:]]*linux" | sed 's/^[[:space:]]*/  /'
"$FSCK" -fn "$FS" 2>&1 | tail -1
