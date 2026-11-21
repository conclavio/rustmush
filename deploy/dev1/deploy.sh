#!/usr/bin/env bash
# Configure + (re)start the rustmush dev1 instance on this box. Run as root.
# Assumes the source tree is at /opt/rustmush/dev1/build and the release binary
# has been built there (cargo build --release; --features tls only if the env
# renders the native-TLS [web] shape — dev1 is behind Caddy and does not).
# Idempotent:
# safe to re-run for a redeploy (it will restart the services).
set -euo pipefail

ROOT=/opt/rustmush/dev1
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"   # .../build/deploy/dev1
INSTANCE_DIR="$SELF/../instance"        # the shared template + render.sh
REDIS_SOCK=/run/redis-dev1mush/redis.sock

echo "== rustmush dev1 deploy =="

if [ -L "$ROOT" ] || [ -L "$BUILD" ]; then
  echo "FATAL: $ROOT and $BUILD must not be symlinks" >&2
  exit 1
fi

# 1. Service user (shared across dev1/dev2 dev boxes; production should isolate
#    dedicated per-instance/per-half users — see docs/ops.md).
if ! id mush >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mush
  echo "created system user 'mush'"
fi

# 2. Directories and root/service ownership boundary.
#    $ROOT, $ROOT/build, instance.env, and mushd.toml stay root:root (0755/0644)
#    so User=mush cannot modify binaries, deploy scripts, or configs. Only
#    $ROOT/data (and its subdirs) is owned by mush:mush:
#      data/                 [server].data_dir
#      data/logs/            [log].file = true writes daily-rotating copies here
#      data/archive/         [store.archive].dir, the J5 ledger tier
#      data/offer-keys/      [offers].key_dir   — K5 offer signing, mode 0700
#      data/resolution-keys/ [resolution].key_dir — K4 resolution RNG, mode 0700
#      data/geology-keys/    [tacsim.geology].key_dir — doc 20 geology, mode 0700
#    /var/log/redis stays redis:redis; /run/redis-dev1mush is redis:mush 0750 so
#    mush can connect to the Unix socket while ghrunner/sshdoorway/caddy cannot.
install -d -o root -g root -m 0755 "$ROOT"
if [ -d "$BUILD" ]; then
  chown -Rh root:root "$BUILD"
fi
if [ -L "$ROOT/data" ] || [ -L "$ROOT/tls" ]; then
  echo "FATAL: $ROOT/data and $ROOT/tls must not be symlinks" >&2
  exit 1
fi
install -d -o mush -g mush -m 0750 "$ROOT/data" "$ROOT/data/logs" "$ROOT/data/archive"
install -d -o mush -g mush -m 0700 "$ROOT/data/offer-keys" "$ROOT/data/resolution-keys" "$ROOT/data/geology-keys"
chown -Rh mush:mush "$ROOT/data"
install -d -o root -g mush -m 0750 "$ROOT/tls"
install -d -o redis -g redis -m 0700 /var/lib/redis-dev1mush
install -d -o redis -g redis -m 0750 /var/log/redis
install -d -o redis -g mush -m 0750 /run/redis-dev1mush

# Refuse a stale client before changing installed configs or services (run
# unprivileged as mush so root never executes build-tree scripts directly).
runuser -u mush -- bash "$BUILD/scripts/check-web-bundle.sh" "$BUILD"

# 3. Second Redis config + unit (Unix socket only, port 0).
REDIS_CHANGED=0
if ! cmp -s "$SELF/redis-dev1mush.conf" /etc/redis/redis-dev1mush.conf 2>/dev/null || \
   ! cmp -s "$SELF/redis-dev1mush.service" /etc/systemd/system/redis-dev1mush.service 2>/dev/null; then
  REDIS_CHANGED=1
fi
install -m 640 -o redis -g redis "$SELF/redis-dev1mush.conf" /etc/redis/redis-dev1mush.conf
install -m 644 -o root -g root "$SELF/redis-dev1mush.service" /etc/systemd/system/redis-dev1mush.service

