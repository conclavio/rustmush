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
- **The one red.** B's single failure, `field_gc4::gc4_the_wingman_drags…` (`field_gc4.rs:317`), also failed on
  TS.12's branch, which has no profile change. It is a real-time race in that journey, not an effect of this
  cut.
