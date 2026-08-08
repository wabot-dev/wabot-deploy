#!/usr/bin/env bash
#
# Run the node on this machine, in the foreground.
#
#   scripts/dev.sh              # start it (installs on the first run)
#   scripts/dev.sh --reset      # throw the local node away and start over
#   DEV_HTTPS_PORT=9443 scripts/dev.sh
#
# For working on the console. The pages, the sessions, the registry and
# the database are the real ones; containerd is not there, so a service
# shows as "unknown" and nothing can actually be deployed. That is the
# trade — everything above the runtime is exactly what runs on a node,
# and it comes up in seconds instead of over SSH.
#
# What still has to happen on a real node: anything touching containerd,
# systemd, CNI or ACME. See CLAUDE.md — "Verify against the node".

set -euo pipefail
cd "$(dirname "$0")/.."

DEV_DIR="${DEV_DIR:-.dev}"
HTTPS_PORT="${DEV_HTTPS_PORT:-8443}"
HTTP_PORT="${DEV_HTTP_PORT:-8080}"
CONFIG="$DEV_DIR/config.toml"

if [[ "${1:-}" == "--reset" ]]; then
  echo "==> removing $DEV_DIR"
  rm -rf "$DEV_DIR"
  shift
fi

# Unprivileged ports on purpose: `serve` binds loopback rather than
# 0.0.0.0 when the HTTPS port is above 1024, so an unauthenticated
# console never appears on the café wifi. See `serve::bind_address`.
if (( HTTPS_PORT < 1024 || HTTP_PORT < 1024 )); then
  echo "refusing a privileged port: that binds 0.0.0.0, not loopback" >&2
  exit 2
fi

export WABOT_DEPLOY_DATA_DIR="$DEV_DIR/data"
export WABOT_DEPLOY_HTTPS_PORT="$HTTPS_PORT"
export WABOT_DEPLOY_HTTP_PORT="$HTTP_PORT"
# No public CA can validate a laptop. Left on, the renewal loop would
# start placing real orders the moment somebody set a domain in the
# console, and production locks the account out after five failures.
export WABOT_DEPLOY_ACME_DISABLED=1
export WABOT_DEPLOY_LOG="${WABOT_DEPLOY_LOG:-${RUST_LOG:-info}}"

echo "==> building"
cargo build

BIN=target/debug/wabot-deploy

if [[ ! -f "$CONFIG" ]]; then
  echo "==> first run: installing into $DEV_DIR"
  # --no-system leaves containerd, /usr/local/bin and the unit alone;
  # --skip-preflight because the checks are about a Linux node and this
  # is not one. Both are the documented way to do this on a laptop.
  "$BIN" install --no-system --skip-preflight --config "$CONFIG"
  echo
  echo "    the setup token above is the one to paste at /setup"
  echo
fi

echo "==> https://localhost:$HTTPS_PORT"
echo "    self-signed: the browser will warn once, and that is correct."
echo "    to lose the warning, trust $DEV_DIR/data/certs/local-ca.crt"
echo
exec "$BIN" serve --config "$CONFIG"
