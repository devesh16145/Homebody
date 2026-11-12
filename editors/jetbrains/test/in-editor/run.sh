#!/bin/bash
# run.sh — run the plugin inside a real JetBrains editor on this computer, against the Homebody
# that is running, and print what was found.
#
#   test/in-editor/run.sh            needs gradle and a Java 21 (GRADLE=/path/to/gradle to name one)
#
# Gradle starts the editor this plugin is built against, with a throw-away profile (the build's
# sandbox) and a throw-away project. A build made for this run carries src/selfcheck, which runs
# the plugin's own actions and presses the buttons of the dialogs they show. Homebody itself is
# the real one: the check brings a project in for one agent, opens that agent in a Terminal
# window for a few seconds, asks one question, blocks everything for a moment, and puts all of
# it back. A Terminal window is left behind, saying its session ended.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
GRADLE="${GRADLE:-gradle}"
TMP="$(mktemp -d /tmp/hb-idea.XXXXXX)"
WORK="$TMP/project"; mkdir -p "$WORK"
echo "hello" > "$WORK/hello.txt"
"$GRADLE" -p "$ROOT" ${GRADLE_FLAGS:-} runIde -PselfCheck="$TMP/result.json" -PselfCheckProject="$WORK" -PselfCheckPlace="${PLACE:-example.net}" > "$TMP/editor.log" 2>&1 &
GRADLE_PID=$!
for i in $(seq 1 420); do [ -s "$TMP/result.json" ] && break; kill -0 "$GRADLE_PID" 2>/dev/null || break; sleep 1; done
sleep 2
pkill -f -- "-Dhomebody.selfcheck=$TMP/result.json" 2>/dev/null
wait "$GRADLE_PID" 2>/dev/null
if [ ! -s "$TMP/result.json" ]; then echo "The editor did not finish the check. The end of its output:"; tail -25 "$TMP/editor.log"; echo "(kept: $TMP)"; exit 1; fi
python3 -I - "$TMP/result.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
print("Editor: " + r["editor"])
bad = 0
for t in r["results"]:
    print(("  ✓ " if t["ok"] else "  ✗ ") + t["name"] + ("" if t["ok"] else "\n      " + t["detail"]))
    bad += 0 if t["ok"] else 1
print(f"\n{len(r['results']) - bad} passed, {bad} failed")
sys.exit(1 if bad else 0)
PY
rc=$?
rm -rf "$TMP"
exit $rc
