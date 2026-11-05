#!/usr/bin/env bash
# Run cell N of tacsim's integration tests (CI's `tacsim-integration-N`), as
# .github/tacsim-integration-shards.txt describes them: first the targets
# line `N:` names, whole (harness on 1, cost_lane on 2), then cell N's hash
# share of the domain binaries the `partition:` line names,
# `cargo nextest run --partition hash:N/<cells>` (doc 43 cut TS.12, adopted
# by D-43-5 on 2026-11-04: every cell builds all four domain binaries and the
# balance is the hash's, so nothing is resharded). Both runs use
# .config/nextest.toml's `ci` profile (no fail-fast, a hung test terminated at
# 20 min). The first run's JUnit report is kept beside the second's as
# target/nextest/ci/junit-first.xml, and the cell uploads both. tacsim's
# doc-tests are the tacsim-lib cell's.
set -euo pipefail
cell="${1:?usage: scripts/ci-tacsim-shard.sh <cell number>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$root/.github/tacsim-integration-shards.txt"
line="$(grep -E "^${cell}:" "$manifest" || true)"
if [ -z "$line" ]; then
  echo "no tacsim integration cell ${cell} in .github/tacsim-integration-shards.txt" >&2
  exit 1
fi
cells="$(grep -cE '^[0-9]+:' "$manifest")"
partition="$(grep -E '^partition:' "$manifest" | head -1)"
if [ -z "$partition" ]; then
  echo "no partition: line in .github/tacsim-integration-shards.txt" >&2
  exit 1
fi
first=()
for target in ${line#*:}; do
  first+=(--test "$target")
done
domains=()
for target in ${partition#*:}; do
  domains+=(--test "$target")
done
rc=0
if [ "${#first[@]}" -gt 0 ]; then
  echo "tacsim integration cell ${cell}: first $(( ${#first[@]} / 2 )) whole target(s)"
  cargo nextest run -P ci -p tacsim "${first[@]}" || rc=$?
  if [ -f "$root/target/nextest/ci/junit.xml" ]; then
    mv "$root/target/nextest/ci/junit.xml" "$root/target/nextest/ci/junit-first.xml"
  fi
fi
echo "tacsim integration cell ${cell}: hash:${cell}/${cells} of $(( ${#domains[@]} / 2 )) domain binaries"
cargo nextest run -P ci -p tacsim "${domains[@]}" --partition "hash:${cell}/${cells}" || rc=$?
exit "$rc"
