#!/bin/bash
# fetch-image.sh — download a Debian arm64 generic cloud image, extract disk.raw, and keep a pristine copy
# so the working image can be reset instantly.
# Source: https://cloud.debian.org/images/cloud/<release>/latest/  (official Debian cloud team builds)
# Each image is a ~200 MB .tar.xz that unpacks to a 2–3 GB sparse raw disk, grown here to 10 GB.
#
#   RELEASE=bookworm ./fetch-image.sh   Debian 12, kernel 6.1 (default: stable on Apple's hypervisor in our runs)
#   RELEASE=trixie   ./fetch-image.sh   Debian 13, kernel 6.12.115 (corrupted memory and ext4 within minutes here)
#   ./fetch-image.sh reset              throw away disk.raw and clone it again from the pristine copy
#
# build/image is a symlink to build/image-<release>, so the other scripts need no release argument.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RELEASE="${RELEASE:-bookworm}"
case "$RELEASE" in bookworm) VER=12 ;; trixie) VER=13 ;; *) echo "unknown release $RELEASE"; exit 1 ;; esac
IMG_DIR="$HERE/build/image-$RELEASE"
NAME="debian-$VER-genericcloud-arm64"
URL="https://cloud.debian.org/images/cloud/$RELEASE/latest/$NAME.tar.xz"
SUMS="https://cloud.debian.org/images/cloud/$RELEASE/latest/SHA512SUMS"
mkdir -p "$IMG_DIR"
# Keep an existing plain directory from before this script was release-aware.
if [ -d "$HERE/build/image" ] && [ ! -L "$HERE/build/image" ]; then mv "$HERE/build/image" "$HERE/build/image-trixie"; fi
ln -sfn "image-$RELEASE" "$HERE/build/image"
cd "$IMG_DIR"

clone_working() {
  rm -f disk.raw
  # APFS clonefile: copy-on-write, no extra space until blocks diverge.
  cp -c disk.raw.pristine disk.raw 2>/dev/null || cp disk.raw.pristine disk.raw
  chmod 644 disk.raw
  echo "disk.raw reset from pristine copy"
}

if [ "${1:-}" = "reset" ]; then
  [ -f disk.raw.pristine ] || { echo "no pristine copy; run without 'reset' first"; exit 1; }
  clone_working; exit 0
fi

if [ ! -f disk.raw.pristine ]; then
  echo "downloading $URL"
  curl -fL --progress-bar -o "$NAME.tar.xz" "$URL"
  curl -fsSL -o SHA512SUMS "$SUMS"
  echo "verifying checksum"
  grep " $NAME.tar.xz\$" SHA512SUMS | shasum -a 512 -c -
  echo "extracting"
  tar -xJf "$NAME.tar.xz"
  rm -f "$NAME.tar.xz"
  truncate -s 10G disk.raw          # cloud-init grows the root partition on first boot
  mv disk.raw disk.raw.pristine
  chmod 444 disk.raw.pristine
fi
[ -f disk.raw ] || clone_working
ls -la disk.raw disk.raw.pristine
du -h disk.raw disk.raw.pristine | awk '{print "on-disk: " $2 " " $1}'
