#!/usr/bin/env bash
# wipe-cutover.sh — WIPE ONE instance and cut it over to the tree stage-build.sh
# built in /opt/rustmush/<instance>/build.new. deploy/README.md, "Wipe and
# redeploy"; as run on dev1 and dev2 on 2026-11-22 (main ebe5840f). It DESTROYS
# the instance's world: every player connection ends and the store is flushed.
#
#   COMMIT=<sha8> [STAFF=keeper] bash wipe-cutover.sh <instance> --yes
#
# In order:
#   1. preflight the staged tree (built at COMMIT, bundle stamped to match, both
#      binaries, the deploy, the bring-up, a reset that knows --no-start);
#   2. make the staff password file $REDEPLOY_DIR/<instance>-<staff>.pw (random,
#      root 0600, never printed) unless it exists; launch-bringup-run.sh reads it;
#   3. stop BOTH halves: the server and both gateway generations, which are
#      disabled too. A wipe ends every session anyway, and a stopped instance
#      is one deploy.sh starts from cleanly (with nothing running, a Redis
#      change is restarted without its REDIS_RESTART=1 refusal);
#   4. snapshot the store and the data dir to /root/backups/<instance>-<UTC>-pre-<sha8>,
#      through the Redis the LIVE instance.env names (TCP or Unix socket);
#   5. swap the trees (build -> build.old-<UTC>, build.new -> build): the old
#      tree's reset-instance.sh may have no --no-start;
#   6. reset-instance.sh --yes --no-start: the wipe, BEFORE deploy.sh (a new
#      server refuses an old store), and the world's HOLD (server.hold);
#   7. SERVER_HOLD=1 deploy.sh: units, config, Redis; the world stays held and
#      a gateway generation holds the front door.
# The bring-up is NOT run here: launch-bringup-run.sh, after a look at the state
# this prints.
set -euo pipefail
trap 'echo "wipe-cutover: FAILED at line $LINENO: $BASH_COMMAND" >&2' ERR

N=${1:?usage: COMMIT=<sha8> [STAFF=keeper] bash wipe-cutover.sh <instance> --yes}
[ "${2:-}" = --yes ] || {
  echo "wipe-cutover: this DESTROYS $N's world (every connection ends, the store is flushed). Re-run with --yes." >&2
  exit 1
}
COMMIT=${COMMIT:?set COMMIT=<sha8>}
STAFF=${STAFF:-keeper}
DIR=${REDEPLOY_DIR:-/root}
if ! [[ "$N" =~ ^[a-z][a-z0-9]*$ ]]; then
  echo "wipe-cutover: not an instance name: $N" >&2
  exit 2
fi
if ! [[ "$COMMIT" =~ ^[0-9a-f]{8}$ ]] || ! [[ "$STAFF" =~ ^[A-Za-z0-9_-]{3,20}$ ]]; then
  echo "wipe-cutover: COMMIT must be 8 hex digits and STAFF an account name" >&2
  exit 2
fi
[ "$(id -u)" -eq 0 ] || { echo "wipe-cutover: must run as root" >&2; exit 1; }
ROOT=/opt/rustmush/$N
NEW=$ROOT/build.new
TS=$(date -u +%Y%m%d-%H%M%S)
SNAP=/root/backups/$N-$TS-pre-$COMMIT
PW=$DIR/$N-$STAFF.pw
UNITS=("${N}mush-server.service" "${N}mush-gateway@a.service" "${N}mush-gateway@b.service")

# The redis-cli arguments for the Redis an installed instance.env names: its
# REDIS_URL (redis+unix:///<socket> or redis://[user@]host[:port]) or a legacy
# REDIS_PORT. Read, never sourced.
redis_args() {
  local env="$1" url port hp
  url="$(sed -n 's/^REDIS_URL=//p' "$env" | tail -1 | tr -d '\r')"
  port="$(sed -n 's/^REDIS_PORT=//p' "$env" | tail -1 | tr -d '\r')"
  case "$url" in
    redis+unix:///*)
      url="${url#redis+unix://}"
      REDIS=(-s "${url%%\?*}")
      ;;
    redis://*)
      hp="${url#redis://}"
      hp="${hp%%[/?#]*}"
      hp="${hp##*@}"
      if [[ "$hp" == *:* ]]; then REDIS=(-h "${hp%:*}" -p "${hp##*:}"); else REDIS=(-h "$hp" -p 6379); fi
      ;;
    '')
      [ -n "$port" ] || { echo "wipe-cutover: $env names no Redis" >&2; return 1; }
      REDIS=(-h 127.0.0.1 -p "$port")
      ;;
    *)
      echo "wipe-cutover: unsupported REDIS_URL in $env: $url" >&2
      return 1
      ;;
  esac
}

