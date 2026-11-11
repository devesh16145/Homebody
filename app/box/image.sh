#!/bin/bash
# image.sh — make the Box's Linux disk.
#
#   image.sh <folder> fetch             download Debian 12 for arm64 from Debian, check it, keep it as pristine.raw
#   image.sh <folder> use <disk.raw>    use an image already on this Mac as pristine.raw instead (cloned, not moved)
#   image.sh <folder> seed              make the key pair and the first-start instructions (seed.iso)
#   image.sh <folder> base              a fresh base.raw from pristine.raw, with its kernel line prepared
#
# The base still has to be started once so the guest configures itself (homebody does that:
# `homebody box setup`). After that every run of the Box starts from an instant copy of the
# base, so nothing a run does can damage the next one.
#
# Needs e2fsprogs for the "base" step (brew install e2fsprogs): the image's start-up line is
# edited without starting it. A ready-made base published with Homebody's releases will remove
# that need; until then the Box is for people who are comfortable with Homebrew.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIR="${1:?folder}"; CMD="${2:?fetch | use | seed | base}"
mkdir -p "$DIR"; chmod 700 "$DIR"
NAME="debian-12-genericcloud-arm64"
BASE_URL="https://cloud.debian.org/images/cloud/bookworm/latest"
EXTRA="console=tty0 console=hvc0 consoleblank=0 systemd.mask=systemd-networkd-wait-online.service apparmor=0 arm64.nopauth"

case "$CMD" in
fetch)
  [ -f "$DIR/pristine.raw" ] && { echo "already downloaded"; exit 0; }
  cd "$DIR"
  echo "downloading $BASE_URL/$NAME.tar.xz"
  curl -fL --progress-bar -o "$NAME.tar.xz" "$BASE_URL/$NAME.tar.xz"
  curl -fsSL -o SHA512SUMS "$BASE_URL/SHA512SUMS"
  echo "checking it against Debian's published checksum"
  grep " $NAME.tar.xz\$" SHA512SUMS | shasum -a 512 -c -
  tar -xJf "$NAME.tar.xz"; rm -f "$NAME.tar.xz"
  truncate -s 10G disk.raw
  mv disk.raw pristine.raw; chmod 444 pristine.raw
  ;;
use)
  SRC="${3:?path to a disk image}"
  rm -f "$DIR/pristine.raw"
  cp -c "$SRC" "$DIR/pristine.raw" 2>/dev/null || cp "$SRC" "$DIR/pristine.raw"
  chmod 444 "$DIR/pristine.raw"
  ;;
seed)
  KEY="$DIR/box_ed25519"
  # A key pair used only to send commands into this Box. Not one of yours.
  [ -f "$KEY" ] || ssh-keygen -q -t ed25519 -N "" -C "homebody-box" -f "$KEY"
  TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
  B64="$(base64 < "$HERE/guest/forward.py" | tr -d '\n')"
  PUB="$(cat "$KEY.pub")"
  sed -e "s|@FORWARD_B64@|$B64|" -e "s|@SSH_PUBKEY@|$PUB|g" -e "s|@UID@|$(id -u)|" "$HERE/guest/user-data" > "$TMP/user-data"
  printf 'instance-id: homebody-box-%s\nlocal-hostname: box\n' "$(date +%s)" > "$TMP/meta-data"
  rm -f "$DIR/seed.iso"
  hdiutil makehybrid -iso -joliet -default-volume-name cidata -o "$DIR/seed.iso" "$TMP" >/dev/null
  echo "seed: $DIR/seed.iso"
  ;;
base)
  [ -f "$DIR/pristine.raw" ] || { echo "no image yet: run fetch (or use) first" >&2; exit 1; }
  E2="${E2FSPROGS:-/opt/homebrew/opt/e2fsprogs/sbin}"
  DBG="$E2/debugfs"; FSCK="$E2/e2fsck"
  [ -x "$DBG" ] && [ -x "$FSCK" ] || { echo "The Box's disk is prepared with e2fsprogs. Install it with:  brew install e2fsprogs" >&2; exit 1; }
  IMG="$DIR/base.raw"
  rm -f "$IMG"
  cp -c "$DIR/pristine.raw" "$IMG" 2>/dev/null || cp "$DIR/pristine.raw" "$IMG"
  chmod 600 "$IMG"
  # Where the root filesystem starts inside the image (the largest partition of its table).
  read -r OFF PARTIDX <<<"$(python3 -I - "$IMG" <<'PYEOF'
import struct, sys
f = open(sys.argv[1], "rb"); f.seek(512); h = f.read(92)
assert h[:8] == b"EFI PART", "not a GPT image"
elba = struct.unpack_from("<Q", h, 72)[0]; n = struct.unpack_from("<I", h, 80)[0]; sz = struct.unpack_from("<I", h, 84)[0]
f.seek(elba * 512); best = None
for i in range(n):
    e = f.read(sz)
    if e[:16] == b"\0" * 16: continue
    first, last = struct.unpack_from("<QQ", e, 32); size = (last - first + 1) * 512
    if best is None or size > best[0]: best = (size, first, i)
print(best[1] * 512, best[2])
PYEOF
)"
  FS="$IMG?offset=$OFF"
  TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
  # Check the filesystem first: editing one that still has unfinished writes loses the edit.
  DEV="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$IMG" 2>/dev/null | awk '/^\/dev\/disk[0-9]+ /{print $1; exit}' || true)"
  set +e
  if [ -n "$DEV" ]; then "$FSCK" -fy "${DEV}s$((PARTIDX+1))" >/dev/null 2>&1; rc=$?; hdiutil detach "$DEV" >/dev/null 2>&1; else "$FSCK" -fy "$FS" >/dev/null 2>&1; rc=$?; fi
  set -e
  [ "$rc" -lt 4 ] || { echo "the image's filesystem is damaged (e2fsck $rc)" >&2; exit 1; }
  "$DBG" -R "dump /boot/grub/grub.cfg $TMP/grub.cfg" "$FS" 2>/dev/null
  [ -s "$TMP/grub.cfg" ] || { echo "the image has no start-up file to prepare" >&2; exit 1; }
  python3 -I - "$TMP/grub.cfg" "$TMP/grub.cfg.new" "$EXTRA" <<'PYEOF'
import re, sys
src, dst, extra = sys.argv[1], sys.argv[2], sys.argv[3].split()
out = []
for line in open(src):
    m = re.match(r"^(\s*linux\s+\S+.*?)\s*$", line)
    if m:
        body = m.group(1); have = body.split()
        line = body + "".join(" " + p for p in extra if p not in have) + "\n"
    out.append(line)
open(dst, "w").write("".join(out))
PYEOF
  if ! cmp -s "$TMP/grub.cfg" "$TMP/grub.cfg.new"; then
    "$DBG" -w -R "rm /boot/grub/grub.cfg" "$FS" 2>/dev/null
    "$DBG" -w -R "write $TMP/grub.cfg.new /boot/grub/grub.cfg" "$FS" 2>/dev/null
  fi
  "$DBG" -R "cat /boot/grub/grub.cfg" "$FS" 2>/dev/null | grep -q "arm64.nopauth" || { echo "the start-up line could not be prepared" >&2; exit 1; }
  echo "base: $IMG"
  ;;
*) echo "unknown step: $CMD" >&2; exit 2 ;;
esac
