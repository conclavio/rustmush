#!/usr/bin/env bash
# Switch the dev2 instance to the SPLIT, hot-restartable deployment and (re)start
# it. Run as root. The split runs TWO services (doc 33 GG.1): dev2mush-server,
# the world (its own systemd unit, `Type=notify`, restarted by systemd), and
# dev2mush-gateway, which owns the SSH + WEB connections, dials the server over
# the [ipc] socket and holds every connection while the server is away. A
# redeploy restarts ONLY the server unit, so no player is dropped (M16c/M16d).
# It reuses dev2's Redis (6791), instance.env, TLS, and env from ../dev2.
#
# Idempotent: safe to re-run for a redeploy. On a redeploy it does the
# hold-the-connections HOT restart (`systemctl restart dev2mush-server`; the
# gateway is left running) — see step 9 — unless GATEWAY_RESTART=1 is set,
# which is the flag for a commit that moved the GATEWAY half itself (see
# README.md here for which those are).
#
# NOTE (web): as of M16d the gateway ALSO serves the web client (HTTPS :6793) on
# the same session layer as SSH, and web connections survive the hot restart too.
# The gateway must be BUILT with `--features tls` to terminate HTTPS from the
# on-disk cert/key (see README.md here for the build step). This deploy.sh only
# installs config/units + (re)starts; it does not build.
set -euo pipefail

ROOT=/opt/rustmush/dev2
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"    # .../build/deploy/dev2-split
DEV2="$SELF/../dev2"                      # shared redis/instance.env/cert setup
INSTANCE_DIR="$SELF/../instance"          # the shared template + render.sh

echo "== rustmush dev2 SPLIT (hot-restart) deploy =="

# 0. Refuse a stale embedded client by source commit, including rollbacks.
# npm run build stamps web/dist/.commit; git archive carries .source-commit.
# The shared guard runs before any installed configuration or service changes.
bash "$BUILD/scripts/check-web-bundle.sh" "$BUILD"

# 1. Service user (shared across mush instances).
if ! id mush >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mush
  echo "created system user 'mush'"
fi

# 2. Directories (same tree the all-in-one uses, provisioned the same way —
# every path the RENDERED config writes to, owned by the unit's User=, rather
# than created on first boot with whatever umask the unit happened to have):
#   data/                 [server].data_dir
#   data/logs/            [log].file = true writes daily-rotating copies here
#                         (gateway.log and server.log — one per half)
#   data/archive/         [store.archive].dir, the J5 ledger tier — this holds
#                         MONEY; back it up with the store, not instead of it
#   data/offer-keys/      [offers].key_dir   — K5 offer signing, mode 0700
#   data/resolution-keys/ [resolution].key_dir — K4 resolution RNG, mode 0700
#   data/geology-keys/    [tacsim.geology].key_dir — doc 20 geology, mode 0700
# All of them are under $ROOT, which is both units' one `ReadWritePaths=`
# entry; the IPC socket's /run/rustmush-dev2 comes from the SERVER unit's
# `RuntimeDirectory=` (the gateway only connects to it). /var/log/redis stays
# redis-owned.
install -d -o mush -g mush -m 0750 "$ROOT" "$ROOT/data" "$ROOT/data/logs" "$ROOT/data/archive" "$ROOT/tls"
install -d -o mush -g mush -m 0700 "$ROOT/data/offer-keys" "$ROOT/data/resolution-keys" "$ROOT/data/geology-keys"
mkdir -p /var/lib/redis-dev2mush /var/log/redis /run/redis-dev2mush
chown redis:redis /var/lib/redis-dev2mush /run/redis-dev2mush /var/log/redis

# 3. Redis config + unit (reused verbatim from the all-in-one instance).
install -m 640 -o redis -g redis "$DEV2/redis-dev2mush.conf" /etc/redis/redis-dev2mush.conf
install -m 644 "$DEV2/redis-dev2mush.service" /etc/systemd/system/redis-dev2mush.service

