#!/usr/bin/env bash
# binary-changed.sh — is the binary a deploy just built different from the one
# a running unit was started with? (doc 33 §4.7 and Q-33-6, cut GG.4)
#
#   binary-changed.sh <candidate> <running>
#
# Prints `changed` or `unchanged` and exits 0; exits 2 on a usage error.
# <running> is what the unit is executing — on the box `/proc/<MainPID>/exe`,
# which still reads the old file after the build replaced it on disk. A
# <running> that cannot be read (no such process, the unit is down) is
# `changed`: there is nothing to keep.
#
# The comparison is by SHA-256 of the contents, never by mtime or path: a
# rebuild of the same commit that produced the same bytes is `unchanged` (no
# swap, no restart), and a rollback to older bytes is `changed`. deploy.sh
# asks this for the server binary (restart the server unit, a hot restart)
# and for the gateway binary (swap generations) and does nothing for the
# half that did not move.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: binary-changed.sh <candidate> <running>" >&2
    exit 2
fi
candidate=$1
running=$2

if [[ ! -r "$candidate" ]]; then
    echo "binary-changed.sh: $candidate is not readable" >&2
    exit 2
fi
if [[ ! -r "$running" ]]; then
    echo changed
    exit 0
fi

digest() { sha256sum "$1" | cut -d' ' -f1; }
if [[ "$(digest "$candidate")" == "$(digest "$running")" ]]; then
    echo unchanged
else
    echo changed
fi
