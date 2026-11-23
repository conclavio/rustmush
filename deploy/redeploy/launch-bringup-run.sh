#!/usr/bin/env bash
# launch-bringup-run.sh — bring the named launch up on ONE instance after
# wipe-cutover.sh (deploy/README.md, "Wipe and redeploy" and "Named launch and
# the player market"); as run on dev1 and dev2 on 2026-11-22. The staff login
# is $STAFF (keeper), its password read from the root-only file wipe-cutover.sh
# made, $REDEPLOY_DIR/<instance>-<staff>.pw. The bring-up lifts the world's hold
# when the Active boot has passed and starts it; a run that stops leaves it held,
# and running this again takes it up where it stopped.
#
#   [STAFF=keeper] bash launch-bringup-run.sh <instance> [launch-bringup.py options]
#
# The three private boots, the unit's start and the LLM check take a few
# minutes; the seed line's rating and a House's purchase after them half an
# hour or more (`--no-seed` leaves them out, `--seed-only` takes them up again).
# Run it detached and watch for its last line, `BRINGUP EXIT <status>`.
set -uo pipefail

N=${1:?usage: [STAFF=keeper] bash launch-bringup-run.sh <instance> [launch-bringup.py options]}
shift
STAFF=${STAFF:-keeper}
DIR=${REDEPLOY_DIR:-/root}
if ! [[ "$N" =~ ^[a-z][a-z0-9]*$ ]]; then
  echo "launch-bringup-run: not an instance name: $N" >&2
  exit 2
fi
ROOT=/opt/rustmush/$N
PW=$DIR/$N-$STAFF.pw
if [ -L "$PW" ] || [ ! -s "$PW" ]; then
  echo "launch-bringup-run: no password file $PW (wipe-cutover.sh makes it)" >&2
  exit 1
fi
echo "== $(date -u +%FT%TZ) $N: launch-bringup.py --staff $STAFF $*"
python3 "$ROOT/build/deploy/instance/launch-bringup.py" "$ROOT/instance.env" \
  --staff "$STAFF" --password-file "$PW" "$@"
rc=$?
echo "== $(date -u +%FT%TZ) $N: BRINGUP EXIT $rc"
exit "$rc"