# 4. Resolve the EXTERNAL IPv4 (source IP of the default route — the public
#    interface, NOT the tailscale address), install dev2's instance.env with
#    that address written into it, and render mushd.toml from the shared
#    template. Same file, same recipe, same result as the all-in-one: the two
#    shapes differ in which binaries read the config, never in the config.
#
#    The committed instance.env carries `EXTERNAL_IP=auto`: the box's address is
#    not a fact about the repository. The INSTALLED copy is the resolved one,
#    and it is the file a re-render reads and the one
#    `scripts/reset-instance.sh /opt/rustmush/dev2/instance.env --yes` is
#    pointed at. ([ipc].listen is a unix socket under /run/rustmush-dev2 and needs no
#    substitution — the server unit's RuntimeDirectory provides that dir, and
#    both halves read the same value: the server binds it, the gateway dials it.)
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if [ -z "${EXT_IP:-}" ]; then echo "FATAL: could not resolve external IP" >&2; exit 1; fi
echo "external IP: $EXT_IP  (ssh :6792, ipc unix /run/rustmush-dev2/dev2.sock)"
#
#    Staged the way render.sh stages the config it produces: the installed
#    instance.env is what `scripts/reset-instance.sh` is pointed at, and a shell
#    redirection truncates its target before `sed` has read a byte — so a ship
#    that left $DEV2/instance.env behind, or a full disk, would replace a good
#    file with an empty one. Write beside it, verify, then move.
sed "s|^EXTERNAL_IP=.*|EXTERNAL_IP=$EXT_IP|" "$DEV2/instance.env" > "$ROOT/instance.env.new"
if ! grep -q "^EXTERNAL_IP=$EXT_IP\$" "$ROOT/instance.env.new"; then
  echo "FATAL: $DEV2/instance.env has no EXTERNAL_IP line to resolve" >&2
  rm -f "$ROOT/instance.env.new"; exit 1
fi
mv "$ROOT/instance.env.new" "$ROOT/instance.env"

# The render is deploy/instance/render.sh — one copy of the sed loop and the
# completeness check for every box. It also runs BOTH split halves over the
# result with `--check-config` (the gateway reads the shared set plus the
# boundary; the server half is the one that dies on [tacsim] and the archive
# window) and fails HERE if either refuses — before any unit is started or
# restarted, and leaving the live mushd.toml alone.
bash "$INSTANCE_DIR/render.sh" \
  "$ROOT/instance.env" "$INSTANCE_DIR/mushd.toml.tmpl" "$ROOT/mushd.toml" \
  "$BUILD/target/release/mush-gateway" "$BUILD/target/release/mush-server"

