#!/usr/bin/env bash
# Run one shard of tacsim's integration test targets: line N of
# .github/tacsim-integration-shards.txt, `N: target target ...`, as one
# `cargo nextest run -P ci -p tacsim --test <target>...` (CI's
# `tacsim-integration-N`; nextest since test-strategy cut T.1, 2026-10-26:
# the shard's binaries' tests run in one pool of four, each in its own
# process, under .config/nextest.toml's `ci` profile — no fail-fast, a hung
# test terminated at 20 min, the JUnit report at target/nextest/ci/junit.xml).
# No build-jobs cap since 2026-10-31 (`--build-jobs 2` was the link cap the
# old `-j 2` was, from the loop-device era). tacsim's doc-tests are the
# tacsim-lib shard's.
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
cargo nextest run -P ci -p tacsim "${args[@]}"
