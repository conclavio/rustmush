#!/usr/bin/env bash
#
# deploy/vps-runner/workflow-dispatch.sh — start one of the repository's
# clock-run workflows with a `workflow_dispatch` at `main`. conclav.dev's
# timers (rustmush-dispatch-*.timer beside this file) call it; no workflow
# has a `schedule` trigger of its own.
#
# Why the box keeps the clock and not GitHub: GitHub starts this repository's
# scheduled runs seven to ten and a half hours late (2026-11: the 04:17 UTC
# nightly started between 11:28 and 14:38 UTC every day for two weeks, and
# the soak's first cron never fired on its night). A dispatch starts within
# seconds of the call.
#
#   workflow-dispatch.sh JOB
#
#   JOB          workflow     inputs            what it is
#   soak         soak.yml     (its defaults)    the nightly soak on conclav.dev (doc 51 FL.2)
#   ci-nightly   ci.yml       slot=nightly      the nightly, Monday to Saturday
#   ci-weekly    ci.yml       slot=weekly       Sunday's: the nightly and the weekly jobs
#   codeql       codeql.yml   rust=true         Monday's Rust analysis
#
#   workflow-dispatch.sh soak MINUTES CELLS
#                a shorter soak by hand, as root, to prove the token:
#                `workflow-dispatch.sh soak 20 2`
#
# THE TOKEN is the owner's to create and place (docs/ops.md, "The clock is on
# conclav.dev"): a fine-grained personal access token for this one repository
# with Actions read and write, in /etc/rustmush/dispatch.env, root's, mode
# 0600, as one line `DISPATCH_TOKEN=<token>`. One token serves every job.
# Under the unit, systemd reads that file as root and the script runs as a
# user of its own with the token in its environment; by hand as root the
# script reads the line itself. The token goes to curl on standard input,
# never on a command line, and is never printed.
#
# Exit 0 means GitHub accepted the dispatch. Anything else is a line on
# standard error that says what to do, in the unit's journal:
#   journalctl -u 'rustmush-dispatch@*' -n 20
set -euo pipefail

REPO="${DISPATCH_REPO:-conclavio/rustmush}"
REF="${DISPATCH_REF:-main}"
TOKEN_FILE="${DISPATCH_ENV:-/etc/rustmush/dispatch.env}"
API="${DISPATCH_API:-https://api.github.com}"
# A dispatch is sent again only when the call certainly never reached GitHub
# (no name, no connection). A second dispatch after one that did arrive would
# start the job twice: for the soak, a second run queued behind the first
# that holds the box into the morning.
TRIES="${DISPATCH_TRIES:-3}"
PAUSE="${DISPATCH_PAUSE:-30}"

die() {
  echo "workflow-dispatch: $*" >&2
  exit 1
}

token="${DISPATCH_TOKEN:-}"
unset DISPATCH_TOKEN

usage="usage: workflow-dispatch.sh soak|ci-nightly|ci-weekly|codeql   (or: soak MINUTES CELLS)"
job="${1:-}"
inputs=""
case "$job" in
  soak) workflow=soak.yml ;;
  ci-nightly)
    workflow=ci.yml
    inputs='"slot":"nightly"'
    ;;
  ci-weekly)
    workflow=ci.yml
    inputs='"slot":"weekly"'
    ;;
  codeql)
    workflow=codeql.yml
    inputs='"rust":"true"'
    ;;
  *) die "$usage" ;;
esac
what="$job ($workflow at $REF on $REPO)"
if [ "$job" = soak ] && [ "$#" -eq 3 ]; then
  if ! [[ "$2" =~ ^[0-9]{1,3}$ && "$3" =~ ^[1-6]$ ]]; then
    die "minutes is 1 to 999 and cells is 1 to 6"
  fi
  inputs="\"minutes\":\"$2\",\"cells\":\"$3\""
  what="$what, $2 minutes, $3 cells"
elif [ "$#" -ne 1 ]; then
  die "$usage"
fi
if ! [[ "$REPO" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ && "$REF" =~ ^[A-Za-z0-9._/-]+$ ]]; then
  die "the repository or the ref is not a name GitHub would take"
