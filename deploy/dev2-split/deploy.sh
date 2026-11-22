#!/usr/bin/env bash
# Switch the dev2 instance to the SPLIT, hot-restartable deployment and (re)start
# it. Run as root. The split runs the server unit and the gateway GENERATIONS
# (doc 33 GG.1, GG.4): dev2mush-server, the world (its own systemd unit,
# `Type=notify`, restarted by systemd), and dev2mush-gateway@a / @b, two
# instances of one template unit, of which ONE serves at a time. The serving
# generation owns the SSH + WEB connections, dials the server over the [ipc]
# socket and holds every connection while the server is away. A redeploy:
#   - restarts ONLY the server unit, and only when the server binary changed:
#     the gateway holds every connection, nobody is dropped (M16c/M16d);
#   - SWAPS the gateway generation, and only when the gateway binary changed
#     (Q-33-6): the idle generation starts on the same ports, the serving one
#     is told to drain (warnings on the doorway's schedule, closes when empty
#     or at 24 h) — nobody is dropped either (doc 33 §4.5–§4.7).
# It reuses dev2's Redis (Unix socket only: /run/redis-dev2mush/redis.sock),
# instance.env, TLS, and env from ../dev2.
#
# Idempotent: safe to re-run for a redeploy. Knobs (environment, or --force):
#   SERVER_RESTART=1  restart the server unit even if its binary is unchanged
#   GATEWAY_SWAP=1    swap the gateway generation even if its binary is unchanged
#   SERVER_HOLD=1     HOLD the world down (doc 44 cut MW.1): when dev2mush-server
#                     is not running, write /opt/rustmush/dev2/server.hold before
#                     anything below can start it. The server unit's
#                     `AssertPathExists=!` then refuses EVERY start while the
#                     file exists: this script's, the one the gateway
#                     generation's `Wants=` pulls in, boot's, a hand `systemctl
#                     start`. deploy/instance/launch-bringup.py lifts it when it
#                     starts the world on an Active launch. A RUNNING server is
#                     restarted as usual and no hold is set. A hold that is
#                     already set (reset-instance.sh --no-start, a bring-up that
#                     stopped) is KEPT with or without this knob: the deploy says
#                     so and starts nothing. By hand: rm /opt/rustmush/dev2/server.hold
#   REDIS_RESTART=1   accept that a changed redis-dev2mush.conf/.service restarts
#                     Redis while the server or a gateway generation runs: every
#                     unit that `Requires=` Redis is restarted with it, the
#                     serving generation included, so every connection drops
#                     once and a draining generation's sessions end. Without it
#                     that deploy is refused before anything is installed. Not
#                     needed when nothing of the instance runs (a wipe stops both
#                     halves first)
#   FORCE=1 / --force finish a still-draining generation now (SIGTERM, one 30 s
#                     warning) where the deploy would otherwise refuse: a swap
#                     while the idle generation still drains, or a server deploy
#                     that bumps IPC_VERSION while a generation drains (it could
#                     not re-attach to the new server)
#   GATEWAY_MIGRATE=1 the ONE-TIME move from the pre-GG.4 single
#                     dev2mush-gateway.service to the generations (connections
#                     drop once); cut GG.5, with the owner
#
# NOTE (web): as of M16d the gateway ALSO serves the web client on the same
# session layer as SSH, and web connections survive the hot restart AND the
# swap. Since 2026-10-31 that listener is PLAIN HTTP on 127.0.0.1:6793 behind
# Caddy, which terminates TLS for dev2.conclav.io (instance.env's WEB_BIND /
# WEB_TLS / WEB_TRUSTED_PROXIES; deploy/README.md → "Behind Caddy"). A change
# to the [web] block is a GATEWAY change the binary comparison cannot see:
# deploy it with GATEWAY_SWAP=1. This deploy.sh only installs config/units +
# (re)starts; it does not build.
set -euo pipefail

