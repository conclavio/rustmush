#!/usr/bin/env bash
# The clock-run workflows' dispatch call (deploy/vps-runner/workflow-dispatch.sh),
# driven against a stub `curl` in a disposable fixture, and the units, the
# timers' spacing and the workflows it goes with. No network, no GitHub, no
# real token: the "tokens" here are made-up strings.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
dir="$root/deploy/vps-runner"
script="$dir/workflow-dispatch.sh"
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/bin"

# The stub: one line of arguments a call, standard input kept, the answer and
# the exit code taken from the Nth line of $STUB_PLAN ("<exit> <status> <body>").
cat > "$fixture/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$STUB_LOG"
cat > "$STUB_STDIN"
n="$(wc -l < "$STUB_LOG")"
read -r code status said < <(sed -n "${n}p" "$STUB_PLAN")
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then out="$2"; fi
  shift
done
printf '%s' "$said" > "$out"
printf '%s' "$status"
exit "$code"
EOF
chmod +x "$fixture/bin/curl"

export STUB_LOG="$fixture/log" STUB_STDIN="$fixture/stdin" STUB_PLAN="$fixture/plan"
env_file="$fixture/dispatch.env"
fake="made_up_PAT_0123456789abcdef"

# run PLAN-LINES... -- ARGS: the script against the stub; sets $rc, $out, $err.
run() {
  : > "$STUB_LOG"
  : > "$STUB_STDIN"
  : > "$STUB_PLAN"
  while [ "$1" != "--" ]; do
    echo "$1" >> "$STUB_PLAN"
    shift
  done
  shift
  rc=0
  PATH="$fixture/bin:$PATH" DISPATCH_ENV="$env_file" DISPATCH_PAUSE=0 \
    bash "$script" "$@" > "$fixture/out" 2> "$fixture/err" || rc=$?
  out="$(cat "$fixture/out")"
  err="$(cat "$fixture/err")"
}
calls() { wc -l < "$STUB_LOG" | tr -d ' '; }
fail() {
  echo "FAILED: $*" >&2
  echo "--- stdout: ${out:-}" >&2
  echo "--- stderr: ${err:-}" >&2
  exit 1
}
never_shown() {
  if grep -q "$fake" "$fixture/out" "$fixture/err" "$STUB_LOG"; then
    fail "$1: the token is in the output or on curl's command line"
  fi
}

# The checks that need no file modes come first, so they also run on Windows.

# U1: the units, the script and the installer name one path, one token file,
# and the same four jobs; no timer catches up a missed firing.
unit="$dir/rustmush-dispatch@.service"
install="$dir/dispatch-timers-install.sh"
grep -qx 'EnvironmentFile=-/etc/rustmush/dispatch.env' "$unit" || fail "U1 the unit's token file"
grep -qx 'ExecStart=/usr/local/lib/rustmush/workflow-dispatch.sh %i' "$unit" || fail "U1 the unit's command"
grep -qx 'DynamicUser=yes' "$unit" || fail "U1 the unit drops root"
grep -qF 'DISPATCH_ENV:-/etc/rustmush/dispatch.env}"' "$script" || fail "U1 the script's token file"
grep -q '/workflow-dispatch.sh" /usr/local/lib/rustmush/workflow-dispatch.sh$' "$install" || fail "U1 the installed path"
grep -qx 'JOBS=(soak ci-nightly ci-weekly codeql)' "$install" || fail "U1 the installer's jobs"
if grep -v '^#' "$install" | grep -v 'echo \|\[ ! -e ' | grep -q 'dispatch\.env'; then
  fail "U1 the installer must not touch the token file"
