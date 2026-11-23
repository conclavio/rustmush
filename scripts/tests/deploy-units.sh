#!/usr/bin/env bash
# The split deploys' systemd helpers (deploy/drain/units.sh), driven against a
# stub `systemctl` in a disposable fixture. Nothing here touches a box.
#
# The stub models what a Redis restart did on dev1 and dev2 on 2026-11-22 (and
# what systemd 259 does with throwaway units on conclav.dev): `systemctl
# restart <redis>` returns once Redis is back, with the units that Requires= it
# still on their way up again: a gateway generation `inactive` with its start
# job waiting, the server `activating`. They come up after $STUB_POLLS polls
# of `list-jobs`. A held server (doc 44 cut MW.1) is not started at all.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/state" "$fixture/enabled" "$fixture/markers"

cat > "$fixture/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$STUB_LOG"
st() { cat "$STUB_STATE/$1" 2>/dev/null || echo inactive; }
pending="$STUB_STATE/.pending"
case "$1" in
  list-jobs)
    # One poll: a restart in flight comes one step closer to done.
    if [ -f "$pending" ]; then
      read -r left units < "$pending"
      if [ "$left" -le 0 ]; then
        for u in $units; do echo active > "$STUB_STATE/$u"; done
        rm -f "$pending"
      else
        echo "$((left - 1)) $units" > "$pending"
        n=100
        for u in $units; do n=$((n + 1)); echo "$n $u start waiting"; done
      fi
    fi
    ;;
  show)
    unit="${*: -1}"
    case "$3" in
      ActiveState) st "$unit" ;;
      MainPID) echo 0 ;;
      *) echo "stub systemctl: unexpected $*" >&2; exit 1 ;;
    esac
    ;;
  is-active) [ "$(st "$3")" = active ] ;;
  is-enabled) [ -e "$STUB_ENABLED/$3" ] ;;
  enable) touch "$STUB_ENABLED/$2" ;;
  disable) rm -f "$STUB_ENABLED/$2" ;;
  start) echo active > "$STUB_STATE/$2" ;;
  reset-failed) ;;
  restart)
    # Only Redis is restarted here. Its running dependents go down with it and
    # their starts are queued; a server under its hold is not started again.
    [ "$2" = redis-t1mush.service ] || { echo "stub systemctl: unexpected $*" >&2; exit 1; }
    moved=""
    for u in t1mush-server.service t1mush-gateway@a.service t1mush-gateway@b.service; do
      if [ "$(st "$u")" = active ]; then
        if [ "$u" = t1mush-server.service ]; then
          if [ -e "$STUB_HOLD" ]; then echo inactive > "$STUB_STATE/$u"; continue; fi
          echo activating > "$STUB_STATE/$u"
        else
          echo inactive > "$STUB_STATE/$u"
        fi
        moved="$moved $u"
      fi
    done
    if [ -n "$moved" ]; then echo "$STUB_POLLS$moved" > "$pending"; fi
    ;;
  *) echo "stub systemctl: unexpected $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$fixture/bin/systemctl"
export PATH="$fixture/bin:$PATH"
export STUB_STATE="$fixture/state" STUB_ENABLED="$fixture/enabled" STUB_LOG="$fixture/log"
export STUB_HOLD="$fixture/server.hold" STUB_POLLS=3 UNITS_SETTLE_POLL=0

GW_UNIT=t1mush-gateway
# (read by deploy/drain/units.sh, sourced below)
# shellcheck disable=SC2034
GW_STATE="$fixture/markers"
# shellcheck source=/dev/null
. "$root/deploy/drain/units.sh"

