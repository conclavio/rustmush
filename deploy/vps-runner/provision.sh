#!/usr/bin/env bash
#
# deploy/vps-runner/provision.sh — a VPS as a self-hosted GitHub Actions
# runner for .github/workflows/gate.yml (test-strategy cut T.2, 2026-10-26:
# conclav.io; the second box, conclav.dev, 2026-10-27). Run as root on the
# box, from this directory copied there (or the checkout):
#
#   RUNNER_TOKEN=<token> bash deploy/vps-runner/provision.sh
#
# where the token is a one-hour runner registration token for the repository:
#
#   gh api -X POST repos/conclavio/rustmush/actions/runners/registration-token --jq .token
#
# The defaults are conclav.io's (name `conclav-vps`, label `vps`, a 22 GB
# memory ceiling). The knobs, as environment variables, are what made
# conclav.dev (32 cores, 62 GB, nothing else on the box):
#
#   RUNNER_TOKEN=<token> RUNNER_NAME=conclav-dev RUNNER_LABELS=vps,conclav-dev \
#     RUNNER_MEMORY_HIGH=56G WITH_MOLD=1 bash deploy/vps-runner/provision.sh
#
#   RUNNER_NAME         the runner's name on GitHub and in its unit's name
#   RUNNER_LABELS       comma-separated; `vps` is the SHARED label (gate.yml's
#                       default `runner` input: either box takes the job),
#                       and each box has a label of its own to pin a dispatch
#   RUNNER_MEMORY_HIGH  the unit's MemoryHigh (runner-override.conf's value
#                       is conclav.io's: it shares its box with the game
#                       servers; conclav.dev shares it with nothing)
#   WITH_MOLD=1         build and install mold 3.0.0 into ~ghrunner/.local
#                       entirely as ghrunner (pinned commit SHA; never runs an
#                       install script as root): optional, for linker
#                       experiments; jobs link with rust-lld by default
#   WITH_DISPATCH_TIMERS=1
#                       also run dispatch-timers-install.sh beside this file:
#                       the four timers that start the soak, the nightly, the
#                       weekly and CodeQL (no workflow has a `schedule`), for
#                       conclav.dev only: ONE box keeps the clock. The token
#                       they call GitHub with is the owner's to place
#                       afterwards; that script's header and docs/ops.md say how
#   WITH_BUILD_TOOLS=1  also run build-tools-install.sh beside this file: the
#                       pinned Node and the few tools a box needs to build a
#                       whole deploy by itself, web client included, with no
#                       other machine in the loop (both boxes; doc 52)
#
# Every step is idempotent: run it again after a change to the files beside
# it, and it reinstalls only what changed. Root never executes scripts from
# ghrunner-writable directories; every remote download is pinned by version
# and verified against its SHA-256 digest (or exact Git commit SHA).
#
# To retire it:
#   systemctl disable --now actions.runner.conclavio-rustmush.<RUNNER_NAME>.service
#   rm -rf /etc/systemd/system/actions.runner.conclavio-rustmush.<RUNNER_NAME>.service*
#   systemctl daemon-reload
#   runuser -u ghrunner -- /home/ghrunner/actions-runner/config.sh remove --token <removal-token>
#   systemctl disable --now redis-gate
#   userdel -r ghrunner
set -euo pipefail

RUNNER_VERSION=2.338.2
RUNNER_SHA256=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
REDIS_GPG_SHA256=817b5a78358d00ed6b71884d70ad5d2eab9934badca1a34299fdc6a2e4a8ad20
RUSTUP_VERSION=1.29.3
RUSTUP_INIT_SHA256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
NEXTEST_VERSION=0.9.148
NEXTEST_SHA256=223ba936714cc861a9cdedad5a8eb664dfdd69d6fbdf6cea6aa8184f523290dc
TOOLCHAIN=1.98.0
MOLD_TAG=v3.0.0
MOLD_COMMIT=8de38c35a2df16a25f7ff87ac3ad07156a925beb
REPO_URL=https://github.com/conclavio/rustmush
RUNNER_NAME="${RUNNER_NAME:-conclav-vps}"
RUNNER_LABELS="${RUNNER_LABELS:-vps}"
RUNNER_MEMORY_HIGH="${RUNNER_MEMORY_HIGH:-22G}"
WITH_MOLD="${WITH_MOLD:-0}"
WITH_DISPATCH_TIMERS="${WITH_DISPATCH_TIMERS:-0}"
WITH_BUILD_TOOLS="${WITH_BUILD_TOOLS:-0}"
RUNNER_USER=ghrunner