fi
timers=0
for timer in "$dir"/rustmush-dispatch-*.timer; do
  job="$(basename "$timer" .timer)"
  job="${job#rustmush-dispatch-}"
  timers=$((timers + 1))
  grep -qx "Unit=rustmush-dispatch@$job.service" "$timer" || fail "U1 $job: the timer's unit"
  grep -qx 'Persistent=false' "$timer" || fail "U1 $job: a missed firing must not be caught up"
  grep -qx 'WantedBy=timers.target' "$timer" || fail "U1 $job: the timer installs"
  grep -qE '^OnCalendar=.* America/Los_Angeles$' "$timer" || fail "U1 $job: the owner's time zone"
  [ "$(grep -c '^OnCalendar=' "$timer")" -eq 1 ] || fail "U1 $job: one OnCalendar"
  grep -qE "^  $job\)" "$script" || fail "U1 $job: the script does not know the job"
  if command -v systemd-analyze > /dev/null; then
    systemd-analyze calendar "$(sed -n 's/^OnCalendar=//p' "$timer")" > /dev/null || fail "U1 $job: systemd refuses the calendar"
  fi
done
[ "$timers" -eq 4 ] || fail "U1 four timers, found $timers"
grep -qx 'OnCalendar=\*-\*-\* 23:17:00 America/Los_Angeles' "$dir/rustmush-dispatch-soak.timer" || fail "U1 the soak is at 23:17"
echo 'U1 four timers in Pacific time with no catch-up; the unit, the script and the installer agree'

# U2: the box keeps the clock, so no workflow has a schedule: a late scheduled
# run beside the timer's would run the job twice (for the soak, hold the box a
# second time, in the owner's morning). And each dispatch sends inputs the
# workflow takes.
for name in ci codeql soak gate; do
  workflow="$root/.github/workflows/$name.yml"
  triggers="$(sed -n '/^on:$/,/^[a-z]/p' "$workflow" | grep -E '^  [a-z_]+:' | tr -d ' ' | tr '\n' ' ')"
  case "$triggers" in
    *schedule:*) fail "U2 $name.yml has a schedule trigger ($triggers); the timers on conclav.dev are the clock" ;;
    *workflow_dispatch:*) ;;
    *) fail "U2 $name.yml takes no dispatch ($triggers)" ;;
  esac
  if grep -nE "event_name == 'schedule'|github\.event\.schedule" "$workflow"; then
    fail "U2 $name.yml still reads a schedule event"
  fi
done
[ "$(sed -n '/^on:$/,/^[a-z]/p' "$root/.github/workflows/soak.yml" | grep -E '^  [a-z_]+:' | tr -d ' ')" = "workflow_dispatch:" ] ||
  fail "U2 soak.yml is on a self-hosted runner: workflow_dispatch and nothing else"
ci="$root/.github/workflows/ci.yml"
grep -qx '        options: \[by-hand, nightly, weekly\]' "$ci" || fail "U2 ci.yml's slot input"
grep -qx '        default: by-hand' "$ci" || fail "U2 a dispatch by hand is the by-hand slot"
grep -qF "MIND_PULSE_MS: \${{ (inputs.slot == 'nightly' || inputs.slot == 'weekly') && '250' || inputs.mind_pulse_ms }}" "$ci" ||
  fail "U2 the nightly's 250 ms pulse"
[ "$(grep -c "^    if: github.event_name == 'workflow_dispatch' && inputs.slot != 'nightly'$" "$ci")" -eq 6 ] ||
  fail "U2 the six weekly jobs"
grep -q "^      rust:$" "$root/.github/workflows/codeql.yml" || fail "U2 codeql.yml's rust input"
grep -q "^      minutes:$" "$root/.github/workflows/soak.yml" || fail "U2 soak.yml's minutes input"
echo 'U2 no workflow has a schedule; ci.yml takes the slot, codeql.yml rust, soak.yml minutes and cells'

# U3: no two timers' kept times overlap. Each timer file says how long is
# kept for its job (`# reserved-minutes:`, one and a half times its longest
# measured run, or the job's own timeout); the week is laid out in Pacific
# wall-clock minutes and every pair is compared, across midnight and the
# week's end. Daylight saving's one short night a year is in docs/ops.md.
python3 - "$dir" <<'EOF'
import glob, os, re, sys

