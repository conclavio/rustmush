#!/usr/bin/env bash
#
# vps-gate.sh — the gate on the VPS runner, from the box (test-strategy cut
# T.2, 2026-10-26): push what the tree holds, dispatch .github/workflows/gate.yml
# on it, wait for the run, download its reports and say what failed. Under bash
# (Linux, or Git Bash on Windows), from anywhere in the tree, with `gh` logged
# in as a collaborator (a dispatch needs write access: the runner is the
# owner's production box, see gate.yml).
#
#   scripts/vps-gate.sh                      the sweep of the pushed branch
#   scripts/vps-gate.sh --wip                the sweep of the WORKING TREE: the
#                                            uncommitted work included, as a
#                                            commit pushed to gate/<branch>
#                                            (never to the branch itself), the
#                                            tree and the index untouched
#   scripts/vps-gate.sh --packages "tacsim mush-server"
#                                            only those packages
#   scripts/vps-gate.sh --extra "-E 'test(sm12c)'"
#                                            extra `cargo nextest run` arguments
#   scripts/vps-gate.sh --no-redis           no REDIS_URL (the in-memory branches)
#   scripts/vps-gate.sh --kind harness       scripts/harness-weekly.sh on the VPS
#   scripts/vps-gate.sh --kind cost-lane     scripts/cost-lane.sh on the VPS
#   scripts/vps-gate.sh --ref <branch>       a branch already on origin, not pushed here
#   scripts/vps-gate.sh --no-push            the current branch as origin has it
#   scripts/vps-gate.sh --runner conclav-dev the box, by label: `vps` (the default
#                                            for a sweep: whichever box is idle),
#                                            `conclav-dev` (32 cores) or `conclav-io`
#                                            (8 cores; the default for harness and
#                                            cost-lane, whose timing gates are live
#                                            on that host alone)
#
# The cut loop's gate runner calls `scripts/vps-gate.sh --wip` for the
# workspace sweep, after its targeted commands, instead of running the sweep on
# the box (the owner's test-once rule of 2026-10-26). The run's reports land
# under target/gate/<run id>/ (gate-junit-<n>.xml, one per nextest invocation: one <testcase> a test with its
# time, a <failure> for each failed test, a <flakyFailure> for a pass on
# retry); the exit status is the run's.
set -euo pipefail

kind=sweep
runner=""
packages=""
extra=""
redis=true
wip=0
ref=""
push=1
while [ $# -gt 0 ]; do
  case "$1" in
    --kind) kind="$2"; shift 2 ;;
    --runner) runner="$2"; shift 2 ;;
    --packages) packages="$2"; shift 2 ;;
    --extra) extra="$2"; shift 2 ;;
    --no-redis) redis=false; shift ;;
    --wip) wip=1; shift ;;
    --ref) ref="$2"; shift 2 ;;
    --no-push) push=0; shift ;;
    -h|--help) sed -n '2,37p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$kind" in sweep|harness|cost-lane) ;; *) echo "--kind is sweep, harness or cost-lane" >&2; exit 2 ;; esac
# A sweep goes to whichever box is idle; the harness and the cost lane go to
# conclav.io, the host whose name blessed their baselines (their timing gates
# are dormant anywhere else), unless the dispatch says otherwise.
if [ -z "$runner" ]; then
  if [ "$kind" = sweep ]; then runner=vps; else runner=conclav-io; fi
fi
case "$runner" in vps|conclav-dev|conclav-io) ;; *) echo "--runner is vps, conclav-dev or conclav-io" >&2; exit 2 ;; esac

cd "$(dirname "${BASH_SOURCE[0]}")/.."
repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
branch="$(git rev-parse --abbrev-ref HEAD)"

if [ "$wip" = 1 ]; then
  # A commit of the working tree, untracked files included, built through a
  # copy of the index so the real index and the tree are not touched, pushed
  # to gate/<branch>. The branch itself is never written.
  index="$(mktemp)"
  cp "$(git rev-parse --git-path index)" "$index"
  GIT_INDEX_FILE="$index" git add -A
  tree="$(GIT_INDEX_FILE="$index" git write-tree)"
  rm -f "$index"
  commit="$(git commit-tree "$tree" -p HEAD -m "gate: the tree of $branch at $(date -u +%FT%TZ), uncommitted work included (scripts/vps-gate.sh --wip)")"
  ref="gate/$branch"
  git push -q --force origin "$commit:refs/heads/$ref"
  echo "pushed the working tree as $commit to $ref"
