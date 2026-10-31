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
# It reuses dev2's Redis (6791), instance.env, TLS, and env from ../dev2.
#
# Idempotent: safe to re-run for a redeploy. Knobs (environment, or --force):
#   SERVER_RESTART=1  restart the server unit even if its binary is unchanged
#   GATEWAY_SWAP=1    swap the gateway generation even if its binary is unchanged
#   FORCE=1 / --force finish a still-draining generation now (SIGTERM, one 30 s
#                     warning) where the deploy would otherwise refuse: a swap
#                     while the idle generation still drains, or a server deploy
#                     that bumps IPC_VERSION while a generation drains (it could
#                     not re-attach to the new server)
#   GATEWAY_MIGRATE=1 the ONE-TIME move from the pre-GG.4 single
#                     dev2mush-gateway.service to the generations (connections
#                     drop once); cut GG.5, with the owner
#
# NOTE (web): as of M16d the gateway ALSO serves the web client (HTTPS :6793) on
# the same session layer as SSH, and web connections survive the hot restart
# AND the swap. The gateway must be BUILT with `--features tls` to terminate
# HTTPS from the on-disk cert/key (see README.md here for the build step). This
# deploy.sh only installs config/units + (re)starts; it does not build.
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

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    *) echo "usage: deploy.sh [--force]  (knobs: SERVER_RESTART=1 GATEWAY_SWAP=1 FORCE=1 GATEWAY_MIGRATE=1)" >&2; exit 2 ;;
  esac
done

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
# `RuntimeDirectory=` (the gateway only connects to it). The gateway
# generations' markers and Discord lock live in the template's
# `StateDirectory=` (/var/lib/dev2mush-gateway, made by systemd), and their
# trusted sockets in the doorway's /run/rustmush-ssh. /var/log/redis stays
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
# restarted, and leaving the live mushd.toml alone. The gateway is checked AS
# a generation: without `--generation`, a symlinked [ssh].trusted_socket (the
# generations' handoff link, doc 33 §4.4) is a refusal by design.
bash "$INSTANCE_DIR/render.sh" \
  "$ROOT/instance.env" "$INSTANCE_DIR/mushd.toml.tmpl" "$ROOT/mushd.toml" \
  "$BUILD/target/release/mush-gateway --generation a" "$BUILD/target/release/mush-server"

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

# 7. The units: the server, and the gateway TEMPLATE (dev2mush-gateway@.service;
#    its instances are @a and @b, doc 33 §4.7). A gateway unit from before GG.1
#    supervised the server as its child; that gateway, if it is the one
#    running, is stopped in step 9 before the pair comes up (stopping it kills
#    its child), which is the ONE deploy that drops connections. Remembered
#    here, before the unit files change.
OLD_GATEWAY_SUPERVISES=0
if systemctl is-active --quiet dev2mush-gateway.service 2>/dev/null && \
   ! [ -f /etc/systemd/system/dev2mush-server.service ]; then
  OLD_GATEWAY_SUPERVISES=1
fi
install -m 644 "$SELF/dev2mush-server.service" /etc/systemd/system/dev2mush-server.service
install -m 644 "$SELF/dev2mush-gateway@.service" /etc/systemd/system/dev2mush-gateway@.service

# 8. Ownership: whole tree is the mush user's EXCEPT the key file (root:mush
#    640). `chown -R` does not touch modes, so the two 0700 key directories
#    provisioned in step 2 stay 0700.
chown -R mush:mush "$ROOT"
if [ -f "$ROOT/dev2.env" ]; then chown root:mush "$ROOT/dev2.env"; chmod 640 "$ROOT/dev2.env"; fi

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