echo "== $(date -u +%FT%TZ) $N: preflight"
[ -d "$ROOT/build" ] && [ -d "$NEW" ] || { echo "wipe-cutover: need $ROOT/build and $NEW (stage-build.sh)" >&2; exit 1; }
if [ -L "$ROOT/build" ] || [ -L "$NEW" ]; then echo "wipe-cutover: the trees must not be symlinks" >&2; exit 1; fi
[ "$(cat "$NEW/.stage-built" 2>/dev/null || true)" = "$(cat "$NEW/.source-commit")" ] || {
  echo "wipe-cutover: $NEW was not built to the end by stage-build.sh (no matching .stage-built)" >&2
  exit 1
}
[ "$(cut -c1-8 "$NEW/.source-commit")" = "$COMMIT" ]
[ "$(cat "$NEW/web/dist/.commit")" = "$(cat "$NEW/.source-commit")" ]
[ -x "$NEW/target/release/mush-server" ]
[ -x "$NEW/target/release/mush-gateway" ]
[ -f "$NEW/deploy/$N-split/deploy.sh" ]
[ -f "$NEW/deploy/instance/launch-bringup.py" ]
grep -q -- '--no-start' "$NEW/scripts/reset-instance.sh"
grep -qx "AssertPathExists=!$ROOT/server.hold" "$NEW/deploy/$N-split/${N}mush-server.service" || {
  echo "wipe-cutover: the staged ${N}mush-server.service does not assert $ROOT/server.hold: nothing would hold the world" >&2
  exit 1
}
redis_args "$ROOT/instance.env"
redis-cli "${REDIS[@]}" ping > /dev/null
echo "running: $(cat "$ROOT/build/.source-commit" 2>/dev/null || echo unknown)  ->  staged: $(cat "$NEW/.source-commit")"
echo "live Redis: ${REDIS[*]}"

# The staff password: random, root-only, never printed. launch-bringup.py reads
# its first line (--password-file) and bootstraps the account with it.
if [ -L "$PW" ]; then echo "wipe-cutover: $PW must not be a symlink" >&2; exit 1; fi
if [ ! -s "$PW" ]; then
  (umask 077; head -c 24 /dev/urandom | base64 | tr -d '/+=\n' > "$PW"; printf '\n' >> "$PW")
fi
chown root:root "$PW"
chmod 600 "$PW"
echo "staff password file: $PW ($(($(wc -c < "$PW") - 1)) characters, mode $(stat -c %a "$PW"))"

echo "== $(date -u +%FT%TZ) $N: stop both halves"
systemctl stop "${N}mush-server.service"
for g in a b; do
  systemctl disable --now "${N}mush-gateway@$g.service" 2> /dev/null || true
done
for u in "${UNITS[@]}"; do
  echo "  $u: $(systemctl is-active "$u" || true)"
  if systemctl is-active --quiet "$u"; then echo "FATAL: $u is still active" >&2; exit 1; fi
done

echo "== $(date -u +%FT%TZ) $N: snapshot -> $SNAP"
mkdir -p "$SNAP"
redis-cli "${REDIS[@]}" save > /dev/null
echo "dbsize before the wipe: $(redis-cli "${REDIS[@]}" dbsize)"
if [ -d "/var/lib/redis-${N}mush" ]; then
  cp -a "/var/lib/redis-${N}mush" "$SNAP/redis"
else
  echo "WARNING: no /var/lib/redis-${N}mush to copy; the snapshot has no store" >&2
fi
cp -a "$ROOT/data" "$SNAP/data"
cp -a "$ROOT/mushd.toml" "$ROOT/instance.env" "$SNAP/"
du -sh "$SNAP"

echo "== $(date -u +%FT%TZ) $N: swap trees"
mv "$ROOT/build" "$ROOT/build.old-$TS"
mv "$NEW" "$ROOT/build"

echo "== $(date -u +%FT%TZ) $N: wipe and hold (reset-instance.sh --yes --no-start)"
bash "$ROOT/build/scripts/reset-instance.sh" "$ROOT/instance.env" --yes --no-start
echo "dbsize after the wipe: $(redis-cli "${REDIS[@]}" dbsize)"

echo "== $(date -u +%FT%TZ) $N: deploy with the world held (SERVER_HOLD=1 deploy.sh)"
SERVER_HOLD=1 bash "$ROOT/build/deploy/$N-split/deploy.sh"

echo "== $(date -u +%FT%TZ) $N: state after the deploy"
sleep 5
for u in "redis-${N}mush.service" "${UNITS[@]}"; do
  echo "  $u: $(systemctl is-active "$u" || true) / $(systemctl is-enabled "$u" 2> /dev/null || true)"
done
if [ -e "$ROOT/server.hold" ]; then
  echo "  hold: $(cat "$ROOT/server.hold")"
else
  echo "  WARNING: no hold at $ROOT/server.hold" >&2
fi
# The deploy may have moved Redis (the new instance.env's REDIS_URL).
redis_args "$ROOT/instance.env"
echo "  redis (${REDIS[*]}): $(redis-cli "${REDIS[@]}" ping) dbsize $(redis-cli "${REDIS[@]}" dbsize)"
ls -la -- /run/rustmush-ssh/"$N".* 2> /dev/null || true
ls -la "/var/lib/${N}mush-gateway/" || true
grep -n 'launch_path\|^redis_url\|wake_majors\|registry_pulse_ms' "$ROOT/mushd.toml" || true
echo "== $(date -u +%FT%TZ) $N: CUTOVER DONE (old tree: $ROOT/build.old-$TS; snapshot: $SNAP)"
echo "next: bash launch-bringup-run.sh $N"
