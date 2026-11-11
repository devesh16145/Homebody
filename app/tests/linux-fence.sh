#!/bin/bash
# linux-fence.sh — the fence on Linux, checked end to end on a real machine.
#
# Needs root once (through sudo) for setup, so it is meant for a throwaway machine or a build
# runner. It adds one app of its own ("Fence probe", which is just curl), runs setup, and then
# tries, as that app's private user, every way out that the fence is there to close.
# A case passes when the attempt is refused.
#
#   tests/linux-fence.sh            set up, run every case, leave the fence installed
#   tests/linux-fence.sh --remove   afterwards remove everything setup created
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HB="${HB:-$HERE/../target/debug/homebody}"
pass=0; fail=0
check() { # name, expected substring, actual
  if [[ "$3" == *"$2"* ]]; then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$1"
  else fail=$((fail+1)); printf '  \033[31m✗ %s\033[0m\n      expected to contain: %s\n      got: %s\n' "$1" "$2" "$(echo "$3" | tail -3 | tr '\n' ' ')"; fi
}
refused() { # name, command... : passes when the command fails
  local name="$1"; shift
  local out; out="$("$@" 2>&1)"; local rc=$?
  if [ $rc -ne 0 ]; then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$name"
  else fail=$((fail+1)); printf '  \033[31m✗ %s\033[0m\n      it worked, and should not have: %s\n' "$name" "$(echo "$out" | tail -2 | tr '\n' ' ')"; fi
}

[ "$(uname -s)" = Linux ] || { echo "This script is for Linux."; exit 1; }
"$HB" status >/dev/null 2>&1 || { echo "Starting the engine"; ("$HB" daemon >/tmp/homebody-daemon.log 2>&1 &); sleep 3; }
"$HB" status >/dev/null 2>&1 || { echo "The engine did not start:"; cat /tmp/homebody-daemon.log; exit 1; }

echo "Setup"
"$HB" call app_add '{"name":"Fence probe","program":"/usr/bin/curl"}' >/dev/null 2>&1 || true
APP=my-fence-probe
"$HB" setup --print > /tmp/homebody-setup.sh
check "the setup script names the probe's private user" "ensure_user '_hbapp1'" "$(cat /tmp/homebody-setup.sh)"
out="$(sudo bash /tmp/homebody-setup.sh 2>&1)"; rc=$?
check "setup finishes" "ok " "$(echo "$out" | tail -1) rc=$rc"
[ $rc -eq 0 ] || { echo "$out" | tail -20; }
out="$(sudo bash /tmp/homebody-setup.sh 2>&1 | tail -1)"
check "setup can be run again" "ok " "$out"
"$HB" check >/dev/null 2>&1
st="$("$HB" status --json)"
fence="$(echo "$st" | python3 -c "import json,sys; print([a['fence'] for a in json.load(sys.stdin)['apps'] if a['id']=='$APP'][0])")"
check "the app reports the probe's protection as up" "up" "$fence"
[ "$fence" = up ] || echo "$st" | python3 -c "import json,sys; s=json.load(sys.stdin); print(s['setupReasons'])"

GATE_PORT="$(grep -o '"proxy_port": *[0-9]*' ~/.local/share/Homebody/settings.json | tail -1 | grep -o '[0-9]*$')"
R() { "$HB" run "$APP" --cwd / -- "$@" 2>&1; }
ME="$(id -un)"

echo "Who and where"
check "runs as its private user"                         "_hbapp1"       "$(R /usr/bin/id -un)"
check "its group is the protected group only"            "_homebody"     "$(R /usr/bin/id -Gn)"
check "your environment does not come along"             "none"          "$(HB_SECRET=leak R /bin/sh -c 'echo ${HB_SECRET:-none}')"
check "it is told where its gate is"                     "127.0.0.1:$GATE_PORT" "$(R /usr/bin/printenv HTTPS_PROXY)"
refused "cannot list your home folder"                   "$HB" run "$APP" --cwd / -- /bin/ls "$HOME"
echo secret > "$HOME/hb-private.txt"; chmod 644 "$HOME/hb-private.txt"
refused "cannot read a world-readable file in your home" "$HB" run "$APP" --cwd / -- /bin/cat "$HOME/hb-private.txt"
rm -f "$HOME/hb-private.txt"
refused "cannot become root with sudo"                   "$HB" run "$APP" --cwd / -- /usr/bin/sudo -n /usr/bin/true
check "set-user-ID programs give it nothing"             "NoNewPrivs:	1" "$(R /bin/grep NoNewPrivs /proc/self/status)"