# 5. TLS cert — the gateway serves the web client (HTTPS :6793) and terminates
#    TLS from this cert/key (M16d; gateway built with --features tls).
if [ ! -f "$ROOT/tls/cert.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 825 \
    -keyout "$ROOT/tls/key.pem" -out "$ROOT/tls/cert.pem" \
    -subj "/CN=conclav.io" \
    -addext "subjectAltName=DNS:conclav.io,IP:$EXT_IP" >/dev/null 2>&1
  echo "generated self-signed cert (825d)"
fi

# 6. LLM keys env file (root:mush 640; injected into the SERVER unit only —
#    the gateway never sees it, doc 33 GG.1).
if [ ! -f "$ROOT/dev2.env" ]; then
  echo "WARNING: $ROOT/dev2.env missing — LLM calls will fail until it exists" >&2
fi

# 7. The two units. A gateway unit from before GG.1 supervised the server as
#    its child; that gateway, if it is the one running, is stopped in step 9
#    before the pair comes up (stopping it kills its child), which is the ONE
#    deploy that drops connections. Remembered here, before the unit file is
#    overwritten.
OLD_GATEWAY_SUPERVISES=0
if systemctl is-active --quiet dev2mush-gateway.service && \
   ! [ -f /etc/systemd/system/dev2mush-server.service ]; then
  OLD_GATEWAY_SUPERVISES=1
fi
install -m 644 "$SELF/dev2mush-server.service" /etc/systemd/system/dev2mush-server.service
install -m 644 "$SELF/dev2mush-gateway.service" /etc/systemd/system/dev2mush-gateway.service

# 8. Ownership: whole tree is the mush user's EXCEPT the key file (root:mush
#    640). `chown -R` does not touch modes, so the two 0700 key directories
#    provisioned in step 2 stay 0700.
chown -R mush:mush "$ROOT"
if [ -f "$ROOT/dev2.env" ]; then chown root:mush "$ROOT/dev2.env"; chmod 640 "$ROOT/dev2.env"; fi

# 9. Bring up Redis, then the server, then the gateway. If the all-in-one
#    dev2mush is running, it is stopped + disabled first (the two cannot share
#    the SSH port). A redeploy where the pair is ALREADY running does the HOT
#    restart: `systemctl restart dev2mush-server` restarts only the server
#    while the gateway holds every SSH and web connection (M16c; doc 33 GG.1)
#    — players see "restarting"/"Reconnected", not a drop. The gateway is not
#    touched: `systemctl stop dev2mush-gateway` no longer stops the server,
#    and nothing about the server unit ever restarts the gateway.
#
#    GATEWAY_RESTART=1 asks for a full `systemctl restart dev2mush-gateway` as
#    well (16b cut Q.5). The server restart moves only the SERVER half, so a
#    commit that changed the gateway half — the embedded SPA, the web or SSH
#    ingress, the session layer, the renderer, the wire types or the IPC
#    boundary — is still running the old code after a server restart, in the
#    one half nothing says so. Connections drop once; that is the price of the
#    half that holds them having moved.
#
#    The server is left alone by a gateway restart (doc 33 GG.2): the
#    gateway's connections go link-dead in the world and nothing else happens.
systemctl daemon-reload
systemctl enable --now redis-dev2mush.service
sleep 1
redis-cli -p 6791 ping >/dev/null && echo "redis-dev2mush: PONG on 6791"

if systemctl is-enabled --quiet dev2mush.service 2>/dev/null || \
   systemctl is-active --quiet dev2mush.service 2>/dev/null; then
  echo "stopping the all-in-one dev2mush (switching to the split)"
  systemctl disable --now dev2mush.service || true
fi

if [ "$OLD_GATEWAY_SUPERVISES" = "1" ]; then
  echo "the running gateway predates the server unit (doc 33 GG.1): stopping it — it kills"
  echo "its server child, so connections drop ONCE on this deploy — then starting the pair"
  systemctl stop dev2mush-gateway.service
fi

systemctl enable dev2mush-server.service dev2mush-gateway.service
if systemctl is-active --quiet dev2mush-server.service; then
  echo "server already up — HOT restart (systemctl restart dev2mush-server): the gateway holds connections"
  systemctl restart dev2mush-server.service
else
  # A unit that exhausted its start limit (Q-33-5: 5 starts in 60 s) stays
  # failed until this is cleared; a fresh deploy is the moment to clear it.
  systemctl reset-failed dev2mush-server.service 2>/dev/null || true
  systemctl start dev2mush-server.service
fi
if systemctl is-active --quiet dev2mush-gateway.service; then
  if [ "${GATEWAY_RESTART:-0}" = "1" ]; then
    echo "GATEWAY_RESTART=1 — FULL gateway restart: connections drop once"
    systemctl restart dev2mush-gateway.service
  fi
else
  systemctl reset-failed dev2mush-gateway.service 2>/dev/null || true
  systemctl start dev2mush-gateway.service
fi
sleep 2
systemctl --no-pager --lines=0 status dev2mush-server.service | head -4 || true
systemctl --no-pager --lines=0 status dev2mush-gateway.service | head -4 || true
echo "== done. ssh -p 6792 play@$EXT_IP  |  web https://$EXT_IP:6793  (hot-restart: systemctl restart dev2mush-server; gateway-half change: GATEWAY_RESTART=1) =="
