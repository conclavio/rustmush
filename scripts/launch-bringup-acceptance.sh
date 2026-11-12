#!/usr/bin/env bash
#
# launch-bringup-acceptance.sh — doc 44 cut MW.1's proof, on a throwaway store.
#
# It stands a whole instance up in a scratch directory — its own Redis on a
# spare port, a rendered mushd.toml, the split pair's release binaries — and
# drives it the way a dev box is driven: scripts/reset-instance.sh, the
# render, and deploy/instance/launch-bringup.py as root over a unit. The unit
# is a `systemctl` shim on PATH that starts and stops the scratch instance's
# own mush-server, so the scripts under test run unchanged and no real unit,
# no real instance and no other Redis is touched. NEVER point it at a box's
# own instance: everything it names is under the scratch directory.
#
# What it holds:
#   1. on a fresh store the bring-up walks the whole runbook and `@mind/launch`
#      names every operator `ready`;
#   2. the same run, interrupted once and started again, ends the same way;
#   3. no player can log in or register before the Active boot: a probe types
#      `create` at the PUBLIC front door every few seconds for the whole
#      bring-up and is never let in, and is let in once the unit is started;
#   4. a House with three titled hulls, a funded treasury and a rated listing
#      buys a licence by its own agent, and the hulls go back to the Co-op.
#      This is the long step: a rating on a launch world is hours. It runs
#      last, on the second bring-up's live world; SEED=0 leaves it out;
#   5. a reset (the real scripts/reset-instance.sh) takes launch_path out, and
#      a second bring-up on the wiped store ends the same way;
#   6. rendering with the switch off is the plain substitution, byte for byte
#      (scripts/tests/render-launch.sh).
#
# Usage (root, Linux; redis-server with the JSON module, python3, ssh):
#   sudo OWNER=<user> scripts/launch-bringup-acceptance.sh <binary dir> [<work dir>]
#
# OWNER is the unprivileged user the instance runs as (it must be able to read
# the tree and the binaries). Knobs: REDIS_PORT (6391), SSH_PORT (16895),
# INTERRUPT_S (seconds into the first run it is interrupted, 45), PULSE (the
# instance's REGISTRY_PULSE_MS, 50), SEED=0 (leave the hours-long step 4 out),
# BRINGUP_ARGS (extra bring-up arguments, e.g. `--seed-line standard`), LLM=stub
# (answer the LLM routes from a local stub and require the LLM check to pass;
# without it the check is required to FAIL, since no provider answers), and
# KEEP=1 to leave the scratch directory and its Redis up for a look.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
tree="$PWD"
bin="$(realpath "${1:?usage: launch-bringup-acceptance.sh <binary dir> [<work dir>]}")"
work="${2:-$(mktemp -d /var/tmp/mw1-acceptance.XXXXXX)}"
owner="${OWNER:?set OWNER to the unprivileged user the instance runs as}"
redis_port="${REDIS_PORT:-6391}"
ssh_port="${SSH_PORT:-16895}"
interrupt_s="${INTERRUPT_S:-45}"
[ "$(id -u)" -eq 0 ] || { echo "acceptance: must run as root (the scripts under test stop and start a unit)" >&2; exit 1; }
for b in mush-server mush-gateway; do
  [ -x "$bin/$b" ] || { echo "acceptance: $bin/$b is not built" >&2; exit 1; }
done
mkdir -p "$work"
work="$(realpath "$work")"
install -d -o "$owner" -g "$owner" -m 0750 "$work" "$work/inst" "$work/inst/data" "$work/redis" "$work/shim"
inst="$work/inst"
unit=mw1mush-server.service
# scripts/reset-instance.sh finds the binaries under <BUILD_DIR>/target/release.
if [ ! -e "$tree/target/release/mush-server" ]; then
  mkdir -p "$tree/target"
  ln -sfn "$bin" "$tree/target/release"