ROOT=/opt/rustmush/dev2
BUILD=$ROOT/build
SELF="$(cd "$(dirname "$0")" && pwd)"    # .../build/deploy/dev2-split
DEV2="$SELF/../dev2"                      # shared redis/instance.env/cert setup
INSTANCE_DIR="$SELF/../instance"          # the shared template + render.sh
DRAIN_DIR="$SELF/../drain"                # swap.sh, binary-changed.sh, the sysctl
GW_UNIT=dev2mush-gateway                  # the template: $GW_UNIT@a, $GW_UNIT@b
GW_STATE=/var/lib/$GW_UNIT                # its StateDirectory: draining-<gen> markers
SINGLE_UNIT=/etc/systemd/system/dev2mush-gateway.service   # pre-GG.4
REDIS_SOCK=/run/redis-dev2mush/redis.sock
HOLD=$ROOT/server.hold                    # the server unit's AssertPathExists=!
# The units a Redis restart reaches (each Requires= Redis), and the ones the
# deploy waits on before it reads which generation serves.
REDIS_DEPENDENTS=(dev2mush-server.service "$GW_UNIT@a.service" "$GW_UNIT@b.service" dev2mush-gateway.service dev2mush.service)
SETTLE_UNITS=(redis-dev2mush.service dev2mush-server.service "$GW_UNIT@a.service" "$GW_UNIT@b.service")
# gen_*, serving_gen, draining_gen, running_exe, gen_first_start,
# gens_one_enabled, units_settle, server_hold_set, server_held.
# (shellcheck checks deploy/drain/units.sh on its own)
# shellcheck source=/dev/null
. "$DRAIN_DIR/units.sh"

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    *) echo "usage: deploy.sh [--force]  (knobs: SERVER_RESTART=1 GATEWAY_SWAP=1 SERVER_HOLD=1 REDIS_RESTART=1 FORCE=1 GATEWAY_MIGRATE=1)" >&2; exit 2 ;;
  esac
done

echo "== rustmush dev2 SPLIT (hot-restart) deploy =="

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
#    $ROOT/data (which is both units' ReadWritePaths= entry) is owned by mush:mush:
#      data/                 [server].data_dir
#      data/logs/            [log].file = true writes daily-rotating copies here
#                            (gateway.log and server.log — one per half)
#      data/archive/         [store.archive].dir, the J5 ledger tier — this holds
#                            MONEY; back it up with the store, not instead of it
#      data/offer-keys/      [offers].key_dir   — K5 offer signing, mode 0700
#      data/resolution-keys/ [resolution].key_dir — K4 resolution RNG, mode 0700
#      data/geology-keys/    [tacsim.geology].key_dir — doc 20 geology, mode 0700
#    The IPC socket's /run/rustmush-dev2 comes from the SERVER unit's RuntimeDirectory=
#    (the gateway only connects to it). The gateway generations' markers and
#    Discord lock live in the template's StateDirectory= (/var/lib/dev2mush-gateway),
#    and their trusted sockets in the doorway's /run/rustmush-ssh. /var/log/redis
#    stays redis:redis; /run/redis-dev2mush is redis:mush 0750 for Unix-socket Redis.
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
install -d -o redis -g redis -m 0700 /var/lib/redis-dev2mush
install -d -o redis -g redis -m 0750 /var/log/redis
install -d -o redis -g mush -m 0750 /run/redis-dev2mush

# 0. Refuse a stale embedded client by source commit, including rollbacks.
#    Runs unprivileged as mush before any installed config or service changes.
runuser -u mush -- bash "$BUILD/scripts/check-web-bundle.sh" "$BUILD"

# 3. Redis config + unit (Unix socket only, port 0).
REDIS_CHANGED=0
if ! cmp -s "$DEV2/redis-dev2mush.conf" /etc/redis/redis-dev2mush.conf 2>/dev/null || \
   ! cmp -s "$DEV2/redis-dev2mush.service" /etc/systemd/system/redis-dev2mush.service 2>/dev/null; then
  REDIS_CHANGED=1
