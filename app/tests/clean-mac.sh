#!/bin/bash
# clean-mac.sh — Homebody the way a Mac that has never seen it meets it.
#
#   tests/clean-mac.sh <Homebody.dmg> --throwaway
#
# FOR A THROWAWAY MAC ONLY (a build runner). It copies the app out of the disk image into
# /Applications, starts it, runs setup as root through sudo, runs the whole bypass corpus against
# it, pulls the packet filter rules out to see them come back, restarts the app, and then removes
# everything setup created. It refuses to start on a Mac that already has Homebody.
#
# Nobody clicks anything: setup is approved with `homebody setup --print | sudo bash`, the way a
# build machine does it. What this cannot show is the windows themselves.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DMG="${1:-}"
APP=/Applications/Homebody.app
HB="$APP/Contents/MacOS/homebody"
DATA="$HOME/Library/Application Support/Homebody"
WORK="$(mktemp -d)"
pass=0; fail=0

check() { # name, expected substring, actual
  if [[ "$3" == *"$2"* ]]; then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$1"
  else fail=$((fail+1)); printf '  \033[31m✗ %s\033[0m\n      expected to contain: %s\n      got: %s\n' "$1" "$2" "$(echo "$3" | tail -3 | tr '\n' ' ')"; fi
}
field() { # a top-level field of the status, or "app.<field>" for the stand-in agent's entry
  "$HB" status --json 2>/dev/null | /usr/bin/python3 -I -c "
import json, sys
s = json.load(sys.stdin)
k = sys.argv[1]
if k.startswith('app.'):
    a = [a for a in s['apps'] if a['id'] == 'claude-code']
    print(a[0].get(k[4:]) if a else 'no such app')
else:
    print(s.get(k))" "$1" 2>&1
}
answers() { for _ in $(seq 1 "${1:-30}"); do "$HB" status >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }
start_app() { (cd / && /usr/bin/open "$APP" --args --hidden); }
quit_app() {
  /usr/bin/osascript -e 'tell application id "app.homebody.mac" to quit' >/dev/null 2>&1
  for _ in $(seq 1 10); do /usr/bin/pgrep -x homebody-app >/dev/null || return 0; sleep 1; done
  /usr/bin/pkill -x homebody-app; sleep 1
}

[ "$(uname -s)" = Darwin ] || { echo "This script is for macOS."; exit 1; }
[ -f "$DMG" ] && [ "${2:-}" = "--throwaway" ] || { echo "usage: tests/clean-mac.sh <Homebody.dmg> --throwaway   (a throwaway Mac only: it installs, sets up and removes Homebody)"; exit 1; }
if [ -e "$APP" ] || [ -e "$DATA" ] || [ -e "/Library/Application Support/Homebody" ]; then
  echo "This Mac already has Homebody. This test is for a Mac that has never had it, and it removes everything at the end, so it will not run here."
  exit 1
fi
/usr/bin/sudo -n /usr/bin/true 2>/dev/null || { echo "This test needs sudo without a password (a build runner)."; exit 1; }

echo "This Mac: macOS $(/usr/bin/sw_vers -productVersion) ($(/usr/bin/sw_vers -buildVersion)), $(uname -m)"
echo
echo "The disk image"
MNT="$WORK/mnt"; mkdir -p "$MNT"
out="$(/usr/bin/hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" 2>&1)"
check "it opens"                                        "yes"   "$([ -d "$MNT/Homebody.app" ] && echo yes || echo "no: $out")"
check "it holds the shortcut to Applications"           "/Applications" "$(/usr/bin/readlink "$MNT/Applications" 2>&1)"
kb="$(/usr/bin/du -sk "$MNT/Homebody.app" | awk '{print $1}')"
check "the app is under 10 MB"                          "yes"   "$([ "${kb:-99999}" -lt 10240 ] && echo yes || echo "no: $kb KB")"
parts=""; for p in homebody-app homebody homebody-box; do [ -x "$MNT/Homebody.app/Contents/MacOS/$p" ] || parts="$parts $p"; done
check "the app, the command and the Box helper are in it" "all" "$([ -z "$parts" ] && echo all || echo "missing:$parts")"
/usr/bin/ditto "$MNT/Homebody.app" "$APP" 2>/dev/null || /usr/bin/sudo /usr/bin/ditto "$MNT/Homebody.app" "$APP"
/usr/bin/hdiutil detach -quiet "$MNT" 2>/dev/null
check "it copies into Applications and the command runs" "the Homebody command line" "$("$HB" --help 2>&1 | sed -n 1p)"
# An app whose parts are signed but which is not sealed as a whole is shown to people who
# download it as "damaged", with no way to open it. Sealed, it gets the ordinary warning.
check "the app is sealed as a whole"                    "sealed" "$(/usr/bin/codesign --verify --deep --strict "$APP" 2>&1 && echo sealed)"
check "the Box helper may still run virtual machines"   "virtualization" "$(/usr/bin/codesign -d --entitlements - "$APP/Contents/MacOS/homebody-box" 2>&1 | grep -o 'virtualization' | head -1)"
echo "  For information, what macOS makes of the copy (a build without an Apple signature is refused here):"
/usr/bin/codesign -dv "$APP" 2>&1 | grep -E 'Signature|Authority|TeamIdentifier' | sed 's/^/      /'
/usr/sbin/spctl -a -vv "$APP" 2>&1 | sed 's/^/      /'

