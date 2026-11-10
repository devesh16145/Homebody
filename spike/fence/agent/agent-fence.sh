#!/bin/bash
# agent-fence.sh — run AI agents (Claude Code) on this Mac with no VM, inside a fence:
#   * a dedicated macOS user with its own home; your home folder stays private to you
#   * pf denies every connection from that user except loopback
#   * on loopback, the Sandbox-ai proxy applies a hostname allowlist, resolves DNS, writes receipts
#   * one shared workspace folder is the only place the agent and you both write
#
# Usage:
#   sudo ./agent-fence.sh install                 create the user, workspace and pf rules
#   ./agent-fence.sh proxy [--allow HOST ...]      start the allowlist proxy (no sudo; keep it running)
#   sudo ./agent-fence.sh run [claude args...]     run Claude Code as the fenced user in the workspace
#   sudo ./agent-fence.sh exec CMD...              run any command as the fenced user
#   sudo ./agent-fence.sh test                     prove: direct egress blocked, proxy allows/denies, receipts
#   sudo ./agent-fence.sh watch                    stream pf denials into signed receipts (foreground)
#   sudo ./agent-fence.sh allow-console            let your account run commands as the fenced user
#                                                  without a password, so the web console can drive it
#   sudo ./agent-fence.sh status | uninstall | purge
set -euo pipefail

AGENT_USER="${AGENT_USER:-_saiagent}"
AGENT_HOME="${AGENT_HOME:-/var/sai/agent}"
WORK="${WORK:-/Users/Shared/sandbox-ai/work}"
ANCHOR="com.sandboxai/agent"
PROXY_PORT="${PROXY_PORT:-3128}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RULES_SRC="$HERE/agent.rules"
RULES="/etc/pf.anchors/com.sandboxai.agent"
MAIN_CONF="/etc/pf.anchors/com.sandboxai.main.conf"
SAIBOX="${SAIBOX:-$HERE/../../box/build/sai-box}"
FENCELOG="${FENCELOG:-$HERE/../fencelog/.build/release/fencelog}"
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME="$(dscl . -read /Users/"$REAL_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
export FENCELOG_DIR="${FENCELOG_DIR:-$REAL_HOME/Library/Application Support/SandboxAI}"
DEFAULT_ALLOW=(api.anthropic.com claude.ai console.anthropic.com platform.claude.com registry.npmjs.org)

need_root() { [ "$(id -u)" -eq 0 ] || { echo "run with sudo"; exit 1; }; }
say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
fencelog() { sudo -u "$REAL_USER" env FENCELOG_DIR="$FENCELOG_DIR" "$FENCELOG" "$@"; }

pick_uid() {
  local used; used="$(dscl . -list /Users UniqueID | awk '{print $2}' | sort -n)"
  for u in $(seq 591 599) $(seq 500 589); do echo "$used" | grep -qx "$u" || { echo "$u"; return; }; done
  echo "no free uid" >&2; exit 1
}

create_user() {
  if dscl . -read /Users/"$AGENT_USER" >/dev/null 2>&1; then say "user $AGENT_USER exists"; return; fi
  local uid; uid="$(pick_uid)"
  say "creating user $AGENT_USER (uid $uid), home $AGENT_HOME"
  dscl . -create /Users/"$AGENT_USER"
  dscl . -create /Users/"$AGENT_USER" UniqueID "$uid"
  dscl . -create /Users/"$AGENT_USER" PrimaryGroupID 20
  dscl . -create /Users/"$AGENT_USER" UserShell /bin/zsh
  dscl . -create /Users/"$AGENT_USER" RealName "Sandbox-ai fenced agent"
  dscl . -create /Users/"$AGENT_USER" NFSHomeDirectory "$AGENT_HOME"
  dscl . -create /Users/"$AGENT_USER" IsHidden 1
  dscl . -passwd /Users/"$AGENT_USER" "$(head -c 32 /dev/urandom | base64)"
}

setup_dirs() {
  mkdir -p "$AGENT_HOME" "$WORK"
  chown -R "$AGENT_USER":staff "$AGENT_HOME"; chmod 750 "$AGENT_HOME"
  # Workspace: agent owns it, your account (group staff) can read and write too.
  chown "$AGENT_USER":staff "$WORK"; chmod 2775 "$WORK"
  [ -f "$WORK/README.md" ] || printf '# Sandbox-ai workspace\n\nThe fenced agent can only write here. Your home folder is not visible to it.\n' > "$WORK/README.md"
  chown "$AGENT_USER":staff "$WORK/README.md"
}

