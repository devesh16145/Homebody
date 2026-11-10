#!/bin/bash
# build.sh — compile sai-box with plain swiftc and sign it with the virtualization entitlement.
# The entitlement is unrestricted: ad-hoc signing is enough to run VMs locally.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
TARGET="$(uname -m)-apple-macos14.0"
mkdir -p "$HERE/build"
swiftc -O -swift-version 5 -target "$TARGET" -sdk "$SDK" -suppress-warnings \
  -framework Virtualization -framework Foundation -framework AppKit \
  "$HERE/Sources/main.swift" "$HERE/Sources/Proxy.swift" "$HERE/../fence/fencelog/Sources/Receipt.swift" \
  -o "$HERE/build/sai-box"
codesign --force --sign - --entitlements "$HERE/sai-box.entitlements" "$HERE/build/sai-box"
codesign -d --entitlements - "$HERE/build/sai-box" 2>&1 | grep -q virtualization && echo "built and signed: $HERE/build/sai-box"
