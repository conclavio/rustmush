#!/usr/bin/env bash
# The gateway swap (deploy/<inst>-split/gateway-swap.sh over the shared
# deploy/drain/swap.sh) and the deploy's binary comparison
# (deploy/drain/binary-changed.sh), driven against a stub `systemctl` in a
# disposable fixture (doc 33 §4.7, cut GG.4). Nothing here touches the box.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT

# ---------------------------------------------------------------------------
# binary-changed.sh: by content, never by path or mtime.
# ---------------------------------------------------------------------------
changed="$root/deploy/drain/binary-changed.sh"
printf 'one' > "$fixture/new"
printf 'one' > "$fixture/same"
printf 'two' > "$fixture/other"
touch -d '2001-01-01' "$fixture/same"
[[ "$(bash "$changed" "$fixture/new" "$fixture/same")" == unchanged ]]
[[ "$(bash "$changed" "$fixture/new" "$fixture/other")" == changed ]]
[[ "$(bash "$changed" "$fixture/new" "$fixture/missing")" == changed ]]
if bash "$changed" "$fixture/missing" "$fixture/same" 2>/dev/null; then
  echo 'binary-changed.sh accepted an unreadable candidate' >&2; exit 1
fi
if bash "$changed" "$fixture/new" 2>/dev/null; then
  echo 'binary-changed.sh accepted one argument' >&2; exit 1
fi

# ---------------------------------------------------------------------------
# A stub systemctl. Unit states live in $STUB_STATE/<unit>; every call is
# appended to $STUB_LOG. `start` makes a unit active (or failed, when
# $STUB_FAIL_START names it); `stop` makes it inactive; `kill --signal=TERM`
# finishes it (inactive); `kill --signal=USR1` plays the daemon's drain: the
# unit stays active and leaves its marker in $STUB_MARKERS.
# ---------------------------------------------------------------------------
mkdir -p "$fixture/bin" "$fixture/state" "$fixture/markers" "$fixture/run"
cat > "$fixture/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$STUB_LOG"
case "$1" in
  show)
    unit="${*: -1}"
    cat "$STUB_STATE/$unit" 2>/dev/null || echo inactive
    ;;
  start)
    unit="$2"
    if [[ "$unit" == "${STUB_FAIL_START:-}" ]]; then
      echo failed > "$STUB_STATE/$unit"
    else
      echo active > "$STUB_STATE/$unit"
    fi
    ;;
  stop)
    echo inactive > "$STUB_STATE/$2"
    ;;
  kill)
    unit="$3"
    case "$2" in
      --signal=TERM) echo inactive > "$STUB_STATE/$unit" ;;
      --signal=USR1) touch "$STUB_MARKERS/draining-${unit##*@}" ;;
      *) echo "stub systemctl: unexpected kill $*" >&2; exit 1 ;;
    esac
    ;;
  enable|disable|reset-failed) ;;
  *) echo "stub systemctl: unexpected $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$fixture/bin/systemctl"

export PATH="$fixture/bin:$PATH"
export STUB_STATE="$fixture/state" STUB_LOG="$fixture/log" STUB_MARKERS="$fixture/markers"
export GATEWAY_SWAP_MARKER_DIR="$fixture/markers"
export GATEWAY_SWAP_LINK="$fixture/run/dev1.sock"
export GATEWAY_SWAP_LINK_TARGET="$fixture/run/dev1.%g.sock"
export GATEWAY_SWAP_CHECK="true"
export GATEWAY_SWAP_LOCK="$fixture/swap.lock"
export SWAP_STAY_UP_SECS=0
swap="$root/deploy/dev1-split/gateway-swap.sh"
unit=dev1mush-gateway
# The sockets each generation would have bound.
: > "$fixture/run/dev1.a.sock"
: > "$fixture/run/dev1.b.sock"

logged() { grep -qxF -- "$1" "$STUB_LOG"; }
not_logged() { ! grep -qxF -- "$1" "$STUB_LOG"; }
state() { cat "$STUB_STATE/$1" 2>/dev/null || echo inactive; }
link_target() { readlink "$GATEWAY_SWAP_LINK"; }
reset_log() { : > "$STUB_LOG"; }

# S1. Nothing runs: the swap starts a, enables it, points the link, drains nobody.
reset_log
bash "$swap" dev1
[[ "$(state $unit@a)" == active ]]
logged "start $unit@a"
logged "enable --quiet $unit@a"
not_logged "kill --signal=USR1 $unit@a"
[[ "$(link_target)" == "$fixture/run/dev1.a.sock" ]]
echo 'S1 first start: a serving, link -> a'

