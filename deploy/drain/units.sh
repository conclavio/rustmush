#!/usr/bin/env bash
# units.sh — the systemd helpers a split deploy reads its instance's units with
# (deploy/<inst>-split/deploy.sh). SOURCED, never run: the caller sets
#   GW_UNIT   the gateway template's stem (<inst>mush-gateway: @a and @b)
#   GW_STATE  its StateDirectory, where a draining generation leaves
#             draining-<gen> (doc 33 §4.7)
# before calling the generation helpers. scripts/tests/deploy-units.sh drives
# every function here against a stub `systemctl`.
#
# Why the waiting matters (2026-11-22, dev1 and dev2): `systemctl restart` and
# `systemctl start` return when the job of the unit they name is done, not when
# the jobs systemd PROPAGATED from it are. Restarting an instance's Redis stops
# every unit that `Requires=` it (the server and the serving gateway
# generation) and queues their starts again, and `systemctl restart` returns
# while those starts are still waiting: the serving generation reads
# `inactive` and the server `activating`. A deploy that read the units at that
# moment found "no generation serves" and started the other one beside it,
# both enabled. `units_settle` waits until systemd has nothing queued and
# nothing between states for the units it is given.

# units_busy <unit>... — print the first of the units that systemd still has a
# job queued for, or that is between states, and succeed; fail when none is.
units_busy() {
  local jobs unit state
  jobs="$(systemctl list-jobs --no-legend --plain "$@" 2>/dev/null | grep '\.service' || true)"
  if [ -n "$jobs" ]; then
    # JOB UNIT TYPE STATE
    printf '%s\n' "$jobs" | awk 'NR == 1 { print $2 " (" $3 " job " $4 ")" }'
    return 0
  fi
  for unit in "$@"; do
    state="$(systemctl show -p ActiveState --value "$unit" 2>/dev/null || true)"
    case "$state" in
      activating | deactivating | reloading | refreshing)
        echo "$unit ($state)"
        return 0
        ;;
    esac
  done
  return 1
}

# units_settle <seconds> <unit>... — wait until none of the units is busy (see
# units_busy). Says what it waits for every 15 polls; fails, naming the unit,
# when the units are still busy after <seconds> polls. The poll is 1 s
# ($UNITS_SETTLE_POLL; the shell tests set 0).
units_settle() {
  local limit="$1" polls=0 busy
  shift
  while busy="$(units_busy "$@")"; do
    if [ "$polls" -ge "$limit" ]; then
      echo "systemd is still moving $busy after ${limit} s" >&2
      return 1
    fi
    if [ $((polls % 15)) -eq 0 ]; then
      echo "waiting for systemd to finish with $busy"
    fi
    sleep "${UNITS_SETTLE_POLL:-1}"
    polls=$((polls + 1))
  done
}

# units_active <unit>... — print, one a line, the units that are active.
units_active() {
  local unit
  for unit in "$@"; do
    if systemctl is-active --quiet "$unit" 2>/dev/null; then echo "$unit"; fi
  done
}

# The two generations of a gateway template (doc 33 GG.4). A drain does not
# change the unit's state (it stays `active`): the generation that serves is the
# active one WITHOUT a draining-<gen> marker. Read them only after
# units_settle: in the middle of a restart a serving generation is not active.
gen_active()   { systemctl is-active --quiet "$GW_UNIT@$1.service" 2>/dev/null; }
gen_enabled()  { systemctl is-enabled --quiet "$GW_UNIT@$1.service" 2>/dev/null; }
gen_draining() { gen_active "$1" && [ -f "$GW_STATE/draining-$1" ]; }
serving_gen()  { local g; for g in a b; do if gen_active "$g" && [ ! -f "$GW_STATE/draining-$g" ]; then echo "$g"; return 0; fi; done; return 1; }
draining_gen() { local g; for g in a b; do if gen_draining "$g"; then echo "$g"; return 0; fi; done; return 1; }
other_gen()    { if [ "$1" = a ]; then echo b; else echo a; fi; }
# What a unit is executing: /proc/<MainPID>/exe reads (and runs) the binary the
# unit was started with even after the build replaced the file on disk.
running_exe()  { local pid; pid="$(systemctl show -p MainPID --value "$1" 2>/dev/null || true)"; if [ -n "$pid" ] && [ "$pid" != 0 ]; then echo "/proc/$pid/exe"; else echo "/proc/0/exe"; fi; }

# gen_first_start <gen> — no generation serves: start <gen> with its marker
# cleared, and make it the ONLY enabled one, so a reboot brings back the one
# that serves and never both. The other is disabled, not stopped: when it is
# running it is draining (it was disabled by the swap that drained it, and it
# exits on its own), and when it is not running, an `enabled` left over from
# before is what would start it beside <gen> at the next boot.
gen_first_start() {
  local gen="$1" other
  other="$(other_gen "$gen")"
  rm -f "$GW_STATE/draining-$gen"
  systemctl reset-failed "$GW_UNIT@$gen.service" 2>/dev/null || true
  systemctl enable "$GW_UNIT@$gen.service"
  if gen_enabled "$other"; then
    echo "disabling $GW_UNIT@$other: only the serving generation is enabled"
    systemctl disable "$GW_UNIT@$other.service"
  fi
  systemctl start "$GW_UNIT@$gen.service"
}

# gens_one_enabled — succeed unless BOTH generations are enabled (which a
# reboot would start side by side); says which when it fails.
gens_one_enabled() {
  if gen_enabled a && gen_enabled b; then
    echo "both $GW_UNIT@a and $GW_UNIT@b are enabled: a reboot would start both" >&2
    return 1
  fi
}

# The server's HOLD (doc 44 cut MW.1). While the hold file exists, the server
# unit's `AssertPathExists=!<file>` refuses every start: deploy.sh's, the one a
# gateway generation's `Wants=` pulls in, boot's and a hand `systemctl start`
# (which fails with "Assertion failed"). A refused start costs no start-limit
# token and changes no unit state. The file lives in the instance's root-owned
# directory beside instance.env, so the service user can neither set nor lift
# it; deploy/instance/launch-bringup.py lifts it when it starts the world.
#
# server_hold_set <file> <who> — write the hold, atomically, saying who set it.
server_hold_set() {
  local file="$1" who="$2" tmp
  if [ -L "$file" ]; then
    echo "refusing a symlinked hold file: $file" >&2
    return 1
  fi
  tmp="$(mktemp "$file.XXXXXX")"
  printf 'held by %s at %s; deploy/instance/launch-bringup.py lifts it when it starts the world\n' \
    "$who" "$(date -u +%FT%TZ)" > "$tmp"
  chmod 0644 "$tmp"
  mv -fT "$tmp" "$file"
}
# server_held <file> — succeed while the hold is set.
server_held() { [ -e "$1" ] || [ -L "$1" ]; }
