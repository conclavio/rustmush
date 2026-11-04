#!/usr/bin/env bash
#
# release-smoke.sh — doc 43 cut TS.17: start the release binaries once.
#
# Until this script, nothing in CI had ever run a release binary: the nightly
# release job built the three of them (thin LTO, native TLS, the SPA embedded)
# and threw them away, and the boxes found out at deploy time. This runs what it
# built, the way a box does, and exits:
#
#   1. renders a throwaway instance from the shipped template with the deploy's
#      own deploy/instance/render.sh, which runs all three binaries'
#      `--check-config` over it; the one deviation is the memory store;
#   2. boots `mushd` with a self-signed certificate made here (D-43-13: never
#      committed) and fetches the embedded SPA over TLS, verified against that
#      certificate, byte-equal to web/dist/index.html;
#   3. boots `mush-server` and `mush-gateway` as the split pair, sees the
#      gateway attach over the IPC socket, and plays one front-door session over
#      SSH: `create` (a service request across the boundary) and `look` (a verb
#      the world task answers).
#
# Usage: scripts/release-smoke.sh [<binary dir>]   (default target/release)
# Linux (the IPC socket is a unix path). Needs openssl, curl and ssh. Logs and
# the session transcript are left in $RELEASE_SMOKE_DIR when it is set (CI
# uploads it), else in a temporary directory that is removed on success.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
bin="${1:-target/release}"
for b in mushd mush-server mush-gateway; do
  if [ ! -x "$bin/$b" ]; then
    echo "release-smoke: $bin/$b is not built" >&2
    exit 1
  fi
done
if [ ! -f web/dist/index.html ]; then
  echo "release-smoke: web/dist/index.html is missing: build the SPA first (the embed is part of the check)" >&2
  exit 1
fi

work="${RELEASE_SMOKE_DIR:-$(mktemp -d)}"
mkdir -p "$work/data"
pids=()
stop_all() {
  for pid in "${pids[@]}"; do
    kill -TERM "$pid" 2>/dev/null || true
  done
  for pid in "${pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  pids=()
}
finish() {
  status=$?
  stop_all
  if [ "$status" -ne 0 ]; then
    for log in "$work"/*.log; do
      [ -f "$log" ] || continue
      echo "::group::$log"
      tail -n 80 "$log"
      echo "::endgroup::"
    done
    echo "release-smoke: FAILED (logs in $work)" >&2
  elif [ -z "${RELEASE_SMOKE_DIR:-}" ]; then
    rm -rf "$work"
  fi
}
trap finish EXIT

# Waits up to $2 seconds for the command in the remaining arguments to succeed.
wait_for() {
  local what="$1" limit="$2"
  shift 2
  local waited=0
  until "$@" > /dev/null 2>&1; do
    if [ "$waited" -ge "$limit" ]; then
      echo "release-smoke: gave up after ${limit}s waiting for $what" >&2
      return 1
    fi
    sleep 1
    waited=$((waited + 1))
  done
  echo "release-smoke: $what after ${waited}s"
}
port_open() { (exec 3<> "/dev/tcp/127.0.0.1/$1") 2> /dev/null; }

ssh_port=16795
web_port=16796
socket="$work/ipc.sock"

# 1. The certificate (D-43-13) and the render.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -keyout "$work/key.pem" -out "$work/cert.pem" \
  -subj "/CN=localhost" -addext "subjectAltName=IP:127.0.0.1,DNS:localhost" 2> "$work/openssl.log"
cat > "$work/instance.env" << EOF
INSTANCE=smoke
INSTANCE_TITLE=rustmush release smoke
DATA_DIR=$work/data
BUILD_DIR=$PWD
EXTERNAL_IP=127.0.0.1
REDIS_PORT=6379
SSH_PORT=$ssh_port
WEB_PORT=$web_port
WEB_BIND=127.0.0.1
WEB_TLS=tls = { cert = "$work/cert.pem", key = "$work/key.pem" }
WEB_TRUSTED_PROXIES=[]
IPC_SOCKET=$socket
TRUSTED_SOCKET=
LOG_LEVEL=info
LOG_FORMAT=json
TACSIM_ENABLED=true
IC_RATIO=1
IC_EPOCH=2020-01-01 00:00
CHARGEN_REQUIRED=true
START_ROOM=2
START_ROOM_UNAPPROVED=3
LLM_ENABLED=false
PIXEL_DIAGNOSTICS=false
EOF
bash deploy/instance/render.sh "$work/instance.env" deploy/instance/mushd.toml.tmpl \
  "$work/smoke.toml" "$bin/mush-gateway" "$bin/mush-server" "$bin/mushd"
# The one deviation from a box: no Redis.
sed -i 's/^store = "redis"$/store = "memory"/' "$work/smoke.toml"
grep -q '^store = "memory"$' "$work/smoke.toml"
for b in mush-gateway mush-server mushd; do
  "$bin/$b" --check-config --config "$work/smoke.toml"
done
echo "release-smoke: --check-config passed for all three on the rendered instance"

# 2. The all-in-one, its embedded SPA over TLS.
"$bin/mushd" --config "$work/smoke.toml" > "$work/mushd.log" 2>&1 &
pids+=("$!")
fetch_spa() { curl -sSf --cacert "$work/cert.pem" "https://127.0.0.1:$web_port/" -o "$work/index.html"; }
wait_for "mushd serving https on $web_port" 180 fetch_spa
if ! cmp -s "$work/index.html" web/dist/index.html; then
  echo "release-smoke: the SPA served over TLS is not web/dist/index.html" >&2
  exit 1
fi
echo "release-smoke: mushd served the embedded SPA over TLS"
stop_all

# 3. The split pair: the server, then the gateway, then one SSH session.
rm -f "$socket"
"$bin/mush-server" --config "$work/smoke.toml" > "$work/mush-server.log" 2>&1 &
pids+=("$!")
wait_for "mush-server listening on the IPC socket" 180 test -S "$socket"
"$bin/mush-gateway" --config "$work/smoke.toml" > "$work/mush-gateway.log" 2>&1 &
pids+=("$!")
wait_for "mush-gateway listening for SSH on $ssh_port" 180 port_open "$ssh_port"
{
  sleep 3
  printf 'create smokebot smoke-pass-1\r\n'
  sleep 8
  printf 'look\r\n'
  sleep 5
  printf 'QUIT\r\n'
  sleep 2
} | timeout 60 ssh -T -p "$ssh_port" \
  -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o LogLevel=ERROR play@127.0.0.1 > "$work/session.log" 2>&1 || true
echo "::group::the SSH session"
cat "$work/session.log"
echo "::endgroup::"
if ! grep -qi 'smokebot' "$work/session.log"; then
  echo "release-smoke: the session never named the account it created" >&2
  exit 1
fi
echo "release-smoke: one front-door session crossed the split (create, look)"
