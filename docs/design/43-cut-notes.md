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

## TS.3 (2026-11-04) — optimised dependencies, measured

Q-43-4 / D-43-4. `[profile.dev.package."*"] opt-level = 1` in `Cargo.toml`: every non-workspace dependency is
optimised in dev and test builds, and the workspace's own crates are not. Variant B added `opt-level = 2` for
`rhai`, `serde_json`, `rmp-serde`, `sha2`, `blake3`, `image` and `flate2` (PR #423, closed unmerged).

**Hosted CI, summed test minutes per cell** (nextest JUnit).
- main: runs 41870246223 (`7a535806`), 41870167197 and 41827163605.
- A: run 41871003719, attempts 1 and 2.
- B: run 41873792386.

| cell | main mean (min–max) | optA mean (min–max) | optB | optA Δ% | optB Δ% |
|---|---|---|---|---|---|
| bot | 10.7 (9.7–11.9) | 8.2 (8.2–8.2) | 8.1 | -23 | -24 |
| bot-stories | 28.3 (28.0–28.6) | 25.3 (25.3–25.3) | 26.2 | -10 | -7 |
| rest-gateway | 5.0 (4.6–5.4) | 3.0 (2.8–3.2) | 3.5 | -39 | -30 |
| rest-libs | 18.9 (16.1–20.6) | 11.3 (10.1–12.5) | 12.6 | -40 | -33 |
| rest-mushd | 31.4 (26.7–34.0) | 17.6 (15.5–19.7) | 17.7 | -44 | -44 |
| rest-server | 46.5 (36.9–51.6) | 40.9 (35.7–46.0) | 38.9 | -12 | -16 |
| rest-server-boundary | 33.6 (30.2–40.2) | 21.4 (21.2–21.6) | 16.6 | -36 | -51 |
| tacsim-integration-1 | 59.0 (43.3–72.6) | 49.6 (43.3–55.9) | 54.8 | -16 | -7 |
| tacsim-integration-10 | 48.3 (37.3–61.2) | 39.1 (35.1–43.1) | 26.4 | -19 | -45 |
| tacsim-integration-2 | 28.2 (21.5–40.4) | 27.5 (27.4–27.7) | 29.6 | -2 | +5 |
| tacsim-integration-3 | 49.0 (46.9–50.1) | 33.3 (32.8–33.7) | 23.1 | -32 | -53 |
| tacsim-integration-4 | 46.5 (30.4–61.7) | 38.9 (33.1–44.6) | 46.3 | -16 | -0 |
| tacsim-integration-5 | 42.0 (34.3–46.0) | 33.6 (33.4–33.8) | 34.7 | -20 | -17 |
| tacsim-integration-6 | 46.3 (46.2–46.5) | 28.5 (23.4–33.6) | 34.3 | -39 | -26 |
| tacsim-integration-7 | 49.6 (48.9–50.0) | 29.2 (28.1–30.3) | 29.6 | -41 | -40 |
| tacsim-integration-8 | 51.5 (30.3–62.7) | 36.4 (36.0–36.7) | 49.6 | -29 | -4 |
| tacsim-integration-9 | 52.3 (51.5–52.9) | 32.1 (24.9–39.2) | 40.6 | -39 | -22 |
| tacsim-lib | 38.3 (37.7–39.0) | 21.9 (18.9–24.9) | 27.5 | -43 | -28 |
| **total** | 685.5 (636.8–751.6) | 497.7 (493.6–501.8) | 520.3 | -27 | -24 |

**conclav.dev, tacsim's whole sweep** (`-P ci`, 16 threads, base `7a535806`, summed minutes per binary):

| binary | main-a | main-b | o1-a | o2-a | main-b vs main-a | o1-a vs main-a | o2-a vs main-a |
|---|---|---|---|---|---|---|---|
| tacsim::shard_1 | 32.7 | 32.7 | 26.2 | 27.5 | -0% | -20% | -16% |
| tacsim::shard_9 | 27.5 | 27.4 | 22.1 | 22.9 | -0% | -20% | -17% |
| tacsim::shard_8 | 26.7 | 26.7 | 20.9 | 21.7 | -0% | -22% | -19% |
| tacsim::shard_10 | 25.5 | 25.7 | 18.5 | 20.1 | +1% | -27% | -21% |
| tacsim::shard_4 | 22.0 | 21.5 | 15.3 | 16.2 | -2% | -31% | -26% |
| tacsim::shard_7 | 20.7 | 20.8 | 16.3 | 17.1 | +1% | -21% | -18% |
| tacsim::shard_3 | 18.7 | 18.1 | 11.9 | 12.8 | -3% | -36% | -32% |
| tacsim::shard_6 | 16.4 | 16.6 | 11.9 | 12.9 | +1% | -27% | -21% |
| tacsim::shard_5 | 16.0 | 15.9 | 11.6 | 12.2 | -1% | -28% | -24% |
| tacsim | 14.6 | 14.7 | 9.4 | 10.4 | +0% | -36% | -29% |
| tacsim::shard_2 | 14.0 | 14.2 | 11.1 | 11.9 | +1% | -21% | -15% |
| tacsim::cost_lane | 4.8 | 4.8 | 3.2 | 3.3 | +1% | -33% | -30% |
| tacsim::harness | 0.4 | 0.4 | 0.3 | 0.3 | -2% | -31% | -27% |
| **total** | 240.1 | 239.4 | 178.7 | 189.2 | -0% | -26% | -21% |

The wall of that sweep was 1366 s on both main runs, 1036 s for A and 1082 s for B. A cold build of tacsim's tests
took 190 s, 198 s and 197 s.

**Reading**
- **A meets D-43-4's bar.**
  - On hosted CI, 11 of the 18 cells fall by a fifth or more, and the total by 27%.
  - On the quiet box every tacsim binary falls by 20–36%, with main against itself at ±0%.
- **B is slower than A on the same host** (−21% against −26%), so per-crate opt-level 2 is not taken.
- **Why it works.** TS.3b found that boot (debug-build composition, content, site generation) is 70% of the
  biggest modules' minutes, and the dependencies (serde, rhai, hashing, compression) do much of that work.
- **The cost.** Every warm target dir (CI's caches, the VPS gate's, the owner's box) rebuilds its dependencies
  once. The hosted cells' first run built in 1–4 min more than main's.
- **The one red.** B's single failure, `field_gc4::gc4_the_wingman_drags…` (`field_gc4.rs:317`), passed on A in
  every run and on main's three. It also failed twice on TS.12's branch, which has no profile change, so it is not
  an effect of the opt level. Its cause is not established: a real-time race in that journey, or (on TS.12) the
  binary's composition.

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

**Refreshed on main `f9912792` (2026-11-05).** Two things on main changed under the first reading:
- TS.3's opt-level 1 for dependencies;
- the field rigs' pinned seed (`field.seed`, `0x5EED_0002`, `903fd04c`/`abbe8379`), which fixed `gc0` and `cf6c`.

The setup:
- **The roots.** The four domain roots were regenerated over current main. Nine new modules went to their domains:
  `field_fr5`, `field_rs5` and `vacated_return_rs4` to field; `in5_production`, `in7_part_recipes`, `in10_pools`,
  `yard_in9`, `port_security_rs7` and `tycho_house_rs6a` to economy.
- **The runs.** Two attempts of run 41953124538, against main's runs at `f9912792` (the base; its one red was
  `rest-server`, not a tacsim cell), `a6bea79f` and `a40aa5de`.
- **The cells** were set up as in the first experiment.

| Run | Compile + link, mean a cell (s) | Slowest cell's job (min) | Slowest other than cell 2 | Fastest (min) | Spread | Tacsim summed test min |
|---|---|---|---|---|---|---|
| main 41952369921 (`f9912792`) | 194 | 19.4 | 19.4 | 8.9 | 2.18 | 352 |
| main 41944128660 (`a6bea79f`) | 198 | 18.6 | 18.6 | 9.9 | 1.88 | 361 |
| main 41944094187 (`a40aa5de`) | 201 | 16.9 | 16.9 | 10.3 | 1.64 | 361 |
| TS.12 41953124538 a1 | 219 | 16.4 | 16.4 | 10.6 | 1.55 | 343 |
| TS.12 41953124538 a2 | 239 | 18.4 | 18.4 | 11.8 | 1.56 | 375 |

- **The race is gone.** `field_gc4::gc4_the_wingman_drags…` passed in both partitioned attempts (203 s and 214 s),
  and every tacsim cell was green. The only reds were the two guards that pin the shard layout (`tacsim-minds`,
  `rest-libs`). The blocker the first reading named no longer stands.
- **The link cost is about the same.** A cell compiles and links in 229 s on average against main's 198 s: about
  +31 s a cell, or about 5 runner-minutes a run. The four domain binaries after the lib took 30–45 s together.
- **The wall and the balance now favour the partition slightly.**
  - The slowest cell was 16.4 and 18.4 min, against main's 16.9–19.4.
  - The spread was 1.55–1.56, against 1.64–2.18.
  - With opt-level 1, cost_lane no longer makes cell 2 the wall.
  - Two attempts against three runs is still inside the hosted machines' 2× noise, so read this as "no worse",
    not "better".
- **What adoption needs is unchanged:** the guards and the `--test shard_N` lines (minds, M8) rewritten for
  domain roots. A new module becomes one `mod` line in its domain root, and no reshard ever follows.

The experiment PR #422 is closed. The decision on Q-43-5 is the owner's.

## TS.6 (2026-11-04) — the fiat budget

D-43-9 as written. `crates/mush-contract-tests/tests/ci_policy/fiat_budget.rs` (module `fiat_budget::` of
`ci_policy`) counts the unchecked state-slot accessors in every `crates/<pkg>/tests/**.rs`, after comments and
literals are blanked. The accessors are `module_state_mut`, `module_state_pair_mut`, `get_pair_mut` and
`get_captured_pair_mut`, matched as whole words, so path-qualified spellings count too. The checked
`state_mut`/`try_state_mut` are not counted.

- **The budgets.** `fiat_budget/<crate>-tests.txt` holds one line per file: `<budget> <low-water> <path>`.
  Today's baseline is **205 sites in 96 files across 21 crates**. Doc 43 counted 210 at its base; tacsim
  has 86 of today's sites.
- **The ratchet.** The gate is equality: a file's count is its budget, so a removal is re-blessed down and
  stays down. A budget above its low-water mark, a new file included (low-water 0), needs
  `# raised <path> to <n>: <reason>`, and the reason must name an as-built section (`§`). Bless lowers
  freely. It raises only with `FIAT_BUDGET_RAISE="<path>: <reason naming §…>"` and refuses without it.
  Unit tests pin the count's spellings (prose and literals excluded) and the raise rules (missing, no `§`,
  wrong number, stale, new file).