write_rules() {
  mkdir -p /etc/pf.anchors
  sed "s/@AGENT_USER@/$AGENT_USER/g" "$RULES_SRC" > "$RULES"
  {
    cat /etc/pf.conf
    echo
    echo "# --- Sandbox-ai anchors (runtime, not persistent) ---"
    for a in fence agent; do
      f="/etc/pf.anchors/com.sandboxai.$a"
      [ -f "$f" ] && { echo "anchor \"com.sandboxai/$a\""; echo "load anchor \"com.sandboxai/$a\" from \"$f\""; }
    done
  } > "$MAIN_CONF"
}

load_pf() {
  say "loading pf ruleset (anchors: $(ls /etc/pf.anchors/ | grep -oE 'com\.sandboxai\.(fence|agent)$' | sed 's/com.sandboxai.//' | tr '\n' ' '))"
  pfctl -q -f "$MAIN_CONF"
  pfctl -q -E 2>/dev/null || true
  ifconfig pflog0 >/dev/null 2>&1 || ifconfig pflog0 create
  pfctl -a "$ANCHOR" -sr
}

agent_env() {
  # Environment for anything run as the fenced user. Only the proxy is reachable, so point everything at it.
  echo "HOME=$AGENT_HOME" "USER=$AGENT_USER" "LOGNAME=$AGENT_USER" "SHELL=/bin/zsh" "TERM=${TERM:-xterm-256color}" \
       "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin" \
       "HTTPS_PROXY=http://127.0.0.1:$PROXY_PORT" "HTTP_PROXY=http://127.0.0.1:$PROXY_PORT" \
       "https_proxy=http://127.0.0.1:$PROXY_PORT" "http_proxy=http://127.0.0.1:$PROXY_PORT" \
       "NO_PROXY=localhost,127.0.0.1" "no_proxy=localhost,127.0.0.1" \
       ${ANTHROPIC_API_KEY:+"ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY"} \
       "DISABLE_AUTOUPDATER=1" "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1"
}

cmd_install() {
  need_root
  [ -x "$SAIBOX" ] || { echo "proxy binary missing: build spike/box first (cd spike/box && ./build.sh)"; exit 1; }
  [ -x "$FENCELOG" ] || { echo "fencelog missing: cd spike/fence/fencelog && swift build -c release"; exit 1; }
  sudo -u "$REAL_USER" mkdir -p "$FENCELOG_DIR"; fencelog init >/dev/null
  create_user
  setup_dirs
  write_rules
  load_pf
  command -v claude >/dev/null || say "note: 'claude' not found in PATH; brew install --cask claude-code"
  say "installed. Next:  ./agent-fence.sh proxy      (keep running)"
  say "               sudo ./agent-fence.sh test"
  say "               sudo ./agent-fence.sh run       (Claude Code, fenced)"
}

