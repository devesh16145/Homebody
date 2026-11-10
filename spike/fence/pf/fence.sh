#!/bin/bash
# fence.sh — Fence v0: run a local model daemon (Ollama) as a dedicated user whose
# internet access is denied by pf, while it keeps serving localhost.
#
# This is the no-entitlement fence that works on any Mac today. The Network Extension
# fence in ../ne replaces it once an Apple Developer identity is available.
#
# Usage (most commands need sudo):
#   sudo ./fence.sh install            create the fenced user, load pf rules, enable logging
#   sudo ./fence.sh run                start `ollama serve` as the fenced user (foreground)
#   sudo ./fence.sh allow HOST [HOST…] temporarily allow HTTPS to these hosts (model downloads)
#   sudo ./fence.sh revoke             remove all temporary allowances
#   sudo ./fence.sh watch              stream pf decisions into signed receipts (foreground)
#   sudo ./fence.sh test               prove: localhost works, internet is blocked, receipts exist
#   sudo ./fence.sh demo               run + watch + test under ONE sudo, keep serving until Ctrl-C
#   sudo ./fence.sh status             show pf rules, allowances and recent receipts
#   sudo ./fence.sh uninstall          restore /etc/pf.conf ruleset (keeps the user and models)
#   sudo ./fence.sh purge              uninstall and also delete the fenced user and its data
set -euo pipefail

FENCE_USER="${FENCE_USER:-_saiollama}"
FENCE_HOME="${FENCE_HOME:-/var/sai/ollama}"
ANCHOR="com.sandboxai/fence"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RULES="$HERE/fence.rules"
GEN_RULES="/etc/pf.anchors/com.sandboxai.fence"
MAIN_CONF="/etc/pf.anchors/com.sandboxai.main.conf"
FENCELOG="${FENCELOG:-$HERE/../fencelog/.build/release/fencelog}"
OLLAMA_BIN="${OLLAMA_BIN:-$(command -v ollama || echo /opt/homebrew/bin/ollama)}"
# Receipts belong to the invoking (non-root) user so `fencelog` works without sudo afterwards.
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME="$(dscl . -read /Users/"$REAL_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
export FENCELOG_DIR="${FENCELOG_DIR:-$REAL_HOME/Library/Application Support/SandboxAI}"

need_root() { [ "$(id -u)" -eq 0 ] || { echo "run with sudo"; exit 1; }; }
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

fencelog() { sudo -u "$REAL_USER" env FENCELOG_DIR="$FENCELOG_DIR" "$FENCELOG" "$@"; }

ensure_fencelog() {
  if [ ! -x "$FENCELOG" ]; then
    say "building fencelog"
    (cd "$HERE/../fencelog" && sudo -u "$REAL_USER" swift build -c release 2>&1 | tail -2)
  fi
  sudo -u "$REAL_USER" mkdir -p "$FENCELOG_DIR"
  fencelog init >/dev/null
}

pick_uid() {
  # First free UID in 500..599 that is not in use; role-style account below normal users.
  local used; used="$(dscl . -list /Users UniqueID | awk '{print $2}' | sort -n)"
  for u in $(seq 590 599) $(seq 500 589); do
    echo "$used" | grep -qx "$u" || { echo "$u"; return; }
  done
  echo "no free uid" >&2; exit 1
}

create_user() {
  if dscl . -read /Users/"$FENCE_USER" >/dev/null 2>&1; then
    say "user $FENCE_USER exists"; return
  fi
  local uid; uid="$(pick_uid)"
  say "creating user $FENCE_USER (uid $uid), no shell, home $FENCE_HOME"
  dscl . -create /Users/"$FENCE_USER"
  dscl . -create /Users/"$FENCE_USER" UniqueID "$uid"
  dscl . -create /Users/"$FENCE_USER" PrimaryGroupID 20
  dscl . -create /Users/"$FENCE_USER" UserShell /usr/bin/false
  dscl . -create /Users/"$FENCE_USER" RealName "Sandbox-ai fenced Ollama"
  dscl . -create /Users/"$FENCE_USER" NFSHomeDirectory "$FENCE_HOME"
  dscl . -create /Users/"$FENCE_USER" IsHidden 1
  dscl . -passwd /Users/"$FENCE_USER" "$(head -c 32 /dev/urandom | base64)"
  mkdir -p "$FENCE_HOME/models"
  chown -R "$FENCE_USER":staff "$FENCE_HOME"
  chmod 750 "$FENCE_HOME"
}

