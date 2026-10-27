#!/usr/bin/env bash
# Free the hosted runner's root disk of the preinstalled toolchains this
# workflow never uses: the body of ci.yml's test job's "Free the root disk"
# step until 2026-10-27, when it moved here to run detached in the
# background of the job's checkout, toolchain install and cache restore
# (the "Free the root disk (in the background)" and "Root disk freed"
# steps there). About 25 GB on ubuntu-latest, 0.8-3.0 min. Prints what
# each removal freed; a failed removal is a non-zero exit, which the
# waiting step reports as its own.
set -euo pipefail
df -h /
free_mb() { df -m / | awk 'NR==2 {print $4}'; }
before=$(free_mb)
drop() {
  if [ -d "$1" ]; then
    local was
    was=$(free_mb)
    sudo rm -rf "$1"
    echo "removed $1: $(( $(free_mb) - was )) MB freed"
  fi
}
# What easimon/maximize-build-space removed (dotnet, android,
# haskell, CodeQL, the docker images), then the other large
# preinstalls the image may carry.
drop /usr/share/dotnet
drop /usr/local/lib/android
drop /opt/ghc
drop /usr/local/.ghcup
drop /opt/hostedtoolcache/CodeQL
before_prune=$(free_mb)
docker image prune -af > /dev/null 2>&1 || true
echo "docker image prune: $(( $(free_mb) - before_prune )) MB freed"
drop /usr/local/share/boost
drop /usr/share/swift
for julia in /usr/local/julia*; do drop "$julia"; done
df -h /
echo "freed in total: $(( $(free_mb) - before )) MB"
