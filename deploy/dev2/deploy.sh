#!/usr/bin/env bash
# Configure + (re)start the rustmush dev2 instance on this box. Run as root.
# Assumes the source tree is at /opt/rustmush/dev2/build and the release binary
# has been built there (cargo build --release --features tls). Idempotent:
# safe to re-run for a redeploy (it will restart the services).
set -euo pipefail

ROOT=/opt/rustmush/dev2
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"   # .../build/deploy/dev2

echo "== rustmush dev2 deploy =="

# 1. Service user (shared across future mush instances).
if ! id mush >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mush
  echo "created system user 'mush'"
fi

# 2. Directories. /var/log/redis is the SHARED redis log dir — it must stay
# owned by redis (the dev instance logs there too); never chown it to mush.
mkdir -p "$ROOT/data" "$ROOT/tls" /var/lib/redis-dev2mush /var/log/redis /run/redis-dev2mush
chown redis:redis /var/lib/redis-dev2mush /run/redis-dev2mush /var/log/redis

# 3. Second Redis config + unit.
install -m 640 -o redis -g redis "$SELF/redis-dev2mush.conf" /etc/redis/redis-dev2mush.conf
install -m 644 "$SELF/redis-dev2mush.service" /etc/systemd/system/redis-dev2mush.service

# 4. Resolve the EXTERNAL IPv4 (source IP of the default route — this is the
#    public interface, NOT the tailscale address, which is not the default
#    route) and render mushd.toml.
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if [ -z "${EXT_IP:-}" ]; then echo "FATAL: could not resolve external IP" >&2; exit 1; fi
echo "external IP: $EXT_IP  (ssh :6792, https :6793)"
sed "s/__EXTERNAL_IP__/$EXT_IP/g" "$SELF/mushd.toml" > "$ROOT/mushd.toml"

# 5. Self-signed TLS cert for the web (6793) if absent. SANs cover the external
#    IP and conclav.io so browsers at least match the host.
if [ ! -f "$ROOT/tls/cert.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 825 \
    -keyout "$ROOT/tls/key.pem" -out "$ROOT/tls/cert.pem" \
    -subj "/CN=conclav.io" \
    -addext "subjectAltName=DNS:conclav.io,IP:$EXT_IP" >/dev/null 2>&1
  echo "generated self-signed cert (825d)"
fi

# 6. LLM keys env file. Created out-of-band (scp) as root:mush 640 so systemd
#    injects the keys and the mush user never reads them.
if [ ! -f "$ROOT/dev2.env" ]; then
  echo "WARNING: $ROOT/dev2.env missing — LLM calls will fail until it exists" >&2
  echo "         (scp your ANTHROPIC_API_KEY/OPENROUTER_API_KEY there, chmod 640 root:mush)"
fi

# 7. mushd unit.
install -m 644 "$SELF/dev2mush.service" /etc/systemd/system/dev2mush.service

# 8. Ownership: the whole tree is the mush user's EXCEPT the key file (root:mush 640).
chown -R mush:mush "$ROOT"
if [ -f "$ROOT/dev2.env" ]; then chown root:mush "$ROOT/dev2.env"; chmod 640 "$ROOT/dev2.env"; fi

# 9. Start everything.
systemctl daemon-reload
systemctl enable --now redis-dev2mush.service
sleep 1
redis-cli -p 6791 ping >/dev/null && echo "redis-dev2mush: PONG on 6791"
systemctl enable dev2mush.service
systemctl restart dev2mush.service
sleep 2
systemctl --no-pager --lines=0 status dev2mush.service | head -4 || true
echo "== done. ssh -p 6792 play@$EXT_IP  |  https://$EXT_IP:6793 =="
