# 51. Flaky tests, removed by class — cut notes

One section a unit of [doc 51](51-flaky-tests-by-class.md) §5. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

## FL.1

The ratchet and the story kit. Built 2026-11-22 on `e468c391b` (main with #771 merged). It adds one
dev-only crate and one contract gate and moves mushd's stories onto the kit. It changes no product
code and converts no wait.

**Survey.** Doc 51's row says `wait_until`, `STEP_TIMEOUT`, `LOAD_FACTOR` and `StoryBudget` are
"moved in from mushd's rig". Two of the four were there:

- `wait_until` and `STEP_TIMEOUT` (60 s) were in `crates/mushd/tests/support/mod.rs`.
- `LOAD_FACTOR` was nowhere in the tree. `StoryBudget` was not either; its model is the hand-built
  18-minute budget in one story of `crates/mush-bot/tests/mining_cohort.rs` (#764). Both are new
  code in this unit.
- mushd's stories held three more step limits of their own: `ssh_e2e.rs` and `server.rs` (60 s
  each, the rig's value written again) and `world_fault.rs` (120 s).

**Built.**

- **`crates/mush-storykit`**, `publish = false`, depending on tokio and nothing else:
  - `wait_until(bound, label, check)`: mushd's loop, moved as it was, with the same failure text.
  - `LOAD_FACTOR = 2`, `STEP_UNLOADED = 30 s`, and `STEP_TIMEOUT = loaded(STEP_UNLOADED)`, which
    is 60 s, the value mushd had. `loaded(span)` applies the factor to any other span.
  - `StoryBudget::new(minutes)` refuses zero and anything over `NEXTEST_TERMINATE` (20 min) less
    `TERMINATE_MARGIN` (2 min). `left()` and `limit(want)` give a wait its bound from what is
    left. `wait_until(want, label, check)` waits under that bound and says in its failure when
    the budget, not the wait's own span, cut it short. `hold(story)` is the outermost limit,
    thirty seconds after the budget's end, so a wait that drew from the budget fails first and
    names what it waited for.
- **mushd's stories on the kit.** `tests/support/mod.rs` re-exports the kit's `wait_until` and
  `STEP_TIMEOUT` and defines neither. `ssh_e2e.rs` and `server.rs` import the rig's
  `STEP_TIMEOUT`. `world_fault.rs` keeps its 120 s as `CHILD_STEP_TIMEOUT`, twice the rig's.
  No bound changed value.
- **Dev-only, held by a gate that already existed.** `mush-storykit` is added to the panic
  budget's `EXCLUDED_CRATES`. That exempts its panics from the production budget, and
  `no_production_crate_links_an_excluded_crate` then refuses any `[dependencies]` or
  `[build-dependencies]` edge to it. mushd and mush-contract-tests name it under
  `[dev-dependencies]`. mush-bot does not name it at all.
- **`wait_budget`**, `crates/mush-contract-tests/tests/ci_policy/wait_budget.rs`, with one
  `wait_budget/<crate>.txt` a crate. A row is `<counter> <budget> <low-water> <path>`. The
  counters are `wait` (the six needles of doc 51 §2.1) and `clock` (`unix_now_s(`, `unix_now_ms(`,
  `SystemTime::now(`); `span` is read by the files and counts nothing yet. Rig files and sites
  inside a `start_paused` test are not counted. The gate is equality, as the fiat budget's is:
  - a count above its budget fails, and the message names the kit (for `clock`, a rig that boots
    at a fixed instant);
  - a count below its budget fails and asks for the budget to be lowered to match;
  - `WAIT_BUDGET_BLESS=1` lowers freely and raises only with
    `WAIT_BUDGET_RAISE="<counter> <path>: <reason>"`, which is written into the file as a
    `# raised` line. A new file is a raise from zero.
  - A second test holds the kit's `NEXTEST_TERMINATE` to `.config/nextest.toml`: the `ci`
    profile's terminate is exactly it, and no profile terminates sooner.
- **`scripts/wait_audit.py --budget`** prints what the gate counts, one row a file, then the
  totals by crate.

**The counts.** `python scripts/wait_audit.py`, before (main at `e468c391b`) and after:

| | Before | After |
|---|---:|---:|
| Wait sites under `crates/*/tests/` | 617 in 104 files | 615 in 104 files |
| On the wall clock, in rig files | 47 | 45 |
| On the wall clock, in story files | 526 | 526 |
| mushd, rig and story | 9 and 124 | 7 and 124 |
| Test files scanned | 635 | 636 |

The two sites that left are `wait_until`'s `timeout` and `sleep`. They are in the kit's `src/`
now, where the primitives belong and the census does not look. The new file scanned is the gate.

The gate's baseline, which is `python scripts/wait_audit.py --budget` row for row (127 rows,
compared with `diff`):

| Crate | Raw waits | Wall-clock reads |
|---|---:|---:|
| mush-bot | 78 | 28 |
| mush-contract-tests | 3 | 0 |
| mush-gateway | 86 | 2 |
| mush-llm | 4 | 0 |
| mush-net-ssh | 25 | 1 |
| mush-net-web | 33 | 3 |
| mush-server | 68 | 13 |
| mush-sshdoorway | 7 | 0 |
| mush-store | 3 | 2 |
| mush-worldgen | 1 | 0 |
| mushd | 124 | 5 |
| tacsim | 94 | 0 |
| **total** | **526** | **54** |

The 526 is doc 51 §3.9's baseline. The 54 is the figure §3.9 left to this unit: a plain `grep`
finds 63 reads in 44 files, one of them a doc comment
(`engine_contracts/world_clock_reads.rs:10`). The lexer finds 62 in 43 files. Eight are in rig
files, and none is in a paused test.

**Tests.** All on conclav.dev, in a throwaway clone of `e468c391b` with this unit's patch, at 16
build jobs and 8 test threads.

- `mush-storykit`: 13 unit tests on the paused clock and one doc test.
- `mushd`, every binary (`cargo nextest run -p mushd`): 261 run, 261 passed, 1 skipped, none
  passed on a retry, 251 s. That is the proof the stories run on the kit's copies.
- `mush-contract-tests`, whole crate with no fail-fast: 214 passed, 8 skipped (the `#[ignore]`d
  scale harness). The first run failed two `restriction_lints` tests, because a new crate needs a
  row in `MEASURED`. The kit's row is `[0, 0, 0, 2, 0, 0]`: its root denies five of the six lints,
  and its two `panic!` sites are the failures of `wait_until` and `StoryBudget::hold`.
- `cargo fmt --all --check`; clippy with `-D warnings` over the three crates' every target;
  rustdoc over the workspace with `--document-private-items`, and over the kit without.
- Dependency edges: `cargo tree -p mush-bot --edges normal,build --target all` holds one RustMUSH
  package, mush-bot. `cargo tree --workspace --edges normal,build -i mush-storykit` prints the
  kit alone; on every edge kind its dependents are mushd and mush-contract-tests, each under
  `[dev-dependencies]`.
- A second bless at the baseline changes no file.

**The ratchet, proven both ways.** Three edits to mushd's story files in the clone, each made and
taken out by hand, with the gate run between them:

1. A raw `tokio::time::sleep` and a `SystemTime::now()` added to `sim_lenses/sim_lobby.rs`. The
   gate fails with:
   ```text
   mushd/tests/sim_lenses/sim_lobby.rs: 2 raw waits against a budget of 1. Wait through the story kit
   (`mush_storykit::wait_until` under `STEP_TIMEOUT`, or a `StoryBudget`), not through a raw sleep or
   timeout in a story. Or raise the budget with a written reason.
   mushd/tests/sim_lenses/sim_lobby.rs: 1 wall-clock read against a budget of 0. A story does not place
   itself by the wall clock (doc 51 rule T): boot the rig at a fixed instant. Or raise the budget with a
   written reason.
   ```
2. That edit removed, and one `tokio::time::sleep` line deleted from `sim_lenses/sim_range.rs`.
   The gate fails with:
   ```text
   mushd/tests/sim_lenses/sim_range.rs: 1 raw wait against a budget of 2. The count fell: lower the
   budget to match (re-bless), and it stays down.
   ```
3. The line put back. `git diff` on the two files is empty and the gate passes.

**Divergences.** Where this differs from doc 51, or from the brief the unit was cut with:

- **The budget is built like the fiat budget, not the panic budget.** The brief said to model it
  on the panic budget, which holds one number a crate. Doc 51 §3.9 says per-file rows with a
  low-water mark, equality as the test and a bless variable, which is the fiat budget's shape.
  The doc is the spec. The panic budget supplied the dev-only gate instead.
- **A row names its counter.** §3.9 gives a row as `<budget> <low-water> <path>`. With two
  counters now and a third to come, a row is `<counter> <budget> <low-water> <path>`.
- **A raise needs a written reason, of any wording.** The fiat budget demands a `§`. There is
  no as-built section for a raw wait to cite, so the gate asks only that the reason is not empty.
  A reviewer reads it in the diff.
- **`STEP_TIMEOUT` already carries the load factor.** §3.7 says a bound is "the rig's
  `STEP_TIMEOUT` times the load factor". Read literally that doubles every bound in mushd to
  120 s. Instead `STEP_TIMEOUT` is `STEP_UNLOADED` (30 s) under `LOAD_FACTOR` (2): one place
  answers a slow runner, and no bound moved.
- **The kit depends on tokio alone.** §3.8 says it depends on tacsim and russh. It will, when
  `Figures` (FL.3) and `Term` (FL.7, FL.8) arrive; an unused dependency now would only lengthen
  every story crate's build.
- **The census script changed.** Doc 51 calls it read-only, and it still reads only. Two edits:
  the `--budget` mode, and its `.timeout(` rule. §2.1 says a method call `.timeout(` is not
  counted. The script skipped one only when the dot followed a non-identifier character (a
  builder chain broken across lines) and would have counted `client.timeout(…)`. The tree holds
  no such site, so no count moved. The script and the gate now both follow §2.1.
- **A definition counts.** A story file that defines a helper named `unix_now_ms` or `sleep`
  counts one site for the definition, in the script and in the gate alike. One exists, in a rig
  (`crates/mush-bot/tests/support/mine.rs:443`), where it is not counted.
- **"Paused" is read from the function a site is written in.** That is doc 51 §2.1's second
  known error, kept on purpose so that the gate and the script agree: a helper that only paused
  tests call is still counted.

**Not built.**

- The literal-span counter (a stub, as the row says; FL.9 baselines it).
- `one_reader` (§3.9), which FL.7 and FL.8 turn on a crate at a time.
- The `--src` scope for mush-server (FL.6).
- A `StoryBudget` for a story on a longer terminate. `fr7_round` runs under an override of 100
  minutes, and `StoryBudget::new` knows only the profile's 20. FL.5, which puts a budget in every
  `mining_cohort` story, needs a constructor that takes the override.
- No story in any crate was converted, mushd's included: mushd's 124 raw waits are where they
  were. The soak rate and the summed minutes that §5 asks of a unit's note do not apply to a
  unit that changes no wait; FL.2's soak did not exist when this was built.

## FL.2

Not built.
