#!/bin/bash
# Build Homebody and pack it into a disk image people can download:
# the app, and a shortcut to the Applications folder to drag it onto.
#
#   scripts/make-dmg.sh            build, then pack
#   scripts/make-dmg.sh --no-build pack what is already built
#
# Signing and notarization happen inside `tauri build` when the usual Apple variables are set
# (APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID). Without them the app is
# unsigned and macOS will warn people who download it.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != "--no-build" ]; then
  npx tauri build
fi

APP="target/release/bundle/macos/Homebody.app"
[ -d "$APP" ] || { echo "No app at $APP. Run without --no-build." >&2; exit 1; }
[ -x "$APP/Contents/MacOS/sai" ] || { echo "The command line tool is missing from the app." >&2; exit 1; }

VERSION="$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - "$APP/Contents/Info.plist")"
ARCH="$(uname -m)"
OUT="target/release/bundle/dmg"
DMG="$OUT/Homebody-$VERSION-$ARCH.dmg"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$OUT"
/usr/bin/ditto "$APP" "$STAGE/Homebody.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
/usr/bin/hdiutil create -quiet -volname "Homebody" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"

echo "$DMG"
/usr/bin/shasum -a 256 "$DMG" | awk '{print "sha256 " $1}'
