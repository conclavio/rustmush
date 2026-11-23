#!/usr/bin/env bash
# The soak (doc 51 §4.1, unit FL.2): the story suites, repeated, the way a
# hosted CI cell runs them, with no retry, so a flake is seen on main before a
# pull request meets it. .github/workflows/soak.yml runs this nightly on
# conclav.dev; a session runs the same script by hand on the same box.
#
#   scripts/soak.sh                       the night's soak: six cells, 180 min
#   SOAK_CELLS=1 SOAK_PASSES=1 scripts/soak.sh    one cell, one pass of each
#                                                 package: the measurement
#
# WHAT RUNS. The binaries that run a real server or process, a package at a
# time as CI's cells build them (`-p <package>`, so each has the feature set
# its own CI cell has; one invocation over all four would unify them):
#
#   mushd         sim_lenses world_journeys durable_process
#   mush-gateway  split_halves
#   mush-server   split_boundary commerce_composition
#   mush-bot      arena_e2e arena_contract mining_cohort
#
# REDIS_URL is unset in a cell, so the `redis` group's tests take their
# in-memory or skip branch: one shared instance cannot serve six cells.
#
# HOW. One build, then SOAK_CELLS cells side by side. A cell is pinned to its
# own SOAK_CORES cores (taskset) and runs
#
#   cargo nextest run --binaries-metadata <pkg> --cargo-metadata <all>
#       --workspace-remap <the cell's copy of the tree> -P soak --test-threads 4
#
# for each package in turn, round and round, until the window closes; no
# invocation starts after it. Each cell starts its round at a different
# package. A cell has its own copy of the tracked tree (its tests' working
# directory, and where nextest keeps the report) and its own TMPDIR, wiped
# between invocations, so two cells running one test at once share nothing
# but the box. The build is not archived: the stories reach their server
# binary through compile-time paths (env!("CARGO_BIN_EXE_mushd")), which
# point into the build's own target directory wherever a test binary is
# copied, so on one box an archive would isolate nothing and cost its
# extraction on every pass.
#
# nextest's own stress flags (--stress-duration) were tried and not used:
# one invocation is one build, so one feature set, and it writes its one
# report at the very end, so a stopped cell would lose the night.
#
# WHAT COMES OUT, under SOAK_OUT:
#   cell-N/pass-K-<package>.xml   one JUnit report an invocation
#   cell-N/pass-K-<package>.log   nextest's own output for it
#   cell-N/cell.log               one line an invocation: seconds, exit code
#   box.tsv                       free memory and load, sampled
#   soak.json                     the head, the window, the counts
# scripts/flake_report.py --junit <SOAK_OUT> reads it. A failing test does
# not fail this script; the report decides. This script fails when the soak
# itself did not work: no build, no report, or the box ran out of memory.
#
# THE BOX. conclav.dev has 32 cores, 62 GB and no swap, and went down once
# under load (2026-11-14). The soak bounds itself:
#   * six cells of four cores: 24 tests at once at most, cores 24-31 left free;
#   * it waits for a quiet box before it starts (load, free memory), and
#     yields with a warning if another run still holds the box after
#     SOAK_WAIT_MINUTES: it never piles on;
#   * a watchdog samples MemAvailable every 15 s and stops every cell, and
#     fails the run, below SOAK_MIN_FREE_MB. The runner's unit also holds the
#     whole job under its MemoryHigh (deploy/vps-runner/). A cell has no hard
#     limit of its own: the runner's user has no systemd manager to ask for a
#     scope, and taskset pins cores only;
#   * on any exit it kills every process that carries this run's mark, so a
#     stopped soak leaves no server behind.
#
# Knobs (environment):
#   SOAK_MINUTES=180        the window
#   SOAK_PASSES=            rounds a cell, instead of the window (by hand)
#   SOAK_CELLS=6  SOAK_CORES=4  SOAK_FIRST_CORE=0
#   SOAK_PACKAGES="mushd mush-gateway mush-server mush-bot"
#   SOAK_DIR=$RUNNER_TEMP/soak (else target/soak)     the cells' working space
#   SOAK_OUT=soak-reports
#   SOAK_MIN_FREE_MB=6144  SOAK_MAX_START_LOAD=8  SOAK_WAIT_MINUTES=30
#   SOAK_MIN_DISK_GB=40
#   SOAK_SKIP_BUILD=1       reuse SOAK_DIR/build from an earlier run (by hand)

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MINUTES="${SOAK_MINUTES:-180}"
PASSES="${SOAK_PASSES:-}"
CELLS="${SOAK_CELLS:-6}"
CORES="${SOAK_CORES:-4}"
FIRST_CORE="${SOAK_FIRST_CORE:-0}"
read -r -a PACKAGES <<< "${SOAK_PACKAGES:-mushd mush-gateway mush-server mush-bot}"
WORK="${SOAK_DIR:-${RUNNER_TEMP:-$REPO/target}/soak}"
OUT="${SOAK_OUT:-$REPO/soak-reports}"
MIN_FREE_MB="${SOAK_MIN_FREE_MB:-6144}"
MAX_START_LOAD="${SOAK_MAX_START_LOAD:-8}"
WAIT_MINUTES="${SOAK_WAIT_MINUTES:-30}"
MIN_DISK_GB="${SOAK_MIN_DISK_GB:-40}"