fi
# The render and the bring-up under test are the tree's own. With LLM=stub they
# run from a copy whose template names the local stub as every provider's
# base_url: the one deviation from a box, and the bring-up renders more than
# once, so it has to be in the template it renders from.
deploy_dir="$tree/deploy/instance"
LLM_ENV=""
if [ "${LLM:-}" = stub ]; then
  LLM_ENV="env OPENROUTER_API_KEY=stub ANTHROPIC_API_KEY=stub"
  mkdir -p "$work/tree/deploy"
  cp -r "$tree/deploy/instance" "$work/tree/deploy/"
  ln -sfn "$tree/crates" "$work/tree/crates"
  deploy_dir="$work/tree/deploy/instance"
  sed -i "s|^base_url = .*|base_url = \"http://127.0.0.1:$((ssh_port + 5))/v1\"|" "$deploy_dir/mushd.toml.tmpl"
  chown -R "$owner:$owner" "$work/tree"
fi
log() { echo "acceptance: $*"; }

# --- the unit: a systemctl that knows one unit, the scratch instance's world.
cat > "$work/shim/systemctl" << EOF
#!/usr/bin/env bash
pidfile="$work/server.pid"
alive() { [ -f "\$pidfile" ] && kill -0 "\$(cat "\$pidfile")" 2> /dev/null; }
stop_unit() {
  if alive; then
    kill -TERM "\$(cat "\$pidfile")"
    for _ in \$(seq 1 120); do alive || break; sleep 0.5; done
  fi
  rm -f "\$pidfile"
  echo "\$(date +%s) stop" >> "$work/unit.log"
}
start_unit() {
  alive && return 0
  rm -f "$inst/ipc.sock"
  $LLM_ENV setsid runuser -u "$owner" -- "$bin/mush-server" --config "$inst/mushd.toml" \\
    >> "$work/unit-server.log" 2>&1 < /dev/null &
  echo "\$!" > "\$pidfile"
  echo "\$(date +%s) start" >> "$work/unit.log"
}
case "\$1" in
cat) [ "\$2" = "$unit" ] ;;
show) if [ "\$3" = User ] || [ "\$3" = Group ]; then echo "$owner"; fi ;;
is-active) alive ;;
stop) stop_unit ;;
start) start_unit ;;
restart) stop_unit; start_unit ;;
*) echo "shim systemctl: unexpected \$*" >&2; exit 1 ;;
esac
EOF
chmod +x "$work/shim/systemctl"
export PATH="$work/shim:$PATH"

pids=()
finish() {
  status=$?
  systemctl stop "$unit" 2> /dev/null || true
  for pid in "${pids[@]}"; do kill -TERM "$pid" 2> /dev/null || true; done
  if [ "${KEEP:-0}" != 1 ]; then
    redis-cli -h 127.0.0.1 -p "$redis_port" shutdown nosave > /dev/null 2>&1 || true
  fi
  if [ "$status" -ne 0 ]; then
    echo "acceptance: FAILED (everything is in $work)" >&2
  elif [ "${KEEP:-0}" != 1 ]; then
    rm -rf -- "$work"
  fi
}
trap finish EXIT

# --- the store: this run's own Redis, with the modules the package ships.
modules=()
while read -r _ module; do modules+=(--loadmodule "$module"); done \
  < <(grep -h '^loadmodule ' /etc/redis/redis.conf 2> /dev/null | sort -u)
if redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null 2>&1; then
  echo "acceptance: something already answers on 127.0.0.1:$redis_port; pick another REDIS_PORT" >&2
  exit 1
fi
runuser -u "$owner" -- redis-server --port "$redis_port" --bind 127.0.0.1 --protected-mode yes \
  --dir "$work/redis" --appendonly yes --daemonize yes --pidfile "$work/redis/redis.pid" \
  --logfile "$work/redis/redis.log" "${modules[@]}"
for _ in $(seq 1 30); do redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null 2>&1 && break; sleep 0.5; done
redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null

