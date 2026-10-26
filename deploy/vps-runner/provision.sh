#!/usr/bin/env bash
#
# deploy/vps-runner/provision.sh — the VPS as a self-hosted GitHub Actions
# runner for .github/workflows/gate.yml (test-strategy cut T.2, 2026-10-26).
# Run as root on the box, from this directory copied there (or the checkout):
#
#   RUNNER_TOKEN=<token> bash deploy/vps-runner/provision.sh
#
# where the token is a one-hour runner registration token for the repository:
#
#   gh api -X POST repos/conclavio/rustmush/actions/runners/registration-token --jq .token
#
# Every step is idempotent: run it again after a change to the files beside
# it, and it reinstalls only what changed (the runner, Rust and nextest are
# skipped once present; the Redis config, the unit and the override are
# reinstalled every time; registration happens once).
#
# What it makes:
#   * the user `ghrunner` (no sudo, no password, its own home): the runner,
#     its own rustup (toolchain pinned by the checkout's rust-toolchain.toml,
#     pre-installed here) and cargo-nextest at .config/nextest.toml's version
#     under /home/ghrunner/.cargo/bin, which the runner puts on every job's
#     PATH through its .path file (written after registration, which writes
#     that file itself);
#   * the runner itself at /home/ghrunner/actions-runner, registered to the
#     repository with the label `vps` and the name `conclav-vps`, as the
#     systemd service actions.runner.conclavio-rustmush.conclav-vps.service,
#     with runner-override.conf beside it (Nice=10, best-effort I/O at the
#     lowest priority, a memory ceiling, the hosted runners' file-descriptor
#     limit): the game servers and the production Redis on this box win
#     under contention;
#   * a private Redis for the jobs, redis-gate.service on 127.0.0.1:6799
#     (redis-gate.conf: the four modules, appendonly with appendfsync always
#     as CI's disposable Redis runs), wiped by every sweep; the instances on
#     6379, 6788, 6791 and 6798 are not touched.
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
REPO_URL=https://github.com/conclavio/rustmush
RUNNER_NAME=conclav-vps
RUNNER_LABELS=vps
RUNNER_USER=ghrunner
UNIT="actions.runner.conclavio-rustmush.${RUNNER_NAME}.service"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$(id -u)" != 0 ]; then
  echo "run as root" >&2
  exit 1
fi

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
# libicu and the rest of the runner's .NET dependencies (apt, as root). The
# script fails when ANY apt source on the box is stale (2026-10-26: a
# speedtest-cli repository answering 404 failed `apt-get update`), so it is
# not fatal here; what is fatal is the runner not starting, checked next.
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
  echo "registered $RUNNER_NAME"
fi
# The jobs' PATH: the runner's service exports .path at start. Written AFTER
# registration, because config.sh writes .path itself from its own PATH
# (2026-10-26: the first provisioning wrote it before and lost cargo's bin).
as_runner bash -c "echo '$job_path' > '$runner_dir/.path'"

# 6. The service, niced, and started.
cd "$runner_dir"
if [ ! -f "/etc/systemd/system/$UNIT" ]; then
  ./svc.sh install "$RUNNER_USER"
fi
mkdir -p "/etc/systemd/system/$UNIT.d"
install -m 0644 "$here/runner-override.conf" "/etc/systemd/system/$UNIT.d/override.conf"
systemctl daemon-reload
systemctl enable "$UNIT" > /dev/null 2>&1 || true
systemctl restart "$UNIT"
sleep 3
systemctl --no-pager --lines=5 status "$UNIT" || true
echo "provisioned: $UNIT, redis-gate on 6799, $RUNNER_USER's toolchain $(as_runner "$home/.cargo/bin/rustc" +"$TOOLCHAIN" --version)"
