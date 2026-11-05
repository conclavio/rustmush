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
# Doc 43 TS.12 EXPERIMENT (measurement branch, never merged): every cell
# builds the four domain binaries and runs its hash share of their tests.
# The extra targets its line names (harness on 1, cost_lane on 2) run first,
# so the domain run's JUnit is the one the cell uploads (the experiment's data;
# the extras' report is kept beside it, not uploaded).
rc=0
if [ "${#args[@]}" -gt 0 ]; then
  cargo nextest run -P ci -p tacsim "${args[@]}" || rc=$?
  cp target/nextest/ci/junit.xml target/nextest/ci/junit-extra.xml 2>/dev/null || true
fi
domains=(--test domain_field --test domain_economy --test domain_arena --test domain_world)
cargo nextest run -P ci -p tacsim "${domains[@]}" --partition "hash:${shard}/10" || rc=$?
exit "$rc"