days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
week = 7 * 24 * 60
spans = []
for path in sorted(glob.glob(os.path.join(sys.argv[1], "rustmush-dispatch-*.timer"))):
    text = open(path, encoding="utf-8").read()
    name = os.path.basename(path)[len("rustmush-dispatch-"):-len(".timer")]
    kept = int(re.search(r"^# reserved-minutes: (\d+)$", text, re.M).group(1))
    when = re.search(r"^OnCalendar=(?:(\S+) )?\*-\*-\* (\d\d):(\d\d):00 America/Los_Angeles$", text, re.M)
    if not when:
        sys.exit(f"FAILED: U3 {name}: an OnCalendar this test cannot read")
    on, hour, minute = when.group(1), int(when.group(2)), int(when.group(3))
    if on is None:
        which = range(7)
    elif ".." in on:
        first, last = on.split("..")
        which = range(days.index(first), days.index(last) + 1)
    else:
        which = [days.index(d) for d in on.split(",")]
    for day in which:
        start = day * 1440 + hour * 60 + minute
        spans.append((start, start + kept, f"{name} {days[day]} {hour:02d}:{minute:02d}"))
problems = []
for i, (a0, a1, a) in enumerate(spans):
    for b0, b1, b in spans[i + 1:]:
        for shift in (-week, 0, week):
            if a0 < b1 + shift and b0 + shift < a1:
                problems.append(f"{a} (+{a1 - a0} min) overlaps {b} (+{b1 - b0} min)")
if problems:
    sys.exit("FAILED: U3 the kept times overlap:\n  " + "\n  ".join(sorted(set(problems))))
print(f"U3 {len(spans)} firings a week, no two kept times overlap")
EOF

# D1: no file. The line names it and says who places it; GitHub is not called.
rm -f "$env_file"
unset DISPATCH_TOKEN
run "0 204" -- soak
[ "$rc" -eq 1 ] && [ "$(calls)" -eq 0 ] || fail "D1 a missing file must stop the call"
[[ "$err" == *"no token: $env_file is not there, so soak was not started. The owner creates it"* ]] || fail "D1 the message"
echo 'D1 a missing token file is one clear line and no call'

# D2: a file others can read is refused, with the variable set as systemd sets it.
printf 'DISPATCH_TOKEN=%s\n' "$fake" > "$env_file"
chmod 644 "$env_file"
if [ "$(stat -c %a "$env_file")" = 644 ]; then
  DISPATCH_TOKEN="$fake" run "0 204" -- soak
  [ "$rc" -eq 1 ] && [ "$(calls)" -eq 0 ] || fail "D2 a readable file must stop the call"
  [[ "$err" == *"is mode 644"* ]] || fail "D2 the message"
  never_shown D2
fi
chmod 600 "$env_file"
if [ "$(stat -c %a "$env_file")" != 600 ]; then
  echo 'workflow dispatch: this file system keeps no modes (Windows): U1 to U3 and D1 passed, D2 to D10 need Linux' >&2
  exit 0
fi
echo 'D2 a token file others can read is refused'

# D3: a file that does not set the variable.
echo '# nothing yet' > "$env_file"
run "0 204" -- soak
[ "$rc" -eq 1 ] && [ "$(calls)" -eq 0 ] || fail "D3 an empty file must stop the call"
[[ "$err" == *"does not set DISPATCH_TOKEN"* ]] || fail "D3 the message"
echo 'D3 a file with no token is one clear line and no call'

# D4: a value no token has (a carriage return from another system's editor).
printf 'DISPATCH_TOKEN=%s\r\n' "$fake" > "$env_file"
run "0 204" -- soak
[ "$rc" -eq 1 ] && [ "$(calls)" -eq 0 ] || fail "D4 a bad value must stop the call"
[[ "$err" == *"has a character no GitHub token has"* ]] || fail "D4 the message"
never_shown D4
echo 'D4 a value with a stray character is refused and not printed'

