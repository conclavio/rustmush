#!/usr/bin/env bash
#
# deploy/vps-runner/dispatch-timers-install.sh — put the clock of the
# repository's clock-run workflows on a box (conclav.dev is the one). Run as
# root on the box, from this directory copied there or from a checkout:
#
#   bash deploy/vps-runner/dispatch-timers-install.sh
#
# What it changes, all of it, and nothing else:
#   /usr/local/lib/rustmush/workflow-dispatch.sh              0755 root   the call
#   /etc/systemd/system/rustmush-dispatch@.service            0644 root
#   /etc/systemd/system/rustmush-dispatch-soak.timer          0644 root   enabled, started
#   /etc/systemd/system/rustmush-dispatch-ci-nightly.timer    0644 root   enabled, started
#   /etc/systemd/system/rustmush-dispatch-ci-weekly.timer     0644 root   enabled, started
#   /etc/systemd/system/rustmush-dispatch-codeql.timer        0644 root   enabled, started
#   /etc/rustmush/                                            0755 root   the directory, if it is not there
#
# It never creates, reads or prints the token. /etc/rustmush/dispatch.env is
# the owner's to place (docs/ops.md, "The clock is on conclav.dev"); while it
# is not there each timer fires at its hour, its service fails with a line
# saying so, and nothing else happens. Idempotent: run it again after a
# change to the files beside it.
#
# To retire it:
#   systemctl disable --now rustmush-dispatch-soak.timer rustmush-dispatch-ci-nightly.timer \
#     rustmush-dispatch-ci-weekly.timer rustmush-dispatch-codeql.timer
#   rm -f /etc/systemd/system/rustmush-dispatch* /usr/local/lib/rustmush/workflow-dispatch.sh
#   rm -rf /etc/rustmush        # the token file too; revoke the token on GitHub
#   systemctl daemon-reload
set -euo pipefail

JOBS=(soak ci-nightly ci-weekly codeql)

if [ "$(id -u)" -ne 0 ]; then
  echo "dispatch-timers-install: run as root" >&2
  exit 1
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
files=(workflow-dispatch.sh "rustmush-dispatch@.service")
timers=()
for job in "${JOBS[@]}"; do
  files+=("rustmush-dispatch-$job.timer")
  timers+=("rustmush-dispatch-$job.timer")
done
for f in "${files[@]}"; do
  if [ ! -f "$here/$f" ]; then
    echo "dispatch-timers-install: $here/$f is missing" >&2
    exit 1
  fi
done

install -d -m 0755 -o root -g root /usr/local/lib/rustmush /etc/rustmush
install -m 0755 -o root -g root "$here/workflow-dispatch.sh" /usr/local/lib/rustmush/workflow-dispatch.sh
install -m 0644 -o root -g root "$here/rustmush-dispatch@.service" "/etc/systemd/system/rustmush-dispatch@.service"
for timer in "${timers[@]}"; do
  install -m 0644 -o root -g root "$here/$timer" "/etc/systemd/system/$timer"
done
systemctl daemon-reload
systemctl enable --now "${timers[@]}"
systemctl list-timers 'rustmush-dispatch-*' --no-pager

if [ ! -e /etc/rustmush/dispatch.env ]; then
  echo "dispatch-timers-install: the token file is not there yet: the timers are set, and each service fails at its hour until the owner places it (docs/ops.md, \"The clock is on conclav.dev\")"
fi