fi
# A Redis restart is not Redis's alone: the server unit and every gateway
# generation `Requires=` it, so systemd stops them with it and starts them
# again, and `systemctl restart` returns before those starts are done (seen on
# both boxes 2026-11-22). The serving generation's restart drops every
# connection. So while anything of the instance runs, a deploy that would
# restart Redis is refused HERE, before a file is installed (a re-run must
# still see the change), unless REDIS_RESTART=1 says the drop is meant.
REDIS_RESTART_NEEDED=0
if [ "$REDIS_CHANGED" = "1" ] || ! systemctl is-active --quiet redis-dev2mush.service || [ ! -S "$REDIS_SOCK" ]; then
  REDIS_RESTART_NEEDED=1
fi
REDIS_REACHES=()
if [ "$REDIS_RESTART_NEEDED" = "1" ] && systemctl is-active --quiet redis-dev2mush.service; then
  if ! units_settle 600 "${SETTLE_UNITS[@]}"; then
    echo "FATAL: systemd is still moving dev2's units; nothing was changed" >&2
    exit 1
  fi
  mapfile -t REDIS_REACHES < <(units_active "${REDIS_DEPENDENTS[@]}")
fi
if [ "${#REDIS_REACHES[@]}" -gt 0 ] && [ "${REDIS_RESTART:-0}" != "1" ]; then
  echo "REFUSED: this deploy restarts redis-dev2mush (its config or unit changed, or its socket is missing)," >&2
  echo "and every unit that Requires= it is restarted with it: ${REDIS_REACHES[*]}." >&2
  echo "The serving gateway generation's restart drops every connection once, and a draining one's" >&2
  echo "sessions end. Nothing was installed or restarted. To go ahead (with the owner):" >&2
  echo "  REDIS_RESTART=1 bash $0" >&2
  echo "or stop both halves first, as a wipe does (deploy/README.md, \"Wipe and redeploy\")." >&2
  exit 1
fi
install -m 640 -o redis -g redis "$DEV2/redis-dev2mush.conf" /etc/redis/redis-dev2mush.conf
install -m 644 -o root -g root "$DEV2/redis-dev2mush.service" /etc/systemd/system/redis-dev2mush.service

