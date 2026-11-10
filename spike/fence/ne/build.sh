#!/bin/bash
# build.sh — compile the Fence host app and its content-filter system extension with plain
# swiftc (no Xcode project), lay out the bundles, and code-sign them.
#
#   ./build.sh check              type-check both targets only (works with Command Line Tools)
#   ./build.sh                    build + ad-hoc sign (bundles are complete but macOS will NOT load the extension)
#   TEAM_ID=ABCDE12345 SIGN_ID="Developer ID Application: Name (ABCDE12345)" ./build.sh
#                                 build + sign for real; then copy build/Fence.app to /Applications
#
# Why the extension cannot load without TEAM_ID/SIGN_ID: the content-filter entitlement must be
# backed by a provisioning profile from a paid Apple Developer account, and system extensions
# must be signed with a Developer ID (or development) certificate from that account.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/build"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
ARCH="$(uname -m)"
TARGET="${ARCH}-apple-macos14.0"
TEAM_ID="${TEAM_ID:-}"
SIGN_ID="${SIGN_ID:--}"           # "-" = ad-hoc
APP_GROUP="${APP_GROUP:-${TEAM_ID:+$TEAM_ID.}com.sandboxai.fence}"
[ -z "$TEAM_ID" ] && APP_GROUP="group.com.sandboxai.fence"

SHARED=("$HERE/Shared/Policy.swift" "$HERE/../fencelog/Sources/Receipt.swift")
EXT_SRC=("$HERE/Extension/FilterDataProvider.swift" "$HERE/Extension/main.swift")
APP_SRC=("$HERE/App/main.swift")

common=(-target "$TARGET" -sdk "$SDK" -O -swift-version 5 -suppress-warnings)

if [ "${1:-}" = "check" ]; then
  echo "type-checking extension"
  swiftc -typecheck "${common[@]}" -module-name FenceExtension \
    -import-objc-header "$HERE/Extension/Bridging.h" -framework NetworkExtension -framework Network -framework Security \
    "${EXT_SRC[@]}" "${SHARED[@]}"
  echo "type-checking app"
  swiftc -typecheck "${common[@]}" -module-name Fence \
    -framework AppKit -framework NetworkExtension -framework SystemExtensions \
    "${APP_SRC[@]}" "${SHARED[@]}"
  echo "OK"
  exit 0
fi

rm -rf "$OUT"; mkdir -p "$OUT"
APP="$OUT/Fence.app"
EXT="$APP/Contents/Library/SystemExtensions/com.sandboxai.fence.extension.systemextension"
mkdir -p "$APP/Contents/MacOS" "$EXT/Contents/MacOS"

echo "compiling extension"
swiftc "${common[@]}" -module-name FenceExtension \
  -import-objc-header "$HERE/Extension/Bridging.h" -framework NetworkExtension -framework Network -framework Security -lbsm \
  "${EXT_SRC[@]}" "${SHARED[@]}" -o "$EXT/Contents/MacOS/com.sandboxai.fence.extension"

echo "compiling app"
swiftc "${common[@]}" -module-name Fence \
  -framework AppKit -framework NetworkExtension -framework SystemExtensions \
  "${APP_SRC[@]}" "${SHARED[@]}" -o "$APP/Contents/MacOS/Fence"

echo "writing plists and entitlements (APP_GROUP=$APP_GROUP)"
sed "s/@APP_GROUP@/$APP_GROUP/g" "$HERE/Extension/Info.plist" > "$EXT/Contents/Info.plist"
sed "s/@APP_GROUP@/$APP_GROUP/g" "$HERE/App/Info.plist" > "$APP/Contents/Info.plist"
sed "s/@APP_GROUP@/$APP_GROUP/g" "$HERE/Extension/Extension.entitlements" > "$OUT/Extension.entitlements"
sed "s/@APP_GROUP@/$APP_GROUP/g" "$HERE/App/App.entitlements" > "$OUT/App.entitlements"
echo -n "APPL????" > "$APP/Contents/PkgInfo"

# Provisioning profiles, when present, must be embedded before signing.
[ -f "$HERE/Resources/extension.provisionprofile" ] && cp "$HERE/Resources/extension.provisionprofile" "$EXT/Contents/embedded.provisionprofile"
[ -f "$HERE/Resources/app.provisionprofile" ] && cp "$HERE/Resources/app.provisionprofile" "$APP/Contents/embedded.provisionprofile"

echo "signing with identity: $SIGN_ID"
codesign --force --options runtime --timestamp=none --sign "$SIGN_ID" \
  --entitlements "$OUT/Extension.entitlements" "$EXT"
codesign --force --options runtime --timestamp=none --sign "$SIGN_ID" \
  --entitlements "$OUT/App.entitlements" "$APP"

echo
codesign -dv --entitlements - "$EXT" 2>&1 | grep -E "Identifier|TeamIdentifier|networkextension|content-filter" || true
echo
du -sh "$APP" | awk '{print "bundle size: " $1}'
if [ "$SIGN_ID" = "-" ]; then
  echo "ad-hoc signed: bundle is complete for inspection, but macOS will refuse to load the extension."
  echo "set TEAM_ID and SIGN_ID (Developer ID or development cert) to produce a loadable build."
else
  echo "copy to /Applications, launch, choose 'Install Fence extension', approve in System Settings, then 'Turn Private mode on'."
fi

# Ad-hoc builds cannot launch with the restricted Network Extension entitlement, so also
# produce an app-only variant (no extension, app-group entitlement only) for UI testing.
if [ "$SIGN_ID" = "-" ]; then
  UI="$OUT/FenceUI.app"
  rm -rf "$UI"; cp -R "$APP" "$UI"; rm -rf "$UI/Contents/Library"
  cat > "$OUT/ui-only.entitlements" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>com.apple.security.application-groups</key><array><string>$APP_GROUP</string></array>
</dict></plist>
PLIST
  codesign --force --sign - --entitlements "$OUT/ui-only.entitlements" "$UI" 2>/dev/null
  echo "UI-only test build (launchable, no extension): $UI"
fi
