#!/bin/bash
# build.sh — compile homebody-box with the Swift that comes with the Command Line Tools and sign
# it with the one entitlement a program needs to run virtual machines. That entitlement is not a
# restricted one: signing "to run locally" is enough, no developer account.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$HERE/../target/box}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
mkdir -p "$OUT"
swiftc -O -swift-version 5 -target "$(uname -m)-apple-macos14.0" -sdk "$SDK" -suppress-warnings \
  -framework Virtualization -framework Foundation "$HERE/Sources/main.swift" -o "$OUT/homebody-box"
strip -x "$OUT/homebody-box" 2>/dev/null || true
codesign --force --sign - --entitlements "$HERE/homebody-box.entitlements" "$OUT/homebody-box"
codesign -d --entitlements - "$OUT/homebody-box" 2>&1 | grep -q virtualization && echo "built and signed: $OUT/homebody-box ($(du -h "$OUT/homebody-box" | cut -f1))"
