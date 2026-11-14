#!/usr/bin/env bash
#
# A whole node in a VM on this machine: everything but ACME.
#
#   scripts/vm.sh up             # create and boot it
#   scripts/vm.sh deploy         # build for it; install on the first run, replace after
#   scripts/vm.sh token          # a setup token for /setup
#   scripts/vm.sh root [cmd…]    # a root shell, or one command as root
#   scripts/vm.sh logs           # follow the node's log
#   scripts/vm.sh suite          # the tests as root on the node, and what they changed
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
# turns the node's switch off and it serves its own certificate.
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

suite)
  # Root is what lets a test that reaches the machine change it: anyone
  # else's run fails quietly at the same call. On 2026-11-14 one run as
  # root left an overlay interface, network namespaces on two projects'
  # bridges, a container and files in the node's directory. A run should
  # change nothing, and this says whether it did.
  arch=$(limactl shell --workdir / "$VM" uname -m)
  target="$arch-unknown-linux-musl"
  rustup target list --installed | grep -qx "$target" || rustup target add "$target"
  echo "==> building the tests for $target"
  tests=$(cargo-zigbuild test --no-run --target "$target" 2>&1 |
    sed -n 's/.*Executable unittests src\/main.rs (\(.*\))/\1/p' | tail -1)
  [[ -n "$tests" ]] || { echo "no test binary came out of the build"; exit 1; }
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  # Tests read the tree at the path they were compiled in, so it goes to
  # the same path in the VM. Without COPYFILE_DISABLE, macOS adds a `._`
  # file beside each one and the tree's own guards fail on them.
  git ls-files -co --exclude-standard | while read -r file; do
    [[ -e "$file" ]] && printf '%s\n' "$file"
  done | COPYFILE_DISABLE=1 tar -cf "$scratch/tree.tar" -T -
  limactl copy "$scratch/tree.tar" "$VM:/var/tmp/wabot-suite.tar"
  limactl copy "$tests" "$VM:/var/tmp/wabot-suite-tests"
  # A file rather than stdin: a test that read stdin would eat the rest.
  root sh -c 'cat > /var/tmp/wabot-suite.sh' <<'SWEEP'
set -u
here=$1
s=/var/tmp/wabot-suite
# The tree is thrown away and replaced. Under a mount of this machine's
# disk it would be the checkout itself.
for mount in $(awk '$2 != "/" {print $2}' /proc/mounts); do
  case "$here/" in "$mount"/*)
    echo "$here is under $mount, a mount: not replacing it"
    exit 1
    ;;
  esac
done
rm -rf "$here" "$s" && mkdir -p "$here" "$s"
tar -xf /var/tmp/wabot-suite.tar -C "$here"
chmod 0755 /var/tmp/wabot-suite-tests

# Asked of the process table: OpenRC says stopped while the node drains.
running=no
pgrep -f "wabot-deploy .*serve" >/dev/null && running=yes
rc-service wabot-deploy stop >/dev/null 2>&1
i=0
while pgrep -f "wabot-deploy .*serve" >/dev/null && [ $i -lt 60 ]; do sleep 1; i=$((i + 1)); done

state() {
  ip -o link | awk -F': ' '{print $2}' | sed 's/@.*//' | sort > $s/links.$1
  ls /run/netns 2>/dev/null | sort > $s/namespaces.$1
  ctr -n wabot containers ls -q | sort > $s/containers.$1
  for table in nat filter mangle; do iptables-save -t $table; done |
    grep -v '^#' | sed 's/\[[0-9]*:[0-9]*\]//' > $s/iptables.$1
  ls /var/lib/cni/networks/*/ 2>/dev/null | sort > $s/reservations.$1
}
state before
touch $s/marker
sleep 1
(cd "$here" && /var/tmp/wabot-suite-tests < /dev/null > $s/tests.log 2>&1)
passed=$?
state after

changed=0
echo "==> $(grep 'test result' $s/tests.log)"
[ $passed -eq 0 ] || grep -E '^test .* FAILED$' $s/tests.log
for what in links namespaces containers iptables reservations; do
  if ! diff -q $s/$what.before $s/$what.after >/dev/null; then
    changed=1
    echo "==> $what changed"
    diff $s/$what.before $s/$what.after | grep '^[<>]'
  fi
done
# What a running copy writes on its own: its log, its volume, its WAL.
for id in $(cat $s/containers.after); do echo "/var/lib/wabot-deploy/logs/$id.log"; done > $s/own
written=$(find / -xdev -newer $s/marker -type f 2>/dev/null |
  grep -vE "^/(proc|sys|dev|tmp|var/tmp|home|root|var/log|var/lib/containerd)/|^$here/" |
  grep -vE '^/var/lib/wabot-deploy/(volumes|wal)/' | grep -vF -f $s/own)
written="$written$(find /run -newer $s/marker -type f 2>/dev/null | grep -vE 'containerd|/run/openrc/')"
if [ -n "$written" ]; then
  changed=1
  echo "==> files written"
  echo "$written"
fi
if pgrep -f "wabot-deploy .*serve" >/dev/null; then
  changed=1
  echo "==> a test started the node"
fi
[ $changed -eq 0 ] && echo "==> nothing on the machine changed"
[ $running = yes ] && rc-service wabot-deploy start >/dev/null 2>&1
[ $passed -eq 0 ] && [ $changed -eq 0 ]
SWEEP
  root sh /var/tmp/wabot-suite.sh "$(pwd)"
  ;;

down)
  limactl stop "$VM"
  ;;

reset)
  limactl delete --force "$VM"
  ;;

*)
  sed -n '3,14p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
  ;;
esac