write_rules() {
  mkdir -p /etc/pf.anchors
  sed "s/@FENCE_USER@/$FENCE_USER/g" "$RULES" > "$GEN_RULES"
  # Main ruleset = Apple's default + our anchor. Loaded at runtime, not persisted across reboots.
  {
    cat /etc/pf.conf
    echo
    echo "# --- Sandbox-ai fence (runtime, not persistent) ---"
    echo "anchor \"$ANCHOR\""
    echo "load anchor \"$ANCHOR\" from \"$GEN_RULES\""
  } > "$MAIN_CONF"
}

load_pf() {
  say "loading pf ruleset with anchor $ANCHOR"
  pfctl -q -f "$MAIN_CONF"
  pfctl -q -E 2>/dev/null || true
  ifconfig pflog0 >/dev/null 2>&1 || ifconfig pflog0 create
  pfctl -a "$ANCHOR" -sr
}

cmd_install() {
  need_root
  ensure_fencelog
  create_user
  write_rules
  load_pf
  say "installed. Next: sudo ./fence.sh run   (in one terminal)"
  say "               sudo ./fence.sh watch (in another), then sudo ./fence.sh test"
}

cmd_run() {
  need_root
  [ -x "$OLLAMA_BIN" ] || { echo "ollama not found; brew install ollama"; exit 1; }
  say "starting ollama as $FENCE_USER (internet denied, localhost allowed). Ctrl-C to stop."
  exec sudo -u "$FENCE_USER" env HOME="$FENCE_HOME" OLLAMA_HOST=127.0.0.1:11434 \
       OLLAMA_MODELS="$FENCE_HOME/models" OLLAMA_NOHISTORY=1 "$OLLAMA_BIN" serve
}

