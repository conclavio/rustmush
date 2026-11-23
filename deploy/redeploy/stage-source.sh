#!/usr/bin/env bash
# stage-source.sh — make the two tars stage-build.sh reads, ON THE BOX, from a
# clone of the repository: no other machine builds or ships anything
# (deploy/README.md, "Redeploy" and "Wipe and redeploy", step 1). Until
# 2026-11-23 the owner's Windows box built the web client and shipped both
# tars; a box provisioned with deploy/vps-runner/build-tools-install.sh does
# it itself.
#
#   COMMIT=<sha or ref> [JOBS=4] [BUILD_USER=ghrunner] bash stage-source.sh <instance> [<instance> ...]
#
# COMMIT is a full or short commit id, or a branch (`main`, a PR's branch); it
# must be on the public repository. For each instance named it leaves in
# $REDEPLOY_DIR (default /root):
#   <instance>-tree-<sha8>.tar   `git archive` of the commit (export-subst
#                                stamps .source-commit)
#   <instance>-dist-<sha8>.tar   web/dist built here at that commit (its
#                                .commit stamp equals .source-commit)
# and prints `STAGED <sha8>`, the value stage-build.sh's COMMIT takes.
# NOTHING LIVE CHANGES: no instance, unit, store or config is touched.
#
# Root runs it, and root builds nothing: git, npm and cargo all run as
# BUILD_USER (default ghrunner, the user provision.sh gave a pinned rustup), in
# that user's own directory ~/rustmush-stage, so no package's script and no
# build script of the web client's ever runs as root. Root only copies the two
# finished tars out. Kept between runs there: the clone (repo.git), the wasm
# target directory and npm's cache; the unpacked tree is removed.
#
# It needs: Node (build-tools-install.sh), BUILD_USER's rustup, git, and
# network to github.com, the npm registry and static.rust-lang.org (the first
# run adds the wasm32 target to the toolchain rust-toolchain.toml pins). It
# needs none of the licensed asset packs: every model, look and ground the
# build reads is a baked file committed under crates/ (docs/ops.md, "A box
# builds its own deploy").
set -euo pipefail
trap 'echo "stage-source: FAILED at line $LINENO: $BASH_COMMAND" >&2' ERR

