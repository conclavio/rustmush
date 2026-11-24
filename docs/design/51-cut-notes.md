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