- **`TestWorld::exec_ok`** runs a line that must succeed and panics, naming the line and its error, on any
  error-styled answer, `Huh?` included. It is the prefer-instead the gate's failure names. 44 test files
  today carry their own `has_error`, so the common case is real.

**Departures from §7.** Inline `#[cfg(test)]` modules in `src/` are not counted; they are in-crate unit tests
of a slot's own owner. Equality rather than a ceiling makes the ratchet bite on removals too. It runs in the
existing `ci_policy` binary, so CI needs no change.

**Summed minutes.** About +0.01: the walk takes 0.4 s warm and 17 s cold.

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

## TS.19 (2026-11-04) — mounted assertions

The structural source probes in `stores/unit.test.ts`, `lib/game.test.ts` and `lib/roomband.test.ts` are gone.
Each case is now a mounted assertion in jsdom, or it was already one elsewhere. The three stale "no harness"
comments went with the probes. The vocabulary bans elsewhere in the suite stay.

- **`components/UnitScreen.test.ts`** (new):
  - `KeyValue` and `Table` render repeated producer ids, rows and cells.
  - `UnitScreen` branches refusal, then unanswered, then no-subject, then the panes.
  - The ring renders under the no-subject line.
  - An actionable `GenericView` row is a focusable button that prefills on Enter and Space; a row with no
    action is neither.
- **`components/RoomBand.test.ts`** (new): the weather chip shows the prose, or the bare state, never an empty
  chip. The vitals strip names its subject as text, not only as a tooltip.
- **`components/Game.test.ts`** (new) mounts the whole shell on a real `GameApp` driven through a fake socket.
  jsdom has no `<dialog>`, so two lines polyfill `showModal`/`close`. It covers:
  - B-08: one command box, after the screen and before the legend, registered with the app; none on the
    channel screen, where the app lets go of it.
  - The unit tab's badge counts nothing until the server answers.
  - Screen 0 has no tab without a seat or a replay, and draws the replay, not the cockpit, once one opens.
- **The three app whitelists** (Cockpit, Picture, Replay) are now `components/reach.ts`'s `onlyReaches`. It is
  a Proxy around each mount's stand-in app that throws on any member outside the list. So every test in those
  three files enforces the whitelist on every path it drives, child components and aliases included, and the
  send bans fall out of it.
- **B-60** (the zoom glyphs) and **B-09** (the label plan) were already asserted by `CockpitScreen.test.ts`'s
  mounts, so their probes simply went. `Table`'s check that its callers declare constant columns also went:
  it is spelling, and the header's keying is commented in `Table.svelte`.

Every new assertion was checked against a mutation of the component, and each mutation failed the test that
covers it.

**Finding.** `PictureView.svelte:487` reads `app.chart` through a cast, `(app as { chart?: … }).chart`. SF.18's
space views draw the sky chart's frame behind the ship. The source probe's `app\.(\w+)` regex could not see the
cast, so the "picture and helm only" whitelist passed while the screen reached a third member. The runtime
whitelist caught it on its first run. `chart` is now an explicit, read-only entry on the Picture and Cockpit
lists. Whether the cast should become a declared prop is the owner's call.