[ "$#" -ge 1 ] || { echo "usage: COMMIT=<sha or ref> [JOBS=4] [BUILD_USER=ghrunner] bash stage-source.sh <instance> [<instance> ...]" >&2; exit 2; }
COMMIT=${COMMIT:?set COMMIT=<sha or ref>}
JOBS=${JOBS:-4}
DIR=${REDEPLOY_DIR:-/root}
BUILD_USER=${BUILD_USER:-ghrunner}
REPO_URL=${REPO_URL:-https://github.com/conclavio/rustmush}
for n in "$@"; do
  if ! [[ "$n" =~ ^[a-z][a-z0-9]*$ ]]; then
    echo "stage-source: not an instance name: $n" >&2
    exit 2
  fi
done
if ! [[ "$COMMIT" =~ ^[A-Za-z0-9._/-]+$ ]] || [[ "$COMMIT" == -* ]]; then
  echo "stage-source: COMMIT is a commit id or a branch name, got '$COMMIT'" >&2
  exit 2
fi
if ! [[ "$JOBS" =~ ^[0-9]+$ ]] || [ "$JOBS" -lt 1 ]; then
  echo "stage-source: JOBS is a whole number, at least 1" >&2
  exit 2
fi
[ "$(id -u)" -eq 0 ] || { echo "stage-source: must run as root (it writes the tars into $DIR)" >&2; exit 1; }
id -u "$BUILD_USER" > /dev/null 2>&1 || { echo "stage-source: no user $BUILD_USER (deploy/vps-runner/provision.sh makes it)" >&2; exit 1; }
if [ "$(id -u "$BUILD_USER")" -eq 0 ]; then
  echo "stage-source: BUILD_USER must not be root" >&2
  exit 1
fi
home="$(getent passwd "$BUILD_USER" | cut -d: -f6)"
[ -x /usr/local/bin/node ] || { echo "stage-source: no /usr/local/bin/node (run deploy/vps-runner/build-tools-install.sh)" >&2; exit 1; }
[ -x "$home/.cargo/bin/rustup" ] || { echo "stage-source: $BUILD_USER has no rustup in $home/.cargo/bin (deploy/vps-runner/provision.sh installs it)" >&2; exit 1; }
[ -d "$DIR" ] || { echo "stage-source: no directory $DIR" >&2; exit 1; }

# As the build user, with ITS home and a PATH that is the pinned Node and its
# own cargo: nothing of root's environment reaches a build.
as_builder() {
  runuser -u "$BUILD_USER" -- env -i HOME="$home" USER="$BUILD_USER" LOGNAME="$BUILD_USER" \
    PATH="$home/.cargo/bin:/usr/local/bin:/usr/bin:/bin" TMPDIR="$home/tmp" \
    CARGO_BUILD_JOBS="$JOBS" CARGO_INCREMENTAL=0 "$@"
}
as_builder mkdir -p "$home/tmp" "$home/rustmush-stage"

echo "== $(date -u +%FT%TZ) fetch $REPO_URL as $BUILD_USER"
# shellcheck disable=SC2016
SHA="$(as_builder bash -c '
  set -euo pipefail
  cd "$HOME/rustmush-stage"
  if [ ! -d repo.git ]; then git clone -q --mirror "$1" repo.git; fi
  git -C repo.git remote set-url origin "$1"
  git -C repo.git fetch -q --prune origin
  git -C repo.git rev-parse --verify --quiet "$2^{commit}"
' _ "$REPO_URL" "$COMMIT")" || { echo "stage-source: $COMMIT is not a commit on $REPO_URL" >&2; exit 1; }
if ! [[ "$SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo "stage-source: could not resolve $COMMIT (got '$SHA')" >&2
  exit 1
fi
S8=${SHA:0:8}
echo "commit: $SHA"

echo "== $(date -u +%FT%TZ) the tree, and the web client at -j $JOBS (Node $(/usr/local/bin/node --version))"
T0=$(date +%s)
# shellcheck disable=SC2016
as_builder bash -c '
  set -euo pipefail
  sha="$1"; s8="${sha:0:8}"
  cd "$HOME/rustmush-stage"
  rm -rf "src-$s8" "tree-$s8.tar" "dist-$s8.tar"
  git -C repo.git archive --format=tar "$sha" > "tree-$s8.tar"
  mkdir "src-$s8"
  tar -xf "tree-$s8.tar" -C "src-$s8"
  [ "$(tr -d "\r\n" < "src-$s8/.source-commit")" = "$sha" ] || { echo "the archive is not stamped with $sha" >&2; exit 1; }
  cd "src-$s8"
  # The toolchain rust-toolchain.toml pins, and its wasm32 target for the
  # cockpit picture (web/scripts/build-wasm.mjs).
  rustup target add wasm32-unknown-unknown > /dev/null
  cd web
  export CARGO_TARGET_DIR="$HOME/rustmush-stage/web-target"
  npm ci --no-audit --no-fund > "$HOME/rustmush-stage/npm-ci-$s8.log" 2>&1 || { tail -20 "$HOME/rustmush-stage/npm-ci-$s8.log" >&2; exit 1; }
  npm run build > "$HOME/rustmush-stage/web-build-$s8.log" 2>&1 || { tail -30 "$HOME/rustmush-stage/web-build-$s8.log" >&2; exit 1; }
  [ -f dist/index.html ] || { echo "the web build left no dist/index.html" >&2; exit 1; }
  [ "$(tr -d "\r\n" < dist/.commit)" = "$sha" ] || { echo "the bundle is stamped $(cat dist/.commit), not $sha" >&2; exit 1; }
  tar -cf "$HOME/rustmush-stage/dist-$s8.tar" dist
  cd "$HOME/rustmush-stage"
  rm -rf "src-$s8"
' _ "$SHA"
echo "the web client built in $(($(date +%s) - T0)) s"

for n in "$@"; do
  for kind in tree dist; do
    src="$home/rustmush-stage/$kind-$S8.tar"
    dst="$DIR/$n-$kind-$S8.tar"
    if [ -L "$src" ] || [ ! -f "$src" ]; then echo "stage-source: $src is missing or a symlink" >&2; exit 1; fi
    if [ -L "$dst" ]; then echo "stage-source: $dst must not be a symlink" >&2; exit 1; fi
    install -o root -g root -m 0600 "$src" "$dst"
  done
  echo "  $DIR/$n-tree-$S8.tar ($(stat -c %s "$DIR/$n-tree-$S8.tar") bytes), $DIR/$n-dist-$S8.tar ($(stat -c %s "$DIR/$n-dist-$S8.tar") bytes)"
done
as_builder rm -f "$home/rustmush-stage/tree-$S8.tar" "$home/rustmush-stage/dist-$S8.tar"
echo "== $(date -u +%FT%TZ) STAGED $S8 (nothing live changed)"
echo "next, for each instance: COMMIT=$S8 JOBS=$JOBS bash stage-build.sh <instance>"
