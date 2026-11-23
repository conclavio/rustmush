#!/usr/bin/env bash
# pull-access.sh — THE OWNER RUNS THIS, as root, on the box that TAKES the
# backups (conclav.io). It changes sshd's settings and the firewall and gives
# one SSH key a way in: no session of Claude's runs it (docs/ops.md, "The copy
# to a second box"; doc 52 section 5).
#
#   bash /usr/local/lib/rustmush-backup/pull-access.sh '<public key line>' [--peer 50.6.20.140] [--port 2222]
#
# The public key line is what pull-key.sh printed on the box that fetches.
# In this order, stopping at the first thing that fails:
#
#   1. Refuse unless root has a key of its own in /root/.ssh/authorized_keys:
#      turning password logins off must not leave the console as the only door.
#   2. The account `rmbackup` (install.sh take --serve-user): no password, a
#      root-owned authorized_keys the account cannot change.
#   3. PASSWORD LOGINS OFF on this box's sshd. A drop-in,
#      /etc/ssh/sshd_config.d/00-rustmush-no-passwords.conf (it sorts first,
#      and sshd keeps the first value it reads); `sshd -t`; a reload, which
#      ends no session; then the proof: `sshd -T` reads passwords off for root
#      and for rmbackup, and the RUNNING sshd, asked on the port, offers
#      neither `password` nor `keyboard-interactive`. If any of that fails the
#      drop-in is removed, sshd reloaded, and the script stops.
#   4. It WAITS: prove a key login from your own box in a SECOND terminal
#      (`ssh -p 2222 root@<this box> true`), then type `yes`. Anything else
#      removes the drop-in again and stops. This terminal's session is never
#      at risk: a reload does not touch it.
#   5. The one key line, for rmbackup, with a forced command:
#      restrict,command="/usr/local/lib/rustmush-backup/backup-serve.sh" <key>
#      That key can run `list`, `status` and `fetch <archive name>`; nothing else.
#   6. The firewall: ONE ufw rule, TCP <port> from <peer> and nothing wider,
#      and only if ufw does not already admit that port from anywhere.
#   7. Print this box's host key fingerprint and the command that finishes the
#      job on the box that fetches.
#
# To undo: rm /etc/ssh/sshd_config.d/00-rustmush-no-passwords.conf &&
# systemctl reload ssh; : > /var/lib/rmbackup/.ssh/authorized_keys;
# ufw delete allow from <peer> to any port <port> proto tcp.
set -euo pipefail

SYSROOT=${RMBAK_SYSROOT:-}                  # the tests' sandbox; empty on a box
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PEER=50.6.20.140
PORT=2222
KEY=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --peer) PEER=${2:?--peer needs an address}; shift 2 ;;
    --port) PORT=${2:?--port needs a number}; shift 2 ;;
    -*) echo "pull-access: unknown option $1" >&2; exit 2 ;;
    *) if [ -n "$KEY" ]; then echo "pull-access: one public key line, in quotes" >&2; exit 2; fi; KEY=$1; shift ;;
  esac
done
say() { echo "pull-access: $*"; }
die() { echo "pull-access: STOPPED: $*" >&2; exit 1; }

[ -n "$KEY" ] || die "usage: pull-access.sh '<public key line>' [--peer $PEER] [--port $PORT]"
[[ "$PEER" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || die "--peer is one IPv4 address, not '$PEER' (no range, no name)"
[[ "$PORT" =~ ^[0-9]{1,5}$ ]] || die "--port is a number"
[[ "$KEY" =~ ^ssh-ed25519\ AAAA[0-9A-Za-z+/]+=*(\ [A-Za-z0-9@._-]+)?$ ]] || die "that is not one ssh-ed25519 public key line"
if [ -z "$SYSROOT" ] && [ "$(id -u)" -ne 0 ]; then die "run as root"; fi
for tool in sshd ssh ssh-keygen systemctl; do
  command -v "$tool" > /dev/null || die "$tool is not on PATH"
done
keyfile="$(mktemp)"
trap 'rm -f "$keyfile"' EXIT
printf '%s\n' "$KEY" > "$keyfile"
ssh-keygen -lf "$keyfile" > /dev/null || die "ssh-keygen does not read that as a public key"
say "the key to admit: $(ssh-keygen -lf "$keyfile")"

DROPIN="$SYSROOT/etc/ssh/sshd_config.d/00-rustmush-no-passwords.conf"
AUTH="$SYSROOT/var/lib/rmbackup/.ssh/authorized_keys"
SERVE=/usr/local/lib/rustmush-backup/backup-serve.sh

# 1. Root's own key.
if ! grep -qE '^[[:space:]]*(ssh-|ecdsa-|sk-)' "$SYSROOT/root/.ssh/authorized_keys" 2> /dev/null; then
  die "root has no key in /root/.ssh/authorized_keys: with passwords off, only the console would let you in. Add your key first."
fi

# 2. The account.
if ! id rmbackup > /dev/null 2>&1 || [ ! -e "$AUTH" ]; then
  say "making the account rmbackup (install.sh take --serve-user)"
  bash "$HERE/install.sh" take --serve-user > /dev/null
fi
[ -e "$AUTH" ] && [ ! -L "$AUTH" ] || die "$AUTH is missing or a symlink"

# 3. Password logins off.
effective() { sshd -T -C "user=$1,host=$PEER,addr=$PEER" 2> /dev/null | grep -E '^(passwordauthentication|kbdinteractiveauthentication|permitrootlogin) ' | sort | tr '\n' ';'; }
reload_sshd() {
  if systemctl is-active --quiet ssh.service; then systemctl reload ssh.service; fi
}
rollback() {
  rm -f "$DROPIN"
  if sshd -t 2> /dev/null; then reload_sshd || true; fi
  echo "pull-access: the drop-in is removed and sshd is as it was" >&2
}
say "sshd before, for root: $(effective root)"
install -d -m 0755 "$(dirname "$DROPIN")"
printf '%s\n' \
  '# Written by deploy/backup/pull-access.sh (doc 52): no password logins on this box.' \
  '# sshd keeps the FIRST value it reads, and this file sorts before the cloud image'"'"'s.' \
  'PasswordAuthentication no' \
  'KbdInteractiveAuthentication no' \
  'PermitRootLogin prohibit-password' > "$DROPIN.new"
chmod 0644 "$DROPIN.new"
mv "$DROPIN.new" "$DROPIN"
if ! sshd -t; then rollback; die "sshd -t refuses the config with the drop-in"; fi
reload_sshd || { rollback; die "sshd did not reload"; }
for who in root rmbackup; do
  now="$(effective "$who")"
  case "$now" in
    *"passwordauthentication no;"*) ;;
    *) rollback; die "sshd -T still reads passwords ON for $who ($now): is sshd_config missing its Include of sshd_config.d?" ;;
  esac
  case "$now" in
    *"kbdinteractiveauthentication no;"*) ;;
    *) rollback; die "sshd -T still reads keyboard-interactive ON for $who ($now)" ;;
  esac