tests_of() {
  case "$1" in
    mushd) echo "sim_lenses world_journeys durable_process" ;;
    mush-gateway) echo "split_halves" ;;
    mush-server) echo "split_boundary commerce_composition" ;;
    mush-bot) echo "arena_e2e arena_contract mining_cohort" ;;
    *) echo "soak: no story suites are named for package '$1'" >&2; return 1 ;;
  esac
}

for number in "$MINUTES" "$CELLS" "$CORES" "$FIRST_CORE" "$MIN_FREE_MB" "$WAIT_MINUTES" "$MIN_DISK_GB" ${PASSES:+"$PASSES"}; do
  [[ "$number" =~ ^[0-9]+$ ]] || { echo "soak: '$number' is not a whole number" >&2; exit 2; }
done
[[ "$MAX_START_LOAD" =~ ^[0-9]+(\.[0-9]+)?$ ]] || { echo "soak: SOAK_MAX_START_LOAD is not a number" >&2; exit 2; }
for package in "${PACKAGES[@]}"; do tests_of "$package" > /dev/null; done

free_mb() { awk '/^MemAvailable:/ { print int($2 / 1024) }' /proc/meminfo; }
load_now() { cut -d' ' -f1 /proc/loadavg; }
now_s() { date +%s; }
stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# sweep SIGNAL NAME VALUE: signal every process whose environment holds
# NAME=VALUE. Every process of a run carries SOAK_RUN (the cells, nextest,
# each test and the servers a test starts), and everything one invocation
# started carries its cell's SOAK_CELL as well.
sweep() {
  local signal="$1" mark="$2=$3" environ pid
  for environ in /proc/[0-9]*/environ; do
    pid="${environ#/proc/}"
    pid="${pid%/environ}"
    [ "$pid" = "$$" ] && continue
    if grep -qzxF -- "$mark" "$environ" 2> /dev/null; then
      kill "-$signal" "$pid" 2> /dev/null || true
    fi
  done
}

# --- one cell (the script re-runs itself as `__cell N` in its own session) ---

run_cell() {
  # Short paths: a story's Unix sockets live under its TMPDIR, and a socket
  # path may be 108 bytes at most.
  local n="$1" src="$WORK/c$1" tmp="$WORK/t$1" out="$OUT/cell-$1"
  local first=$(( FIRST_CORE + ($1 - 1) * CORES ))
  local last=$(( first + CORES - 1 )) count="${#PACKAGES[@]}"
  mkdir -p "$src" "$tmp" "$out"
  # The tracked tree as it stands (a by-hand run may carry a patch).
  (cd "$REPO" && git ls-files -z | tar --null --no-recursion --ignore-failed-read -T - -cf -) | tar -xf - -C "$src"
  echo "cell $n: cores $first-$last, tree $src" > "$out/cell.log"
  local round=0 at package began took code report
  while :; do
    round=$(( round + 1 ))
    if [ -n "$PASSES" ] && [ "$round" -gt "$PASSES" ]; then break; fi
    for (( at = 0; at < count; at++ )); do
      package="${PACKAGES[$(( (n - 1 + at) % count ))]}"
      if [ -z "$PASSES" ] && [ "$(now_s)" -ge "$SOAK_DEADLINE" ]; then break 2; fi
      began="$(now_s)"
      code=0
      # STORY_PHASE=random: doc 51 §3.6. A rig that knows the variable boots
      # at a random phase of the IC calendar and prints it (unit FL.4 on).
      env -u REDIS_URL TMPDIR="$tmp" STORY_PHASE=random SOAK_CELL="$SOAK_RUN.$n" \
        taskset -c "$first-$last" \
        cargo nextest run \
          --binaries-metadata "$WORK/build/$package.binaries.json" \
          --cargo-metadata "$WORK/build/cargo-metadata.json" \
          --workspace-remap "$src" \
          -P soak --test-threads "$CORES" \
          > "$out/pass-$round-$package.log" 2>&1 || code=$?
      took=$(( $(now_s) - began ))
      report="$src/target/nextest/soak/junit.xml"
      if [ -f "$report" ]; then
        mv -f "$report" "$out/pass-$round-$package.xml"
        echo "pass $round $package: ${took}s exit $code" >> "$out/cell.log"
      else
        echo "pass $round $package: ${took}s exit $code NO REPORT" >> "$out/cell.log"
      fi
      # A server a test left running would otherwise add up over the night.
      sweep KILL SOAK_CELL "$SOAK_RUN.$n"
      find "$tmp" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2> /dev/null || true
    done
  done
  echo "cell $n: done" >> "$out/cell.log"
}

