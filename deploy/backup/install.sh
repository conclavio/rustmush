#!/usr/bin/env bash
# install.sh — put the backup tool and its units on a box. Run as root, from a
# checkout or a deployed tree (docs/ops.md, "Whole-instance backup and restore").
#
#   bash deploy/backup/install.sh take [--serve-user]   the box that TAKES backups (conclav.io)
#   bash deploy/backup/install.sh pull                  the box that FETCHES them (conclav.dev)
#
# Idempotent. It installs files and reloads systemd; it ENABLES NOTHING and
# starts nothing: the last lines it prints are the commands that do.
#
#   take   /usr/local/lib/rustmush-backup/{rmbak.py,backup-serve.sh,README.md},
#          /etc/rustmush-backup.conf (only if there is none: an existing one is
#          never overwritten), /var/backups/rustmush (root, 0750),
#          rustmush-backup.service and .timer.
#          --serve-user also makes the system user and group `rmbackup`, whose
#          one job is to be the forced-command account the copy's SSH key logs
#          in as: home /var/lib/rmbackup, an empty root-owned
#          .ssh/authorized_keys (the owner adds the one key line), and group
#          read on the backup directory. It adds no key and no sshd setting.
#   pull   the same tool and config, /var/backups/rustmush-pulled (root, 0700),
#          rustmush-backup-pull.service and .timer.
set -euo pipefail
trap 'echo "install: FAILED at line $LINENO: $BASH_COMMAND" >&2' ERR

ROLE=${1:?usage: install.sh take [--serve-user] | pull}
SERVE_USER=0
if [ "${2:-}" = --serve-user ]; then SERVE_USER=1; fi
case "$ROLE" in take | pull) ;; *)
  echo "install: the role is take or pull, not $ROLE" >&2
  exit 2
  ;;
esac
[ "$(id -u)" -eq 0 ] || { echo "install: must run as root" >&2; exit 1; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB=/usr/local/lib/rustmush-backup
CONF=/etc/rustmush-backup.conf
for tool in python3 redis-cli redis-server systemctl; do
  command -v "$tool" > /dev/null || { echo "install: $tool is not on PATH" >&2; exit 1; }
done
command -v zstd > /dev/null || echo "install: note: no zstd on this box; archives will be gzip (.tar.gz)" >&2

# The whole directory goes to $LIB, this script and the owner's two among it, so
# the box can run any of them again without a tree.
if [ "$HERE" != "$LIB" ]; then
  install -d -o root -g root -m 0755 "$LIB"
  install -o root -g root -m 0755 "$HERE/rmbak.py" "$HERE/backup-serve.sh" "$HERE/install.sh" \
    "$HERE/pull-access.sh" "$HERE/pull-key.sh" "$LIB/"
  install -o root -g root -m 0644 "$HERE/README.md" "$HERE/backup.conf.example" \
    "$HERE"/rustmush-backup*.service "$HERE"/rustmush-backup*.timer "$LIB/"
fi
if [ -e "$CONF" ]; then
  echo "install: $CONF exists and is kept as it is"
else
  install -o root -g root -m 0644 "$HERE/backup.conf.example" "$CONF"
  echo "install: wrote $CONF from the example: set INSTANCES (take) or PULL_SSH (pull) in it"
fi

if [ "$ROLE" = take ]; then
  if [ "$SERVE_USER" = 1 ]; then
    getent group rmbackup > /dev/null || groupadd --system rmbackup
    if ! id rmbackup > /dev/null 2>&1; then
      useradd --system --gid rmbackup --home-dir /var/lib/rmbackup --no-create-home --shell /bin/sh rmbackup
      echo "install: created the system user rmbackup (no password, no key yet)"
    fi
    install -d -o root -g rmbackup -m 0750 /var/lib/rmbackup
    install -d -o root -g rmbackup -m 0750 /var/lib/rmbackup/.ssh
    if [ ! -e /var/lib/rmbackup/.ssh/authorized_keys ]; then
      install -o root -g rmbackup -m 0640 /dev/null /var/lib/rmbackup/.ssh/authorized_keys
    fi
  fi
  if getent group rmbackup > /dev/null; then
    install -d -o root -g rmbackup -m 0750 /var/backups/rustmush
    # Archives written before the account existed are root's alone: the group reads them all.
    for f in /var/backups/rustmush/*.rmbak.tar.* /var/backups/rustmush/status.json; do
      if [ -f "$f" ] && [ ! -L "$f" ]; then
        chgrp rmbackup "$f"
        chmod 0640 "$f"
      fi
    done
  else
    install -d -o root -g root -m 0750 /var/backups/rustmush
  fi
  install -o root -g root -m 0644 "$HERE/rustmush-backup.service" "$HERE/rustmush-backup.timer" /etc/systemd/system/
  systemctl daemon-reload
  echo "install: done. This run enabled and started nothing (a timer that was enabled still is). On a new box, in order:"
  echo "  1. set INSTANCES in $CONF"
  echo "  2. one run by hand:    systemctl start rustmush-backup.service; python3 $LIB/rmbak.py status"
  echo "  3. the timer:          systemctl enable --now rustmush-backup.timer"
else
  install -d -o root -g root -m 0700 /var/backups/rustmush-pulled
  install -o root -g root -m 0644 "$HERE/rustmush-backup-pull.service" "$HERE/rustmush-backup-pull.timer" /etc/systemd/system/
  systemctl daemon-reload
  echo "install: done. This run enabled and started nothing (a timer that was enabled still is). On a new box:"
  echo "  the OWNER, as root: bash $LIB/pull-key.sh     (it makes the key and prints the next two commands;"
  echo "  the last of them sets PULL_SSH, runs one pull and enables rustmush-backup-pull.timer)"
fi
