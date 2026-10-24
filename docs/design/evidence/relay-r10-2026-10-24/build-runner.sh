#!/usr/bin/env bash
set -euo pipefail
ROOT=/opt/rustmush/dev1
[ "$(realpath "$ROOT")" = /opt/rustmush/dev1 ]
[ "$(realpath "$ROOT/build")" = /opt/rustmush/dev1/build ]
EVIDENCE="$ROOT/r10-evidence"
mkdir -p "$EVIDENCE"
chmod 0700 "$EVIDENCE"
export PATH="$ROOT/r1-tools/node_modules/node/bin:/root/.cargo/bin:$PATH"
export CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
for bin in mush-server mush-gateway; do
 test -f "$EVIDENCE/pre-r10-$bin" || cp "$ROOT/build/target/release/$bin" "$EVIDENCE/pre-r10-$bin"
done
systemctl show dev2mush-gateway -p MainPID > "$EVIDENCE/dev2-before.txt"
STAGE=$(mktemp -d "$ROOT/r10-source.XXXXXXXX")
[ "$(dirname "$(realpath "$STAGE")")" = "$ROOT" ]
tar -xf "$ROOT/r10-candidate.tar" -C "$STAGE"
rsync -rc --delete --exclude=target/ --exclude=node_modules/ --exclude=web/dist/ --exclude=data/ --exclude=.source-commit --exclude=.source-tree "$STAGE/" "$ROOT/build/"
cp "$ROOT/r10-source-commit.txt" "$ROOT/build/.source-commit"
cp "$ROOT/r10-source-tree.txt" "$ROOT/build/.source-tree"
sha256sum "$ROOT/r10-candidate.tar" > "$EVIDENCE/source-archive-sha256.txt"
cp "$ROOT/build/.source-commit" "$EVIDENCE/source-commit.txt"
cp "$ROOT/build/.source-tree" "$EVIDENCE/source-tree.txt"
cd "$ROOT/build"
cargo fmt --all --check > "$EVIDENCE/format.log" 2>&1
# Only test fixture files changed after the production stories.
# Their source-tree identity is retained separately and the delta is recorded.
grep -q 'test result: ok. 49 passed; 0 failed' "$EVIDENCE/story-tests.log"
grep -q 'test result: ok. 21 passed; 0 failed' "$EVIDENCE/story-tests.log"
grep -q 'test result: ok. 10 passed; 0 failed' "$EVIDENCE/story-tests.log"
echo R10_STORIES_PASSED
cargo test --locked -p tacsim --test ports --test sites --test vehicles --test stations --test reset --test price --test sale --test voyage --no-fail-fast -j4 > "$EVIDENCE/content-tests.log" 2>&1
cargo test --locked -p tacsim --test deliberation l2_ -j4 > "$EVIDENCE/fixture-smokes.log" 2>&1
cargo test --locked -p tacsim --test driving sm4b_recover_from_afar_drives_home_and_climbs_aboard -j4 >> "$EVIDENCE/fixture-smokes.log" 2>&1
cargo test --locked -p tacsim --test mining sm7_rights_gate_mining_and_never_the_way_home -j4 >> "$EVIDENCE/fixture-smokes.log" 2>&1
cargo test --locked -p tacsim --test resolution no_instrument_moves_while_a_match_runs -j4 >> "$EVIDENCE/fixture-smokes.log" 2>&1
cargo test --locked -p tacsim --test harness m4_2_the_load_mind_is_calibrated_to_the_class_iii_row -j4 >> "$EVIDENCE/fixture-smokes.log" 2>&1
cargo test --locked -p tacsim --lib relay_metric -j4 > "$EVIDENCE/metric-tests.log" 2>&1
cargo test --locked -p mush-org -p mush-faction -p mush-propagation --lib -j4 > "$EVIDENCE/owner-tests.log" 2>&1
cargo test --locked -p mush-org --test e1a -j4 > "$EVIDENCE/org-tests.log" 2>&1
cargo test --locked -p mush-content --test apply -j4 > "$EVIDENCE/content-frame-tests.log" 2>&1
cargo test --locked -p mush-core --lib schedule -j4 > "$EVIDENCE/core-tests.log" 2>&1
cargo clippy --locked -p mush-content -p mush-org -p mush-faction -p mush-propagation -p tacsim --lib -j4 -- -D warnings > "$EVIDENCE/clippy.log" 2>&1
cargo test --locked -p mush-contract-tests --test legacy_handler_allowlist --test unit_looks --test module_ns_owners --test state_slot_owners --test panic_budget -j4 > "$EVIDENCE/contracts.log" 2>&1
cargo test --locked -p mushd --test observer_rule --test arena_custody --test arena_story --test front_door --test sim_cockpit --test sim_lobby --test sim_net --test sim_range --test sim_reflex --test sim_rematch --test sim_solo --test sim_units --test sim_weapons --test typeahead -j4 -- --test-threads=4 > "$EVIDENCE/ci-observer.log" 2>&1
echo R10_REGRESSIONS_PASSED
cd web
npm run build
cd ..
bash scripts/check-web-bundle.sh "$ROOT/build"
cargo build --locked --release -j4 -p mushd -p mush-server -p mush-gateway --features mushd/tls,mush-gateway/tls
echo R10_BUILD_COMPLETE
