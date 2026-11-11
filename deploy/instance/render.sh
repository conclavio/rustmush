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
# `mushd` for the all-in-one. A binary that is named but not built fails the
# render, exactly as one that refuses the config does (doc 43 TS.0: a quiet
# skip let the list here drift from the build line unnoticed). Every deploy
# script builds before it renders; to render with no preflight at all, name no
# binary. Either failure comes before the caller has started or reloaded a
# unit.
#
# Nothing is installed until every check has passed: the render must carry no
# unsubstituted `__TOKEN__` and no unresolved `EXTERNAL_IP=auto` marker, it must
# satisfy every binary named, and only then is it moved into place from
# `<out>.new`. A redeploy whose instance.env has gone wrong therefore leaves the
# live config alone.
#
# Two instance.env keys are not tokens: MINDS_LAUNCH and REGISTRY_PULSE_MS
# decide whether lines of the template's commented `[tacsim.minds]` block are
# rendered live (doc 44 cut MW.1; the rule is beside the code below).

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
trap 'rm -f "$SED_SCRIPT" "$STAGED.minds"' EXIT

# The loop READS instance.env; it never sources it. A value there is a sed
# replacement, not a shell word, so `INSTANCE_TITLE=rustmush dev1 (TacSim)` is a
# good line in it and a syntax error to `.` — which would leave every key after
# it unset and render a config with empty ports and an empty start room.
while IFS='=' read -r k v; do
  case "$k" in '' | '#'*) continue ;; esac
  printf 's|__%s__|%s|g\n' "$k" "$v"
done <"$ENV_FILE" >"$SED_SCRIPT"

sed -f "$SED_SCRIPT" "$TEMPLATE" >"$STAGED"

# Doc 44 cut MW.1: the two keys this script reads for ITSELF. Neither is a
# `__TOKEN__`, because what each one decides is whether a line of the
# template's commented `[tacsim.minds]` block is live at all, and a token can
# only fill a line that is already there.
#
#   MINDS_LAUNCH       `on` or `off` (empty is off; anything else is refused).
#                      On, AND with the box's launch document at
#                      <DATA_DIR>/launch.json, the `launch_path` line is
#                      rendered live, naming that file. On with no document,
#                      or off, the line stays the template's comment.
#   REGISTRY_PULSE_MS  empty, or a number: the `registry_pulse_ms` line
#                      rendered live with it (a dev box sets 50, Q-44-41).
#
# Why the document must exist first: a live `launch_path` whose file is
# missing refuses `--check-config`, `--bootstrap-staff` and the boot, and the
# document names ids that only a booted world has printed. So a deploy onto a
# fresh or wiped store renders the line commented whatever the switch says,
# deploy/instance/launch-bringup.py writes the document and renders again, and
# scripts/reset-instance.sh takes the line out again when it wipes the data
# directory the document lives in. An empty path is never written: there is no
# value of either key that renders `launch_path = ""`.
#
# With MINDS_LAUNCH off and REGISTRY_PULSE_MS empty nothing below touches the
# staged file: it is the plain substitution, byte for byte.
env_value() {
  local wanted="$1" key value
  while IFS='=' read -r key value; do
    case "$key" in '' | '#'*) continue ;; esac
    if [ "$key" = "$wanted" ]; then
      # Leading spaces and the CR a Windows-edited instance.env leaves on the
      # line: harmless inside a sed replacement, wrong in a comparison.
      value="${value#"${value%%[![:space:]]*}"}"
      value="${value%"${value##*[![:space:]]}"}"
      printf '%s\n' "$value"
      return 0
    fi
  done <"$ENV_FILE"
  return 0
}

# Make the ONE line matching `$1` (a basic regular expression, anchored by the
# caller) read `$2`. Exactly one line must match: a template whose minds block
# was reworded is a refusal here, not a silently commented launch.
uncomment_line() {
  local pattern="$1" replacement="$2" count
  count="$(grep -c -- "$pattern" "$STAGED" || true)"
  if [ "$count" != 1 ]; then
    echo "$0: $TEMPLATE has $count lines matching '$pattern', expected 1; $OUT is untouched" >&2
    rm -f "$STAGED"
    exit 1
  fi
  sed "s|$pattern|$replacement|" "$STAGED" >"$STAGED.minds"
  mv "$STAGED.minds" "$STAGED"
}

minds_launch="$(env_value MINDS_LAUNCH)"
registry_pulse="$(env_value REGISTRY_PULSE_MS)"
launch_live=0
case "$minds_launch" in
'' | off) ;;
on)
  data_dir="$(env_value DATA_DIR)"
  if [ -z "$data_dir" ]; then
    echo "$0: MINDS_LAUNCH=on needs DATA_DIR in $ENV_FILE (the launch document is <DATA_DIR>/launch.json)" >&2
    rm -f "$STAGED"
    exit 1
  fi
  if [ -f "$data_dir/launch.json" ]; then
    launch_live=1
  else
    echo "$0: MINDS_LAUNCH=on, but $data_dir/launch.json does not exist yet: launch_path stays commented (deploy/instance/launch-bringup.py writes it)" >&2
  fi
  ;;
*)
  echo "$0: MINDS_LAUNCH must be 'on' or 'off' in $ENV_FILE, got '$minds_launch'" >&2
  rm -f "$STAGED"
  exit 1
  ;;
esac
case "$registry_pulse" in
*[!0-9]*)
  echo "$0: REGISTRY_PULSE_MS must be empty or a number of milliseconds in $ENV_FILE, got '$registry_pulse'" >&2
  rm -f "$STAGED"
  exit 1
  ;;
esac
if [ "$launch_live" = 1 ] || [ -n "$registry_pulse" ]; then
  uncomment_line '^# \[tacsim\.minds\]$' '[tacsim.minds]'
fi
if [ "$launch_live" = 1 ]; then
  uncomment_line '^# launch_path = .*$' "launch_path = \"$data_dir/launch.json\""
fi
if [ -n "$registry_pulse" ]; then
  uncomment_line '^# registry_pulse_ms = .*$' "registry_pulse_ms = $registry_pulse"
fi

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
    echo "$0: $binary is not built (no executable at that path); $OUT is untouched and nothing was started" >&2
    rm -f "$STAGED"
    exit 1
  fi
  if ! "${words[@]}" --check-config --config "$STAGED"; then
    echo "$0: $binary refuses the render; $OUT is untouched and nothing was started" >&2
    rm -f "$STAGED"
    exit 1
  fi
done

mv "$STAGED" "$OUT"
echo "rendered $OUT from $ENV_FILE"
