#!/usr/bin/env bash
#
# Build on the target machine and install there.
#
#   scripts/deploy.sh root@node.example.com
#   scripts/deploy.sh root@node.example.com --domain node.example.com --email you@example.com
#   DEPLOY_SSH_OPTS="-i ~/.ssh/id_node -p 2222" scripts/deploy.sh ...
#
# Builds *on* the node rather than cross-compiling to it. That is
# usually faster than emulating the target architecture locally, and it
# removes an entire class of "the binary does not run there" — the
# libc, the linker and the SQLite are the ones it will actually use.
#
# Everything after the host is passed to `install`, so this is the same
# thing you would type by hand, minus the copying.

set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${1:-}"
if [[ -z "$HOST" ]]; then
  echo "usage: scripts/deploy.sh user@host [install flags…]" >&2
  exit 2
fi
shift

SSH_OPTS="${DEPLOY_SSH_OPTS:-}"
REMOTE_DIR="/opt/wabot-deploy-src"
FRAMEWORK="$(cd ../../framework/wabot-rust && pwd)"

# shellcheck disable=SC2086
ssh_run() { ssh $SSH_OPTS "$HOST" "$@"; }

echo "==> preparing $HOST"
ssh_run 'set -e
  if ! command -v cargo >/dev/null 2>&1; then
    echo "    installing rust…"
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
  fi
  # Two build-time needs beyond rustc:
  #   cc      — `bundled` rusqlite compiles SQLite from C.
  #   protoc  — containerd-client generates its gRPC bindings from
  #             vendored protos in a build script.
  missing=""
  command -v cc     >/dev/null 2>&1 || missing="$missing build-essential"
  command -v protoc >/dev/null 2>&1 || missing="$missing protobuf-compiler"
  if [[ -n "$missing" ]]; then
    echo "    installing:$missing"
    # apk is listed because the second node this is tested on is
    # Alpine, and the script knowing about apt and dnf but not apk is
    # how a build there fails at the preparation step with a message
    # about `build-essential` — a package name that machine has never
    # heard of.
    (apk add --no-cache build-base protobuf-dev) >/dev/null 2>&1 \
      || (apt-get update -qq && apt-get install -y -qq $missing) >/dev/null 2>&1 \
      || (dnf install -y -q gcc make protobuf-compiler) >/dev/null 2>&1 \
      || { echo "    could not install$missing; do it by hand"; exit 1; }
  fi
  mkdir -p '"$REMOTE_DIR"

echo "==> copying sources"
# `target/` is excluded: it is host artifacts for the wrong platform.
# shellcheck disable=SC2086
tar czf - \
    --exclude="wabot-deploy/target" --exclude="wabot-rust/target" \
    --exclude=".git" \
    -C "$(dirname "$(pwd)")" "$(basename "$(pwd)")" \
    -C "$(dirname "$FRAMEWORK")" "$(basename "$FRAMEWORK")" \
  | ssh $SSH_OPTS "$HOST" "tar xzf - -C $REMOTE_DIR"

echo "==> building on the node"
ssh_run "set -e
  . \"\$HOME/.cargo/env\" 2>/dev/null || true
  cd $REMOTE_DIR/wabot-deploy
  # The layout [patch.crates-io] expects: ../../framework/wabot-rust.
  mkdir -p ../../framework
  ln -sfn $REMOTE_DIR/wabot-rust ../../framework/wabot-rust 2>/dev/null || true
  # The `node` profile, not `release`: a fat-LTO build on a one-core
  # VM swaps until the machine stops answering. See Cargo.toml.
  cargo build --profile node
  install -m 0755 target/node/wabot-deploy /usr/local/bin/wabot-deploy
  wabot-deploy --version"

if [[ $# -gt 0 ]]; then
  echo "==> installing"
  ssh_run "wabot-deploy install $*"
fi

echo
echo "done. Useful next:"
echo "  ssh $HOST 'wabot-deploy doctor'"
echo "  ssh $HOST 'wabot-deploy serve'"