elif [ -z "$ref" ]; then
  ref="$branch"
  if [ "$push" = 1 ]; then
    git push -q -u origin "$ref"
    echo "pushed $ref"
  fi
fi

since="$(date -u +%s)"
# GitHub finds a workflow to dispatch by its file on the DEFAULT branch (the
# ref's copy is what runs); until gate.yml is on main the dispatch is a 404.
if ! gh workflow run gate.yml --repo "$repo" --ref "$ref" \
  -f "kind=$kind" -f "runner=$runner" -f "packages=$packages" -f "extra=$extra" -f "redis=$redis"; then
  echo "the dispatch failed: if gate.yml is not on the default branch yet, nothing can dispatch it (a 404 above); otherwise see the error" >&2
  exit 1
fi

# The dispatch returns no run id: the newest gate run on the ref created
# since the dispatch is it (a few seconds, usually).
id=""
for _ in $(seq 1 30); do
  sleep 5
  id="$(gh run list --repo "$repo" --workflow gate.yml --branch "$ref" --limit 5 \
        --json databaseId,createdAt \
        --jq "[.[] | select((.createdAt | fromdateiso8601) >= $since - 10)] | .[0].databaseId // empty")"
  if [ -n "$id" ]; then break; fi
done
if [ -z "$id" ]; then
  echo "no gate run appeared on $ref within 150 s; is the runner online? (gh api repos/$repo/actions/runners)" >&2
  exit 1
fi
echo "gate run $id ($kind on $ref, runner $runner): https://github.com/$repo/actions/runs/$id"

# Wait. `gh run watch` can drop on a network timeout (the client, not the
# run), so it is retried until the run is complete.
while :; do
  if gh run watch "$id" --repo "$repo" --interval 30 --exit-status > /dev/null 2>&1; then
    break
  fi
  status="$(gh run view "$id" --repo "$repo" --json status --jq .status 2>/dev/null || echo unknown)"
  if [ "$status" = completed ]; then break; fi
  sleep 20
done
conclusion="$(gh run view "$id" --repo "$repo" --json conclusion --jq .conclusion)"

out="target/gate/$id"
mkdir -p "$out"
gh run download "$id" --repo "$repo" -D "$out" 2>/dev/null || true
# One JUnit per nextest invocation (the engine sweep, the bot), each a whole
# <testsuites>; totals over all of them, then each failed and flaky test.
junits="$(find "$out" -name 'gate-junit-*.xml' | sort || true)"
if [ -n "$junits" ]; then
  # The first python that RUNS: on the owner's Windows box `python3` is the
  # Store's stub (it exits 49 and installs nothing) and `python` is the one.
  py=""
  for cand in python3 python; do
    if "$cand" -c pass > /dev/null 2>&1; then py="$cand"; break; fi
  done
  if [ -z "$py" ]; then
    echo "no python on PATH: the reports are under $out/, unparsed" >&2
  else
  # shellcheck disable=SC2086
  "$py" -X utf8 - $junits <<'EOF'
import sys
import xml.etree.ElementTree as ET
suites = []
for path in sys.argv[1:]:
    suites += ET.parse(path).getroot().findall("testsuite")
tests = sum(int(s.get("tests", 0)) for s in suites)
failures = sum(int(s.get("failures", 0)) for s in suites)
errors = sum(int(s.get("errors", 0)) for s in suites)
seconds = sum(float(c.get("time", 0)) for s in suites for c in s.findall("testcase"))
print(f"junit: {tests} tests, {failures} failed, {errors} errors, {len(suites)} binaries, {seconds:.0f} s of test time")
for s in suites:
    for case in s.findall("testcase"):
        if case.find("failure") is not None or case.find("error") is not None:
            print(f"  FAIL  {case.get('classname')} {case.get('name')}  {case.get('time')}s")
        elif case.find("flakyFailure") is not None:
            print(f"  FLAKY {case.get('classname')} {case.get('name')}  {case.get('time')}s")
EOF
  fi
fi
echo "gate run $id: $conclusion (reports under $out/)"
if [ "$conclusion" != success ]; then
  gh run view "$id" --repo "$repo" --log-failed 2>/dev/null | tail -n 150 || true
  exit 1
fi