done
case "$(effective root)" in
  *"permitrootlogin without-password;"* | *"permitrootlogin prohibit-password;"*) ;;
  *) rollback; die "sshd -T does not read root as key-only ($(effective root))" ;;
esac
say "sshd -T now, for root: $(effective root)"
# The running daemon, asked on the port: which ways in does it offer?
offered="$(ssh -p "$PORT" -o BatchMode=yes -o PreferredAuthentications=none -o PubkeyAuthentication=no \
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=8 -v root@127.0.0.1 true 2>&1 |
  grep -m1 'Authentications that can continue' || true)"
case "$offered" in
  *password* | *keyboard-interactive*) rollback; die "the RUNNING sshd still offers a password: $offered" ;;
  *publickey*) say "the running sshd on port $PORT offers: ${offered##*: }" ;;
  *) say "NOTE: could not ask the running sshd on 127.0.0.1:$PORT which logins it offers; step 4 is the proof" ;;
esac

# 4. The owner proves a key login before anything else changes.
echo
echo "  Password logins are OFF. This session is not affected."
echo "  In a SECOND terminal, from your own box, prove a key login now:"
echo "      ssh -p $PORT root@<this box> true && echo ok"
echo "  Type yes when it printed ok. Anything else puts sshd back as it was."
printf '  > '
answer=""
read -r answer < "${RMBAK_TTY:-/dev/tty}" || true
if [ "$answer" != yes ]; then rollback; die "not confirmed: nothing else was changed"; fi

# 5. The one key, with its forced command.
printf 'restrict,command="%s" %s\n' "$SERVE" "$KEY" > "$AUTH.new"
if [ -z "$SYSROOT" ]; then chown root:rmbackup "$AUTH.new"; fi
chmod 0640 "$AUTH.new"
mv "$AUTH.new" "$AUTH"
say "rmbackup's one key line is in $AUTH (forced command: list, status, fetch <name>)"

# 6. The firewall: one narrow rule, if one is needed at all.
if command -v ufw > /dev/null && ufw status | grep -q '^Status: active'; then
  if ufw status | grep -E "^$PORT(/tcp)?[[:space:]]+ALLOW[[:space:]]+Anywhere" > /dev/null; then
    say "ufw already admits TCP $PORT from anywhere (the provider's panel is what narrows it): no rule added"
    say "  to narrow ufw itself to the two addresses, by hand: ufw allow from <your address> to any port $PORT proto tcp;"
    say "  ufw allow from $PEER to any port $PORT proto tcp; then ufw delete allow $PORT/tcp"
  else
    ufw allow from "$PEER" to any port "$PORT" proto tcp comment 'rustmush backup pull' > /dev/null
    say "ufw: added TCP $PORT from $PEER, and nothing wider"
  fi
  ufw status | grep -E "(^|[[:space:]])$PORT(/tcp)?[[:space:]]" | sed 's/^/pull-access:   /' || true
else
  say "ufw is not active on this box: no rule added"
fi

# 7. What the other box needs.
fp="$(ssh-keygen -lf "$SYSROOT/etc/ssh/ssh_host_ed25519_key.pub" | awk '{print $2}')"
echo
say "DONE. On the box that fetches, run:"
echo "    bash /usr/local/lib/rustmush-backup/pull-key.sh --finish $fp"