**Summed minutes.** About +0.1: three jsdom files, 13 cases, a few seconds of the web job.

## TS.5a (2026-11-04) — the generated lifecycle, part one

§7's row has three parts. This PR lands the first two: the state machine with the invariants as its model,
and the check that a generated history's checkpoint boot equals its whole replay. The fault matrix as
sequences is TS.5b.

- **The state machine** (`mush-economy/tests/e2a/lifecycle.rs`) replaces `e2a.rs`'s fixed-seed
  conservation fuzz. That fuzz had one instrument, two-line transfers and no shrinking.
  - The inputs are histories of up to 47 steps, run against the pure engine: pay, place, capture, release
    and expire a hold, title, transfer (with or without releasing liens), assign, unassign, encumber,
    release a lien, and resubmit an earlier plan.
  - After every step the model checks:
    - conservation;
    - the held projection equals the account's `Held` holds;
    - invariant 23's available floor;
    - invariant 26's cap;
    - invariant 24: a hold leaves `Held` once and captures at most what it held, and an `Expire` without a
      deadline never posts;
    - invariants 12 and 13: no lien rides a title move, a moved title loses its assignment and maintenance
      payer, and an asset is never assigned to its own owner;
    - a posted plan resubmitted is `AlreadyPosted`.
  - `the_histories_reach_every_kind_of_post` is the reach floor: every step kind posts somewhere in a fixed
    sample.
  - Mutation check: with invariant 12's `Encumbered` refusal disabled, the property fails and shrinks to a
    three-step history (title, lien, transfer).
- **Checkpoint boot equals whole replay** (`mush-economy/tests/e2b/lifecycle_replay.rs`).
  - Histories of the economy's own staff verbs run through `TestWorld`:
    - `@econ/mint`, `@econ/pay`;
    - `@hold/place`, `@hold/capture`, `@hold/release`;
    - `@asset/title`, `/transfer`, `/assign`, `/unassign`, `/lien`, `/release`.
  - The sweep fires at random points, with the checkpoint cadence at 4 and the tail at 3.
  - The written store is booted twice: as written, which resumes from the newest checkpoint, and with every
    checkpoint document and the boot manifest removed, which replays the whole journal. The two snapshots
    must be equal to each other, and to the live one.
  - Its reach floor: every verb succeeds somewhere, and some boot resumes from a checkpoint. The floor caught
    the generator's own gap on its first run. Only a hold placed `for=` someone can be captured
    (`api.rs:710`), and unfunded accounts rarely placed at all, so the cast now starts funded and places
    with a beneficiary.

**Departures from §7.** It ships in two PRs. The pure engine has no custody provider, so custody is covered
only through the module rig's title and lien verbs. `@vault` deposit and withdraw need a vault and a carried
object, and wait for TS.5b's sequences. No minimised failure exists yet to keep as a regression: none was
found.

**Summed minutes.** About +0.05: 0.7 s in `e2a` and 2.3 s in `e2b`.

## TS.12, adopted (2026-11-05) — four domain binaries, hash-partitioned

D-43-5 (Q-43-5 ruled 2026-11-04, owner: "Adopt it"), on main `bf0b5f24`.

**The layout.**
- **The move.** The ten `shard_N.rs` roots are gone. Their 115 modules moved with `git mv` into four domain
  directories, each module with its `#[path]` siblings and its subdirectory (`knowing`'s eight quest files,
  `match_lifecycle/`). Each root, `crates/tacsim/tests/<domain>.rs`, declares its directory's modules and the ten
  shared `support` modules.
  - `domain_field`: 37 modules, the field, crew, bay, escort, sortie and salvage journeys.
  - `domain_economy`: 37 modules, the surface, ports, market, claims, plans, mining, drones and industry.
  - `domain_arena`: 30 modules, the arena, cockpit, match lifecycle, minds (`twin` among them) and definitions.
  - `domain_world`: 11 modules, knowing, vehicles, voyages, the chart, the sky, geology and `moon_ports`.
- **The suffix rule moved one module.** `moon_ports` went to world rather than economy, because in economy its
  name ends with `ports`. Nothing imports across domains.
- **What did not change.** No test was edited; only comments and run lines that named a shard were updated. The
  state-slot rows are keyed by module (TS.8) and did not change. The fiat budget's 28 tacsim rows are keyed by
  path and were re-keyed.

**The cells.**
- `.github/tacsim-integration-shards.txt` has a new shape. A `partition:` line names the four domain binaries,
  and numbered lines name the whole targets a cell runs first: harness on 1, cost_lane on 2. cost_lane gets its
  clean home first in cell 2, where TS.12's refresh showed it no longer sets the wall.
