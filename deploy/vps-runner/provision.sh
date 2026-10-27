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
#                       (conclav.io's `conclav-io` was added through the API:
#                       `gh api -X POST repos/conclavio/rustmush/actions/runners/<id>/labels -f 'labels[]=conclav-io'`)
#   RUNNER_MEMORY_HIGH  the unit's MemoryHigh (runner-override.conf's value
#                       is conclav.io's: it shares its box with the game
#                       servers; conclav.dev shares it with nothing)
#   WITH_MOLD=1         build and install mold as the owner did on conclav.io
#                       (step 7 below): optional, for linker experiments; the
#                       jobs link with rust-lld unless a dispatch says otherwise
#
# Every step is idempotent: run it again after a change to the files beside
# it, and it reinstalls only what changed (the packages, the runner, Rust,
# nextest and mold are skipped once present; the Redis config, the unit and
# the override are reinstalled every time; registration happens once).
#
# What it makes:
#   * the build dependencies, from apt: what cargo pulls on for this
#     workspace is a C toolchain (ring, the one native build), pkg-config,
#     libssl-dev (not read by the lock as it stands, kept for the day it is),
#     git, curl and jq; and, when the box has no Redis, Redis Open Source
#     from packages.redis.io for the box's own Ubuntu release (the `redis`
#     metapackage: server, tools and the four modules under
#     /usr/lib/redis/modules), bound to loopback by its package defaults, as
#     the owner installed it on conclav.io on 2026-08-27;
#   * the user `ghrunner` (no sudo, no password, its own home): the runner,
#     its own rustup (toolchain pinned by the checkout's rust-toolchain.toml,
#     pre-installed here) and cargo-nextest at .config/nextest.toml's version
#     under /home/ghrunner/.cargo/bin, which the runner puts on every job's
#     PATH through its .path file (written after registration, which writes
#     that file itself);
#   * the runner itself at /home/ghrunner/actions-runner, registered to the
#     repository with RUNNER_LABELS and RUNNER_NAME, as the systemd service
#     actions.runner.conclavio-rustmush.<RUNNER_NAME>.service, with
#     runner-override.conf beside it (Nice=10, best-effort I/O at the lowest
#     priority, the memory ceiling, the hosted runners' file-descriptor
#     limit): on conclav.io the game servers and the production Redis win
#     under contention; on conclav.dev the nice costs nothing;
#   * a private Redis for the jobs, redis-gate.service on 127.0.0.1:6799
#     (redis-gate.conf: the four modules, appendonly with appendfsync always
#     as CI's disposable Redis runs), wiped by every sweep; on conclav.io the
#     instances on 6379, 6788, 6791 and 6798 are not touched;
#   * with WITH_MOLD=1, mold 3.0.0 under /usr (the owner's recipe on
#     conclav.io: `git clone https://github.com/rui314/mold.git && cd mold &&
#     cargo build --release && PREFIX=/usr ./install-mold.sh`, at the v3.0.0
#     tag, built by the runner's user with the pinned toolchain and installed
#     by root).
#
# To retire it: `cd /home/ghrunner/actions-runner && ./svc.sh stop &&
# ./svc.sh uninstall && runuser -u ghrunner -- ./config.sh remove --token
# <a removal token>`, then `systemctl disable --now redis-gate` and
# `userdel -r ghrunner`.
set -euo pipefail

RUNNER_VERSION=2.338.2
RUNNER_SHA256=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
NEXTEST_VERSION=0.9.148
TOOLCHAIN=1.98.0
MOLD_TAG=v3.0.0
REPO_URL=https://github.com/conclavio/rustmush
RUNNER_NAME="${RUNNER_NAME:-conclav-vps}"
RUNNER_LABELS="${RUNNER_LABELS:-vps}"
RUNNER_MEMORY_HIGH="${RUNNER_MEMORY_HIGH:-22G}"
WITH_MOLD="${WITH_MOLD:-0}"
RUNNER_USER=ghrunner
UNIT="actions.runner.conclavio-rustmush.${RUNNER_NAME}.service"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$(id -u)" != 0 ]; then
  echo "run as root" >&2
  exit 1
fi
export DEBIAN_FRONTEND=noninteractive

# 0. The build dependencies, and Redis when the box has none. `apt-get
#    update` is not fatal: it fails when ANY apt source on the box is stale
#    (conclav.io, 2026-10-26: a speedtest-cli repository answering 404),
#    and that box already has everything; what is fatal is a tool missing
#    afterwards, checked below.
apt-get update > /tmp/provision-apt-update.log 2>&1 \
  || echo "apt-get update failed (a stale source? see /tmp/provision-apt-update.log): installing from the lists the box has"
apt-get install -y --no-install-recommends \
  build-essential pkg-config libssl-dev git curl jq ca-certificates gnupg \
  > /tmp/provision-apt-install.log 2>&1 \
  || echo "apt-get install failed (see /tmp/provision-apt-install.log): checking what the box has"
for tool in cc pkg-config git curl jq gpg; do
  command -v "$tool" > /dev/null || { echo "$tool is missing and apt could not install it" >&2; exit 1; }