# S2. a serves: the swap starts b, re-points the link after the stay-up check,
# enables b, disables a, drains a (which leaves its marker and stays active).
reset_log
bash "$swap"
[[ "$(state $unit@b)" == active && "$(state $unit@a)" == active ]]
logged "start $unit@b"
logged "enable --quiet $unit@b"
logged "disable --quiet $unit@a"
logged "kill --signal=USR1 $unit@a"
[[ -e "$fixture/markers/draining-a" ]]
[[ "$(link_target)" == "$fixture/run/dev1.b.sock" ]]
# The link was replaced by rename: no stray temporary link is left behind.
[[ -z "$(find "$fixture/run" -name 'dev1.sock.swap.*')" ]]
echo 'S2 swap: b serving, a draining, link -> b'

# S3. a still drains: a third generation is refused and nothing changes.
reset_log
if bash "$swap" 2>"$fixture/refusal"; then
  echo 'a swap beside a draining generation was not refused' >&2; exit 1
fi
grep -q 'still draining' "$fixture/refusal"
not_logged "start $unit@a"
not_logged "start $unit@b"
[[ "$(link_target)" == "$fixture/run/dev1.b.sock" ]]
echo 'S3 third generation refused'

# S4. --force finishes the draining a (TERM), starts it again, drains b.
reset_log
bash "$swap" dev1 --force
logged "kill --signal=TERM $unit@a"
logged "start $unit@a"
logged "kill --signal=USR1 $unit@b"
[[ -e "$fixture/markers/draining-b" ]]
[[ ! -e "$fixture/markers/draining-a" ]]
[[ "$(link_target)" == "$fixture/run/dev1.a.sock" ]]
echo 'S4 --force: a serving again, b draining'

# S5. The successor does not stay up: it is stopped, reset-failed and
# disabled, the link still points at the old generation, nobody is drained.
rm -f "$fixture/markers/draining-b"
echo inactive > "$STUB_STATE/$unit@b"
reset_log
if STUB_FAIL_START="$unit@b" bash "$swap" 2>"$fixture/stayup"; then
  echo 'a successor that did not stay up was accepted' >&2; exit 1
fi
grep -q 'did not stay up' "$fixture/stayup"
logged "stop $unit@b"
logged "reset-failed $unit@b"
logged "disable --quiet $unit@b"
not_logged "kill --signal=USR1 $unit@a"
not_logged "disable --quiet $unit@a"
[[ "$(link_target)" == "$fixture/run/dev1.a.sock" ]]
echo 'S5 stay-up failure: a keeps serving'

# S6. The wrapper is dev1's: another instance's name is a usage error.
if bash "$swap" dev2 2>/dev/null; then
  echo 'the dev1 wrapper accepted dev2' >&2; exit 1
fi
echo 'S6 instance guard'

# S7. A box without the doorway (dev2 on 2026-10-31, cut GG.5): the rendered
# config has no [ssh].trusted_socket, so there is no link to re-point, and the
# swap must not refuse the new generation for a socket it never binds. The
# link overrides are withdrawn; the wrapper reads the fixture's config.
printf '[ssh]\nlisten = "127.0.0.1:0"\n' > "$fixture/mushd.toml"
rm -f "$fixture/run/dev1.sock"
reset_log
env -u GATEWAY_SWAP_LINK -u GATEWAY_SWAP_LINK_TARGET GATEWAY_SWAP_CONFIG="$fixture/mushd.toml" \
  bash "$swap" dev1 > "$fixture/nolink"
grep -q 'no \[ssh\].trusted_socket' "$fixture/nolink"
[[ "$(state $unit@b)" == active ]]
logged "start $unit@b"
logged "kill --signal=USR1 $unit@a"
[[ -e "$fixture/markers/draining-a" ]]
[[ ! -e "$fixture/run/dev1.sock" ]]
echo 'S7 no trusted socket configured: swapped without a link'

# S8. The link comes from the rendered config when it names one: --force
# finishes the draining a, starts it, and the link points at a's own socket.
printf '[ssh]\nlisten = "127.0.0.1:0"\ntrusted_socket = "%s"\n' "$fixture/run/dev1.sock" > "$fixture/mushd.toml"
reset_log
env -u GATEWAY_SWAP_LINK -u GATEWAY_SWAP_LINK_TARGET GATEWAY_SWAP_CONFIG="$fixture/mushd.toml" \
  bash "$swap" dev1 --force
logged "kill --signal=TERM $unit@a"
logged "start $unit@a"
logged "kill --signal=USR1 $unit@b"
[[ "$(link_target)" == "$fixture/run/dev1.a.sock" ]]
echo 'S8 trusted socket from the config: link -> a'

echo 'gateway swap: first start, swap, refused third, --force, stay-up failure, instance guard, no-doorway swap, config-derived link, binary comparison passed'
