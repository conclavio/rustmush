#!/usr/bin/env bash
# check-logging.sh — the doc 29 §8.1 ship-time gate (D-29-18).
#
# Must pass before sshdoorway is enabled on a box, and after every swap. It
# fails loudly: any failed check exits non-zero, nothing is downgraded to a
# warning.
#
#   check-logging.sh [--budget-mb N]   (default 300)
set -euo pipefail

budget_mb=300
case "${1:-}" in
    "") ;;
    --budget-mb) budget_mb="${2:?--budget-mb needs a number}" ;;
    *) echo "usage: check-logging.sh [--budget-mb N]" >&2; exit 2 ;;
esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

conf=/etc/sshdoorway/logrotate.conf
state=/var/lib/sshdoorway/logrotate.status
logdir=/var/log/sshdoorway
fail=0
ok()   { echo "  ok    $*"; }
bad()  { echo "  FAIL  $*" >&2; fail=1; }

echo "sshdoorway logging check"

command -v zstd >/dev/null && command -v unzstd >/dev/null \
    && ok "zstd is installed" || bad "zstd is not installed (apt-get install zstd)"

[[ -r "$conf" ]] && ok "$conf exists" || bad "$conf is missing"

if logrotate -d -s "$tmp/status" "$conf" >"$tmp/out" 2>&1 \
    && ! grep -qiE '^error' "$tmp/out"; then
    ok "logrotate parses the config"
else
    bad "logrotate -d rejected the config:"; sed 's/^/        /' "$tmp/out" >&2
fi

systemctl is-enabled --quiet sshdoorway-logrotate.timer \
    && ok "sshdoorway-logrotate.timer is enabled" || bad "sshdoorway-logrotate.timer is not enabled"
last=$(systemctl show -p LastTriggerUSec --value sshdoorway-logrotate.timer 2>/dev/null || true)
if [[ -n "$last" && "$last" != "n/a" && "$last" != "0" ]]; then
    ok "the timer has fired (last: $last)"
else
    bad "the timer has never fired (systemctl start sshdoorway-logrotate.timer, then wait 15 min or run it once)"
fi

for unit in $(systemctl list-units --plain --no-legend 'sshdoorway@*.service' | awk '{print $1}'); do
    interval=$(systemctl show -p LogRateLimitIntervalUSec --value "$unit")
    burst=$(systemctl show -p LogRateLimitBurst --value "$unit")
    if [[ "$interval" == "30s" && "$burst" == "200" ]]; then
        ok "$unit journald rate limit 30s/200"
    else
        bad "$unit journald rate limit is $interval/$burst, want 30s/200"
    fi
done

# Every running generation's file is rotated, and each must be reopened.
gens=$(systemctl list-units --plain --no-legend --state=active 'sshdoorway@*.service' \
    | sed -n 's/^sshdoorway@\([a-z0-9]*\)\.service.*/\1/p')
if [[ -z "$gens" ]]; then
    bad "no sshdoorway generation is running"
else
    before=$(find "$logdir" -maxdepth 1 -name '*.zst' | wc -l)
    stamp="$tmp/stamp"; touch "$stamp"; sleep 1
    for g in $gens; do
        # Something to rotate: notifempty skips an empty file.
        [[ -s "$logdir/$g.log" ]] || bad "$logdir/$g.log is empty or missing before rotation (is [log].file_dir set?)"
    done
    if logrotate -f -s "$state" "$conf"; then ok "forced rotation ran"; else bad "forced rotation failed"; fi
    after=$(find "$logdir" -maxdepth 1 -name '*.zst' | wc -l)
    # -cnewer, not -newer: zstd gives the archive the rotated log's mtime, which
    # predates the stamp, so an mtime test never sees a fresh archive (found
    # on conclav.io, DW.4, 2026-10-31); the inode change time is the creation.
    fresh=$(find "$logdir" -maxdepth 1 -name '*.zst' -cnewer "$stamp" | head -n1)
    if (( after > before )) && [[ -n "$fresh" ]] && unzstd -t -q "$fresh"; then
        ok "rotation left a new, valid zstd archive ($(basename "$fresh"))"
    else
        bad "no new valid .zst archive after the forced rotation"
    fi
    sleep 2
    for g in $gens; do
        # logrotate renamed the file; only the doorway's SIGUSR2 reopen
        # creates a fresh one at the same path.
        if [[ -e "$logdir/$g.log" ]]; then
            ok "generation $g reopened $g.log after rotation"
        else
            bad "generation $g did not reopen $g.log (SIGUSR2 not delivered or not handled)"
        fi
    done
fi

used_kb=$(du -sk "$logdir" 2>/dev/null | awk '{print $1}')
if (( ${used_kb:-0} <= budget_mb * 1024 )); then
    ok "$logdir uses $(( ${used_kb:-0} / 1024 )) MB of its ${budget_mb} MB budget"
else
    bad "$logdir uses $(( used_kb / 1024 )) MB, over its ${budget_mb} MB budget"
fi

if (( fail )); then
    echo "sshdoorway logging check FAILED" >&2
    exit 1
fi
echo "sshdoorway logging check passed"
