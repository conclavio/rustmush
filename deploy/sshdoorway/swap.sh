#!/usr/bin/env bash
# swap.sh — move sshdoorway to the other generation without dropping a player
# (doc 29 §9.4).
#
#   swap.sh            start the idle generation, confirm it stays up, then
#                      drain the old one (SIGUSR1: warnings at 12h/6h/2h/1h/
#                      15m/1m remaining, closes at 24 h or when empty)
#   swap.sh --force    as above, but if the idle generation is still draining
#                      from the last swap, finish it now (SIGTERM: one 30 s
#                      warning, then close) instead of refusing
#
# At most two generations ever run. A drain does not change the unit's state
# (it stays "active"), so "both active" means one is still draining. Only one
# swap runs at a time (flock).
set -euo pipefail

force=0
case "${1:-}" in
    "") ;;
    --force) force=1 ;;
    *) echo "usage: swap.sh [--force]" >&2; exit 2 ;;
esac

exec 9>/run/lock/sshdoorway-swap.lock
flock -n 9 || { echo "another swap is running" >&2; exit 1; }

state() { systemctl show -p ActiveState --value "sshdoorway@$1"; }
running() { case "$(state "$1")" in active|activating|deactivating|reloading) return 0;; *) return 1;; esac; }
# The unit's StateDirectory=sshdoorway; the doorway reads $STATE_DIRECTORY.
# A generation that finds its marker at start does not serve, so only this
# script (which clears it) ever brings a drained generation back.
marker() { echo "/var/lib/sshdoorway/draining-$1"; }
draining() { [[ -e "$(marker "$1")" ]]; }

if running a && running b; then
    if (( force )); then
        echo "both generations are running; finishing the draining one now"
    else
        echo "both generations are running: one is still draining." >&2
        echo "wait for it, or re-run with --force to finish it now." >&2
        exit 1
    fi
fi

# The serving generation is the running one without a drain marker.
if running a && ! draining a; then old=a; new=b
elif running b && ! draining b; then old=b; new=a
elif running a; then old=; new=b
elif running b; then old=; new=a
else old=; new=a
fi

if running "$new"; then
    # --force with both running: "new" is the one still draining.
    systemctl kill --signal=TERM "sshdoorway@$new"
    echo "waiting for sshdoorway@$new to finish (up to 2 min)..."
    for _ in $(seq 120); do running "$new" || break; sleep 1; done
    if running "$new"; then
        echo "sshdoorway@$new did not finish; stop it by hand" >&2
        exit 1
    fi
fi

/opt/rustmush/sshdoorway/sshdoorway --config /etc/rustmush/sshdoorway.toml --check

rm -f "$(marker "$new")"
echo "starting sshdoorway@$new"
# Type=notify: returns once the new generation is bound and READY.
systemctl start "sshdoorway@$new"
# A generation that crashes right after READY must not take the old one down
# with it: confirm it is still serving before draining its predecessor.
sleep 5
if [[ "$(state "$new")" != active ]]; then
    echo "sshdoorway@$new did not stay up; the old generation keeps serving" >&2
    # Restart=on-failure would otherwise start it again in a moment, beside
    # the old one, with no marker to stop it.
    systemctl stop "sshdoorway@$new" || true
    systemctl reset-failed "sshdoorway@$new" || true
    systemctl disable --quiet "sshdoorway@$new" || true
    exit 1
fi

# Exactly one generation is enabled, so a reboot brings back the serving one
# (the drained one would refuse anyway, by its marker).
systemctl enable --quiet "sshdoorway@$new"
if [[ -n "$old" ]]; then
    systemctl disable --quiet "sshdoorway@$old" || true
    echo "draining sshdoorway@$old (warnings at 12h/6h/2h/1h/15m/1m remaining; closes at 24 h)"
    systemctl kill --signal=USR1 "sshdoorway@$old"
fi

check="$(dirname "$0")/check-logging.sh"
if [[ -x "$check" ]]; then
    "$check" || echo "WARNING: the logging check failed after the swap; see above" >&2
fi
echo "swap done: serving on sshdoorway@$new"