if [ "${1:-}" = "__cell" ]; then
  run_cell "$2"
  exit 0
fi

# --- the run -----------------------------------------------------------------

command -v taskset > /dev/null || { echo "soak: taskset is not installed" >&2; exit 2; }
needed=$(( FIRST_CORE + CELLS * CORES ))
if [ "$(nproc)" -lt "$needed" ]; then
  echo "soak: $CELLS cells of $CORES cores from core $FIRST_CORE need $needed cores; this box has $(nproc)" >&2
  exit 2
fi

mkdir -p "$OUT" "$WORK"
# Last run's reports, and nothing else that may be under SOAK_OUT.
rm -rf "$OUT"/cell-* "$OUT/box.tsv" "$OUT/soak.json"
[ -n "${SOAK_SKIP_BUILD:-}" ] || rm -rf "$WORK/build"
rm -rf "$WORK"/c[0-9]* "$WORK"/t[0-9]*
mkdir -p "$WORK/build"

head_sha="$(git -C "$REPO" rev-parse HEAD)"
branch="${GITHUB_REF_NAME:-$(git -C "$REPO" rev-parse --abbrev-ref HEAD)}"
link=""
if [ -n "${GITHUB_RUN_ID:-}" ]; then
  link="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
fi
started="$(stamp)"
SOAK_RUN="soak-$$-$(now_s)"
export SOAK_RUN
min_free="$(free_mb)"
max_load="$(load_now)"
state="ran"

write_summary() {
  local reports
  reports="$(find "$OUT" -name 'pass-*.xml' | wc -l)"
  printf '{"head": "%s", "branch": "%s", "link": "%s", "started": "%s", "ended": "%s", "state": "%s", "minutes": %s, "cells": %s, "cores": %s, "packages": "%s", "reports": %s, "min_free_mb": %s, "max_load": %s}\n' \
    "$head_sha" "$branch" "$link" "$started" "$(stamp)" "$state" "$MINUTES" "$CELLS" "$CORES" \
    "${PACKAGES[*]}" "$reports" "$min_free" "$max_load" > "$OUT/soak.json"
}

cells=()
stop_cells() {
  local pid
  for pid in "${cells[@]}"; do kill -TERM -- "-$pid" 2> /dev/null || true; done
  # nextest ends its tests on SIGTERM. A cancelled Actions job has about ten
  # seconds before the runner kills this script, so the cells get six and
  # the sweep ends whatever is left.
  for _ in $(seq 1 6); do
    local alive=0
    for pid in "${cells[@]}"; do kill -0 "$pid" 2> /dev/null && alive=1; done
    [ "$alive" = 0 ] && break
    sleep 1
  done
  sweep KILL SOAK_RUN "$SOAK_RUN"
}
finish() {
  local code=$?
  trap - EXIT
  trap '' INT TERM
  if [ "${#cells[@]}" -gt 0 ]; then stop_cells; fi
  write_summary || true
  exit "$code"
}
trap finish EXIT
trap 'state="stopped"; exit 130' INT TERM

echo "::group::The box"
echo "$(hostname): $(nproc) cores, ${min_free} MB free, load $(cat /proc/loadavg)"
df -h "$WORK" | tail -1
echo "::endgroup::"

disk_gb="$(df --output=avail -BG "$WORK" | tail -1 | tr -dc '0-9')"
if [ "$disk_gb" -lt "$MIN_DISK_GB" ]; then
  state="no disk"
  echo "::error title=soak::$disk_gb GB free under $WORK; the soak wants $MIN_DISK_GB. Free the disk (old target directories under /home/ghrunner are the usual cause)."
  exit 1
