#!/usr/bin/env bash
#
# reset-instance.sh — wipe one rustmush instance back to a fresh world.
#
# rustmush is pre-launch: there are no running games and no players, stores are
# disposable, and the deployment target is fresh VPS instances. That makes
# flush-and-rebuild the STANDARD remedy for a wedged store, a bad migration or a
# world someone tested into a corner — not a last resort. A standard remedy that
# lives in somebody's shell history is a remedy that gets half-performed at
# 2am (the unit left running while Redis is flushed, the data dir kept so the
# quarantine spool and the key rings outlive the world they belonged to), so it
# is written down here instead:
#
#   1. stop the unit, so nothing is writing while the store goes away;
#   2. FLUSHDB this instance's OWN Redis — the one on REDIS_PORT, never the
#      box's 6379 development instance;
#   3. delete and recreate the data dir (the ledger archive, the quarantine
#      spool, the offer/resolution/decision key rings and the logs all live
#      under it), owned by the user the unit runs as;
#   4. start the unit again.
#
# The rendered `mushd.toml` is NOT touched: the config is the thing you keep.
#
# Usage:
#   sudo scripts/reset-instance.sh <instance.env> --yes [--unit <name>]
#
# <instance.env> is the file the instance's mushd.toml was rendered from (see
# deploy/instance/). It is READ, never sourced — a value here is a sed
# replacement, not a shell word, and `INSTANCE_TITLE=rustmush dev3 (oWoD)` is a
# good line in it and a syntax error to `.`.
#
# This runs on the Linux VPS. It is deliberately small and dependency-free
# (systemctl, redis-cli, install, rm) so it can be read in one sitting.

set -euo pipefail

usage() {
	echo "usage: $0 <instance.env> --yes [--unit <name>]" >&2
	exit 2
}

env_file=""
confirmed=0
unit=""

while [ "$#" -gt 0 ]; do
	case "$1" in
	--yes)
		confirmed=1
		shift
		;;
	--unit)
		[ "$#" -ge 2 ] || usage
		unit="$2"
		shift 2
		;;
	-h | --help)
		usage
		;;
	-*)
		echo "$0: unknown option $1" >&2
		usage
		;;
	*)
		[ -z "$env_file" ] || usage
		env_file="$1"
		shift
		;;
	esac
done

[ -n "$env_file" ] || usage
[ -f "$env_file" ] || {
	echo "$0: no such instance.env: $env_file" >&2
	exit 1
}
[ "$confirmed" -eq 1 ] || {
	echo "$0: this DESTROYS the instance's world. Re-run with --yes." >&2
	exit 1
}
[ "$(id -u)" -eq 0 ] || {
	echo "$0: must run as root (it stops a unit and rewrites the data dir)." >&2
	exit 1
}

# Read one KEY's value out of the instance.env. The same `while IFS='=' read`
# loop the render recipe uses, for the same reason: the line is read, not
# evaluated.
env_value() {
	local wanted="$1"
	local key
	local value
	while IFS='=' read -r key value; do
		case "$key" in '' | '#'*) continue ;; esac
		if [ "$key" = "$wanted" ]; then
			printf '%s\n' "$value"
			return 0
		fi
	done <"$env_file"
	return 1
}

require_value() {
	local key="$1"
	local value
	if ! value="$(env_value "$key")" || [ -z "$value" ]; then
		echo "$0: $env_file has no $key line" >&2
		exit 1
	fi
	printf '%s\n' "$value"
}

instance="$(require_value INSTANCE)"
data_dir="$(require_value DATA_DIR)"
redis_port="$(require_value REDIS_PORT)"

# Everything this needs, checked BEFORE anything is stopped. A missing
# `redis-cli` discovered at step 2 would leave the instance down with its store
# intact — the one outcome worse than not resetting at all.
for tool in systemctl redis-cli install; do
	if ! command -v "$tool" >/dev/null 2>&1; then
		echo "$0: $tool is not on PATH" >&2
		exit 1
	fi
done

# The instance runs one of two ways (deploy/README.md): the all-in-one
# `<name>mush.service`, or the split, hot-restartable `<name>mush-gateway.service`
# which supervises the mush-server child. Take whichever systemd knows about,
# unless the operator named one.
if [ -z "$unit" ]; then
	for candidate in "${instance}mush-gateway.service" "${instance}mush.service"; do
		if systemctl cat "$candidate" >/dev/null 2>&1; then
			unit="$candidate"
			break
		fi
	done
fi
if [ -z "$unit" ]; then
	echo "$0: no unit found for instance '$instance' (tried ${instance}mush-gateway.service, ${instance}mush.service); pass --unit" >&2
	exit 1
fi

# The unit's own User= is who must own the recreated tree. Falling back to
# `mush` matches every shipped unit; getting this wrong is a box that comes up
# and cannot write its own logs.
owner="$(systemctl show -p User --value "$unit" 2>/dev/null || true)"
[ -n "$owner" ] || owner="mush"
group="$(systemctl show -p Group --value "$unit" 2>/dev/null || true)"
[ -n "$group" ] || group="$owner"

echo "resetting instance '$instance': unit $unit, redis 127.0.0.1:$redis_port, data $data_dir"

# Same rule as the tool check: prove the store answers before taking the game
# down, or a wrong REDIS_PORT ends with a stopped unit and an untouched world.
if ! redis-cli -h 127.0.0.1 -p "$redis_port" PING >/dev/null 2>&1; then
	echo "$0: no Redis answering on 127.0.0.1:$redis_port (REDIS_PORT in $env_file)" >&2
	exit 1
fi

# 1. Stop first. Flushing the store under a running world would have the world
#    write its RAM copy straight back.
systemctl stop "$unit"

# 2. This instance's own Redis, logical db 0 — the db `redis://127.0.0.1:PORT`
#    with no path selects, which is what the template renders.
redis-cli -h 127.0.0.1 -p "$redis_port" -n 0 FLUSHDB

# 3. The data dir, whole: the archive tier, the quarantine spool, the three key
#    rings and the logs. Keys are regenerated on the next boot; keeping them
#    beside a world that no longer exists is what makes a half-reset confusing.
rm -rf -- "$data_dir"
install -d -o "$owner" -g "$group" -m 0750 -- "$data_dir"

# 4. Back up, on an empty store: the boot seeds Limbo #0 and God #1 and nothing
#    else, so the instance's own seed (its start rooms) has to be re-applied.
systemctl start "$unit"

echo "instance '$instance' reset. The world is a fresh seed: Limbo #0 and God #1."
echo "Re-dig START_ROOM / START_ROOM_UNAPPROVED before letting anyone connect."
