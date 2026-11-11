#!/bin/bash
# windows-fence.sh — the fence on Windows, checked end to end on a real machine.
#
# Run from Git Bash. Needs no administrator. It adds one app of its own ("Fence probe", which
# is just Windows' own curl), and then tries, from inside that app's private space, every way
# out that the fence is there to close. A case passes when the attempt is refused.
set -u
export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*'
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HB="${HB:-$HERE/../target/debug/homebody.exe}"
SYS='C:\Windows\System32'
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

"$HB" status >/dev/null 2>&1 || { echo "Starting the engine"; ("$HB" daemon >"$TEMP/homebody-daemon.log" 2>&1 &); sleep 4; }
"$HB" status >/dev/null 2>&1 || { echo "The engine did not start:"; cat "$TEMP/homebody-daemon.log"; exit 1; }

echo "Setup"
"$HB" call app_add '{"name":"Fence probe","program":"C:\\Windows\\System32\\curl.exe"}' >/dev/null 2>&1 || true
APP=my-fence-probe
"$HB" check >/dev/null 2>&1
st="$("$HB" status --json)"
fence="$(echo "$st" | python -c "import json,sys; print([a['fence'] for a in json.load(sys.stdin)['apps'] if a['id']=='$APP'][0])" 2>&1)"
check "the probe's protection is up, with no setup step" "up" "$fence"
[ "$fence" = up ] || { echo "$st" | head -40; cat "$TEMP/homebody-daemon.log"; }
R() { "$HB" run "$APP" -- "$@" 2>&1; }

echo "Controls: the same things work for you"
check "a direct connection works for you"                "ok"     "$("$SYS\\curl.exe" -sS --noproxy '*' -m 10 -o NUL https://1.1.1.1/ && echo ok)"
check "a name is found for you"                          "ok"     "$("$SYS\\curl.exe" -sS --noproxy '*' -m 10 -o NUL https://example.com/ && echo ok)"

echo "Inside the private space"
check "a program runs there and its output comes back"   "hello"  "$(R "$SYS\\cmd.exe" /c echo hello)"
check "its exit code comes back"                         "rc=7"   "$("$HB" run "$APP" -- "$SYS\\cmd.exe" /c exit 7 >/dev/null 2>&1; echo rc=$?)"
check "your environment does not come along"             "none"   "$(HB_SECRET=leak R "$SYS\\cmd.exe" /c "if defined HB_SECRET (echo leaked) else (echo none)")"
check "it is told where its gate is"                     "127.0.0.1:" "$(R "$SYS\\cmd.exe" /c echo %HTTPS_PROXY%)"

echo "Straight to the internet"
refused "direct connection to an address"                "$HB" run "$APP" -- "$SYS\\curl.exe" -sS --noproxy '*' -m 6 -o NUL https://1.1.1.1/
refused "direct connection on port 80"                   "$HB" run "$APP" -- "$SYS\\curl.exe" -sS --noproxy '*' -m 6 -o NUL http://1.1.1.1/
refused "a name looked up without the gate"              "$HB" run "$APP" -- "$SYS\\curl.exe" -sS --noproxy '*' -m 6 -o NUL https://example.com/

echo "Through the gate"
"$HB" ask off >/dev/null 2>&1
"$HB" trust "$APP" example.com >/dev/null 2>&1
check "a trusted place is reached (control)"             "200"    "$(R "$SYS\\curl.exe" -sS -m 25 -o NUL -w '%{http_code}' https://example.com/)"
check "a place that is not trusted is refused"           "403"    "$(R "$SYS\\curl.exe" -sS -m 25 -o NUL https://example.org/)"

echo "Other programs on this computer"
(python -m http.server 8765 --bind 127.0.0.1 >/dev/null 2>&1 &)
sleep 2
check "a local server answers you (control)"             "200"    "$("$SYS\\curl.exe" -s --noproxy '*' -m 5 -o NUL -w '%{http_code}' http://127.0.0.1:8765/)"
refused "cannot reach a local server of yours"           "$HB" run "$APP" -- "$SYS\\curl.exe" -sS --noproxy '*' -m 5 -o NUL http://127.0.0.1:8765/
refused "cannot use another app's gate"                  "$HB" run "$APP" -- "$SYS\\curl.exe" -sS -m 6 -o NUL -x http://127.0.0.1:47110 https://api.anthropic.com/

echo "Files"
echo secret > "$USERPROFILE/hb-private.txt"
check "the file is there for you (control)"              "secret" "$(cat "$USERPROFILE/hb-private.txt")"
refused "cannot read a file in your user folder"         "$HB" run "$APP" -- "$SYS\\cmd.exe" /c type "$USERPROFILE\\hb-private.txt"
refused "cannot list your user folder"                   "$HB" run "$APP" -- "$SYS\\cmd.exe" /c dir "$USERPROFILE"
rm -f "$USERPROFILE/hb-private.txt"
check "can write in its workspace"                       "made"   "$(R "$SYS\\cmd.exe" /c "echo x> f.txt && echo made")"
check "you can read what it wrote"                       "x"      "$(cat "/c/ProgramData/Homebody/work/$APP/f.txt" 2>&1)"
refused "cannot write in the Windows folder"             "$HB" run "$APP" -- "$SYS\\cmd.exe" /c "echo x> C:\\Windows\\hb-test.txt"

echo "Headless run"
out="$("$HB" ci --trust example.com --fail-on-blocked "$APP" -- "$SYS\\curl.exe" -sS -m 25 -o NUL https://example.com/ 2>&1)"; rc=$?
check "a clean run exits 0 and says nothing was refused" "Nothing was refused. rc=0" "$(echo "$out" | grep -o 'Nothing was refused.') rc=$rc"
[ "$rc" = 0 ] || echo "$out" | tail -6

"$HB" ask on >/dev/null 2>&1
echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
