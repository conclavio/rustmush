#!/usr/bin/env bash
# pull-key.sh — THE OWNER RUNS THIS, as root, on the box that FETCHES the
# backups (conclav.dev). It makes an SSH key and trusts a host key: no session
# of Claude's runs it (docs/ops.md, "The copy to a second box"; doc 52
# section 5). Three commands in all, this script twice:
#
#   1. here:            bash /usr/local/lib/rustmush-backup/pull-key.sh
#        makes /root/.ssh/rustmush-backup-pull (ed25519, no passphrase: a timer
#        uses it, and its whole power is the far side's forced command) unless
#        it exists, and prints the command for step 2 with the public key in it.
#   2. on conclav.io:   bash /usr/local/lib/rustmush-backup/pull-access.sh '<public key line>'
#        (password logins off, the account, the key line, the firewall); it
#        prints the command for step 3 with its host key's fingerprint in it.
#   3. here:            bash /usr/local/lib/rustmush-backup/pull-key.sh --finish <fingerprint>
#        reads conclav.io's host key off the wire, keeps it ONLY if it has that
#        fingerprint, writes PULL_SSH into /etc/rustmush-backup.conf, asks the
#        far side for its list, runs one pull and enables the pull timer.
#
# Options, either form: --host conclav.io --port 2222 --user rmbackup.
set -euo pipefail

SYSROOT=${RMBAK_SYSROOT:-}                  # the tests' sandbox; empty on a box
HOST=conclav.io
PORT=2222
RUSER=rmbackup
FINISH=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --finish) FINISH=${2:?--finish needs the fingerprint pull-access.sh printed}; shift 2 ;;
    --host) HOST=${2:?}; shift 2 ;;
    --port) PORT=${2:?}; shift 2 ;;
    --user) RUSER=${2:?}; shift 2 ;;
    *) echo "pull-key: unknown argument $1" >&2; exit 2 ;;
  esac
done
say() { echo "pull-key: $*"; }
die() { echo "pull-key: STOPPED: $*" >&2; exit 1; }
[[ "$HOST" =~ ^[A-Za-z0-9.-]+$ ]] || die "--host is a host name"
[[ "$PORT" =~ ^[0-9]{1,5}$ ]] || die "--port is a number"
[[ "$RUSER" =~ ^[a-z][a-z0-9_-]*$ ]] || die "--user is an account name"
if [ -z "$SYSROOT" ] && [ "$(id -u)" -ne 0 ]; then die "run as root"; fi

KEY="$SYSROOT/root/.ssh/rustmush-backup-pull"
KNOWN="$SYSROOT/root/.ssh/rustmush-backup-known_hosts"
CONF="$SYSROOT/etc/rustmush-backup.conf"
LIB=/usr/local/lib/rustmush-backup

if [ -z "$FINISH" ]; then
  install -d -m 0700 "$(dirname "$KEY")"
  if [ -e "$KEY" ]; then
    say "the key exists and is kept: $KEY"
  else
    ssh-keygen -q -t ed25519 -N '' -C "backup-pull@$(hostname -f 2> /dev/null || hostname)" -f "$KEY"
    say "made $KEY (ed25519, no passphrase; mode $(stat -c %a "$KEY"))"
  fi
  [ -f "$KEY.pub" ] || die "$KEY.pub is missing"
  say "its fingerprint: $(ssh-keygen -lf "$KEY.pub")"
  echo
  say "NEXT, on $HOST, as root:"
  echo "    bash $LIB/pull-access.sh '$(tr -d '\r\n' < "$KEY.pub")'"
  exit 0
fi

[[ "$FINISH" =~ ^SHA256:[A-Za-z0-9+/]{43}$ ]] || die "--finish takes a fingerprint like SHA256:…, as pull-access.sh printed it"
[ -f "$KEY" ] || die "no key at $KEY: run this script with no argument first"
[ -f "$CONF" ] || die "no $CONF: run install.sh pull first"
scan="$(mktemp)"
trap 'rm -f "$scan"' EXIT
ssh-keyscan -T 10 -p "$PORT" -t ed25519 "$HOST" > "$scan" 2> /dev/null || true
[ -s "$scan" ] || die "$HOST:$PORT did not answer a key scan (the panel rule, the firewall, the port?)"
seen="$(ssh-keygen -lf "$scan" | awk '{print $2}')"
if [ "$seen" != "$FINISH" ]; then
  die "$HOST:$PORT shows the host key $seen, NOT $FINISH: nothing was trusted. Check the fingerprint on $HOST itself."
fi
install -m 0600 "$scan" "$KNOWN"
say "$HOST's host key is the one you named; kept in $KNOWN"

PULL="ssh -p $PORT -i $KEY -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$KNOWN $RUSER@$HOST"
if grep -q '^PULL_SSH=' "$CONF"; then
  tmp="$(mktemp "$CONF.XXXXXX")"
  awk -v line="PULL_SSH=$PULL" '/^PULL_SSH=/ { print line; next } { print }' "$CONF" > "$tmp"
  chmod 0644 "$tmp"
  mv "$tmp" "$CONF"
else
  printf 'PULL_SSH=%s\n' "$PULL" >> "$CONF"
fi
say "PULL_SSH is set in $CONF"

say "asking $HOST for its list:"
# shellcheck disable=SC2086
$PULL list | sed 's/^/pull-key:   /'
# The forced command must be all the key can do.
# shellcheck disable=SC2086
if $PULL 'cat /etc/hostname' > /dev/null 2>&1; then
  die "the key ran a command the far side should have refused: check rmbackup's authorized_keys line on $HOST"
fi
say "the far side refuses anything but list, status and fetch: good"
if [ -z "$SYSROOT" ]; then
  systemctl start rustmush-backup-pull.service
  systemctl enable --now rustmush-backup-pull.timer > /dev/null 2>&1
  python3 "$LIB/rmbak.py" status || true
  say "DONE: one pull has run and the pull timer is enabled"
fi
