#!/bin/bash
# make-seed.sh — build the cloud-init NoCloud seed ISO (volume label "cidata") with hdiutil.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$HERE/../build/seed.iso}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
B64="$(base64 < "$HERE/sai-forward.py" | tr -d '\n')"
# A key pair used only to reach this Box over the vsock SSH bridge (not the user's own key).
KEY="$HERE/../build/box_ed25519"
[ -f "$KEY" ] || ssh-keygen -q -t ed25519 -N "" -C "sandbox-ai-box" -f "$KEY"
PUB="$(cat "$KEY.pub")"
sed -e "s|@SAI_FORWARD_B64@|$B64|" -e "s|@SSH_PUBKEY@|$PUB|" "$HERE/user-data" > "$TMP/user-data"
printf 'instance-id: sai-box-%s\nlocal-hostname: box\n' "$(date +%s)" > "$TMP/meta-data"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil makehybrid -iso -joliet -default-volume-name cidata -o "$OUT" "$TMP" >/dev/null
echo "seed: $OUT ($(du -h "$OUT" | cut -f1))"
