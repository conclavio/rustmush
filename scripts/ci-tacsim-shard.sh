#!/usr/bin/env bash
# Run cell N of tacsim's integration tests (CI's `tacsim-integration-N`), as
# .github/tacsim-integration-shards.txt describes them: first the targets
# line `N:` names, whole (harness on 1, cost_lane on 2), then cell N's share
# of the domain binaries the `partition:` line names. The share is planned by
# measured time (scripts/tacsim-cells.py plan; 2026-11-18, replacing
# `--partition hash:N/10`, under which one cell held the slow 512-site field
# stories): every cell lists the same tests and computes the same plan, so
# each test runs in one cell. Both runs use .config/nextest.toml's `ci`
# profile (no fail-fast, a hung test terminated at 20 min). The first run's
# JUnit report is kept beside the second's as target/nextest/ci/junit-first.xml,
# and the cell uploads both. tacsim's doc-tests are the tacsim-lib cell's.
set -euo pipefail
cell="${1:?usage: scripts/ci-tacsim-shard.sh <cell number>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$root/.github/tacsim-integration-shards.txt"
line="$(grep -E "^${cell}:" "$manifest" || true)"
if [ -z "$line" ]; then
  echo "no tacsim integration cell ${cell} in .github/tacsim-integration-shards.txt" >&2
  exit 1
fi
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
listing="$root/target/nextest/tacsim-cells-list.json"
mkdir -p "$(dirname "$listing")"
cargo nextest list -P ci -p tacsim "${domains[@]}" --message-format json > "$listing"
filter="$(python3 "$root/scripts/tacsim-cells.py" plan "$cell" "$manifest" "$listing")"
echo "tacsim integration cell ${cell}: its planned share of $(( ${#domains[@]} / 2 )) domain binaries"
cargo nextest run -P ci -p tacsim "${domains[@]}" -E "$filter" || rc=$?
exit "$rc"
