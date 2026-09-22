#!/usr/bin/env bash
# Read-only deploy guard exercised against disposable archive-shaped fixtures.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/web/dist"
a=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
b=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
printf '%s\n' "$a" > "$fixture/.source-commit"
printf '<html></html>\n' > "$fixture/web/dist/index.html"
refuses() {
  if bash "$root/scripts/check-web-bundle.sh" "$fixture" >/dev/null 2>&1; then
    echo 'guard accepted a missing/stale bundle' >&2; exit 1
  fi
}
refuses
printf '%s\n' "$a" > "$fixture/web/dist/.commit"
bash "$root/scripts/check-web-bundle.sh" "$fixture"
# Rollback: a freshly touched bundle for B must not pass archived source A.
printf '%s\n' "$b" > "$fixture/web/dist/.commit"
touch "$fixture/web/dist/index.html"
refuses
# Changed source commit with old bundle also refuses, regardless of source path.
printf '%s\n' "$b" > "$fixture/.source-commit"
printf '%s\n' "$a" > "$fixture/web/dist/.commit"
refuses
printf '%s\n' "$b" > "$fixture/web/dist/.commit"
bash "$root/scripts/check-web-bundle.sh" "$fixture"
rm "$fixture/web/dist/index.html"
refuses
echo 'bundle guard: missing stamp, rollback, source mismatch and missing index passed'
