#!/bin/bash
# bypass-corpus.sh — every way we know of to get past the gate or the fence, as executable checks.
#
# Runs against a live engine (the app, or `sai daemon`). Each case is something that was a real
# bypass in some agent sandbox in 2025–2026, or an obvious variation. A case passes when the
# attempt is refused. Add a case here whenever a new bypass is published anywhere.
#
#   tests/bypass-corpus.sh            run everything
#   tests/bypass-corpus.sh gate       only the gate (name rules, request parsing, TLS greeting)
#   tests/bypass-corpus.sh fence      only the OS fence (runs commands as the protected user)
set -u   # no pipefail: a refused command exits non-zero, and that is the outcome we are testing for
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAI="${SAI:-$HERE/../target/debug/sai}"
APP="${APP_ID:-claude-code}"
PORT="${GATE_PORT:-47110}"
GATE="http://127.0.0.1:$PORT"
pass=0; fail=0

check() { # name, expected substring, actual
  if [[ "$3" == *"$2"* ]]; then pass=$((pass+1)); printf '  \033[32m✓\033[0m %s\n' "$1"
  else fail=$((fail+1)); printf '  \033[31m✗ %s\033[0m\n      expected to contain: %s\n      got: %s\n' "$1" "$2" "$(echo "$3" | tail -2 | tr '\n' ' ')"; fi
}
code() { curl -x "$GATE" -sS -m 12 -o /dev/null -w '%{http_code}' "$@" 2>&1 | tr '\n' ' '; }
raw()  { (printf "$1"; sleep "${2:-1}") | nc 127.0.0.1 "$PORT" 2>/dev/null | head -1 | tr -d '\r'; }

"$SAI" status >/dev/null 2>&1 || { echo "The engine is not running. Open the app or run: sai daemon"; exit 1; }

gate() {
  echo "Gate: destination names"
  check "trusted name is allowed (control)"            "404"  "$(code https://api.anthropic.com/)"
  check "raw IPv4 address"                              "403"  "$(code https://1.1.1.1/)"
  check "raw IPv6 address"                              "403"  "$(code 'https://[2606:4700:4700::1111]/')"
  check "IPv4 as one decimal number"                    "403"  "$(code https://16843009/)"
  check "IPv4 in hex"                                   "403"  "$(code https://0x01010101/)"
  check "IPv4-mapped IPv6"                              "403"  "$(code 'https://[::ffff:1.1.1.1]/')"
  check "single-label name (localhost)"                 "403"  "$(code https://localhost/)"
  check "NUL byte inside the name"                      "403"  "$(raw 'CONNECT api.anthropic.com\0.evil.example.org:443 HTTP/1.1\r\n\r\n')"
  check "NUL byte after a trusted name"                 "403"  "$(raw 'CONNECT api.anthropic.com\0:443 HTTP/1.1\r\n\r\n')"
  check "trusted name as a prefix of a blocked one"     "403"  "$(code https://api.anthropic.com.o1.ingest.sentry.io/)"
  check "trusted name in the user part of the URL"      "403"  "$(code https://api.anthropic.com@statsig.anthropic.com/)"
  check "blocked wildcard, deep subdomain"              "403"  "$(code https://a.b.c.sentry.io/)"
  check "upper-case spelling of a blocked name"         "403"  "$(code https://STATSIG.Anthropic.COM/)"
  check "trailing dot on a blocked name"                "403"  "$(code https://statsig.anthropic.com./)"
  check "port other than 80 or 443 on a trusted name"   "403"  "$(code https://api.anthropic.com:22/)"
  check "port 8443 on a trusted name"                   "403"  "$(code https://api.anthropic.com:8443/)"

  echo "Gate: names that point inward"
  "$SAI" trust "$APP" localtest.me >/dev/null 2>&1
  check "trusted name resolving to this computer"       "403"  "$(code https://localtest.me/)"
  check "same, plain HTTP"                              "403"  "$(code http://localtest.me/)"
  "$SAI" call remove_trust "{\"appId\":\"$APP\",\"host\":\"localtest.me\"}" >/dev/null 2>&1

  echo "Gate: request parsing"
  check "lower-case method"                             "400"  "$(raw 'connect api.anthropic.com:443 HTTP/1.1\r\n\r\n')"
  check "https:// in absolute form"                     "400"  "$(raw 'GET https://api.anthropic.com/ HTTP/1.1\r\n\r\n')"
  check "origin-form request (not a proxy request)"     "400"  "$(raw 'GET / HTTP/1.1\r\nHost: api.anthropic.com\r\n\r\n')"
  check "extra token on the request line"               "400"  "$(raw 'CONNECT api.anthropic.com:443 HTTP/1.1 x\r\n\r\n')"
  check "port 0"                                        "400"  "$(raw 'CONNECT api.anthropic.com:0 HTTP/1.1\r\n\r\n')"
  check "port out of range"                             "400"  "$(raw 'CONNECT api.anthropic.com:70000 HTTP/1.1\r\n\r\n')"
  check "line break smuggled into the target"           "400"  "$(raw 'CONNECT api.anthropic.com:443\r\nX: y HTTP/1.1\r\n\r\n')"

  echo "Gate: disguises inside an approved connection"
  check "greets the server as another name (SNI)"       "000"  "$(code --connect-to example.org:443:api.anthropic.com:443 https://example.org/)"
  check "no name in the greeting"                       "CUT"  "$( (echo | openssl s_client -proxy 127.0.0.1:$PORT -connect api.anthropic.com:443 -noservername 2>&1 | grep -q 'BEGIN CERTIFICATE' && echo CONNECTED) || echo CUT)"
  check "not TLS at all inside the tunnel"              "CUT"  "$( (printf 'CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n'; sleep 0.4; printf 'SSH-2.0-OpenSSH_9.6\r\n'; sleep 1.5) | nc 127.0.0.1 $PORT 2>/dev/null | tail -n +3 | grep -q . && echo ANSWERED || echo CUT)"
  check "Host header naming another site (plain HTTP)"  "claude.ai" "$(curl -x "$GATE" -sS -m 12 -o /dev/null -D - -H 'Host: evil.example.org' http://claude.ai/ 2>&1 | grep -i '^location:' | tr -d '\r')"
}