echo
echo "First start"
# A stand-in for an agent, so that there is an installed program to protect. It is never run:
# the attacks below use the system's own tools as the agent's private user.
/usr/bin/sudo /bin/mkdir -p /usr/local/bin
printf '#!/bin/sh\necho "stand-in for an agent"\n' | /usr/bin/sudo /usr/bin/tee /usr/local/bin/claude >/dev/null
/usr/bin/sudo /bin/chmod 755 /usr/local/bin/claude
start_app
if answers 40; then check "the app starts and its engine answers" "yes" "yes"
else
  check "the app starts and its engine answers" "yes" "no: $(/usr/bin/pgrep -lf homebody | tr '\n' ' ')"
  /usr/bin/log show --last 2m --style compact --predicate 'process == "homebody-app"' 2>/dev/null | tail -15
  echo "  Carrying on with the engine alone."
  ("$HB" daemon >"$WORK/daemon.log" 2>&1 &); answers 15 || { echo "The engine did not start either:"; cat "$WORK/daemon.log"; printf '\n%d passed, %d failed\n' "$pass" $((fail+1)); exit 1; }
fi
check "it is the version the image was named for"       "$(basename "$DMG" | sed -E 's/^Homebody-([0-9.]+)-.*/\1/')" "$(field version)"
check "it found the installed agent"                    "True"  "$(field app.installed)"
check "it says setup is needed"                         "True"  "$(field setupNeeded)"
check "and says why"                                    "'"     "$(field setupReasons)"
"$HB" run claude-code --cwd / -- /usr/bin/true >"$WORK/early.txt" 2>&1; rc=$?
check "nothing runs as protected before setup"          "refused" "$([ $rc -ne 0 ] && echo refused || echo "IT RAN: $(cat "$WORK/early.txt")")"
echo "      it said: $(tail -1 "$WORK/early.txt")"

echo
echo "Setup"
"$HB" setup --print > "$WORK/setup.sh"
out="$(/usr/bin/sudo /bin/bash "$WORK/setup.sh" 2>&1)"; rc=$?
check "setup finishes"                                  "ok"    "$(echo "$out" | tail -1) rc=$rc"
[ $rc -eq 0 ] || echo "$out" | tail -25
out="$(/usr/bin/sudo /bin/bash "$WORK/setup.sh" 2>&1)"; rc=$?
check "setup can be run again"                          "ok"    "$(echo "$out" | tail -1) rc=$rc"
"$HB" check >/dev/null 2>&1
for _ in $(seq 1 20); do [ "$(field app.fence)" = up ] && break; sleep 1; "$HB" check >/dev/null 2>&1; done
check "the agent's protection is up"                    "up"    "$(field app.fence)"
check "and nothing more is asked of you"                "False" "$(field setupNeeded) $(field setupReasons)"
check "a protected program runs as its private user"    "_hbagent" "$("$HB" run claude-code --cwd / -- /usr/bin/id -un 2>&1 | tail -1)"

check "the app's own escape test finds everything stopped" "Everything was stopped." "$("$HB" test claude-code 2>&1 | tail -1)"