done
if ! command -v redis-server > /dev/null; then
  # Redis Open Source from packages.redis.io, the suite named after the
  # box's Ubuntu release (noble on conclav.io, resolute on conclav.dev; the
  # repository has both). The Ubuntu archive's own redis has no modules.
  . /etc/os-release
  curl -fsSL https://packages.redis.io/gpg | gpg --dearmor --yes -o /usr/share/keyrings/redis-archive-keyring.gpg
  echo "deb [signed-by=/usr/share/keyrings/redis-archive-keyring.gpg] https://packages.redis.io/deb ${VERSION_CODENAME} main" \
    > /etc/apt/sources.list.d/redis.list
  apt-get update > /tmp/provision-apt-update-redis.log 2>&1 \
    || echo "apt-get update failed after adding the Redis source (see /tmp/provision-apt-update-redis.log)"
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

# 2. The runner, checked against the release's published digest.
if [ ! -x "$runner_dir/run.sh" ]; then
  as_runner mkdir -p "$runner_dir"
  tarball="$runner_dir/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz"
  as_runner curl -fsSL -o "$tarball" \
    "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz"
  echo "${RUNNER_SHA256}  ${tarball}" | sha256sum -c -
  as_runner tar xzf "$tarball" -C "$runner_dir"
  rm -f "$tarball"
  echo "installed the runner ${RUNNER_VERSION}"
fi
# libicu and the rest of the runner's .NET dependencies (apt, as root). Not
# fatal, for the same stale-source reason as above; what is fatal is the
# runner not starting, checked next.
"$runner_dir/bin/installdependencies.sh" > /tmp/runner-installdependencies.log 2>&1 \
  || echo "installdependencies.sh failed (apt; see /tmp/runner-installdependencies.log): checking the runner runs anyway"
as_runner "$runner_dir/bin/Runner.Listener" --version

# 3. Rust and nextest for the runner's user: its own rustup, the pinned
#    toolchain with the two components rust-toolchain.toml lists, nextest's
#    binary into cargo's bin, and that bin on every job's PATH (the runner
#    reads .path at service start).
if [ ! -x "$home/.cargo/bin/rustup" ]; then
  as_runner bash -c 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal --no-modify-path'
  echo "installed rustup for $RUNNER_USER"
fi
as_runner "$home/.cargo/bin/rustup" toolchain install "$TOOLCHAIN" --profile minimal --component clippy --component rustfmt
if [ ! -x "$home/.cargo/bin/cargo-nextest" ]; then
  as_runner bash -c "curl -LsSf https://get.nexte.st/${NEXTEST_VERSION}/linux | tar zxf - -C '$home/.cargo/bin'"
  echo "installed cargo-nextest ${NEXTEST_VERSION}"
fi
job_path="$home/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# 4. The jobs' private Redis.
install -m 0644 "$here/redis-gate.conf" /etc/redis/redis-gate.conf
install -m 0644 "$here/redis-gate.service" /etc/systemd/system/redis-gate.service
systemctl daemon-reload
systemctl enable --now redis-gate.service
redis-cli -p 6799 ping

# 5. Registration (once; needs the token).
if [ ! -f "$runner_dir/.runner" ]; then
  if [ -z "${RUNNER_TOKEN:-}" ]; then
    echo "RUNNER_TOKEN is unset: gh api -X POST repos/conclavio/rustmush/actions/runners/registration-token --jq .token" >&2
    exit 2
  fi
  as_runner env RUNNER_TOKEN="$RUNNER_TOKEN" PATH="$job_path" bash -c "cd '$runner_dir' && ./config.sh --unattended --url '$REPO_URL' --token \"\$RUNNER_TOKEN\" --name '$RUNNER_NAME' --labels '$RUNNER_LABELS' --work _work --replace"
  echo "registered $RUNNER_NAME with the labels $RUNNER_LABELS"
fi
# The jobs' PATH: the runner's service exports .path at start. Written AFTER
# registration, because config.sh writes .path itself from its own PATH
# (2026-10-26: the first provisioning wrote it before and lost cargo's bin).
as_runner bash -c "echo '$job_path' > '$runner_dir/.path'"

# 6. The service, niced, and started. The override is conclav.io's file with
#    this box's memory ceiling in place of the 22G.
cd "$runner_dir"
if [ ! -f "/etc/systemd/system/$UNIT" ]; then
  ./svc.sh install "$RUNNER_USER"
fi
mkdir -p "/etc/systemd/system/$UNIT.d"
sed "s/^MemoryHigh=.*/MemoryHigh=${RUNNER_MEMORY_HIGH}/" "$here/runner-override.conf" > "/etc/systemd/system/$UNIT.d/override.conf"
chmod 0644 "/etc/systemd/system/$UNIT.d/override.conf"
systemctl daemon-reload
systemctl enable "$UNIT" > /dev/null 2>&1 || true
systemctl restart "$UNIT"
sleep 3
systemctl --no-pager --lines=5 status "$UNIT" || true

# 7. mold, optional: the owner's recipe, pinned to the release tag, built by
#    the runner's user (root has no cargo here) and installed under /usr by
#    root. A dispatch opts in per job; nothing links with it by default.
if [ "$WITH_MOLD" = 1 ] && ! command -v mold > /dev/null; then
  src="$home/mold"
  if [ ! -d "$src/.git" ]; then
    as_runner git clone --branch "$MOLD_TAG" --depth 1 https://github.com/rui314/mold.git "$src"
  fi
  as_runner env PATH="$job_path" TMPDIR="$home/tmp" bash -c "cd '$src' && cargo +$TOOLCHAIN build --release"
  (cd "$src" && PREFIX=/usr ./install-mold.sh)
  echo "installed $(mold --version)"
fi
echo "provisioned: $UNIT, redis-gate on 6799, $RUNNER_USER's toolchain $(as_runner "$home/.cargo/bin/rustc" +"$TOOLCHAIN" --version)"