fence() {
  R() { "$SAI" run "$APP" --cwd "$AGENT_HOME" -- "$@" 2>&1; }
  AGENT_HOME="$("$SAI" run "$APP" --cwd / -- /usr/bin/printenv HOME 2>/dev/null | tail -1)"
  [ -n "$AGENT_HOME" ] || { echo "Fence: skipped (the fence is not up for $APP: run setup first)"; return; }
  ME="$(id -un)"; MYHOME="$HOME"

  echo "Fence: who and where"
  check "runs as the protected user, not as you"        "NOT_ME" "$([ "$(R /usr/bin/id -un | tail -1)" != "$ME" ] && echo NOT_ME || echo SAME_USER)"
  export SAI_CORPUS_SECRET=hunter2
  check "your environment does not leak in"             "CLEAN"  "$(R /usr/bin/env | grep -q hunter2 && echo LEAKED || echo CLEAN)"
  check "trusted name through the gate (control)"       "404"    "$(R /usr/bin/curl -sS -m 10 -o /dev/null -w '%{http_code}' https://api.anthropic.com/ | tail -1)"

  echo "Fence: going around the gate"
  check "direct connection, by name"                    "000"    "$(R /usr/bin/curl -sS -m 6 --noproxy '*' -o /dev/null -w '%{http_code}' https://api.anthropic.com/ | tail -1)"
  check "direct connection, IPv4 address"               "not permitted" "$(R /usr/bin/nc -v -z -G 3 1.1.1.1 443)"
  check "direct connection, IPv6 address"               "not permitted" "$(R /usr/bin/nc -v -z -G 3 2606:4700:4700::1111 443)"
  check "local network address"                         "not permitted" "$(R /usr/bin/nc -v -z -G 3 192.168.1.1 80)"
  check "name lookup through the system resolver"       "NONE"   "$(R /usr/bin/dscacheutil -q host -a name example.com | grep -q address && echo RESOLVED || echo NONE)"
  check "name lookup from a program (getaddrinfo)"      "gaierror" "$(R /usr/bin/python3 -c "import socket; print(socket.getaddrinfo('example.com',443))")"
  check "direct DNS query over UDP"                     "NONE"   "$(R /usr/bin/dig +time=2 +tries=1 +short example.com @1.1.1.1 | grep -qE '^[0-9]+\.' && echo ANSWERED || echo NONE)"
  check "another app's gate"                            "000"    "$(R /usr/bin/curl -sS -m 6 -x http://127.0.0.1:47111 -o /dev/null -w '%{http_code}' https://ollama.com/ | tail -1)"
  check "another service on this computer (port 22)"    "not permitted" "$(R /usr/bin/nc -v -z -G 3 127.0.0.1 22)"

  echo "Fence: your files"
  refused() { "$@" 2>&1 | grep -qiE 'not permitted|permission denied' && echo denied || echo ALLOWED; }
  check "list your home folder"                         "denied" "$(refused R /bin/ls "$MYHOME")"
  check "read a file in your home by exact path"        "denied" "$(R /bin/cat "$MYHOME/.zshrc" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo READ)"
  check "read your SSH keys"                            "denied" "$(R /bin/ls "$MYHOME/.ssh" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo LISTED)"
  check "create a file in your home"                    "denied" "$(refused R /usr/bin/touch "$MYHOME/.sai-corpus-probe")"
  check "reach the control socket (approve itself)"     "denied" "$(R /bin/ls "$("$SAI" call get_policy_text >/dev/null 2>&1; echo "${SAI_DATA_DIR:-$MYHOME/Library/Application Support/SandboxAI}")/control.sock" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo REACHED)"
}

case "${1:-all}" in
  gate) gate ;;
  fence) fence ;;
  *) gate; echo; fence ;;
esac
echo
printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