SETTLE=(redis-t1mush.service t1mush-server.service "$GW_UNIT@a.service" "$GW_UNIT@b.service")
fresh() {
  rm -f "$fixture/state/"* "$fixture/state/.pending" "$fixture/enabled/"* "$fixture/markers/"* "$STUB_HOLD"
  : > "$STUB_LOG"
}
state() { cat "$STUB_STATE/$1" 2>/dev/null || echo inactive; }
enabled() { [ -e "$STUB_ENABLED/$1" ]; }
disabled() { ! enabled "$1"; }
unheld() { ! server_held "$1"; }
logged() { grep -qxF -- "$1" "$STUB_LOG"; }
not_logged() { ! grep -qxF -- "$1" "$STUB_LOG"; }
serving_b() {
  echo active > "$STUB_STATE/redis-t1mush.service"
  echo active > "$STUB_STATE/t1mush-server.service"
  echo active > "$STUB_STATE/$GW_UNIT@b.service"
  touch "$STUB_ENABLED/$GW_UNIT@b.service"
}

# U1. The race of 2026-11-22: b serves, Redis restarts. Read at once, no
# generation serves (what the deploy took for a first start); after
# units_settle, b serves again and the server is back.
fresh
serving_b
systemctl restart redis-t1mush.service
if serving_gen > /dev/null; then
  echo 'the stub did not model the restart: a generation served in the middle of it' >&2; exit 1
fi
[[ "$(state t1mush-server.service)" == activating ]]
units_settle 30 "${SETTLE[@]}" > "$fixture/settle.out"
grep -q 'waiting for systemd to finish with t1mush-server.service (start job waiting)' "$fixture/settle.out"
[[ "$(serving_gen)" == b ]]
[[ "$(state t1mush-server.service)" == active ]]
gens_one_enabled
not_logged "start $GW_UNIT@a.service"
echo 'U1 a propagated Redis restart: settled, b serves again, nothing else started'

# U2. The same with the world HELD: the server is not brought back, b is.
fresh
serving_b
echo inactive > "$STUB_STATE/t1mush-server.service"
touch "$STUB_HOLD"
systemctl restart redis-t1mush.service
units_settle 30 "${SETTLE[@]}" > /dev/null
[[ "$(serving_gen)" == b ]]
[[ "$(state t1mush-server.service)" == inactive ]]
echo 'U2 held: b serves again, the server stays down'

# U3. units_settle gives up, naming what is still moving.
fresh
serving_b
STUB_POLLS=1000 systemctl restart redis-t1mush.service
if units_settle 5 "${SETTLE[@]}" > /dev/null 2> "$fixture/timeout"; then
  echo 'units_settle did not give up on a job that never ends' >&2; exit 1
fi
grep -q 'systemd is still moving t1mush-server.service (start job waiting) after 5 s' "$fixture/timeout"
echo 'U3 a job that never ends: units_settle gives up and names it'

# U4. A unit between states with no job queued (a restart's hold-off) is busy too.
fresh
echo deactivating > "$STUB_STATE/$GW_UNIT@a.service"
[[ "$(units_busy "${SETTLE[@]}")" == "$GW_UNIT@a.service (deactivating)" ]]
echo active > "$STUB_STATE/$GW_UNIT@a.service"
if units_busy "${SETTLE[@]}" > /dev/null; then echo 'an active unit was taken for busy' >&2; exit 1; fi
echo 'U4 a unit between states is busy; an active one is not'

# U5. The first start leaves ONE generation enabled: b, left enabled but not
# running, is disabled when a starts; a's stale marker is cleared.
fresh
touch "$STUB_ENABLED/$GW_UNIT@b.service" "$fixture/markers/draining-a"
gen_first_start a > "$fixture/first"
[[ "$(state "$GW_UNIT@a.service")" == active ]]
enabled "$GW_UNIT@a.service"
disabled "$GW_UNIT@b.service"
[[ ! -e "$fixture/markers/draining-a" ]]
grep -q "disabling $GW_UNIT@b" "$fixture/first"
gens_one_enabled
echo 'U5 first start: a enabled and serving, the left-over b disabled'

# U6. A draining a (active, marker, disabled by its swap): b starts beside it,
# a drains on, b is the only enabled one.
fresh
echo active > "$STUB_STATE/$GW_UNIT@a.service"
touch "$fixture/markers/draining-a"
[[ "$(draining_gen)" == a ]]
if serving_gen > /dev/null; then echo 'a draining generation was taken for serving' >&2; exit 1; fi
gen_first_start b > /dev/null
[[ "$(serving_gen)" == b ]]
gen_draining a
enabled "$GW_UNIT@b.service"
disabled "$GW_UNIT@a.service"
not_logged "disable $GW_UNIT@a.service"
echo 'U6 first start beside a draining generation: b serves, a drains on'