cmd_allow() {
  need_root
  [ $# -ge 1 ] || { echo "usage: allow HOST [HOST…]"; exit 1; }
  for h in "$@"; do
    local ips; ips="$(dig +short "$h" A | grep -E '^[0-9.]+$' || true)"
    [ -n "$ips" ] || { echo "could not resolve $h"; continue; }
    for ip in $ips; do pfctl -q -a "$ANCHOR" -t sai_allow -T add "$ip" >/dev/null; done
    say "allowed $h → $(echo $ips | tr '\n' ' ')"
    fencelog append --source pf --process ollama --host "$h" --dst "$(echo "$ips" | head -1)" --port 443 \
      --decision ask-allow --rule "user:allow $h" >/dev/null
  done
  say "allowances are IP-based and expire on 'revoke' or reboot. Resolve again if IPs rotate."
}

cmd_revoke() { need_root; pfctl -q -a "$ANCHOR" -t sai_allow -T flush; say "all allowances removed"; }

cmd_watch() {
  need_root
  ensure_fencelog
  local uid; uid="$(id -u "$FENCE_USER")"
  say "streaming pf decisions for uid $uid into $FENCELOG_DIR/receipts.jsonl (Ctrl-C to stop)"
  exec tcpdump -n -e -ttt -l -i pflog0 2>/dev/null | fencelog ingest-pflog --process ollama --uid "$uid"
}

cmd_test() {
  need_root
  local ok=0 fail=0
  say "1/4 localhost: can the host reach ollama on 127.0.0.1:11434?"
  if curl -fsS -m 5 http://127.0.0.1:11434/api/version; then echo " ✓"; ok=$((ok+1)); else echo " ✗ (is 'fence.sh run' running?)"; fail=$((fail+1)); fi
  say "2/4 internet as $FENCE_USER: must FAIL"
  if sudo -u "$FENCE_USER" curl -sS -m 5 -o /dev/null https://example.com 2>/dev/null; then echo " ✗ reached example.com — fence is NOT working"; fail=$((fail+1)); else echo " ✓ blocked"; ok=$((ok+1)); fi
  say "3/4 DNS as $FENCE_USER: must FAIL"
  if sudo -u "$FENCE_USER" dig +time=2 +tries=1 example.com @1.1.1.1 +short 2>/dev/null | grep -q .; then echo " ✗ DNS went out"; fail=$((fail+1)); else echo " ✓ blocked"; ok=$((ok+1)); fi
  say "4/4 receipts: were the blocked attempts logged and is the chain valid?"
  sleep 1
  if fencelog verify && fencelog tail -n 5 | grep -q Blocked; then echo " ✓"; ok=$((ok+1)); else echo " ✗ (is 'fence.sh watch' running?)"; fail=$((fail+1)); fi
  echo; say "result: $ok passed, $fail failed"
  fencelog proof --process ollama || true
  [ "$fail" -eq 0 ]
}

cmd_status() {
  need_root
  say "pf enabled: $(pfctl -s info 2>/dev/null | head -1)"
  say "anchor rules:"; pfctl -a "$ANCHOR" -sr 2>/dev/null || echo "  (not loaded)"
  say "allowances:"; pfctl -a "$ANCHOR" -t sai_allow -T show 2>/dev/null || echo "  (none)"
  say "fenced user: $(id "$FENCE_USER" 2>/dev/null || echo 'not created')"
  say "recent receipts:"; fencelog tail -n 10 2>/dev/null || true
}

cmd_uninstall() {
  need_root
  say "restoring /etc/pf.conf ruleset"
  pfctl -q -a "$ANCHOR" -F all 2>/dev/null || true
  pfctl -q -f /etc/pf.conf
  rm -f "$GEN_RULES" "$MAIN_CONF"
  say "pf restored. User $FENCE_USER and $FENCE_HOME kept (use purge to remove)."
}

cmd_purge() {
  need_root
  cmd_uninstall
  if dscl . -read /Users/"$FENCE_USER" >/dev/null 2>&1; then
    say "deleting user $FENCE_USER and $FENCE_HOME"
    dscl . -delete /Users/"$FENCE_USER"
    rm -rf "$FENCE_HOME"
  fi
}

cmd_demo() {
  # Everything under one sudo: fenced ollama + receipt watcher in the background, then the test,
  # then keep serving until Ctrl-C. Children are stopped on exit.
  need_root
  ensure_fencelog
  [ -x "$OLLAMA_BIN" ] || { echo "ollama not found; brew install ollama"; exit 1; }
  local uid; uid="$(id -u "$FENCE_USER")"
  local logdir="$FENCE_HOME/logs"; mkdir -p "$logdir"; chown "$FENCE_USER" "$logdir"
  say "starting fenced ollama (log: $logdir/ollama.log)"
  sudo -u "$FENCE_USER" env HOME="$FENCE_HOME" OLLAMA_HOST=127.0.0.1:11434 OLLAMA_MODELS="$FENCE_HOME/models" \
       OLLAMA_NOHISTORY=1 "$OLLAMA_BIN" serve >"$logdir/ollama.log" 2>&1 &
  local ollama_pid=$!
  say "starting receipt watcher"
  tcpdump -n -e -ttt -l -i pflog0 2>/dev/null | fencelog ingest-pflog --process ollama --uid "$uid" &
  local watch_pid=$!
  trap 'say "stopping"; kill $ollama_pid $watch_pid 2>/dev/null; pkill -P $$ 2>/dev/null; wait 2>/dev/null; exit 0' INT TERM EXIT
  say "waiting for ollama to listen on 127.0.0.1:11434 (startup is slow while its cloud calls time out)"
  for i in $(seq 1 60); do
    curl -fsS -m 1 http://127.0.0.1:11434/api/version >/dev/null 2>&1 && break
    sleep 1
  done
  cmd_test || true
  echo
  say "fenced ollama keeps serving on 127.0.0.1:11434 and receipts keep flowing. Ctrl-C to stop."
  say "try from another terminal:  ollama list   |   sudo -u $FENCE_USER curl -m 5 https://example.com"
  wait $ollama_pid
}

case "${1:-}" in
  install) cmd_install ;;
  run) cmd_run ;;
  allow) shift; cmd_allow "$@" ;;
  revoke) cmd_revoke ;;
  watch) cmd_watch ;;
  test) cmd_test ;;
  demo) cmd_demo ;;
  status) cmd_status ;;
  uninstall) cmd_uninstall ;;
  purge) cmd_purge ;;
  *) sed -n '2,20p' "$0"; exit 1 ;;
esac