- `scripts/ci-tacsim-shard.sh N` runs that cell's whole targets, then `cargo nextest run --partition
  hash:N/10` over the four domain binaries.
- The whole targets' JUnit is kept as `junit-first.xml`, and the cell uploads both reports.
- The M8 jobs name `domain_field`, and the three tacsim-minds steps name `domain_arena`.

**The guards.**
- `ci_tacsim_shards` now reads the domain layout. It checks:
  - every target is listed exactly once (the partition or one cell);
  - the matrix runs exactly the numbered cells, and the script partitions;
  - every file under `tests/<domain>/` is declared by that domain's root, and a root declares nothing outside its
    directory and `support/`;
  - no module name in a domain ends with another's.
- It also refuses any directory under `tests/` that is not a domain's, `support/` or `harness_baseline/`. A
  branch cut before the adoption therefore fails until it moves its new modules.
- `state_slot_owners`' key-normalisation test and the gated-test inventory's substring example name domain paths.

**Doc 43.**
- Rule 2.3 #3: a new test is a `mod` line in its domain root.
- Rule 2.3 #14: a cut reports its domain binary's summed-minutes delta, summed over the ten cells.
- Q-43-5 and D-43-5 carry the ruling and the figures.
- docs/ops.md's test-targets section drops the "shard with the most room" guidance; the reshard recipe it implied
  is gone.

**The gate.** The contract tests, all four domain binaries' build, clippy and strict rustdoc on `tacsim` and
`mush-contract-tests`, shellcheck, and cells 1 and 3 run through the script on conclav.dev; then the PR's own
run.

## Rule 2.3 #15 (2026-11-05) — dispatch the nightly jobs a PR touches

Owner, after the nightly of 2026-11-05 broke on #475. Its grace windows moved the M7 and M8 stories' pins, and
no pull-request run exercises those stories. The rule is in §2.3 as #15, the twin of #13 for the harness.

**The map**, `.github/nightly-paths.txt`, has one line per path: `<job id> <tier> <path or directory/>`. It
covers all 12 of `ci.yml`'s non-PR test jobs:
- **nightly:** M7, M8, FR.7, the harness, the scale harness and the release build;
- **weekly:** M6's two, the M8 leak battery, the House doctrines and the bot's two win rates.

The map is deliberately the paths a story's pins move with, not everything it runs:
- mush-core's `action_state`: the timers, grace windows and terminal capacity;
- the economy and the market a story buys through;
- the minds and the registry;
- each job's own test modules.

A nightly that breaks on a path the map did not name widens its line in the PR that fixes the nightly.

**The advice.** `scripts/nightly_advice.py [<base> [<head>]]` prints the jobs a change should dispatch, with
the `gh workflow run CI --ref <branch>` line to do it.
- **Locally** it defaults to this branch against `origin/main`.
- **In CI**, the `lint` cell runs it on every pull request against the PR's base (fetched, since the checkout is
  one commit deep). It writes a warning annotation and the step summary.
- **It is advice, never a failure** (`continue-on-error`, exit 0). The rule is the PR author's to keep.
- **Run against #475** (`13c72c21`^1..`13c72c21`), it names `mind-commerce-acceptance` and `m8-gate-stories` (17 and 20
  files under `mush-core/src/action_state/` and `mush-economy/src/`), plus `scale` and `release` (for
  `Cargo.lock`).

**The guard**, `ci_policy::nightly_paths`, reads `ci.yml`'s jobs and their job-level `if:`. A Sunday-schedule
`if:` makes a job weekly; any other schedule, nightly; `workflow_dispatch` alone, dispatch. It refuses:
- a mapped job ci.yml does not have;
- a non-PR job with no line (`nightly-report` is exempt: it tests nothing);
- a tier that disagrees with the `if:`;
- a path that does not exist;
- a workflow that no longer runs the advice.

A third test pins the tier reading on the three schedule spellings.

**No dispatch selector was added.** `gh workflow run CI` runs every non-PR job at once, in parallel, at the fast
pulse unless `-f mind_pulse_ms=1000` is given. A per-job `only` input would have meant an `if:` edit in thirteen
jobs, for a saving the hosted runners do not need.

**Refined the same day: the gate is the fast run** (owner, relayed 2026-11-05). In the owner's words: "we can
leave the slow-run for the next nightly, or maybe have it fire off the nightly but not wait for it (otherwise we
add 3+ hours to landing any PR touching a nightly)."
- **Rule #15 now gates on the fast pace.** That is the CI dispatch with `mind_pulse_ms` empty, the M7 stories'
  50-ms pulse; every other non-PR job has no pace input and runs as it does nightly.
- **The real pace is not a gate.** Production's 1,000-ms pulse, the schedule's always, belongs to the next
  nightly. It may also be fired with `-f mind_pulse_ms=1000` and left: `nightly-report` runs on dispatches too,
  so its failure reaches the owner through D-43-3's channel, not the PR.
- **The script follows.** `scripts/nightly_advice.py` prints the fast command as the gate and the real-pace one
  as optional. `--dispatch` fires the fast run for the current branch, and `--dispatch --real-pace-too` also
  fires the real-pace run, unwaited.
- **The map's header and the lint step's comment** say "fast pace".

## The RS.6c-2 resale stories' run time (2026-11-12) — idle turns, not work

`tacsim::domain_economy house_resale_rs6c2::rs6c2_tycho_restores_and_sells_a_wreck_from_{ahuna_flank,urvara_reach}`
were the long pole of every pull request: 1,417 s and 1,347 s on hosted cells (run 43332528656, main
`cf9a036a`), which made `tacsim-integration-9` and `-6` the two longest test jobs at 31.8 and 30.0 minutes
against 21.8 for the next. On conclav.dev they took 672 s and 675 s in a whole sweep (run 43332747562) and
672 s and 663 s run alone.

**The cause.** The story's main loop stepped the clock 300 s a turn while Tycho's lifter was under way and
60 s a turn while the workshop's order was open. The supply leg (doc 41 cut RS.9b) is three long passages:
Ceres to Luna (16.6 IC days), Luna to Deimos (10.9) and Stickney home (16.0). At 301 s a turn that is 12,501
turns in which no arm has anything to do, and each turn is one whole fire of every module tick: 43 to 52 ms
in a test build. The time is CPU, on one thread: 671.1 s of user time in 672.4 s of wall, 1.0 s of system
time. There is no sleep and no wall-clock timer in the story.

**Measured** on conclav.dev, the CI profile's build (`cargo nextest run -P ci --no-run`, tacsim at
opt-level 0 over dependencies at 1), the test binary run alone with timed spans in the story and a timer
round each tick handler. Ahuna Flank; Urvara Reach reads the same within a few seconds.

| phase | before, s | turns | after, s | turns |
|---|---:|---:|---:|---:|
| world boot, ports stocked (`stocked_tuned`) | 4.5 | | 4.6 | |
| the pilot chartered, leased, flown to the pad, the site warmed (`on_pad_as_at`) | 8.7 | | 9.0 | |
| the wreck fixture, the house standing up | 0.1 | | 0.1 | |
| the census's steady balance | 0.5 | | 0.5 | |
| 57 IC days skipped (one jump, four fires) | 0.15 | | 0.15 | |
| the sweep: out, launch, tow, load, home, berth, the award | 10.6 | 630 | 10.2 | 626 |
| the supply leg's four passages | 609.9 | 12,541 | 2.4 | 52 |
| the supply leg docked (Luna, Deimos, Stickney) | 4.9 | 258 | 4.1 | 258 |
| the restoration (the order open eight hours) | 23.6 | 474 | 2.4 | 64 |
| listing, preview, the mint, the buy | 0.003 | | 0.003 | |
| the sale's settlement | 0.27 | 16 | 0.17 | 13 |
| the final census | 0.006 | | 0.005 | |
| **the story** | **663.5** | 13,904 | **33.7** | 1,001 |

**The change** (the story only). `stride` in `house_resale_rs6c2.rs`: a passage's first hour is stepped 300 s
a turn as before, so what follows a launch lands as it did; the rest is one stride that ends on the
journey's arrival second (`JourneyBook`, `CarrierJourney::arrival_at`). An order's first hour is stepped 60 s
a turn; the rest is one stride that ends on its `due_ic`. It is how the rig already crosses the pilot's own
passage (`on_pad_loaded_at`: `to_ic(depart_ic + total_s)`). No assertion changed. The submissions the main
loop lands are the same before and after, on both sites: 11 `market_lot`, 6 `tacsim_port_burn`, 10
`tacsim_voyage_epoch`, 4 `tacsim_surface_act`, 4 `tacsim_surface_done`, 6 `tacsim_surface_tow`, 1
`tacsim_bay_work`, all landed. The census balance, the listing's price and the order count are the same.
What the story no longer does is fire the idle world 12,900 more times between the census's two reads:
the clerks' ambient fires across those 43 IC days are now the 1,001 turns that remain.

**Times.** conclav.dev, alone: 672.4 s to 34.5 s (Ahuna Flank), 662.9 s to 34.6 s (Urvara Reach). Twenty
runs in a row of each after the change, other builds running on the box: all forty passed, 33.1 to 35.6 s
and 33.4 to 37.1 s. Side by side under `cargo nextest run -P ci`: 33.7 s and 34.3 s. The hosted times after
the change are in the pull request that carries this note.

**The override is gone.** `.config/nextest.toml` gave these two tests forty minutes before termination in
the `ci` profile (RS.9b; the profile's limit is twenty). It set nothing else: no thread count, no group. The
stories were as slow alone as beside others.

**Why the hosted cell took twice as long.** Same profile, same single thread of CPU. The same 13,105 tests
sum to 39,327 s hosted and 22,315 s on conclav.dev. Clock-bound tests read the same on both (the bot
stories, 0.99 to 1.01 times); CPU-bound ones read 2.0 to 2.8 times. The story did not slow down inside a
32-thread sweep on conclav.dev, so it is not contention there. The hosted cell runs four tests on four
vCPUs; which part of the factor is the core and which the sharing was not measured.

**What an idle fire costs** (the first passage, 4,779 fires, nobody connected, no field, the lifter under
way; 44.6 ms a fire):

| tick | ms a fire | |
|---|---:|---|
| `tacsim.port` | 31.9 | the stock arm 20.0 (five clerks, 4.0 each), the site arm 10.1 (seven site clerks, 1.4 each), the house arm 0.8, the rest under 1 |
| `industry.sweep` | 4.5 | |
| `propagation.delivery` | 2.9 | |
| `inventory.sweep` | 1.6 | |
| `tacsim.surface` | 1.6 | |
| the other ticks | 2.0 | none over 0.4 |

The port tick's share grew through the story: 30.7 ms a fire on the first passage, 38.1 ms on the last. That
is a test build; a release build was not measured, and production fires the port tick every five seconds.
Nothing was changed in the game. It matters to tests: a story that fires the world N times pays about
45 ms a fire here and twice that on a hosted cell, whatever happens in the fire.

**The twins.** They differ only on the site: the crawler's drive, tow and load at 256 and at 512, about 7 s
of each run. At 34 s each on conclav.dev both stay on every pull request; moving one to the nightly would
save about a minute of one cell.

**The slowest after them**, hosted run 43332528656 against the self-hosted sweep 43332747562, the same
commit. The cause column is a reading of each test, not a measurement; the ratio agrees with it (about 1 is
a real clock, 2 or more is CPU).

| test | hosted, s | conclav.dev, s | the same cause? |
|---|---:|---:|---|
| `mush-bot::mining_cohort a_captain_and_its_escort_fight_the_raider_in_an_ic_field` | 576.5 | 579.6 | no: two bots over SSH on a live server, real time |
| `mush-bot::mining_cohort a_bot_hires_launches_mines_recovers_and_sells_with_a_detachment` | 493.9 | 491.3 | no: a bot over SSH, real time |
| `mush-bot::mining_cohort the_mining_root_runs_under_the_bots_serde` | 363.4 | 358.6 | no: the bot's lane over SSH, real time |
| `domain_economy drones::sm12b_the_drones_mine_deliver_and_never_sell` | 362.7 | 130.8 | yes, in part: 4,000 fixed six-second fires across 6.7 IC days; it asserts after every fire, so a stride needs those checks kept |
| `domain_field field_story::s2_the_field_story_at_512` | 235.8 | 199.2 | no: the arena runs at the wall's pace; the last wait is a 30 s negative |
| `domain_field field_gc4::gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift` | 215.0 | 198.9 | no: two fields each wait out a 90 s disengage in real time |
| `domain_field field_story::gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark` | 170.5 | 154.6 | no: as `s2`, at 256 |
| `domain_field escort_lease::cf6c_a_leased_escort_flies_out_fights_withdraws_and_goes_back` | 151.4 | 120.2 | no: real-time arena waits, a 90 s disengage |
| `domain_economy claims::sm10_the_registry_and_the_list_print_public_facts` | 142.0 | 54.4 | no: four worlds booted and set up; its voyages already jump |
| `domain_arena match_lifecycle::af3_a_boot_voids_a_match_it_cannot_resume_for_each_reason` | 134.5 | 69.7 | no: sixteen arms, 32 boots, a snapshot wait each |

`drones::sm12b_the_restart_changes_nothing` (113.5 s hosted) steps the same way as its sibling: about 1,600
six-second fires over two arms.

## The tacsim cells planned by time (2026-11-18)

TS.12's hash partition drifted out of balance as the 512-site field stories grew (CC.2 slowed the ground
cycles). Over main's green runs 44171357746, 44195103450, 44207692403, 44223456939 and 44362324881, the
summed test-minutes per cell (the partition share, from the `nextest-junit-tacsim-integration-N` artifacts):

| run | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 44171357746 | 43.4 | 52.9 | 44.8 | 54.1 | 53.7 | 78.9 | 68.3 | 48.0 | 69.6 | 59.4 |
| 44195103450 | 60.0 | 35.2 | 40.5 | 74.6 | 54.6 | 73.4 | 49.0 | 43.0 | 39.0 | 60.8 |
| 44207692403 | 49.0 | 34.0 | 40.7 | 74.4 | 70.5 | 91.2 | 71.6 | 37.5 | 52.8 | 48.6 |
| 44223456939 | 39.2 | 46.6 | 65.2 | 60.0 | 43.1 | 57.5 | 58.1 | 65.7 | 72.5 | 33.1 |
| 44362324881 | 38.8 | 48.1 | 66.5 | 50.7 | 56.6 | 96.5 | 61.4 | 68.2 | 58.1 | 48.8 |

Cell 6 was the heaviest in four of five: `s2_the_field_story_at_512`, `su3_a_restart_redraws_nothing_at_512`,
`su5_an_unloaded_yield_lapses_after_28_ic_days`, `ap4a_a_leader_orders…`, `mw14a…_at_512` and `ap8…` hash
there, about 22 of its minutes. The rest is spread: any cell swings 15-30 minutes between runs as tests land.

The fix replaces `--partition hash:N/10` with a plan: `scripts/tacsim-cells.py plan` reads `cargo nextest
list` and places every test longest first (LPT) onto the least-loaded cell, by the manifest's `weight:` lines
(the 113 tests whose mean is at least 60 s) and `default-weight:` (9.8 s, the mean of the rest), with cell 1's
harness and cell 2's cost_lane preloaded. Each cell runs its share as one nextest filterset. Replayed on
the five runs (weights from four, times from the held-out fifth), the heaviest cell was 1.04-1.10× the
median at the 60 s threshold, against 1.23-1.87× under the hash. The threshold barely matters (20 s:
1.03-1.13×; 30 s: 1.04-1.09×), so the manifest keeps the short list.

Weights go stale as tests change; `scripts/tacsim-cells.py weights <junit dirs>` regenerates them, and
`ci_tacsim_shards` fails on a weight for a test that no longer exists.

## TS.10 (2026-11-18) — the clock seam

Doc 43 §7's TS.10 row, designed in [43c-the-clock-seam.md](43c-the-clock-seam.md), which this cut brings to
Draft 0.2: every anchor and count re-read at main `1e9b4117`, the corrections made in place and listed in its §8.
The owner asked for it to be built on 2026-11-18.

**The owner's questions, at the lead's defaults** (43c §7; not ruled, flagged here for the owner):
- **Q-43c-1, where `TimeSource` lives:** `mush-config`, beside `unix_now_s`. That takes no new dependency:
  `mush-config` already depends on `tokio`.
- **Q-43c-2, TS.10c's scope:** in-process memory-store rigs only. Never Redis, a child process, the gateway
  generations, the bot's SSH stories or `web_e2e`.
- **Q-43c-3:** moot. The domain binaries landed on 2026-11-05.

**The gate** (the owner's, agreed 2026-11-18: "keep the gate, agreed"): TS.10b, TS.10c and TS.10e are built only
if TS.10a's converted field tests lose at least half their wall time. They lost 63% (below), so the train goes
on. TS.10d (`[tacsim.time].play_rate`) is not built. It waits on the owner's rulings on `PLAY_RATE`
(docs/testing-rate.md §5).

### TS.10a — the field family on the paused clock

No engine change. Every edit is under `crates/tacsim/tests/`.

**What was built.**
- **`field_host_rig::Pace`, the wait loop's beat.** The deadline is a `tokio::time::Instant`. The beat is a
  25-ms tokio sleep, as before.
  - **On a paused clock** the world is stepped one second for each second of virtual time the arena has seen,
    so the world and the arena it hosts keep one clock. Doc 43c §1's 5:1 ratio goes.
  - **On a running clock** it keeps the old rule exactly, one world second every eighth beat. The dispatch
    stories run on a running clock and still need that rule.
  - The ratio lived in the beat count, not the deadline, so a paused runtime alone would have kept it (43c §1,
    Draft 0.2). Which clock is running is read by `clock_is_paused`: a paused clock does not move while the
    thread spins, and a running one does.
- **The loops that use it.** `until_with`, `field_raiders_rig::until`, and the five copies of the loop in
  modules: `crew_exit`, `field_ap1`, `field_carrier`, `field_crash` and `field_fr5`. `field_ap8`, `field_mw7`,
  `field_mw14a` and `field_mw14c` keep their own loops, with the deadline moved to `tokio::time::Instant`.
- **181 of `domain_field`'s 189 two-worker tests** now run on `#[tokio::test(flavor = "current_thread",
  start_paused = true)]`, in 47 modules. The pilot was the two the row named: gc4's wingman journey and
  `field_host`'s one test. The rest followed in one step, since the pilot held.
- **What did not move, eight tests:**
  - m8_world's four dispatch stories and c4_filed_mind_field's two. Their genuine Registry ratings and the
    author service run in real time (43c §3.3). One of c4's is in the default sweep, and its loops wait on that
    same service.
  - Two that lean on the old ratio, below.

**No rig blocks its runtime thread** (43c §3.1). Nothing hung, and nothing needed `spawn_blocking`.

**What leaned on the old clock** (rule 2.3 #7: each investigated, none retuned):
- **`field_mw7`'s two hand-over stories and `field_ap5`'s link-loss story.** Each failed on "at five minutes the
  mind has the unit": the record said handed, and the pilot's seat was still bound.
  - **The cause.** `field_mw7::pass` steps up to about 280 world seconds without awaiting. The arena's inbox is
    bounded. On one thread the arena never ran during the loop, the inbox filled, and the hand-over's
    `offer(Absent)` was refused, so `field::absent`'s `tell_arena` released no seat. On two workers the arena
    drained the inbox alongside.
  - **The fix.** `pass` now yields after each world second, with no tokio time passing.
  - **The wrong guess first.** I first took it for the arena's answer arriving late and added an egress-only
    wait. It failed 6 of 6; the yield passed 4 of 4 stress iterations of all 14 tests, 8 at a time.
- **`ex6_escort` (one story; stayed on two workers until the owner ruled it, 2026-11-19, below).**
  - **What happens.** The field closes before the miner's `recover go`, because the raider's side is out of it,
    so the journey ends with the field.
  - **Why it fails.** The escort watch steps on the port tick (`services::escort::watch`). Its last tick reads
    the escort out of a field that is gone and counts that whole interval far.
  - **The numbers.** At 5:1 that interval is a small share of the journey: 140 of 145 IC seconds near, kept. At
    1:1, which is the production rate, it is 5 of 35, and the escort fails at 85% against the 90% bar. A probe
    on both clocks showed the same closing interval counted far.
  - **The question.** So a field escort can be failed by one port interval at the end, and a field journey
    under about 50 s can never be kept. Whether the closing interval should count is a game question for the
    owner.
- **`field_mw14c`'s CONCENTRATE story (one; stayed on two workers until the owner ruled it, 2026-11-19, below).**
  - **How the answer is made.** The host answers WILCO or NEGATIVE by reading the hull's frame at its visits
    (`field::orders::concentrate`). The intruder is fragile, so the first round kills it.
  - **Why it fails.** At 1:1 the kill can fall between two visits, and a hull that concentrated and fired is
    answered NEGATIVE ("Taken, and the kernel found no row"). At 5:1 the visits caught it standing first.
  - **The question.** Also a game question for the owner. Both tests say so in their doc comments.

**Measured** on conclav.dev, the ci profile, `cargo nextest run -P ci -p tacsim --test domain_field`, 12 test
threads, the box shared with other sessions (load 6-15), base `1e9b4117` against the branch, from JUnit:

| | before | after | ratio |
|---|---:|---:|---:|
| gc4's wingman journey (the pilot) | 206.5 s | 35.8 s | 0.17 |
| `field_host::cf2_…` (the pilot's short test) | 72.7 s | 24.5 s | 0.34 |
| the 181 converted tests, summed | 10,800.5 s | 4,023.4 s | **0.37** |
| `domain_field`'s other 143 tests, summed | 1,901.8 s | 1,875.1 s | 0.99 |
| the whole binary, 324 tests, summed | 12,702.4 s | 5,898.5 s | 0.46 |
| the whole binary, wall at 12 threads | 1,180 s | 508 s | 0.43 |

The slowest converted test is now `field_story::s2_the_field_story_at_512` at 72.5 s (388.3 s before).
`gc0_the_raiders_barge_lifts…` was 352.6 s and is now 48.0 s, and surrender's `su3`/`su5` were 298-299 s and
are now about 42 s.

Per module, the converted tests only (seconds summed):

| module | tests | before | after | ratio |
|---|---:|---:|---:|---:|
| `surrender` | 17 | 2401.1 | 469.2 | 0.20 |
| `field_story` | 3 | 760.0 | 139.6 | 0.18 |
| `field_fr5` | 10 | 645.9 | 239.5 | 0.37 |
| `field_ap6` | 5 | 487.6 | 118.7 | 0.24 |
| `field_mw14c` | 9 | 463.5 | 231.1 | 0.50 |
| `crew_custody` | 12 | 413.8 | 255.4 | 0.62 |
| `field_gc4` | 5 | 413.8 | 123.1 | 0.30 |
| `field_mw14a` | 4 | 377.0 | 127.5 | 0.34 |
| `field_ap7` | 4 | 374.4 | 94.7 | 0.25 |
| `field_mw7` | 7 | 368.6 | 183.6 | 0.50 |
| `field_ap4a` | 3 | 302.3 | 89.2 | 0.30 |
| `field_ap8` | 5 | 245.1 | 136.3 | 0.56 |
| `field_director` | 4 | 238.2 | 87.3 | 0.37 |
| `escort_lease` | 4 | 218.4 | 82.5 | 0.38 |
| `field_raiders` | 3 | 212.7 | 65.5 | 0.31 |
| `field_ap5` | 6 | 198.7 | 131.6 | 0.66 |
| `field_carrier` | 5 | 183.9 | 98.0 | 0.53 |
| `field_mw15` | 2 | 161.3 | 52.4 | 0.33 |
| `field_fr2` | 5 | 160.9 | 105.2 | 0.65 |
| `field_ap11` | 1 | 135.6 | 30.1 | 0.22 |
| `field_rs1` | 2 | 135.0 | 48.5 | 0.36 |
| `field_fr1` | 2 | 121.5 | 44.1 | 0.36 |
| `field_gc3` | 3 | 116.2 | 58.0 | 0.50 |
| `field_boarding` | 3 | 116.0 | 60.1 | 0.52 |
| `field_ap2` | 4 | 113.0 | 79.5 | 0.70 |
| `field_ap1` | 2 | 111.1 | 43.7 | 0.39 |
| `crew_treatment` | 3 | 107.8 | 67.8 | 0.63 |
| `field_gc5` | 2 | 105.7 | 47.0 | 0.44 |
| `field_crash` | 3 | 103.1 | 63.5 | 0.62 |
| `bay_36a` | 14 | 99.0 | 98.0 | 0.99 |
| `field_fr4` | 2 | 92.6 | 42.9 | 0.46 |
| `one_picture` | 2 | 85.9 | 42.0 | 0.49 |
| `field_ap3` | 1 | 84.2 | 23.5 | 0.28 |
| `testimony` | 2 | 79.7 | 49.6 | 0.62 |
| `field_host` | 1 | 72.8 | 24.8 | 0.34 |
| `field_rs2` | 2 | 70.2 | 41.4 | 0.59 |
| `field_gc2` | 2 | 63.1 | 38.4 | 0.61 |
| `crew_exit` | 1 | 62.2 | 28.9 | 0.46 |
| `salvage_close` | 2 | 59.2 | 56.0 | 0.95 |
| `vacated_return_rs4` | 2 | 51.8 | 49.5 | 0.95 |
| `field_co5` | 1 | 38.4 | 21.1 | 0.55 |
| `bay_gc8` | 4 | 32.7 | 31.0 | 0.95 |
| `field_gc6` | 1 | 27.3 | 26.7 | 0.98 |
| `cc7b_berth_rearm` | 3 | 26.9 | 26.3 | 0.98 |
| `field_gc10` | 1 | 24.9 | 19.1 | 0.77 |
| `field_ap4` | 1 | 24.0 | 18.9 | 0.79 |
| `ex0_matter_sinks` | 1 | 13.1 | 12.7 | 0.97 |

What is left in a converted test is CPU: boot, the world's fires and the arena's pulses. The modules near 1.0
(`bay_36a`, `bay_gc8`, `salvage_close`, `cc7b_berth_rearm`, `vacated_return_rs4`, `ex0_matter_sinks`) never
waited on the arena; their cost is the boot and the fires (TS.3b's figure).

**Rule #14, hosted.** Dispatch run 44453889978 at `ec7b9be6`, ten cells, from JUnit, against main's three
green runs 44381619838, 44413835411 and 44436872677:

| | branch | main (three runs) | change |
|---|---:|---|---:|
| `domain_field`, summed | 266.4 min | 248.9, 312.7, 331.4 | −11% on the mean |
| the 181 converted tests, summed | 185.2 min | 203.1, 245.4, 257.9 | −21% on the mean |
| the converted tests over the other binaries' minutes | 0.393 | 0.707, 0.591, 0.573 | −37% on the mean |

- **The third row is the fair one.** The hosted runners were loaded during the branch run: the binaries this
  cut does not touch summed 471 min, against 287-450 on main. That row divides each run's converted tests by
  its own untouched binaries.
- **Why hosted gains less than conclav.dev's 63%.**
  - A converted test no longer sleeps. It runs the world and the arena flat out, so on a hosted cell's two
    vCPUs the tests contend for the CPU their sleeps used to leave idle, and each test's own time stretches.
  - conclav.dev ran 12 threads on 32 cores.
  - The saving is real on both, but on a hosted cell it is bounded by CPU, not by the clock.
- **The cells** came to 37.6 / 79.4 / 91.2 test-minutes (min / median / max), against main's maxima of
  71.4-96.9. They were still placed by the old weights.
- **The weights were refreshed** by PR #670's recipe: the field tests from this run alone, every other test
  from it and main's three runs (stripped of `domain_field`). That gives 191 `weight:` lines, 127 of them
  field tests, and `default-weight: 10.5`. A converted test that waited is now CPU-bound, so more field
  tests than before pass the 60-s threshold on a hosted cell.

**The hosted dispatch run's other jobs** are green except two, both main's own:
- **M7 author and buyer lifecycle.** The CT.0b capture `ct0b/purchase_house.snap` still says "licence" where
  the US-spelling commit `2c13e5db` made the refusal say "license". The same holds at `origin/main`
  `a018dfd4`.
- **M8 MW.21.** Already red on the 2026-11-18 nightly at main `1d08abd5`.

Neither touches anything this cut changed.

### TS.10b — `TimeSource`, the world's clock provider

**What was built.**
- **`mush_config::TimeSource`** (`crates/mush-config/src/time_source.rs`), as 43c §2 drew it.
  - `System` is the OS clock and tokio's sleeps. `Tokio { epoch_unix_ms, base }` derives unix time from
    tokio's clock.
  - It is `Copy`. There is no global and no trait object. Only `tokio_at`, which a test calls, makes the
    `Tokio` arm.
- **`ServerDeps::time`.** It lands in one commit with every literal that names the struct's fields: 48 of
  them, each now `time: mush_config::TimeSource::System`. The literals that spread
  `..ServerDeps::in_memory()` take `System` from it.
- **The world loop's gating reads go through it.** `run` reads `deps.time` once. These go through the
  provider:
  - the per-iteration `set_now`;
  - the outage wake;
  - the timer wake (`sleep_until_unix_ms`);
  - `fire_due`;
  - the shutdown grace, now a `tokio::time::Instant`.
- **Measuring reads stay `std`:** `iteration_start`'s loop lag, `boot_unix`, and the sample throttle.
- **Production behaves exactly as before.** `System` reads `unix_now_s`/`unix_now_ms`. Its instant is
  `tokio::time::Instant::now()`, which on a running runtime is `std`'s.
- **The test.** `ts10b_a_paused_world_fires_an_hours_wait_on_tokio_time` runs a paused world on
  `tokio_at`. An hour's `@wait` does not fire at 3,590 s of tokio time, fires by 3,610, and takes 43 ms of
  wall time. Mutation-checked: on `System` the wait never fires.

**Where it departs from 43c's row** (recorded, not ruled; the owner may want to look):
- **The arena's spawn terms do not carry the provider.**
  - The arena reads no wall clock: its clock is a `tokio::time::Instant` base, which a paused runtime
    already moves (43c §2, "the arena's arithmetic does not change").
  - The three readers 43c named for an arena handle are not arena code. The tape's `Restart` and a
    snapshot's wall stamp carry no OS-clock read today. The bulk executor's worker day
    (`minds/bulk/executor.rs`) is the genuine Registry's real-time UTC budget, real time by design
    (43c §3.3).
  - Carrying the handle would also need a new crate edge. The host seam is `mush-core`'s
    `OffLoopSimulationHost`, `mush-config` depends on `mush-core`, and `tacsim` does not depend on
    `mush-config`.
  - So the arena gets the provider when it first has a reader. TS.10e's lint scans `arena_task*` so the
    arena stays free of OS-clock reads meanwhile.
- **`staff.rs:349` stays on the OS clock.** Its tempban-lift stamp is compared by `IpGuard` against
  its own OS clock (`ipguard.rs`, `apply_tempban_lift_at`), and the gateway shares that guard, so the
  stamp belongs to the guard's clock, not the world's.
- **`staff_ops.rs:316` stays on the OS clock.** It is the economy journal's per-hour rate, which is a
  measurement.
- **`boot_engine`'s first `set_now`** stays too. The loop's first iteration overwrites it from the
  provider.

### TS.10c — the paused rigs and the guard

**The guard.**
- `ObjectStore::holds_process_memory` defaults to false, and `InMemoryStore` says true. A forwarding test
  double opts in by asking its inner store.
- `spawn_persist_with_gateway_mode` asserts that `TimeSource::Tokio` only runs over a store that says yes.
  Every in-process composition comes through that function.
- 43c §5 first put the guard at boot (`boot.rs`, `serve_classified`). It sits at the spawn instead, for two
  reasons. The store is `dyn ObjectStore` at boot. And both production boots build
  `ServerDeps::production`, which is always `System`, so the spawn is the one place a `Tokio` arm can
  reach.
- **Tested both ways.** `Tokio` over a store that does not claim process memory is refused before any task
  starts, and `System` over the same store spawns.
- `assert!` is not one of the panic budget's needles. A `panic!` in the first test's gateway closure did
  count (`retry.rs` is not test-fenced), and it was replaced.

**What moved to the paused clock** (Q-43c-2 at the lead's default: in-process memory-store rigs only):
- **`mush-server::world_task::preflight_retry_bounds`, 4 tests.** It moved with no engine change, because
  the coordinator's retry window already reads tokio's clock. The slow store's 2-s answers are tokio sleeps.
  The window's floor-and-ceiling assertions now measure tokio time.
  - Main's hosted run summed 10.3 s; locally the module takes 0.19 s.

**What did not move, measured first** (main run 44436872677's hosted JUnit, summed):

| suite | summed | why not |
|---|---:|---|
| mush-server `world_task::submission_disposition::tests::player_market*` | 637 s | CPU. One test on the paused clock took 8.55 s against 8.21 s on two workers: the cost is the production launch's boot, not a wait |
| mush-server `commerce_composition`, `split_boundary` | 5,020 s | `TestWorld` (43c §4: their cost is work) |
| mush-server `hostile_lines` | 10.8 s | `TestWorld` fuzz |
| mush-server's other src `world_task` tests | 31 s | under 0.5 s each |
| mushd `sim_lenses`, and `world_journeys`' `modules_e2e`, `onboarding_e2e`, `ssh_e2e` and `web_e2e` | 1,061.9 s | real SSH or web clients (43c §3.3) |
| mushd `world_journeys`' network-free files (`admin`, `legal_action_web`, `mind_market_web`, `web_bridge`) | under 0.5 s each | nothing to win |

So outside `domain_field` the seam pays only where a test waits on a timer it owns. In the in-process
memory-store suites that is the one module above. TS.10c's value is the guard, plus the seam being in place
for the next test that waits.

### TS.10e — the contract lint

**The gate.** `mush-contract-tests::engine_contracts::world_clock_reads` runs on TS.8's scanner.
- **The rule.** No `unix_now_s()`, `unix_now_ms()` or `SystemTime::now()` call may appear in the production
  code of a `world_task*` or `arena_task*` file under a crate's `src/`. That is 9 files today: seven in
  mush-server and two in tacsim.
- **The allow-list.** Eight rows, each with its count and reason:
  - displays: `@ps`, `@stats`, the uptime;
  - the store health's last-write stamp;
  - the economy journal's rate;
  - `boot_engine`'s first `set_now`;
  - three account-side records that the gateway or the Discord bridge compares on the OS clock (an `@sshkey`
    stamp, a link code's expiry, a ban stamp).
- **Counts are exact.** A new read fails the gate, and a removed one asks for its row to go.
- **The arena reads none.** It is scanned so that stays true.
- **Mutation-checked.** Putting `fire_due` back on `unix_now_ms()` fails the gate with the file, the line
  and the count.

### What is left of TS.10

- **More paused rigs** only where a test waits on a timer it owns (TS.10c's table).
- **The arena's handle** (TS.10b, above) waits on its first reader.
- **TS.10d** waits on the owner (above).
- ~~Two owner questions from this cut~~ **Ruled 2026-11-19** ("Yes, fix both"): the escort watch no longer
  counts a field journey's closing interval (doc 48's cut notes, EX.6, "The closing interval"). A `CONCENTRATE`
  whose row reads destroyed at the first visit that sees it stand is answered WILCO (doc 44's cut notes,
  MW.14c, "The answer to a kill between visits"). `ex6_escort` and `field_mw14c`'s story 3 are on the paused
  clock, as each rule's acceptance at the production rate. With them, 183 of `domain_field`'s 189 two-worker
  tests are paused; the six left are m8_world's and c4_filed's dispatch stories.