# 4. Resolve the EXTERNAL IPv4 (source IP of the default route — the public
#    interface, NOT the tailscale address), install dev2's instance.env with
#    that address written into it, and render mushd.toml from the shared
#    template.
EXT_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1)"
if ! [[ "${EXT_IP:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "FATAL: could not resolve external IPv4 (got '${EXT_IP:-}')" >&2
  exit 1
fi
echo "external IP: $EXT_IP  (ssh :6792, ipc unix /run/rustmush-dev2/dev2.sock, redis unix $REDIS_SOCK)"

if [ -L "$ROOT/instance.env" ] || [ -L "$ROOT/mushd.toml" ]; then
  echo "FATAL: $ROOT/instance.env and $ROOT/mushd.toml must not be symlinks" >&2
  exit 1
fi
ENV_STAGED="$(mktemp "$ROOT/.instance.env.new.XXXXXX")"
sed "s|^EXTERNAL_IP=.*|EXTERNAL_IP=$EXT_IP|" "$DEV2/instance.env" > "$ENV_STAGED"
if ! grep -q "^EXTERNAL_IP=$EXT_IP\$" "$ENV_STAGED"; then
  echo "FATAL: $DEV2/instance.env has no EXTERNAL_IP line to resolve" >&2
  rm -f "$ENV_STAGED"
  exit 1
fi
chown root:root "$ENV_STAGED"
chmod 0644 "$ENV_STAGED"
mv "$ENV_STAGED" "$ROOT/instance.env"

# Render mushd.toml and run BOTH split halves' --check-config unprivileged as mush.
RENDER_CHECK_USER=mush bash "$INSTANCE_DIR/render.sh" \
  "$ROOT/instance.env" "$INSTANCE_DIR/mushd.toml.tmpl" "$ROOT/mushd.toml" \
  "$BUILD/target/release/mush-gateway --generation a" "$BUILD/target/release/mush-server"
chown root:root "$ROOT/mushd.toml"
chmod 0644 "$ROOT/mushd.toml"

# 5. TLS cert — read by the gateway only when instance.env renders the native
#    TLS shape (WEB_TLS = a `tls = { ... }` line; gateway built with --features
#    tls). dev2 is behind Caddy and renders no `tls` key, so nothing reads this
#    pair there; it is kept for an env that does.
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

# 6. LLM keys env file (root:root 0600; read by systemd PID 1 as root and
#    injected into the SERVER unit only — the gateway omits EnvironmentFile=
#    and mush can never read dev2.env off disk, doc 33 GG.1).
if [ -L "$ROOT/dev2.env" ]; then
  echo "FATAL: $ROOT/dev2.env must not be a symlink" >&2
  exit 1
fi
if [ -f "$ROOT/dev2.env" ]; then
  chown root:root "$ROOT/dev2.env"
  chmod 0600 "$ROOT/dev2.env"
else
  echo "WARNING: $ROOT/dev2.env missing — LLM calls will fail until it exists" >&2
  echo "         (scp your ANTHROPIC_API_KEY/OPENROUTER_API_KEY there, chown root:root, chmod 600)" >&2
fi

# 7. The units: the server, and the gateway TEMPLATE (dev2mush-gateway@.service;
#    its instances are @a and @b, doc 33 §4.7).
OLD_GATEWAY_SUPERVISES=0
if systemctl is-active --quiet dev2mush-gateway.service 2>/dev/null && \
   ! [ -f /etc/systemd/system/dev2mush-server.service ]; then
  OLD_GATEWAY_SUPERVISES=1
fi
install -m 644 -o root -g root "$SELF/dev2mush-server.service" /etc/systemd/system/dev2mush-server.service
install -m 644 -o root -g root "$SELF/dev2mush-gateway@.service" /etc/systemd/system/dev2mush-gateway@.service

# 9. Bring up Redis, then the server, then the gateway generation. If the
#    all-in-one dev2mush is running, it is stopped + disabled first (the two
#    cannot share the SSH port). A redeploy where the pair is ALREADY running:
#
#    - SERVER: `systemctl restart dev2mush-server` when the server binary
#      changed (or SERVER_RESTART=1). It restarts only the server while the
#      gateway holds every SSH and web connection (M16c; doc 33 GG.1) —
#      players see "restarting"/"Reconnected", not a drop. A restart that
#      bumps IPC_VERSION (the two binaries' `--ipc-version`) is refused while a
#      gateway generation is draining — that generation speaks the old version
#      and could not re-attach — unless FORCE=1 finishes it first (§4.7).
#
#    - GATEWAY: `gateway-swap.sh` when the gateway binary changed (or
#      GATEWAY_SWAP=1): the idle generation starts on the same ports
#      (SO_REUSEPORT), stays up 5 s, takes the trusted link, is enabled; the
#      serving one is disabled and told to drain (SIGUSR1). Its sessions are
#      warned on the doorway's schedule and nobody is dropped; it exits when
#      empty or at 24 h and leaves its marker (doc 33 §4.5–§4.7). A commit that
#      changed the gateway half — the embedded SPA, the web or SSH ingress, the
#      session layer, the renderer, the wire types or the IPC boundary — moves
#      this way; nothing is ever `systemctl restart`ed on the gateway side.
#
#    Neither half's restart touches the other (doc 33 GG.2): the server is
#    left alone by a swap, and a server restart never restarts a gateway.
#
#    Redis is another matter: both halves `Requires=` it, so its restart (step
#    3 refused one under a running instance unless REDIS_RESTART=1) restarts
#    them too, and its propagated jobs are still running when `systemctl
#    restart` returns. Everything below reads the units only after systemd has
#    finished with them (units_settle), so a generation in the middle of its
#    restart is never taken for "no generation serves".
systemctl daemon-reload
# The hold first, before the Redis restart or a generation's start can pull
# the server in through the gateway's Wants= (doc 44 cut MW.1).
if [ "${SERVER_HOLD:-0}" = "1" ] && ! systemctl is-active --quiet dev2mush-server.service; then
  server_hold_set "$HOLD" "SERVER_HOLD=1 deploy.sh"
  echo "SERVER_HOLD=1 — the world is HELD: $HOLD (every start of dev2mush-server is refused until it goes)"
fi
systemctl enable redis-dev2mush.service
if [ "$REDIS_RESTART_NEEDED" = "1" ]; then
  if [ "${#REDIS_REACHES[@]}" -gt 0 ]; then
    echo "REDIS_RESTART=1 — restarting redis-dev2mush restarts ${REDIS_REACHES[*]} with it: connections drop once"
  fi
  systemctl restart redis-dev2mush.service
fi
if ! units_settle 600 "${SETTLE_UNITS[@]}"; then
  echo "FATAL: systemd is still moving dev2's units 600 s after the Redis step; see systemctl list-jobs" >&2
  exit 1
fi
sleep 1
redis-cli -s "$REDIS_SOCK" ping >/dev/null && echo "redis-dev2mush: PONG on $REDIS_SOCK"

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

# 9a. Queued connections survive the swap only with net.ipv4.tcp_migrate_req = 1
#     (doc 33 §4.4): the kernel moves what sits in the closing listener's accept
#     queue to the sibling. The doorway's sysctl file sets it box-wide where the
#     doorway is installed; otherwise the gateway's own copy does.
if [ ! -f /etc/sysctl.d/99-sshdoorway.conf ]; then
  install -m 644 "$DRAIN_DIR/99-tcp-migrate-req.conf" /etc/sysctl.d/99-rustmush-gateway.conf
  sysctl -q -p /etc/sysctl.d/99-rustmush-gateway.conf || true
fi
if [ "$(cat /proc/sys/net/ipv4/tcp_migrate_req 2>/dev/null || echo 0)" != "1" ]; then
  echo "WARNING: net.ipv4.tcp_migrate_req is not 1 — connections queued at the moment of a swap are reset, not moved" >&2
fi

# The helpers of a two-generation unit (gen_active, serving_gen, draining_gen,
# running_exe, gen_first_start, gens_one_enabled) are ../drain/units.sh's,
# sourced above. A drain does not change the unit's state (it stays `active`):
# the generation that serves is the active one WITHOUT a draining-<gen> marker
# in the StateDirectory.

# 9b. The server.
systemctl enable dev2mush-server.service
if systemctl is-active --quiet dev2mush-server.service && server_held "$HOLD"; then
  # A hold set while the world ran: a restart would stop it and the hold would
  # refuse the start (the deploy would die there with the world down).
  echo "WARNING: dev2mush-server runs but is HELD ($HOLD): a restart would stop it and leave it" >&2
  echo "         down. It is NOT restarted. Lift the hold (rm $HOLD) and re-run to restart it, or" >&2
  echo "         stop it to keep it held." >&2
elif systemctl is-active --quiet dev2mush-server.service; then
  SERVER_EXE="$(running_exe dev2mush-server.service)"
  SERVER_CHANGED="$(bash "$DRAIN_DIR/binary-changed.sh" "$BUILD/target/release/mush-server" "$SERVER_EXE")"
  if [ "${SERVER_RESTART:-0}" = "1" ] || [ "$SERVER_CHANGED" = "changed" ]; then
    NEW_IPC="$(runuser -u mush -- "$BUILD/target/release/mush-server" --ipc-version)"
    OLD_IPC="$(runuser -u mush -- "$SERVER_EXE" --ipc-version 2>/dev/null)" || OLD_IPC=unknown
    if [ "$NEW_IPC" != "$OLD_IPC" ]; then
      if DRAINING="$(draining_gen)"; then
        if [ "${FORCE:-0}" = "1" ]; then
          echo "IPC_VERSION $OLD_IPC -> $NEW_IPC while $GW_UNIT@$DRAINING drains: FORCE=1 — finishing it (SIGTERM: one 30 s warning, then close)"
          systemctl kill --signal=TERM "$GW_UNIT@$DRAINING.service"
          for _ in $(seq 1 120); do gen_active "$DRAINING" || break; sleep 1; done
          if gen_active "$DRAINING"; then
            echo "FATAL: $GW_UNIT@$DRAINING did not exit within 120 s of SIGTERM; the server is left running" >&2
            exit 1
          fi
        else
          echo "REFUSED: this server speaks IPC_VERSION $NEW_IPC, the running one $OLD_IPC, and $GW_UNIT@$DRAINING is" >&2
          echo "still draining with the old version: its held players could not re-attach to the new server." >&2
          echo "Wait for it to exit (it does when empty, or at its deadline), or FORCE=1 to finish it now" >&2
          echo "(one 30 s warning, then its sessions close). The server was not restarted." >&2
          exit 1
        fi
      fi
      echo "IPC_VERSION $OLD_IPC -> $NEW_IPC: a lockstep change; the serving gateway must move too (it swaps below if its binary changed)"
    fi
    echo "server binary $SERVER_CHANGED — HOT restart (systemctl restart dev2mush-server): the gateway holds connections"
    systemctl restart dev2mush-server.service
  else
    echo "server binary unchanged — dev2mush-server is left running (SERVER_RESTART=1 restarts it anyway)"
  fi
elif server_held "$HOLD"; then
  # Doc 44 cut MW.1: a wiped box that is about to take the named launch. The
  # world stays down, so nobody can log in or register on a store that is
  # still being brought up; the gateway below starts and holds the front door,
  # and the start its Wants= pulls in is refused by the hold.
  echo "dev2mush-server is installed and enabled, and HELD: NOT started. $HOLD says:"
  echo "  $(head -c 300 "$HOLD" 2>/dev/null || true)"
  echo "Bring the launch up, which lifts the hold and starts the world when the Active boot is ready:"
  echo "  python3 $INSTANCE_DIR/launch-bringup.py $ROOT/instance.env"
  echo "(to start the world without the launch instead: rm $HOLD && systemctl start dev2mush-server)"
else
  # A unit that exhausted its start limit (Q-33-5: 5 starts in 60 s) stays
  # failed until this is cleared; a fresh deploy is the moment to clear it.
  systemctl reset-failed dev2mush-server.service 2>/dev/null || true
  systemctl start dev2mush-server.service
fi

# 9c. The gateway. First, the single-unit dev2mush-gateway.service of the
#     deployments before GG.4: it cannot share the ports with a generation, so
#     while it serves, the generations are installed but NOT started — the
#     move (connections drop once) is cut GG.5, with the owner, as
#     GATEWAY_MIGRATE=1. A single unit that is installed but not running is
#     retired here (nothing drops), so it never comes back at boot beside a
#     generation.
retire_single_unit() {
  systemctl disable dev2mush-gateway.service 2>/dev/null || true
  rm -f "$SINGLE_UNIT"
  systemctl daemon-reload
}
GATEWAY_SKIP=0
if [ -f "$SINGLE_UNIT" ]; then
  if systemctl is-active --quiet dev2mush-gateway.service 2>/dev/null; then
    if [ "${GATEWAY_MIGRATE:-0}" = "1" ]; then
      echo "GATEWAY_MIGRATE=1 — moving from the single dev2mush-gateway.service to the generations: connections drop ONCE"
      systemctl stop dev2mush-gateway.service
      retire_single_unit
    else
      echo "the single-unit dev2mush-gateway.service (before doc 33 GG.4) is serving; left as it is."
      echo "The generation template is installed. The move is cut GG.5, with the owner:"
      echo "  GATEWAY_MIGRATE=1 bash $0    # stops the single unit (connections drop ONCE), starts $GW_UNIT@a, makes the trusted link"
      echo "After that every gateway change is a swap (gateway-swap.sh), and nobody is dropped again."
      GATEWAY_SKIP=1
    fi
  else
    echo "retiring the stopped single-unit dev2mush-gateway.service (the gateway is $GW_UNIT@a|b now)"
    retire_single_unit
  fi
fi

if [ "$GATEWAY_SKIP" != "1" ]; then
  if SERVING="$(serving_gen)"; then
    GW_CHANGED="$(bash "$DRAIN_DIR/binary-changed.sh" "$BUILD/target/release/mush-gateway" "$(running_exe "$GW_UNIT@$SERVING.service")")"
    if [ "${GATEWAY_SWAP:-0}" = "1" ] || [ "$GW_CHANGED" = "changed" ]; then
      echo "gateway binary $GW_CHANGED — swapping generations: $GW_UNIT@$SERVING will drain, nobody is dropped"
      SWAP_ARGS=(dev2)
      if [ "${FORCE:-0}" = "1" ]; then SWAP_ARGS+=(--force); fi
      bash "$SELF/gateway-swap.sh" "${SWAP_ARGS[@]}"
    else
      echo "gateway binary unchanged — $GW_UNIT@$SERVING keeps serving (GATEWAY_SWAP=1 swaps anyway)"
    fi
  else
    # No generation serves: the first start, or both are down. Start one —
    # the one that is not still draining — with its marker cleared, and make
    # the trusted link the doorway dials (doc 33 §4.4): the configured
    # [ssh].trusted_socket is a symlink to the serving generation's own
    # socket beside it, which only the swap script re-points from here on.
    # The units were read after units_settle, so a generation that is in the
    # middle of a restart is not "no generation"; and the one started is the
    # only one left enabled (gen_first_start disables the other).
    START=a
    if gen_draining a; then START=b; fi
    if DRAINING="$(draining_gen)"; then
      echo "WARNING: $GW_UNIT@$DRAINING is draining and no generation serves — starting $GW_UNIT@$START" >&2
    fi
    gen_first_start "$START"
    TRUSTED="$(sed -n 's/^[[:space:]]*trusted_socket[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/mushd.toml" | head -1)"
    if [ -n "$TRUSTED" ]; then
      if ! [[ "$TRUSTED" =~ ^/run/rustmush-ssh/[A-Za-z0-9._-]+$ ]]; then
        echo "FATAL: refusing unexpected [ssh].trusted_socket outside /run/rustmush-ssh/: $TRUSTED" >&2
        exit 1
      fi
      case "$TRUSTED" in
        *.sock) TARGET="${TRUSTED%.sock}.$START.sock" ;;
        *)      TARGET="$TRUSTED.$START" ;;
      esac
      if [ ! -d "$(dirname "$TRUSTED")" ] || [ -L "$(dirname "$TRUSTED")" ]; then
        echo "WARNING: $(dirname "$TRUSTED") does not exist (the doorway's runtime directory): no trusted link made; gateway-swap.sh makes it on the first swap" >&2
      elif [ -L "$TRUSTED" ] || [ ! -e "$TRUSTED" ]; then
        # shellcheck disable=SC2016
        runuser -u mush -- bash -c '
          set -euo pipefail
          tmp="$(mktemp -u "${2}.new.XXXXXX")"
          ln -s "$1" "$tmp"
          mv -T "$tmp" "$2"
        ' _ "$TARGET" "$TRUSTED"
        echo "trusted link: $TRUSTED -> $TARGET"
      else
        echo "WARNING: $TRUSTED exists and is not a symlink (a gateway from before GG.4 bound it?): remove it and re-run," >&2
        echo "or the doorway will not reach $GW_UNIT@$START" >&2
      fi
    fi
  fi
fi

# Exactly one generation is enabled, or a reboot starts both side by side.
if ! gens_one_enabled; then
  echo "FATAL: disable the generation that does not serve (systemctl disable $GW_UNIT@<gen>)" >&2
  exit 1
fi

sleep 2
if server_held "$HOLD"; then
  echo "dev2mush-server: HELD ($HOLD) — launch-bringup.py lifts it when it starts the world"
fi
systemctl --no-pager --lines=0 status dev2mush-server.service | head -4 || true
for g in a b; do
  if gen_active "$g"; then
    STATE=serving
    if [ -f "$GW_STATE/draining-$g" ]; then STATE=draining; fi
    echo "$GW_UNIT@$g: $STATE"
    systemctl --no-pager --lines=0 status "$GW_UNIT@$g.service" | head -4 || true
  fi
done
echo "== done. ssh -p 6792 play@$EXT_IP  |  web https://dev2.conclav.io (Caddy -> 127.0.0.1:6793)  (server change: hot restart, held; gateway change: generation swap, drained; see README.md) =="
