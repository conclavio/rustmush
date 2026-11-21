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
# Environment:
#   RENDER_CHECK_USER  when set and running as root, execute each trailing
#                      `binary --check-config` unprivileged via `runuser -u`
#                      so root never executes service binaries directly.
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
# satisfy every binary named, and only then is it moved into place from a
# uniquely created sibling staging file (`mktemp`). A redeploy whose
# instance.env has gone wrong therefore leaves the live config alone.
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
  if [ ! -f "$required" ] || [ -L "$required" ]; then
    echo "$0: missing or symlinked input file: $required" >&2
    exit 1
  fi
done

if [ -L "$OUT" ]; then
  echo "$0: refusing to overwrite symlink output path: $OUT" >&2
  exit 1
fi

OUT_DIR="$(dirname "$OUT")"
OUT_BASE="$(basename "$OUT")"
STAGED="$(mktemp "$OUT_DIR/.${OUT_BASE}.new.XXXXXX")"
MINDS_TMP=""
cleanup() {
  rm -f "$STAGED" ${MINDS_TMP:+"$MINDS_TMP"}
}
trap cleanup EXIT

# Pure literal string substitution via awk `index()`/`substr()` — never
# `source`d and never fed into GNU `sed -f` (where `|`, `&`, `\`, `e`, or `w`
# in instance.env values can inject commands or file writes). Every `__KEY__`
# token is replaced literally with the value on the first `KEY=value` line.
awk '
  FNR == NR {
    line = $0
    sub(/\r$/, "", line)
    eq = index(line, "=")
    if (eq == 0) next
    k = substr(line, 1, eq - 1)
    if (k == "" || substr(k, 1, 1) == "#") next
    if (!(k in seen)) {
      seen[k] = 1
      n++
      keys[n] = "__" k "__"
      vals[n] = substr(line, eq + 1)
    }
    next
  }
  {
    line = $0
    for (i = 1; i <= n; i++) {
      pat = keys[i]
      rep = vals[i]
      plen = length(pat)
      out = ""
      while ((pos = index(line, pat)) > 0) {
        out = out substr(line, 1, pos - 1) rep
        line = substr(line, pos + plen)
      }
      line = out line
    }
    print line
  }
' "$ENV_FILE" "$TEMPLATE" >"$STAGED"

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
# With MINDS_LAUNCH off and REGISTRY_PULSE_MS empty nothing below touches the
# staged file: it is the plain substitution, byte for byte.
env_value() {
  local wanted="$1" key value
  while IFS='=' read -r key value; do
    case "$key" in '' | '#'*) continue ;; esac
    if [ "$key" = "$wanted" ]; then
      # Leading spaces and the CR a Windows-edited instance.env leaves on the
      # line.
      value="${value#"${value%%[![:space:]]*}"}"
      value="${value%"${value##*[![:space:]]}"}"
      printf '%s\n' "$value"
      return 0
    fi
  done <"$ENV_FILE"
  return 0
}

# Make the ONE line matching `$2` (by exact equality or literal prefix, per
# `$1`) read `$3` literally. Passes strings via ENVIRON (not `awk -v` or `sed`)
# so backslashes, `&`, `|`, and regex metacharacters are never interpreted.
# Exactly one line must match: a template whose minds block was reworded is a
# refusal here, not a silently commented launch.
uncomment_line() {
  local mode="$1" needle="$2" replacement="$3"
  MINDS_TMP="$(mktemp "$OUT_DIR/.${OUT_BASE}.minds.XXXXXX")"
  if ! RENDER_MODE="$mode" RENDER_NEEDLE="$needle" RENDER_REP="$replacement" \
    awk '
      BEGIN {
        mode = ENVIRON["RENDER_MODE"]
        needle = ENVIRON["RENDER_NEEDLE"]
        rep = ENVIRON["RENDER_REP"]
        nlen = length(needle)
        hits = 0
      }
      {
        matched = 0
        if (mode == "exact") {
          matched = ($0 == needle)
        } else if (mode == "prefix") {
          matched = (substr($0, 1, nlen) == needle)
        }
        if (matched) {
          hits++
          print rep
        } else {
          print $0
        }
      }
      END {
        if (hits != 1) {
          print hits > "/dev/stderr"
          exit 3
        }
      }
    ' "$STAGED" >"$MINDS_TMP" 2>/dev/null; then
    echo "$0: $TEMPLATE does not have exactly 1 line matching '$needle'; $OUT is untouched" >&2
    exit 1
  fi
  mv "$MINDS_TMP" "$STAGED"
  MINDS_TMP=""
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
    exit 1
  fi
  case "$data_dir" in
  *\"* | *\\*)
    echo "$0: DATA_DIR contains unsafe TOML string characters: $data_dir" >&2
    exit 1
    ;;
  esac
  if [ -f "$data_dir/launch.json" ] && [ ! -L "$data_dir/launch.json" ]; then
    launch_live=1
  else
    echo "$0: MINDS_LAUNCH=on, but $data_dir/launch.json does not exist yet: launch_path stays commented (deploy/instance/launch-bringup.py writes it)" >&2
  fi
  ;;
*)
  echo "$0: MINDS_LAUNCH must be 'on' or 'off' in $ENV_FILE, got '$minds_launch'" >&2
  exit 1
  ;;