# U7. Both enabled is the failure the deploy refuses to end on.
fresh
touch "$STUB_ENABLED/$GW_UNIT@a.service" "$STUB_ENABLED/$GW_UNIT@b.service"
if gens_one_enabled 2> "$fixture/both"; then echo 'two enabled generations passed' >&2; exit 1; fi
grep -q 'a reboot would start both' "$fixture/both"
echo 'U7 two enabled generations are caught'

# U8. units_active names the running units, which the deploy's Redis gate
# refuses a restart under.
fresh
serving_b
mapfile -t reaches < <(units_active t1mush-server.service "$GW_UNIT@a.service" "$GW_UNIT@b.service")
[[ "${reaches[*]}" == "t1mush-server.service $GW_UNIT@b.service" ]]
fresh
mapfile -t reaches < <(units_active t1mush-server.service "$GW_UNIT@a.service" "$GW_UNIT@b.service")
[[ "${#reaches[@]}" -eq 0 ]]
echo 'U8 units_active: the running units, and none on a stopped instance'

# U9. The hold: written whole and readable, saying who set it; never through a
# symlink.
fresh
server_hold_set "$STUB_HOLD" "SERVER_HOLD=1 deploy.sh"
server_held "$STUB_HOLD"
[[ "$(stat -c %a "$STUB_HOLD")" == 644 ]]
grep -q '^held by SERVER_HOLD=1 deploy.sh at .*launch-bringup.py lifts it' "$STUB_HOLD"
[[ -z "$(find "$fixture" -maxdepth 1 -name 'server.hold.*')" ]]
rm -f "$STUB_HOLD"
unheld "$STUB_HOLD"
printf 'keep\n' > "$fixture/elsewhere"
ln -s "$fixture/elsewhere" "$STUB_HOLD"
if server_hold_set "$STUB_HOLD" test 2> /dev/null; then echo 'the hold was written through a symlink' >&2; exit 1; fi
[[ "$(cat "$fixture/elsewhere")" == keep ]]
echo 'U9 the hold file: written whole, 0644, never through a symlink'

# U10. Both split deploys source the library and keep the race fixes: the
# settle before the generations are read, the first start that disables the
# other, the end check, the hold before the Redis step, the Redis gate.
# The patterns are the deploys' own lines, $ and all.
# shellcheck disable=SC2016
for inst in dev1 dev2; do
  deploy="$root/deploy/$inst-split/deploy.sh"
  grep -q '^\. "\$DRAIN_DIR/units.sh"$' "$deploy"
  grep -q '^if ! units_settle 600 "\${SETTLE_UNITS\[@\]}"; then$' "$deploy"
  grep -q '^    gen_first_start "\$START"$' "$deploy"
  grep -q '^if ! gens_one_enabled; then$' "$deploy"
  grep -q 'server_hold_set "\$HOLD" "SERVER_HOLD=1 deploy.sh"' "$deploy"
  grep -q 'REDIS_RESTART=1 bash \$0' "$deploy"
  hold_line="$(grep -n 'server_hold_set "\$HOLD"' "$deploy" | head -1 | cut -d: -f1)"
  restart_line="$(grep -n '^  systemctl restart redis-' "$deploy" | head -1 | cut -d: -f1)"
  [[ "$hold_line" -lt "$restart_line" ]]
  unit="$root/deploy/$inst-split/${inst}mush-server.service"
  grep -qx "AssertPathExists=!/opt/rustmush/$inst/server.hold" "$unit"
  grep -q '^HOLD=\$ROOT/server.hold ' "$deploy"
  grep -qx "ROOT=/opt/rustmush/$inst" "$deploy"
done
echo 'U10 both split deploys wire the helpers; both server units assert the hold their deploy writes'

echo 'deploy units: propagated restart settled, held restart, settle timeout, busy states, one enabled generation, the Redis gate, the hold file, the wiring passed'