# D5: each timer's call, as the unit makes it: the variable from systemd, the
# file there and private. One POST, the token on standard input only.
printf 'DISPATCH_TOKEN=%s\n' "$fake" > "$env_file"
for row in \
  'soak|soak.yml|{"ref":"main"}' \
  'ci-nightly|ci.yml|{"ref":"main","inputs":{"slot":"nightly"}}' \
  'ci-weekly|ci.yml|{"ref":"main","inputs":{"slot":"weekly"}}' \
  'codeql|codeql.yml|{"ref":"main","inputs":{"rust":"true"}}'; do
  IFS='|' read -r job workflow body <<< "$row"
  DISPATCH_TOKEN="$fake" run "0 204" -- "$job"
  [ "$rc" -eq 0 ] && [ "$(calls)" -eq 1 ] || fail "D5 $job: the dispatch"
  [ "$(cat "$STUB_STDIN")" = "Authorization: Bearer $fake" ] || fail "D5 $job: the header on standard input"
  grep -q -- '--request POST --header @- ' "$STUB_LOG" || fail "D5 $job: the call's shape"
  grep -qF -- "--data $body --output" "$STUB_LOG" || fail "D5 $job: the body"
  grep -q " https://api.github.com/repos/conclavio/rustmush/actions/workflows/$workflow/dispatches\$" "$STUB_LOG" || fail "D5 $job: the URL"
  [[ "$out" == "workflow-dispatch: dispatched $job ($workflow at main on conclavio/rustmush) (HTTP 204)" ]] || fail "D5 $job: the line"
  never_shown "D5 $job"
done
echo 'D5 soak, ci-nightly, ci-weekly, codeql: one POST each with its inputs, the token on standard input and nowhere else'

# D6: by hand as root, a shorter soak: the script reads the line itself, quoted or not.
printf '# the clock\nDISPATCH_TOKEN="%s"\n' "$fake" > "$env_file"
run "0 204" -- soak 20 2
[ "$rc" -eq 0 ] && [ "$(calls)" -eq 1 ] || fail "D6 the dispatch by hand"
[ "$(cat "$STUB_STDIN")" = "Authorization: Bearer $fake" ] || fail "D6 the token read from the file"
grep -qF -- '--data {"ref":"main","inputs":{"minutes":"20","cells":"2"}} --output' "$STUB_LOG" || fail "D6 the inputs"
never_shown D6
echo 'D6 by hand: the line read from the file, minutes and cells as inputs'

# D7: arguments that are not a job, or not a window and a cell count.
for bad in "" "nightly" "soak 20" "soak 20 7" "soak abc 2" "soak 20 2 1" "soak 1000 2" "ci-nightly 20 2" "codeql x"; do
  # shellcheck disable=SC2086
  run "0 204" -- $bad
  [ "$rc" -eq 1 ] && [ "$(calls)" -eq 0 ] || fail "D7 '$bad' must be refused"
done
echo 'D7 an unknown job and bad arguments are refused before any call'

# D8: GitHub's refusals, each with what to do, never with the token.
for refusal in "401|refused the token" "403|may not dispatch" "404|cannot see conclavio/rustmush" "422|takes no dispatch" "500|HTTP 500 from GitHub"; do
  status="${refusal%%|*}"
  want="${refusal#*|}"
  run "0 $status {\"message\":\"GitHub's words\"}" -- ci-nightly
  [ "$rc" -eq 1 ] && [ "$(calls)" -eq 1 ] || fail "D8 HTTP $status is a failure after one call"
  [[ "$err" == *"$want"* && "$err" == *"GitHub's words"* && "$err" == *"ci-nightly was not started"* ]] || fail "D8 HTTP $status: the message"
  never_shown "D8 $status"
done
echo 'D8 401, 403, 404, 422 and 500 each fail once, name the job and say what to do'

# D9: no connection twice, then accepted: sent again only because it never arrived.
run "7 000" "6 000" "0 204" -- soak
[ "$rc" -eq 0 ] && [ "$(calls)" -eq 3 ] || fail "D9 an unreached GitHub is tried again"
run "7 000" "7 000" "7 000" "0 204" -- soak
[ "$rc" -eq 1 ] && [ "$(calls)" -eq 3 ] || fail "D9 three tries and no more"
echo 'D9 a call that never reached GitHub is sent again, three times at most'

# D10: a timeout may have arrived: never sent twice.
run "28 000" "0 204" -- soak
[ "$rc" -eq 1 ] && [ "$(calls)" -eq 1 ] || fail "D10 a timeout is not sent again"
[[ "$err" == *"is not known"* ]] || fail "D10 the message"
echo 'D10 a timeout is one call: a second dispatch would start the job twice'

echo 'workflow dispatch: the units, the triggers, the spacing, the token file, the four calls, the refusals and the retries passed'
