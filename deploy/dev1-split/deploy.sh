#!/usr/bin/env bash
# Switch the dev1 instance to the SPLIT, hot-restartable deployment and (re)start
# it. Run as root. The split runs ONE service — dev1mush-gateway — which owns the
# SSH + WEB connections and spawns + supervises the mush-server child; a redeploy
# SIGHUPs the gateway to restart ONLY the server child, so no player is dropped
# (M16c/M16d). It reuses dev1's Redis (6788), config, TLS, and env from ../dev1.
#
# Idempotent: safe to re-run for a redeploy. On a redeploy it does the
# hold-the-connections HOT restart (reload, not restart) — see step 9.
#
# NOTE (web): as of M16d the gateway ALSO serves the web client (HTTPS :6790) on
# the same session layer as SSH, and web connections survive the hot restart too.
# The gateway must be BUILT with `--features tls` to terminate HTTPS from the
# on-disk cert/key (see README.md here for the build step). This deploy.sh only
# installs config/units + (re)starts; it does not build.
set -euo pipefail

ROOT=/opt/rustmush/dev1
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"    # .../build/deploy/dev1-split
DEV1="$SELF/../dev1"                      # shared redis/toml/cert setup

echo "== rustmush dev1 SPLIT (hot-restart) deploy =="

# 1. Service user (shared across mush instances).
if ! id mush >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mush
  echo "created system user 'mush'"
fi

# 2. Directories (same tree the all-in-one uses). /var/log/redis stays redis-owned.
mkdir -p "$ROOT/data" "$ROOT/tls" /var/lib/redis-dev1mush /var/log/redis /run/redis-dev1mush
chown redis:redis /var/lib/redis-dev1mush /run/redis-dev1mush /var/log/redis

# 3. Redis config + unit (reused verbatim from the all-in-one instance).
install -m 640 -o redis -g redis "$DEV1/redis-dev1mush.conf" /etc/redis/redis-dev1mush.conf
install -m 644 "$DEV1/redis-dev1mush.service" /etc/systemd/system/redis-dev1mush.service

# 4. Resolve the EXTERNAL IPv4 and render mushd.toml (the same file the all-in-one
#    uses; its [ipc] boundary port is localhost-only and needs no substitution).
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if [ -z "${EXT_IP:-}" ]; then echo "FATAL: could not resolve external IP" >&2; exit 1; fi
echo "external IP: $EXT_IP  (ssh :6789, ipc :6794 localhost)"
sed "s/__EXTERNAL_IP__/$EXT_IP/g" "$DEV1/mushd.toml" > "$ROOT/mushd.toml"

# 5. TLS cert — the gateway serves the web client (HTTPS :6790) and terminates
#    TLS from this cert/key (M16d; gateway built with --features tls).
if [ ! -f "$ROOT/tls/cert.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 825 \
    -keyout "$ROOT/tls/key.pem" -out "$ROOT/tls/cert.pem" \
    -subj "/CN=conclav.io" \
    -addext "subjectAltName=DNS:conclav.io,IP:$EXT_IP" >/dev/null 2>&1
  echo "generated self-signed cert (825d)"
fi

# 6. LLM keys env file (root:mush 640; injected into the gateway, inherited by
#    the server child).
if [ ! -f "$ROOT/dev1.env" ]; then
  echo "WARNING: $ROOT/dev1.env missing — LLM calls will fail until it exists" >&2
fi

# 7. The gateway unit.
install -m 644 "$SELF/dev1mush-gateway.service" /etc/systemd/system/dev1mush-gateway.service

# 8. Ownership: whole tree is the mush user's EXCEPT the key file (root:mush 640).
chown -R mush:mush "$ROOT"
if [ -f "$ROOT/dev1.env" ]; then chown root:mush "$ROOT/dev1.env"; chmod 640 "$ROOT/dev1.env"; fi

# 9. Bring up Redis, then the gateway. If the all-in-one dev1mush is running, it
#    is stopped + disabled first (the two cannot share the SSH port). A redeploy
#    where the gateway is ALREADY running does the HOT restart: `reload` sends
#    SIGHUP, which restarts only the server child while every SSH connection is
#    held (M16c) — players see "restarting"/"reconnected", not a drop.
systemctl daemon-reload
systemctl enable --now redis-dev1mush.service
sleep 1
redis-cli -p 6788 ping >/dev/null && echo "redis-dev1mush: PONG on 6788"

if systemctl is-enabled --quiet dev1mush.service 2>/dev/null || \
   systemctl is-active --quiet dev1mush.service 2>/dev/null; then
  echo "stopping the all-in-one dev1mush (switching to the split)"
  systemctl disable --now dev1mush.service || true
fi

systemctl enable dev1mush-gateway.service
if systemctl is-active --quiet dev1mush-gateway.service; then
  echo "gateway already up — HOT restart (SIGHUP): holding connections"
  systemctl reload dev1mush-gateway.service
else
  systemctl start dev1mush-gateway.service
fi
sleep 2
systemctl --no-pager --lines=0 status dev1mush-gateway.service | head -4 || true
echo "== done. ssh -p 6789 play@$EXT_IP  |  web https://$EXT_IP:6790  (hot-restart: systemctl reload dev1mush-gateway) =="
