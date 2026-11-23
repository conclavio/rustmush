# 51. Flaky tests, removed by class — the audit of waits, the shared helpers, the ratchet and the detection loop

Draft 0.2, 2026-11-22, docs and one read-only script. Written against `origin/main` at `6f35070f8`.
It extends [doc 43](43-testing-strategy.md), the testing strategy, and its clock-seam note
[43c](43c-the-clock-seam.md). It changes no test and no product code. Draft 0.1, the same day, was
the audit and the design with every question open; it merged as #771.

**Draft 0.2 records the owner's ruling of 2026-11-22.** Asked "Take doc 51's proposed defaults for
all eleven questions and cut FL.1 and FL.2?", the owner answered: "Take defaults, cut both".
- **All eleven questions are ruled**, each accepted as written and marked **Ruled** in §6. Where
  §3 and §4 say "proposed" beside a question's number, read it as ruled.
- **FL.1 and FL.2 are cut** (§5). **FL.3 to FL.10 are not**; each waits for a further word from
  the owner.
- Nothing else in the document changed from Draft 0.1.

**The questions, each ruled at its default** (each in full in §6, with the alternative it was
weighed against and the reason):

| | Question | Ruled 2026-11-22, as proposed |
|---|---|---|
| Q-51-1 | Where the shared story helpers live | a new dev-only crate, `mush-storykit` |
| Q-51-2 | A pinned dice seed for a sim-center match | yes: `ArenaConfig::seed`, the twin of `FieldConfig::seed`, which no config file can set |
| Q-51-3 | The odds a chance-sampling story may fail by bad luck | stated in the test; at most 1 in 1,000,000 a run on the pull-request tier |
| Q-51-4 | An end-of-reply marker in player text, for the bot | none now; the test rigs use the sentinel line; `OUTPUTSUFFIX` is a later unit if the bot's own reads need it |
| Q-51-5 | The retry policy | no blanket retry; one retry only for tests on a generated list whose rows expire after 14 days |
| Q-51-6 | What the ratchet counts | story files only; raw waits, wall-clock reads and literal spans passed to a wait |
| Q-51-7 | The soak's schedule and size on conclav.dev | nightly, six cells of four cores, about three hours |
| Q-51-8 | Where the observed-flake table lives | a generated file a session refreshes in the pull request that adds or closes a row; the job only reports |
| Q-51-9 | The instant a stock rig boots at | a fixed epoch that sits on no IC boundary; schedule-bound stories also run at three named phases |
| Q-51-10 | tacsim's 416 real-clock arena tests | left as they are, under the ratchet, until the soak names one |
| Q-51-11 | A fire count on the staff schedule readout | not now; the barrier reads `@ps` as it prints today |

A count is CONFIRMED only when `scripts/wait_audit.py` or a named `grep` prints it. A cause is
taken from `docs/flaky-tests.md` and checked against the test's source where §2.4 says so. Where
this document differs from the brief it was written to, it says so and why.

## 0. In brief

- **The failures are not where the waits are.** Test code under `crates/*/tests/` holds 617 wait
  sites in 104 files (§2.2). The flake list holds 17 rows and one newer failure, about 84 failed or
  retried runs in all (§2.4). **Forty-four of those 84 came from two causes that are not waits:**
  a span or a distance that restated a game figure and an outcome left to dice (rows 2, 3, 4 and
  9, 23 runs), and a rig that boots at the wall clock, so the IC calendar's phase follows the
  minute the run started (rows 7 and 17, 21 runs). A wait census sees those least: 14 of the 617
  sites restate a figure, and the wall-clock boot is 63 reads in 44 files.
- **Every class has code in the tree that already fixes it once.** The fixes are private to one
  file. In the stories of mushd, mush-gateway and mush-bot, `ask_whole` (the reader that knows
  where a reply ends) is defined 12 times and `say` 26 times, and the gateway's stories carry ten
  copies of one `Term` struct. Row 3 is the plain case: the 12-s
  wait was mended in `sim_range` and missed in `observer_rule`, which held a copy.
- **tacsim's suites are absent from the list because of how they are built** (§2.5), not by
  luck: 1,946 of its 2,606 integration tests have no runtime and no clock but the one the test
  sets; 244 more drive tokio's paused clock; fields draw from a pinned seed; a reply is a return
  value, not text on a stream. About half of that carries to the other crates' stories, and §2.6
  says which half.
- **The design is one rule and one helper a class** (§3), in one shared dev-only crate, and a
  ratchet in `mush-contract-tests` that counts raw waits in story files and lets the count only
  fall.
- **Detection is a nightly soak on conclav.dev and one script** (§4). The script reads the JUnit
  reports CI already uploads, which nothing reads today. A pass on retry was printed in 120 green
  runs before anyone looked.
- **The retry stays narrow** (§4.3). Twelve of the 17 rows were on no retry list, and 18 of the
  list's 23 entries match no row. The list becomes a file of rows, each with a class and an
  expiry.
- **Ten units** (§5), the suites that flaked most first: mushd's `sim_lenses` and mush-server's
  `commerce_composition`, 21 runs each.

## 1. Why this document exists

