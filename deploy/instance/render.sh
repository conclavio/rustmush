#!/usr/bin/env bash
#
# render.sh — turn one `instance.env` plus the shared template into one
# `mushd.toml`, and prove the binaries that will read it accept it.
#
# THE render recipe lives here. It is written out in deploy/README.md and in
# mushd.toml.tmpl's own header as well, but those are prose: a deploy script
# that keeps its own copy of the loop is a second render that can disagree with
# the first one about what a complete config is, and the disagreement shows up
# as a box that boots with `__START_ROOM__` in its config. Both deploy scripts
# call this; so does CI's provisioning job.
#
# Usage:
#   render.sh <instance.env> <mushd.toml.tmpl> <out.toml> [binary ...]
#
# Each trailing `binary` is preflighted with its own `--check-config` over the
# rendered file — `mush-gateway` and `mush-server` for the split deployment,
# `mushd` for the all-in-one. A binary that is not built yet is skipped with a
# note rather than failing the render: the FIRST deploy of a box renders the
# config before anything has been compiled. A binary that IS there and refuses
# the config fails this script, and it does so before the caller has started or
# reloaded a unit.
#
# Nothing is installed until every check has passed: the render must carry no
# unsubstituted `__TOKEN__` and no unresolved `EXTERNAL_IP=auto` marker, it must
# satisfy every binary named, and only then is it moved into place from
# `<out>.new`. A redeploy whose instance.env has gone wrong therefore leaves the
# live config alone.

set -euo pipefail

usage() {
  echo "usage: $0 <instance.env> <mushd.toml.tmpl> <out.toml> [binary ...]" >&2
  exit 2
}

[ "$#" -ge 3 ] || usage
ENV_FILE="$1"
TEMPLATE="$2"
OUT="$3"
shift 3

for required in "$ENV_FILE" "$TEMPLATE"; do
  if [ ! -f "$required" ]; then
    echo "$0: no such file: $required" >&2
    exit 1
  fi
done

STAGED="$OUT.new"
SED_SCRIPT="$STAGED.sed"
trap 'rm -f "$SED_SCRIPT"' EXIT

# The loop READS instance.env; it never sources it. A value there is a sed
# replacement, not a shell word, so `INSTANCE_TITLE=rustmush dev1 (TacSim)` is a
# good line in it and a syntax error to `.` — which would leave every key after
# it unset and render a config with empty ports and an empty start room.
while IFS='=' read -r k v; do
  case "$k" in '' | '#'*) continue ;; esac
  printf 's|__%s__|%s|g\n' "$k" "$v"
done <"$ENV_FILE" >"$SED_SCRIPT"

sed -f "$SED_SCRIPT" "$TEMPLATE" >"$STAGED"

# deploy/README.md's completeness check: a token left in a LIVE (non-comment)
# line means a key with no line in instance.env, and the config it would install
# holds the literal `__START_ROOM__`. Comment lines are exempt because the
# template's own header spells the mechanism out with `__KEY__` tokens — and
# `crates/mush-config/tests/shipped_configs.rs` applies the same rule to the
# same lines, so the shell and the test cannot disagree about what a complete
# render is. Spelled as an `if` rather than `grep ... && { ...; }` because this
# script runs under `set -e`, where a grep that correctly finds nothing would
# end the script.
if grep -n '^[^#]*__' "$STAGED"; then
  echo "$0: INCOMPLETE $ENV_FILE — the lines above still carry a token" >&2
  rm -f "$STAGED"
  exit 1
fi

# The OTHER incomplete render, and the one the grep above cannot see. A
# committed instance.env that resolves its address at deploy time writes
# `EXTERNAL_IP=auto` — a marker, not an address — and the deploy script replaces
# that whole line before it calls this script. Rendered unresolved it produces
# `[ssh].listen = "auto:6789"` (the web listener's address is its own key,
# WEB_BIND, since the boxes went behind Caddy), which no `--check-config` below
# refuses (`[ssh].listen` is on no preflight list), so the
# symptom would be a box that boots clean, logs nothing wrong, and answers on no
# port. That refusal belongs HERE, with the completeness rule it is a case of,
# and not only in CI and in crates/mush-config/tests/shipped_configs.rs — an
# operator re-rendering by hand from the COMMITTED env runs neither of those.
# Whole-word, and live lines only, exactly as those two spell it.
if grep -n '^[^#]*\bauto\b' "$STAGED"; then
  echo "$0: UNRESOLVED $ENV_FILE — the lines above carry the deploy-time marker" >&2
  echo "$0: 'auto' is not an address; the deploy resolves EXTERNAL_IP before rendering" >&2
  rm -f "$STAGED"
  exit 1
fi

# The binaries' own preflight (WS04-S3). `--check-config` opens neither the
# store, nor a socket, nor the quarantine spool's lifetime lock, so it is safe
# to run beside the instance this render is about to replace the config of.
#
# A trailing argument may carry the binary's own flags after its path, split
# on whitespace ("<path>/mush-gateway --generation a"): the split deploys
# preflight the gateway AS a generation (doc 33 §4.4), because without
# `--generation` a symlinked [ssh].trusted_socket — the generations' handoff
# link — is a refusal. No path under /opt/rustmush has a space.
for spec in "$@"; do
  read -r -a words <<<"$spec"
  binary="${words[0]}"
  if [ ! -x "$binary" ]; then
    echo "$0: $binary is not built yet — skipping its preflight" >&2
    continue
  fi
  if ! "${words[@]}" --check-config --config "$STAGED"; then
    echo "$0: $binary refuses the render; $OUT is untouched and nothing was started" >&2
    rm -f "$STAGED"
    exit 1
  fi
done

mv "$STAGED" "$OUT"
echo "rendered $OUT from $ENV_FILE"
