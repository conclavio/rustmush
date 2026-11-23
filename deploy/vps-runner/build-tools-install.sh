#!/usr/bin/env bash
#
# deploy/vps-runner/build-tools-install.sh — what a box needs, beyond what
# provision.sh already installs, to build a whole deploy by itself: the web
# client and both binaries from a clean checkout, with no other machine in the
# loop (owner's word, 2026-11-23: "let's install node on both boxes so they
# can do their own full builds, and any other tools required so my windows box
# isnt in the loop"). Run as root, on its own or through provision.sh's
# WITH_BUILD_TOOLS=1:
#
#   bash deploy/vps-runner/build-tools-install.sh
#
# What it installs:
#
#   Node.js NODE_VERSION, the release CI builds the web client with (`web`
#   job: Node 22). The official linux-x64 tarball from nodejs.org, verified
#   against the SHA-256 pinned below, which is the value nodejs.org publishes
#   in https://nodejs.org/dist/v<version>/SHASUMS256.txt. Unpacked by root into
#   /usr/local/lib/nodejs/node-v<version>-linux-x64 (root-owned), with
#   /usr/local/bin/{node,npm,npx} linked to it. Ubuntu 24.04's own nodejs is
#   18, which Vite refuses; no third-party apt source is added.
#
#   xz-utils, zstd, python3 and git from apt, when one is missing.
#
# What it does NOT install: the wasm32-unknown-unknown target. That belongs to
# the toolchain rust-toolchain.toml pins, and deploy/redeploy/stage-source.sh
# adds it in the tree it builds, for the user that builds, so it always
# matches the commit being built.
#
# BUMPING NODE: read the new version's line for `linux-x64.tar.xz` in its
# SHASUMS256.txt on nodejs.org, change the two values below together, and run
# the script again on each box; the old directory under /usr/local/lib/nodejs
# may then be removed. Keep the major the one CI's `web` job names.
#
# Idempotent: a box that already runs this exact Node is left alone.
set -euo pipefail

NODE_VERSION=22.23.5
NODE_SHA256=cdeb8670a0c81f51b4dc4ac61919f9b1ad917b304bf8221eb939570663d240f8
NODE_HOME=/usr/local/lib/nodejs
NODE_DIR="$NODE_HOME/node-v${NODE_VERSION}-linux-x64"

if [ "$(id -u)" != 0 ]; then
  echo "build-tools-install: run as root" >&2
  exit 1
fi
if [ "$(uname -m)" != x86_64 ]; then
  echo "build-tools-install: the pinned Node tarball is linux-x64; this box is $(uname -m)" >&2
  exit 1
fi
export DEBIAN_FRONTEND=noninteractive

missing=()
for pair in xz:xz-utils zstd:zstd python3:python3 git:git curl:curl sha256sum:coreutils; do
  command -v "${pair%%:*}" > /dev/null || missing+=("${pair##*:}")
done
if [ "${#missing[@]}" -gt 0 ]; then
  echo "build-tools-install: installing ${missing[*]} from apt"
  apt-get update > /dev/null 2>&1 || echo "apt-get update failed (a stale source?): installing from the lists the box has"
  apt-get install -y --no-install-recommends "${missing[@]}" > /dev/null
fi
for tool in xz zstd python3 git curl sha256sum; do
  command -v "$tool" > /dev/null || { echo "build-tools-install: $tool is missing and apt could not install it" >&2; exit 1; }
done

if [ -x "$NODE_DIR/bin/node" ] && [ "$("$NODE_DIR/bin/node" --version)" = "v$NODE_VERSION" ]; then
  echo "build-tools-install: Node v$NODE_VERSION is already in $NODE_DIR"
else
  # A private root-owned stage (0700): the download is verified before
  # anything is unpacked, and nothing is ever run from a world-writable path.
  stage="$(mktemp -d /tmp/rustmush-node-stage.XXXXXX)"
  chmod 0700 "$stage"
  trap 'rm -rf "$stage"' EXIT
  tarball="$stage/node-v${NODE_VERSION}-linux-x64.tar.xz"
  curl --proto =https --tlsv1.2 -fsSL -o "$tarball" \
    "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz"
  echo "${NODE_SHA256}  ${tarball}" | sha256sum -c -
  install -d -o root -g root -m 0755 "$NODE_HOME"
  rm -rf "$NODE_DIR.new"
  mkdir "$NODE_DIR.new"
  tar -xJf "$tarball" -C "$NODE_DIR.new" --strip-components=1 --no-same-owner
  chown -R root:root "$NODE_DIR.new"
  rm -rf "$NODE_DIR"
  mv "$NODE_DIR.new" "$NODE_DIR"
  echo "build-tools-install: installed Node v$NODE_VERSION in $NODE_DIR"
fi
for tool in node npm npx; do
  ln -sfn "$NODE_DIR/bin/$tool" "/usr/local/bin/$tool"
done
got="$(/usr/local/bin/node --version)"
if [ "$got" != "v$NODE_VERSION" ]; then
  echo "build-tools-install: /usr/local/bin/node says $got, not v$NODE_VERSION" >&2
  exit 1
fi
echo "build-tools-install: node $got, npm $(/usr/local/bin/npm --version), $(zstd --version | head -1), $(python3 --version), $(git --version)"
echo "build-tools-install: done. A whole deploy's sources and web client are now built on this box by deploy/redeploy/stage-source.sh"
