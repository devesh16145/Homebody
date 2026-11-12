#!/bin/bash
# run.sh — run the extension inside a real editor on this computer, against the Homebody that
# is running, and print what was found.
#
#   test/in-editor/run.sh [editor command]      default: code, else cursor
#
# The editor is started with a throw-away profile and a throw-away folder, so your own windows,
# settings and extensions are not touched. Homebody itself is the real one: the test brings a
# folder in for one agent, opens that agent in a terminal for a few seconds, asks one question,
# blocks everything for a moment, and puts all of it back.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT="$(cd "$HERE/../.." && pwd)"
EDITOR_CMD="${1:-$(command -v code || command -v cursor || true)}"
[ -n "$EDITOR_CMD" ] || { echo "No editor found. Give its command: run.sh /path/to/code"; exit 2; }
TMP="$(mktemp -d /tmp/hb-editor.XXXXXX)"
WORK="$TMP/project"; mkdir -p "$WORK/.vscode" "$WORK/bad"
echo "hello" > "$WORK/hello.txt"
# The folder asks for two places (one Homebody already knows, one it does not) ...
printf '{ "trust": ["api.anthropic.com", "homebody-editor-test.example", "not a place"] }\n' > "$WORK/.homebody.json"
# ... and tries to name the program that "runs the agent protected". It must not be obeyed.
printf '#!/bin/sh\necho "the folder chose this program" > "%s/obeyed"\n' "$TMP" > "$TMP/not-homebody"; chmod +x "$TMP/not-homebody"
printf '{ "homebody.cliPath": "%s/not-homebody" }\n' "$TMP" > "$WORK/.vscode/settings.json"
printf '{ "trust": "everything", "also": 1 }\n' > "$WORK/bad/.homebody.json"
export HOMEBODY_TEST_OUT="$TMP/result.json"
"$EDITOR_CMD" --user-data-dir="$TMP/profile" --extensions-dir="$TMP/extensions" --disable-workspace-trust --skip-welcome --skip-release-notes --disable-updates --disable-telemetry \
  --extensionDevelopmentPath="$EXT" --extensionTestsPath="$HERE/index.js" "$WORK" > "$TMP/editor.log" 2>&1 &
for i in $(seq 1 180); do [ -s "$HOMEBODY_TEST_OUT" ] && break; sleep 1; done
sleep 2
pkill -f -- "--user-data-dir=$TMP/profile" 2>/dev/null
if [ ! -s "$HOMEBODY_TEST_OUT" ]; then echo "The editor did not finish the test. Its output:"; tail -20 "$TMP/editor.log"; echo "(kept: $TMP)"; exit 1; fi
[ -e "$TMP/obeyed" ] && echo "✗ THE FOLDER'S PROGRAM WAS RUN"
node -e '
const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
console.log("Editor: " + r.editor);
for (const t of r.results) console.log((t.ok ? "  ✓ " : "  ✗ ") + t.name + (t.ok ? "" : "\n      " + t.detail));
const bad = r.results.filter((t) => !t.ok).length;
console.log(`\n${r.results.length - bad} passed, ${bad} failed`);
process.exit(bad ? 1 : 0);
' "$HOMEBODY_TEST_OUT"; rc=$?
[ -e "$TMP/obeyed" ] && rc=1
rm -rf "$TMP"
exit $rc