# --- the instance: a dev box's switches, on loopback.
llm_enabled=false
if [ "${LLM:-}" = stub ]; then llm_enabled=true; fi
cat > "$inst/instance.env" << EOF
INSTANCE=mw1
INSTANCE_TITLE=rustmush mw1 acceptance
DATA_DIR=$inst/data
BUILD_DIR=$tree
EXTERNAL_IP=127.0.0.1
REDIS_PORT=$redis_port
SSH_PORT=$ssh_port
WEB_PORT=$((ssh_port + 1))
WEB_BIND=127.0.0.1
WEB_TLS=# no native tls on the acceptance instance
WEB_TRUSTED_PROXIES=[]
IPC_SOCKET=$inst/ipc.sock
TRUSTED_SOCKET=
LOG_LEVEL=info
LOG_FORMAT=pretty
TACSIM_ENABLED=true
IC_RATIO=24
IC_EPOCH=0987-04-20 06:00
CHARGEN_REQUIRED=false
START_ROOM=0
START_ROOM_UNAPPROVED=0
MINDS_LAUNCH=on
REGISTRY_PULSE_MS=${PULSE:-50}
RAID_DIRECTOR=true
LLM_ENABLED=$llm_enabled
PIXEL_DIAGNOSTICS=false
EOF
head -c 18 /dev/urandom | base64 | tr -d '/+=' > "$work/staff-password"
chmod 600 "$work/staff-password"
render() {
  bash "$deploy_dir/render.sh" "$inst/instance.env" "$deploy_dir/mushd.toml.tmpl" \
    "$inst/mushd.toml" "$bin/mush-gateway" "$bin/mush-server"
  chown "$owner:$owner" "$inst/mushd.toml" "$inst/instance.env"
}
render
if grep -q '^launch_path' "$inst/mushd.toml"; then
  echo "acceptance: a fresh store's render carries a live launch_path" >&2; exit 1
fi
# BRINGUP_ARGS: extra bring-up arguments for every run (for example
# `--seed-line standard`), split on spaces.
read -r -a extra <<< "${BRINGUP_ARGS:-}"
bringup() {
  python3 "$deploy_dir/launch-bringup.py" "$inst/instance.env" --bin-dir "$bin" \
    --password-file "$work/staff-password" "${extra[@]}" "$@"
}

# --- the public front door, up for the whole run (the deploy's SERVER_HOLD shape).
runuser -u "$owner" -- "$bin/mush-gateway" --config "$inst/mushd.toml" > "$work/public-gateway.log" 2>&1 &
pids+=("$!")
for _ in $(seq 1 120); do (exec 3<> "/dev/tcp/127.0.0.1/$ssh_port") 2> /dev/null && break; sleep 0.5; done

# One attempt to register at the public front door: prints `in` or `out`.
probe() {
  local name="$1" out
  out="$({ sleep 2; printf 'create %s probe-pass-%s\r\n' "$name" "$name"; sleep 6; } |
    timeout 20 ssh -tt -p "$ssh_port" -o BatchMode=yes -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR play@127.0.0.1 2>&1 || true)"
  if grep -q 'Account created\.\|Connected as ' <<< "$out"; then echo in; else echo out; fi
}
prober() {
  local n=0 answer
  while [ -f "$work/probing" ]; do
    n=$((n + 1))
    # Stamped when the answer is in: a probe that began before the unit
    # started and was let in after it is not a player who got in early.
    answer="$(probe "probe$1x$n")"
    echo "$(date +%s) $answer" >> "$work/probes.log"
    sleep 3
  done
}
# No probe got in before the unit's latest start, and at least one probe ran.
kept_off() {
  local started
  started="$(awk '$2 == "start" { t = $1 } END { print t + 0 }' "$work/unit.log" 2> /dev/null || echo 0)"
  [ -s "$work/probes.log" ] || { echo "acceptance: no probe ran" >&2; return 1; }
  if [ "$started" = 0 ]; then started=9999999999; fi
  if awk -v s="$started" '$1 < s && $2 == "in" { bad = 1 } END { exit !bad }' "$work/probes.log"; then
    echo "acceptance: a player got in before the Active boot's unit start:" >&2
    cat "$work/probes.log" >&2
    return 1
  fi
  log "$(grep -c ' out$' "$work/probes.log") probes at the public front door were kept out"
}

