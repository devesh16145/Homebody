#!/bin/bash
# image.sh — fetch the Box's Linux disk and make its first-start instructions.
#
#   image.sh <folder> fetch             download Debian 12 for arm64 from Debian, check it, keep it as pristine.raw
#   image.sh <folder> use <disk.raw>    use an image already on this Mac as pristine.raw instead (cloned, not moved)
#   image.sh <folder> seed              make the key pair and the first-start instructions (seed.iso)
#
# Homebody does the rest itself (`homebody box setup`, or "Set up the Box" in the app): it
# copies pristine.raw to base.raw, sets the image's start-up line (crates/core/src/boxdisk.rs),
# and starts the base once so the guest configures itself. After that every run of the Box
# starts from an instant copy of the base, so nothing a run does can damage the next one.
#
# Uses only what macOS ships: curl, shasum, tar, ssh-keygen, hdiutil.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIR="${1:?folder}"; CMD="${2:?fetch | use | seed}"
mkdir -p "$DIR"; chmod 700 "$DIR"
NAME="debian-12-genericcloud-arm64"
BASE_URL="https://cloud.debian.org/images/cloud/bookworm/latest"

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
*) echo "unknown step: $CMD" >&2; exit 2 ;;
esac
