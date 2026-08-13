#!/usr/bin/env bash
#
# Build one static musl binary here, and put it on a node.
#
#   scripts/cross.sh                       # just build
#   scripts/cross.sh root@node.example     # build, copy, install, restart
#   scripts/cross.sh root@a root@b         # …on several
#
# ## Why this exists, and what it replaces
#
# `deploy.sh` builds *on* the node, which removed the whole class of
# "the binary does not run there" and cost the node's whole afternoon.
# On the one-core test box a build takes three to four minutes with the
# profile degraded, twenty-five with it intact, and takes SSH away for
# the duration — sshd cannot fork while `rustc` holds the memory, so the
# machine answers HTTPS and refuses logins. It was also, on 2026-08-12,
# OOM-killed outright.
#
# This builds on the developer's machine in about two minutes on eight
# cores, with the profile *not* degraded, and produces one artifact that
# runs on both test nodes — which building on a node cannot: the Ubuntu
# box is glibc and the Alpine box is musl, so a binary built on one will
# not start on the other.
#
# ## Why zig rather than a musl cross toolchain
#
# `zig cc` is a complete cross C compiler in one package, which is what
# the C in this tree needs — SQLite is compiled from source by rusqlite,
# and ring has its own. `cargo-zigbuild` wires it to cargo. The
# alternative is a musl cross-gcc per host platform, which on macOS is a
# tarball from a third party.
#
# The release workflow still uses `musl-tools` on an x86_64 GitHub
# runner: there it is a native build and needs no cross compiler at all.
# Both produce the same target; this is the one for a laptop.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=x86_64-unknown-linux-musl
# The `node` profile, and here it is the real one: thin LTO fits on a
# machine with more than one core. See Cargo.toml.
PROFILE=node
BINARY="target/$TARGET/$PROFILE/wabot-deploy"

for tool in zig cargo-zigbuild protoc; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "missing $tool" >&2
    echo "  brew install zig protobuf && cargo install cargo-zigbuild" >&2
    exit 1
  }
done
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

echo "==> building $TARGET"
cargo zigbuild --profile "$PROFILE" --target "$TARGET"

# Static, or it is not the artifact this script promises: a binary that
# turns out to need an interpreter fails on the Alpine node at exec,
# with a message about a file that is right there.
file "$BINARY" | grep -q "statically linked" || {
  echo "$BINARY is not statically linked" >&2
  exit 1
}
echo "==> $(file -b "$BINARY")"
echo "==> $(shasum -a 256 "$BINARY" | cut -c1-16)  $(du -h "$BINARY" | cut -f1)"

for host in "$@"; do
  echo
  echo "==> $host"
  scp -q "$BINARY" "$host:/tmp/wabot-deploy.new"
  # A rename, never a write: the destination is running, and writing
  # over a running binary is refused where replacing the name it is
  # reached by is not. The previous one is kept — this is the only way
  # back if the new one does not start.
  ssh "$host" 'set -e
    sha256sum /tmp/wabot-deploy.new | cut -c1-16
    /tmp/wabot-deploy.new --version
    cp -a /usr/local/bin/wabot-deploy /usr/local/bin/wabot-deploy.previous
    chmod 0755 /tmp/wabot-deploy.new
    mv -f /tmp/wabot-deploy.new /usr/local/bin/wabot-deploy'

  # systemd returns; OpenRC does not, so it gets its own short timeout
  # and the state is read on a second connection either way.
  ssh "$host" 'command -v systemctl >/dev/null && systemctl restart wabot-deploy' \
    || timeout 25 ssh "$host" 'rc-service wabot-deploy restart' \
    || true
  sleep 8
  ssh "$host" 'wabot-deploy --version; wabot-deploy doctor 2>&1 | tail -2'
done