echo
echo "Every way out we know of"
SAI="$HB" /bin/bash "$HERE/bypass-corpus.sh" > "$WORK/corpus.txt" 2>&1; rc=$?
cat "$WORK/corpus.txt" | sed 's/^/  /'
check "the whole bypass corpus holds on this Mac"       "0 failed rc=0" "$(tail -1 "$WORK/corpus.txt") rc=$rc"

echo
echo "A protected program and its terminal"
/usr/bin/python3 -I "$HERE/terminal.py" "$HB" > "$WORK/terminal.txt" 2>&1; rc=$?
cat "$WORK/terminal.txt" | sed 's/^/  /'
check "nothing outlives its terminal"                   "0 failed rc=0" "$(tail -1 "$WORK/terminal.txt") rc=$rc"

echo
echo "Someone pulls the rules out"
rules() { /usr/bin/sudo /sbin/pfctl -a homebody -sr 2>/dev/null | grep -c '^block'; }
before="$(rules)"
check "the packet filter rules are loaded"              "yes"   "$([ "${before:-0}" -gt 0 ] && echo yes || echo "no: $before block rules")"
/usr/bin/sudo /sbin/pfctl -q -a homebody -F rules 2>/dev/null
back=no; for _ in $(seq 1 30); do [ "$(rules)" = "$before" ] && { back=yes; break; }; sleep 1; done
check "the watchdog puts them back"                     "yes"   "$back (now $(rules), before $before)"
check "meanwhile a direct connection was still refused" "000"   "$("$HB" run claude-code --cwd / -- /usr/bin/curl -sS -m 6 --noproxy '*' -o /dev/null -w '%{http_code}' https://1.1.1.1/ 2>&1 | tail -1)"

echo
echo "The app is closed and opened again"
quit_app
check "it quits"                                        "gone"  "$(/usr/bin/pgrep -x homebody-app >/dev/null && echo STILL RUNNING || echo gone)"
check "the rules stay while it is closed"               "yes"   "$([ "$(rules)" = "$before" ] && echo yes || echo "no: $(rules)")"
start_app
check "it starts again"                                 "yes"   "$(answers 40 && echo yes || echo no)"
"$HB" check >/dev/null 2>&1
check "protection is up with nothing asked"             "up False" "$(field app.fence) $(field setupNeeded)"
check "the signed log is intact"                        "intact" "$("$HB" verify 2>&1 | tail -1)"

echo
echo "Removal"
"$HB" setup --print-remove > "$WORK/remove.sh"
out="$(/usr/bin/sudo /bin/bash "$WORK/remove.sh" 2>&1)"
check "removal finishes"                                "removed" "$(echo "$out" | tail -1)"
check "no private users are left"                       "0"     "$(/usr/bin/dscl . -list /Users | grep -c '^_hb')"
check "no private group is left"                        "0"     "$(/usr/bin/dscl . -list /Groups | grep -c '^_homebody')"
left=""; for f in /etc/pf.anchors/homebody /etc/sudoers.d/homebody /Library/LaunchDaemons/app.homebody.fence.plist "/Library/Application Support/Homebody"; do [ -e "$f" ] && left="$left $f"; done
check "no system files are left"                        "none"  "$([ -z "$left" ] && echo none || echo "left:$left")"
check "no packet filter rules are left"                 "0"     "$(rules)"
check "the watchdog is gone"                            "gone"  "$(/usr/bin/sudo /bin/launchctl print system/app.homebody.fence >/dev/null 2>&1 && echo STILL LOADED || echo gone)"
"$HB" check >/dev/null 2>&1
check "the app says setup is needed again"              "True"  "$(field setupNeeded)"
"$HB" run claude-code --cwd / -- /usr/bin/true >/dev/null 2>&1
check "and runs nothing as protected"                   "refused" "$([ $? -ne 0 ] && echo refused || echo IT RAN)"
quit_app

if [ "$fail" -gt 0 ]; then
  echo
  echo "What the app wrote:"
  tail -40 "$DATA"/logs/* 2>/dev/null
  /usr/bin/sudo /usr/bin/tail -5 /var/run/homebody.fence.err 2>/dev/null
fi
echo
printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
