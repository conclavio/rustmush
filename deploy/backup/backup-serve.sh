#!/usr/bin/env bash
# backup-serve.sh — the ONLY thing the backup copy's SSH key may run on the box
# that takes the backups (docs/ops.md, "Whole-instance backup and restore").
#
# It is the forced command of one line in the `rmbackup` user's authorized_keys:
#
#   restrict,command="/usr/local/lib/rustmush-backup/backup-serve.sh" ssh-ed25519 AAAA... backup-pull@conclav.dev
#
# `restrict` turns off the terminal, every forwarding and the agent; the forced
# command replaces whatever the client asked to run, which arrives here as
# SSH_ORIGINAL_COMMAND and is read as DATA: one of the three requests below and
# nothing else. The key can list archives, read the status file and fetch one
# archive by its exact name. It cannot write, delete, name a path or get a shell.
#
#   list           one line per archive: <bytes> <name>, oldest first
#   status         the last run's status.json
#   fetch <name>   the archive's bytes on stdout
#
# The directory is BACKUP_DIR's value in /etc/rustmush-backup.conf (default
# /var/backups/rustmush); archives there are root:rmbackup 0640 in a 0750
# directory, so this user reads them and nobody else does.
set -euo pipefail

CONF=${RUSTMUSH_BACKUP_CONF:-/etc/rustmush-backup.conf}
DIR=/var/backups/rustmush
if [ -r "$CONF" ]; then
  # Read, never sourced: the file is KEY=value lines.
  v="$(sed -n 's/^BACKUP_DIR=//p' "$CONF" | tail -1 | tr -d '\r')"
  if [ -n "$v" ]; then DIR="$v"; fi
fi

# An archive's name, whole: <instance>-<UTC stamp>-<commit8>.rmbak.tar.zst (or .gz).
NAME_RE='^[a-z][a-z0-9]*-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{8}\.rmbak\.tar\.(zst|gz)$'

refuse() {
  echo "backup-serve: refused: $1" >&2
  logger -t rustmush-backup-serve -p auth.warning -- "refused: $1" 2> /dev/null || true
  exit 2
}

read -r verb arg extra <<< "${SSH_ORIGINAL_COMMAND:-}"
[ -z "${extra:-}" ] || refuse "too many words"
case "${verb:-}" in
  list)
    [ -z "${arg:-}" ] || refuse "list takes no argument"
    cd "$DIR"
    for f in *.rmbak.tar.*; do
      [ -f "$f" ] && [ ! -L "$f" ] || continue
      [[ "$f" =~ $NAME_RE ]] || continue
      printf '%s %s\n' "$(stat -c %s -- "$f")" "$f"
    done | sort -k2
    ;;
  status)
    [ -z "${arg:-}" ] || refuse "status takes no argument"
    if [ -f "$DIR/status.json" ] && [ ! -L "$DIR/status.json" ]; then cat -- "$DIR/status.json"; else echo '{}'; fi
    ;;
  fetch)
    [[ "${arg:-}" =~ $NAME_RE ]] || refuse "not an archive name"
    [ -f "$DIR/$arg" ] && [ ! -L "$DIR/$arg" ] || refuse "no such archive"
    exec cat -- "$DIR/$arg"
    ;;
  *)
    refuse "unknown request"
    ;;
esac