# 4. Resolve the EXTERNAL IPv4 (source IP of the default route — this is the
#    public interface, NOT the tailscale address, which is not the default
#    route), install this instance's instance.env with that address written into
#    it, and render mushd.toml from the shared template.
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if ! [[ "${EXT_IP:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "FATAL: could not resolve external IPv4 (got '${EXT_IP:-}')" >&2
  exit 1
fi
echo "external IP: $EXT_IP  (ssh :6789; web on 127.0.0.1:6790 behind Caddy; redis unix $REDIS_SOCK)"

if [ -L "$ROOT/instance.env" ] || [ -L "$ROOT/mushd.toml" ]; then
  echo "FATAL: $ROOT/instance.env and $ROOT/mushd.toml must not be symlinks" >&2
  exit 1
fi
ENV_STAGED="$(mktemp "$ROOT/.instance.env.new.XXXXXX")"
sed "s|^EXTERNAL_IP=.*|EXTERNAL_IP=$EXT_IP|" "$SELF/instance.env" > "$ENV_STAGED"
if ! grep -q "^EXTERNAL_IP=$EXT_IP\$" "$ENV_STAGED"; then
  echo "FATAL: $SELF/instance.env has no EXTERNAL_IP line to resolve" >&2
  rm -f "$ENV_STAGED"
  exit 1
fi
chown root:root "$ENV_STAGED"
chmod 0644 "$ENV_STAGED"
mv "$ENV_STAGED" "$ROOT/instance.env"

# Render mushd.toml and run --check-config unprivileged as mush.
RENDER_CHECK_USER=mush bash "$INSTANCE_DIR/render.sh" \
  "$ROOT/instance.env" "$INSTANCE_DIR/mushd.toml.tmpl" "$ROOT/mushd.toml" \
  "$BUILD/target/release/mushd"
chown root:root "$ROOT/mushd.toml"
chmod 0644 "$ROOT/mushd.toml"

# 5. Self-signed TLS cert if absent. Read only by an env that renders the
#    native-TLS [web] shape; dev1 is behind Caddy and renders no `tls` key.
if [ -L "$ROOT/tls/cert.pem" ] || [ -L "$ROOT/tls/key.pem" ]; then
  echo "FATAL: $ROOT/tls cert/key must not be symlinks" >&2
  exit 1
fi
if [ ! -f "$ROOT/tls/cert.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 825 \
    -keyout "$ROOT/tls/key.pem" -out "$ROOT/tls/cert.pem" \
    -subj "/CN=conclav.io" \
    -addext "subjectAltName=DNS:conclav.io,IP:$EXT_IP" >/dev/null 2>&1
  echo "generated self-signed cert (825d)"
fi
if [ -f "$ROOT/tls/cert.pem" ]; then
  chown root:mush "$ROOT/tls/cert.pem"
  chmod 0644 "$ROOT/tls/cert.pem"
fi
if [ -f "$ROOT/tls/key.pem" ]; then
  chown root:mush "$ROOT/tls/key.pem"
  chmod 0640 "$ROOT/tls/key.pem"
fi

# 6. LLM keys env file. Owned root:root 0600 so systemd PID 1 reads and injects
#    the keys as root before dropping to User=mush; mush can never read it off disk.
if [ -L "$ROOT/dev1.env" ]; then
  echo "FATAL: $ROOT/dev1.env must not be a symlink" >&2
  exit 1
fi
if [ -f "$ROOT/dev1.env" ]; then
  chown root:root "$ROOT/dev1.env"
  chmod 0600 "$ROOT/dev1.env"
else
  echo "WARNING: $ROOT/dev1.env missing — LLM calls will fail until it exists" >&2
  echo "         (scp your ANTHROPIC_API_KEY/OPENROUTER_API_KEY there, chown root:root, chmod 600)" >&2
fi

# 7. mushd unit.
install -m 644 -o root -g root "$SELF/dev1mush.service" /etc/systemd/system/dev1mush.service

# 8. Start everything.
systemctl daemon-reload
systemctl enable redis-dev1mush.service
if [ "$REDIS_CHANGED" = "1" ] || ! systemctl is-active --quiet redis-dev1mush.service || [ ! -S "$REDIS_SOCK" ]; then
  systemctl restart redis-dev1mush.service
fi
sleep 1
redis-cli -s "$REDIS_SOCK" ping >/dev/null && echo "redis-dev1mush: PONG on $REDIS_SOCK"
systemctl enable dev1mush.service
systemctl restart dev1mush.service
sleep 2
systemctl --no-pager --lines=0 status dev1mush.service | head -4 || true
echo "== done. ssh -p 6789 play@$EXT_IP  |  https://dev1.conclav.io (Caddy -> 127.0.0.1:6790) =="
