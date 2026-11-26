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

The detection loop. Built 2026-11-23 on `e468c391b` (main with #771 merged), beside FL.1 (#774), and rebased onto it
(`867e2d530`) before the pull request. It
adds a workflow, a nextest profile, two scripts, a list and a contract gate. It changes no product
code and converts no story.

**Survey.**

- **The stories find their server binary by a compile-time path.** `env!("CARGO_BIN_EXE_mushd")`
  (`crates/mushd/tests/support/mod.rs:823`) and its twins in mush-gateway, mush-server and
  mush-bot point into the build's own target directory. A nextest archive extracted somewhere
  else still runs the server from the build directory, so on one box an archive isolates
  nothing. That decided how a cell runs (Divergences, 1).
- **Each package's CI cell has its own feature set.** mush-server's tests turn on
  `mush-sim/debug-checks` and `mush-housing/debug-checks`; mushd's and mush-gateway's do not; the
  bot has its own (`gate.yml` already runs it apart for that reason). One build over the four
  packages would unify them, so the soak builds a package at a time, as CI does.
- **Nothing in the nine binaries binds a fixed port or kills a process by name**, so six cells
  can run one test at once. `authoritative_process_kill` and `durability_e2e` only do anything
  under `REDIS_URL`, which a cell does not have.
- **The runner's user cannot ask for a cgroup.** `ghrunner` has no sudo and no user systemd
  manager (`loginctl`: not logged in or lingering), and the runner's unit has `Delegate=no`. So
  a cell cannot have the `MemoryMax` doc 51 §4.1 offers; `taskset` pins cores only. The unit's
  `MemoryHigh` (56 GiB on conclav.dev) covers the whole job.
- **nextest checks a filter's binary ids against the workspace when it loads its
  configuration.** A `binary_id()` that names no target is a hard error on every run, so a row
  naming a deleted binary cannot reach CI. A deleted test or module is not checked by nextest;
  the gate checks those.
- **nextest 0.9.148 has stress flags** (`--stress-count`, `--stress-duration`). Tried in a toy
  crate: an iteration in flight at the deadline is finished, not cut, and each iteration is a
  `<testsuite name="…@stress-N">` in one report written at the end of the whole run.
- **GitHub lists every attempt's artifacts for a run.** A rerun shard has two
  `nextest-junit-<shard>` artifacts; the one made while attempt 1's job ran is attempt 1's. That
  is how the report sees "failed, then passed at the same head" without reading a log.

**Built.**

- **`crates/mush-contract-tests/flaky-known.toml`**, 36 rows. Sixteen are the tests of the rows
  of `docs/flaky-tests.md` that are open, covered or on watch (rows 5, 6, 10, 12, 13, 14, 15, 16
  and 17), each with doc 51's class and its row. Twenty are the eighteen entries of the old
  filter that match no row, carried over as `class = "unclassed"` with a `carried` line; two of
  them (`arena_story`, `relay_bridge`) are narrowed to the one test a cut note names, and the two
  substring entries (`~e7c6_b2`, `~what_still_waits`) are written out as their four tests. Every
  row was listed 2026-11-23 and expires 2026-12-07.
- **The retry filter is generated.** `.config/nextest.toml` holds one retry override, between
  `# BEGIN GENERATED flaky-known` and `# END GENERATED flaky-known`: one term a row.
  `FLAKY_KNOWN_BLESS=1 cargo test -p mush-contract-tests --test ci_policy flaky_known::` rewrites
  it. The `load-sensitive` group stays, as a load limit of two at a time, with no retry. The
  `redis` group and the run-alone override are untouched.
- **The gate**, `tests/ci_policy/flaky_known.rs` over `src/flaky_known.rs`. It fails on a row
  past its `expires`; on an `expires` more than fourteen days ahead of today; on a row with no
  class, or a classed row with no row of the record, or a row of the record that does not name
  the test; on a classed row wider than one test; on a test, module or binary the tree does not
  hold (it follows the `mod` and `#[path]` declarations from the binary's root file and looks for
  the function under a test attribute); on a generated block that differs from the list; and on a
  `retries` key anywhere else in the file, in any profile (`retries = 0` is allowed).
- **The `soak` profile**: four threads, no fail-fast, the `ci` terminate (20 minutes), JUnit, and
  an `all()` override with `retries = 0`, which outranks the generated retry because a profile's
  own overrides are read first. Checked in a toy crate: a listed failing test is tried twice
  under `default` and once under `soak`.
- **`scripts/soak.sh`**: the whole soak, and the same script by hand. A wait for a quiet box, one
  build a package, then the cells; a cell runs
  `cargo nextest run --binaries-metadata <package> --cargo-metadata <all> --workspace-remap <its copy of the tree> -P soak --test-threads 4`
  for each package in turn until the window closes, and keeps one JUnit report an invocation.
- **`.github/workflows/soak.yml`**: `workflow_dispatch` only since the follow-up (Divergences, 12;
  as merged it also had a `schedule` at 07:17 UTC, Divergences, 11), on
  `[self-hosted, linux, conclav-dev]`, with a concurrency group of its own, a target directory of
  its own beside the gate's, the report in the step summary and the `soak-junit` artifact.
- **`scripts/flake_report.py`**: the soak's reports and CI's `nextest-junit-*` artifacts into one
  table, with `--check` and `--write`, and `scripts/test_flake_report.py` beside it. ci.yml
  changes in two places: the `nightly-report` job runs the report over the last day's runs
  (doc 51 §4.2), and the `provisioning` job runs the script's tests.
- **`docs/flaky-observed.md`**, generated, from the fourteen days of CI before this unit.

**Tests.**

All on 2026-11-23, at `e468c391b` with this unit's patch. "The box" is conclav.dev, by hand as
`ghrunner`, in a clone with its own target directory.

**The gate.** `flaky_known` has ten tests; two read the real files and eight doctor a copy. On
the real files, by hand:

| Edit | The gate says |
|---|---|
| row 6's `expires` set to 2026-11-22 | `a_captain_and_its_escort_…: EXPIRED on 2026-11-22 (today is 2026-11-23). Fix the test and delete the row, or, on evidence …, set a new expires no later than 2026-12-07 in a diff the owner sees` |
| row 14's test renamed to `web_e2e::spa_and_healthz_are_served` | `names nothing in the tree: no test spa_and_healthz_are_served in crates/mushd/tests/world_journeys/web_e2e.rs`, and `row 14 of docs/flaky-tests.md does not name spa_and_healthz_are_served`, and the block no longer matches the list |
| `retries = 1` put back on the `load-sensitive` override | `[[profile.default.overrides]] #2 asks for a retry outside the generated block` |

The workspace guards on the box: `cargo fmt --check`, `cargo clippy -p mush-contract-tests
--all-targets -- -D warnings`, rustdoc in both modes, and `ci_policy` and `engine_contracts` whole
(183 passed, 8 ignored; 189 after the rebase onto FL.1, whose `wait_budget` counts no new wait or
clock read in this unit's files).

**The retry, before and after.** `cargo nextest list -E '<filter>'` over the six packages the two
filters name: 105 tests were retried, 77 are. 37 lose theirs (the tests of the FIXED rows 3, 4 and
11, and 34 module-mates of rows 12 and 13 with no failure on record), 9 gain one (rows 5, 6, 10,
14, 15, 16, 17), 68 keep theirs. The pull request lists them by name.

**The report, over real artifacts.** `--ci-days 14` read 738 `ci.yml` runs, 105 of them with
reports, in about 650 API calls. It found by itself rows 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 16
and 18 of the hand-written record. The red-main windows of the record's last section
(`in8_mass_scrap`, `house_sweep_rs6c`, `field_raiders::cf6`, `absent_census`) came out as
"branches (to read)", which is why that reason alone fails nothing. Run 45057892782, whose bot
cell failed on attempt 1 and passed on attempt 2, reads `same head x1`: the two attempts'
artifacts were told apart. `docs/flaky-observed.md` is that run's table. The script's own tests
are `scripts/test_flake_report.py` (15, on synthetic reports), which the `provisioning` job runs.

**The soak, by hand on the box**, through `scripts/soak.sh`, the command the workflow's Soak step
runs:

- *One cell, one package* (`SOAK_CELLS=1 SOAK_PASSES=1 SOAK_PACKAGES=mush-gateway`): 47 tests, 47
  passed, 192 s; the report read; no process and no temporary file left.
- *Six cells, one round* (`SOAK_CELLS=6 SOAK_PASSES=1`), with a working directory as long as the
  runner's own, so a Unix socket path is as long as it will be in the job:

  | Package | Tests | Seconds an invocation, six cells | Result |
  |---|---:|---|---|
  | mushd | 320 | 278 to 321 | 6 of 6 passed |
  | mush-gateway | 47 | 132 to 193 | 6 of 6 passed |
  | mush-server | 248 | 448 to 457 | 6 of 6 passed |
  | mush-bot | 39 | 800 to 814 | 6 of 6 passed |

  24 reports, 3,924 test runs, none failed, so the first soak names no flake. **A round is about
  29 minutes a cell** (1,690 to 1,750 s), not the 40 doc 51 §4.1 guessed, so a 180-minute window
  is about six rounds a cell: **about 37 passes a night**, not 24. The build of the four packages
  took 5 min 20 s on a target directory that already held two of them. Least free memory 52.5 GB of 62; highest load 22.5,
  mean 10.6, with another session's builds on the box at the time. The stories wait on clocks
  more than they compute, so the cap of six is set by doc 51 and not by the box; it is not
  raised here.
- *The yield* (`SOAK_MAX_START_LOAD=1 SOAK_WAIT_MINUTES=0`): a warning, exit 0, `"state":
  "yielded"`, no pass.
- *A stop in mid-run* (SIGTERM to the script): it exited in 0.6 s, `"state": "stopped"`, and no
  process carried the run's mark afterwards. While the six cells ran, all 39 of their processes
  (nextest, the test binaries, the `mush-bot` children) carried it.
- *The profile*, in a toy crate: a listed failing test is tried twice under `default` and once
  under `soak`.

**Not run:** the workflow file itself, and the `nightly-report` step inside Actions (Not built,
below).

**Divergences.**

1. **No archive.** Doc 51 §4.1 says one build, archived. The build is one build, and the cells
   run it through nextest's reuse-build files (`--binaries-metadata`, `--cargo-metadata`) with no
   tar file, for the Survey's first reason, and because an archive would be extracted again on
   every invocation of every cell.
2. **Four builds, not one.** A package at a time, for the Survey's second reason. A pass is
   therefore four invocations, one a package, and a report is one package's.
3. **nextest's stress flags are not used.** Doc 51 §7 says to take them if they are there. They
   are there and they work, but one invocation is one build (so one feature set) and one report
   written at the end, so a cell stopped by the watchdog or a cancelled job would lose every
   pass it had finished. The cell's loop is the script's.
4. **No hard memory limit a cell.** The Survey's fourth point. The bound is the cell count, a
   quiet-box check before the start, and a watchdog that stops every cell when free memory falls
   under 6 GB.
5. **`STORY_PHASE=random` is exported and nothing reads it yet.** The rigs learn the variable in
   FL.4, and the match seed's knob is FL.3's. Until then a soak pass differs from a CI run only in
   its load and its count.
6. **The brief said `flaky-known.toml`; the doc says where**, and the file is at the doc's path.
   The brief seeded the list from the record's open rows; doc 51 §4.3 point 5 also carries the
   eighteen unmatched filter entries over, and they are carried.
7. **Row 1 is not listed.** Its row is FIXED and its WATCH is a second window that never fired
   in 192 runs. It was a product ordering fault, which is the kind doc 51 §4.3 says a retry
   hides. Row 5 is FIXED too and is listed, for its second failure, which is on watch.
8. **`--check` does not fail on "two branches" alone.** Doc 51 §4.2 counts a test that failed on
   two unrelated branches. The script cannot tell unrelated branches from two stacked ones with
   one bug, or from a red main, so that row is printed, marked "to read", and fails nothing. A
   pass on retry and a failure and a pass at one head are findings and do fail it.
9. **`--check` also fails on a soak test that failed every pass.** That is a broken test on
   main, not a flake; a green soak beside it would be wrong.
10. **By default the report downloads only the shards with a failed job**, from runs that failed
    or were rerun, to stay inside GitHub's rate limit (a day's runs are about 1,100 artifacts, and
    the job's token has 1,000 requests an hour). It therefore does not see a pass on retry in a
    green run; `--all-shards` does. Since the retry is now the list's alone, such a pass can only
    be a listed test.
11. **The soak starts at 07:17 UTC, not doc 51's 01:17.** The owner moved it on 2026-11-23, on the
    pull request: 01:17 UTC is 5:17 pm Pacific, his working day, and 07:17 is 11:17 pm (standard
    time; the cron is UTC), so the soak holds the box from about 07:17 to 11:00 UTC, about 11 pm
    to 3 am his time. What else is scheduled then, checked that day: ci.yml's 04:17 nightly and
    Monday's 06:17 CodeQL run are on hosted runners and share nothing with the box; `gate.yml` is
    dispatch-only, and a dispatch pinned to `conclav-dev` in those hours waits for the runner. On
    the box itself there is no cron daemon and no crontab. `apt-daily-upgrade.timer` fires
    between 06:00 and 07:00 UTC, so 17 minutes or more before the soak and never inside it; an
    upgrade that is still running at 07:17 meets the quiet-box wait, and one that restarted the
    runner's service would end the job, which has not been seen. `apt-daily.timer` (a package-list
    refresh at a random hour) and `motd-news.timer` can fall inside the window and are light.
    `fstrim`, `e2scrub_all`, `dpkg-db-backup` and `systemd-tmpfiles-clean` all run outside it.
12. **Timers on conclav.dev keep the clock, not GitHub's `schedule`** (a follow-up pull request,
    2026-11-23, after FL.2 merged). The soak did not run on its first night: the workflow was on
    main from 04:12 UTC with `17 7 * * *`, the runner was online and idle, and at 14:48 UTC it had
    no run. `gh run list --event schedule` shows why: GitHub starts this repository's scheduled
    runs seven to ten and a half hours late. The 04:17 UTC nightly started between 11:28 and
    14:38 UTC on every day from 2026-11-09 to 2026-11-23 (and between 09:52 and 12:34 UTC in the
    two weeks before), and Monday's 06:17 CodeQL run at 15:45 to 16:17 UTC. A soak that late
    holds the box from the owner's morning into his afternoon. Asked how the run should be
    started on time, the owner ruled "Timer on conclav.dev", and then widened it to every
    scheduled job, spaced by their runtimes. What the follow-up built:
    - **Four timers** in `deploy/vps-runner/`, in `America/Los_Angeles` so they follow daylight
      saving, all `Persistent=false` (a firing the box was off for is missed, not caught up):
      the soak every day at 23:17, the CI nightly Monday to Saturday at 05:00, the CI weekly on
      Sunday at 05:00, CodeQL on Monday at 13:30. `docs/ops.md` ("The clock is on conclav.dev")
      has the measured runtimes and the arithmetic: the time kept for a job is one and a half
      times its longest scheduled run of the last 30 days (the soak, with no run yet, keeps its
      330-minute timeout), and no two kept times overlap.
    - **It does not fit the owner's night.** The kept times in a row are 13 h 57 min (16 h 32
      min on Mondays) against eight hours. The headroom was not shrunk: the soak, the one job
      that holds his box, is in the night, and the hosted jobs follow it into the day. Only the
      nightly-to-CodeQL gap protects a shared resource (hosted-runner concurrency); the gaps
      either side of the soak are tidiness, because the box and the hosted runners share
      nothing. `docs/ops.md` gives the alternative that ends everything by 07:00.
    - **`rustmush-dispatch@.service` and `workflow-dispatch.sh`**: one `workflow_dispatch` at
      `main` through GitHub's API with curl (`gh` is not on the box), for the job the instance
      names. The service runs as a user of its own; systemd reads the token as root from
      `/etc/rustmush/dispatch.env` (one line, `DISPATCH_TOKEN=`). The token goes to curl on
      standard input. A missing file, a file others can read, an empty or malformed value and
      each refusal from GitHub (401, 403, 404, 422) is one line that says what to do. The call
      is sent again, three times at most, only when it never reached GitHub; a second dispatch
      after one that arrived would start the job twice.
    - **No workflow has a `schedule`.** `soak.yml` and `codeql.yml` lose theirs. `ci.yml` loses
      both crons and gains a `slot` input: `by-hand` (the default: everything a dispatch always
      ran, at the fast pulse), `nightly` (the Monday-to-Saturday cron's set, the M7 stories at
      250 ms) and `weekly` (Sunday's: that and the six weekly jobs). A job's tier is still read
      from its `if:`: the nightly jobs' is `github.event_name == 'workflow_dispatch'`, the
      weekly jobs' adds `&& inputs.slot != 'nightly'`. `ci_gated_test_inventory`,
      `nightly_paths`, `test-tiers.toml` and `.github/nightly-paths.txt` read the new strings,
      and the first refuses an `if:` that names a schedule. `scripts/flake_report.py` never
      filtered by event and is unchanged.
    - `deploy/vps-runner/dispatch-timers-install.sh` puts the files on the box and enables the
      timers; `scripts/tests/workflow-dispatch.sh` drives the call against a stub curl, lays
      the week out from the timer files and fails on an overlap, and fails if any workflow has
      a `schedule`. ci.yml's provisioning job runs it.
    - **The token is the owner's** to create and place (`docs/ops.md`): a fine-grained personal
      access token for this one repository with Actions read and write, one for all four jobs.
      Nobody else creates, reads or copies it. While it is not there each timer fires and its
      service fails with a line that names the missing file.
    - **Mail.** A scheduled run's mail goes to the account that last changed the cron line; a
      dispatched run's goes to the account that dispatched it, which is the token's owner. So
      a red soak, nightly, weekly or CodeQL run still mails the owner, subject to his
      notification settings for Actions.
    - **The trade.** If conclav.dev is down, or the token has expired, nothing clock-run runs,
      and nobody is mailed, because no run exists. `docs/ops.md` has the commands that show it.
      A small hosted workflow on GitHub's own schedule (where lateness does not matter) that
      fails when the newest nightly is too old would watch it; it is not built.
    - **In summer** the soak starts at 06:17 UTC, inside the hour `apt-daily-upgrade.timer` fires
      in (item 11). It installs security updates only, needrestart is not on the box and no
      automatic reboot is configured, so nothing restarts the runner after one; the quiet-box
      wait covers the load.

**Not built.**

- **The first week's soak.** Doc 51 §5 asks for it in this note. It cannot exist until
  the workflow is on main and has run seven nights. Whoever closes it adds here: the nights that
  ran, yielded or were stopped (`soak.json`'s `state`); the passes a night of each package; every
  test the report named, with its rate (failed passes over passes) and whether it was on the
  list; least free memory and highest load a night (`box.tsv`); and any test that failed for a
  reason the soak made itself (two cells at once, the box), which is a fault in `soak.sh` and not
  a flake. The same week decides the carried rows: one with no failure in about 250 passes is
  deleted on 2026-12-07, not renewed.
- **A dispatch of `soak.yml` itself.** A workflow can be dispatched only once it is on the default
  branch, so the first run of the workflow file is after the merge. The Soak step's script ran by
  hand on the box (Tests); the checkout, the report step and the upload did not. Still owed on
  2026-11-23: the workflow has never run (Divergences, 12). Its first run is the first night
  after the owner places the token, or his short run by hand
  (`/usr/local/lib/rustmush/workflow-dispatch.sh soak 20 2`).
- **The `nightly-report` step** runs on the schedule and on a dispatch of the whole nightly, so it
  first runs on the night after the merge. Its command ran by hand over real runs (Tests).
- **A hard limit a cell** (above), and **a random phase and seed** (FL.3, FL.4).
- **The `redis` group in the soak.** One shared instance cannot serve six cells (doc 51 §4.1).

## FL.3

`sim_lenses`: figures, seed, barriers. Built 2026-11-23 on `b0c0e6832` (main with #774, #775 and
#777 to #783 merged), beside FL.4. It adds three modules to `mush-storykit`, one field and one
function to tacsim (the one product change, ruling Q-51-2), and converts mushd's `sim_lenses`
stories. Three tests leave `flaky-known.toml`.

**The suite's soak rate.** `scripts/soak.sh` on conclav.dev, six cells of four cores, sixteen
rounds each, `sim_lenses` alone (97 tests), no retry:

| | Passes | Failed passes | Tests that failed | A pass, wall | A pass, summed test time |
|---|---:|---:|---|---:|---:|
| Before, main `b0c0e6832` | 96 | 3 | `nothing_of_the_shooting…` twice, `the_switch_names_a_contact…` once | 286 s | 1,075 s |
| After, this unit | 96 | 0 | none | 263 s | 965 s |

Both soaks ran beside other sessions' builds on the box (load 20 to 35 of 32 cores). The "after"
row is the third soak of the converted suite. The first two were stopped on failures in one
story, `nothing_about_an_undetected_unit…`, and what they found is under Tests.

**Merged with main before the pull request.** Both rows above are at `b0c0e6832`. Main then
took FL.4 (#784), the P2-Tier 2E security pass over the arena (#785), #786 and #787. The unit was
merged with main `9de381e74` and soaked again there, eight rounds a cell: 48 passes, none
failed, 263 s a pass and 967 s of summed test time. The guards under Tests ran again on the
merged tree.

**Survey.** What the code said before anything was built, where it bears on the design:

- **Every story file is its own rig.** Each of the fifteen files boots its own server from its
  own `tacsim_config()` and carries private copies of `say`, `ask` and `ask_whole`; they share
  one `Term` in `tests/support/mod.rs`. The kit has no `Term` until FL.7 and FL.8, so a barrier
  needs a seam to type and read through (Built, `Session`).
- **`shipped.rs` was already `Figures` in small.** Doc 49 cut CC.0 gave the lenses a reader of a
  mount's load and burst off `tacsim::builtin_definitions()`. It carried no cycle, "because no
  lens asserts one"; rows 3 and 9 were two lenses waiting on one.
- **The hit roll's label is the shot's number and nothing else** (`hit:<shot_seq>`,
  `arena/rng.rs`; the number is the arena's own counter, taken when a shot is accepted). So a
  pinned seed gives each shot one roll whatever pulse it fell in, as doc 51 §3.2 argues.
- **No shot is certain by the rules.** The resolution table caps a chance at 950 per mille and
  floors it at 10. Doc 51 §3.2's second tool, `fig.sure_range`, "a range at which the rules'
  chance is the whole die", does not exist to build. Its third tool does the work instead: a
  roll under the floor lands at any odds, and a seed can be chosen for it.
- **`ArenaConfig` is carried whole in the arena snapshot**, and `snapshot.rs` says a key added
  to it is a change of the snapshot's shape, which moves the format number and the structural
  pin (`af1_the_snapshot_shape_is_pinned`). The brief said the pins must not move.
- **61 raw waits in the fifteen story files.** Thirty-eight are in the private readers (`say`,
  `ask_whole`, the web-socket drains), which are FL.8's. Twenty-three were in stories.
- **Row 16 had a record nobody had read.** The one failed run's log prints the loser's whole
  unit record (Rows, below).

**Built.**

- **The seed** (tacsim, the product change). `ArenaConfig::seed: Option<u64>`, `None` by default,
  and `match_::opening_seed(pinned, match_id, world_now)`, which `match_::start` calls where it
  called `seed_for`: the pinned seed when there is one, `seed_for` otherwise. Sixteen lines in
  `config.rs` and twenty-four in `match_/mod.rs`, comments included.
  - **Live play is untouched.** With none pinned the seed is still 64 bits from `OsRng`
    (EXT-P2-11). `seed_for`, `MatchRecord::seed` and `FieldConfig::seed` are not edited.
  - **No file, command or player sets it.** `[tacsim.arena]` refuses an unknown key and
    `mush_server::config_ext` writes `None`; `fl3_no_config_key_pins_a_match_seed` holds both.
    Only code that builds a `TacsimConfig` by hand, which is a test rig, can pin one.
  - **It is not a key of the encoded block.** The field is `serde(skip)`, so a snapshot carries
    no `seed` key, its shape and format number stand, and a document that names one is refused
    (`fl3_the_pinned_seed_is_no_key_of_the_encoded_arena_block`). A resumed arena reads its seed
    from its own state, as before.
- **`mush_storykit::Figures`** (`figures.rs`), built from the story's own `TacsimConfig`:
  - `mount(unit, component)`: the load, burst, cycle, muzzle speed and reach, with `full()` and
    `left(n)` as the cockpit prints a row. It reads the catalog that config builds, the shipped
    one or the fielded catalog over the config's own packs (`observer_rule` adds two).
  - `pulse()`, `pulse_under(floor)`; `after_one_round(mount)`, `crossing(unit, meters)` and
    `after_reclaim()`, each a limit under the kit's load factor; `sensor_pass_seconds()`.
  - The dice of a seed: `hit_roll`, `sure_hit` (the roll is under the table's floor),
    `sure_miss`, `harms_a_part` (doc 36 cut GC.2's draw against the unit's chances), and
    `seed_where(from, predicate)`, the first seed that gives the dice a story asks for.
  - `story_seed(pinned)`: the pinned seed, or a fresh one printed with the line that replays it;
    `STORY_SEED=<n>` in the environment overrides both.
- **`mush_storykit::Odds`** (`odds.rs`): `false_failure()`, the binomial's lower tail, and
  `assert_false_failure_below(bar)`, which fails with the arithmetic and the number of tries
  that would meet the bar. `PULL_REQUEST_BAR` is ruling Q-51-3's one in 1,000,000.
- **The barriers** (`barrier.rs`), over `trait Session` (a mark, a line, a wait for one of
  several needles, the text after a mark), which mushd's `Term` implements:
  - `settled`, `ask_whole`;
  - `arena_wake`, `arena_seconds`, `arena_frames`, `the_match`, over one parser of
    `@arena/status`'s row that undoes the terminal's wrapping. The readout is read to its own
    last line, one line a poll;
  - `at_one_wake(staff, id, floor_ms, send)`, with `set_floor` and `lift_floor`.
- **`wait_for`** (`wait.rs`): `wait_until` for a wait that reads something. Its failure prints
  what it last read.
- **mushd's rig** (`tests/support/mod.rs`): the kit's re-exports; the three chosen seeds as
  numbers (`FIRST_SHOT_LANDS`, `FIRST_TWO_SHOTS_LAND`, `A_CLEAN_DUEL`) and the two assertions
  that hold a story's seed to its promise; `Term::room_for(lines)`, a wait for the session's
  flood bucket on the server's own bucket type and figures. `wait_arena_seconds` and
  `arena_clock_seconds` are now two lines over the kit, and `shipped.rs` is one.

**The stories.** Every rig that starts a match pins a seed: a chosen one where an assertion
leans on a roll, a fresh printed one elsewhere.

| File | What changed |
|---|---|
| `sim_range` | Row 9. One round on `FIRST_SHOT_LANDS` in place of "fire until one lands, twenty at most"; the 21-s literal is `after_one_round`. |
| `observer_rule` | Rows 3, 4 and 13. `o3`: as `sim_range`. `ae2`: one missile in phase 1; `ae2_frames` and the hand-built wake sequence are `at_one_wake`. `i3`: its two back-to-back lines go through `at_one_wake` too, its four 1-s sleeps are `arena_seconds` at the sensors' own cadence, and its 1.5-s sleep is the round's flight on the arena's clock and `settled`. The shooting story: each round is typed until the cockpit takes it, inside the cannon's cycle, and `SCAN` is read whole. The four flood-bucket sleeps are `room_for`. `o1_drain` is `settled`. The catch-up's 90-s literal is `crossing`. |
| `sim_rematch` | Row 16. The duel on `A_CLEAN_DUEL`; `fight_to_the_end` fires the magazine at most (it was a literal thirty) with no sleep; `easting_m` and `board_again` wait through the kit. |
| `arena_story` | The same duel and seed. The sixteen reads 300 ms apart that decided whether a round "went nowhere" are a wait for the arena's own answer. |
| `sim_units` | Both locks are waited for, both guns fire, and the hit is waited for inside one cycle, on `FIRST_TWO_SHOTS_LAND`. It was forty passes with an 800-ms sleep, "for an outcome that is a draw". |
| `sim_solo` | Two "thirty passes, 400 ms apart" loops wait for their event under the step limit. |
| `sim_weapons` | Pinned. "One of the burst lands" states its odds with `Odds`, at the chance the cockpit gives. |
| `sim_cockpit`, `sim_lobby` | One poll loop and one fixed sleep before an `expect` that already waited. |
| `arena_custody` | Ten event waits bounded by literal 10 and 20 s take the rig's `STEP_TIMEOUT` (Divergences, 6). |
| `sim_net`, `sim_reflex` | A fresh printed seed; nothing else. |

**The counts.**

| | Before | After |
|---|---:|---:|
| Raw waits in `sim_lenses` story files (`wait_budget`) | 61 | 42 |
| mushd, all story files (`wait_budget/mushd.txt`) | 124 | 105 |
| Wait sites under `crates/*/tests/` (`scripts/wait_audit.py`) | 615 in 104 files | 597 in 103 |
| `Duration::from_secs` and `from_millis` literals in `sim_lenses` | 99 | 67 |

`wait_budget` was re-blessed down by record: seven rows fell and `sim_lobby`'s is gone. What is
left of the 42: thirty-eight in the private readers, two `@arena/place` retries on a busy
answer, and the two sleeps of Not built.

**Rows.**

- **Rows 3 and 9** were fixed by hand in #764 and #765 (a literal 21 s). They are fixed by class
  here: no literal, and no second round to wait a cycle for.
- **Row 4.** (a) is closed by the seed, not by the 60 m of #765: the first missile lands at any
  range. (b) is `at_one_wake`.
- **Row 13 is re-described: not clock.** `nothing_of_the_shooting…` had a dice cause (two rounds
  fired, one hit needed: its `timeout waiting for "Hit on the "`) and a stream cause (`SCAN`
  parsed after a needle read that the second round's own outcome line could satisfy). The soak
  of main caught each once. `nothing_about_an_undetected_unit…` was never caught on main, in
  CI's record or in the soak. It was stream too: two reads that an arena notice in flight could
  spoil (the report's window, the classification wait), behind four hand-written sleeps. Taking
  the sleeps out is what showed them (Tests).
- **Row 16 is re-described: dice.** The failed run's record shows the loser eliminated at 43 s
  with its hull and turret standing, the turret at 10 of 150 inside after three hits, and every
  part intact. Two survivable turret hits each jammed one of the turret's two guns (doc 36 cut
  GC.2, one chance in twenty a hit), a side with no gun left is out (16b cut AP.2), and the sim
  center's check-in returns parts unjammed. `damage.rs`' own tests prove each step. It is a
  reading of the record and the code, not a reproduction: the story failed this way once in the
  1,000 CI runs the record was collected from.
- **Row 19 is new**: `the_switch_names_a_contact…`, one soak pass of main, a stream fault, fixed.

**Tests.** All on conclav.dev, in clones of `b0c0e6832` with this unit's patch.

- `mush-storykit`: 37 unit tests and 4 doc tests.
- tacsim: `fl3_a_pinned_seed_opens_every_match_and_none_keeps_the_os_draw`,
  `fl3_the_pinned_seed_is_no_key_of_the_encoded_arena_block`, and
  `af1_the_snapshot_shape_is_pinned` with its neighbors, unmoved. mush-server:
  `fl3_no_config_key_pins_a_match_seed`.
- `sim_lenses` whole, one cell's shape (four cores, four threads, no retry): 97 of 97, 279 s.
- mushd, every binary (`cargo nextest run -p mushd`, eight threads, no retry): 261 run, 261
  passed, 1 skipped, as at FL.1.
- **A stress run found a hole in the first conversion.** Thirty-two duel stories at once on
  eight cores: the cockpit answers `Firing cannon.` off its last frame, which under that load was
  seconds old, so it accepted 24 `FIRE`s in a row; the loop counted the magazine spent after one
  real shot and the story timed out. Main's loop had the same hole behind a 400-ms sleep. The
  three shooting loops now count a round when the arena has taken it (the magazine's row moves)
  and type it again when the arena says `FIRE cannon refused`. Sixteen of sixteen passed under
  the same load afterwards.
- **The first two soaks of the converted suite failed 3 passes in 66**, every one in
  `nothing_about_an_undetected_unit…`, which had not failed in main's 96. Each was stopped and
  fixed; the soak in the table above is the run after. That story's four sleeps "for the flood
  bucket" had been giving its reads time to settle, and without them three reads showed what
  they were:
  - The rig's line readers drop a sentinel's echo only when the word opens `zzq-`, which is why
    six files' sentinels are `zzq-…` words. The kit's was not, and `observer_rule` now types the
    kit's, so an echo that landed inside a brief read as a stray line. The kit's sentinel is
    `zzq-sentinel`.
  - `SIM REPORT`'s read window opens at its mark. With the story's hand-written sleeps gone, a
    lock notice still in flight (`Locked on BW, 78% to hit.`) landed inside it and was read as
    the report naming a contact. `sim_report` now returns the window from the center's own
    opening line on. Main has the same window; its sleeps hid it.
  - The wait for the sensor to classify its contact read the word `blue` anywhere in the
    `CONTACTS` window, and the arena's `identified` notice carries it: landing behind a row that
    still read `X1`, it ended the wait a frame early (one pass in twelve of the second soak).
    The story's own comment warns of exactly this read. The wait now reads the row, and the
    notice is waited for where it was looked for. Main has this one unchanged.
  - Found on the way: a status wait that asks five times a second, typing a line and a sentinel
    each time, types at the flood bucket's refill rate. The kit reads `@arena/status` to its own
    last line, one line a poll, as the rig did.
  - Between the soaks, the two row-13 stories and their neighbors ran 24 invocations side by
    side: 144 of 144 after the first soak's fixes, 216 of 216 after the second's.
- **The duel's length depends on the seed.** Sixteen clean seeds, both duel stories each: 74 to
  178 s for `sim_rematch`, 101 to 176 s for `arena_story`. `A_CLEAN_DUEL` is the shortest pair
  (74 and 106 s; main's random seeds average 127 and 136 s).
- `cargo fmt --all --check`; clippy with `-D warnings` over tacsim, mush-server, mush-storykit,
  mushd and mush-contract-tests, every target; rustdoc over the workspace with
  `--document-private-items`, and over the kit without; `mush-contract-tests` whole.

**Divergences.**

1. **`Figures` is keyed by unit and mount, and a round's limit takes no range.** Doc 51 §3.1
   sketches `fig.cycle("cannon")` and `fig.after_one_round("cannon", range_m)`. A mount belongs
   to a unit, as `shipped::mount` already had it. `after_one_round(mount)` uses the mount's whole
   reach for the flight, so a story restates no distance either; it is a limit, and the wait
   returns when the cockpit says ready. For the stock cannon it comes to 36.4 s where the literal
   was 21 s: doc 51 §3.1's formula (cycle, flight, two pulses) under the load factor of two, not
   a number picked to pass.
2. **The seeds are numbers in the rig, checked, not searched for at boot.** `seed_where` finds
   them, but finding one binds a second catalog, about 3 s a test in a debug build, in every test
   of a file that pins. The rig holds the three numbers; a story that leans on one asserts it
   against the content before it spends a round, and that failure names the seed to pin instead.
3. **A pinned seed opens every match of the world**, as `FieldConfig::seed` does every field. Two
   matches in one story (`sim_rematch`) draw the same dice. A pinned `0` opens on `1`.
4. **`at_one_wake` takes the floor and a closure, and is not exact.** The two lines of row 4b
   come from two sessions, so the helper cannot take a list of lines. It reads the frame count,
   sets the floor (which pulls a wake in), waits for the count to pass the reading, and runs the
   closure. That leans on the arena task taking the floor before the next round trip of a
   session returns, which a floor of five seconds makes safe in practice and nothing proves. An
   exact version waits for a floored wake it has seen and costs up to one floor a use.
5. **No blanket pacing of a terminal.** The first build charged every line to a copy of the
   flood bucket and waited when it ran dry. That cost about 3 s a test. `Term` now only keeps
   count, and the four places that typed a battery wait for the bucket's room.
6. **Ten bounds grew, and one shrank.** `arena_custody`'s ten event waits were bounded by 10 and
   20 s, numbers chosen at the call. Doc 51 §3.7 says a bound on an event is the rig's one
   limit, so they are `STEP_TIMEOUT` (60 s). None was failing and none was changed to make a
   test pass. The catch-up's 90 s became the greater of `crossing` (57 s) and `STEP_TIMEOUT`.
7. **A story that leans on no roll gets a fresh seed from the rig**, printed, where doc 51 §4.1
   has the soak supply one. `scripts/soak.sh` is unchanged: every run prints its seed, a soak
   pass included.
8. **Three stream fixes that are FL.8's kind.** `SCAN` read whole, row 19's wait for the lock's
   own line, and `sim_report` returning the report alone. The soaks named all three in this
   suite, so they are fixed here. The other files' copies of `sim_report` have the same window.
9. **`observer_rule`'s sentinel word is the kit's, and the kit's opens `zzq-`.** Its lenses strip
   the sentinel's echo from a transcript, `settled` types the kit's word, and the rig's line
   readers know a sentinel by that opening.
10. **`Odds` is used once**, in `sim_weapons`. Everywhere else a seed made the outcome certain,
    which doc 51 §3.2 prefers. FL.5 is its next user.
11. **The kit's `clippy::panic` count is 16, from 2** (`restriction_lints`' record): the
    failures of `Figures` and `Odds` are a story's premise not holding.

**Not built.**

- **`after_tick`**, which doc 51 gives to FL.8. Two sleeps of 2.5 s wait for "two more boarding
  ticks" after a restart (`af3_…`, `a_restart_mid_match_through_the_lobby…`) and are left.
- **The private readers**: 38 raw waits, FL.8's. The kit's `ask_whole` types a line again on a
  busy answer as the twelve copies do, and shares their one hazard: the first try's sentinel is
  still answered, and that `Huh?` can end the second try's read early. FL.8's `Term` should count
  answers.
- **The web-socket drains** of the mind feed (`m5_drain`, quiet and most spans), FL.9's kind.
- **Two assertions that a transcript took under 150 s** (`sim_solo`, `sim_weapons`). They are
  about run time, doc 43's, and are not waits.
- **The carried rows** for `arena_story::two_pilots_walk_in…` and `sim_reflex::*`. The duel story
  got row 16's fix; its row has no failure on record and is left for the soak's week.
- **The literal-span counter** is still a stub (FL.9).
- **A reproduction of row 16.** The seed tool makes one possible (a seed whose survivable turret
  hits both jam); it needs the facet draws read too, and was not done.

**For the owner.**

- **Q-FL3-1.** `arena_custody`'s ten event bounds went from 10 and 20 s to the rig's 60 s
  (Divergences, 6). Default: keep, as doc 51 §3.7 reads.
- **Q-FL3-2.** A sim-center duel can end with the loser disarmed by two jams, and its record then
  reads every part intact (row 16). 16c's T-101 already notes that a disarmed loser's record
  contradicts its end. Whether the check-in should say what disarmed it is a product call.
  Default: no change here; the story no longer meets it.

## FL.4

`commerce_composition`: the boot instant. Built 2026-11-23 on `b0c0e6832` (main with #783 merged). It adds two modules
to `mush-storykit`, takes the three rigs of mush-server's `commerce_composition` suite that booted at the wall clock
onto a fixed instant and a fixed geology key, runs five stories at named phases of the IC calendar, and settles row
17 of `docs/flaky-tests.md`. It changes no product code.

Every measurement below is from conclav.dev, by hand as `ghrunner`, in clones with their own target directory, on
cores 24 to 31 while another session's soak held the rest. A run is one test in its own process, as nextest runs
it. A "boot second" is the minute and second of the real hour a rig boots at: at 24:1 an IC day turns on the real
hour.

**Survey.**

- **Three rigs booted at the wall clock, not four.** `ic_slice_loop` and `ic_buyout_dormancy` passed
  `mush_config::unix_now_s()` to `build_engine_at`. `sm_story` called `build_engine`, which reads it, and then set its
  clock to a start computed from `SystemTime::now()` (`story_t0`): 1,900,000,000 until 2030, so that story has always
  run at one phase, IC 18:40. The fourth file with a `clock` row, `market_issuer`, boots a `TestWorld` and read the
  wall clock only to name its scratch directory, as all four did.
- **The calendar.** The rigs boot `[game]` at its defaults with `ic_ratio = 24`: `ic = ic_at_unix_zero + 24 × unix`,
  so a day turns every 3,600 real seconds, on the hour. The game's monthly schedules (the Assembly's vote, a
  stockpile's budget) turn on thirty IC days counted from IC second zero (`tacsim::concession::IC_MONTH_S`), not on the
  civil calendar's month.
- **Row 17 is a phase fault, and it reproduces at both heads that failed.** Each tree below is that head with one
  change, the boot second read from a variable. "In the window" is failed runs over runs booted inside it.

  | Tree | Story | In the window | Outside it |
  |---|---|---|---|
  | `597d2c6b`, 2026-11-19, the head of [44444459400](https://github.com/conclavio/rustmush/actions/runs/44444459400) | `ic13_a_buyout_is_refused_…` | 16 of 17 booted from 41:00 to 45:00 | 0 of 35 |
  | | `sm2_a_week_of_hops_…` | 4 of 4 booted from 47:20 to 48:00 | 0 of 55 |
  | `62a8a8382`, 2026-11-04, the parent of `4ce4158e5` | `ic13_a_buyout_is_refused_…` | 3 of 3 booted at 42:00, 44:00 and 46:00 | 0 of 27 |
  | | `sm2_a_week_of_hops_…` | none found | 0 of 30 |

  The CI run started the first story at 02:42:09 UTC (read from its log). Booted at that second it fails with CI's
  own text, to the minute it prints: `Clearance filed for Stickney Yards: … arrival in 18.4 IC days, about 03:09 UTC
  today. … left: 2, right: 1`. The second story fails in its window with CI's `the hop's burn … left: 2, right: 1`.
  CI started it at 44:37, three minutes before the window, and at that second it passed here in every run. The
  likely difference is the random geology key (below), which moves a story's steps by up to 192 s from run to run;
  that is a reading, not shown.
- **The cause.** Each story counts what one typed line queued (`drain_players`, then `len() == 1`). The count takes
  in whatever the ticks queued since the last drain, less the actors on a house list the rig keeps by hand, whose
  submissions it lands quietly. A clerk that acts once an IC day queues its act at or soon after the day's turn. The
  stories have stretches where ticks fire and nothing is drained (two 61-s sweeps at a time), and a run failed when
  its wall-clock boot put the turn in one while the list lacked an actor. Twice a daily actor reached a head before
  the list had it: IN.5's Stickney clerk (added to the list in `4ce4158e5`, 2026-11-04) and EX.9's lane-port buyers
  (added in `bca28628f`, two hours after the failing run of 2026-11-19).
  **It is not a stream fault.** Both stories read state in one process; nothing is parsed off a stream.
- **Main does not fail at any boot second tried**, 763 runs: the whole suite once at the wall clock, unchanged
  (93); then with the boot second read from a variable, the suite at the epoch, at the day-turn instant and at the
  mid-month instant (92 each), in 2020 and in 2049 (92 each), the five stories at 30 seconds of the hour two minutes
  apart (150), and the same five twelve times each at one second (60). Nothing in the suite depends on the phase
  today, or on whether the boot second is before or after the wall clock. **No product defect was found.**
- **The three named phases do not reach row 17's stretch.** On this unit's tree, with one daily actor taken off the
  dormancy rig's list (Luna's clerk) and the two stories run at 60 boot seconds a minute apart (`STORY_PHASE=+N`):
  the first fails at four in a row (a turn 913 to 1,093 s after the boot), the second at one (733 s), and both pass
  at the three named phases. With all of EX.9's actors off the list the first fails at seven in a row and the second
  at all sixty. The steps a turn has to reach: the world reads Ready 708 s after the boot; the first story's two
  sweeps before its outfit run from 958 s to 1,080 s and the two that close it from 1,080 s to 1,202 s; the
  second's two before boarding run from 708 s to 830 s. From the epoch the first turn is 2,173 s away; from the
  day-turn phase, 3 s.
- **A story was not the same on every run, even at one boot second.** The two ic rigs gave the engine an empty
  geology key directory, so each boot provisioned a random key, and the ground decides how many fires the sites'
  clerks need before the world reads Ready. On main, at one boot second, 74 of 84 world bring-ups took 98 fires,
  nine took 96 and one 130 (a fire is 6 s). With `sm_story`'s fixed key, 84 of 84 took 98 and ended on the same
  second. `sm_story` has pinned its key since SM.8 for this reason. The same variation shows at the failing head:
  the first story booted at 44:00 failed in one run and passed in another.
- **nextest's test ids move.** `phases!` makes `<story>::<phase>`, so the old ids of the five stories are gone.
  Outside the suite they were named only in `flaky-known.toml` (row 17's two) and its generated block.

**Built.**

- **`mush_storykit::phase`** (`src/phase.rs`):
  - `STORY_EPOCH`, unix second 1,900,862,627: IC 09:30:48 on day 7 of a schedule month under the servers' calendar,
    17:23:47 on day 10 under the engine test world's.
  - `Calendar::new(rate, ic_at_unix_zero)`, which a rig builds from the `GameClock` of the config it boots with. It
    reads the IC second at a real one, finds the real second for an IC instant, and counts to and from a turn.
  - `Phase::{Epoch, DayTurn, MidMonth, Past(n)}`, `Phase::NAMED`, `Phase::instant(calendar)`, and
    `Phase::boot_unix(calendar)`, which is what a rig calls: the phase's instant unless `STORY_PHASE` asks for
    another, and one printed line naming the phase taken and where it falls (`story phase mid-month: boot at unix
    1900891427, IC 09:30:48 on day 15 of the schedule month's 0 to 29`).
  - `STORY_PHASE` (`PHASE_VAR`): `random` draws from a whole schedule year at every boot; `+<seconds>` and the words
    `epoch`, `day-turn` and `mid-month` set every boot of a run. Anything else fails the boot.
    `Phase::overridden()` says whether it is set.
  - `phases!(#[tokio::test] async story)`, and the same without `async` for a plain test: a module named for the
    story holding `at_the_epoch`, `before_a_days_turn`, `mid_month`, and one test for each pinned phase the call
    names. `PHASE_TESTS` is the three names.
  - `IC_HOUR`, `IC_DAY` and `IC_MONTH`, which the suite holds equal to `mush_core::clock`'s and tacsim's.
- **`mush_storykit::scratch_dir(suite, tag)`** (`src/scratch.rs`): a fresh directory named by the process id and a
  counter, with no clock in it.
- **The rigs.** `ic_slice_loop` and `ic_buyout_dormancy`: `boot(tag)` is `boot_at(tag, Phase::Epoch)`, and
  `boot_at` takes the phase's second from the kit. Both write `sm_story`'s fixed geology key into their key
  directory before the boot. `sm_story`: both arms boot at one second, taken once from `Phase::Epoch`; `story_t0`,
  its three constants and its test `sm8_the_story_start_is_never_in_the_past` are deleted, since no start is
  computed from the clock any more. All four files take their scratch directory from the kit.
- **Five stories at the three named phases**, through `phases!`: `ic12_part_one_a_leg_by_hand`,
  `ic12_part_two_the_queue_runs_the_return`, `ic12_the_sailing_plan_skips_and_holds`,
  `ic13_a_buyout_is_refused_under_way_cleared_elsewhere_or_with_goods_moving` and
  `sm2_a_week_of_hops_trips_no_recall`. Each body is as it was but for its boot.
- **Two pinned phases, one for each row 17 story**: `the_day_turns_before_the_outfit` (`Phase::Past(1_143)`, a
  turn 1,030 s after the boot) and `the_day_turns_before_she_boards` (`Phase::Past(1_440)`, 733 s). Each story
  checks, at its pinned phase only, that the IC day did turn across the two sweeps the phase was pinned for, and
  fails with how to pin it again if it did not.
- **Two tests of the kit's figures, in the suite**: the hour, the day and the month are the game's; and the epoch
  is clear of every boundary of the calendar the rig really boots with.
- **Row 17 closed.** Its two rows are off `flaky-known.toml` (34 rows left), the generated retry block follows,
  `docs/flaky-tests.md` has the cause, and `docs/flaky-observed.md` is refreshed.
- **`wait_budget` re-blessed down**: the four files' `clock` rows are gone.
- **The gate and the report read `<story>::<phase>`.** `flaky_known` finds a listed test of that form by the
  story's `phases!` call (the three named phases, or a phase the call pins under that name) and reads the record's
  row by the story's name. `scripts/flake_report.py` matches such a test to the record the same way. Without this
  a story run at phases could not be put on the known list at all, because no source line spells its tests.

**Tests.**

**The suite by boot, before and after.** A cell is failed runs over runs. "Before" is main `b0c0e6832` with the boot
second read from a variable and nothing else changed. "After" is this unit.

| The rigs boot at | Before | After |
|---|---|---|
| the wall clock | 0 of 93, the suite once on unchanged main | no rig reads it |
| the epoch | 0 of 92, the suite; 0 of 60, the five stories twelve times | 0 of 29, every rig story with its boot set to the epoch |
| the day-turn instant | 0 of 92 | 0 of 29, the same set to the day-turn phase |
| the mid-month instant | 0 of 92 | 0 of 29, the same set to the mid-month phase |
| a boot in 2020, a boot in 2049 | 0 of 184 | not run |
| as a pull request runs it: the five stories at their named and pinned phases, the rest at the epoch | no such run | 0 of 318, three passes of the suite's 106 tests |
| seconds across the hour | 0 of 150: the five stories at 30 seconds, two minutes apart | 0 of 435: every rig story at 15 seconds, four minutes apart |
| a drawn phase a boot (`STORY_PHASE=random`), six passes | no such run | 0 of 174: every rig story six times, 186 boots, each at its own drawn phase |

Before, 763 runs and no failure. After, 1,014 runs and no failure. "Every rig story" is the 29 stories of the three
rigs, one test a story. The suite did not fail before this unit at any phase tried, so the rate that changed is the
one row 17 records: at the heads that failed, its first story in about one boot in fifteen (the Survey's table).

**The pinned phases, proven both ways**, in a throwaway clone:

| Tree | The six tests at named phases | `the_day_turns_before_the_outfit` | `the_day_turns_before_she_boards` |
|---|---|---|---|
| this unit | pass | pass | pass |
| this unit, with Luna's clerk taken off the house list | pass | fails: `Clearance filed for Stickney Yards: … left: 2, right: 1` | fails: `the hop's burn … left: 2, right: 1` |

Those are the two texts of row 17.

**The rest.**

- `mush-storykit`: 32 unit tests and one doc test. Three examples that show `phases!` are marked not to compile,
  because a `#[test]` in a doc test never runs and clippy refuses one; the unit tests call the macro for real, in
  its plain, async and pinned forms.
- `mush-contract-tests`, whole, with no fail-fast: 35 in the lib, 98 passed and 8 ignored in `ci_policy`, 92 in
  `engine_contracts`. `flaky_known::a_story_run_at_named_phases_is_found_by_its_phases_call` is new: a listed
  `<story>::mid_month` is found, a phase nobody pinned is refused, and so is a phase's name under a story that is
  not run through `phases!`.
- `scripts/test_flake_report.py`: 17, two of them new.
- `cargo fmt --check` on the three crates; clippy with `-D warnings` over `mush-storykit` and
  `mush-contract-tests` on every target and over mush-server's tests; rustdoc with `-D warnings` over the kit in
  both modes and over `mush-contract-tests` with private items.
- **CI on the branch**, before the pull request was opened (run
  [45311775699](https://github.com/conclavio/rustmush/actions/runs/45311775699), a dispatch):
  every cell a pull request runs is green. `rest-server` took 27 minutes against 21 on main's last run
  ([45285692521](https://github.com/conclavio/rustmush/actions/runs/45285692521)); the slowest cell of the run was
  `tacsim-integration-2` at 28.

**`wait_budget`**, the gate's files, which `python scripts/wait_audit.py --budget` prints row for row:

| | Before | After |
|---|---:|---:|
| Wall-clock reads in mush-server's story files | 13 | 6 |
| of them in `commerce_composition` | 7, in four files | 0 |
| Wall-clock reads in story files, the tree | 54 | 47 |
| Raw waits in story files, the tree | 526 | 526 |

**Summed minutes** (doc 43 rule 2.3 #14), from CI's JUnit reports. On main's last run the suite was 93 tests and 45.5 summed
minutes, the five stories 9.7 of them. On the branch's run it is 106 tests and 70.1 summed minutes: the five stories
at the epoch 10.0, at their two other named phases 20.1, and the two pinned phases 4.0. So the suite costs about 24
summed minutes more a run, and the `rest-server` cell six minutes more of wall clock.

**Divergences.** Where this differs from doc 51, or from the brief the unit was cut with:

1. **Three rigs, not four, and the read that was left is gone too.** The Survey's first point. The fourth file's
   read was a directory name. `scratch_dir` is not in the unit's row; it is why the suite's count is zero and not
   four.
2. **The three named phases are kept as doc 51 wrote them, and they are not what catches row 17.** Doc 51 §3.6
   expected a schedule-bound story to be covered by the epoch, a day's turn and mid-month. Row 17 needs a turn
   inside one stretch of the story, so each of its stories has a fourth, pinned phase. A pinned phase is a measured
   second, which rule F would not allow a wait to be. It stands because the story's steps now fall on the same
   seconds every run, and because the story checks it at run time. The soak's drawn phase is the net under it.
3. **The geology key is pinned in the two ic rigs.** That is rule D, not rule T, and no row of §5 names it. Without
   it a named phase is not one instant of the story, and a pinned phase could not be aimed.
4. **A month is the game's thirty-day schedule month**, which the kit's constant says in its comment.
5. **`MidMonth` keeps the epoch's time of day.** "15 IC days before a month's turn" to the second is also a day's
   turn. The phase boots in the day that begins fifteen days before the turn, at the epoch's time of day, so that
   it moves one thing.
6. **`DayTurn` is three real seconds before the turn** under the servers' calendar: the last whole second at least
   50 IC seconds before it, and a second is 24 IC seconds.
7. **`phases!` takes the story's name, not its body**, and the test attribute to use. `macro_rules!` cannot join two
   names into one, so the tests are `<story>::<phase>` in a module, and the story stays an ordinary function that
   rustfmt formats. The old test ids of the five stories are gone.
8. **`STORY_PHASE` takes more than `random`**: `+<seconds>` to set what the soak printed, and the three words.
9. **The kit still depends on tokio alone.** `Calendar` is two numbers and `GameClock`'s one rule. Linking
   mush-core for them would put the engine under every story crate's kit, so the suite holds the kit's three
   calendar constants to the game's instead.
10. **The epoch is in 2030**, as `sm_story`'s old start was. The suite passes the same booted in 2020 and in 2049.
11. **The gate and the report changed** (Built, the last point), and with them four of FL.2's files:
    `src/flaky_known.rs`, its test, `scripts/flake_report.py` and its test.
12. **One test of the old set is deleted**: `sm8_the_story_start_is_never_in_the_past` tested arithmetic that is
    gone.
13. **The rates are by-hand runs of the suite's binary, not `scripts/soak.sh`'s.** Doc 51 §5 asks for the soak's
    rate. These stories run on a clock the test sets and wait on nothing, so the soak's load adds nothing to them,
    and a by-hand run can set the boot second, which the soak cannot do for the tree before this unit.

**Not built.**

- **A drawn phase in any other suite.** Only the three `commerce_composition` rigs read `STORY_PHASE`. tacsim's
  field rigs still boot on a month's boundary (doc 51 §3.6), and mushd's, the gateway's and the bot's rigs boot real
  servers at the wall clock.
- **One rig for the suite.** The three rigs are still three copies of one file, by an old lead ruling, and the
  house list that row 17 turned on is kept in each. A daily actor all three must know is still three edits.
- **A count that does not depend on the list.** The stories still count every submission that is not the house's.
  A count of the pilot's own would not see a new daily actor at all. Whether a commerce story should fail when one
  appears is a question about what the stories are for, and it is not this unit's.
- **The other private copies of `scratch_dir`.** The tree holds 48; four are converted here.
- **The first night's soak with drawn phases.** The soak reads `STORY_PHASE=random` from the night this merges.
  What it draws for this suite is the six passes above until then.
- **A cell of its own for the suite.** The `rest-server` cell is now within a minute of the slowest cell of a
  pull request's run (27 against 28). Giving `commerce_composition` its own cell, as #244 gave `split_boundary`
  one, is a `ci.yml` change and the owner's call. It is asked in the pull request.

## FL.5

The bot's stories. Built 2026-11-25 on `0e3a26804` (main with FL.1 to FL.4 merged), beside FL.6 and
FL.7. It adds two modules to `mush-storykit` (`busy`, `reply`), moves the bot's rig onto the kit, and
fixes one new flake that the soak of 2026-11-25 found. It changes no product code. Both mush-bot rows
leave `flaky-known.toml`.

**The suite's soak rate.** The bot's three soaked binaries (`arena_e2e`, `arena_contract`,
`mining_cohort`), six cells of four cores, nextest's `soak` profile, no retry:

| | Passes | Failed passes | Tests that failed | A pass, summed test time |
|---|---:|---:|---|---:|
| Before: the nightly soaks of main, 2026-11-24 (`9de381e74`) and 2026-11-25 (`0ae636469`) | 72 | 1 | `the_opening_reads_the_ground_at_the_muster`, once | 1,664 s |
| After: this unit, on conclav.dev by hand (below) | 24 | 0 | none | 1,713 s |

The "after" soak is the bot's three binaries alone, by a script of this unit's that runs each cell
from one nextest archive, at `0e3a26804` with this unit's patch, on cores 0 to 23 from 23:10 UTC.
`scripts/soak.sh` was not used (Not built). Before, 72 passes of 39 tests; after, 24 of 40 (the new
contract test).

**Survey.**

- **Six private busy lists, none complete.** The rig's `TRANSIENTS` (four needles), the kit's
  `BUSY` (two), the bot's own `mine::lines::ENGINE_BUSY` and `TRANSIENT_TAILS`, the arena reader's
  `transient`, and `@newchar`'s one sentence in `Client::create_character`. The rig's list lacked
  the engine's three back-pressure sentences, which are row 5's class: a `Client::say` that met
  `An authoritative request is already pending.` took it for an answer. Nothing checked any list
  against the sentences the server writes; none of them is a product constant (each is a literal
  in its module, `mush-core/src/engine.rs`, `mush-server/src/world_task/mode.rs`, five tacsim and
  mush-sim modules).
- **The rig's step limit was 20 s of its own.** Doc 51 §3.7 asks for the kit's.
- **`mining_cohort`'s three stories a pull request runs had three budgets.** The canary ran under a
  20-minute timeout (the terminate itself) with a 19-minute drive bound; row 5's story under 22
  minutes, past the terminate, so a stall in its last two minutes printed nothing; row 6's under the
  18-minute budget #764 built by hand. The fifteen `#[ignore]`d stories run by name under their own
  terminates (`ci-bot`'s 100 minutes for `fr7_round`, none locally) and keep their limits.
- **Row 18's reader** (`arena_contract`'s `Live::ask_whole`) was its own loop: type the line and
  its sentinel, wait for the first `Huh?`, and type both again on a busy answer. After a busy
  refusal of the line, the `Huh?` it then waited for was the sentinel's, so nothing was owed; but a
  busy refusal of the sentinel itself left no `Huh?` to come and failed at the step limit. The kit's
  `ask_whole` (FL.3) had the hazard FL.3's note names: it ended a try at the first answer, and after
  a busy refusal of the line the sentinel's `Huh?` was still owed and ended the next try early.
- **Needle reads that were parsed.** Of the 45 `say`, `answer` and `expect` results bound in the
  bot's tests, four were parsed past their needle: `@account/list`'s credit row in the float and in
  `rs9_story_b` (the needle is inside the row it parses, so the balance could arrive cut short),
  `arena_mind_author_story`'s report and bench reads (one asserts an absence on a read that stopped
  at a needle), and the wingman's `SIM STATUS` board. The readiness wait reads `@tacsim/port status`
  to its last line (`Ready: …` or `Not ready: …`), so it already knew where the reply ended.
- **The soak of 2026-11-25 failed one unlisted test**, `the_opening_reads_the_ground_at_the_muster`,
  once in 36 passes: `stepped off the road: … step from (267.0, 975.0), now (315, 994)`.
  `scripts/flake_report.py --run 45566917887` could not download the run's artifact on this box
  (`gh api …/zip` failed); `--junit` over the artifact fetched with `gh run download` gave the
  table: the one test, 1 in 36, not listed.

**The muster story (row 20).** 144 runs of it on main at `0e3a26804`, in six 4-core cells with four
copies at a time each, kept every transcript. None failed. Where the tank stood at the hold, north of
where its step began:

| Held at, m out | 21 | 22 | 26 | 30 to 34 | 36 to 40 |
|---|---:|---:|---:|---:|---:|
| Runs | 3 | 103 | 1 | 13 | 24 |

Every step was 35 m. The test held 20 m, a number written by hand, against a tank that most runs
stopped 22 m out: the soak's run stopped at 19. The pilot ends the step `OPENING_ARRIVE_M` (10 m)
short of its aim on a dead-reckoned position (`Picture::own_pos_at`: the last read position carried
along the current heading at the current speed), which runs ahead of the tank while it is still
turning off the road at the end of the move in. **Class: figure.**

**Built.**

- **`mush_storykit::busy`** (`src/busy.rs`): `BUSY_SENTENCES`, nine rows, each a sentence a server
  refuses a typed line with for a moment and the product files that write it. The kit's test reads
  each file and fails when its sentence is gone, so a reworded refusal fails in one place. `is_busy`
  (a line ends with one, a closing `.` aside, and does not open with `[`), `holds_busy`,
  `BUSY_NEEDLES` and `busy_needles`. The kit's own `BUSY` (two needles) is gone; `settled`,
  `ask_whole` and the status reads take the table.
- **`mush_storykit::reply`** (`src/reply.rs`): `Reply`, a whole reply with its echoes left out, and
  `row`, `rows`, `section`, `number_after` and `contains`; and `ask(term, line) -> Reply`, whose
  try is over only when both of its answers are in. A refusal first: done. A busy answer after the
  line's own answer: the sentinel was refused, done. A busy answer with nothing before it but the
  echoes: the line was refused, so the try waits for the sentinel's answer and types the line again.
  It is for a line whose answer prints something and that is safe to type twice, which every read
  is. Four of its tests script each case.
- **The bot's rig** (`tests/support/arena.rs`): `STEP_TIMEOUT` is the kit's (60 s, from 20);
  `TRANSIENTS` is the kit's `BUSY_NEEDLES`; `Client` implements the kit's `Session`, and
  `Client::ask_whole` is the kit's `ask`.
- **`the_bots_busy_sentences_are_the_story_kits`** (`arena_contract.rs`): every sentence of the
  bot's `ENGINE_BUSY` and `TRANSIENT_TAILS` is in the kit's table, and every sentence of the table
  is typed again by the bot's arena reader and, but for the two that are the arena's alone, by its
  mining lane. `@newchar`'s refusal is staff's and is left out.
- **`mining_cohort`**: the canary, row 5's story and row 6's story each run inside one
  `StoryBudget` of 18 minutes (`STORY_MINUTES`), and every wait in them draws from it. Row 5's
  half-second enrollment poll is `wait_for`, whose failure prints the transcript's tail.
- **`arena_e2e`**: every story opens its matches on a fresh seed, printed (`seeded()`), so a failure
  names the seed that replays its rolls. The solo flight's figures (`Solo`): the match holds the
  least number of the cannon's own pulls at which bad luck at the rules' ceiling meets the
  pull-request bar (five, so 75 s of cycles), plus `after_one_round` for the way to the first shot
  (56 s at the rig's tempo), so 131 s; the bot's run limit is the boarding window, the match and
  one step, 251 s. One run by hand printed `8 shot(s), 7 hit(s)`, the least chance offered 33 per
  cent, and a miss throughout by bad luck alone at those odds 0.04 of the time. "The pilot hit" states
  its odds: a match with no hit fails only when the cockpit's least offered chance, over the
  shots the pilot fired, makes a miss throughout rarer than the bar, and the odds are printed either
  way. The muster story's bound is the pilot's own geometry: `opening_step` makes a step the hiding
  run's start, its allowance and `OPENING_COVER_M` long, so the hiding ground begins no farther out
  than the step less `OPENING_COVER_M` (15 m on a 35 m step). The test reads the step's length from
  the pilot's own note.
- **The parsed needle reads** are whole reads: `@account/list` (twice), the mind report and the
  three bench reads, and the wingman's board.
- **`arena_contract`'s `Live::ask_whole`** is the kit's read; its own sentinel is kept only for the
  echo replay test.
- **`flaky-known.toml`**: rows 5 and 6 are deleted (25 rows left once merged with FL.6, #804); the generated retry block follows,
  and `docs/flaky-observed.md` is refreshed. Its 14-day table now shows both tests **NOT LISTED**,
  because their CI failures (the last on 2026-11-21) are still inside the window. `--check` in the
  soak and the nightly reads only that night's reports, so neither goes red; the rows age out of the
  table by 2026-12-05.
- **`scripts/flake_report.py`**: GitHub's `stream error … CANCEL` is retried as a transient. Two of
  this unit's three runs of `--ci-days 14 --write` died on it. `--run <id>` also failed here, on the
  soak artifact's `gh api …/zip`; it was not mended. `docs/flaky-tests.md`: rows 2, 5, 6 and 18 say what
  this unit did, and row 20 is new.
- **`wait_budget`** re-blessed down: `arena_contract.rs` 5 to 4, `mining_cohort.rs` 25 to 21.

**Tests.**

- `mush-storykit`: 65 unit tests and 4 doc tests, nine of them new (the busy table against its sources, a busy line, the needles; the four tries of `ask`, a section, a number).
- `the_bots_busy_sentences_are_the_story_kits`, and the echo replay, on Windows and conclav.dev.
- The after-soak above: 24 passes of the three binaries' 40 tests, 960 runs, none failed. Per pass, against the nightly soaks' 72: the solo flight 60 to 139 s (it was 57 to 68), the muster story 38 s (38 to 43), the canary 332 to 365 s (332 to 365), row 5's story 487 to 499 s (487 to 500), row 6's 570 to 596 s (566 to 595). The solo flight is the whole of the summed time's rise.
- `mush-contract-tests`: `ci_policy`'s `feature_matrix_inventory`, `panic_budget`, `restriction_lints`, `wait_budget` and `flaky_known`, 41 tests; `engine_contracts`' `us_spelling`, 5.
- `cargo fmt --check`; clippy with `-D warnings` over `mush-storykit` and mush-bot's tests; rustdoc
  over the kit; the census, the panic budget, the lints and the help lint.

**Divergences.**

1. **Row 2's range is the fixture's, not derived.** Doc 51 §3.2 sketches `fig.chance(...)` for the
   odds. The kit has the table's bounds and the dice, not the chance of a shot: that is the whole
   resolution (the error cone's five terms, the target's facet presented, the curve), and a copy in
   the kit would restate the arena's arithmetic. What is derived is the match's length, from the
   cannon's cycle and the bar at the rules' ceiling, and the odds are the cockpit's own figure,
   which `Odds` allows. The tanks stand 200 m apart as #764 posed them.
2. **"The pilot hit" is asserted only when the odds make a miss a fault.** Rule D: the assertion
   does not depend on a draw. With a moving tank no seed makes the hit certain, and the least
   chance the cockpit offered (33 per cent in the run measured) does not meet the bar over eight
   shots. A match with no hit at such odds passes and prints them. The longer match makes that
   rare; the record of the 72 soak passes before had no match with none.
3. **The solo flight costs about a minute more a pass**: a 131 s match where it was 60 s, and
   about 140 s a run where it was 68.
4. **The busy table is checked against source files, not product constants.** None of the
   sentences is a constant the kit could import, and making them constants is a product change in
   five crates. The kit's test reads the files instead; a sentence that moves to another file is a
   one-line edit of its row.
5. **The kit's FL.3 `ask_whole` is kept** for mushd, unchanged but for the table: FL.8 moves
   mushd's readers. `reply::ask` is the counted read; `ask_whole` returns its whole scrollback, and
   mushd's callers may lean on that.
6. **The rig's step limit grew from 20 s to the kit's 60 s.** No wait was failing on it. A story
   that asserts a refusal comes fast (`a_refused_pose_answers_at_once`, a quarter of the limit)
   now allows 15 s, not 5.
7. **Row 6 is closed on the soak's record, not on a cause.** 72 of 72 passes since #764, and the
   story now fails inside its budget with both bots' last lines. Its cause is not known.
8. **The muster flake is fixed in the test, and the pilot's own claim is left short.**
   `OPENING_COVER_M`'s comment says the tank "then stands at least 10 m past the run's true
   start". On its reckoning it stands 6 to 7 m past it in most runs, and 4 in the soak's. Q-FL5-1.

**Not built.**

- **The fifteen `#[ignore]`d `mining_cohort` stories' limits.** They run by name under terminates
  of their own (30 to 100 minutes), past what `StoryBudget::new` allows. A budget under another
  terminate is a kit change, `StoryBudget::under(terminate, minutes)`, for the unit that soaks them.
- **The other needle reads.** About forty bound `say` and `answer` results wait and are not
  parsed, and the field and salvage stories parse several lines read by the bot's own lane. They
  are left.
- **`one_reader` for mush-bot.** The bot's rig has no private `Term`, `ask_whole` or `strip_ansi`
  left but `Live`'s wrapper, which delegates; turning the gate on is FL.10's sweep.
- **`scripts/soak.sh` for the after-soak.** conclav.dev's root was at 98 per cent when this unit
  began its runs and reached 100 at 23:00 UTC, with other sessions' targets on it; this unit deleted
  its own, told the lead session, and rebuilt only the bot. By 23:23 the disk was at 48 per cent.
  The after-soak's script ran the cells from one archive with one remapped workspace, so the cells'
  JUnit files overwrote each other; its figures are from nextest's own output, a log a pass.

**For the owner.**

- **Q-FL5-1.** The pilot ends its step off the road on a dead-reckoned position, and stops 3 to 6 m
  short of what `OPENING_COVER_M`'s comment promises: on the hiding ground, but nearer its edge.
  Reading the step's end off the last position the cockpit printed, rather than a reckoned one,
  would keep the promise and make the step a second or so longer. Default: no change in this
  program; the test holds the tank to the hiding ground, which it reaches in every run seen.
- **Q-FL5-2.** "The pilot hit" now passes on a miss throughout when the odds allow one (Divergence
  2). The other way is a longer match still: about fifteen of the cannon's pulls, near four minutes,
  would meet the bar at a chance of 60 per cent. Default: keep the stated odds.

## FL.6

mush-server's lib and boundary. Built 2026-11-25 on `0e3a26804` (main with #797 merged). It adds one module to
`mush-storykit`, moves the in-process tests it fits onto tokio's paused clock, replaces two absence windows with the
world's barrier, gives the IPC proofs the kit's one step limit, and widens `wait_budget` to mush-server's test code
under `src/`. It closes every mush-server row of `flaky-known.toml` (four). It changes no product code.

Every measurement below is from conclav.dev, by hand as `ghrunner`, in a throwaway clone with its own target
directory, between 22:34 and 23:10 UTC, before the soak's hours.

**Survey.**

- **mush-server's rows of `flaky-known.toml` were four**: row 10 (`boot_parity::stop_ledger_archive_…`, class
  `clock`), and three carried from cut T.1's `load-sensitive` filter with no failure on record: the whole modules
  `ipc_authoritative_action` and `ipc_window` of `split_boundary`, and the lib test
  `world_task::tests::what_still_waits_when_the_loop_ends_is_answered_not_dropped`. In the 14 days of CI that
  `docs/flaky-observed.md` reads and the soak's nights, only row 10 failed (once, run 44713758290).
- **Row 10's cause is plain from the code.** The test spawned a task that ticks every 1 ms, slept 20 ms of real time,
  and asserted the task had ticked. On a runner that did not schedule the task inside those 20 ms it failed with
  its own message. The class is `clock`: a sleep standing in for "the task has run".
- **`what_still_waits` had already been mended once, by a count.** Doc 18 cut IC14 found the world's unbiased
  `select!` took the receipt before the inbound's close about one run in fifteen, and gave the world sixteen
  `yield_now`s before the receipt was sent. Sixteen is a guess at how many turns the world needs; nothing checks it.
- **The two IPC modules are event waits on a real socket**, not clock waits. Every wait reads frames under a
  `timeout` and returns when its frame comes; the sleeps are the pace of a busy retry (25 ms) and of the gateway's
  replay (250 ms), which are what the proofs model, not waits for the server. What was wrong by doc 51's rule §3.7
  was the limits: 10, 15, 20 and 30 s chosen at each call, and 60 s in `ipc_window`. They cannot run on the paused
  clock: the server is another runtime on its own thread, and doc 43c §3.3 keeps that off it.
- **The `--src` census** (`python scripts/wait_audit.py --src`) counted 162 wait sites in mush-server's `src/`, but
  its rule (a file is test code from its first `#[cfg(test)]` to its end) took in production code: the store
  writer's retry sleeps in `persist.rs` (ten sites) and others. The gate needs an exact rule (Built).
- **Which in-`src` waits are row 8's kind.** Row 8's cause was equality over a drain window. The in-`src` tests
  hold three shapes of window: an absence asserted after a fixed sleep (`tests.rs`' hard-mode gate, 20 ms and
  10 ms; the decision tests, 400 to 1,100 ms; `player_market_typed`, two of 200 ms), a "quiet for a second" drain
  (`player_market_story::settle`), and polls under a bound. Only the first is a window in row 8's sense; the polls
  wait for an event.

**Built.**

- **`mush_storykit::idle`** (`src/paused.rs`) and `IDLE_STEP`: on a paused clock, a sleep of tokio's least step,
  which returns only once every task on the runtime has parked. It is the barrier for "the world has taken
  everything ready for it", and an absence is asserted after it. Its doc says what it does not cover: I/O the
  runtime does not own, and a task woken by a timer due inside the step. Three unit tests on the paused clock.
- **Row 10** runs on the paused clock. The stand-in's sleeps are virtual, the test waits for its first tick with
  `idle` (failing if 20 ms of virtual time pass first), and the 50 ms after the stop are fifty of the stand-in's
  periods.
- **`what_still_waits`** runs on the paused clock with `TimeSource::tokio_at(STORY_EPOCH × 1000)`
  (`held_receipt_rig_on`, a new constructor beside `held_receipt_rig`), and waits for the world to park with `idle`
  in place of the sixteen yields.
- **The live hard-mode gate** (`live_hard_modes_block_every_mutating_execution_seam`, row 10's neighbor in the lib)
  runs on the paused clock with the Tokio time source. Its two absences, after a 20 ms and a 10 ms sleep, are
  asserted after `idle`, and its world is set from the time source instead of `unix_now_s()`.
- **`split_boundary`'s other in-process neighbors**: `production_worker_recovers_and_stops_with_world_lifetime`
  (memory store, a worker whose retry is its poll interval) runs on the paused clock, so its 4 s and 1 s bounds are
  virtual; `spawn_ledger_archive_schedules_when_enabled` waits for its gauge through the kit's `wait_until` under
  `STEP_TIMEOUT`, not 200 polls of 5 ms. That one stays on the real clock, because its archive tier reads files.
- **`player_market_typed`'s two absence windows** are asserted after the rig's barrier: `Running::settled`, a name
  for the rig's `barrier_retried`, which the world answers once everything sent before it is answered and any
  commit's output released.
- **The IPC proofs' limits** are `mush_storykit::STEP_TIMEOUT` (60 s): fourteen literals in
  `ipc_authoritative_action`, one constant in `ipc_window`. The 90 s bound of the re-arm proof is kept; it covers
  several bounded preflight rounds, and it is the one wait in the file longer than a step.
- **`wait_budget`'s `src` scope** (`SRC_SCOPE = ["mush-server"]`). Test code under `src/` is a file under a
  `tests` directory, a file named `tests.rs` or `*_tests.rs`, a file fenced by a leading `#![cfg(test)]`, and in any
  other file each `#[cfg(test)]` item, brace-counted (the same `scan::blank_cfg_test_items` the panic budget uses).
  The rows read `src/<path>`. A crate enters the scope by a baseline: its first bless writes the `src/` counts as
  their own low-water marks and the line `# scope src`, and after that a `src/` row rises only with a reason. Two
  new tests: the rule counts test code and never production code, and the baseline happens once.
  `scripts/wait_audit.py --budget` prints the same rows by the same rule.
- **The four rows closed.** `flaky-known.toml` holds 27 rows (31 before), the generated retry block follows, and
  `docs/flaky-tests.md` has row 10's cause and its fix and `what_still_waits`' note. `docs/flaky-observed.md` is
  refreshed.

**Tests.**

**The changed tests, before and after.** "Before" is main `0e3a26804`, "after" this unit. A cell is failed runs over
runs; a run is one test in its own process, as nextest runs it (`--stress-count`, no retry).

| Test | Quiet box, before | Quiet box, after | Contended, before | Contended, after |
|---|---|---|---|---|
| `boot_parity::stop_ledger_archive_…` (row 10) | 0 of 400 | 0 of 400 | **2 of 1,000** | 0 of 1,000 |
| `world_task::tests::what_still_waits_…` | 0 of 200 | 0 of 200 | 0 of 1,000 | 0 of 1,000 |
| `world_task::tests::live_hard_modes_…` | 0 of 200 | 0 of 200 | 0 of 1,000 | 0 of 1,000 |
| `ipc_authoritative_action::*` and `ipc_window::*` (12 tests) | 0 of 288 | 0 of 288 | not run | not run |

"Quiet" is eight busy loops beside one stress run. "Contended" is 32 busy loops, one on every core, with four
stress runs of each test side by side. The two contended failures of row 10 are CI's own text, at the old line:
`the stand-in pass never ran, so this case would prove nothing` (`boot_parity.rs:1248`). The paused tests are also
faster: row 10 from 82 ms to 9 ms a run, the hard-mode gate from 115 ms to 13 ms, `what_still_waits` from 24 ms to
16 ms.

**The barrier, proven load-bearing.** In the clone, with the one `idle` line taken out of `what_still_waits` (so the
receipt is sent with no wait at all, as before IC14's yields): 24 of 200 runs fail, at the test's own
`a queued barrier hears that the world is gone` and its neighbors, which is IC14's race. With the line: 0 of 1,200
above.

**The neighbors.** `production_worker_recovers_…` (now paused) and `spawn_ledger_archive_schedules_…` (now on
`wait_until`): 0 of 200 each. `player_market_typed`'s two converted tests: 0 of 24 each.

**The rest.**

- mush-server's lib, `split_boundary` and `world_task` binaries whole on the final tree: 651 passed, 6 skipped,
  157 s. The lib and `split_boundary` alone on the first patch: 619 passed.
- `mush-storykit`: 59 tests, three of them new.
- `mush-contract-tests`: `ci_policy` 100 passed and 8 ignored (two `wait_budget` tests new), `engine_contracts` 92,
  the lib 35. `flaky_known::` after the bless: 11 passed.
- `python scripts/wait_audit.py --budget` and the budget files agree row for row (142 rows, compared with `diff`).
- `cargo fmt --check` on the three crates; clippy with `-D warnings` over `mush-storykit`, `mush-server` and
  `mush-contract-tests` on every target; rustdoc with `-D warnings` over the kit in both modes.

**`wait_budget`**, mush-server, by the gate's rule (`python scripts/wait_audit.py --budget`, the same counts):

| | Main | This unit |
|---|---:|---:|
| Raw waits in story files (`tests/`) | 68 | 61 |
| Wall-clock reads in story files | 6 | 6 |
| Raw waits in test code under `src/` | 149 (not gated) | 144 |
| Wall-clock reads in test code under `src/` | 32 (not gated) | 31 |
| Raw waits in story files, the tree | 513 | 506 |

The seven `tests/` sites that left: three in row 10 (now paused), three in the durable-outbox worker test (now
paused), one in `boot_store`. The `src/` sites: three in `world_task/tests.rs` (the hard-mode gate's two sleeps and
`what_still_waits`' bound, now paused), two in `player_market_typed`; the clock read is the hard-mode gate's.

**Divergences.** Where this differs from doc 51, or from the brief the unit was cut with:

1. **"Row 10 and its neighbors onto the paused clock" is four tests, not a suite.** The paused clock fits a test
   whose every wait the runtime owns. In `split_boundary` most tests boot the server on a runtime of its own over a
   socket, which doc 43c §3.3 keeps off the paused clock; what was left in process is row 10, the durable-outbox
   worker, and (in the lib) the hard-mode gate and `what_still_waits`.
2. **The IPC modules were not converted to barriers; their limits were.** Their waits are already for an event,
   so the class fix is rule §3.7's one limit. Their rows go on the record's evidence, none failed, and on the runs
   below.
3. **The `src` scope's rule is narrower than `wait_audit.py --src`'s.** The census script's `--src` takes a file
   from its first `#[cfg(test)]` to its end and so counts production retry loops. The gate counts `#[cfg(test)]`
   items only. `--budget` follows the gate; `--src` is left as the census it was, so doc 51 §2's numbers still
   reproduce.
4. **A scope enters by a baseline**, a mechanism doc 51 §3.9 does not have: without it, the first bless would have
   needed a written raise for every one of the 144 sites.
5. **Not every in-`src` window moved.** See Not built.
6. **`idle` is in the kit, which mush-server's lib tests now use** as a dev-dependency they already had.

**Not built.**

- **The decision tests' absence windows** in `world_task/tests.rs` (`worker_saturation_…` 500 ms,
  `malformed_provider_…` and `provider_outage_…` 400 ms, `expired_decision_lease_…` 1,100 ms). The decision
  runtime's leases are on the OS clock, not `TimeSource`, so those tests cannot go on the paused clock until the
  decision runtime reads its time from the provider. That is a product change (doc 43c's TS line), not this unit's.
- **`player_market_story::settle`**, the "quiet for a whole second" drain, and the market stories' fixed waits (31 s
  and 20 s in `part_two` and `part_three`). Those stories are the M7 nightly's, `#[ignore]`d on a pull request, and
  wait for the renewal service's own schedule; a barrier for "the service has nothing more to say" does not exist
  yet.
- **The polls under a bound** across the in-`src` tests (about a hundred sites): each waits for an event and is
  sound by §3.7; moving them to `wait_until` would change no outcome. They are held by the new `src` rows.
- **A `src` scope for any other crate.** `SRC_SCOPE` names mush-server alone; a crate joins with one word and a
  bless.
- **The soak's rate for the suites.** The soak reads this tree from the night it merges; the rates above are
  by-hand runs, as FL.4's were.

## FL.7

The gateway's stories. Built 2026-11-25 on `0e3a26804`. It adds one module to `mush-storykit` (`term.rs`, the one
`Term`), moves mush-gateway's `split_halves` stories onto it, turns on `one_reader` for mush-gateway, and closes
ten of mush-gateway's twelve rows in `flaky-known.toml`; two stay, not renewed (Rows). **It changes
product code in two places.** Rows 12 and 15 were not the rig's wall-clock limits. They were two product faults,
and the restart stories found both once their waits were bounded by the gateway's own schedule.

Every measurement is from conclav.dev, by hand as `ghrunner`, in a clone with its own target directory. Another
session's soak held cores 0 to 23 for part of the time; where this unit's runs used fewer cores, it says so.

**The suite's soak rate.** `scripts/soak.sh` with `SOAK_PACKAGES=mush-gateway`, four cores a cell, no retry.
`split_halves` is 47 tests here, the Linux-only modules among them.

| | Cells | Passes | Failed passes | Tests that failed | A pass, wall |
|---|---:|---:|---:|---|---:|
| Before, main `0e3a26804` | 2, then 6 | 48 | 0 | none | 130 to 207 s |
| The stories converted and the drain fixed, the keeper unchanged | 4 | 24 | 3 | `restart_survival::reply_settings_…`, `::server_self_exit_…`, `::client_disconnect_during_gap_…`, each once: `Reconnected` never came | 146 to 207 s |
| After, this unit | 2, then 6 | 48 | 0 | none | 116 to 223 s |

The middle row is how the keeper fault was found (Rows). **The rates do not prove the fix by themselves.**

- **Load.** The middle soak ran while other sessions built on the box (load up to 22 on 32 cores). The other four
  ran on a quiet box (load at most 7.65).
- **Main did not reproduce the fault** in 48 quiet passes, and neither did the fix.
- **The case for the fix** is the trace, the unit test that hangs without it, and CI's record. Whether the
  conversion itself made the window wider, or only the load did, is not known.

**CI's record, read whole** (every `rest-gateway` JUnit report of `ci.yml`, 2026-11-04 to 11-25: 1,122 runs, 106
entries for `split_halves`). It is larger than `docs/flaky-tests.md`'s count, which read 120 green runs:

- **Restarts that never healed:** 15 entries, all ending at the gateway's `restarting` line and timing out after
  about 61 s. They cover `restart_survival`'s five restart stories and both of row 15's stories, five of the 15 on
  main.
- **A drain that closed before its reason:** 83 entries. `a_finish_warns_once…` alone has 81, all at the same
  read, 60 of them on 11-04 and 21 in the 875 runs after it. `a_drains_while_b_serves…` has two.
- **The skewed-server story** (`server_panic_relaunch::a_version_skewed…`): three entries, all on main, where the
  skewed server waited for the gateway to hang up on it and it never did. That is read as the keeper fault: an
  attempt parked on the server it dialed before never reaches the next one. It is a reading, not reproduced.
- **`server_unit::a_gateway_started…`**: one entry, its channel closed after a gap refusal and a second `connect`.
  It is not explained.
- **`a_drains_while_b_serves…`, once more** (43258253021, 2026-11-12, a branch): a queued-connection burst answered
  15 of 16. That count is the kernel's listener migration (`tcp_migrate_req`), not the drain.
- **`arena_story_split`**: two entries, one of them the security branch's double failure.

**Survey.**

- **Eleven `Term`s, not ten, one shape.** Doc 51 §2.3 counted ten under `split_halves/`; `authoritative_process_kill`
  holds an eleventh. Every copy was an SSH channel and a `String`, with the same `expect_after` loop. They
  differed in their busy lists (one sentence, or two), their retry budgets (8 or 40), whether a retry slept 200 ms,
  their pty (80 by 24 in eight files, 120 by 40 in three), and in eleven file-local helpers.
- **Four more copies of the admission loop.** Five files typed their first login again while the gateway answered
  that its server half was not up yet, in 200-ms steps, each with its own constant for the sentence.
- **The restart waits were already 60 s.** Every `Reconnected` was read under the file's `STEP_TIMEOUT`, the same
  60 s as the kit's. A literal limit was not what failed. Row 15's message, `timeout waiting for "Reconnected"`, is a
  60-s wait running out.
- **The gateway's schedule had no public figure.** `RECONNECT_TIMEOUT`, `RESYNC_TIMEOUT`, `BACKOFF_CAP` and
  `KEEPER_STOP_WAIT` are private constants of `mush-gateway/src/lib.rs`.

**Built.**

- **`mush_storykit::Term`** (`term.rs`): `Term::play(addr)` (the passwordless `play` user, a 120-by-40 pty, the
  owner's floor) and `play_sized`; `mark`, `raw`, `since`; `send_line`; `expect`, `expect_after`,
  `expect_one_of_after` and `expect_one_of_within`; `say`, which types a line again on any sentence of FL.5's one table (`BUSY_NEEDLES`);
  `ask`; `expect_close`. It is the barriers' `Session`, so `ask_whole` and `settled` take it. `strip_ansi` and
  `AcceptAll` are its one copy each.
- **`Term::across_restart(heal, restart)`** and **`Term::healed_after(mark, heal)`**. The rig's restart runs under
  its own bound event and limit. Then the helper reads `restarting` and `Reconnected` from the mark, within `heal`
  under the load factor, and calls `settled`. `reconnected_after` is the same without the sentinel, for the one
  story whose session frames every reply.
- **`mush_gateway::HEAL_WITHIN`**, the product's figure for one heal once a server answers: the old writer awaited,
  one paced pause, one dial and handshake, one resync. That is 27 s, so a story waits 54 s. A unit test holds the
  figure to the steps it sums.
- **The rig** (`split_halves/harness.rs`): the kit's re-exports, `HEAL`, and `when_admitted`, the one admission
  loop. It types a line again while the answer is one of the two not-up sentences.
- **The stories.** All eleven files are on the kit's `Term`. Every restart's wait is `healed_after` or
  `across_restart`, and a line typed after a heal goes through `say`. File-local helpers are free functions over
  `&mut Term`. `arena_custody_split`'s own `wait_until` is now the kit's.
- **`one_reader`** (`mush-contract-tests`, `ci_policy/one_reader.rs`). It refuses a `struct Term`, `fn ask_whole`
  or `fn strip_ansi` under the `tests/` of a crate on its list, which is mush-gateway. It also requires the kit to
  hold exactly one of each.

**The two product fixes.**

1. **A reconnect attempt waited for ever on a Hello** (`mush-gateway`, `connect_and_wire`).
   - The keeper bounded an attempt's dial loop by `RECONNECT_TIMEOUT`, but not one dial, and not the read of the
     server's Hello after a dial that connected.
   - In the converted soak, three restarts in 24 passes never healed. One run's trace shows the gap open at
     `boundary writer closing (deliberate restart)`, the fresh server up 0.74 s later, and no keeper line at all
     for the 54 s after. No dial failed and none was retried, so the attempt was parked.
   - The first dial after a restart can reach the server half that is stopping, which in this rig is a runtime being
     dropped. Which of its tasks held the accepted socket open without greeting is not established.
   - The fix holds the dial, both Hellos and the dial loop to the attempt's one deadline. An expiry is
     `Unreachable`, so the next attempt follows at once.
   - `a_peer_that_never_says_hello_ends_the_attempt_inside_its_budget` accepts and never writes. With the fix it
     ends in 0.3 s. Without it the attempt is still waiting when the test's own 10-s bound ends it.
   - A production server whose accept loop lives and whose handshake is wedged would strand every held player the
     same way. There is no sign that one has.
2. **A drain's close was cut before it reached the wire** (`mush-drain`, `Drain::run` and the lifecycle).
   - A session leaves the live set when its close is queued. `Drain::run` raced its own `close_all` against the
     generation emptying. The lifecycle returned on "empty" while the drain's task was inside the close's 500-ms
     flush, so the gateway's runtime dropped with the close reason unwritten.
   - CI run 44774353324: `channel closed waiting for "This gateway is restarting; …"`, the transcript ending at the
     warning.
   - The close now runs outside the race, and the lifecycle does not end on "empty" while the drain's task runs.
   - Two tests, a finish and a drain whose session drops its guard in its own close as the gateway's does, fail
     without the fix and pass with it.
   - The doorway uses the same crate and gets the same fix.

**The counts.**

| | Before | After |
|---|---:|---:|
| Raw waits in mush-gateway story files (`wait_budget/mush-gateway.txt`) | 86 | 55 |
| `struct Term` definitions under `crates/mush-gateway/tests/` | 11 | 0 |
| `strip_ansi` definitions there | 11 | 0 |
| `ask_whole` definitions there | 1 | 0 |
| mush-gateway's rows in `flaky-known.toml` | 12 | 2, not renewed |

`scripts/wait_audit.py` agrees with the budget: 55 waits and 2 wall-clock reads in mush-gateway's story files.

**Rows.**

- **Row 12, five tests, re-described.** The class is still process, but the cause is not the rig's limits:
  - `restart_survival`'s two and `server_unit`'s heal are the keeper fault.
  - `gateway_generations::a_finish_warns_once…` is the drain fault. CI's one failure of it carries the drain's
    message, and its retried pass came from the same run.
  - Its twin, `a_drains_while_b_serves…`, keeps its row, not renewed. Two of its three entries are the drain
    fault. The third, the burst of 15 of 16, is not explained.
- **Row 15, two tests, read as row 12's first fault.** `timeout waiting for "Reconnected"` after a restart is what
  the keeper fault prints, and the converted soak printed it in this binary's restart stories. Its two runs are
  from 2026-11-06 and their logs are gone, so that is a reading of the symptom, not a reproduction.
- **One more test of the fault, never listed:** `restart_survival::server_self_exit_auto_restarts_without_dropping_players`
  failed once and passed on its same head on 2026-11-23 (run 45277135577). It failed in the converted soak the
  same way.
- **Four of the five carried module rows are deleted** (`two_process_e2e`, `server_panic_relaunch`, `two_gateways`,
  `gateway_generations_unit`). They passed in every soak pass above. `server_panic_relaunch`'s three CI entries are
  read as the keeper fault (the record, above).
- **`arena_story_split::*` keeps its carried row, not renewed** (it expires 2026-12-07 with the rest).
  - Its one story, `the_story_is_the_same_story_through_the_gateway`, passed all 96 soak passes.
  - It then failed once, at 54 s, in one 480-test run of the tree merged with main `ecc6887b8`, beside
    mush-contract-tests' and the kit's tests at 24 threads. That run kept no message.
  - The ten runs of the same shape after it passed, and so did it alone and four at once.
  - Its cause is not known, so the row stays for the soak to name one. The row says so in its note.
- **Not a flake:** `arena_story_split::the_story_is_the_same_story_through_the_gateway` failed both tries in run
  44774353324, at a security branch's head (`fix/security-tier1d-tacsim-highs`), and nowhere else.
- **Ten of the twelve rows are deleted.** The two kept are not renewed; both expire 2026-12-07.

**Tests.**

- `mush-drain`: 29 unit tests, the two new ones among them.
- `mush-gateway` lib: `the_heal_figure_covers_every_step_of_one_heal` and
  `a_peer_that_never_says_hello_ends_the_attempt_inside_its_budget`.
- `mush-storykit`: its unit tests, `strip_ansi`'s two new ones among them.
- `split_halves` whole, sixteen threads, no retry: 47 of 47.
- `restart_survival` alone, traced, 204 runs on six cells before the keeper fix: one failure, the trace above.
- `mush-contract-tests`' `ci_policy`: `one_reader`, `wait_budget` (re-blessed down), `flaky_known` (re-blessed),
  `restriction_lints` (the kit's `panic` count, 16 to 29) and `panic_budget`.
- `cargo fmt`, clippy with `-D warnings` over mush-gateway, mush-storykit and mush-drain, every target, and rustdoc.

**Divergences.**

1. **Two product changes**, where doc 51 §5 has FL.7 change only tests and the kit. Both were found by the unit's
   own soak or by CI's record. They are the rows' causes, and a rig cannot fix either.
2. **The heal limit is one product figure, not `Figures`.** Doc 51 §3.5 sketches the limit "through `Figures`".
   `Figures` reads the game's content, and the heal is the gateway's, so it is a `pub const` of the gateway. The kit
   takes it as a `Duration` and does not link the gateway.
3. **`across_restart` is used once.** Most restart stories stop, do something in the gap, and then start, which one
   future cannot hold, so they call `healed_after` with the mark taken before the stop.
4. **The heal also asserts that `restarting` came first.** The stories each read both lines already, and the
   helper keeps that.
5. **Every story's pty is 120 by 40**, where eight files asked for 80 by 24: the owner's floor. No assertion here
   reads a wrapped line.
6. **The kit's `say` types a line again on every busy sentence of FL.5's table**, where four files knew only the
   commit one and the rest two. A busy answer is never the needle a story waits for.

**Not built.**

- **The rest of each file's rig.** `scratch_dir`, `test_config`, `seeded_gw_store` and `start_split` are still one
  copy a file. Most of the raw waits left are theirs, such as `timeout(Duration::from_secs(30), ssh_rx)` on a
  gateway's bind.
- **`web_restart_survival`.** It reads a web socket, not a `Term`, and its restart wait is still `STEP_TIMEOUT`.
- **`authoritative_process_kill`'s restart.** Its worker is a re-exec of the test binary that boots from Redis and
  is started without a bound event, so the heal figure does not bound it. It keeps its step limit and its 5-s
  sleep.
- **`after_tick`.** `arena_custody_split` still sleeps 2.5 s for "two more boarding ticks"; that barrier is FL.8's.
- **The holder of the silent socket.** The fix bounds the attempt. Which task of a dropping runtime kept the
  accepted socket open is not traced.

**For the owner.**

- **Q-FL7-1.** Two product fixes landed in a test unit (Divergences, 1). Default: keep them here, since the rows
  cannot close without them.

## FL.8

mushd's readers. Built 2026-11-26 on main `2f3101aec` (FL.7, #806, merged). It moves every mushd story onto the kit's
`Term` and its readers, adds one module to `mush-storykit` (`readout.rs`), builds `after_tick`, fixes row 14, turns
on `one_reader` for mushd, and deletes mushd's seven rows in `flaky-known.toml`. **No product code changes.**

Every measurement is from conclav.dev, by hand as `ghrunner`, in a clone with its own target directory.

**The suite's soak rate.** `scripts/soak.sh` with `SOAK_PACKAGES=mushd` (`sim_lenses`, `world_journeys`,
`durable_process`), six cells of four cores, four rounds each, no retry:

| | Passes | Failed passes | Tests that failed | A pass, wall | A pass, summed test time |
|---|---:|---:|---|---:|---:|
| Before, main `2f3101aec` | 24 | 0 | none | 269 to 282 s | 1,019 to 1,054 s |
| After, this unit | 24 | 0 | none | 271 to 284 s | 1,026 to 1,055 s |

Each pass is 248 tests: the binaries' Redis-gated tests skip without `REDIS_URL`, as in the nightly soak. The six
cells ran on cores 0 to 23 of a quiet box (load at most 12.9 on 32 cores).

**Survey.**

- **Fifteen `say`, twelve `ask` and ten `ask_whole`** in `sim_lenses`, as doc 51 §5 counted, behind 607 `say` and
  247 `ask` calls. They differed in:
  - their busy lists: one sentence, two, or the fragment `please retry in a moment`;
  - their budgets: 8 to 40 tries;
  - whether a retry slept 200 ms.

  `observer_rule`'s `ask_whole` drained a refused try's sentinel before it typed again. The other nine did not
  (FL.3's hazard).
- **Four `Term`s, not one.** `tests/support` held mushd's. `durable_process/server.rs`, `world_fault.rs` and
  `world_journeys/ssh_e2e.rs` each held a private one, with its own `strip_ansi` and `AcceptAll`. `one_reader`
  refuses all four.
- **mushd's `Term` had four things the kit's lacked:**
  - a public `seen`, which about 300 reads sliced by hand (`ada.seen[mark..]`);
  - `room_for`, over a copy of the flood bucket;
  - `expect_squeezed_after`, for the `@tui` screen;
  - `resize`.
- **Six files typed their own sentinel word** (`zzq-solo-sentinel`, `zzz-sentinel` and others), and their line
  readers dropped that word's echo. The kit types `zzq-sentinel`.
- **`@ps` cannot carry `after_tick`.** The one tick a story waits on is tacsim's boarding tick (`tacsim.board`,
  `BOARD_TICK_PERIOD`, 1 s). `render_ps` prints whole seconds, `(due - now) / 1000`, so the tick reads
  `next in 0s` at every instant (Q-51-11).
- **Row 14's twin never failed.** `metrics_healthz_and_stats…` asks `/healthz` after its session's answers.
  `spa_if_built…` asked it straight after the boot.

**Built.**

- **The kit.** `readout.rs` is FL.8's own module. The edits elsewhere are small:
  - **`Term::ask_settled(line, needle)`**: `say`, then `settled`, and the text from the answered try on, with the
    sentinel's echo and answers left out. Every story `ask` is this now, so a parse follows a whole read (rule S).
    The needle may be an event after the line's reply, which `ask_whole` alone cannot wait for.
  - **`ask_whole` counts its answers.** It is FL.5's `ask` read, answering text through the refusal as the copies
    did. A refused try's sentinel can no longer end the next try's read.
  - **`Term::say`** waits `POLL_INTERVAL` before typing a refused line again. It also reports the mark of the try
    that was answered, so `ask` and `ask_settled` read from there and not from a refused try.
  - **`FLOODED`**: the flood bucket's `You're doing that too fast.`, which `is_busy` and `busy_needles()` now
    know (Divergences, 2). Its test holds the sentence to the two product files that write it.
  - **`Term::expect_squeezed_after`, `Term::resize` and `Term::sent_at`** (the instants the session typed). `Term`
    keeps that list. Its fields are `pub(crate)` for this module.
  - **`after_ticks(staff, period, n)`**, doc 51's `after_tick`. Staff queues `@wait <n periods + 1 s>=think
    zzq-tick` and waits for the thought. The scheduler fires its entries in `(due, seq)` order
    (`mush_core::sched::Scheduler::pop_due`), so every tick due before the thought has fired by then.
- **mushd's rig** (`tests/support`):
  - the kit's `Term`, `AcceptAll`, `strip_ansi`, `ask_whole` and `after_ticks`, re-exported;
  - `play_client_sized` is `Term::play_sized`;
  - `room_for(&term, lines)` replays `sent_at()` through the server's own `FloodBucket` at its own figures, as
    FL.3's copy of the bucket did.
- **The stories.**
  - All 37 private readers are deleted. `say(&mut t, …)` is `t.say(…)`, `ask(…)` is `t.ask_settled(…)`, and
    `x.seen` is `x.raw()`.
  - The six private sentinel words are the kit's.
  - Two hand-written sentinel loops are `settled`: `m5_fog_fence`, and one in the attach story.
  - The three private `Term`s are the rig's. `world_fault`'s login reads under its own child-process limit with
    `expect_one_of_within`.
- **`after_ticks` in use.** Two stories slept 2.5 s for "two more boarding ticks" after a restart:
  `observer_rule::af3_…` and `sim_rematch::a_restart_mid_match…`. Both wait for two ticks of
  `BOARD_TICK_PERIOD` now.
- **Row 14** (Rows), and row 21, found in CI's record on the way (Rows).
- **`one_reader`** lists mushd beside mush-gateway.

**The counts.**

| | Before | After |
|---|---:|---:|
| `say` / `ask` / `ask_whole` definitions in `sim_lenses` | 15 / 12 / 10 | 0 |
| `struct Term` / `strip_ansi` / `AcceptAll` under `crates/mushd/tests/` | 4 / 4 / 4 | 0 |
| Raw waits in mushd's story files (`wait_budget/mushd.txt`) | 105 | 73 |
| Literal spans passed to a wait in them (FL.9's counter, after main was merged in) | 80 | 55 |
| mushd's rows in `flaky-known.toml` | 7 | 0 |

`scripts/wait_audit.py --budget` agrees: 73 waits and 5 wall-clock reads in mushd's story files.

**Rows.**

- **Row 14, re-described as process, and fixed.**
  - `ok` needs the world loop's heartbeat. The loop stamps it at the start of every iteration
    (`world_task.rs`, `set_world_loop_iteration`), and a rig asked within milliseconds of its boot may not have
    run one.
  - The story had a 5-s poll for it. It now logs in staff, calls `settled` and asks once. The world has answered
    by then, so it has run iterations and stamped them.
  - This is a reading of the code and of the twin, not a reproduction. The three CI failures are from 2026-11-05
    to 11-12.
- **Row 21 is new**: `observer_rule::af3_the_resume_names_nothing…`, one CI failure on a branch
  (45426726188, 2026-11-24), which `flake_report.py` listed with no record. Stream: Bo's `has connected.` landed
  inside Ada's refusal read. Fixed with a `settled` before the read. Never listed in `flaky-known.toml`.
- **The six carried rows are deleted**: `scripting_e2e`, `sim_reflex`, `load_soak` and `world_fault`, and the two
  narrowed tests, `relay_bridge::r6b_…` and `arena_story::two_pilots_walk_in…`. None has a failure on record.
  - The four modules and two tests ran in every one of the 24 soak passes, and passed.
  - The Redis-gated ones (`relay_bridge`, `load_soak`, `world_fault`, `persistence`, `durability_e2e`, `m10c_e2e`)
    ran against a throwaway Redis 8.10 on port 6391: 50 of 50.

**Tests.**

- `mush-storykit`: 74 unit tests and 4 doc tests. The new ones:
  - `after_ticks`' two, on the line it queues;
  - the sentinel left out of a settled read, a flooded sentinel among them;
  - the flood sentence held to its sources.
- mushd whole (`-P soak`, sixteen threads, no retry), and its Redis-gated part (one thread), as above.
- `split_halves` whole, sixteen threads, no retry: 47 of 47. The kit's `say` changed under it.
- `mush-contract-tests`' `ci_policy`: `one_reader`, `wait_budget` (re-blessed down), `flaky_known` (re-blessed),
  `restriction_lints` and `panic_budget`.
- `cargo fmt`; clippy with `-D warnings` over mushd's tests, mush-storykit and mush-contract-tests, every target,
  and over mush-gateway and mush-bot once; rustdoc.

**What the conversion found.** The first run of the converted suite failed 13 of 261 tests. Every one was the kit
meeting something the private readers had hidden:

- **The flood bucket.**
  - A whole read types two lines where a needle read typed one.
  - Under a fast battery, the session's bucket (10 lines a second, a burst of 30) dropped a sentinel with
    `You're doing that too fast.`.
  - No reader knew that sentence, so the read waited 60 s for a refusal that never came. Four tests failed this
    way, and one more after the first fix.
- **A refused try's text in the reply.** `Term::ask` read from the first try's mark. A flooded or busy first try's
  answer was then part of what the story compared (`You're doing that too fast.\r\nVIEW BW…`).
- **A queued line has no one to answer.** `after_ticks`' first version queued a nonsense word and waited for its
  `Huh?`. A line the scheduler runs has no connection, so the refusal reached no one. A `think` does reach the
  actor.
- **Six private sentinel words.** The files' own line readers dropped their echo, and did not drop the kit's.
- **One needle in its own echo**: `typeahead`'s `say The tower is listening.`, which the private `say` had not
  checked.

Each of the next three runs found the next layer. The fourth was clean.

**Divergences.**

1. **`ask` became `ask_settled`, not `ask_whole`.** Doc 51 §5 says "every parse after a needle read moved to
   `ask_whole`". Many of the 247 wait for an event the line causes later, such as a lock confirmed at the next
   pulse. So the needle is still awaited, and then the read is closed with `settled`. It is whole by the same
   ordering argument, and it does not change what a story waits for.
2. **The flood sentence is not a row of `BUSY_SENTENCES`.**
   - That table is also the bot's. mush-bot's `the_bots_busy_sentences_are_the_story_kits` requires the bot's
     readers to type a line again after every row, and the bot does not know the flood sentence.
   - Adding the row would need a bot product change. So `FLOODED` sits beside the table, and the kit's own
     readers (`is_busy`, `busy_needles()`) know both.
   - `BUSY_NEEDLES`, which is the bot's `TRANSIENTS`, is unchanged.
3. **`after_tick` reads the scheduler, not `@ps`.** Doc 51 §3.3 sketched it over `@ps`'s next fire, and whole
   seconds cannot tell one fire of a 1-s tick from the next. That is Q-51-11's report, now written into doc 51 §6.
   The name is `after_ticks(staff, period, n)`, and the period is tacsim's own `BOARD_TICK_PERIOD`.
4. **mushd's ptys stay 80 by 24.** The rig's line-mode stories pin the line client's wrap at eighty columns, as
   `play_client`'s comment says. FL.7 moved the gateway's to 120 by 40. Doing that here would move every wrapped
   read at once, which is not this unit's work.
5. **`Term::say` now pauses 200 ms before a retry**, where FL.7's typed again at once. A flooded line typed at
   once is refused again before a token comes back. The gateway's suite passed whole with the pause.

**Not built.**

- **`arena_custody_split`'s 2.5-s sleep** (mush-gateway). It has no staff session to queue the thought on.
- **Busy loops that are not one of the three readers**: `observer_rule`'s panel and TUI loops,
  `whole_macro_answer`, and the files' `COMMIT_PENDING` needles beside an `expect_one_of_after`. They keep their
  private sentence fragments. The next unit to touch them should read `is_busy`.
- **The web-socket drains** of the mind feed, which are FL.9's kind.

**For the owner.**

- **Q-FL8-1.** The flood bucket's refusal is a busy answer to the kit's readers but not to the bot's
  (Divergences, 2). Default: leave the bot as it is. A bot that typed fast enough to meet the bucket would want it.

## FL.9

The small crates. Built 2026-11-25 on `6b63cb8be` (FL.7's branch, PR #806, with main merged in), stacked on FL.7
by the owner's word of 2026-11-26; #806 merged before this unit's pull request opened, and main was merged in
(it brought only Unity files). It adds one module to `mush-storykit` (`step.rs`), moves the stories of
mush-net-web, mush-net-ssh and mush-sshdoorway onto the kit, and builds the `span` counter of `wait_budget`, which
FL.1 left a stub. **It changes no product code.** None of the three crates had a row in `flaky-known.toml`, so no
row closes.

**The suites' soak rate.** None of the three is in `scripts/soak.sh`'s list, so the soak was by hand on
conclav.dev. It ran the story binaries (mush-net-web's `http`, `ws` and `tls_serve` with `--features tls`,
mush-net-ssh's `loopback`, mush-sshdoorway's `e2e`; 146 tests) with `--retries 0` and eight threads. The base ran
on cores 0 to 15 and this unit on 16 to 31, at the same time.

| | Passes | Failed passes | Tests that failed | A pass, summed test time (median) |
|---|---:|---:|---|---:|
| Before, `6b63cb8be` | 24 | 0 | none | 73.9 s |
| After, this unit | 24 | 0 | none | 74.7 s |

**Neither soak found a flake, and CI's record has none for these crates.** `docs/flaky-observed.md` held one row
for them, mush-net-ssh's `tui::port_walk::tests::corridor_destinations_are_exactly_the_lanes_venue_words`
(2026-11-20, two branches), and it is not a flake:

- Both runs (44701582254 and 44694457791) were on doc 50 branches.
- Each branch added the `isidis` port to `tacsim/content/lane/ceres_phobos.json` before the TUI's `CORRIDORS`
  had it, and the assertion prints the one missing word.
- The test reads two files and no clock.

So this unit's case is the rules, not a soak rate. Every wait below was a sleep, or a limit chosen at the call,
that a slower box could break.

**Survey.** The three crates' story files held 65 raw waits, 4 wall-clock reads and 70 literal spans passed to a
wait. The span count is by this unit's counter, run at the base.

- **mush-net-web** (33 waits):
  - `ws.rs`: five 50-ms sleeps before "never attached, so no disconnect", a 400-ms loop answering pings, and a
    200-ms window for "no binary frame".
  - `http.rs`: two 50-ms sleeps "to let the accept loop take the permits", three after a dropped socket, and a
    300-ms and a 600-ms sleep to outlive a request timeout and a connection age.
  - `tls_serve.rs`: a retry loop of 100 times 50 ms.
  - Every read was under a 2-, 3- or 10-s limit, and the rig's `wait_for` polled at 10 ms for 3 s.
- **mush-net-ssh** (25 waits, 1 clock read):
  - Three hand-rolled poll loops.
  - An 1,100-ms sleep for the flood bucket to refill.
  - Three sleeps past an accept deadline before "still open" or "still held".
  - 80- and 50-ms sleeps before "the pre-shell data did nothing" and "no second attach".
  - 150 ms "to let the SFTP inbound overflow", 120 ms "for the held queries to run", and 5-ms pacing in a flood.
  - Its own `STEP_TIMEOUT` of 15 s, and a host key path named by the wall clock's nanoseconds.
- **mush-sshdoorway** (7 waits in `e2e.rs`; the rig held the rest):
  - A 200-ms sleep after `drain.begin()` before "new connections are refused".
  - A 2,250-ms sleep in `cockpit.rs` to outlive the idle grace. It is a rig file, so it was never counted.
  - Two poll loops of 20 ms.
  - The rig's `wait_for_secs`, with limits of 6 to 30 s chosen at each call.
  - Its data directory, named by the wall clock.

**Built.**

- **The kit's `step.rs`.**
  - **`within(label, event)`** and **`within_for(bound, label, event)`**: one event under the kit's one step
    limit, or a bound the caller derives. A failure names the event.
  - **`at_most(span, work)`**, for a future whose end is not the point (the doorway's flood).
  - **`round_trip(handle)`**: a `tcpip-forward` request, which no server here grants, and its answer. An SSH
    server's session loop takes one connection's packets in order and awaits each handler before the next. So the
    answer proves that every packet sent before it has been handled.
  - Four unit tests on the paused clock.
- **mush-net-web.** The rig's `wait_for` is the kit's loop.
  - **`sockets_ended(n)`** is the absence mark. The slot guard is the last thing a socket task drops, after its one
    `disconnected` call, so once `n` slots are back no disconnect is still to come.
  - In `http.rs` the permits need no wait. One accept loop takes sockets in handshake order and acquires a
    socket's permit before it accepts the next.
  - A freed slot is waited for as the fact, a fresh `/healthz` served (`until_served`).
  - "Survives past the request timeout" and "past the connection age" read the socket's first keepalive ping. The
    ping is set one interval past the timeout, so it is the server's own mark of time.
  - In `ws.rs` the ping test answers five times the missed-pong cutoff in pings.
  - "No binary frame" is asserted between two marks: a line that reached the game after the picture requests, and
    the game's next envelope.
- **mush-net-ssh** (`loopback.rs`, still one file; its rig did not move).
  - The kit's `STEP_TIMEOUT` replaces the file's 15 s, so a limit is 60 s.
  - The flood bucket's refill is the server's own answer: the story types the guess again while it is "too fast".
  - **`DeadlineMark`** marks a deadline's end without a sleep. A second server on the same deadline, with hooks of
    its own, accepts one client after the story's. When that client's slot is back, the story's deadline has run
    out too. The stalled variant is cut only after the grace, so it marks that as well.
  - The pre-shell data and the second shell are asserted after a `round_trip`.
  - The SFTP overflow is asserted once a line sent on the game channel after the flood has reached the game.
  - The burst cap is asserted once a TUI line typed after the burst has. The connection's one command worker takes
    its queue in order.
  - `Term::read_until` keeps the terminal read while it waits for a hook.
  - The host key path comes from `scratch_dir`.
- **mush-sshdoorway's rig onto the kit.**
  - `wait_for` is the kit's loop under its step limit, and `wait_for_secs` is gone.
  - Every read and reply is under the step limit.
  - The data directory comes from `scratch_dir`.
  - The rig keeps the accept loop's task. **`Rig::stopped_serving`** awaits it: the event that the listener is
    closed after `drain.begin()`.
  - **`Rig::deadline_mark`** is the doorway's deadline mark, a client cut at its auth deadline. `cockpit.rs`
    asserts "the shell outlives the idle grace" after a `round_trip` and that mark.
- **The `span` counter**, in `wait_budget` and in `scripts/wait_audit.py --budget`, by one rule.
  - A `Duration::from_<unit>(<number>)` counts when it is written in the argument list of a raw wait, or of a kit
    wait (`wait_until`, `wait_for`, `within`, `within_for`, `at_most`, a method call included).
  - It does not count inside a block passed among the arguments. The waits in the block count their own.
  - A budget file written before the counter is baselined at its first bless and gains `# spans counted`.
  - A crate with no file gets no baseline, so its spans rise from zero.
  - Two unit tests: the rule, and the baseline once.

**The counts.** These are `wait_budget`'s, and `scripts/wait_audit.py --budget` agrees with them file for file.

| Story files of | Raw waits, before | after | Clock reads, before | after | Literal spans, before | after |
|---|---:|---:|---:|---:|---:|---:|
| mush-net-web | 33 | 1 | 3 | 3 | 32 | 0 |
| mush-net-ssh | 25 | 7 | 1 | 0 | 31 | 2 |
| mush-sshdoorway | 7 | 0 | 0 | 0 | 7 | 1 |
| **the tree** | 608 | 551 | 78 | 77 | 488 | **421** |

421 is the literal-span baseline that every budget file now carries. What stays, and why:

- **mush-net-web's one wait** is the mock's `login_delay` sleep, the slow hook a request timeout is tested
  against.
- **Its three clock reads** set the world loop's last iteration to now, which `/healthz` compares with the wall
  clock.
- **mush-net-ssh's seven waits** are `Term`'s three needle reads, `read_to_end_or_reset`, `exec_result`'s bound,
  the local `wait_until`'s poll, and the `^D` burst's read. Each is under one deadline.
- **Its two spans** are the local `wait_until`'s 25-ms poll and the over-cap story's 6 s, the product's
  five-second cut asserted as timing.
- **The doorway's one span** is the flood's 40 s, under `at_most`.

**Tests.** On conclav.dev, in a clone of `6b63cb8be` with the patch:

- `cargo nextest run -p mush-storykit -p mush-net-web -p mush-net-ssh -p mush-sshdoorway`: 784 of 784.
- `-p mush-net-web --features tls --test tls_serve`: 3 of 3.
- `mush-contract-tests --test ci_policy`: 105 of 105 after the bless. That includes `wait_budget`, `panic_budget`,
  `flaky_known`, and `restriction_lints` with the kit's `panic` count moved from 29 to 30, measured with clippy.
- clippy with `-D warnings` over the five crates, every target, with `mush-net-web/tls` on.
- `cargo fmt`, and the soak above.

**Divergences.**

1. **Not every bounded read moved, and no rig moved into `tests/support/`.** `loopback.rs` holds mush-net-ssh's
   mock and `Term` in its own 3,900 lines. Moving them would make their waits rig waits and lower the count
   without changing a wait, so the bounded reads stay, counted.
2. **mush-net-ssh keeps its own `Term`.** Its stories reach into `term.channel` and `term.seen` about ninety times,
   and its sessions authenticate four ways at 80 by 24. `one_reader` is not turned on for it; that would be a unit
   of its own.
3. **Three marks are a second connection.** `DeadlineMark` and `Rig::deadline_mark` lean on two facts: a deadline
   starts at accept, and a server's timers fire in order. They give no bound tighter than the deadline, so "cut
   within five seconds" stays a timing assertion with its literal.
4. **The kit's waits poll at 200 ms**, where the net-web rig polled at 10 ms and the doorway's at 20. The summed
   test time moved 0.8 s.
5. **`round_trip` sends a request no server here grants.** If one ever granted `tcpip-forward`, the barrier would
   still hold: a grant is an answer too.

**Not built.**

- The three crates in `scripts/soak.sh`'s list. Their stories now wait for events, and a pass is about 75 s
  summed, so adding them is cheap. FL.10 can decide.
- `one_reader` for mush-net-ssh (Divergences, 2).
- mush-net-ssh's rig in `tests/support/`, which would let `market_screen.rs` and the other screen tests share its
  mock.
