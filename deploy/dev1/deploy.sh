#!/usr/bin/env bash
# Configure + (re)start the rustmush dev1 instance on this box. Run as root.
# Assumes the source tree is at /opt/rustmush/dev1/build and the release binary
# has been built there (cargo build --release --features tls). Idempotent:
# safe to re-run for a redeploy (it will restart the services).
set -euo pipefail

ROOT=/opt/rustmush/dev1
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"   # .../build/deploy/dev1
INSTANCE_DIR="$SELF/../instance"        # the shared template + render.sh

echo "== rustmush dev1 deploy =="

# Refuse a stale client before changing the instance, including on rollbacks.
bash "$BUILD/scripts/check-web-bundle.sh" "$BUILD"

# 1. Service user (shared across future mush instances).
if ! id mush >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mush
  echo "created system user 'mush'"
fi

# 2. Directories. Every path the RENDERED config writes to is provisioned here,
# owned by the unit's User=, rather than created on first boot with whatever
# umask the unit happened to have:
#   data/                 [server].data_dir
#   data/logs/            [log].file = true writes daily-rotating copies here
#   data/archive/         [store.archive].dir, the J5 ledger tier — this holds
#                         MONEY; back it up with the store, not instead of it
#   data/offer-keys/      [offers].key_dir   — K5 offer signing, mode 0700
#   data/resolution-keys/ [resolution].key_dir — K4 resolution RNG, mode 0700
#   data/geology-keys/    [tacsim.geology].key_dir — doc 20 geology, mode 0700
# The three key directories are DIFFERENT private key families and are never
# pointed at one directory. /var/log/redis is the SHARED redis log dir — it must
# stay owned by redis (the dev instance logs there too); never chown it to mush.
install -d -o mush -g mush -m 0750 "$ROOT" "$ROOT/data" "$ROOT/data/logs" "$ROOT/data/archive" "$ROOT/tls"
install -d -o mush -g mush -m 0700 "$ROOT/data/offer-keys" "$ROOT/data/resolution-keys" "$ROOT/data/geology-keys"
mkdir -p /var/lib/redis-dev1mush /var/log/redis /run/redis-dev1mush
chown redis:redis /var/lib/redis-dev1mush /run/redis-dev1mush /var/log/redis

# 3. Second Redis config + unit.
install -m 640 -o redis -g redis "$SELF/redis-dev1mush.conf" /etc/redis/redis-dev1mush.conf
install -m 644 "$SELF/redis-dev1mush.service" /etc/systemd/system/redis-dev1mush.service

# 4. Resolve the EXTERNAL IPv4 (source IP of the default route — this is the
#    public interface, NOT the tailscale address, which is not the default
#    route), install this instance's instance.env with that address written into
#    it, and render mushd.toml from the shared template.
#
#    The committed instance.env carries `EXTERNAL_IP=auto`: the box's address is
#    not a fact about the repository. The INSTALLED copy is the resolved one,
#    and it is the file a re-render reads and the one
#    `scripts/reset-instance.sh /opt/rustmush/dev1/instance.env --yes` is
#    pointed at, exactly as deploy/README.md promises for a rendered instance.
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if [ -z "${EXT_IP:-}" ]; then echo "FATAL: could not resolve external IP" >&2; exit 1; fi
echo "external IP: $EXT_IP  (ssh :6789, https :6790)"
#
#    Staged the way render.sh stages the config it produces: the installed
#    instance.env is what `scripts/reset-instance.sh` is pointed at, and a shell
#    redirection truncates its target before `sed` has read a byte — so a ship
#    that left $SELF/instance.env behind, or a full disk, would replace a good
#    file with an empty one. Write beside it, verify, then move.
sed "s|^EXTERNAL_IP=.*|EXTERNAL_IP=$EXT_IP|" "$SELF/instance.env" > "$ROOT/instance.env.new"
if ! grep -q "^EXTERNAL_IP=$EXT_IP\$" "$ROOT/instance.env.new"; then
  echo "FATAL: $SELF/instance.env has no EXTERNAL_IP line to resolve" >&2
  rm -f "$ROOT/instance.env.new"; exit 1
fi
mv "$ROOT/instance.env.new" "$ROOT/instance.env"

# The render is deploy/instance/render.sh — one copy of the sed loop and the
# completeness check for every box. It also runs this deployment's binary over
# the result with `--check-config` and fails HERE, before any unit is started,
# if the config would refuse the boot; the live mushd.toml is left alone when
# it does.
bash "$INSTANCE_DIR/render.sh" \
  "$ROOT/instance.env" "$INSTANCE_DIR/mushd.toml.tmpl" "$ROOT/mushd.toml" \
  "$BUILD/target/release/mushd"

# 5. Self-signed TLS cert for the web (6790) if absent. SANs cover the external
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
if [ ! -f "$ROOT/dev1.env" ]; then
  echo "WARNING: $ROOT/dev1.env missing — LLM calls will fail until it exists" >&2
  echo "         (scp your ANTHROPIC_API_KEY/OPENROUTER_API_KEY there, chmod 640 root:mush)"
fi

# 7. mushd unit.
install -m 644 "$SELF/dev1mush.service" /etc/systemd/system/dev1mush.service

# 8. Ownership: the whole tree is the mush user's EXCEPT the key file (root:mush
#    640). `chown -R` does not touch modes, so the two 0700 key directories
#    provisioned in step 2 stay 0700.
chown -R mush:mush "$ROOT"
if [ -f "$ROOT/dev1.env" ]; then chown root:mush "$ROOT/dev1.env"; chmod 640 "$ROOT/dev1.env"; fi

# 9. Start everything.
systemctl daemon-reload
systemctl enable --now redis-dev1mush.service
sleep 1
redis-cli -p 6788 ping >/dev/null && echo "redis-dev1mush: PONG on 6788"
systemctl enable dev1mush.service
systemctl restart dev1mush.service
sleep 2
systemctl --no-pager --lines=0 status dev1mush.service | head -4 || true
echo "== done. ssh -p 6789 play@$EXT_IP  |  https://$EXT_IP:6790 =="
