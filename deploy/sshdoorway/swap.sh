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
# The mechanism (the flock, the two-generation guard, the marker, the 5 s
# stay-up check, exactly one generation enabled) is deploy/drain/swap.sh,
# shared with the gateway (doc 33 §4.5, cut GG.3); this script only names the
# doorway's units, marker directory and checks. It is installed as
# /opt/rustmush/sshdoorway/swap.sh with the shared script at
# /opt/rustmush/drain/swap.sh, so ../drain/swap.sh resolves on the box as in
# the tree; $DRAIN_SWAP overrides that path.
set -euo pipefail

case "${1:-}" in
    "") ;;
    --force) ;;
    *) echo "usage: swap.sh [--force]" >&2; exit 2 ;;
esac
if [[ $# -gt 1 ]]; then
    echo "usage: swap.sh [--force]" >&2
    exit 2
fi

here="$(cd "$(dirname "$0")" && pwd)"
shared="${DRAIN_SWAP:-$here/../drain/swap.sh}"
if [[ ! -x "$shared" ]]; then
    echo "the shared swap script is missing or not executable: $shared" >&2
    exit 1
fi

args=(
    --unit sshdoorway
    # The unit's StateDirectory=sshdoorway; the doorway reads $STATE_DIRECTORY.
    --marker-dir /var/lib/sshdoorway
    --check "/opt/rustmush/sshdoorway/sshdoorway --config /etc/rustmush/sshdoorway.toml --check"
    --drain-note "warnings at 12h/6h/2h/1h/15m/1m remaining; closes at 24 h"
)
# The ship-time logging gate runs again after every swap (doc 29 §8.1).
if [[ -x "$here/check-logging.sh" ]]; then
    args+=(--after "$here/check-logging.sh")
fi
exec "$shared" "${args[@]}" "$@"
