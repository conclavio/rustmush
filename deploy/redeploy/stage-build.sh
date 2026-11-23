#!/usr/bin/env bash
# stage-build.sh — stage and build ONE instance's new tree BESIDE the live one.
# The first step on the box of deploy/README.md's "Wipe and redeploy", as run on
# dev1 and dev2 on 2026-11-22 (main ebe5840f). NOTHING LIVE CHANGES: the live
# build dir, the units, the store and the config are untouched, so it can run
# for both instances at once while they serve.
#
#   COMMIT=<sha8> [JOBS=4] bash stage-build.sh <instance>
#
# <instance> is the name under /opt/rustmush (dev1, dev2, ...). In
# $REDEPLOY_DIR (default /root) it expects the two tars the dev box shipped:
#   <instance>-tree-<sha8>.tar   `git archive --format=tar <sha>` (export-subst
#                                stamps .source-commit)
#   <instance>-dist-<sha8>.tar   web/dist built at the same commit (its .commit
#                                stamp must match; scripts/check-web-bundle.sh)
# It makes /opt/rustmush/<instance>/build.new from them, copies the live tree's
# target/ in so the build is warm, builds mush-server and mush-gateway
# (--features tls) at -j $JOBS, and leaves build.new/.stage-built naming the
# commit for wipe-cutover.sh to check. Its log is $REDEPLOY_DIR/<instance>-build-<UTC>.log.
#
# Run it detached and watch for its last line, not for the process (a pattern
# that greps the process list matches the ssh command that started it):
#   (COMMIT=<sha8> setsid nohup bash stage-build.sh dev1 > /root/dev1-stage-<sha8>.log 2>&1 < /dev/null &)
#   ...until grep -q 'BUILT <sha8>' /root/dev1-stage-<sha8>.log; do sleep 10; done
set -euo pipefail
trap 'echo "stage-build: FAILED at line $LINENO: $BASH_COMMAND" >&2' ERR

N=${1:?usage: COMMIT=<sha8> [JOBS=4] bash stage-build.sh <instance>}
COMMIT=${COMMIT:?set COMMIT=<sha8>}
JOBS=${JOBS:-4}
DIR=${REDEPLOY_DIR:-/root}
if ! [[ "$N" =~ ^[a-z][a-z0-9]*$ ]]; then
  echo "stage-build: not an instance name: $N" >&2
  exit 2
fi
if ! [[ "$COMMIT" =~ ^[0-9a-f]{8}$ ]]; then
  echo "stage-build: COMMIT must be the commit's first 8 hex digits, got '$COMMIT'" >&2
  exit 2
fi
ROOT=/opt/rustmush/$N
TREE="$DIR/$N-tree-$COMMIT.tar"
DIST="$DIR/$N-dist-$COMMIT.tar"
TS=$(date -u +%Y%m%d-%H%M%S)
LOG="$DIR/$N-build-$TS.log"
[ -d "$ROOT/build" ] || { echo "stage-build: no live tree at $ROOT/build" >&2; exit 1; }
[ -f "$TREE" ] || { echo "stage-build: missing $TREE" >&2; exit 1; }
[ -f "$DIST" ] || { echo "stage-build: missing $DIST" >&2; exit 1; }
if [ -L "$ROOT" ] || [ -L "$ROOT/build" ] || [ -L "$ROOT/build.new" ]; then
  echo "stage-build: $ROOT, its build and build.new must not be symlinks" >&2
  exit 1
fi

echo "== $(date -u +%FT%TZ) $N: stage build.new beside the live tree (running: $(cat "$ROOT/build/.source-commit" 2>/dev/null || echo unknown))"
rm -rf "$ROOT/build.new"
mkdir "$ROOT/build.new"
tar -xf "$TREE" -C "$ROOT/build.new"
if [ -d "$ROOT/build/target" ]; then
  cp -a "$ROOT/build/target" "$ROOT/build.new/target"
fi
rm -rf "$ROOT/build.new/web/dist"
tar -xf "$DIST" -C "$ROOT/build.new/web"
# git-archive mtimes can predate the copied target dir: make every source newer,
# or cargo takes the old artifacts for fresh.
find "$ROOT/build.new" -path "$ROOT/build.new/target" -prune -o -type f -print0 | xargs -0 touch
STAGED="$(cat "$ROOT/build.new/.source-commit")"
case "$STAGED" in
  "$COMMIT"*) ;;
  *) echo "stage-build: $TREE is commit $STAGED, not $COMMIT" >&2; exit 1 ;;
esac
echo "staged: $STAGED / bundle $(cat "$ROOT/build.new/web/dist/.commit")"
bash "$ROOT/build.new/scripts/check-web-bundle.sh" "$ROOT/build.new"

echo "== $(date -u +%FT%TZ) $N: build both halves at -j $JOBS (log $LOG)"
# shellcheck source=/dev/null
. "${CARGO_ENV:-$HOME/.cargo/env}"
cd "$ROOT/build.new"
T0=$(date +%s)
cargo build -j "$JOBS" --release --locked -p mush-server > "$LOG" 2>&1
echo "server built in $(($(date +%s) - T0)) s"
T1=$(date +%s)
cargo build -j "$JOBS" --release --locked -p mush-gateway --features tls >> "$LOG" 2>&1
echo "gateway built in $(($(date +%s) - T1)) s"
ls -la target/release/mush-gateway target/release/mush-server
echo "warnings: $(grep -c '^warning' "$LOG" || true)"
echo "IPC: server $(target/release/mush-server --ipc-version) gateway $(target/release/mush-gateway --ipc-version)"
printf '%s\n' "$STAGED" > "$ROOT/build.new/.stage-built"
echo "== $(date -u +%FT%TZ) $N: BUILT $COMMIT (staged in $ROOT/build.new; nothing live changed)"
