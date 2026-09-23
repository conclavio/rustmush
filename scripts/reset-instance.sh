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
#   3. empty the data dir, preserving ssh_host_ed25519 (the ledger archive, the quarantine
#      spool, the offer/resolution/decision key rings and the logs all live
#      under it), owned by the user the unit runs as;
#   4. optionally bootstrap the first staff login, then start the unit again.
#
# The rendered `mushd.toml` is NOT touched: the config is the thing you keep.
#
# Usage:
#   sudo scripts/reset-instance.sh <instance.env> --yes [--unit <name>] [--staff <account>]
#
# <instance.env> is the file the instance's mushd.toml was rendered from (see
# deploy/instance/). It is READ, never sourced — a value here is a sed
# replacement, not a shell word, and `INSTANCE_TITLE=rustmush dev3 (oWoD)` is a
# good line in it and a syntax error to `.`.
#
# This runs on the Linux VPS with systemctl, redis-cli, GNU coreutils and find;
# optional staff bootstrap also uses runuser and the current server binary.

set -euo pipefail
# A shell invoked with -x must not echo the bootstrap password.
set +x

usage() {
	echo "usage: $0 <instance.env> --yes [--unit <name>] [--staff <account>]" >&2
	exit 2
}

env_file=""
confirmed=0
unit=""
staff=""

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
	--staff)
		if [ "$#" -lt 2 ] || [ -n "$staff" ]; then
			usage
		fi
		staff="$2"
		[ -n "$staff" ] || usage
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
			# Trim what the render's `sed` would have carried into the config
			# harmlessly but a path cannot survive: leading spaces, and the CR
			# a Windows-edited instance.env leaves on every line. `rm -rf` on
			# "/opt/rustmush/dev3<CR>" deletes nothing and `install -d` then
			# makes a ghost tree beside the real one.
			value="${value#"${value%%[![:space:]]*}"}"
			value="${value%"${value##*[![:space:]]}"}"
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

# `DATA_DIR` is deleted whole, as root, from a value an operator typed into a
# text file. `require_value` only proves it is non-empty, and nothing further
# down would blink at `/`, `/opt`, or a relative typo. Refuse anything that is
# not plausibly one instance's own tree — here, with the other preflights,
# before a thing is taken down.
case "$data_dir" in
/*) ;;
*)
	echo "$0: DATA_DIR must be an absolute path, got '$data_dir'" >&2
	exit 1
	;;
esac
case "$data_dir" in
*..*)
	echo "$0: DATA_DIR must not contain '..', got '$data_dir'" >&2
	exit 1
	;;
esac
if [ -L "$data_dir" ]; then
	echo "$0: DATA_DIR must not be a symlink" >&2
	exit 1
fi
# Canonicalize dot and repeated-slash components before the destructive path
# check: '/opt/./' is still a top-level directory and must also be refused.
command -v realpath >/dev/null 2>&1 || { echo "$0: realpath is not on PATH" >&2; exit 1; }
data_dir="$(realpath -m -- "$data_dir")"
if [ "$(dirname "$data_dir")" = "/" ]; then
	echo "$0: DATA_DIR must be nested under a parent (got '$data_dir'); refusing to delete a top-level directory" >&2
	exit 1
fi
host_key="$data_dir/ssh_host_ed25519"
if [ -e "$host_key" ] || [ -L "$host_key" ]; then
	if [ ! -f "$host_key" ] || [ -L "$host_key" ]; then
		echo "$0: SSH host key must be a regular file: $host_key" >&2
		exit 1
	fi
fi

# Everything this needs, checked BEFORE anything is stopped. A missing
# `redis-cli` discovered at step 2 would leave the instance down with its store
# intact — the one outcome worse than not resetting at all.
for tool in systemctl redis-cli install find; do
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

# Resolve and preflight the optional CLI before stopping anything. Only the
# account name reaches argv; the password stays in shell memory and a pipe.
if [ -n "$staff" ]; then
	if ! [[ "$staff" =~ ^[A-Za-z0-9_-]{3,20}$ ]] ||
		[[ "${staff,,}" =~ ^(god|play|guest|new|root|admin)$ ]]; then
		echo "$0: invalid or reserved staff account name" >&2
		exit 1
	fi
	build_dir="$(require_value BUILD_DIR)"
	config="$(dirname "$env_file")/mushd.toml"
	bootstrap="$build_dir/target/release/mush-server"
	[ -x "$bootstrap" ] || bootstrap="$build_dir/target/release/mushd"
	if [ ! -x "$bootstrap" ] || [ ! -f "$config" ] || ! command -v runuser >/dev/null 2>&1; then
		echo "$0: --staff needs the current mush-server or mushd, installed mushd.toml and runuser" >&2
		exit 1
	fi
	runuser -u "$owner" -- "$bootstrap" --help | grep -q -- --bootstrap-staff || {
		echo "$0: rebuild the binary with --bootstrap-staff support before resetting" >&2
		exit 1
	}
	runuser -u "$owner" -- "$bootstrap" --config "$config" --check-config
	staff_password=""
	if [ -t 0 ]; then
		IFS= read -r -s -p "Password for $staff: " staff_password
		printf '\n' >&2
		IFS= read -r -s -p "Confirm password: " staff_password_confirm
		printf '\n' >&2
		[ "$staff_password" = "$staff_password_confirm" ] || {
			echo "$0: passwords did not match; instance unchanged" >&2
			exit 1
		}
		unset staff_password_confirm
	else
		IFS= read -r staff_password || [ -n "$staff_password" ]
	fi
	# Match the Rust validator's byte limit, including UTF-8 input.
	password_bytes="$(LC_ALL=C printf '%s' "$staff_password" | wc -c)"
	if [ "$password_bytes" -lt 4 ] || [ "$password_bytes" -gt 128 ]; then
		echo "$0: password must be 4-128 bytes" >&2
		exit 1
	fi
fi

echo "resetting instance '$instance': unit $unit, redis 127.0.0.1:$redis_port, data $data_dir"
echo "preserving SSH host identity: $host_key (if present)"

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

# 3. World-bound data goes; the SSH host identity is deliberately unchanged.
#    Keep that file in place, so a failed wipe cannot lose a staged backup.
#    find does not follow symlinks, and rm unlinks symlink entries themselves.
if [ -d "$data_dir" ]; then
	find "$data_dir" -mindepth 1 -maxdepth 1 ! -name ssh_host_ed25519 -exec rm -rf -- {} +
fi
install -d -o "$owner" -g "$group" -m 0750 -- "$data_dir"

# 4. Bootstrap while no gateway can register a player first. Failure leaves
#    the service stopped, so the operator can retry the CLI on the empty store.
if [ -n "$staff" ]; then
	printf '%s\n' "$staff_password" | runuser -u "$owner" -- "$bootstrap" --config "$config" --bootstrap-staff "$staff"
	unset staff_password
fi
systemctl start "$unit"

echo "instance '$instance' reset; configured boot packs are applied on startup."
echo "Check START_ROOM / START_ROOM_UNAPPROVED before letting anyone connect."
