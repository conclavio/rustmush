#!/usr/bin/env bash
# gateway-swap.sh — move dev1's gateway to its other generation without
# dropping a player (doc 33 §4.7, cut GG.4). Run as root.
#
#   gateway-swap.sh [dev1]            start the idle generation
#                                     (dev1mush-gateway@a or @b), wait for its
#                                     READY=1, confirm it stays up for 5 s,
#                                     re-point /run/rustmush-ssh/dev1.sock at
#                                     its trusted socket, enable it and disable
#                                     the old one, then drain the old one
#                                     (SIGUSR1: warnings at 12h/6h/2h/1h/15m/1m
#                                     remaining; it exits when empty or at 24 h)
#   gateway-swap.sh [dev1] --force    as above, but if the idle generation is
#                                     still draining from the last swap, finish
#                                     it now (SIGTERM: one 30 s warning, then
#                                     close) instead of refusing
#
# The instance name is optional and must be this script's own (`dev1`): the
# guard against running one instance's swap script for another.
#
# The mechanism (the flock, the two-generation guard, the marker, the 5 s
# stay-up check, the symlink re-pointed by rename after the stay-up check,
# exactly one generation enabled) is deploy/drain/swap.sh, shared with
# sshdoorway (doc 33 §4.5, cut GG.3); this script only names dev1's units,
# marker directory, trusted socket and config check. On the box it is run
# from the shipped tree, /opt/rustmush/dev1/build/deploy/dev1-split/, so
# ../drain/swap.sh resolves as in the tree; $DRAIN_SWAP overrides that path.
#
# Environment overrides, for the shell test (scripts/tests/gateway-swap.sh)
# and nothing else: GATEWAY_SWAP_MARKER_DIR, GATEWAY_SWAP_CONFIG (the rendered
# config the trusted socket is read from), GATEWAY_SWAP_LINK,
# GATEWAY_SWAP_LINK_TARGET (with %g for the generation), GATEWAY_SWAP_CHECK,
# GATEWAY_SWAP_LOCK.
set -euo pipefail

INST=dev1
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

config="${GATEWAY_SWAP_CONFIG:-$ROOT/mushd.toml}"
args=(
    --unit "$UNIT"
    # The unit's StateDirectory=dev1mush-gateway; the gateway reads $STATE_DIRECTORY.
    --marker-dir "${GATEWAY_SWAP_MARKER_DIR:-/var/lib/$UNIT}"
    # The gateway's own preflight over the installed config, as a generation
    # (without --generation a symlinked trusted socket is a refusal, doc 33 §4.4).
    --check "${GATEWAY_SWAP_CHECK:-$ROOT/build/target/release/mush-gateway --config $config --check-config --generation a}"
    --drain-note "warnings at 12h/6h/2h/1h/15m/1m remaining; exits when empty or at 24 h"
)
# The doorway dials the configured [ssh].trusted_socket, a symlink that the
# swap re-points at the new generation's own <stem>.<gen>.sock beside it (doc
# 33 §4.4). The link is read from the rendered config, as deploy.sh reads it: a
# box without the doorway renders no trusted_socket (dev2 on 2026-10-31, GG.5),
# and then there is no link to re-point and the new generation must not be
# refused for a socket it was never asked to bind. GATEWAY_SWAP_LINK names the
# link outright (set and empty = no link), for the shell test.
trusted_socket() {
    sed -n 's/^[[:space:]]*trusted_socket[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$1" 2>/dev/null | head -1
}
link="${GATEWAY_SWAP_LINK-$(trusted_socket "$config")}"
if [[ -n "$link" ]]; then
    case "$link" in
        *.sock) target_default="${link%.sock}.%g.sock" ;;
        *)      target_default="$link.%g" ;;
    esac
    args+=(--symlink "$link" --symlink-target "${GATEWAY_SWAP_LINK_TARGET:-$target_default}")
else
    echo "no [ssh].trusted_socket in $config: no doorway link to re-point; the swap runs without the symlink step"
fi
if [[ -n "${GATEWAY_SWAP_LOCK:-}" ]]; then
    args+=(--lock "$GATEWAY_SWAP_LOCK")
fi
exec "$shared" "${args[@]}" "${force[@]}"