full_run() {
  # $1 names the run; the rest are extra bring-up arguments.
  local name="$1" out="$work/bringup-$1.log" status=0
  shift
  : > "$work/probes.log"
  : > "$work/unit.log"
  touch "$work/probing"
  prober "$name" &
  local prober_pid=$!
  bringup "$@" > "$out" 2>&1 || status=$?
  rm -f "$work/probing"
  wait "$prober_pid" || true
  cat "$out"
  return "$status"
}
expect_ready() {
  grep -q 'Registry operators: #[0-9]* ready\.' "$1" || { echo "acceptance: $1 names no operator ready" >&2; return 1; }
}
llm_args=(--no-llm-check)
if [ "${LLM:-}" = stub ]; then
  python3 scripts/tests/llm-stub.py "$((ssh_port + 5))" > "$work/llm-stub.log" 2>&1 &
  pids+=("$!")
  llm_args=()
fi

# --- 1 to 3: a fresh store; the first run interrupted, the second to the end.
# Neither rates a line (--no-seed): the seed is step 4's, on the live world.
log "run 1: interrupted after ${interrupt_s}s"
: > "$work/probes.log"; : > "$work/unit.log"; touch "$work/probing"
prober first &
prober_pid=$!
status=0
timeout -s INT "$interrupt_s" python3 "$deploy_dir/launch-bringup.py" "$inst/instance.env" \
  --bin-dir "$bin" --password-file "$work/staff-password" "${extra[@]}" --no-seed --no-llm-check \
  > "$work/bringup-interrupted.log" 2>&1 || status=$?
rm -f "$work/probing"; wait "$prober_pid" || true
tail -5 "$work/bringup-interrupted.log"
[ "$status" -ne 0 ] || { echo "acceptance: the first run finished inside ${interrupt_s}s; lower INTERRUPT_S" >&2; exit 1; }
if pgrep -u "$owner" -f "$inst/data/bringup" > /dev/null; then
  echo "acceptance: the interrupted run left a private boot running" >&2; exit 1
fi
kept_off
log "run 2: the same store, to the end"
full_run second --no-seed "${llm_args[@]}"
expect_ready "$work/bringup-second.log"
kept_off
[ "$(probe afterstart)" = in ] || { echo "acceptance: nobody can register after the unit started" >&2; exit 1; }
log "a player registers once the unit is up"
grep -q '^launch_path = ' "$inst/mushd.toml"
if [ "${LLM:-}" != stub ]; then
  # The negative control: with no provider answering, the check must fail.
  if bringup --llm-check > "$work/llm-check.log" 2>&1; then
    echo "acceptance: the LLM check passed with no provider answering" >&2; exit 1
  fi
  grep -q 'the LLM check FAILED' "$work/llm-check.log"
  log "the LLM check fails with no provider answering, as it must"
fi

# --- 5: the real reset, then a second bring-up on the wiped store.
log "reset: scripts/reset-instance.sh --staff keeper --no-start"
bash scripts/reset-instance.sh "$inst/instance.env" --yes --staff keeper --no-start \
  < "$work/staff-password" > "$work/reset.log" 2>&1 || { cat "$work/reset.log" >&2; exit 1; }
if grep -q '^launch_path' "$inst/mushd.toml"; then
  echo "acceptance: the reset left a live launch_path" >&2; exit 1
fi
grep -q '^# launch_path = ' "$inst/mushd.toml"
[ ! -e "$inst/data/launch.json" ] && [ ! -e "$inst/data/bringup" ]
log "run 3: the second bring-up, on the reset store"
full_run third --no-seed "${llm_args[@]}"
expect_ready "$work/bringup-third.log"
kept_off

# --- 4: the seed line and a House's purchase, on the live world of run 3.
# HOURS: the Registry's rating is a few hundred trial matches. SEED=0 skips it.
if [ "${SEED:-1}" != 0 ]; then
  log "the seed line and the House's purchase (a rating: hours)"
  bringup --seed-only > "$work/bringup-seed.log" 2>&1 || { tail -30 "$work/bringup-seed.log" >&2; exit 1; }
  cat "$work/bringup-seed.log"
  grep -q 'bought licence' "$work/bringup-seed.log" || { echo "acceptance: no House purchase" >&2; exit 1; }
  grep -q 'handed the 3 Drays back' "$work/bringup-seed.log" || { echo "acceptance: the hulls did not go back" >&2; exit 1; }
fi

# --- 6: the switch off.
bash scripts/tests/render-launch.sh
log "PASSED"
grep -h 'rating took\|bought licence\|boot [0-9]' "$work"/bringup-*.log || true
