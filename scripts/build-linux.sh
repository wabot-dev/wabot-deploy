#!/usr/bin/env bash
#
# Build the Linux binary from a Mac, using Docker.
#
#   scripts/build-linux.sh              # x86_64
#   scripts/build-linux.sh aarch64      # arm64
#
# Slow on Apple Silicon: an x86_64 build runs under emulation, which
# turns a one-minute compile into half an hour. Building on the target
# machine is usually faster and always simpler — see `scripts/deploy.sh`,
# which does that over SSH. Reach for this one when the target has no
# toolchain and you would rather not add one.
#
# Cross-compiling from macOS needs a Linux linker and a cross-built
# libsqlite3; a Linux container has both, and building *in* the target
# environment removes a class of "works here, not there" from the
# picture entirely.
#
# Sources are streamed in rather than bind-mounted: Docker Desktop
# shares only a few host directories by default, and a project outside
# them mounts as an empty directory — which fails with a confusing
# "could not find Cargo.toml" rather than "that path is not shared".
# A tar over stdin works wherever Docker does.
#
# The framework is a path dependency, so it goes in too, at the
# relative position `[patch.crates-io]` expects.

set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
PROJECT="$(pwd)"
FRAMEWORK="$(cd ../../framework/wabot-rust && pwd)"

case "$ARCH" in
  x86_64)  PLATFORM="linux/amd64" ;;
  aarch64) PLATFORM="linux/arm64" ;;
  *) echo "unknown architecture: $ARCH (try x86_64 or aarch64)"; exit 1 ;;
esac

echo "building for ${ARCH}-unknown-linux-gnu…"
mkdir -p target/linux

# `target/` is excluded from both: it holds host-built artifacts for a
# different platform, and copying it in is slow and useless.
tar czf - \
    -C "$(dirname "$PROJECT")" --exclude="$(basename "$PROJECT")/target" "$(basename "$PROJECT")" \
    -C "$(dirname "$FRAMEWORK")" --exclude="$(basename "$FRAMEWORK")/target" "$(basename "$FRAMEWORK")" \
| docker run --rm -i \
    --platform "$PLATFORM" \
    -v wabot-deploy-cargo-registry:/usr/local/cargo/registry \
    -v wabot-deploy-target:/build/cloud/wabot-deploy/target \
    rust:1-bookworm \
    bash -c '
      set -euo pipefail
      # The layout `[patch.crates-io]` expects: the framework two
      # levels up and across from the project.
      mkdir -p /build/cloud /build/framework
      cd /tmp && tar xzf -
      mv /tmp/wabot-deploy /build/cloud/
      mv /tmp/wabot-rust /build/framework/
      cd /build/cloud/wabot-deploy
      cargo build --release --quiet
      cat target/release/wabot-deploy
    ' > target/linux/wabot-deploy

chmod +x target/linux/wabot-deploy
echo
echo "built: target/linux/wabot-deploy"
ls -lh target/linux/wabot-deploy | awk '{print "  size:", $5}'
shasum -a 256 target/linux/wabot-deploy | awk '{print "  sha256:", $1}'