fi

# A quiet box, or wait for one; never pile on.
want_free=$(( MIN_FREE_MB + CELLS * 4096 ))
waited=0
until awk -v l="$(load_now)" -v m="$MAX_START_LOAD" 'BEGIN { exit !(l <= m) }' && [ "$(free_mb)" -ge "$want_free" ]; do
  if [ "$waited" -ge "$WAIT_MINUTES" ]; then
    state="yielded"
    echo "::warning title=soak did not run::the box was busy for $WAIT_MINUTES min (load $(load_now), limit $MAX_START_LOAD; ${want_free} MB free wanted, $(free_mb) there). The soak yields to whatever is running and ran no pass tonight."
    exit 0
  fi
  echo "the box is busy (load $(load_now), $(free_mb) MB free); waiting a minute ($waited of $WAIT_MINUTES)"
  sleep 60 &
  wait $!
  waited=$(( waited + 1 ))
done

# One build: each package's story binaries with its own feature set, and the
# two metadata files a cell runs from. The target directory is cargo's
# (CARGO_TARGET_DIR): the same binaries serve every cell and every pass.
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(( CELLS * CORES > 4 ? CELLS * CORES : 4 ))}"
if [ -z "${SOAK_SKIP_BUILD:-}" ]; then
  cd "$REPO"
  cargo metadata --format-version=1 --all-features --no-deps > "$WORK/build/cargo-metadata.json"
  for package in "${PACKAGES[@]}"; do
    echo "::group::build $package"
    targets=()
    for target in $(tests_of "$package"); do targets+=(--test "$target"); done
    cargo nextest list -p "$package" "${targets[@]}" --list-type binaries-only --message-format json \
      > "$WORK/build/$package.binaries.json"
    echo "::endgroup::"
  done
fi

SOAK_DEADLINE=$(( $(now_s) + MINUTES * 60 ))
export SOAK_DEADLINE SOAK_MINUTES="$MINUTES" SOAK_PASSES="$PASSES" SOAK_CELLS="$CELLS" SOAK_CORES="$CORES"
export SOAK_FIRST_CORE="$FIRST_CORE" SOAK_PACKAGES="${PACKAGES[*]}" SOAK_DIR="$WORK" SOAK_OUT="$OUT"

if [ -n "$PASSES" ]; then
  echo "$CELLS cell(s) of $CORES cores, $PASSES round(s) each of: ${PACKAGES[*]}"
else
  echo "$CELLS cell(s) of $CORES cores for $MINUTES min, round and round: ${PACKAGES[*]}"
fi
for n in $(seq 1 "$CELLS"); do
  setsid "${BASH_SOURCE[0]}" __cell "$n" &
  cells+=("$!")
done

# The watchdog: sample the box while a cell lives; stop everything when free
# memory falls under the floor.
printf 'time\tfree_mb\tload1\n' > "$OUT/box.tsv"
while :; do
  alive=0
  for pid in "${cells[@]}"; do kill -0 "$pid" 2> /dev/null && alive=1; done
  [ "$alive" = 1 ] || break
  free="$(free_mb)"
  load="$(load_now)"
  printf '%s\t%s\t%s\n' "$(stamp)" "$free" "$load" >> "$OUT/box.tsv"
  [ "$free" -lt "$min_free" ] && min_free="$free"
  max_load="$(awk -v a="$load" -v b="$max_load" 'BEGIN { print (a > b) ? a : b }')"
  if [ "$free" -lt "$MIN_FREE_MB" ]; then
    state="out of memory"
    echo "::error title=soak stopped::$free MB of memory free, under the floor of $MIN_FREE_MB: every cell was stopped before the box could go down. Something on the box grew; see box.tsv and the last pass logs."
    exit 3
  fi
  sleep 15 &
  wait $!
done
cells=()
sweep KILL SOAK_RUN "$SOAK_RUN"

echo "::group::What ran"
cat "$OUT"/cell-*/cell.log
echo "::endgroup::"
reports="$(find "$OUT" -name 'pass-*.xml' | wc -l)"
echo "$reports report(s); least free memory ${min_free} MB; highest load ${max_load}"
if grep -q "NO REPORT" "$OUT"/cell-*/cell.log; then
  state="no report"
  echo "::error title=soak::an invocation wrote no JUnit report (nextest did not start, or was killed); its pass log says why"
  exit 1
fi
if [ "$reports" -eq 0 ]; then
  state="no report"
  echo "::error title=soak::no pass ran inside the window"
  exit 1
fi
