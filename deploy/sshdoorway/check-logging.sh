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
[[ "${1:-}" == "--budget-mb" ]] && budget_mb="${2:?--budget-mb needs a number}"

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

if logrotate -d -s /tmp/sshdoorway-logrotate-check.status "$conf" >/tmp/sshdoorway-logrotate-check.out 2>&1 \
    && ! grep -qiE '^error' /tmp/sshdoorway-logrotate-check.out; then
    ok "logrotate parses the config"
else
    bad "logrotate -d rejected the config:"; sed 's/^/        /' /tmp/sshdoorway-logrotate-check.out >&2
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

live=$(ls "$logdir"/*.log 2>/dev/null | head -n1 || true)
if [[ -z "$live" ]]; then
    bad "no live log file in $logdir (is [log].file_dir set and a generation running?)"
else
    before=$(ls "$logdir" | grep -c '\.zst$' || true)
    if logrotate -f -s "$state" "$conf"; then
        ok "forced rotation ran"
    else
        bad "forced rotation failed"
    fi
    after=$(ls "$logdir" | grep -c '\.zst$' || true)
    newest=$(ls -t "$logdir"/*.zst 2>/dev/null | head -n1 || true)
    if [[ -n "$newest" ]] && (( after >= before )) && unzstd -t -q "$newest"; then
        ok "rotation left a valid zstd archive ($(basename "$newest"))"
    else
        bad "no valid .zst archive after rotation"
    fi
    # The reopen: the doorway must write to the fresh file after SIGUSR2.
    # A connection attempt guarantees a line at info (an unauthenticated
    # close is counted into the minute summary, so poke the menu instead).
    sleep 1
    for addr in $(sed -n 's/^addrs *= *\[\(.*\)\]/\1/p' /etc/rustmush/sshdoorway.toml | tr -d '" ' | tr ',' ' '); do
        host=${addr%:*}; port=${addr##*:}; host=${host#[}; host=${host%]}
        timeout 5 ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
            -p "$port" "logcheck@$host" true >/dev/null 2>&1 || true
        break
    done
    sleep 2
    if [[ -s "$live" ]]; then
        ok "the live file was reopened and written ($(basename "$live"))"
    else
        bad "the live file is empty after rotation: SIGUSR2 reopen did not happen"
    fi
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
