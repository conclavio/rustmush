#!/usr/bin/env bash
# gateway-swap.sh — move dev2's gateway to its other generation without
# dropping a player (doc 33 §4.7, cut GG.4). Run as root.
#
#   gateway-swap.sh [dev2]            start the idle generation
#                                     (dev2mush-gateway@a or @b), wait for its
#                                     READY=1, confirm it stays up for 5 s,
#                                     re-point /run/rustmush-ssh/dev2.sock at
#                                     its trusted socket, enable it and disable
#                                     the old one, then drain the old one
#                                     (SIGUSR1: warnings at 12h/6h/2h/1h/15m/1m
#                                     remaining; it exits when empty or at 24 h)
#   gateway-swap.sh [dev2] --force    as above, but if the idle generation is
#                                     still draining from the last swap, finish
#                                     it now (SIGTERM: one 30 s warning, then
#                                     close) instead of refusing
#
# The instance name is optional and must be this script's own (`dev2`): the
# guard against running one instance's swap script for another.
#
# The mechanism (the flock, the two-generation guard, the marker, the 5 s
# stay-up check, the symlink re-pointed by rename after the stay-up check,
# exactly one generation enabled) is deploy/drain/swap.sh, shared with
# sshdoorway (doc 33 §4.5, cut GG.3); this script only names dev2's units,
# marker directory, trusted socket and config check. On the box it is run
# from the shipped tree, /opt/rustmush/dev2/build/deploy/dev2-split/, so
# ../drain/swap.sh resolves as in the tree; $DRAIN_SWAP overrides that path.
#
# Environment overrides, for the shell test (scripts/tests/gateway-swap.sh)
# and nothing else: GATEWAY_SWAP_MARKER_DIR, GATEWAY_SWAP_LINK,
# GATEWAY_SWAP_LINK_TARGET (with %g for the generation), GATEWAY_SWAP_CHECK,
# GATEWAY_SWAP_LOCK.
set -euo pipefail

INST=dev2
ROOT=/opt/rustmush/$INST
UNIT=${INST}mush-gateway

usage() {
    echo "usage: gateway-swap.sh [$INST] [--force]" >&2
    exit 2
}

force=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        "$INST") shift ;;
        --force) force=(--force); shift ;;
        *) usage ;;
    esac
done

here="$(cd "$(dirname "$0")" && pwd)"
shared="${DRAIN_SWAP:-$here/../drain/swap.sh}"
if [[ ! -x "$shared" ]]; then
    echo "the shared swap script is missing or not executable: $shared" >&2
    exit 1
fi

args=(
    --unit "$UNIT"
    # The unit's StateDirectory=dev2mush-gateway; the gateway reads $STATE_DIRECTORY.
    --marker-dir "${GATEWAY_SWAP_MARKER_DIR:-/var/lib/$UNIT}"
    # The gateway's own preflight over the installed config, as a generation
    # (without --generation a symlinked trusted socket is a refusal, doc 33 §4.4).
    --check "${GATEWAY_SWAP_CHECK:-$ROOT/build/target/release/mush-gateway --config $ROOT/mushd.toml --check-config --generation a}"
    # The doorway dials the configured [ssh].trusted_socket, a symlink; each
    # generation binds its own <stem>.<gen>.sock beside it (doc 33 §4.4).
    --symlink "${GATEWAY_SWAP_LINK:-/run/rustmush-ssh/$INST.sock}"
    --symlink-target "${GATEWAY_SWAP_LINK_TARGET:-/run/rustmush-ssh/$INST.%g.sock}"
    --drain-note "warnings at 12h/6h/2h/1h/15m/1m remaining; exits when empty or at 24 h"
)
if [[ -n "${GATEWAY_SWAP_LOCK:-}" ]]; then
    args+=(--lock "$GATEWAY_SWAP_LOCK")
fi
exec "$shared" "${args[@]}" "${force[@]}"
