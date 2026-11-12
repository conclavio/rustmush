#!/usr/bin/env bash
#
# bot-minds-lane.sh — doc 44 unit MW.19: the bot's minds lane, on a rig composed
# as the launch composes a live server.
#
# It stands one instance up in a scratch directory the way a dev box is stood
# up: the deploy's own render (deploy/instance/render.sh) from an instance.env
# with the dev boxes' switches, a Redis of its own, the split pair's binaries,
# and the launch brought up by deploy/instance/launch-bringup.py (--no-systemd:
# the rig has no unit). Then two players, each on a login of its own, walk the
# mind market over SSH as the bot (crates/mush-bot, `--minds author` and
# `--minds buyer`): one writes a mind and files it, the Registry rates it, the
# author lists it at the counter, the other buys a licence, flies it in the
# simulator and takes it out on an IC sortie.
#
# Nothing here is typed by staff once the bring-up has ended. The two logins are
# registered at the front door with `create`, and each registers its own SSH
# public key with `@sshkey/add`: what a player does by hand before a bot of
# theirs can connect (the bot never types a credential or an account verb).
#
# Usage (Linux; python3, ssh, ssh-keygen; a Redis with the JSON module):
#   scripts/bot-minds-lane.sh <binary dir> [<work dir>]
#
# <binary dir> holds mush-server, mush-gateway and mush-bot (target/debug in
# CI: the lane is paced by the game's clocks, not by the build's speed).
#
# Knobs: REDIS_PORT (6396) and SSH_PORT (16995), the instance's own; REDIS=own
# (the default: start a redis-server here) or REDIS=external (one already
# answers on REDIS_PORT, as CI's container does; it must be empty); PULSE (the
# instance's REGISTRY_PULSE_MS, 50, the dev boxes' value); UP_ONLY=1 (stand the
# world and the two logins up, print how to reach them, and wait for a signal:
# for a look by hand); KEEP=1 (leave the scratch directory after a pass).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
tree="$PWD"
bin="$(realpath "${1:?usage: bot-minds-lane.sh <binary dir> [<work dir>]}")"
given_work="${2:-}"
work="${given_work:-$(mktemp -d "${TMPDIR:-/tmp}/minds-lane.XXXXXX")}"
redis_port="${REDIS_PORT:-6396}"
ssh_port="${SSH_PORT:-16995}"
for b in mush-server mush-gateway mush-bot; do
  [ -x "$bin/$b" ] || { echo "minds-lane: $bin/$b is not built" >&2; exit 1; }
done
mkdir -p "$work"
work="$(realpath "$work")"
inst="$work/inst"
mkdir -p "$inst/data" "$work/redis" "$work/bots"
log() { echo "minds-lane: $(date -u +%H:%M:%S) $*"; }
started_s="$(date +%s)"
since() { echo "$(( $(date +%s) - started_s ))s"; }

pids=()
own_redis=0
finish() {
  status=$?
  for pid in "${pids[@]}"; do kill -TERM "$pid" 2> /dev/null || true; done
  for pid in "${pids[@]}"; do wait "$pid" 2> /dev/null || true; done
  if [ "$own_redis" = 1 ]; then
    redis-cli -h 127.0.0.1 -p "$redis_port" shutdown nosave > /dev/null 2>&1 || true
  fi
  if [ "$status" -ne 0 ]; then
    echo "minds-lane: FAILED after $(since) (everything is in $work)" >&2
  elif [ "${KEEP:-0}" != 1 ] && [ -z "$given_work" ]; then
    rm -rf -- "$work"
  fi
}
trap finish EXIT

# --- the store: this run's own Redis.
if [ "${REDIS:-own}" = external ]; then
  redis_ping() { (exec 3<> "/dev/tcp/127.0.0.1/$redis_port") 2> /dev/null; }
  for _ in $(seq 1 60); do redis_ping && break; sleep 0.5; done
  redis_ping || { echo "minds-lane: REDIS=external and nothing answers on 127.0.0.1:$redis_port" >&2; exit 1; }