echo "Straight to the internet"
refused "direct connection to an address (TCP 443)"      "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS --noproxy '*' -m 6 https://1.1.1.1/
refused "direct connection on another port (TCP 80)"     "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS --noproxy '*' -m 6 http://1.1.1.1/
refused "direct connection over IPv6"                    "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS --noproxy '*' -m 6 'https://[2606:4700:4700::1111]/'
refused "UDP packet to a name server"                    "$HB" run "$APP" --cwd / -- /usr/bin/python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(3); s.sendto(b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01", ("1.1.1.1", 53)); print(s.recv(512))'
refused "ping"                                           "$HB" run "$APP" --cwd / -- /usr/bin/ping -c 1 -W 3 1.1.1.1

echo "Name lookups"
refused "lookup through the system (getent)"             "$HB" run "$APP" --cwd / -- /usr/bin/getent hosts example.com
refused "lookup from a program (getaddrinfo)"            "$HB" run "$APP" --cwd / -- /usr/bin/python3 -c 'import socket; print(socket.getaddrinfo("example.com", 443))'
refused "lookup through the local resolver's own port"   "$HB" run "$APP" --cwd / -- /usr/bin/python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(3); s.sendto(b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01", ("127.0.0.53", 53)); print(s.recv(512))'
if command -v resolvectl >/dev/null; then
  refused "lookup by asking the resolver service"        "$HB" run "$APP" --cwd / -- /usr/bin/resolvectl query example.com
fi
refused "curl without the gate cannot find a name"       "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS --noproxy '*' -m 6 https://example.com/

echo "Through the gate"
"$HB" ask off >/dev/null 2>&1
"$HB" trust "$APP" example.com >/dev/null 2>&1
check "a trusted place is reached (control)"             "200"           "$(R /usr/bin/curl -sS -m 15 -o /dev/null -w '%{http_code}' https://example.com/)"
check "a place that is not trusted is refused"           "403"           "$(R /usr/bin/curl -sS -m 15 -o /dev/null -w '%{http_code}' https://example.org/ ; R /usr/bin/curl -sS -m 15 https://example.org/ 2>&1 | tail -1)"
OTHER=47110; [ "$GATE_PORT" = 47110 ] && OTHER=47111
check "the other gate is listening (control, as you)"    "403"           "$(curl -sS -m 6 -x http://127.0.0.1:$OTHER -o /dev/null -w '%{http_code}' https://example.org/ 2>&1)"
refused "cannot use another app's gate"                  "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS -m 6 -x "http://127.0.0.1:$OTHER" https://api.anthropic.com/

echo "Workspace"
WS="/var/lib/homebody/work/$APP"
check "can write in its workspace"                       "made"          "$("$HB" run "$APP" -- /bin/sh -c 'mkdir -p d && echo x > d/f.txt && echo made')"
check "you can read what it wrote"                       "x"             "$(cat "$WS/d/f.txt" 2>&1)"
check "you can change and remove what it wrote"          "gone"          "$(echo y >> "$WS/d/f.txt" && rm -r "$WS/d" && echo gone 2>&1)"
refused "cannot write outside its workspace and home"    "$HB" run "$APP" --cwd / -- /usr/bin/touch /var/lib/homebody/work/hb-test

echo "Headless run"
out="$("$HB" ci --trust example.com --fail-on-blocked "$APP" -- /usr/bin/curl -sS -m 15 -o /dev/null https://example.com/ 2>&1)"; rc=$?
check "a clean run exits 0 and says nothing was refused" "Nothing was refused. rc=0" "$(echo "$out" | grep -o 'Nothing was refused.') rc=$rc"
out="$("$HB" ci --fail-on-blocked "$APP" -- /bin/sh -c 'curl -sS -m 15 -o /dev/null https://example.net/; true' 2>&1)"; rc=$?
check "a run with a refusal exits 3"                     "rc=3"          "rc=$rc"

echo "Staying up"
sudo nft delete table inet homebody
sleep 24
refused "rules removed by hand are back within 20 seconds" "$HB" run "$APP" --cwd / -- /usr/bin/curl -sS --noproxy '*' -m 6 https://1.1.1.1/
check "refused attempts reach Activity"                  "around_gate"   "$("$HB" call get_feed "{\"appId\":\"$APP\",\"limit\":400}" | grep -o around_gate | head -1) (needs the kernel log to be readable by you)"

"$HB" ask on >/dev/null 2>&1
if [ "${1:-}" = "--remove" ]; then
  echo "Removal"
  "$HB" setup --print-remove > /tmp/homebody-remove.sh
  check "everything setup created can be removed" "removed" "$(sudo bash /tmp/homebody-remove.sh 2>&1 | tail -1)"
  refused "the private user is gone" id -u _hbapp1
fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
