#!/bin/bash
# box-ssh.sh — open a shell (or run a command) inside the running Box over the vsock SSH bridge.
# Requires sai-box to be running with --ssh-port 2222 (see README).
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec ssh -i "$HERE/build/box_ed25519" -p "${BOX_SSH_PORT:-2222}" \
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
  root@127.0.0.1 "$@"
