#!/bin/bash
# repro-build.sh — build the engine (the `homebody` command line tool) the reproducible way.
#
# Anyone who builds the same source with the same Rust version this way should get the same
# file, bit for bit, whatever folder the source is in and whenever it is built. That is what
# lets someone check that a published binary really was built from the published source.
#
#   scripts/repro-build.sh [target-dir]     prints the SHA-256 of the result
#
# What makes it reproducible:
#   - `--locked`: exactly the dependency versions in Cargo.lock
#   - `--remap-path-prefix`: the folder the source and the dependencies are in is not recorded
#   - no incremental build, one code generation unit, link-time optimisation (set in Cargo.toml)
#   - the linker's map of input files is left out (see LINK_FLAGS below)
#   - SOURCE_DATE_EPOCH: fixed to the last commit's time for anything that asks what time it is
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-$HERE/target/repro}"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$HERE" log -1 --format=%ct 2>/dev/null || echo 0)}"
export CARGO_INCREMENTAL=0
# The linker is told to leave out its map of where each input file was (-S). Without that the
# file's built-in ID is computed over those paths, which include a randomly named temporary
# folder, and no two builds match (seen: 48 bytes differed, the ID and its checksum).
LINK_FLAGS="-C link-arg=-Wl,-S"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) LINK_FLAGS="" ;; esac
export RUSTFLAGS="--remap-path-prefix=$HERE=/homebody --remap-path-prefix=$CARGO_HOME_DIR=/cargo --remap-path-prefix=$HOME=/home $LINK_FLAGS ${RUSTFLAGS_EXTRA:-}"
cd "$HERE"
cargo build --release --locked -p homebody-cli --target-dir "$TARGET" >&2
BIN="$TARGET/release/homebody"
[ -f "$BIN" ] || BIN="$TARGET/release/homebody.exe"
if command -v sha256sum >/dev/null 2>&1; then sha256sum "$BIN" | cut -d' ' -f1; else shasum -a 256 "$BIN" | cut -d' ' -f1; fi