The owner asked for a systematic approach after a run of fixes that each mended one test. The
hygiene pass of 2026-11-22 wrote `docs/flaky-tests.md` from 1,000 CI runs and merged its fixes as
#764 and #765. Each fix there is sound and local: a tank moved to 200 m, a wait widened to 21 s, a
sentence added to a list. The balance figures those fixes lean on are provisional (doc 43 rule
2.3 #9), so the next tuning pass will break the same tests in the same way. Doc 43 rule 2.3 #7
says a flake is investigated, not discarded. This document adds the step after that: a fix is
written once, in a place every story uses, and a gate stops the old shape from coming back.

## 2. The audit

### 2.1 Method

`scripts/wait_audit.py` is the census. It reads every `.rs` file under `crates/<pkg>/tests/`,
blanks comments and string literals, and finds each call of `sleep(`, `sleep_until(`, `timeout(`,
`timeout_at(`, `recv_timeout(` and `wait_timeout(`. A method call `.timeout(` is a client's
configuration and is not counted.

```text
python scripts/wait_audit.py                 # every table in §2.2
python scripts/wait_audit.py --dump          # one TSV row a site
python scripts/wait_audit.py --class clock   # the sites of one class
python scripts/wait_audit.py --literals      # the literal spans of §2.3
python scripts/wait_audit.py --src           # with the test code under src/ (takes minutes)
```

Each site gets two labels.

- **Shape** is mechanical. `fixed` is a sleep outside any loop: the test waits a span and goes
  on. `poll` is a sleep inside a loop: the loop asks again until something holds. `bound` is a
  timeout around a future: the future is the wait and the span is its limit.
- **Class** is what the site waits for, by the rule table in the script (first match wins). The
  rules read the site's statement, the lines around it and the function it is in. A site no rule
  names takes its shape's default: a `bound` is `event`, a `poll` or a bare sleep is `clock`.

The classes are the brief's five and one more:

| Class | What the site waits for |
|---|---|
| `figure` | a span that restates a game or product figure: a cycle, a grace, a window |
| `clock` | a clock the test does not drive: the arena's wake, a world tick, a tokio timer in the code under test |
| `stream` | a reply read from a stream: an SSH channel, a web socket, a frame reader, a child's output |
| `process` | a real process, a server task or a listener starting, binding or stopping |
| `event` | a channel, a join handle, a notify or a future of the code under test; the span is only the failure limit |
| `driven` | virtual time, in a `start_paused` test that drives the clock itself |

**How far to trust it.** The rules are heuristics over text. The two defaults carry 250 of the
617 sites (`By rule`, at the end of the script's report). I read every `figure` and `process`
site, every site in tacsim, and a sample of three a file from the defaults. Two errors remain
that I know of. A retry loop that types a line again after a busy answer is counted `clock` when
its function has no reader's name, and it belongs to `stream`. A helper that only paused tests
call is counted on the wall clock, because the script reads the attribute of the function a site
is in and not of its callers. The classes are sizes and member lists, not a proof about one site.

**The brief's figure.** The brief gave 664 call sites in 114 files. The script finds 617 in 104
under `crates/*/tests/`, and 930 in 156 when the test code under `src/` is added. I did not
reproduce 664; a plain `grep` over a wider path also counts needles in comments and strings.

### 2.2 The counts

Test code under `crates/*/tests/`, 635 files scanned, 617 sites in 104 files:

| Crate | figure | clock | stream | process | event | driven | all |
|---|---:|---:|---:|---:|---:|---:|---:|
| mush-bot | 3 | 19 | 26 | 9 | 41 | 0 | 98 |
| mush-contract-tests | 1 | 2 | 0 | 0 | 0 | 0 | 3 |
| mush-gateway | 1 | 14 | 35 | 41 | 3 | 0 | 94 |
| mush-llm | 0 | 3 | 0 | 0 | 1 | 0 | 4 |
| mush-net-ssh | 1 | 9 | 8 | 1 | 6 | 0 | 25 |
| mush-net-web | 0 | 10 | 6 | 13 | 4 | 0 | 33 |
| mush-server | 0 | 10 | 22 | 9 | 27 | 7 | 75 |
| mush-sshdoorway | 0 | 4 | 6 | 0 | 4 | 0 | 14 |
| mush-store | 0 | 3 | 0 | 0 | 0 | 0 | 3 |
| mush-worldgen | 0 | 1 | 0 | 0 | 0 | 0 | 1 |
| mushd | 5 | 23 | 73 | 6 | 26 | 0 | 133 |
| tacsim | 3 | 82 | 2 | 0 | 10 | 37 | 134 |
| **total** | **14** | **180** | **178** | **79** | **122** | **44** | **617** |

By class and shape:

| Class | fixed | poll | bound | all |
|---|---:|---:|---:|---:|
| figure | 5 | 6 | 3 | 14 |
| clock | 36 | 144 | 0 | 180 |
| stream | 11 | 64 | 103 | 178 |
| process | 8 | 6 | 65 | 79 |
| event | 4 | 3 | 115 | 122 |
| driven | 27 | 10 | 7 | 44 |
| **total** | **91** | **233** | **293** | **617** |

The 573 sites on the wall clock, by where they stand. A rig is a file with `/support/` in its
path or named `harness.rs`; every other test file is a story.

| Crate | rig | story |
|---|---:|---:|
| mush-bot | 20 | 78 |
| mush-gateway | 8 | 86 |
| mush-server | 0 | 68 |
| mush-sshdoorway | 7 | 7 |
| mushd | 9 | 124 |
| tacsim | 3 | 94 |
| the six small crates | 0 | 69 |
| **total** | **47** | **526** |

**Ninety-two percent of the wall-clock waits are written in a story file, not in a rig.** That is
the structural finding. A wait in a story is a wait someone copied.

With `--src` the census is 930 sites in 156 files. The additions are mostly mush-server's
in-`src` tests (162 sites), mush-net-ssh (49), tacsim (36), mush-gateway (32), mush-drain (17) and
mush-net-discord (12). Row 8 of the flake list is one of those in-`src` tests.

### 2.3 What the primitives hide

A story seldom calls `sleep` itself. It calls a helper, and the flake is in how the helper is
used. Three counts, each a `grep` over `crates/*/tests/`:

- **Private copies of one helper.** Function definitions by name in the tests of mushd,
  mush-gateway and mush-bot together: `say` 26, `login` 19, `ask` 17, `strip_ansi` 15,
  `ask_whole` 12, `expect_after` 12, `until` 6, `start_split` 5.
  `struct Term`, the SSH line reader, is defined ten times under
  `crates/mush-gateway/tests/split_halves/` and once more in `crates/mushd/tests/support/mod.rs`.
- **Reads that stop at a needle, against reads that know the reply's end.** In mushd's stories
  `say(` is called about 620 times, `ask(` about 250 and `expect_after(` about 190; each returns
  when one needle has arrived. `ask_whole(`, which returns when the whole reply has, is called
  about 180 times. About one read in seven knows where its reply ends.
- **Literal spans.** `python scripts/wait_audit.py --literals` counts `Duration::from_*(<number>)`
  in test code: 949 in all, 60 in rigs, 457 on a wait site's own lines in a story and 432
  elsewhere in a story (a constant, a helper's bound, a config value). Of mushd's 148
  `wait_until(` calls, 131 pass the rig's `STEP_TIMEOUT` or twice it, 16 pass a literal and one
  passes `CYCLE_AND_FLIGHT`. Rows 3 and 9 were two literals.

And one count that is not a wait at all: **63 reads of the wall clock** (`unix_now_s()`,
`unix_now_ms()`, `SystemTime::now()`) in 44 test files, 32 of them in mush-bot and 13 in
mush-server. `commerce_composition`'s rigs boot the engine at `mush_config::unix_now_s()`
(`ic_slice_loop.rs:187`, `ic_buyout_dormancy.rs:177`), which is row 7's cause.

### 2.4 The flake list, row by row

"Runs" is the number of failed or retried runs the list names for the row. Row 18 is the failure
the hygiene session is fixing now, not yet in the list.

| Row | Test (short) | Suite | Runs | Class | Why |
|---|---|---|---:|---|---|
| 1 | `dt_preparation_plans::…carrier_is_lost…` | mush-server `split_boundary` | 4 | clock | a write queued on the arena task's clock was staged after a later world tick; fixed by a drain, which is a barrier |
| 2 | `arena_e2e::the_bot_flies_a_match_alone…` | mush-bot | 5 | figure, dice | a 60-s match at 500 m leaves four rounds at the 15-s cycle; one match in eight lands none |
| 3 | `observer_rule::o3_…` | mushd `sim_lenses` | 7 | figure | a 12-s wait written for a 1-s cycle |
| 4a | `observer_rule::ae2_…` (the empty rack) | mushd `sim_lenses` | 7 with 4b | figure, dice | "fire until one lands" against a rack of four |
| 4b | `observer_rule::ae2_…` (the wake) | mushd `sim_lenses` | | clock | two lines had to land in one arena wake |
| 5 | `a_bot_hires_launches_mines…` | mush-bot | 4 | stream | a busy answer the reader did not know as "type it again" |
| 6 | `a_captain_and_its_escort_fight…` | mush-bot | 6 | not known | every wait was longer than nextest's terminate, so the stall printed nothing |
| 7 | `ic_slice_loop::ic12_…` (three tests) | mush-server `commerce_composition` | 17 | clock phase | the rig boots at the wall clock; an IC day's turn fell inside the story at boot seconds 47:20 to 48:10 of any hour |
| 8 | `player_market_commands::rate_queues…` | mush-server lib | 9 | stream | equality over a drain window that could hold a stray `Done.` |
| 9 | `sim_range::one_pilot_on_the_range…` | mushd `sim_lenses` | 4 | figure | row 3's wait, the other copy |
| 10 | `boot_parity::stop_ledger_archive…` | mush-server `split_boundary` | 1 | clock | a bare 20-ms sleep, then an assertion that a spawned task had run (`boot_parity.rs:1247`) |
| 11 | `ipc_teardown::wedged_gateway…` | mush-server `split_boundary` | 1 | stream | it expected one whole frame larger than the 4 KiB receive buffer |
| 12 | five restart and drain stories | mush-gateway `split_halves` | 6 | process | real restarts on wall-clock limits |
| 13 | two `observer_rule` absence stories | mushd `sim_lenses` | 2 | clock | paint and poll timing |
| 14 | `web_e2e::spa_if_built_and_healthz…` | mushd `world_journeys` | 3 | process, likely | `/healthz` answered before the server said it was ready; cause not known |
| 15 | two `arena_custody_split` stories | mush-gateway `split_halves` | 2 | process, likely | a restart's `Reconnected` did not come inside the limit; cause not known |
| 16 | `sim_rematch::two_players_start_a_second_match…` | mushd `sim_lenses` | 1 | not known | |
| 17 | `ic_buyout_dormancy::ic13_…`, `sm2_…` | mush-server `commerce_composition` | 4 | clock phase, likely | a line counted twice; cause not known. Its rig boots at the wall clock exactly as row 7's does, in the same binary |
| 18 | `arena_contract::the_bot_reads_a_live_centre…` | mush-bot | 1 or more | stream | a status readout parsed before its last two sections had arrived |

By class: figure and dice 23 runs (rows 2, 3, 4, 9, with 4b's share inside row 4's seven); clock
phase 21 (7, 17); stream 15 (5, 8, 11, 18); process 11 (12, 14, 15); clock 7 (1, 10, 13); not
known 7 (6, 16). **No row is an `event` wait.** The 122 `event` sites need a rule about their
limits (§3.7) and no other work.

By suite: mushd `sim_lenses` 21; mush-server `commerce_composition` 21; mush-bot 16; mush-server's
lib 9; mush-gateway `split_halves` 8; mush-server `split_boundary` 6; mushd `world_journeys` 3.

**Where this differs from the brief.**
- The brief put rows 12 and 15 with the clock races. They are real restarts, so they are
  `process`, the class the brief itself names for that.
- The brief put row 17 with the stream reads, on its symptom (a line told twice). Its cause is
  not known. I put it with row 7 because the two tests boot the same rig at the wall clock and
  sit in the same binary, and row 7's cause was a listing re-posted on an IC day's turn, which
  can also tell a line twice. That is a reading of the code, not a reproduction. Unit FL.4
  settles it: once the rig boots at a fixed instant, the row either stops or is a stream fault.

### 2.5 Why tacsim's suites are not in the list

tacsim holds 2,606 integration tests (`#[test]` 1,946, `#[tokio::test]` 660) and none of the 18
rows. From the code, four things keep it out.

1. **Most of its tests have no clock of their own.** 1,946 are synchronous tests over
   `mush_testkit::TestWorld`. There is no runtime and no socket. Time is what the test says it is
   (`set_now`, `fire_due`, `mush-testkit/src/lib.rs:387` and `:393`). A command's output is the
   return value of `run` (`:267`), so the reply is whole by construction.
2. **Its arena tests drive the clock.** 244 tests run on `#[tokio::test(flavor =
   "current_thread", start_paused = true)]`: the arena task's own tests, 30 in `cockpit.rs`, and
   183 field tests since TS.10a. Their sleeps are virtual. The field rig has one wait loop for all
   of them, `field_host_rig::Pace` (`support/field/field_host_rig.rs:310`), which also keeps the
   world's clock and the arena's as one.
3. **Its fields draw from a pinned seed.** `FieldConfig::seed` (`crates/tacsim/src/config.rs:2099`)
   seeds every field a rig admits, and the driving rig sets it (`support/field/driving_rig.rs:104`).
   Every draw in a match is a keyed hash of the match seed and a label
   (`crates/tacsim/src/arena/rng.rs`), with no stream, so a pinned seed gives the same dice
   whatever ran before.
4. **It reads state, not text.** The remaining 416 tokio tests run a real arena task on a
   two-worker runtime, and they hold tacsim's 82 `clock` sites. Those polls ask a typed question of the
   world in the same process (`registry.last_frame(pilot)`, the match mirror), the test itself
   pumps the world between asks, and nothing is parsed off a stream.

The record agrees. The field family's own flakes (`field_rs1`'s tender-tow stories,
`escort_lease::cf6c_…`) are in the cut notes from before TS.10a and have not failed in the
1,000-run window. Two caveats. The 416 real-clock arena tests are the same shape as mushd's
polls with a shorter path, and `cockpit::i4_eight_watch…` is on the older list, so "has not
flaked lately" is the claim, not "cannot". And a sim-center match has no pinned seed: `start`
draws from the OS every time (`crates/tacsim/src/match_/mod.rs:1470`).

### 2.6 What carries to the other crates' stories

| tacsim's method | Carries to | Does not carry to | Why |
|---|---|---|---|
| The test sets the clock (`TestWorld`) | mush-server `commerce_composition`, which is already `TestWorld` and then boots it at the wall clock (row 7) | stories that run a real server | the fix there is the boot instant, §3.6 |
| The paused clock | in-process rigs on the memory store with no network client: mush-server's `world_task` tests and the in-process parts of `split_boundary` (row 10). `TimeSource::Tokio` and its guard are built (TS.10b, TS.10c) | mushd `sim_lenses`, the gateway's stories, the bot's: real SSH, child processes, Redis | 43c §3.3: paused time fires a timeout at once across I/O the runtime does not own |
| A pinned seed | every story that starts a sim-center match, once `ArenaConfig` has the field's knob (Q-51-2) | a story with a mind or the bot moving a unit: the seed fixes the dice, not where the unit stands when it fires | §3.2 |
| Typed reads | nothing as it stands. A story that crosses SSH reads text by design; that is what it proves | | the fix is a reader that knows where a reply ends, §3.4 |
| One wait loop in the rig | every crate | | §3.8 |

So the first three carry in part, and the fourth is replaced by its equivalent for text. The
stories that keep the wall clock (real SSH, real processes) need the barrier of §3.3, which
tacsim never needed.

## 3. The design: one rule and one helper a class

Each class below has a rule a reviewer can hold a test to, a helper, and the code in the tree
that already does it. §3.8 says where the helpers live and §3.9 how the gate holds the rules.

### 3.1 Figures: a wait is derived, never written

**Rule F.** A span, a distance or a count that follows from a game figure is computed from that
figure at run time. A story holds no literal that restates content.

**The helper: `Figures`.** It is built from the same `TacsimConfig` and content packs the story's
server booted with, so it cannot disagree with the server.

```rust
let fig = Figures::of(&rig.config);
fig.cycle("cannon")                 // the mount's cycle, from its pack: 15 s today
fig.flight("cannon", range_m)       // range over the round's speed
fig.load("heavy_mk1", "rack")       // the rounds a unit carries: 4 today
fig.pulse()                         // the arena's slowest wake at the story's tempo
fig.after_one_round("cannon", range_m)   // cycle + flight + two pulses: row 3's wait
```

A wait is then `wait_until(fig.after_one_round("cannon", 200), …)`. A count is `fig.load(…) - 1`,
which is what `AE2_PHASE_ONE_ROUNDS = 3` says in a comment today (`observer_rule.rs:7058`).

**Two refinements.**
- **Wait for the event, bound by the figure.** The readiness wait polls the cockpit until it
  says the cannon is ready, and the figure is only its limit. A tuning change then makes the
  story slower, never red. Rows 3 and 9 were already this shape with a literal limit.
- **A limit carries a load factor.** `fig` spans are multiplied by one rig constant
  (`LOAD_FACTOR`, 2 to start), so a slow runner is answered in one place.

**In the tree already.** `arena::match_wait(max_duration_ms)` derives God's wait from the match
cap (`crates/mush-bot/tests/support/arena.rs:1231`); it is the model. `CYCLE_AND_FLIGHT`
(`sim_range.rs:68`) is halfway: a named constant whose comment cites the pack, with 21 written by
hand.

### 3.2 Dice: an outcome is certain, or its odds are written down

**Rule D.** A story's assertions do not depend on a draw. A story whose point is to sample chance
says so, states its odds in code, and runs on the tier those odds allow.

Three tools, in the order to reach for them.

1. **A pinned seed.** `ArenaConfig::seed: Option<u64>`, read at `match_::start` where `seed_for`
   is called today, as `field/admit.rs:1027` does for a field. `None` is production's. No config
   file key sets it, as with the field's. The mushd, gateway and bot rigs set it. With units
   posed by staff and standing still, the same seed gives the same result on every run, because a
   draw's label is the shot's number and not the pulse it fell in.
2. **A fixture where the rules make the result certain.** `fig.sure_range(shooter, target)`: a
   range and facing at which the rules' own chance is the whole die. Row 4's fix chose 60 m by
   measurement; the helper asks the rules. It is what to use when something in the story moves.
3. **A chosen seed.** `draw_permille(seed, label)` is a public pure function
   (`arena/rng.rs:259`). A rig can search for a seed whose third shot misses and pin it. That
   gives "a forced roll" with no new door into the arena.

**A story that must sample chance** (the bot against a moving machine, the win-rate lanes) uses
`Odds`:

```rust
Odds { trials: fig.rounds_in(match_len, "cannon"), need: 1, p_each: fig.chance(...) }
    .assert_false_failure_below(1e-6);
```

The call computes the binomial tail and fails at the top of the test, with the arithmetic, when
bad luck alone would fail the story more often than the bar (Q-51-3). Row 2 would have failed
there the day the cycle moved to 15 s: four trials could not meet any sane bar, and the message
would have said so. A story that cannot meet the bar belongs on the dispatch tier and reports an
interval, which the win-rate lanes already do (`arena::wilson_95`, `support/arena.rs:1587`).

### 3.3 Clock: a barrier, not a sleep

**Rule C.** A story never sleeps to let the server get somewhere. It waits for the server to say
it is there.

For an in-process rig on the memory store the answer is the paused clock (§2.6). For a story
that runs a real server, the rig gets four barriers. Each reads a staff readout over the story's
own SSH session, with the whole-reply reader of §3.4.

| Barrier | Waits until | Reads | In the tree today |
|---|---|---|---|
| `settled()` | the world has answered everything sent before it | the sentinel's refusal (§3.4) | `o1_drain`, `observer_rule.rs:6315` |
| `arena_wake(id)` | match `id` has pulsed at least once more | `frames F` on `@arena/status` | `ae2_frames`, `observer_rule.rs:7070`, private to one test |
| `arena_seconds(n)` | the arena's own clock has advanced `n` seconds | `clock m:ss` on `@arena/status` | `wait_arena_seconds`, `mushd/tests/support/mod.rs:235` |
| `after_tick(tag)` | the named world tick has fired once more | the tick's next-fire on `@ps` | none; `render_ps` prints it (`world_task/staff_ops.rs:163`) |

One composite replaces row 4b's hand-built sequence: `at_one_wake(id, &[line_a, line_b])` sets
the tempo floor, waits for `arena_wake`, and sends the lines, so they land a whole floor before
the next wake.

**Absence.** "Nothing was told" cannot be waited for. The rule: an absence is asserted between
two barriers, never after a sleep. Send, call `settled()`, then assert on the transcript between
the marks. mush-net-web's thirteen bare sleeps (five in `ws.rs`) and four in
`observer_rule::nothing_about_an_undetected_unit…` are this pattern.

The staff verbs are the rig's, typed by a staff character the test owns. The bot under test never
types them.

### 3.4 Stream: one reader that knows where a reply ends

**Rule S.** Text is parsed only from a reply the reader knows is whole. A read that stops at a
needle may be used to wait, never to parse.

**The helper** is one `Term` for a line-mode SSH session, with:

- `ask_whole(line) -> Reply`: sends `line`, then a nonsense line, and returns everything up to
  the nonsense line's refusal. The world answers a session's commands in the order it took them
  from one ordered outbox, so the refusal marks the end of the reply. A busy answer is retried
  inside it.
- `say(line, needle)`: waits for a needle and returns nothing to parse.
- `Reply`: the text, with `section(name)`, `row(prefix)` and `number_after(label)`, so a story
  does not slice `seen[mark..]` by hand.
- **One table of busy sentences**, read by every reader. Row 5 was a sentence missing from one
  list. The engine's own constants are the source (`mine::lines::ENGINE_BUSY` is checked against
  them in mush-bot's tests today).

**In the tree already.** `ask_whole` with its sentinel is at `observer_rule.rs:548`, with eleven
more definitions; the comment there states the ordering argument. The bot's `Client`
(`mush-bot/tests/support/arena.rs`) has `say`, `answer` and `expect`, all needle reads, and
gains `ask_whole`.

**Does player text need an end marker for the bot?** Not for the test rigs: the sentinel uses
only what a player may type, and it has about 180 calls behind it in mushd. For the bot itself:

- In the arena the bot already reads replies by their rows and closes one at the first line that
  is not a row ("a pause never ends a reply", `src/arena/picture.rs:9`). Row 18 was a test
  reading the picture early, not the bot's reader.
- Outside the arena the bot waits for a needle or a quiet span. A sentinel line would work for
  it too, at the cost of a second line a read and a `Huh?` in its transcript.
- The MU\* convention for this is `OUTPUTPREFIX` and `OUTPUTSUFFIX`: a player command that sets
  a string the server prints before and after each command's output for that session. It is a
  player-facing command, so `crates/mush-bot/AGENTS.md` allows the bot to use it. It does not
  exist here; the parser's tests name `OUTPUTPREFIX` only as an example
  (`mush-core/src/cmd.rs:2544`).

The default (Q-51-4) is no product change in this program. `OUTPUTSUFFIX` is the right later
unit if the bot's out-of-arena reads turn up in the soak.

### 3.5 Process: wait for the fact, in the rig

**Rule P.** A start, a bind or an exit is awaited as an event from the rig, with the rig's one
limit. A story does not poll a log or sleep through a restart.

**In the tree already**, and it is the model: the gateway's `ServerUnit` and gateway harness
(`split_halves/harness.rs`) hand back a channel that resolves when the listener has bound
(`ipc_rx`, `ssh_rx`) and one that resolves on exit (`exited`), under `BIND_TIMEOUT` and
`EXIT_TIMEOUT`. Sixty-five of the 79 `process` sites are already that shape.

What is left is the client's side of a restart, which is rows 12 and 15. A story sends a line
through a held session and waits for `Reconnected` under a literal limit. The helper is
`Term::across_restart(|| unit.restart())`: it starts the restart, awaits the rig's bound event,
then calls `settled()` on the held session. The limit is derived from the gateway's own retry
schedule and grace, through `Figures`, as §3.1 does for a cycle. Row 14 is the same on the web
side: await the bound event, then ask `/healthz`.

### 3.6 Clock phase: a rig boots at a named instant

**Rule T.** No test reads the wall clock to place itself in the IC calendar. A rig boots at a
fixed epoch. A story about anything scheduled on the game clock also runs at named phases.

- **The epoch.** One constant, `STORY_EPOCH`, chosen to sit on no IC hour, day or month
  boundary, so a rig does not start with every schedule aligned. The stock field rig boots on a
  month's boundary today, which is the most aligned instant there is.
- **Phases.** `phases!` runs a story at three instants: `STORY_EPOCH`, 50 IC seconds before a
  day's turn, and 15 IC days before a month's. A story declares it when its subject is a
  schedule: a re-post, a sweep, a vote, a deadline.
- **The soak adds a random phase** (§4.1). It sets `STORY_PHASE=random`, the rig prints the
  phase it drew, and a failure names the phase to pin.

**In the tree already.** `driving_rig::stocked_phased(keyed, cfg, ic_phase)` and
`op8_campaign`'s `composed_phased`, with stories booted 15 and 25 IC days before a boundary
(`b811089a9`, in #763). That fix found a product fault of this kind: the Concession Review's
votes fell on absolute month boundaries while its house ran from boot.

**The gate** is the twin of `world_clock_reads` (TS.10e), turned on test code: the 63 wall-clock
reads are counted a file, and the count may only fall (§3.9).

### 3.7 Event waits: one limit, and budgets that nest

The 122 `event` sites are sound. Two rules keep them useful when they do fail.

- **One limit a rig.** A bound on an event is the rig's `STEP_TIMEOUT` times the load factor, not
  a number chosen at the call.
- **A story's waits fit inside nextest's terminate.** Row 6 could not be diagnosed for weeks
  because each wait was longer than the 20 minutes after which nextest kills the test, so no
  assertion ever printed. #764 fixed that story by hand with an 18-minute budget. The helper is
  `StoryBudget::new(minutes)`: every wait in the story draws its limit from what is left, and the
  rig refuses a budget longer than the profile's terminate less two minutes.

### 3.8 Where the helpers live

The helpers above are wanted by mushd, mush-gateway, mush-server and mush-bot. Today each has
its own `tests/support`, and mush-gateway has none shared.

**Proposed: one dev-only crate, `mush-storykit`** (Q-51-1), `publish = false`, a
dev-dependency only. It holds `Term`, `Reply`, the barriers, `Figures`, `Odds`, `StoryBudget`,
`STORY_EPOCH` and `phases!`. It depends on tacsim (for the packs) and russh. `mush-testkit` is
not the place: it is the engine-level `TestWorld`, every crate's unit tests link it, and it must
not gain tacsim or an SSH client.

For mush-bot this stays inside `AGENTS.md`: game crates are allowed as development dependencies
for server-under-test fixtures, and `cargo tree -p mush-bot --edges normal,build` is unchanged.

Each rig's `tests/support` keeps what is its own (how it boots its server) and re-exports the
kit. The private copies counted in §2.3 are deleted as each suite's unit lands.

### 3.9 The ratchet

A gate in `crates/mush-contract-tests/tests/ci_policy/`, built like the fiat budget (TS.6) on
TS.8's scanner: per-crate files of `<budget> <low-water> <path>`, equality as the test, a raise
that needs a written reason, and a bless variable.

**`wait_budget`** counts, in every story file under `crates/<pkg>/tests/`:

1. **Raw waits:** the six needles of §2.1. Baseline 526.
2. **Wall-clock reads:** `unix_now_s()`, `unix_now_ms()`, `SystemTime::now()`. Baseline 63, less
   the rig files' share, taken at the unit.
3. **Literal spans passed to a wait:** `Duration::from_*(<number>)` on a line that calls a wait
   primitive or a kit helper. Baselined after FL.3, when there is a helper to move to.

What it does not count: rig files (a path with `/support/`, `harness.rs`, and the kit itself),
where the primitives belong; and a site inside a `start_paused` test, which drives its own
clock.

A new story therefore cannot add a sleep, a wall-clock read or a hand-written span without a
raise line that a reviewer sees. A unit that converts a suite blesses its files down, and they
stay down. The gate's count and `scripts/wait_audit.py`'s must agree at the baseline; the unit's
note records both.

A second, smaller gate, **`one_reader`**, refuses a second definition of `struct Term`,
`ask_whole` or `strip_ansi` under a crate's `tests/` once that crate's unit has landed. It is a
list of crates that have converted, so it tightens a unit at a time.

## 4. Detection

### 4.1 The soak

A scheduled job on conclav.dev that repeats the story suites the way a hosted CI cell runs them.

- **Where.** A new workflow, `.github/workflows/soak.yml`, on `schedule` and
  `workflow_dispatch` only, `runs-on` the `conclav-dev` runner. It has no `pull_request`
  trigger, for `gate.yml`'s reason: a fork's pull request must never reach the owner's boxes.
- **What.** The binaries that run a real server or process: mushd `sim_lenses`,
  `world_journeys` and `durable_process`; mush-gateway `split_halves`; mush-server
  `split_boundary` and `commerce_composition`; mush-bot `arena_e2e`, `arena_contract` and
  `mining_cohort`. The `redis` group is left out of the first version; one shared instance
  cannot serve six cells.
- **How it is loaded.** One build, archived (`cargo nextest archive`). Then six cells, each
  pinned to four cores and 16 GB (a systemd scope with `AllowedCPUs` and `MemoryMax`, or
  `taskset`), each running `cargo nextest run --archive-file … -P soak --test-threads 4` in a
  loop for the window. Six cells use 24 of the box's 32 cores. The brief asks for a cap on
  concurrency there; six is the cap, and FL.2 measures before raising it.
- **The `soak` profile.** No retries, no fail-fast, the `ci` terminate, JUnit a pass. A flake
  shows as a failure, with its output.
- **A random phase.** Each pass sets `STORY_PHASE=random` (§3.6) and a fresh `ArenaConfig` seed
  where a story has not pinned one, and prints both.
- **What comes out.** The JUnit of every pass, uploaded, and §4.2's report in the step summary.
  The job is red when the report finds a test that failed in some passes and passed in others
  and is not on the known list. A red soak reaches the owner the way a red nightly does (D-43-3).

**What it can and cannot find.** A pass over those binaries is about 40 minutes at four threads
(131 summed minutes on 2026-11-03, doc 43 §2.1; FL.2 measures it). Three hours of six cells is
about 24 passes a night.

| A flake that fails one run in | Seen in one night | Seen in a week |
|---:|---:|---:|
| 8 (row 2) | 96% | 100% |
| 20 (row 4a) | 71% | 100% |
| 100 | 21% | 82% |
| 500 | 5% | 29% |

So the soak finds the common flakes before a pull request does, on main, with no branch's own
bugs mixed in. It will not find a 1-in-500 flake quickly. CI itself is the wider net for those:
about 55 runs a day, each already uploading JUnit. That is why §4.2 reads both.

### 4.2 The report

`scripts/flake_report.py`, one script with two sources and one table.

- **Sources.** The `nextest-junit-*` artifacts of the last N `ci.yml` runs (`gh run download`,
  as `scripts/tacsim-cells.py` does for weights), and a soak's JUnit directory.
- **What counts**, by `docs/flaky-tests.md`'s own rule: a `<flakyFailure>` (a pass on retry); a
  test that failed and passed at one head across attempts or passes; a test that failed on two
  or more unrelated branches in the window. A failure on one branch only is that branch's.
- **What it writes.** One row a test: binary, class and list row (from the known list below),
  first and last seen, count in 14 and in 30 days, the soak's rate, the last three run links.
  `--write` puts the table in `docs/flaky-observed.md` (Q-51-8). `--check` exits 1 on a test
  that is not on the known list, or on a listed test past its expiry.
- **Where it runs.** In the soak job, with `--check`. In `ci.yml`'s `nightly-report` job over
  the day's runs, with `--check`. By hand, with `--run <id>`, to answer the question a red pull
  request raises: "has this test failed anywhere else?"

`docs/flaky-tests.md` stays the hand-written record of causes. The generated table is what was
seen; a row there with no row in the hand-written file is the work queue.

### 4.3 The retry policy

**Today.** `.config/nextest.toml` gives one retry to a hand-kept `load-sensitive` filter of 23
entries, most of them whole modules. A pass on retry prints `FLAKY`. Nothing reads it. Against
the 17 rows:

- Five rows had the retry (3, 4, 11, 12, 13) and so mostly passed, unread, for weeks.
- Twelve rows had none, and failed the pull request they landed on. The owner reran them by
  hand, which is a blanket retry with no record.
- Eighteen of the 23 entries match no row. Their retry is either unneeded or hiding something;
  the list cannot say which.

**The trade.** A blanket retry on pull-request runs would have turned most of those twelve
green. It would also have hidden what they were. Row 1 was a product ordering fault that sends a
player a refusal for a line they never typed. Row 7's class produced a product fix in #763. Row 2
passed on a second try seven times in eight while the bot's solo match was being decided by
dice. A retry answers "is this test usually green", and the question that matters is "is this
product ever wrong". The reader in §4.2 narrows that gap but does not close it: with a blanket
retry, a new flake still reaches main first and is found afterward.

| | Blanket retry on PR runs | Narrow, listed, expiring (proposed) |
|---|---|---|
| A known flake on someone else's PR | green | green: it is on the list |
| A new flake on a PR | green, found later by the report | red once; `flake_report.py --run` says whether it is known elsewhere |
| A product race | retried away; found only if someone reads the table | red on the PR that meets it |
| A fix that does not work | invisible | the row expires and the gate goes red |
| Cost | every flake's time, twice | a rerun by hand for a flake nobody has listed yet |

**Proposed** (Q-51-5):

1. **No blanket retry.** A test is retried once only if it is on the known list.
2. **The list is a file with rows, not a filter kept by hand:**
   `crates/mush-contract-tests/flaky-known.toml`, one row a test with its binary, class, the row
   of `docs/flaky-tests.md` that explains it, the date it was listed and an expiry 14 days on.
   A contract test checks that nextest's retry filter is exactly the list (the filter is
   generated from it), that each row has a class and a doc row, and that none is past its expiry.
3. **An expired row fails the gate.** The test is fixed and the row goes, or the owner extends
   the date in a diff he sees. A whole-module entry is narrowed to the tests that flaked.
4. **The soak never retries.**
5. **The 18 unmatched entries are carried over as rows with no class and the same 14 days.** A
   row is renewed only on evidence: a soak failure, or a cause written down. One of them,
   `what_still_waits`, is an unbiased `select!` and will want a real row.

The soak is what makes the narrow policy bearable: most flakes meet the list before they meet a
pull request.

## 5. Units of work (FL.1 and FL.2 cut; FL.3 to FL.10 wait for a further word)

**Where the units stand at Draft 0.2.** The owner cut FL.1 and FL.2 on 2026-11-22 ("Take defaults,
cut both"). FL.3 to FL.10 stay a sketch: none is cut, and each waits for a further word from the
owner.

One pull request each. S is under a day's session, M one to two, L more. "Runs" is §2.4's count
for the suite. FL.0 is this document and its script, merged as #771.

| Unit | What | Crates | Size | Runs |
|---|---|---|---|---:|
| **FL.1 the ratchet** | `wait_budget` at today's counts (raw waits and wall-clock reads; the literal-span counter is stubbed); the `mush-storykit` crate with `wait_until`, `STEP_TIMEOUT`, `LOAD_FACTOR` and `StoryBudget` moved in from mushd's rig | mush-contract-tests, new mush-storykit, mushd tests | M | |
| **FL.2 the detection loop** | `soak.yml`, the `soak` profile, `scripts/flake_report.py`, `flaky-known.toml` and its contract test, the retry filter generated from it; the first week's soak recorded in the cut note | .github, .config, scripts, mush-contract-tests | M | |
| **FL.3 `sim_lenses`: figures, seed, barriers** | `Figures` and `Odds`; `ArenaConfig::seed` (the one product change, about thirty lines beside `seed_for`); `arena_wake`, `arena_seconds`, `at_one_wake`, `settled` in the kit; every literal span and "fire until" loop in mushd `sim_lenses` converted; rows 3, 4, 9, 13 and 16 closed or re-described | tacsim (the knob), mush-storykit, mushd tests | L | 21 |
| **FL.4 `commerce_composition`: the boot instant** | `STORY_EPOCH` and `phases!`; the four rigs that boot at `unix_now_s()` boot at the epoch; ic12's three stories and the two of row 17 run at the three phases; row 17 settled as phase or stream | mush-storykit, mush-server tests, mush-contract-tests (the wall-clock budget falls) | M | 21 |
| **FL.5 the bot's stories** | `Client::ask_whole` and `Reply`; one busy-sentence table for the rig; `StoryBudget` in every `mining_cohort` story; the seed and `Figures` in `arena_e2e`, with row 2's range derived and its odds stated; row 18's reader if the hygiene fix left a needle read | mush-bot tests, mush-storykit | M | 16 |
| **FL.6 mush-server's lib and boundary** | row 10 and its neighbors onto the paused clock with `TimeSource::Tokio`; the in-`src` `world_task` tests' drain windows (row 8's kind) onto `settled`-style barriers; the `--src` scope added to `wait_budget` for mush-server | mush-server | M | 15 |
| **FL.7 the gateway's stories** | the ten `Term` copies replaced by the kit's; `across_restart`; restart limits from the gateway's own schedule; rows 12 and 15 closed or re-described; `one_reader` turned on for mush-gateway | mush-gateway tests, mush-storykit, mush-contract-tests | M | 8 |
| **FL.8 mushd's readers** | the 15 `say`, 12 `ask` and 10 `ask_whole` definitions in `sim_lenses` replaced by the kit's; every parse after a needle read moved to `ask_whole`; `after_tick`; row 14; `one_reader` turned on for mushd | mushd tests, mush-storykit, mush-contract-tests | L, mechanical | 3 |
| **FL.9 the small crates** | mush-net-web's and mush-net-ssh's bare sleeps onto barriers and absence-between-marks; mush-sshdoorway's rig onto the kit; the literal-span counter baselined | mush-net-web, mush-net-ssh, mush-sshdoorway, mush-contract-tests | M | |
| **FL.10 close-out** | budgets re-blessed down and the totals recorded; `docs/flaky-tests.md` rows closed against the soak's record; doc 43 §2.3 gains rules F, D, C, S, P and T as #16 to #21; what the soak found that no rule covers | docs, mush-contract-tests | S | |

**Order.** FL.1 and FL.2 first and together (both cut); they share no file. FL.1 stops the
counts rising while the rest lands, and FL.2 is how each later unit proves itself (its note
gives the soak's rate for its suite before and after). Then FL.3 and FL.4 together, the two
suites with 21 runs each; they share only the kit, where each adds its own module. Then FL.5,
FL.6 and FL.7 in any order, FL.8, FL.9 and FL.10.

**What each unit's note must show.** The suite's soak rate before and after, at 24 passes or
more; `wait_budget`'s counts for its files before and after; and, by doc 43 rule 2.3 #14, the
binary's summed minutes, since a barrier is usually faster than the sleep it replaces.

**Not in these units.** The hygiene session's open fix for row 18. Row 1's product note (a quiet
refusal for a host-owed write), which is a design call in `docs/flaky-tests.md`. The nightly's
standing red jobs, which have their own owners.

## 6. Owner survey

Every question has a proposed default, an alternative and a reason. **(expensive to reverse)**
marks an answer that would take a significant refactor to undo.

**Where each stands at Draft 0.2.** The owner ruled once, on 2026-11-22: "Take defaults, cut
both". All eleven questions are ruled, each at its proposed default. Each heading says so; the
alternative and the reason are kept as the record of what was weighed.

**Q-51-1 — Where the shared helpers live.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** a new dev-only crate, `mush-storykit`, as §3.8.
- **Alternative:** each crate keeps its own `tests/support`, and a contract test holds the
  copies to the same text.
- **Reason:** the copies are the fault (§2.3). A crate makes one definition the only one; a
  text-equality gate keeps four and adds a chore. The cost is one more workspace member that the
  four story crates' tests link.

**Q-51-2 — A pinned dice seed for a sim-center match.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** `ArenaConfig::seed: Option<u64>`, read where `seed_for` is called in
  `match_::start`, set by no config file key, exactly as `FieldConfig::seed`.
- **Alternative:** no product change; stories use only fixtures where the result is certain
  (§3.2, tool 2).
- **Reason:** fields have had the knob since FR.5 and it is why field stories are exact. The
  alternative works for standing units and leaves every story with a moving unit to its odds.

**Q-51-3 — The odds bar.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** a story that samples chance states its odds with `Odds`. On the
  pull-request tier, bad luck alone may fail it at most once in 1,000,000 runs. A story that
  cannot meet that runs on the dispatch tier and reports an interval.
- **Alternative:** 1 in 10,000.
- **Reason:** CI runs about 55 times a day. At 1 in 10,000 a tree with fifty such stories fails
  by luck about once in four days, which is the complaint this document answers. At 1 in
  1,000,000 it is once in about a year.

**Q-51-4 — An end-of-reply marker in player text.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** none in this program. Test rigs use the sentinel line. If the soak
  shows the bot's own out-of-arena reads flaking, a later unit adds `OUTPUTPREFIX` and
  `OUTPUTSUFFIX` as player commands, the MU\* convention.
- **Alternative:** build `OUTPUTSUFFIX` now, and have FL.5 use it.
- **Reason:** the 18 rows hold no failure of the bot's own reader. A new player command is a
  surface players' clients will come to rely on, so it should answer a seen need.

**Q-51-5 — The retry policy.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** §4.3's five points: no blanket retry; one retry for tests on
  `flaky-known.toml`; rows expire after 14 days; the soak never retries; the 18 unmatched
  entries are carried over and must earn their renewal.
- **Alternative:** one retry for every test in a story binary on pull-request runs, with
  `flake_report.py --check` failing the run on a flaky pass that is not listed.
- **Reason:** §4.3's table. The alternative blocks the pull request just as often (the check
  goes red on a new flake), and it invites adding a row to make the check pass.

**Q-51-6 — What the ratchet counts.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** story files only, three counters, paused tests exempt, as §3.9.
- **Alternative:** count rig files too.
- **Reason:** a rig is where a wait primitive belongs. Counting rigs would put a raise line on
  every helper the units add.

**Q-51-7 — The soak's schedule and size.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** nightly at 01:17 UTC, clear of the 04:17 nightly; six cells of four
  cores; about three hours.
- **Alternative:** weekly, or on dispatch only.
- **Reason:** §4.1's table. A weekly soak of the same size finds a 1-in-20 flake about 71% of
  the time, after up to a week of pull requests have met it. The box has no other standing
  load at that hour.

**Q-51-8 — Where the observed-flake table lives.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** `docs/flaky-observed.md`, generated, refreshed by whoever opens the
  pull request that adds or closes a row. The soak and the nightly report print it and go red
  on a new name; they commit nothing.
- **Alternative:** the job commits the file to main, or keeps one GitHub issue up to date.
- **Reason:** nothing lands on main today without the owner merging it, and an issue is one
  more place to look.

**Q-51-9 — The instant a stock rig boots at.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** `STORY_EPOCH`, fixed and on no IC boundary. Schedule-bound stories run
  at three named phases. The soak draws a random phase and prints it.
- **Alternative:** a random phase on every run, pull requests included.
- **Reason:** a random phase on pull requests finds more and is a flake by design. The soak is
  the place for it: a failure there costs nobody a merge and names the phase to pin.

**Q-51-10 — tacsim's 416 real-clock arena tests.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** leave them. The ratchet holds their 82 `clock` sites where they are, and a unit
  is cut for one only when the soak names it.
- **Alternative:** an FL.11 that moves `cockpit.rs` and `match_lifecycle.rs` to the paused
  clock, as TS.10a did for the fields.
- **Reason:** none has failed in the window, and TS.10a's measure was run time, which doc 43
  owns. The alternative is about 270 tests in two files of 17,000 lines and more.

**Q-51-11 — A fire count on the staff schedule readout.** **Ruled 2026-11-22: accepted as written.**
- **Proposed default:** not now. `after_tick` reads `@ps` as it prints today (a tick's next
  fire), and FL.8 reports whether whole seconds are fine enough.
- **Alternative:** add a per-tick fire count to `@ps` in FL.8.
- **Reason:** a readout change is a product change, and the first barrier that needs it may not
  exist. If it does, the count is a small one.

## 7. What this audit did not check

- **Every site by hand.** §2.1 says which were read. The two known errors are named there.
- **Poll loops with no sleep.** A loop that steps a fake clock and asks again has no needle and
  is not a wait; it is not counted.
- **Whether a pinned seed makes every `sim_lenses` story exact.** §3.2 argues it from the draw's
  label. FL.3's soak is the proof, and tool 2 is the fallback.
- **The pass length on conclav.dev.** Forty minutes is from 2026-11-03's summed minutes. FL.2
  measures it.
- **Whether nextest 0.9.148's own stress flags can replace the soak's loop.** FL.2 reads
  `cargo nextest run --help` and takes them if they are there.
- **Rows 6 and 16.** Their causes are not known and they are in no class. Row 6 is now
  diagnosable (#764). Row 16 was seen once.
- **The web client's tests and the shell tests.** No row names them.
