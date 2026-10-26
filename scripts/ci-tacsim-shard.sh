#!/usr/bin/env bash
# Run one shard of tacsim's integration test targets: line N of
# .github/tacsim-integration-shards.txt, `N: target target ...`, as one
# `cargo test -p tacsim --test <target>...` (CI's `tacsim-integration-N`).
set -euo pipefail
shard="${1:?usage: scripts/ci-tacsim-shard.sh <shard number>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
line="$(grep -E "^${shard}:" "$root/.github/tacsim-integration-shards.txt" || true)"
if [ -z "$line" ]; then
  echo "no tacsim integration shard ${shard} in .github/tacsim-integration-shards.txt" >&2
  exit 1
fi
args=()
for target in ${line#*:}; do
  args+=(--test "$target")
done
echo "tacsim integration shard ${shard}: $(( ${#args[@]} / 2 )) targets"
cargo test -p tacsim "${args[@]}" --no-fail-fast -j 2
