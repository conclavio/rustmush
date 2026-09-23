#!/usr/bin/env bash
# Read-only preflight, shared by deployment and its local regression tests.
set -euo pipefail
build="${1:?usage: check-web-bundle.sh <build-root>}"
marker="$build/.source-commit"
expected=""
if [ -f "$marker" ]; then expected="$(tr -d '\r\n' < "$marker")"; fi
if ! [[ "$expected" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]]; then
  expected="$(git -C "$build" rev-parse HEAD 2>/dev/null || true)"
fi
if ! [[ "$expected" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]]; then
  echo "FATAL: no source commit found in $build (use a checkout or git archive)" >&2
  exit 1
fi
actual=""
if [ -f "$build/web/dist/.commit" ]; then actual="$(tr -d '\r\n' < "$build/web/dist/.commit")"; fi
if [ ! -f "$build/web/dist/index.html" ] || [ "$actual" != "$expected" ]; then
  echo "FATAL: web bundle commit '${actual:-missing}' does not match source $expected; cd $build/web && npm ci && npm run build" >&2
  exit 1
fi
echo "web bundle: source commit $expected matches"
