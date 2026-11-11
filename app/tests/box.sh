#!/bin/bash
# box.sh — the Box, checked from inside on a real Mac (Apple silicon).
#
# Needs a running engine and the Box set up (`homebody box setup`). It starts the Box for one
# agent, tries from inside every way out the Box is there to close, and stops it again.
# A case passes when the attempt fails.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HB="${HB:-$HERE/../target/debug/homebody}"
APP="${APP_ID:-claude-code}"
pass=0; fail=0
check() { # name, expected substring, actual
  if [[ "$3" == *"$2"* ]]; then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$1"
  else fail=$((fail+1)); printf '  \033[31m✗ %s\033[0m\n      expected to contain: %s\n      got: %s\n' "$1" "$2" "$(echo "$3" | tail -2 | tr '\n' ' ')"; fi
}
"$HB" box | grep -q "set up" || { echo "Set the Box up first: homebody box setup"; exit 1; }
"$HB" box stop >/dev/null 2>&1
WS="$("$HB" status --json | python3 -c "import json,sys; print([a['workspace'] for a in json.load(sys.stdin)['apps'] if a['id']=='$APP'][0])")"
"$HB" ask off >/dev/null
check "starts from a fresh copy of its disk"            "running for $APP" "$("$HB" box start "$APP" 2>&1)"
R() { "$HB" box run -- "$@" 2>&1; }

echo "What the Box is"
check "runs as an ordinary person, not root"            "agent"   "$(R id -un)"
check "cannot become root"                              "password is required" "$(R sudo -n true)"
check "has no network interface at all"                 "only: lo " "only: $(R sh -c 'ip -br link | cut -d" " -f1 | tr "\n" " "')"
check "is told to use the gate"                         "127.0.0.1:3128" "$(R sh -c 'echo $https_proxy')"

echo "Ways out"
check "a direct connection has nothing to go out on"    "Couldn't connect" "$(R curl -sS --noproxy '*' -m 5 -o /dev/null https://1.1.1.1/)"
check "a direct connection over IPv6"                   "Couldn't connect" "$(R curl -sS --noproxy '*' -m 5 -o /dev/null 'https://[2606:4700:4700::1111]/')"
check "there is no name server to ask"                  "none"    "$(R sh -c 'getent hosts example.com || echo none')"
check "a UDP packet has nowhere to go"                  "unreachable" "$(R python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.sendto(b"x", ("1.1.1.1", 53))')"
check "the host answers on the gate's socket only"      "ConnectionResetError" "$(R python3 -c 'import socket; s=socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(4); s.connect((2, 22)); print("connected")')"
check "nor on any other socket"                         "Error"   "$(R python3 -c 'import socket; s=socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(4); s.connect((2, 47110)); print("connected")')"

echo "Through the gate"
check "a trusted place is reached (control)"            "404"     "$(R curl -sS -m 20 -o /dev/null -w '%{http_code}' https://api.anthropic.com/)"
check "a place that is not trusted is refused"          "403"     "$(R curl -sS -m 20 -o /dev/null https://example.org/)"
check "an address instead of a name is refused"         "403"     "$(R curl -sS -m 20 -o /dev/null https://1.1.1.1/)"
check "it shows in Activity under the agent's name"     "example.org" "$("$HB" call get_feed "{\"appId\":\"$APP\",\"limit\":6,\"verb\":\"blocked\"}")"

echo "What it sees of this Mac"
check "none of the Mac's folders"                       "No such file" "$(R ls /Users)"
R sh -c 'echo from-the-box > /work/box-test.txt; touch /home/agent/stays-in-the-box'
check "one shared folder, and what it writes arrives"   "from-the-box" "$(cat "$WS/box-test.txt" 2>&1)"
check "as your own file"                                "$(id -un)" "$(stat -f %Su "$WS/box-test.txt" 2>&1)"

echo "Starting over"
"$HB" box stop >/dev/null
check "a new start does not have the last run's files"  "No such file" "$("$HB" box start "$APP" >/dev/null 2>&1; R ls /home/agent/stays-in-the-box)"
check "the shared folder is yours and stays"            "from-the-box" "$(R cat /work/box-test.txt)"
"$HB" box stop >/dev/null
rm -f "$WS/box-test.txt"
"$HB" ask on >/dev/null
check "the Box is stopped and its disk gone"            "stopped" "$("$HB" box)"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
