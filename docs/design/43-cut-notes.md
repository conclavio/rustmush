# 43. Cut notes — the testing strategy

One `## TS.n` heading per cut of [43-testing-strategy.md](43-testing-strategy.md) §7 (`TS.0`–`TS.21`), in
the order they landed. Each note says what was built, the gate that proves it, where the PR departed from
§7's row, and the cell's summed-minutes delta (rule 2.3 #14).

## TS.21 (2026-11-04) — the timing baseline on conclav.dev

D-43-6 (Q-43-6 amended) and §8.1. The harness's timing gate had been dormant everywhere since its blessing
host went away. That host was `linux/4/srv1930616`, conclav.io's old plan; the box now has eight CPUs and
serves dev1 and dev2. The gate is now live on conclav.dev (`linux/32/hg.conclav.dev`), so FR.5's rebless has a
live baseline to re-bless against.

**The re-bless** (`crates/tacsim/tests/harness_baseline/arena.linux.json`).
- **The order.** `scripts/vps-gate.sh --kind harness --runner conclav-dev` went first, as gate run
  `41756575489` at main `a2ef4075`. Every arena and minds lane held its lens under `TACSIM_ASSERT`, and all
  four minds variants read `held=yes`. The run then failed at the lance lane (below), so its delta never ran.
- **The bless itself** was run by hand as `ghrunner` in a throwaway clone of `a2ef4075`, at the runner's
  `Nice=10`, with `HOSTNAME` unset, so the host comes from `/etc/hostname` as every `gate.yml` job reads it.
  It had three steps:
  - The asserting lanes ran with `--skip tacsim_harness_lance`, then the delta ran against main's file. The
    deterministic block (106 fields) and all four minds blocks were equal.
  - The delta ran again with `TACSIM_BASELINE_BLESS=1` over those reports.
  - The lanes and the delta ran a second time against the blessed file, with the gate live. The p50 read
    −4.1%, the p99 −3.6% and the window p99 −4.3%.
- **What moved.** Only the bless's timing leaves were carried onto main's file: `tempos`, `setup_ms`, the
  window's milliseconds, the snapshot's encode and decode times, and the run-only `thresholds.assert`/`lens`.
  Everything else is main's byte for byte, including the deterministic block, every OOC hash pin, the leaves
  no path compares, and the eleven earlier records. The `blessed` stanza's host, commit and date are
  conclav.dev's. A new `harness_timing_update` record names the host, its core count, the reason, the
  procedure, the five readings and the leaves it moved, with their previous values. The lenses did not move.
- **The proof.** The committed file read equal with the gate live on the box. The harness binary's 24
  ordinary tests passed there too, among them `every_committed_baseline_agrees_on_the_deterministic_block`.

**The five readings** (CI cohort, `a2ef4075`; ms):

| Run | Wake p50 | Wake p99 | Window p99 |
|---|---|---|---|
| gate `41756575489` | 5.414 | 7.896 | 14.970 |
| gate `41760419368` | 5.281 | 7.213 | 14.553 |
| gate `41760428226` | 5.487 | 6.644 | 12.787 |
| by hand, blessed | 5.467 | 6.595 | 12.777 |
| by hand, confirming | 5.243 | 6.358 | 12.231 |

The blessed run is the median. The slowest gate run is +19.7% on the p99 against it, which is inside the 20%
bound but close to it. So 16b N.1's rule still applies: a timing-only failure is re-run once before it is
believed.

**The cost lane.** It has no baseline file and no host in it: its lens is the absolute 5,000 µs p99, asserted
under `TACSIM_ASSERT=1`. Moving it therefore needed only a run on the new box, not a bless.
`scripts/cost-lane.sh` on conclav.dev at `a2ef4075` read the tick gate's minimum p99 at 887 µs over five runs
and the 30-TV month's surface-fire p99 at 2,053 µs, with no breach. The month's wall was 730 s.

**Defaults and docs.**
- `scripts/vps-gate.sh` and `gate.yml` now send `--kind harness` and `--kind cost-lane` to `conclav-dev`
  (they went to `conclav-io` before). Their header comments, and the `runner` input's description, name the
  blessing host.
- In docs/ops.md, the VPS-gate table, the "Which box" paragraph, the weekly-harness intro and the cost-lane
  section now say both gates are live on conclav.dev. A new "Re-blessed on conclav.dev" paragraph follows
  the hash-v2 one.
- The module docs of `harness.rs` and the header of `scripts/harness-weekly.sh` name the new host. Only
  comments changed there; the two cargo lines did not.

**Departure: the lance lane.** Under `TACSIM_ASSERT` on main, `tacsim_harness_lance` fails its at-defaults
W8 fact pin (`harness.rs`, the `command.span_max == squads_per_side - 1` assert). The span reads 3 where 2 is
pinned. The CI job never asserts, so it never saw this. Its `harness-lance.json` reads `span_max` 3 on every
nightly back to 2026-10-19 (`c44c0bb6`), the oldest run that kept the artifact. This is a deterministic pin,
not a timing figure, and fixing it is outside this cut. Until it is re-pinned or fixed,
`scripts/harness-weekly.sh` stops before its delta, so every `--kind harness` dispatch is red. Rule #13's
dispatch for FR.5 and S.4b can't go green until then: those cuts read the lanes before it and run the delta
by hand, as this cut did. The bless skipped the lane, and nothing it records depends on it.

**Cell delta** (rule #14). Zero: no test was added, removed or lengthened, and only comments changed in a
test file. The figures are from the JUnit of #388's green CI run `41762280651` (head `faaa00bc`), set
against main's last CI run `41754225803` at `c0d123a9`; main's merges after it, up to the base `a2ef4075`,
were docs-only. Summed minutes, main → PR:

| Cell | Tests | Main | PR | Read |
|---|---|---|---|---|
| tacsim-integration-1 (shard 1) | 220 → 220 | 78.3 | 66.7 | The cell that holds the harness binary's 24 ordinary tests: 1.00 → 0.74 min. |
| Every other cell | unchanged | 558.8 | 626.3 | Untouched by TS.21. Cells moved both ways, from tacsim-lib +16.6 and rest-server-boundary +15.0 to rest-libs −8.2 and shard 7 −9.0: runner noise. |
| **All cells** | | 637.1 | 693.0 | |

No cell's wall is set by this cut, and no story moves to another shard.

## TS.21b (2026-11-04) — the lance span, re-pinned by record

The follow-up TS.21's departure owed. Under `TACSIM_ASSERT` the lance lane's at-defaults span pin read 3 where
17b cut M4.5 pinned `squads_per_side - 1`, 2. That stopped `scripts/harness-weekly.sh` before its delta, so
every `--kind harness` dispatch was red.

**The culprit.** A first-parent bisect on conclav.dev ran from M4.5's `f19a88fb` (span 2) to `c44c0bb6` (3).
It probed the lance lane alone and read `command.span_max` from `harness-lance.json`, in seven steps. It names
`5993ba20`, "feat(tacsim): restore M5r doctrine and motor behavior" (2026-10-17, committed straight to main):
its parent `d9e6ef1f` reads 2 and it reads 3. That commit moved the stock mind's doctrine and the motor and
re-blessed the arena lane's baseline. It never ran the lance lane under the flag, and the delta does not read
`harness-lance.json`.

**Re-pin, not fix.** A probe of the live orders at the read, at `d9e6ef1f`, `5993ba20` and main `3cb04168`,
shows the fight moved and the span did not:

- In M4.5's fight `blue-1`'s #100003 was wrecked at 71,132 ms, before the minute-one mark.
- Since `5993ba20` it outlives the mark, so `blue-1` stands three at 71,500 ms. The hull takes that order
  (live until 161,500 ms) and is wrecked before the 90,000 ms release.
- The release lands on standing machines only. A wreck keeps the order it died under on its mind row: 16b cut
  AI.1, and M4.5's own `blue-2` #100011.
- So blue's commander holds `blue-1` on that wreck alone, beside `blue-2` and `blue-3`. Red's holds `red-2` and
  `red-3`.

The span's definition (every live order a mind row holds, wrecks counted) and the release's are unchanged.
Neither is a defect.

**The pins that moved, and the evidence:**

- The span: 2 → 3.
- "Rests on no wreck" is re-stated exactly: the counted join equals the standing join plus `blue-1` under
  blue's commander. The standing span holds at 2.
- `blue-1`'s standing at 71,500 ms: 2 → 3.

At main every other figure holds as M4.5 pinned it: no end, 131,500 ms, eight orders, a busiest minute of 3,
five releases. `5993ba20` itself read four releases (`red-1` stood two at the release), and GC.2's `c2653243`
moved that back. The end pin's prose now says neither side is out (red four wrecks and blue six at main, where
M4.5's fight had five and seven). Verified under `TACSIM_ASSERT=1` at `3cb04168` on conclav.dev.

**So it cannot drift unseen again.** `tacsim_harness_lance` now asserts whatever `TACSIM_ASSERT` says, because
every check it makes is deterministic and its timing is report-only. The CI job still sets no `TACSIM_ASSERT`
(D-H-8, D-16-19, held by `the_weekly_script_runs_the_ci_jobs_two_lines_verbatim`). It holds the lane's pins
now, as does every `--kind harness` dispatch (rule #13).

**Cell delta** (rule #14). Zero: the lane is `#[ignore]`d, and no ordinary test changed.

## TS.8 (2026-11-04) — one scanner, one strip chain

§7's TS.8 row and §10.4, as PR #393. Test-only: no crate's production code changes.

**Built.**
- **`mush-contract-tests/src/scan.rs`** is the one copy of `repo_root`, the walker (`files`, `rust_files`,
  `crate_dirs`, `relative`), the comment-and-literal stripper, the `#[cfg(test)]` blanker and
  `file_is_test_only`. Fifteen gates call it. Gone: 13 private `repo_root`s, 13 recursive walkers and the two
  drifted lexers (`panic_budget.rs`, `matcher_call_sites.rs`). The single-level listings of non-crate
  directories (help pages, golden frames, workflows, budget files, unit packs) stay at their gates.
- **`state-slot-owners.toml`:** the 78 rows whose site is under a crate's `tests/` are keyed
  `module = "<crate>/tests::<module>"`: the module path, less a `mod.rs`, less the group directory of the
  target that declares it. So `shard_3/drones.rs` is `tacsim/tests::drones`, and a reshard rewrites no row.
  The 61 production rows keep `path`. The guard refuses a test-rig row keyed by path (naming its key), a
  row with both keys or neither, and a key that resolves to two files.
- **One strip chain.** `tacsim/tests/support/lane_chain.rs` holds `strip_ladder`, `strip_workshops`,
  `strip_wide_sites` and their two tables, moved verbatim from `support/sites.rs`; it needs only
  `serde_json`. mush-server's `split_boundary.rs` includes it by `#[path]`, and `relay_registry.rs`'s
  `strip_urvara` and `strip_ladder` copies are gone. Doc 41's lane v21 (RS.1, RS.6) extends the one chain.

**The gate: behaviour unchanged, measured.** A throwaway probe ran the old copies against the new ones over
the whole tree. The 13 walkers return identical file lists. The merged stripper is byte-identical to
`panic_budget.rs`'s on all 2,568 `.rs` files, and every per-file production panic count is unchanged. It
differs from `matcher_call_sites.rs`'s copy on 17 files, every one a `br#"…"#` literal: that copy's
identifier guard read the `b` prefix as an identifier and lexed the raw string's body as code. The merged
stripper reads the guard from the blanked output, so it lexes those literals as the panic budget's copy
did. The matcher gate pins its census exactly and passes, so no verdict moved. `scan.rs` carries a unit
test for the case. The relay registry's lane v13 and v15 rebuilds hold their published digests.

**Departure from §7's row.** None in scope. The strip chain is shared by `#[path]` rather than through a
crate, so it stays test code; the file's header says why it must not grow a `super::` or a tacsim-only type.

**Cell delta** (rule #14). Structurally zero: no test was added or lengthened (four unit tests in `scan.rs`
and one in `state_slot_owners` are all under 0.1 s), and the walk and lexing work is the same. From the JUnit
of #393's green CI run `41768292766` (head `041ccbf4`) against main's CI run `41767438548` at the PR's merge
base `c252ec8d`. Summed, main → PR:

| What | Main | PR | Read |
|---|---|---|---|
| rest-libs, the 15 ported gates + lib | 42.3 s | 34.6 s | The cell ran faster as a whole (19.6 → 16.4 min); untouched gates fell as much (`module_ns_owners` 5.1 → 3.9 s). |
| rest-server-boundary, `relay_registry::` (65 tests) | 643.9 s | 840.7 s | Load: the binary's untouched modules rose the same (17.5 → 22.4 min), and the untouched rest-server cell rose 49.6 → 64.9. |
| shard 3, `drones::` (23 tests) | 1,715.5 s | 1,689.2 s | Flat; the cell 50.9 → 49.9 min. |
| shard 5, `sites::` (27 tests) | 218.5 s | 221.4 s | Flat; the cell 33.6 → 34.1 min. |

No cell's wall is set by this cut, and no story moves to another shard.

## TS.3b (2026-11-04) — the step audit

Read-only. The question was: for the dozen modules of §2.4, where do each story's minutes go? Four buckets:
- **boot**: composing the world, opening the site, the pad fixture;
- **real-time wait**: a real clock, a tokio sleep, an arena window;
- **work**: simulation CPU on a stepped clock;
- **assertion**: reading back.

The twelve are the nine §2.4 names (`knowing`, `claims`, `match_lifecycle`, `cockpit`, `mining`, `drones`,
`plans`, `unloading`, `field_gc4`), plus `field_gc5` (5.8's family), `driving` and `vehicles`, the next-largest
tacsim modules. The minutes are run 41827163605's JUnit, from main `4390f04c` on hosted 4-vCPU cells, after
TS.16. The split is a source reading of every test of 30 s or more and of each rig. Its anchors are tests that
do only one thing, such as a test that only boots. Per test it is good to about ±30 %, because a test shares
its cell with three others. The per-module write-ups, with the evidence for every row, are kept outside the
tree in the doc-43 notes folder (`ts3b-audit/`).

| Module (cell) | Tests | Summed min | Boot | Wait | Work | Assert | Where the minutes go |
|---|---|---|---|---|---|---|---|
| **5.8 the field window**: `field_gc4` + `field_gc5` (shard_1) | 12 | 13.4 | 6.6 | **6.2** | 0.4 | 0.2 | gc4: the arena's wall-clock `disengage_s`: the kept journey's two 90-s fields, and 15-s fields in the four lane3 tests. gc5: the keyed boot (~33 s a test), then 15-s windows |
| `claims` (shard_5) | 35 | 41.0 | 24.3 | 0 | 15.4 | 1.3 | 41 keyed worlds for 35 tests (a world = keyed `boot_over` + `stock` + Ahuna opened + pad, ~32–39 s) |
| `plans` (shard_6) | 28 | 34.4 | 22.6 | 0 | 11.0 | 0.8 | 37 worlds for 28 tests; P3 and P10 build three each |
| `drones` (shard_3) | 23 | 28.2 | 13.5 | 0 | 12.3 | 2.4 | work: T5's fixed 4,000 fires (473 s) and T9's 2 × 800 (311 s), ~0.1 s of debug CPU a fire |
| `knowing` (shard_4) | 83 | 26.7 | 22.0 | 0 | 3.3 | 1.5 | keyed `stocked()` (~8 s) in every test; `on_pad` (~13 s, stepped fires) in the 41 SM.4/survey tests |
| `cockpit` (shard_7) | 192 | 26.5 | 21.6 | 3.5 | 0.3 | 1.2 | the production-roster `Rig::boot` (~5 s) + a match check-out (~3 s) in ~150 tests; the wait is the ledger tape's 16-KiB chunk fill (`CHUNK_BYTES`, ~4–5 s a wait, 12 real-clock tests) |
| `mining` (shard_7) | 26 | 23.5 | 19.3 | 0 | 3.5 | 0.7 | one keyed world + Ahuna + pad a test; M12 and M8e boot two in a loop; M8 rewarms four times |
| `match_lifecycle` (shard_8) | 144 | 15.5 | 12.1 | 0.7 | 1.9 | 0.9 | ~190 compositions of the production roster (~3.2 s each) and 20 reboots |
| `driving` (shard_1) | 22 | 13.9 | 11.9 | 0 | 1.6 | 0.5 | the keyed `stocked()` + `on_pad` (~32 s a world), one or two a test |
| `vehicles` (shard_6) | 52 | 12.0 | 9.2 | 0 | 2.3 | 0.5 | one stocked world (~7 s) a test; three preflight tests build 22 worlds for 22 refusal sentences |
| `unloading` (shard_8) | 24 | 8.9 | 7.8 | 0 | 0.7 | 0.4 | the keyed `stocked()` + `on_pad` rig (~19.5 s) in 20 of 24 tests |
| **The twelve** | **641** | **244.0** | **170.9 (70 %)** | **10.4 (4 %)** | **52.6 (22 %)** | **10.3 (4 %)** | |

**Measured, not read: wall against CPU.** To check the reading, the same twelve modules ran once on conclav.dev (main `7a535806`, `-P ci`, 8 threads). A cargo target runner wrapped every test process and recorded its wall time and its CPU time (`getrusage` of the child, user + sys). A test whose wall exceeds its CPU spent the difference waiting. The idle column is that difference, summed per module.

| module | tests | wall min | CPU min | idle min (wall − CPU, per test, ≥ 0) | idle share |
|---|---|---|---|---|---|
| knowing | 83 | 19.1 | 19.1 | 0.0 | 0% |
| claims | 35 | 14.4 | 14.4 | 0.0 | 0% |
| cockpit | 192 | 12.6 | 9.2 | 3.5 | 27% |
| plans | 28 | 11.6 | 11.6 | 0.0 | 0% |
| match_lifecycle | 144 | 10.9 | 10.8 | 0.2 | 2% |
| drones | 23 | 10.4 | 10.4 | 0.0 | 0% |
| mining | 26 | 8.1 | 8.1 | 0.0 | 0% |
| driving | 22 | 6.5 | 6.5 | 0.0 | 0% |
| field_gc4 | 6 | 6.5 | 2.6 | 3.9 | 60% |
| unloading | 24 | 6.0 | 6.0 | 0.0 | 0% |
| vehicles | 52 | 4.7 | 4.7 | 0.0 | 0% |
| field_gc5 | 4 | 2.1 | 1.4 | 0.7 | 32% |
| **all** | 639 | 112.9 | 104.7 | 8.3 | 7% |

The measurement agrees with the reading. Overall the twelve are 7 % idle; the reading put real-time wait at 4 %, and the idle figure also counts lock and I/O waits.
- Nine modules are 0–2 % idle.
- The idle time is in `field_gc4` (60 %), `field_gc5` (32 %) and `cockpit` (27 %).
- The most idle single test is the gc4 journey kept at 90: 153 s idle of 204 s.
- After it come the lane3 and gc5 tests at the rig default: ~13 s idle per 15-s field, ~27 s for two fields.
- Then come cockpit's tape and reclaim waits, 9–21 s each.
- The box runs these tests about 2.2 times faster than a hosted cell: 112.9 wall minutes against the hosted 244.

**What the table says.**
- **Real-time wait is 4 % of these minutes, and nearly all of it is 5.8's family and cockpit's ledger tape.**
  - Nine of the twelve modules wait on no real clock at all. They are `TestWorld` suites, and every window in
    them (`sweep(120)`, `T9_WINDOW`, the IC-hour spans) is a `set_now` jump costing one `fire_due`.
  - The clock seam (TS.10) is therefore the lever for the field family and the arena-backed modules, not for the
    bulk of tacsim's minutes.
- **Boot is 70 %, and boot is CPU.** It is debug-build composition, content load, site generation and stepped
    port fires. That puts it squarely in TS.3's reach: opt-level 1 for dependencies cut these cells' summed
    minutes by about a fifth to a half on its first run (TS.3's note: −26 % for tacsim on the box, −27 % hosted).
- **The 5.8 row after TS.16.**
  - The four lane3 tests and the two one-field gc5 tests now run at the rig default of 15. The two-field tests
    went from ~211–214 s to 66–70 s, and the one-field tests from ~115–123 s to 51–53 s: 75 s a field, as
    constructed.
  - One copied window is left inside the journey TS.16 kept at 90.
    `gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift` sets `disengage_s = 90` for
    the rig (`field_gc4.rs:190`), so both of its fields get it. Only the second field needs to stay open (the
    drag happens inside it); the first does nothing inside its field. That is ~75 s of idle wall.
  - CR.1 renamed §5.8's two gc5 tests (`…keeps_the_captive`, `…captures_the_crew`) and added `_at_512` twins
    in shard_8 (`crew_custody`, 118.9 s).
- **The arena clock is pausable today.** `match_lifecycle::af2_…a_minute_of_fire` runs 60 arena seconds under
    `current_thread, start_paused` in 8.8 s. 25 cockpit tests on the paused clock average the same ~9 s as the
    real-clock ones, and their tape waits cost nothing there. Doc 43c §0 has the consequence for TS.10.

**Steps that boot a world to test one refusal** (Q-43-12's question; D-43-12 answers it: no unit rewrite).
Named here for the record:
- `vehicles`: `sm12b_the_yards_preflight_refuses_each_recorded_breach` (78.9 s),
  `sm3a_the_rig_pools_preflight_refuses_each_recorded_breach` (67.6 s) and
  `sm3a_the_vehicles_line_names_what_is_missing` (39.3 s): 22 worlds for 22 sentences.
- `match_lifecycle`: `af3_a_boot_voids_a_match_it_cannot_resume_for_each_reason` (112.8 s): 32 compositions for
  16 void reasons.
- `claims`: C16, C2 and C23 build 4, 3 and 2 worlds to prove a text identical across deposit tables (413 s,
  ~300 s of it boot). C21, C20 and C14b are single refusals (52–72 s).
- `mining`: M9 opens a second site for one `MINE_CLAIMED` refusal (59 s).
- `driving`: four refusal tests, at ~32 s of boot each.
- `field_gc4`: `lane3_a_load_into_a_strangers_carrier_is_refused` boots three pilots on two sites and runs one
  real 15-s field for one refusal line.
- `knowing`: the cold-site and moved-rig refusals (23–25 s).
- `cockpit`: `o1_start_refuses_a_mode_the_ground_does_not_declare`, and the two registry-collision checks.

Under D-43-12 the lever for these is the boot's cost itself (TS.3; a cached composed world is a question for
the owner, not this audit), never a unit test in place of the story.

## TS.4 (2026-11-04) — the hostile-input suites

§5.1's parsers, each fed what an attacker or a broken peer can send. **No suite found a panic, an abort or an
unbounded state**, so nothing landed `#[ignore]`d and no production code changed.

- **The command line** (`mush-server/tests/world_task/hostile_lines.rs`). `boot::build_engine`, with the
  TacSim layer on, is wrapped in a `TestWorld`. It runs 1,500 seeded lines as a plain player and 1,500 as God.
  Most lines are built on a registered verb: a name or alias, a prefix character, switches, and edge tokens
  (dbrefs, numbers past every width, splitters, markup, control and bidi characters, 600-character runs). The
  rest are arbitrary text. The engine is shared, so earlier lines' objects and input contexts are in play. A
  reach floor stops a vacuous pass: more than a quarter of each actor's lines must get an answer other than
  `Huh?`. They measured 688 and 683 of 1,500. `ParsedArgs` gets its own contract property inline in
  `mush-core/src/cmd.rs`: every shape, with `raw`, `lhs`/`rhs` and `rhs_list` checked against the split rule.
- **The key parser and the caps replies** (`mush-net-ssh/tests/editor_proptest/key_parser.rs`). The input is
  arbitrary bytes, plus bursts weighted to `ESC [ O _ P \ BEL`, split anywhere. The properties: no panic; no
  control character in a `Char`/`Alt`; and the parser's carried state (its `Debug`) stays under 4 KiB after
  256 KiB of an unterminated CSI, APC or DCS. Capability replies with each final (`c t u ~`) reach
  `DeviceAttributes::parse`/`CellSize::parse` through the parser, and any text reaches them directly.
- **Both Rhai engines** (`mush-mind/tests/sandbox/nesting.rs`, `mush-script/tests/script/nesting.rs`). There
  are fifteen nesting shapes: parens, unary, arrays, maps, blocks, `if`, `while`, calls, interpolation,
  closures (script only), flat `+`/`&&` chains, index and method chains. Each runs at the guard's edges, far
  past it, and up to the 32 KiB source cap, at the top level and inside a function. Each runs on an explicit
  2 MiB thread, the stack the minds pool and the world task get (nothing in the tree sets another), through
  `Sandbox::compile`/`think` and through the script module's boot compile and dispatch. One deep compiles.
  Twice the guard is refused with Rhai's "exceeds maximum complexity", the flat chains included. Unbounded
  recursion faults at the call-level limit.
- **mush-ipc** (`mush-ipc/tests/roundtrip/hostile_frames.rs`). Every sample frame of both directions is cut at
  every byte behind a whole frame. The codec reads needs-more-bytes and `FrameReader` reads `UnexpectedEof`;
  a cut at the boundary is a clean EOF. Six corrupt bodies behind a valid prefix, including length-bomb
  `array32`/`str32` headers, are `InvalidData` after the leading frame, and the valid frame after them is
  never reached. An oversized prefix faults without its body. Arbitrary bytes in any chunking, and sample
  bodies with overwritten bytes, never panic.
- **`client_command_strategy`** (`mush-proto/tests/decode_proptest.rs`) now generates every `ClientCommand`,
  `Query` and `InvokeRequest` family. Before, it was missing `Invoke`, `Ack`, `Composing`, `Respond`,
  `Picture` and `Query::Commands`. `variant_tag` matches all three enums with no wildcard, so a new variant
  does not compile until it is listed. `strategy_reaches_every_variant` fails until the strategy draws it.

**Departures from §7.** The fuzzer runs one shared engine, not a fresh `TestWorld` per case. A boot costs
seconds and shared state reaches more handlers. The runner is seeded, so CI is deterministic and a failure
prints its line. `mush-ipc` and `mush-server` gain `proptest` as a dev-dependency. `mush-proto`'s
`tests/wire.rs` exhaustive round trip (doc 05 I12 C2) is not touched. The proptest round trip now covers
every variant.

**Summed minutes.** About +0.3: `hostile_lines` is 12 s, and the rest is under 4 s across five binaries.
Every module joins a binary that already exists.

## TS.20 (2026-11-04) — pinned reconstructions

§7's four unpinned rebuilds of old content each get a pin. A pin is the shape the old version actually had, read
from the commit before the change it rebuilds around and written into the test as fixed history. Pinning
turned up drift in two of the four: those reconstructions had been carrying fields their version never wrote.

- **`ooc_v3`** (`mush-atlas/tests/interior.rs`) is held to `V3_REBUILT_DIGEST`, like `ooc_v6` and
  `staff_lobby_v4`. It cannot be the published digest (`74344b62…`, `44115c47`). The published version 3 has
  the same 24 entries, and 20 are byte-equal, but four differ:
  - the Welcome Hall's desc and sign: the shipped text is kept, which is why the upgrade plans one update;
  - the three `Lounge <L>` exits' `l` segment: since 16b cut D.5, apply refuses it.

  So the real bytes cannot boot, and a committed fixture of them would prove nothing. The doc comment lists the
  four entries. A re-pin must confirm the rebuild still differs from version 3 in those four entries and no
  others.
- **The format-3 checkpoint body** (`mush-economy/tests/e2b.rs`) is held to `FORMAT_3_FOLD_FIELDS`, the 28
  fields `FoldState` serialized at `b205438c^`. **Finding: the rebuild had drifted.** Besides
  `title_lineage`, it carried `seizure_receipts` and `bay_refit_receipts`, two audits added after format 4
  without a format bump, which format 3 never had. The rebuild now strips all three.
- **The pre-O.1 replay** (`tacsim/tests/shard_1/replay.rs`) is held to `PRE_O1_REPLAY_KEYS`, the 18 keys
  `Replay::to_document` wrote at `8c2f0388^`. It matched: `mode` was the only key added since.
- **The R4 relay shape** (`mush-server/tests/split_boundary/relay_registry.rs`) is held by `assert_r4_shape`
  to the registry, node, pair and half fields at `233fbb2e`. R4 and R7 are consecutive commits on `relay.rs`.
  **Finding: the rebuild had drifted.** R7 added `losses` and each half's `damaged` as well as
  `structural_version`, and the "R4" document still carried both. The rebuild now drops them, after asserting
  they are empty and false (so R4 could express that state).

**Departures from §7.** None of the four uses a committed fixture. Three rebuild documents from a live run,
whose bytes depend on that run. The fourth is the OOC pack, whose published bytes no longer apply. So the pin
is the version's field set, or its rebuilt digest, and the `golden/v12` template does not fit any of them.

**Summed minutes.** Zero: four assertions in tests that already exist.

## TS.12 (2026-11-04) — the partition experiment, measured

Q-43-5 / D-43-5. This was a measurement on branch `claude/ts12-partition-experiment` (draft PR #422), never
merged.
- **The layout.** tacsim's ten `shard_N.rs` roots became four domain roots over the same module files, with no
  file moved:
  - `domain_field`: 34 modules, the field, crew, bay and sortie journeys;
  - `domain_economy`: 31 modules, the surface, market, claims, plans, mining and drones;
  - `domain_arena`: 29 modules, the arena, cockpit, match lifecycle and minds;
  - `domain_world`: 10 modules, knowing, vehicles, voyage, chart and ephemeris.
- **The cells.** Every `tacsim-integration-N` cell built all four binaries and ran
  `cargo nextest run --partition hash:N/10` over them. `harness` (cell 1) and `cost_lane` (cell 2) ran first, as
  today.
- **The runs.** Three attempts of run 41873443102, against main's runs 41870246223 (`7a535806`, the branch's
  base), 41870167197 and 41827163605.
- **The compile figures** are cargo's `Finished` lines in each cell's log. **The job and test minutes** are from
  the jobs API and the JUnit.

| Run | Compile + link, mean a cell (s) | Cells' compile, summed (min) | Slowest cell's job (min) | Fastest (min) | Spread | Tacsim summed test min (10 cells) |
|---|---|---|---|---|---|---|
| main 41870246223 | 215 | 35.8 | 23.1 | 12.5 | 1.85 | 523 |
| main 41870167197 | 217 | 36.1 | 21.7 | 12.2 | 1.77 | 461 |
| main 41827163605 | 197 | 32.8 | 19.4 | 10.4 | 1.86 | 435 |
| TS.12 a1 | 249 | 41.5 | 24.6 | 11.6 | 2.12 | 484 |
| TS.12 a2 | 233 | 38.8 | 23.8 | 9.3 | 2.54 | 485 |
| TS.12 a3 | 226 | 37.7 | 21.8 | 10.6 | 2.05 | 460 |

The TS.12 summed column leaves out harness and cost_lane (about 5 min), whose JUnit the experiment did not
upload.

**Reading**
- **The link cost is small.**
  - A cell compiles and links in 236 s on average against main's 210 s: about +26 s a cell, or 4–5 runner-minutes
    a run.
  - The hosted runner links the four binaries in parallel on its four cores. After the lib, the four domain
    binaries took 38–45 s together (cells 1 and 2, where they built second).
- **The wall is a wash.**
  - Cell 2 was the slowest in two attempts: 24.6 and 23.8 min. That is the experiment's own artifact: cost_lane,
    which runs alone, went serially before cell 2's hash share.
  - Without cell 2 the slowest cell was 20.9, 21.2 and 21.8 min, against main's 19.4–23.1.
- **The hash does not balance by time.**
  - Spread (slowest over fastest job) was 2.05–2.54 against main's 1.77–1.86.
  - The same cell's summed minutes varied up to 2× between attempts (cell 10: 50, 25, 29), and the partition is
    deterministic, so that variation is the hosted machine's speed.
  - Neither layout escapes that noise, but the hash layout never has to be rebalanced.
- **The reshard cost it removes.**
  - There were five layout changes in eight days (§2.4). Each was a `git mv`, `mod` lines, run lines and (until
    TS.8) state-slot rows, and each was judged on several runs.
  - Under partition, a new test module is one `mod` line in its domain root, and no shard ever moves.
- **What adoption would also need** (not built here):
  - the guards rewritten for domain roots (`ci_tacsim_shards`, the gated-test inventory's `shard_4` line, and
    the `tacsim-minds` steps' `--test shard_2` names);
  - the M8 jobs' `--test shard_9` lines;
  - cost_lane given its own cell or run inside the partition.
- **One test failed in all three attempts:** `field_gc4::gc4_the_wingman_drags…` at `field_gc4.rs:317`.
  - It also failed on #423 and on #429 (a web-only diff on the standard layout), so it is a race in main's own
    code.
  - Still, 3 of 3 under partition against 0 of 4 on main's runs means the neighbour mix the hash gives it makes the
    race likelier. Adopting the partition would first need that race found (rule 2.3 #7). The record is in the
    doc-43 notes folder.

D-43-5's test was whether the partition's link cost is lower than the reshard cost. Measured, it is about 26 s a
cell and no wall, against a hand-made layout PR every day or two. The decision is the owner's.

## TS.5b (2026-11-04) — the fault matrix as sequences

The second half of §7's TS.5 row. The new test is `gate5_fault_matrix::`
`lifecycle_sequences_with_two_faults_and_restarts_recover_to_the_clean_run`, in `ci_policy`.

- **The script.** Each case runs one script over each of the matrix's five real bundles (Gate-1 brace, Gate-2
  tactical fire, and the three Gate-4 delayed-decision bundles):
  - submit;
  - reconcile;
  - one outbox attempt per pending delivery that **fails (`Retry`)**, then is delivered;
  - checkpoint.
- **The cases.** Every single fault, and every pair at two different stages, from the matrix's own modes:
  `BeforeIntent`, `AfterIntent`, every `AfterNMaterializationOps`, `AckLostAfterFinalize`, `TokenConflict`
  and `AckLostAfterCheckpoint`. Each is crossed with:
  - no restart;
  - a restart before each one stage;
  - a restart before every stage.

  A restart is `InMemoryStore::dump`, then `restore` into a fresh store, as a process reopens the same
  durable state. The outbox stage's restart falls between the failed attempt and its retry. That makes
  over 500 cases, and the test asserts the floor.
- **The check.** Every case ends with its exact receipt, a compact rejoin and no delivery left pending. Its
  final store, dumped to JSON, must be **identical to the same script run clean**: recovered equals live.
  All pass. No sequence found a divergence.

**Departures from §7.** The Redis backend is not driven. `AuthoritativeFaultStore` wraps the memory store
only, so a Redis twin needs a fault wrapper over `RedisStore`, and that run belongs in the `redis` group
(`.config/nextest.toml`'s `redis-serial`). `cancel` and `advance time` are not modelled, because the store
has no cancel of an authoritative bundle: a decision's cancellation and its deadline live in the engine.
Conservation and sole custody are covered by TS.5a's model, not here; this test's check is whole-store
identity. The minimised-failures part of the row has nothing to keep yet.

**Summed minutes.** About +0.15: 9 s in a debug build.
