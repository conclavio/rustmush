#!/usr/bin/env bash
# swap.sh — move sshdoorway to the other generation without dropping a player
# (doc 29 §9.4).
#
#   swap.sh            start the idle generation, wait for READY, drain the old
#   swap.sh --force    as above, but if the idle generation is still draining
#                      from the last swap, finish it now (SIGUSR1: one 30 s
#                      warning, then close) instead of refusing
#
# At most two generations ever run. The old generation is sent SIGTERM with
# `systemctl kill`, not `systemctl stop`, so this script never blocks for the
# drain (up to 24 h). It exits 0 on drain and is not restarted.
set -euo pipefail

force=0
[[ "${1:-}" == "--force" ]] && force=1

active() { systemctl is-active --quiet "sshdoorway@$1"; }

if active a && active b; then
    if (( force )); then
        echo "both generations are running; finishing the draining one now"
    else
        echo "both generations are running: one is still draining." >&2
        echo "wait for it, or re-run with --force to finish it now." >&2
        exit 1
    fi
fi

if active a; then old=a; new=b
elif active b; then old=b; new=a
else old=; new=a
fi

if active "$new"; then
    # --force with both running: the "new" one is the one still draining.
    systemctl kill --signal=USR1 "sshdoorway@$new"
    echo "waiting for sshdoorway@$new to finish..."
    while active "$new"; do sleep 1; done
fi

/opt/rustmush/sshdoorway/sshdoorway --config /etc/rustmush/sshdoorway.toml --check

echo "starting sshdoorway@$new"
# Type=notify: returns once the new generation is bound and READY.
systemctl start "sshdoorway@$new"

if [[ -n "$old" ]]; then
    echo "draining sshdoorway@$old (warnings at 12h/6h/2h/1h/15m/1m remaining; closes at 24 h)"
    systemctl kill --signal=TERM "sshdoorway@$old"
fi

if [[ -x "$(dirname "$0")/check-logging.sh" ]]; then
    "$(dirname "$0")/check-logging.sh"
fi
echo "swap done: serving on sshdoorway@$new"