else
  if redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null 2>&1; then
    echo "minds-lane: something already answers on 127.0.0.1:$redis_port; pick another REDIS_PORT" >&2
    exit 1
  fi
  # The JSON module, where the package ships its modules as files (a Redis
  # that has it built in loads none): the directory the package's own
  # redis.conf names them in, which an unprivileged user cannot read.
  modules=()
  for module in "${REDIS_MODULES:-/usr/lib/redis/modules}"/*.so; do
    [ -f "$module" ] && modules+=(--loadmodule "$module")
  done
  redis-server --port "$redis_port" --bind 127.0.0.1 --protected-mode yes \
    --dir "$work/redis" --appendonly yes --daemonize yes --pidfile "$work/redis/redis.pid" \
    --logfile "$work/redis/redis.log" "${modules[@]}"
  own_redis=1
  for _ in $(seq 1 30); do redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null 2>&1 && break; sleep 0.5; done
  redis-cli -h 127.0.0.1 -p "$redis_port" ping > /dev/null
fi

# --- the instance: a dev box's switches (deploy/dev2/instance.env), on loopback.
# IC_RATIO is the live game's 24:1. LLM_ENABLED is false: CI never calls a
# provider, and nothing the lane walks asks one.
cat > "$inst/instance.env" << EOF
INSTANCE=mindslane
INSTANCE_TITLE=rustmush minds lane
DATA_DIR=$inst/data
BUILD_DIR=$tree
EXTERNAL_IP=127.0.0.1
REDIS_PORT=$redis_port
SSH_PORT=$ssh_port
WEB_PORT=$((ssh_port + 1))
WEB_BIND=127.0.0.1
WEB_TLS=# no native tls on the lane's instance
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
LLM_ENABLED=false
PIXEL_DIAGNOSTICS=false
EOF
head -c 18 /dev/urandom | base64 | tr -d '/+=' > "$work/staff-password"
chmod 600 "$work/staff-password"
bash deploy/instance/render.sh "$inst/instance.env" deploy/instance/mushd.toml.tmpl \
  "$inst/mushd.toml" "$bin/mush-gateway" "$bin/mush-server"

# --- the launch: the bring-up's three boots, as a dev box runs them. No seed
# line (the lane's author files the line), no LLM check (no provider).
log "the launch bring-up (three boots)"
python3 deploy/instance/launch-bringup.py "$inst/instance.env" --no-systemd --bin-dir "$bin" \
  --password-file "$work/staff-password" --no-seed --no-llm-check > "$work/bringup.log" 2>&1 ||
  { tail -40 "$work/bringup.log" >&2; exit 1; }
grep -q 'Registry operators: #[0-9]* ready\.' "$work/bringup.log" ||
  { echo "minds-lane: the bring-up names no Registry operator ready" >&2; tail -20 "$work/bringup.log" >&2; exit 1; }
grep -q '^launch_path = ' "$inst/mushd.toml"
log "the launch is Active after $(since)"

# --- the world, and its front door.
rm -f "$inst/ipc.sock"
"$bin/mush-server" --config "$inst/mushd.toml" > "$work/server.log" 2>&1 &
pids+=("$!")
for _ in $(seq 1 240); do [ -S "$inst/ipc.sock" ] && break; sleep 0.5; done
[ -S "$inst/ipc.sock" ] || { echo "minds-lane: the world never opened its IPC socket" >&2; tail -30 "$work/server.log" >&2; exit 1; }
"$bin/mush-gateway" --config "$inst/mushd.toml" > "$work/gateway.log" 2>&1 &
pids+=("$!")
for _ in $(seq 1 240); do (exec 3<> "/dev/tcp/127.0.0.1/$ssh_port") 2> /dev/null && break; sleep 0.5; done
(exec 3<> "/dev/tcp/127.0.0.1/$ssh_port") 2> /dev/null ||
  { echo "minds-lane: the front door never opened on 127.0.0.1:$ssh_port" >&2; tail -30 "$work/gateway.log" >&2; exit 1; }
log "the world is up on 127.0.0.1:$ssh_port after $(since)"

# --- two players: a login each, registered at the front door, each with its
# own SSH key. This is the player's own hand, not the bot's and not staff's.
register() {
  # The login's password is made here and kept nowhere: the bot connects by its
  # key, and the front door's echo of the line is taken out of the kept log.
  local account="$1" out password
  ssh-keygen -q -t ed25519 -N '' -C "$account@minds-lane" -f "$work/bots/$account.key"
  password="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"
  out="$({
    sleep 2
    printf 'create %s %s\r\n' "$account" "$password"
    sleep 4
    printf '@sshkey/add %s\r\n' "$(cat "$work/bots/$account.key.pub")"
    sleep 3
    printf 'QUIT\r\n'
    sleep 2
  } | timeout 60 ssh -tt -p "$ssh_port" -o BatchMode=yes -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR play@127.0.0.1 2>&1 || true)"
  out="${out//"$password"/[password]}"
  printf '%s\n' "$out" > "$work/bots/$account.register.log"
  grep -q 'Connected as ' <<< "$out" ||
    { echo "minds-lane: $account could not register at the front door" >&2; tail -20 "$work/bots/$account.register.log" >&2; exit 1; }
  grep -q 'SHA256:' <<< "$out" ||
    { echo "minds-lane: $account's key was not taken" >&2; tail -20 "$work/bots/$account.register.log" >&2; exit 1; }
}
register lathe
register tally
host_key="$(ssh-keyscan -p "$ssh_port" -t ed25519 127.0.0.1 2> /dev/null | ssh-keygen -lf - | awk '{print $2}')"
[ -n "$host_key" ] || { echo "minds-lane: the front door's host key could not be read" >&2; exit 1; }
log "two logins registered (lathe the author, tally the buyer); host key $host_key"

if [ "${UP_ONLY:-0}" = 1 ]; then
  log "UP_ONLY: the world stays up. ssh -i $work/bots/lathe.key -p $ssh_port lathe@127.0.0.1"
  log "staff: connect keeper $(cat "$work/staff-password") at ssh -p $ssh_port play@127.0.0.1"
  wait "${pids[0]}"
  exit 0
fi


# --- the SSH mind service, for a player who is not logged in (doc 44 unit
# MW.24's promise, held here against the workshop itself on the split pair): the
# buyer's login, with no session open, makes a draft with `mind push` and reads
# it back with `mind pull`. Until MW.19 the workshop refused every such request.
mind_exec() {
  local account="$1" verb="$2" request="$3"
  printf '%s' "$request" | timeout 60 ssh -i "$work/bots/$account.key" -o IdentitiesOnly=yes \
    -p "$ssh_port" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR "$account+$account@127.0.0.1" mind "$verb" 2>&1 || true
}
pushed="$(mind_exec tally push '{"op":"create","name":"offline","from":null}')"
pulled="$(mind_exec tally pull '{"op":"read","name":"offline","revision":null}')"
printf '%s\n%s\n' "$pushed" "$pulled" > "$work/bots/offline-exec.log"
if ! grep -q '"status":"edited"' <<< "$pushed" || ! grep -q '"status":"read"' <<< "$pulled"; then
  echo "minds-lane: an offline player's SSH mind push and pull were not served:" >&2
  cat "$work/bots/offline-exec.log" >&2
  exit 1
fi
log "an offline player's mind push and mind pull are served"

# --- the two bots. Each is the bot's own binary, headless, with a config that
# names its login's key and the front door's pinned host key, and an empty .env
# of its own so that no key of the caller's reaches it. No model, no provider.
charter="${CHARTER:-Sentry Line}"
site="${SITE:-kerwan}"
bot() {
  local account="$1" seconds="$2"
  shift 2
  mkdir -p "$work/bots/$account"
  : > "$work/bots/$account/.env"
  cat > "$work/bots/$account/bot.toml" << EOF
[ssh]
host = "127.0.0.1"
port = $ssh_port
username = "$account+$account"
identity_file = "$work/bots/$account.key"
host_key_sha256 = "$host_key"

[agent]
max_seconds = $seconds
settle_ms = 100
observation_ms = 1000

[memory]
persistent = false

[costs]
persistent = false
EOF
  (
    cd "$work/bots/$account"
    exec env -u OPENROUTER_API_KEY -u ANTHROPIC_API_KEY -u MUSH_BOT_PASSWORD \
      "$bin/mush-bot" --config bot.toml --headless \
      --transcript "$work/bots/$account.jsonl" --report "$work/bots/$account.json" "$@" \
      > "$work/bots/$account.out" 2>&1 < /dev/null
  ) &
}
# The author's bound is the rating's (about half an hour at this pulse; the
# lane itself waits two hours at most); the buyer waits for the author and then
# flies, so its bound is the author's and forty minutes.
bots_s="$(date +%s)"
bot lathe "${AUTHOR_SECONDS:-6000}" --minds author --charter "$charter"
author_pid=$!
bot tally "${BUYER_SECONDS:-8400}" --minds buyer --charter "$charter" --minds-site "$site"
buyer_pid=$!
log "the author (lathe) and the buyer (tally) are playing; charter '$charter', site $site"
author_status=0
buyer_status=0
wait "$author_pid" || author_status=$?
log "the author ended (exit $author_status) after $(( $(date +%s) - bots_s ))s"
if [ "$author_status" -ne 0 ]; then
  # The buyer would read an empty counter for two hours: stop it now.
  kill -TERM "$buyer_pid" 2> /dev/null || true
fi
wait "$buyer_pid" || buyer_status=$?
log "the buyer ended (exit $buyer_status) after $(( $(date +%s) - bots_s ))s"

# --- what the two reports say, and the lane's verdict.
status=0
python3 - "$work" "$author_status" "$buyer_status" "$bots_s" << 'PY' || status=$?
import json, os, sys

work, author_status, buyer_status, bots_s = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
failures, known = [], []


def report(account):
    path = os.path.join(work, "bots", account + ".json")
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as error:
        failures.append(f"{account}: no report ({error})")
        return {}


def say(line):
    print("MINDS_LANE " + line)


def at(mine, name):
    for stamp in mine.get("milestones", []):
        if stamp.get("name") == name:
            return stamp.get("at_unix_ms", 0) / 1000 - bots_s
    return None


def clock(seconds):
    return "not reached" if seconds is None else f"{seconds:.0f}s"


author, buyer = report("lathe"), report("tally")
for who, whole, status in (("author", author, author_status), ("buyer", buyer, buyer_status)):
    mine = whole.get("mine") or {}
    market = mine.get("market") or {}
    say(f"{who}: exit {status}, success {whole.get('success')}, {mine.get('lines_sent', 0)} lines typed")
    if status != 0 or not whole.get("success"):
        failures.append(f"{who}: {mine.get('failure') or whole.get('outcome') or 'did not finish'}")
    for key, value in market.items():
        if value not in (None, False, 0, ""):
            say(f"{who} {key}: {value}")

a, b = (author.get("mine") or {}), (buyer.get("mine") or {})
am, bm = (a.get("market") or {}), (b.get("market") or {})
asked, finished = am.get("rating_asked_ms"), am.get("rating_finished_ms")
if asked is not None and finished is not None:
    say(f"time rating (asked to finished): {(finished - asked) / 1000:.0f}s")
say(f"time author rated at {clock(at(a, 'mind_rated'))}, filed at {clock(at(a, 'mind_filed'))}, "
    f"listed at {clock(at(a, 'mind_listed'))} (from the bots' start)")
say(f"time buyer bought at {clock(at(b, 'licence_bought'))}, simulator flown at "
    f"{clock(at(b, 'mind_flown_sim'))}, on the site at {clock(at(b, 'arrived_site'))}, launched at "
    f"{clock(at(b, 'launched'))}, recovered at {clock(at(b, 'recovered'))}, home at "
    f"{clock(at(b, 'docked_home'))}")

# What the two halves must agree on: one listing, bought and flown.
if am.get("code") and bm.get("code") and am["code"] != bm["code"]:
    failures.append(f"the buyer bought listing {bm['code']}, not the author's {am['code']}")
licence = bm.get("licence")
if licence:
    for key in ("chosen", "sealed"):
        if f"filed:{licence}" not in (bm.get(key) or "") and am.get("charter", "").lower() not in (bm.get(key) or "").lower():
            failures.append(f"the simulator's {key} line does not name the licensed mind: {bm.get(key)!r}")

# A known defect is held at the truth: while it reproduces the lane says so and
# passes; the day it stops reproducing the lane fails until its marker is taken
# out here and in the cut note (docs/design/44-cut-notes.md, MW.19).
unasked = am.get("rating_unasked_lines", 0)
if author_status == 0:
    if unasked > 0:
        known.append(f"the Registry sent the author {unasked} lines it did not ask for during the rating "
                     "(`Queued.` and `Done.` for the Registry's own steps)")
    else:
        failures.append("KNOWN DEFECT GONE: the rating sent the author no unasked line; take the marker out "
                        "of scripts/bot-minds-lane.sh and the MW.19 cut note")
for line in known:
    say("KNOWN DEFECT (expected to fail): " + line)
for line in failures:
    say("FAILED: " + line)
sys.exit(1 if failures else 0)
PY
say_end="$(( $(date +%s) - started_s ))"
if [ "$status" -ne 0 ]; then
  for account in lathe tally; do
    echo "--- the last lines $account read ---" >&2
    tail -n 30 "$work/bots/$account.out" >&2 || true
  done
  exit 1
fi
log "PASSED in ${say_end}s"
