#!/bin/bash
# repro-check.sh — prove the reproducible build is reproducible on this machine.
#
# Copies the source to two different folders, builds each from scratch with
# scripts/repro-build.sh, and compares the results. Different folders are the point: a build
# that records where it was built would differ.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${1:-$(mktemp -d)}"
export SOURCE_DATE_EPOCH="$(git -C "$HERE" log -1 --format=%ct 2>/dev/null || echo 0)"
for n in one "two/deeper/path"; do
  mkdir -p "$WORK/$n"
  # The source only: no build output, no node modules.
  (cd "$HERE" && tar -cf - --exclude ./target --exclude ./node_modules --exclude ./tools/cedar-check/target .) | (cd "$WORK/$n" && tar -xf -)
done
A="$(bash "$WORK/one/scripts/repro-build.sh" "$WORK/one/target/repro")"
B="$(bash "$WORK/two/deeper/path/scripts/repro-build.sh" "$WORK/two/deeper/path/target/repro")"
echo "first build:  $A"
echo "second build: $B"
if [ "$A" = "$B" ]; then echo "REPRODUCIBLE: both builds are the same file."; else echo "NOT REPRODUCIBLE: the two builds differ."; exit 1; fi