cmd_proxy() {
  [ -x "$SAIBOX" ] || { echo "proxy binary missing"; exit 1; }
  local allow=()
  if [ $# -gt 0 ]; then allow=("$@"); else for h in "${DEFAULT_ALLOW[@]}"; do allow+=(--allow "$h"); done; fi
  say "proxy on 127.0.0.1:$PROXY_PORT for the fenced agent. Ctrl-C to stop."
  exec "$SAIBOX" proxy --port "$PROXY_PORT" --label "agent" "${allow[@]}"
}

cmd_exec() {
  need_root
  local uid; uid="$(id -u "$AGENT_USER")"
  # shellcheck disable=SC2046
  exec sudo -u "$AGENT_USER" -H env -i $(agent_env) /bin/zsh -c 'cd "$1" && shift && exec "$@"' -- "$WORK" "$@"
}

cmd_run() {
  need_root
  command -v claude >/dev/null || { echo "claude not found; brew install --cask claude-code"; exit 1; }
  say "Claude Code as $AGENT_USER in $WORK, egress via proxy only. Your home folder is not visible to it."
  [ -n "${ANTHROPIC_API_KEY:-}" ] || say "no ANTHROPIC_API_KEY in your environment: Claude Code will offer its login flow (paste the code it asks for)."
  cmd_exec claude "$@"
}

cmd_test() {
  need_root
  local ok=0 fail=0
  say "1/4 direct internet as $AGENT_USER (bypassing proxy): must FAIL"
  if sudo -u "$AGENT_USER" -H env -i PATH=/usr/bin:/bin curl -sS -m 5 --noproxy '*' -o /dev/null https://example.com 2>/dev/null; then echo " ✗ reached example.com"; fail=$((fail+1)); else echo " ✓ blocked"; ok=$((ok+1)); fi
  say "2/4 DNS as $AGENT_USER: must FAIL"
  if sudo -u "$AGENT_USER" -H env -i PATH=/usr/bin:/bin dig +time=2 +tries=1 example.com @1.1.1.1 +short 2>/dev/null | grep -q .; then echo " ✗ DNS went out"; fail=$((fail+1)); else echo " ✓ blocked"; ok=$((ok+1)); fi
  say "3/4 untrusted host via proxy: must be refused (403)"
  if out=$(sudo -u "$AGENT_USER" -H env -i PATH=/usr/bin:/bin curl -x "http://127.0.0.1:$PROXY_PORT" -sS -m 10 https://webhook.site/sai 2>&1); then echo " ✗ reached webhook.site"; fail=$((fail+1)); else echo " ✓ $out"; ok=$((ok+1)); fi
  say "4/4 trusted host via proxy: must connect"
  code=$(sudo -u "$AGENT_USER" -H env -i PATH=/usr/bin:/bin curl -x "http://127.0.0.1:$PROXY_PORT" -sS -m 10 -o /dev/null -w '%{http_code}' https://api.anthropic.com/ 2>/dev/null || echo 000)
  if [ "$code" != "000" ]; then echo " ✓ api.anthropic.com answered http $code"; ok=$((ok+1)); else echo " ✗ could not reach api.anthropic.com (is the proxy running?)"; fail=$((fail+1)); fi
  echo; say "result: $ok passed, $fail failed"
  fencelog verify || true
  fencelog tail -n 4 || true
  [ "$fail" -eq 0 ]
}

cmd_watch() {
  need_root
  local uid; uid="$(id -u "$AGENT_USER")"
  say "streaming pf denials for $AGENT_USER into receipts (Ctrl-C to stop)"
  exec tcpdump -n -e -ttt -l -i pflog0 2>/dev/null | fencelog ingest-pflog --process agent --uid "$uid" --anchor agent
}

cmd_allow_console() {
  need_root
  local f=/etc/sudoers.d/sandbox-ai-agent
  say "allowing $REAL_USER to run commands as $AGENT_USER without a password (for the web console)"
  printf '%s ALL=(%s) NOPASSWD: ALL\n' "$REAL_USER" "$AGENT_USER" > "$f.tmp"
  chmod 440 "$f.tmp"
  visudo -cf "$f.tmp" >/dev/null && mv "$f.tmp" "$f" && say "written $f" || { rm -f "$f.tmp"; echo "sudoers validation failed"; exit 1; }
}

cmd_status() {
  need_root
  say "fenced user: $(id "$AGENT_USER" 2>/dev/null || echo 'not created')"
  say "workspace: $WORK"; ls -la "$WORK" 2>/dev/null | head -5
  say "anchor rules:"; pfctl -a "$ANCHOR" -sr 2>/dev/null || echo "  (not loaded)"
  say "proxy: $(pgrep -fl 'sai-box proxy' || echo 'not running')"
  say "recent receipts:"; fencelog tail -n 8 2>/dev/null || true
}

cmd_uninstall() {
  need_root
  say "removing agent anchor and restoring pf"
  pfctl -q -a "$ANCHOR" -F all 2>/dev/null || true
  rm -f "$RULES"
  if [ -f /etc/pf.anchors/com.sandboxai.fence ]; then write_rules_without_agent=1; { cat /etc/pf.conf; echo; echo 'anchor "com.sandboxai/fence"'; echo 'load anchor "com.sandboxai/fence" from "/etc/pf.anchors/com.sandboxai.fence"'; } > "$MAIN_CONF"; pfctl -q -f "$MAIN_CONF"; else rm -f "$MAIN_CONF"; pfctl -q -f /etc/pf.conf; fi
  rm -f /etc/sudoers.d/sandbox-ai-agent
  say "done. User $AGENT_USER and $WORK kept (purge removes them)."
}

cmd_purge() {
  need_root; cmd_uninstall
  dscl . -read /Users/"$AGENT_USER" >/dev/null 2>&1 && { say "deleting $AGENT_USER, $AGENT_HOME and $WORK"; dscl . -delete /Users/"$AGENT_USER"; rm -rf "$AGENT_HOME" "$WORK"; }
}

case "${1:-}" in
  install) cmd_install ;;
  proxy) shift; cmd_proxy "$@" ;;
  run) shift; cmd_run "$@" ;;
  exec) shift; cmd_exec "$@" ;;
  test) cmd_test ;;
  watch) cmd_watch ;;
  allow-console) cmd_allow_console ;;
  status) cmd_status ;;
  uninstall) cmd_uninstall ;;
  purge) cmd_purge ;;
  *) sed -n '2,20p' "$0"; exit 1 ;;
esac