esac
case "$registry_pulse" in
*[!0-9]*)
  echo "$0: REGISTRY_PULSE_MS must be empty or a number of milliseconds in $ENV_FILE, got '$registry_pulse'" >&2
  exit 1
  ;;
esac
if [ "$launch_live" = 1 ] || [ -n "$registry_pulse" ]; then
  uncomment_line exact '# [tacsim.minds]' '[tacsim.minds]'
fi
if [ "$launch_live" = 1 ]; then
  uncomment_line prefix '# launch_path = ' "launch_path = \"$data_dir/launch.json\""
fi
if [ -n "$registry_pulse" ]; then
  uncomment_line prefix '# registry_pulse_ms = ' "registry_pulse_ms = $registry_pulse"
fi

# deploy/README.md's completeness check: a token left in a LIVE (non-comment)
# line means a key with no line in instance.env, and the config it would install
# holds the literal `__START_ROOM__`.
if grep -n '^[^#]*__' "$STAGED"; then
  echo "$0: INCOMPLETE $ENV_FILE — the lines above still carry a token" >&2
  exit 1
fi

# Unresolved deploy-time marker check: `EXTERNAL_IP=auto` must be replaced by
# deploy.sh before rendering. Whole-word, live lines only.
if grep -n '^[^#]*\bauto\b' "$STAGED"; then
  echo "$0: UNRESOLVED $ENV_FILE — the lines above carry the deploy-time marker" >&2
  echo "$0: 'auto' is not an address; the deploy resolves EXTERNAL_IP before rendering" >&2
  exit 1
fi

# Readable by the unprivileged check user (and installed config is 0644).
chmod 0644 "$STAGED"

# The binaries' own preflight (WS04-S3). `--check-config` opens neither the
# store, nor a socket, nor the quarantine spool's lifetime lock, so it is safe
# to run beside the instance this render is about to replace the config of.
#
# A trailing argument may carry the binary's own flags after its path, split
# on whitespace ("<path>/mush-gateway --generation a"): the split deploys
# preflight the gateway AS a generation (doc 33 §4.4). No path under
# /opt/rustmush has a space.
for spec in "$@"; do
  read -r -a words <<<"$spec"
  binary="${words[0]}"
  if [ ! -x "$binary" ]; then
    echo "$0: $binary is not built (no executable at that path); $OUT is untouched and nothing was started" >&2
    exit 1
  fi
  if [ -n "${RENDER_CHECK_USER:-}" ] && [ "$(id -u)" -eq 0 ]; then
    if ! runuser -u "$RENDER_CHECK_USER" -- "${words[@]}" --check-config --config "$STAGED"; then
      echo "$0: $binary refuses the render; $OUT is untouched and nothing was started" >&2
      exit 1
    fi
  else
    if ! "${words[@]}" --check-config --config "$STAGED"; then
      echo "$0: $binary refuses the render; $OUT is untouched and nothing was started" >&2
      exit 1
    fi
  fi
done

mv "$STAGED" "$OUT"
echo "rendered $OUT from $ENV_FILE"