# The helpers of a two-generation unit. A drain does not change the unit's
# state (it stays `active`): the generation that serves is the active one
# WITHOUT a draining-<gen> marker in the StateDirectory.
gen_active()   { systemctl is-active --quiet "$GW_UNIT@$1.service" 2>/dev/null; }
gen_draining() { gen_active "$1" && [ -f "$GW_STATE/draining-$1" ]; }
serving_gen()  { local g; for g in a b; do if gen_active "$g" && [ ! -f "$GW_STATE/draining-$g" ]; then echo "$g"; return 0; fi; done; return 1; }
draining_gen() { local g; for g in a b; do if gen_draining "$g"; then echo "$g"; return 0; fi; done; return 1; }
# What a unit is executing: /proc/<MainPID>/exe reads (and runs) the binary the
# unit was started with even after the build replaced the file on disk.
running_exe()  { local pid; pid="$(systemctl show -p MainPID --value "$1" 2>/dev/null || true)"; if [ -n "$pid" ] && [ "$pid" != 0 ]; then echo "/proc/$pid/exe"; else echo "/proc/0/exe"; fi; }

# 9b. The server.
systemctl enable dev2mush-server.service
if systemctl is-active --quiet dev2mush-server.service; then
  SERVER_EXE="$(running_exe dev2mush-server.service)"
  SERVER_CHANGED="$(bash "$DRAIN_DIR/binary-changed.sh" "$BUILD/target/release/mush-server" "$SERVER_EXE")"
  if [ "${SERVER_RESTART:-0}" = "1" ] || [ "$SERVER_CHANGED" = "changed" ]; then
    NEW_IPC="$("$BUILD/target/release/mush-server" --ipc-version)"
    OLD_IPC="$("$SERVER_EXE" --ipc-version 2>/dev/null)" || OLD_IPC=unknown
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
    START=a
    if gen_draining a; then START=b; fi
    if DRAINING="$(draining_gen)"; then
      echo "WARNING: $GW_UNIT@$DRAINING is draining and no generation serves — starting $GW_UNIT@$START" >&2
    fi
    rm -f "$GW_STATE/draining-$START"
    systemctl reset-failed "$GW_UNIT@$START.service" 2>/dev/null || true
    systemctl enable "$GW_UNIT@$START.service"
    systemctl start "$GW_UNIT@$START.service"
    TRUSTED="$(sed -n 's/^[[:space:]]*trusted_socket[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' "$ROOT/mushd.toml" | head -1)"
    if [ -n "$TRUSTED" ]; then
      case "$TRUSTED" in
        *.sock) TARGET="${TRUSTED%.sock}.$START.sock" ;;
        *)      TARGET="$TRUSTED.$START" ;;
      esac
      if [ ! -d "$(dirname "$TRUSTED")" ]; then
        echo "WARNING: $(dirname "$TRUSTED") does not exist (the doorway's runtime directory): no trusted link made; gateway-swap.sh makes it on the first swap" >&2
      elif [ -L "$TRUSTED" ] || [ ! -e "$TRUSTED" ]; then
        ln -sfn "$TARGET" "$TRUSTED.new"
        mv -T "$TRUSTED.new" "$TRUSTED"
        echo "trusted link: $TRUSTED -> $TARGET"
      else
        echo "WARNING: $TRUSTED exists and is not a symlink (a gateway from before GG.4 bound it?): remove it and re-run," >&2
        echo "or the doorway will not reach $GW_UNIT@$START" >&2
      fi
    fi
  fi
fi

sleep 2
systemctl --no-pager --lines=0 status dev2mush-server.service | head -4 || true
for g in a b; do
  if gen_active "$g"; then
    STATE=serving
    if [ -f "$GW_STATE/draining-$g" ]; then STATE=draining; fi
    echo "$GW_UNIT@$g: $STATE"
    systemctl --no-pager --lines=0 status "$GW_UNIT@$g.service" | head -4 || true
  fi
done
echo "== done. ssh -p 6792 play@$EXT_IP  |  web https://$EXT_IP:6793  (server change: hot restart, held; gateway change: generation swap, drained; see README.md) =="
