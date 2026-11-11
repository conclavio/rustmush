#!/usr/bin/env bash
# Hermetic acceptance of the launch switch in deploy/instance/render.sh (doc 44
# cut MW.1): only a temporary tree is real, and no binary is named, so nothing
# is preflighted. It renders the SHIPPED template, so a template whose
# commented minds block was reworded past the switch fails here.
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
render="$root/deploy/instance/render.sh"
template="$root/deploy/instance/mushd.toml.tmpl"
mkdir -p "$fixture/data"

# The reference env with its paths moved into the fixture, and one edit per
# case. `instance.env` is read, never sourced, so it is edited as text.
env_with() {
  local switch="$1" pulse="$2"
  sed -e "s|^DATA_DIR=.*|DATA_DIR=$fixture/data|" \
    -e "s|^MINDS_LAUNCH=.*|MINDS_LAUNCH=$switch|" \
    -e "s|^REGISTRY_PULSE_MS=.*|REGISTRY_PULSE_MS=$pulse|" \
    "$root/deploy/instance/instance.env.example" >"$fixture/instance.env"
  grep -q "^MINDS_LAUNCH=$switch\$" "$fixture/instance.env"
  grep -q "^REGISTRY_PULSE_MS=$pulse\$" "$fixture/instance.env"
}
# The render as it was before the switch: the substitution and nothing else.
plain() {
  while IFS='=' read -r k v; do
    case "$k" in '' | '#'*) continue ;; esac
    printf 's|__%s__|%s|g\n' "$k" "$v"
  done <"$fixture/instance.env" >"$fixture/plain.sed"
  sed -f "$fixture/plain.sed" "$template" >"$fixture/plain.toml"
}
run() { bash "$render" "$fixture/instance.env" "$template" "$fixture/mushd.toml" >"$fixture/output" 2>&1; }
live_lines() { grep -c "$1" "$fixture/mushd.toml" || true; }

# 1. Off, no pulse: the plain substitution, byte for byte, document or not.
env_with off ''
plain
run
cmp "$fixture/plain.toml" "$fixture/mushd.toml"
printf '{}\n' >"$fixture/data/launch.json"
run
cmp "$fixture/plain.toml" "$fixture/mushd.toml"
rm "$fixture/data/launch.json"
# An empty switch is off.
env_with '' ''
plain
run
cmp "$fixture/plain.toml" "$fixture/mushd.toml"

# 2. On, no document: still the plain substitution, and the script says why.
env_with on ''
plain
run
cmp "$fixture/plain.toml" "$fixture/mushd.toml"
grep -q 'launch.json does not exist yet' "$fixture/output"

# 3. On, the document written: the header and the one line, and nothing else.
printf '{}\n' >"$fixture/data/launch.json"
run
[ "$(live_lines '^\[tacsim\.minds\]$')" = 1 ]
[ "$(live_lines "^launch_path = \"$fixture/data/launch.json\"\$")" = 1 ]
[ "$(live_lines '^registry_pulse_ms')" = 0 ]
[ "$(diff "$fixture/plain.toml" "$fixture/mushd.toml" | grep -c '^>')" = 2 ]
[ "$(wc -l <"$fixture/plain.toml")" = "$(wc -l <"$fixture/mushd.toml")" ]

# 4. The pulse alone: the header and its line, the launch still commented.
rm "$fixture/data/launch.json"
env_with off 50
plain
run
[ "$(live_lines '^\[tacsim\.minds\]$')" = 1 ]
[ "$(live_lines '^registry_pulse_ms = 50$')" = 1 ]
[ "$(live_lines '^launch_path')" = 0 ]
[ "$(diff "$fixture/plain.toml" "$fixture/mushd.toml" | grep -c '^>')" = 2 ]

# 5. Both, as a dev box after its bring-up: three lines.
env_with on 50
plain
printf '{}\n' >"$fixture/data/launch.json"
run
[ "$(live_lines '^\[tacsim\.minds\]$')" = 1 ]
[ "$(live_lines '^launch_path = ')" = 1 ]
[ "$(live_lines '^registry_pulse_ms = 50$')" = 1 ]
[ "$(diff "$fixture/plain.toml" "$fixture/mushd.toml" | grep -c '^>')" = 3 ]
# No state renders an empty path.
[ "$(live_lines '^launch_path = ""')" = 0 ]

# 6. A typo is a refusal that leaves the installed config alone; so is a pulse
#    that is not a number.
cp "$fixture/mushd.toml" "$fixture/kept.toml"
env_with yes ''
if run; then echo 'render accepted MINDS_LAUNCH=yes' >&2; exit 1; fi
grep -q "MINDS_LAUNCH must be 'on' or 'off'" "$fixture/output"
cmp "$fixture/kept.toml" "$fixture/mushd.toml"
[ ! -e "$fixture/mushd.toml.new" ]
env_with off fast
if run; then echo 'render accepted REGISTRY_PULSE_MS=fast' >&2; exit 1; fi
grep -q 'REGISTRY_PULSE_MS must be empty or a number' "$fixture/output"
cmp "$fixture/kept.toml" "$fixture/mushd.toml"
[ ! -e "$fixture/mushd.toml.new" ]

# 7. The shipped dev envs carry the switch on and the fast pulse, and render
#    with no document as a fresh store's deploy does: no live launch_path.
for box in dev1 dev2; do
  sed -e "s|^DATA_DIR=.*|DATA_DIR=$fixture/data-$box|" -e 's|^EXTERNAL_IP=.*|EXTERNAL_IP=203.0.113.10|' \
    "$root/deploy/$box/instance.env" >"$fixture/instance.env"
  grep -q '^MINDS_LAUNCH=on$' "$fixture/instance.env"
  run
  [ "$(live_lines '^launch_path')" = 0 ]
  [ "$(live_lines '^registry_pulse_ms = 50$')" = 1 ]
  [ "$(live_lines '^raid_director = true$')" = 1 ]
done

echo 'render fixture: the launch switch off, on without and with its document, the pulse, and the refusals passed'
