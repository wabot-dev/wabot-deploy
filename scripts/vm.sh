#!/usr/bin/env bash
#
# A whole node in a VM on this machine: everything but ACME.
#
#   scripts/vm.sh up             # create and boot it
#   scripts/vm.sh deploy         # build for it; install on the first run, replace after
#   scripts/vm.sh token          # a setup token for /setup
#   scripts/vm.sh root [cmd…]    # a root shell, or one command as root
#   scripts/vm.sh logs           # follow the node's log
#   scripts/vm.sh down           # stop it, keep it
#   scripts/vm.sh reset          # throw it away
#
# Then https://localhost:9443.
#
# `dev.sh` has no containerd, so every service there reads `unknown` and
# nothing deploys. This is the rest of a node — OpenRC, containerd, CNI,
# cgroups, the registry a push lands in — on Alpine, like the test nodes,
# and on this machine's own architecture so it runs at native speed.
#
# ACME is off because no authority can reach a laptop: `--self-signed`
# writes `[acme] disabled` and the node serves its own certificate.
#
# Needs `brew install lima zig protobuf` and `cargo install cargo-zigbuild`.
set -euo pipefail
cd "$(dirname "$0")/.."

VM="${WABOT_VM:-wabot-dev}"
# The profile a node runs, not a debug build: that one is half a
# gigabyte, and hashes a password slowly enough to look like a bug.
PROFILE="${WABOT_VM_PROFILE:-node}"

root() { limactl shell --workdir / "$VM" sudo "$@"; }

case "${1:-}" in
up)
  if limactl list -q 2>/dev/null | grep -qx "$VM"; then
    limactl start "$VM"
  else
    limactl start --name="$VM" --tty=false scripts/vm.yaml
  fi
  ;;

deploy)
  arch=$(limactl shell --workdir / "$VM" uname -m)
  target="$arch-unknown-linux-musl"
  binary="target/$target/$PROFILE/wabot-deploy"
  rustup target list --installed | grep -qx "$target" || rustup target add "$target"
  echo "==> building $target ($PROFILE)"
  cargo zigbuild --profile "$PROFILE" --target "$target"
  echo "==> $(shasum -a 256 "$binary" | cut -c1-16)  $(du -h "$binary" | cut -f1)"
  limactl copy "$binary" "$VM:/tmp/wabot-deploy.new"
  if root test -f /etc/wabot-deploy/config.toml; then
    # A rename, never a write over the running binary — see cross.sh.
    root sh -c 'set -e
      cp -a /usr/local/bin/wabot-deploy /usr/local/bin/wabot-deploy.previous
      chmod 0755 /tmp/wabot-deploy.new
      mv -f /tmp/wabot-deploy.new /usr/local/bin/wabot-deploy'
    # OpenRC's restart returns only once the node has drained, which
    # can outlast the patience of a script; the state is read after.
    timeout 30 limactl shell --workdir / "$VM" sudo rc-service wabot-deploy restart || true
    sleep 3
    root wabot-deploy doctor 2>&1 | tail -3
  else
    echo "==> first run: installing"
    root sh -c 'chmod 0755 /tmp/wabot-deploy.new && /tmp/wabot-deploy.new install --self-signed'
    echo
    echo "    https://localhost:9443 — the token above goes into /setup"
  fi
  ;;

token)
  root wabot-deploy setup-token
  ;;

root)
  shift
  if (($#)); then root "$@"; else root -i; fi
  ;;

logs)
  root tail -n 100 -F /var/log/wabot-deploy.log
  ;;

down)
  limactl stop "$VM"
  ;;

reset)
  limactl delete --force "$VM"
  ;;

*)
  sed -n '3,13p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
  ;;
esac
