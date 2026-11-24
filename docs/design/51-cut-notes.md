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
| Before, main `b0c0e6832` | 96 | 3 | `nothing_of_the_shooting…` twice, `the_switch_names_a_contact…` once | 286 s | 1,077 s |
| After, this unit | @@PASSES@@ | @@FAILED@@ | @@WHICH@@ | @@WALL@@ s | @@SUMMED@@ s |

Both soaks ran beside other sessions' builds on the box (load 20 to 35 of 32 cores).

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
    `@arena/status`'s row that undoes the terminal's wrapping;
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
  of main caught each once. `nothing_about_an_undetected_unit…` was never caught, in CI's record
  or in either soak; what it held of this class was the four hand-written sleeps.
- **Row 16 is re-described: dice.** The failed run's record shows the loser eliminated at 43 s
  with its hull and turret standing, the turret at 10 of 150 inside after three hits, and every
  part intact. Two survivable turret hits each jammed one of the turret's two guns (doc 36 cut
  GC.2, one chance in twenty a hit), a side with no gun left is out (16b cut AP.2), and the sim
  center's check-in returns parts unjammed. `damage.rs`' own tests prove each step. It is a
  reading of the record and the code, not a reproduction: the chance is about one match in
  three thousand.
- **Row 19 is new**: `the_switch_names_a_contact…`, one soak pass of main, a stream fault, fixed.

**Tests.** All on conclav.dev, in clones of `b0c0e6832` with this unit's patch.

- `mush-storykit`: @@KIT_TESTS@@ unit tests and @@KIT_DOCS@@ doc tests.
- tacsim: `fl3_a_pinned_seed_opens_every_match_and_none_keeps_the_os_draw`,
  `fl3_the_pinned_seed_is_no_key_of_the_encoded_arena_block`, and
  `af1_the_snapshot_shape_is_pinned` with its neighbors, unmoved. mush-server:
  `fl3_no_config_key_pins_a_match_seed`.
- `sim_lenses` whole, one cell's shape (four cores, four threads, no retry): 97 of 97, 279 s.
- **A stress run found a hole in the first conversion.** Thirty-two duel stories at once on
  eight cores: the cockpit answers `Firing cannon.` off its last frame, which under that load was
  seconds old, so it accepted 24 `FIRE`s in a row; the loop counted the magazine spent after one
  real shot and the story timed out. Main's loop had the same hole behind a 400-ms sleep. The
  three shooting loops now count a round when the arena has taken it (the magazine's row moves)
  and type it again when the arena says `FIRE cannon refused`. Sixteen of sixteen passed under
  the same load afterwards.
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
8. **Two stream fixes that are FL.8's kind.** `SCAN` read whole, and row 19's wait for the lock's
   own line. The soak named both in this suite, so they are fixed here.
9. **`observer_rule`'s sentinel word is the kit's.** Its lenses strip the sentinel's echo from a
   transcript, and `settled` types the kit's word.
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