fi
body="{\"ref\":\"$REF\"}"
if [ -n "$inputs" ]; then
  body="{\"ref\":\"$REF\",\"inputs\":{$inputs}}"
fi

how="one line, DISPATCH_TOKEN=<token>, in a file that is root's and mode 0600 (docs/ops.md, \"The clock is on conclav.dev\")"
if [ ! -e "$TOKEN_FILE" ]; then
  die "no token: $TOKEN_FILE is not there, so $job was not started. The owner creates it: $how"
fi
mode="$(stat -c %a "$TOKEN_FILE")"
if [ $((8#$mode & 8#77)) -ne 0 ]; then
  die "$TOKEN_FILE is mode $mode, readable by more than its owner: chmod 600 it, and make a new token if anyone else is on the box"
fi
if [ -z "$token" ] && [ -r "$TOKEN_FILE" ]; then
  # By hand as root: systemd is not here to read the file.
  token="$(sed -n 's/^DISPATCH_TOKEN=//p' "$TOKEN_FILE" | tail -n 1)"
  token="${token#\"}"
  token="${token%\"}"
fi
if [ -z "$token" ]; then
  die "no token: $TOKEN_FILE does not set DISPATCH_TOKEN (or this is not root and not the unit). It wants $how"
fi
if ! [[ "$token" =~ ^[A-Za-z0-9_]+$ ]]; then
  die "the value of DISPATCH_TOKEN in $TOKEN_FILE has a character no GitHub token has (a space, a quote, a line end from another system). It wants $how"
fi

answer="$(mktemp)"
trap 'rm -f -- "$answer"' EXIT

# Prints the HTTP status. The token is one header on standard input.
dispatch() {
  printf 'Authorization: Bearer %s\n' "$token" |
    curl --silent --show-error --connect-timeout 20 --max-time 60 \
      --request POST \
      --header @- \
      --header 'Accept: application/vnd.github+json' \
      --header 'X-GitHub-Api-Version: 2022-11-28' \
      --header 'User-Agent: rustmush-workflow-dispatch' \
      --data "$body" \
      --output "$answer" --write-out '%{http_code}' \
      "$API/repos/$REPO/actions/workflows/$workflow/dispatches"
}

try=1
while :; do
  rc=0
  status="$(dispatch)" || rc=$?
  if [ "$rc" -eq 0 ]; then
    break
  fi
  # 6: the name did not resolve. 7: no connection. Neither reached GitHub.
  if { [ "$rc" -eq 6 ] || [ "$rc" -eq 7 ]; } && [ "$try" -lt "$TRIES" ]; then
    echo "workflow-dispatch: curl exit $rc, GitHub not reached; again in $PAUSE s ($try of $TRIES)" >&2
    try=$((try + 1))
    sleep "$PAUSE"
    continue
  fi
  die "curl exit $rc: the call to $API did not finish, so whether GitHub took the dispatch of $job is not known. Look at the Actions page before starting it again by hand"
done

# GitHub's own words for a refusal; they never hold the token.
said="$(tr -d '\r\n' < "$answer" | cut -c1-300)"
case "$status" in
  2??)
    echo "workflow-dispatch: dispatched $what (HTTP $status)"
    ;;
  401)
    die "GitHub refused the token (HTTP 401), so $job was not started: it has expired, was revoked or is not a token. The owner makes a new one and replaces the line in $TOKEN_FILE. GitHub said: $said"
    ;;
  403)
    die "the token may not dispatch (HTTP 403), so $job was not started: it wants Actions read and write on $REPO, and the organization may still have to approve it. GitHub said: $said"
    ;;
  404)
    die "HTTP 404, so $job was not started: the token cannot see $REPO or $workflow. A fine-grained token is for the repositories picked when it was made; $REPO must be one. GitHub said: $said"
    ;;
  422)
    die "HTTP 422, so $job was not started: $workflow at $REF takes no dispatch, or not these inputs. GitHub said: $said"
    ;;
  *)
    die "HTTP $status from GitHub, so $job was not started. GitHub said: $said"
    ;;
esac
