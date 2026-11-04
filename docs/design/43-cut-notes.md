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