if ! [[ "$RUNNER_NAME" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "invalid RUNNER_NAME: '$RUNNER_NAME'" >&2
  exit 2
fi
if ! [[ "$RUNNER_MEMORY_HIGH" =~ ^[0-9]+[KMGTP]?$ ]]; then
  echo "invalid RUNNER_MEMORY_HIGH: '$RUNNER_MEMORY_HIGH'" >&2
  exit 2
fi

UNIT="actions.runner.conclavio-rustmush.${RUNNER_NAME}.service"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$(id -u)" != 0 ]; then
  echo "run as root" >&2
  exit 1
fi
export DEBIAN_FRONTEND=noninteractive

# Private root-owned directories (mode 0700) for logs and download verification:
# never write root output to predictable paths inside world-writable /tmp.
log_dir="$(mktemp -d /tmp/rustmush-provision-logs.XXXXXX)"
stage_dir="$(mktemp -d /tmp/rustmush-provision-stage.XXXXXX)"
chmod 0700 "$log_dir" "$stage_dir"
trap 'rm -rf "$stage_dir"' EXIT

# 0. The build dependencies, and Redis when the box has none. `apt-get
#    update` is not fatal: it fails when ANY apt source on the box is stale
#    (conclav.io, 2026-10-26: a speedtest-cli repository answering 404),
#    and that box already has everything; what is fatal is a tool missing
#    afterwards, checked below.
apt-get update > "$log_dir/apt-update.log" 2>&1 \
  || echo "apt-get update failed (a stale source? see $log_dir/apt-update.log): installing from the lists the box has"
apt-get install -y --no-install-recommends \
  build-essential pkg-config libssl-dev git curl jq ca-certificates gnupg \
  > "$log_dir/apt-install.log" 2>&1 \
  || echo "apt-get install failed (see $log_dir/apt-install.log): checking what the box has"
for tool in cc pkg-config git curl jq gpg sha256sum; do
  command -v "$tool" > /dev/null || { echo "$tool is missing and apt could not install it" >&2; exit 1; }
done
if ! command -v redis-server > /dev/null; then
  # Redis Open Source from packages.redis.io, the suite named after the
  # box's Ubuntu release. Verify the downloaded GPG key against its pinned
  # SHA-256 digest before installing it into /usr/share/keyrings.
  # shellcheck source=/dev/null
  . /etc/os-release
  curl --proto =https --tlsv1.2 -fsSL -o "$stage_dir/redis.gpg" https://packages.redis.io/gpg
  echo "${REDIS_GPG_SHA256}  ${stage_dir}/redis.gpg" | sha256sum -c -
  gpg --dearmor --yes -o /usr/share/keyrings/redis-archive-keyring.gpg "$stage_dir/redis.gpg"
  chmod 0644 /usr/share/keyrings/redis-archive-keyring.gpg
  echo "deb [signed-by=/usr/share/keyrings/redis-archive-keyring.gpg] https://packages.redis.io/deb ${VERSION_CODENAME} main" \
    > /etc/apt/sources.list.d/redis.list
  apt-get update > "$log_dir/apt-update-redis.log" 2>&1 \
    || echo "apt-get update failed after adding the Redis source (see $log_dir/apt-update-redis.log)"
  apt-get install -y redis
  echo "installed $(redis-server --version)"
fi
for m in rejson redisearch redisbloom redistimeseries; do
  [ -f "/usr/lib/redis/modules/$m.so" ] || { echo "/usr/lib/redis/modules/$m.so is missing: redis-gate.conf loads it" >&2; exit 1; }
done

# 1. The user.
if ! id -u "$RUNNER_USER" > /dev/null 2>&1; then
  useradd --create-home --shell /bin/bash "$RUNNER_USER"
  echo "created $RUNNER_USER"
fi
home="$(getent passwd "$RUNNER_USER" | cut -d: -f6)"
runner_dir="$home/actions-runner"
# As the runner's user, with ITS home: `runuser -u` keeps the caller's
# environment, and rustup, cargo and the runner all go by HOME.
as_runner() {
  runuser -u "$RUNNER_USER" -- env HOME="$home" USER="$RUNNER_USER" LOGNAME="$RUNNER_USER" "$@"
}
# By-hand runs as the user go through here too (/tmp is root's leftovers on
# conclav.io and a 32 GB tmpfs on conclav.dev); the jobs get the runner's own.
as_runner mkdir -p "$home/tmp"

# 2. The runner, downloaded and verified by root in `$stage_dir` before
#    extracting. `installdependencies.sh` is executed ONLY from the root-owned,
#    SHA-256-verified staging tree during fresh installation — never from
#    ghrunner-writable `$runner_dir` on re-runs.
if [ ! -x "$runner_dir/run.sh" ]; then
  tarball="$stage_dir/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz"
  curl --proto =https --tlsv1.2 -fsSL -o "$tarball" \
    "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz"
  echo "${RUNNER_SHA256}  ${tarball}" | sha256sum -c -
  mkdir -p "$stage_dir/runner"
  tar xzf "$tarball" -C "$stage_dir/runner"
  # libicu and the rest of the runner's .NET dependencies (apt, as root).
  "$stage_dir/runner/bin/installdependencies.sh" > "$log_dir/runner-installdependencies.log" 2>&1 \
    || echo "installdependencies.sh failed (apt; see $log_dir/runner-installdependencies.log): checking the runner runs anyway"
  as_runner mkdir -p "$runner_dir"
  # Stream the verified tarball on stdin so root never writes inside ghrunner's tree.
  as_runner tar xzf - -C "$runner_dir" < "$tarball"
  echo "installed the runner ${RUNNER_VERSION}"
fi
as_runner "$runner_dir/bin/Runner.Listener" --version

# 3. Rust and nextest for the runner's user: verified pinned rustup-init binary,
#    the pinned toolchain with clippy/rustfmt, and SHA-256-verified nextest.
if [ ! -x "$home/.cargo/bin/rustup" ]; then
  rustup_bin="$stage_dir/rustup-init"
  curl --proto =https --tlsv1.2 -fsSL -o "$rustup_bin" \
    "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init"
  echo "${RUSTUP_INIT_SHA256}  ${rustup_bin}" | sha256sum -c -
  # shellcheck disable=SC2016
  as_runner bash -c '
    set -euo pipefail
    tmp="$(mktemp "$1/rustup-init.XXXXXX")"
    trap "rm -f \"\$tmp\"" EXIT
    cat > "$tmp"
    chmod 0700 "$tmp"
    "$tmp" -y --default-toolchain none --profile minimal --no-modify-path
  ' _ "$home/tmp" < "$rustup_bin"
  echo "installed rustup ${RUSTUP_VERSION} for $RUNNER_USER"
fi
as_runner "$home/.cargo/bin/rustup" toolchain install "$TOOLCHAIN" --profile minimal --component clippy --component rustfmt
if [ ! -x "$home/.cargo/bin/cargo-nextest" ]; then
  nextest_tar="$stage_dir/cargo-nextest-${NEXTEST_VERSION}.tar.gz"
  curl --proto =https --tlsv1.2 -fsSL -o "$nextest_tar" \
    "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/cargo-nextest-${NEXTEST_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
  echo "${NEXTEST_SHA256}  ${nextest_tar}" | sha256sum -c -
  as_runner tar zxf - -C "$home/.cargo/bin" < "$nextest_tar"
  echo "installed cargo-nextest ${NEXTEST_VERSION}"
fi
job_path="$home/.cargo/bin:$home/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# 4. The jobs' private Redis (ephemeral wiped CI test store on loopback 6799).
install -m 0640 -o redis -g redis "$here/redis-gate.conf" /etc/redis/redis-gate.conf
install -m 0644 -o root -g root "$here/redis-gate.service" /etc/systemd/system/redis-gate.service
systemctl daemon-reload
systemctl enable --now redis-gate.service
redis-cli -p 6799 ping

# 5. Registration (once; needs the token).
if [ ! -f "$runner_dir/.runner" ]; then
  if [ -z "${RUNNER_TOKEN:-}" ]; then
    echo "RUNNER_TOKEN is unset: gh api -X POST repos/conclavio/rustmush/actions/runners/registration-token --jq .token" >&2
    exit 2
  fi
  as_runner env RUNNER_TOKEN="$RUNNER_TOKEN" PATH="$job_path" bash -c "cd \"\$1\" && ./config.sh --unattended --url \"\$2\" --token \"\$RUNNER_TOKEN\" --name \"\$3\" --labels \"\$4\" --work _work --replace" _ "$runner_dir" "$REPO_URL" "$RUNNER_NAME" "$RUNNER_LABELS"
  echo "registered $RUNNER_NAME with the labels $RUNNER_LABELS"
fi
# The jobs' PATH: the runner's service exports .path at start. Written AFTER
# registration, because config.sh writes .path itself from its own PATH.
# shellcheck disable=SC2016
as_runner bash -c 'printf "%s\n" "$1" > "$2/.path"' _ "$job_path" "$runner_dir"

# 6. The systemd service, written directly by root instead of executing
#    ghrunner-writable `$runner_dir/svc.sh`. `runsvc.sh` and `.service` inside
#    `$runner_dir` are written as `$RUNNER_USER`.
as_runner cp -f "$runner_dir/bin/runsvc.sh" "$runner_dir/runsvc.sh"
as_runner chmod 0755 "$runner_dir/runsvc.sh"
# shellcheck disable=SC2016
as_runner bash -c 'printf "%s\n" "$1" > "$2/.service"' _ "$UNIT" "$runner_dir"

if [ ! -f "/etc/systemd/system/$UNIT" ]; then
  cat > "/etc/systemd/system/$UNIT" << EOF
[Unit]
Description=GitHub Actions Runner (conclavio.rustmush.${RUNNER_NAME})
After=network-online.target

[Service]
ExecStart=${runner_dir}/runsvc.sh
User=${RUNNER_USER}
WorkingDirectory=${runner_dir}
KillMode=process
KillSignal=SIGTERM
TimeoutStopSec=5min

[Install]
WantedBy=multi-user.target
EOF
  chmod 0644 "/etc/systemd/system/$UNIT"
fi
mkdir -p "/etc/systemd/system/$UNIT.d"
sed "s/^MemoryHigh=.*/MemoryHigh=${RUNNER_MEMORY_HIGH}/" "$here/runner-override.conf" > "/etc/systemd/system/$UNIT.d/override.conf"
chmod 0644 "/etc/systemd/system/$UNIT.d/override.conf"
systemctl daemon-reload
systemctl enable "$UNIT" > /dev/null 2>&1 || true
systemctl restart "$UNIT"
sleep 3
systemctl --no-pager --lines=5 status "$UNIT" || true

# 7. mold, optional: cloned at pinned commit SHA, built and installed into
#    `$home/.local` ENTIRELY as `$RUNNER_USER` (never executing `install-mold.sh`
#    or installing ghrunner-compiled binaries into `/usr` as root).
if [ "$WITH_MOLD" = 1 ] && ! command -v mold > /dev/null && [ ! -x "$home/.local/bin/mold" ]; then
  mold_build="$(as_runner mktemp -d "$home/tmp/mold-build.XXXXXX")"
  as_runner git clone --branch "$MOLD_TAG" --depth 1 https://github.com/rui314/mold.git "$mold_build"
  actual_commit="$(as_runner git -C "$mold_build" rev-parse HEAD)"
  if [ "$actual_commit" != "$MOLD_COMMIT" ]; then
    as_runner rm -rf "$mold_build"
    echo "mold $MOLD_TAG resolved to $actual_commit, expected pinned commit $MOLD_COMMIT" >&2
    exit 1
  fi
  # shellcheck disable=SC2016
  as_runner env PATH="$job_path" TMPDIR="$home/tmp" bash -c 'cd "$1" && cargo +"$2" build --release && PREFIX="$3/.local" ./install-mold.sh' _ "$mold_build" "$TOOLCHAIN" "$home"
  as_runner rm -rf "$mold_build"
  echo "installed $(as_runner "$home/.local/bin/mold" --version)"
fi
# 8. The clock-run workflows' timers, optional (conclav.dev, and only one box:
#    two would start every job twice): root-owned files from beside this
#    script, nothing from a ghrunner-writable directory.
if [ "$WITH_DISPATCH_TIMERS" = 1 ]; then
  bash "$here/dispatch-timers-install.sh"
fi
# 9. The box builds its own deploys, optional: Node at a pinned version,
#    verified by SHA-256, from a root-owned file beside this script.
if [ "$WITH_BUILD_TOOLS" = 1 ]; then
  bash "$here/build-tools-install.sh"
fi
echo "provisioned: $UNIT, redis-gate on 6799, $RUNNER_USER's toolchain $(as_runner "$home/.cargo/bin/rustc" +"$TOOLCHAIN" --version) (logs in $log_dir)"
