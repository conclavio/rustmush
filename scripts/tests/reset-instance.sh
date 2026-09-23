#!/usr/bin/env bash
# Hermetic reset acceptance: only the temporary tree is real. No systemd,
# Redis, account switching or service binary is ever contacted.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
fixture="$(mktemp -d)"
cleanup() {
  status=$?
  if [ "$status" -ne 0 ] && [ -f "$fixture/output" ]; then cat "$fixture/output" >&2; fi
  rm -rf -- "$fixture"
  exit "$status"
}
trap cleanup EXIT
mkdir -p "$fixture/bin" "$fixture/build/target/release"
export RESET_TEST_LOG="$fixture/operations"
export RESET_TEST_OWNER="$(id -u)" RESET_TEST_GROUP="$(id -g)"
export RESET_TEST_REAL_ID="$(command -v id)"
cat > "$fixture/bin/id" <<'SH'
#!/usr/bin/env bash
if [ "${1:-}" = -u ]; then echo 0; else exec "$RESET_TEST_REAL_ID" "$@"; fi
SH
cat > "$fixture/bin/systemctl" <<'SH'
#!/usr/bin/env bash
case "$1" in
cat) exit 0 ;;
show)
  if [ "$3" = User ]; then echo "$RESET_TEST_OWNER"; else echo "$RESET_TEST_GROUP"; fi ;;
stop|start) printf '%s\n' "$1" >> "$RESET_TEST_LOG" ;;
*) echo 'unexpected mock systemctl command' >&2; exit 1 ;;
esac
SH
cat > "$fixture/bin/redis-cli" <<'SH'
#!/usr/bin/env bash
case "${*: -1}" in
PING) echo PONG ;;
FLUSHDB) echo flush >> "$RESET_TEST_LOG" ;;
*) exit 1 ;;
esac
SH
cat > "$fixture/bin/runuser" <<'SH'
#!/usr/bin/env bash
[ "$1" = -u ] && [ "$3" = -- ] || exit 1
shift 3
exec "$@"
SH
cat > "$fixture/bin/install" <<'SH'
#!/usr/bin/env bash
[ "$1" = -d ] && [ "$2" = -o ] && [ "$3" = "$RESET_TEST_OWNER" ] || exit 1
[ "$4" = -g ] && [ "$5" = "$RESET_TEST_GROUP" ] && [ "$6" = -m ] && [ "$7" = 0750 ] && [ "$8" = -- ] || exit 1
mkdir -p -- "$9"
SH
cat > "$fixture/build/target/release/mush-server" <<'SH'
#!/usr/bin/env bash
case "${*: -1}" in
--help) echo --bootstrap-staff ;;
--check-config) exit 0 ;;
Keeper)
  [ "$#" -eq 4 ] && [ "$3" = --bootstrap-staff ] || exit 1
  IFS= read -r password
  [ "$password" = 'reset-test secret' ] || exit 1
  printf 'bootstrap\n' >> "$RESET_TEST_LOG"
  [ "${RESET_TEST_FAIL:-0}" != 1 ] ;;
*) exit 1 ;;
esac
SH
chmod +x "$fixture/bin/"* "$fixture/build/target/release/mush-server"
export PATH="$fixture/bin:$PATH"
cat > "$fixture/instance.env" <<EOF
INSTANCE=reset-test
DATA_DIR=$fixture/data
REDIS_PORT=65001
BUILD_DIR=$fixture/build
EOF
printf '[server]\n' > "$fixture/mushd.toml"
printf 'must survive symlink cleanup\n' > "$fixture/outside"
printf 'stable SSH host identity\n' > "$fixture/expected-key"
prepare() {
  mkdir -p "$fixture/data/archive" "$fixture/data/quarantine"
  cp "$fixture/expected-key" "$fixture/data/ssh_host_ed25519"
  chmod 600 "$fixture/data/ssh_host_ed25519"
  expected_mode="$(stat -c %a "$fixture/data/ssh_host_ed25519")"
  printf old > "$fixture/data/archive/old-world"
  printf old > "$fixture/data/quarantine/old-world"
  ln -s "$fixture/outside" "$fixture/data/outside-link"
  : > "$RESET_TEST_LOG"
}
assert_clean() {
  cmp "$fixture/expected-key" "$fixture/data/ssh_host_ed25519"
  [ "$(stat -c %a "$fixture/data/ssh_host_ed25519")" = "$expected_mode" ]
  [ -f "$fixture/outside" ]
  [ "$(find "$fixture/data" -mindepth 1 -maxdepth 1 | wc -l)" -eq 1 ]
}
prepare
printf 'reset-test secret\n' | bash "$root/scripts/reset-instance.sh" "$fixture/instance.env" --yes --staff Keeper > "$fixture/output" 2>&1
assert_clean
printf 'stop\nflush\nbootstrap\nstart\n' > "$fixture/expected-order"
cmp "$fixture/expected-order" "$RESET_TEST_LOG"
if grep -q 'reset-test secret' "$fixture/output"; then
  echo 'reset leaked a password' >&2; exit 1
fi

# A failed bootstrap leaves the service stopped, preserving the SSH identity.
prepare
export RESET_TEST_FAIL=1
if printf 'reset-test secret\n' | bash "$root/scripts/reset-instance.sh" "$fixture/instance.env" --yes --staff Keeper > "$fixture/output" 2>&1; then
  echo 'reset accepted a failed bootstrap' >&2; exit 1
fi
assert_clean
printf 'stop\nflush\nbootstrap\n' > "$fixture/expected-order"
cmp "$fixture/expected-order" "$RESET_TEST_LOG"
unset RESET_TEST_FAIL

# An operator can also reset without creating a staff account.
prepare
bash "$root/scripts/reset-instance.sh" "$fixture/instance.env" --yes > "$fixture/output" 2>&1
assert_clean
printf 'stop\nflush\nstart\n' > "$fixture/expected-order"
cmp "$fixture/expected-order" "$RESET_TEST_LOG"
echo 'reset fixture: host identity, secret stdin, cleanup and restart ordering passed'
