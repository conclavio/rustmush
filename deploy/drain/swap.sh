#!/usr/bin/env bash
# swap.sh — move a two-generation daemon to its other generation without
# dropping a session (doc 29 §9.4; doc 33 §4.5 and §4.7, cut GG.3).
#
# Shared by every daemon built on mush-drain. Each daemon's own swap.sh
# (deploy/sshdoorway/swap.sh) is a thin call into this one that names its
# units, its marker directory and its checks.
#
#   swap.sh --unit <stem> --marker-dir <dir> [options]
#       start the idle generation, confirm it stays up, then drain the old one
#       (SIGUSR1: warnings on its schedule, closes at its deadline or when
#       empty)
#   swap.sh ... --force
#       as above, but if the idle generation is still draining from the last
#       swap, finish it now (SIGTERM: one warning, then close) instead of
#       refusing
#
# Options:
#   --unit <stem>           the template unit: <stem>@a and <stem>@b (required)
#   --marker-dir <dir>      the unit's StateDirectory, where a draining
#                           generation leaves draining-<gen> (required)
#   --check <command>       run (with bash -c) before anything starts; a
#                           non-zero exit aborts the swap (a config check)
#   --symlink <link>        a path to re-point at the new generation once it
#                           has stayed up: a fresh temporary link is made and
#                           renamed over <link>, so nothing ever sees a
#                           missing link and a successor that died is never
#                           pointed at (doc 33 §4.4, the trusted socket)
#   --symlink-target <path> what <link> points at; %g stands for the generation
#   --after <command>       run (with bash -c) once the swap is done; a failure
#                           is a warning, not an error (a logging check)
#   --lock <path>           the flock (default /run/lock/<stem>-swap.lock)
#   --drain-note <text>     said when the old generation is told to drain
#   --force                 see above
#
# At most two generations ever run. A drain does not change the unit's state
# (it stays "active"), so "both active" means one is still draining. Only one
# swap of a unit runs at a time (flock).
set -euo pipefail

unit= marker_dir= check= link= link_target= after= lock= force=0
drain_note="warnings on its schedule; closes at its deadline or when empty"

usage() {
    echo "usage: swap.sh --unit <stem> --marker-dir <dir> [--check <cmd>] [--symlink <link> --symlink-target <path>] [--after <cmd>] [--lock <path>] [--drain-note <text>] [--force]" >&2
    exit 2
}
need() { [[ $# -ge 2 ]] || { echo "$1 needs a value" >&2; usage; }; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --unit) need "$@"; unit=$2; shift 2 ;;
        --marker-dir) need "$@"; marker_dir=$2; shift 2 ;;
        --check) need "$@"; check=$2; shift 2 ;;
        --symlink) need "$@"; link=$2; shift 2 ;;
        --symlink-target) need "$@"; link_target=$2; shift 2 ;;
        --after) need "$@"; after=$2; shift 2 ;;
        --lock) need "$@"; lock=$2; shift 2 ;;
        --drain-note) need "$@"; drain_note=$2; shift 2 ;;
        --force) force=1; shift ;;
        *) usage ;;
    esac
done
[[ -n "$unit" && -n "$marker_dir" ]] || usage
if [[ -n "$link" || -n "$link_target" ]]; then
    [[ -n "$link" && -n "$link_target" ]] || { echo "--symlink and --symlink-target go together" >&2; usage; }
    [[ "$link_target" == *%g* ]] || { echo "--symlink-target must contain %g" >&2; usage; }
fi
: "${lock:=/run/lock/$unit-swap.lock}"

exec 9>"$lock"
flock -n 9 || { echo "another swap of $unit is running" >&2; exit 1; }

state() { systemctl show -p ActiveState --value "$unit@$1"; }
running() { case "$(state "$1")" in active|activating|deactivating|reloading) return 0;; *) return 1;; esac; }
# The daemon reads $STATE_DIRECTORY. A generation that finds its marker at
# start does not serve, so only this script (which clears it) ever brings a
# drained generation back.
marker() { echo "$marker_dir/draining-$1"; }
draining() { [[ -e "$(marker "$1")" ]]; }
# A generation that must not come back on its own: Restart=on-failure would
# otherwise start it again in a moment, beside the old one, with no marker to
# stop it.
put_down() {
    systemctl stop "$unit@$1" || true
    systemctl reset-failed "$unit@$1" || true
    systemctl disable --quiet "$unit@$1" || true
}

if running a && running b; then
    if (( force )); then
        echo "both generations are running; finishing the draining one now"
    else
        echo "both generations are running: one is still draining." >&2
        echo "wait for it, or re-run with --force to finish it now." >&2
        exit 1
    fi
fi

# The serving generation is the running one without a drain marker.
if running a && ! draining a; then old=a; new=b
elif running b && ! draining b; then old=b; new=a
elif running a; then old=; new=b
elif running b; then old=; new=a
else old=; new=a
fi

if running "$new"; then
    # --force with both running: "new" is the one still draining.
    systemctl kill --signal=TERM "$unit@$new"
    echo "waiting for $unit@$new to finish (up to 2 min)..."
    for _ in $(seq 120); do running "$new" || break; sleep 1; done
    if running "$new"; then
        echo "$unit@$new did not finish; stop it by hand" >&2
        exit 1
    fi
fi

if [[ -n "$check" ]]; then
    bash -c "$check"
fi

rm -f "$(marker "$new")"
echo "starting $unit@$new"
# Type=notify: returns once the new generation is bound and READY.
systemctl start "$unit@$new"
# A generation that crashes right after READY must not take the old one down
# with it: confirm it is still serving before draining its predecessor.
sleep 5
if [[ "$(state "$new")" != active ]]; then
    echo "$unit@$new did not stay up; the old generation keeps serving" >&2
    put_down "$new"
    exit 1
fi

if [[ -n "$link" ]]; then
    target="${link_target//%g/$new}"
    if [[ ! -e "$target" ]]; then
        echo "$target does not exist, so $unit@$new is not serving it; the old generation keeps serving" >&2
        put_down "$new"
        exit 1
    fi
    tmp="$link.swap.$$"
    rm -f "$tmp"
    ln -s "$target" "$tmp"
    # rename(2) over the old link: no moment without a link.
    mv -T "$tmp" "$link"
    echo "$link -> $target"
fi

# Exactly one generation is enabled, so a reboot brings back the serving one
# (the drained one would refuse anyway, by its marker).
systemctl enable --quiet "$unit@$new"
if [[ -n "$old" ]]; then
    systemctl disable --quiet "$unit@$old" || true
    echo "draining $unit@$old ($drain_note)"
    systemctl kill --signal=USR1 "$unit@$old"
fi

if [[ -n "$after" ]]; then
    bash -c "$after" || echo "WARNING: the post-swap check failed ($after); see above" >&2
fi
echo "swap done: serving on $unit@$new"
