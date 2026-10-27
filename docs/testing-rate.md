# Testing at a higher world rate

*The testing-rate trial, 2026-10-27, on main `4d49d32d`. The owner's question: what would it take to
run the wall-clock-bound tests at a world rate of 168 (an IC week per real hour) instead of 24,
selectively where a test needs 24, and why does the 24 requirement exist when production is not
guaranteed to run at 24? This note classifies every rate-24 assumption in the port, surface, market,
server-story and bot code, measures what a lift to 168 buys the one test that is wall-bound on the
game clock, and recommends. It changes no behaviour: the one code change is the env-gated lift in
the bot's mining rig (§4).*

## 0. In brief

- **Why 24.** Doc 18 §4.4 rule R3 sizes the trade run so a passage is "under a day and around a
  day" of real time and a pilot who logs in once a day runs a career; doc 10 1A's "one OOC hour per
  IC day" is 24 IC seconds per real second. `PLAY_RATE = 24` (`crates/tacsim/src/port/readiness.rs:37`)
  is that number made a readiness line: the Rate line reads `[ready]` at exactly 24, `[ready]` at
  any rate on a test world (`TacsimStore::test_world`, which only `tacsim_test_world_module` sets —
  IC3 lead ruling L15: `build_engine` never does), and `[flagged] ... play runs at 24` otherwise, and
  `+coop/enrol` is gated on it (`coop.rs:782`, `:880`; IC12 L5). Nothing else reads it: a departure
  to a mining site asks `port_ready`, which is the site's own lines (`readiness.rs:1262`).
- **Production is not guaranteed at 24 — and today it is configured at 24.** `deploy/dev1/instance.env:81`
  and `deploy/dev2/instance.env:81` both render `IC_RATIO=24` (doc 18 §4.4's "dev1 seeds
  `IC_RATIO=4`" is the IC0-era state; the persisted timeline wins over a changed `[game].ic_ratio`
  at boot, `crates/mush-server/src/timeline.rs:55-66`, so a long-lived world's live generation is
  whatever staff last opened). A deploy at any other rate closes the Co-op desk world-wide until a
  staffer types `@tacsim/time rate=24`.
- **Which tests are wall-bound at all.** Only the bot's rigs and a handful of real-server tests
  (§2). The three mush-server stories that boot at 24 (`ic_slice_loop.rs`, `sm_story.rs`,
  `ic_buyout_dormancy.rs`) drive a fake clock with `TestWorld::set_now` (`sweep`, `to_ic`): their
  147–170 s is landings and ground builds, not clock, and a higher rate buys them nothing. The
  tacsim suites are the same.
- **What 168 buys the bot canary: 165 s of 528 (31 %), measured** — 528.0 s at 24 against 363.0 s
  with God lifting to 168 after the enrolment (§4), the lane reaching `Scanned` in both. The hop
  falls from 158 s to 23 s, the launch and the scan from 38 s to 8; the other 330 s are the desk
  and pad openings (one act a fire on the 5-s port tick), the market's 60-s sweep and the lane's
  own retry cadences, which no rate shrinks. The lift is safe after the enrolment (nothing on the
  mining loop reads the Rate line again), and the lane's own cadences, sized to 24, are upper bounds
  that hold at any rate at or above about 6.
- **Recommendation (§5).** Lift the canary to 168 after enrolment — built here as the canary's
  default, the owner's call of 2026-10-27, `MINE_RATE` overriding it (24 is the rate-24 run) —
  and nothing else; the per-fire budgets (b) survive 168 by arithmetic but the
  dispatcher's one slot, the emergency reading's real hour and the marker rings need the owner's
  word before any world runs there for long; the real-time windows (c) are design decisions whose
  IC meaning shifts 7× and want a ruling, not a code change; and `PLAY_RATE` should become
  `[tacsim.time].play_rate` (one number, default 24), not a range.

## 1. The clock, in one paragraph

`ic = epoch_offset + real_seconds × ic_ratio` (`crates/mush-core/src/clock.rs`). Generation one is
seeded from `[game].ic_ratio` into the persisted world timeline (`timeline.rs`); `@tacsim/time
rate=<n>` opens a later generation (`crates/tacsim/src/commands.rs:471-580`), inside the kernel's band
`1..=3600` (`MIN_WORLD_TIME_RATE`, `MAX_WORLD_TIME_RATE`; owner decision F9(a)), and the compatibility
`GameClock` every module reads follows it (`world.rs:241`, `:399`, `:425` — the economy sweep's
`ratio` at `sweep.rs:93` is the live rate, not the configured one). Every tick fires on real time:
the port tick every 5 s (`port_tick.rs:85`), the surface tick every 1 s (`surface/tick.rs:108`),
the market sweep every 60 s (`mush-market/src/module.rs:31`), the economy sweep every 30 s
(`mush-economy/src/clock.rs:39`); `[tacsim].ground_pulse_ms` 2000 and `space_pulse_ms` 6000 are
parsed and inert (`tacsim/src/config.rs:20`). So a higher rate means more IC time per fire and
fewer fires per IC day: 720 port fires an IC day at 24, 103 at 168, 4.8 at 3600.

## 2. Which tests are wall-bound

| Suite | Clock | Wall at the gate | What a higher rate buys |
|---|---|---|---|
| `tacsim/tests/*` (every `TestWorld` suite) | fake: `set_now` / `fire_due` | by fire count | nothing: a `to_ic` jump is one `set_now`; only loops that fire until an IC condition shrink, and those already pick their rate |
| `mush-server` `commerce_composition::ic_slice_loop` (five `ic12_*`, the `r9*`, `trade_*`) | fake: `sweep` = `set_now` + `fire_due` (`ic_slice_loop.rs:474-480`), `to_ic` one jump (`:675-684`); the lane flown at 3600 after enrolment (`:1314`, L1) | 59–149 s a test; the slowest `ic12_the_sailing_plan_skips_and_holds` 149.1 s | nothing: the cost is `fire_until` loops × landings (`:4004`) |
| `commerce_composition::sm_story::sm8_the_story_on_the_production_composition` | fake; the world rate 24 throughout, asserted (`sm_story.rs:1125`) | 147.6 s | nothing: 6-s fake fires in loops (`:531` 300 fires, `:1261` the mining run), each a landing plus the ground warm |
| `commerce_composition::ic_buyout_dormancy` (`ic13_*`, `ic14_*`, `sm2_*`) | fake; the dormancy week jumped by `sweep` | 57–170 s a test; the slowest `ic14_a_relet_that_never_lands_gives_up_after_six` 169.8 s | nothing |
| `mush-server` lib `submission_disposition::tests::player_market::*` | real server, 50 ms polls, no IC wait | 12–27 s | nothing: no IC-timed wait |
| `submission_disposition::tests::paid_purchase::genuine_registry_purchase_writer_unknown_expiry_and_recovery` (`#[ignore]`) | real: the Registry worker's maintenance ticks (~7 s real) and a 30-s real quote expiry (`paid_purchase.rs:41-44`, `:705` `wall_after`) | about 1,070 s by its own comment | nothing: real-time by design |
| `mush-bot/tests/arena_e2e.rs`, `arena_lane.rs` (the wall-clock arena stories) | real: the arena's 250 ms pulse and match clock | minutes | nothing: the arena's clock is not the world rate |
| **`mush-bot/tests/mining_cohort.rs::the_mining_root_runs_under_the_bots_serde`** (the canary, every build) | **real: the production root over SSH, the game clock at 24** | **528 s** (conclav.dev, run 40422445955) | **the hop (3.7 real min at 24), the launch (12.5 s), the scan (25 s) — §4** |
| `mining_cohort.rs::four_bots_mine_and_sell_over_ssh_in_line_mode` (`#[ignore]`, by hand) | real, as the canary, the whole loop | tens of minutes | the hops both ways, the drill cycles (75 s each at 24, up to eight), the soundings |

The gate's JUnit (run 40424121890, the last green sweep) is where the mush-server times above come
from; the canary's figure is the conclav.dev cold sweep's.

## 3. Every rate-24 assumption, classified

Kinds: **(a)** the `PLAY_RATE` enrolment gate; **(b)** per-fire budget arithmetic that scales with
the ratio (fewer fires an IC day at a higher rate); **(c)** a window specified in real time by
design; **aware** marks a site that converts at the live rate and assumes nothing (listed because
the grep found it, and because it is what makes (b) and (c) survivable).

| Site | What it assumes | Kind | At 168 | A test pins the 24 value? |
|---|---|---|---|---|
| `tacsim/src/port/readiness.rs:36-37` `PLAY_RATE = 24` | doc 18 §4.4 R3: play runs at 24 | (a) | the Rate line `[flagged] … play runs at 24` on any production world not at 24 | `mush-bot/tests/support/mine.rs:64` `RATE_READY` (at boot, every canary run); `ic_slice_loop.rs:1159`; `sm_story.rs:1125`; `ic_slice_loop.rs:3431` (the flagged text at 3600); `readiness.rs`'s own gate tests |
| `readiness.rs:376-402` the Rate line's three arms | Ready at exactly `PLAY_RATE`; Ready at any rate on a test world; Flagged otherwise | (a) | as above; a test world (`tacsim_test_world_module` only, IC3 L15) is Ready at 168 | the `ic_slice_loop.rs:3431` text |
| `port/coop.rs:782-800` `gated(Binding::Rate)`, `:880` `enrol_gate`, its one caller `:1285` (`+coop/enrol`) | the gate reads the Rate line | (a) | enrolment refused (`NOT_OPEN`) until a staffer opens 24; nothing after enrolment reads it (`port_ready`, `readiness.rs:1262`, is the site's lines only) | `coop.rs` tests `:2149-2220` |
| the rigs' `[game].ic_ratio = 24`: `mine.rs:128`, `ic_slice_loop.rs:115`, `sm_story.rs:182`, `ic_buyout_dormancy.rs:105`, `paid_purchase.rs:48`, `player_market.rs:161` | boot at the play rate so the pilots enrol | (a) | unchanged: boot at 24, lift after enrolment (L1); the last two enrol nobody and read no Rate line (24 by convention) | each asserts "the world timeline is seeded from [game].ic_ratio" |
| `ic_slice_loop.rs:1314` + `:1799` `set_rate`, `ic_buyout_dormancy.rs:1003`: `@tacsim/time rate=3600` as the one staff act after enrolment | lead ruling L1 / Q-18-61: the lift comes after the enrolment | (a) | the pattern this trial reuses | — |
| `split_boundary/relay_registry.rs:3961-3963, 5188, 5263, …` `@tacsim/time rate=24` then `rate=1` | a chosen rate for the subscription's IC billing periods | — (fake clock, deliberate) | — | — |
| `split_boundary/provisioning_smoke.rs:375` `IC_RATIO` renders `[game].ic_ratio` (3) | the template substitution | — | — | — |
| `port_tick.rs:85-89` 5 s; 64 agents visited, 16 worked a fire | the campaign rotation's budget per real fire | (b) | 720 → 103 fires an IC day: a principal's one act a fire is 103 acts an IC day | no; `drones.rs` T5 and the cost lane count fires, not days |
| `port/stock.rs:8, :19-31` one act per principal per fire (D-18-32); the drip one act a fire, one drip per IC day per row | the clerk's step budget | (b) | the Ceres clerk's rows (emergency bids, restocks, batches, 4 drips, re-posts) share 103 slots an IC day instead of 720; doc 20a §5.3.9's figure ("whether the one-act-a-fire slot starves the drip or the mill") is the one to re-read at 168 | no |
| `port/mill.rs:45-49` one act a fire; at most two batches and two restocks a mill an IC day | the mill's cadence (D6, D7) | (b) | 4 of 103 slots: holds | no |
| `port/def.rs:848-856` the drip's 40 t an IC day "at world rate 24 … 960 t of each bulk ore a real day"; "the capital clause binds Occator in about nine real days" (doc 20 §7.3, §12.3) | the ore economy's real-day throughput at 24 | (b)-economics | ×7 a real day: 6,720 t; Occator's capital clause in about 1.3 real days, the Exchange's in about 5 | no |
| `port/site.rs:37, :919` the site clerk one act a fire; the opening "N of M" rows | the pad opens after N port fires plus the ground warm | (b) | a fixed real cost (N × 5 s), rate-independent: a floor no rate shrinks (§4) | no |
| `surface/tick.rs:36, :395`, `surface/ground.rs:36`, `surface/mod.rs:129` warm one wanted site a fire (1 s) | the ground build is paid per site per boot | (b) | fixed real cost, rate-independent | no |
| `surface/tick.rs:108-112` 1 s; 64 rigs visited, 16 worked a fire; a finish lands on the fire after its due | the surface act budget | (b) | a launch (300 IC s) is 1.8 real s, a scan 3.6, a drill cycle 10.7 — each a start and a finish, so 30 rigs ≈ 5.6 acts/s against 16 a fire: holds; at 3600 a cycle is 0.5 s, under the pulse, and acts serialise at one fire each | no |
| `port/yard.rs:1985-1992` the dispatcher one act a fire on ONE submission slot shared with both rigs' surface steps | the drones' throughput ("below two players with one rig each", 12b-E12) | (b) | the house's IC-day output falls 7× (103 slots); the mill's 40 t an IC day no longer "binds first" | no (`drones.rs` T5 is 4,000 fake fires) |
| `port/recall.rs:73` a blocked recall re-evaluated every 24 fires | 2 real min | (b) | 48 IC min at 24 → 5.6 IC h at 168 | no |
| `port/executor.rs:1631, :2333` "the step waits a fire" on back-pressure or an unlanded burn | a queued order advances at the port tick | (b) | a wait costs 7× more IC per fire | `ic_slice_loop.rs:1480-1505` asserts fire boundaries against IC instants (rate-aware through `to_ic`) |
| `port/price.rs:244-246` `EMERGENCY_CHECK_SECS` 3,600 real, "once an hour — one IC day at the shipped world rate of 24" | the emergency reading's cadence is one IC day | (b)/(c) | one reading per 7 IC days: a one-IC-day price spike can pass unread | no |
| `mush-market/src/boot.rs:145, :265, :300, :657` the boot/withdraw arms' per-fire budgets | a visit budget per 60-s sweep | (b) | 7× fewer sweeps an IC day | `boot.rs:657` asserts the budget, not a rate |
| `mush-economy/src/clock.rs:34-39` the 30-s sweep, `MAX_CATCH_UP_CAP` 8 periods a sweep | a schedule catches up at most 8 periods a sweep | (b) | an IC-day schedule is 17 sweeps apart: holds; an IC-hour schedule at 3600 (1 real s) would fall behind the cap | no |
| `surface/plan.rs:88` `PLAN_WAIT_IC_SECS` 300 IC s "12.5 real seconds at rate 24"; `:61` `PLAN_BOUND_IC_SECS` 288 IC h "twelve real hours"; `:67` `IDLE_PARK_IC_SECS` 24 IC h | IC constants, their real pace quoted at 24 | IC (shrinks) | 1.8 s (two surface fires), 1.7 real h, 8.6 real min | no |
| `surface/excursion.rs:72-84` `LAUNCH_S`/`RECOVER_S` 300, `UNLOAD_S` 600 IC s "25 real seconds at rate 24" | IC constants | IC (shrinks) | 1.8 s, 3.6 s | no |
| `port/def.rs:93, :768` `dormancy_real_days` (the lane: 7; `MAX_DORMANCY_REAL_DAYS` 365); `port/recall.rs:66-67, :795-830` `idle`/`dormant` in real seconds | doc 18 §4.1 "seven real days without a departure, judged by the tacsim tick" (D-18-21; IC13 L3) | (c) | 7 real days = 1,176 IC days (3.2 IC years) against 168 IC days at 24 | `ic_buyout_dormancy.rs:1784-1790` asserts 7 real days less an hour (fake clock; rate-independent) |
| `port/coop.rs:260-266` `leased_at`, `ended_at` real seconds | the dormancy clock's anchor | (c) | unchanged | — |
| `surface/tenure.rs:20-21` `CLAIM_TERM_IC_SECS` 168 IC days "seven real days at rate 24, the charter's dormancy week"; `:50-53` `CLAIM_FEE_WINDOW_SECS` 3,600 real; `port/claims.rs:536` the fee's real hour; the licence 168 IC days "seven real days" (doc 20 §1 beat 1, §11 Tenure; `port/licence.rs:91-95` `LICENCE_SETTLE_WINDOW_SECS` 3,600 real) | an IC term chosen to equal the real-day dormancy week at 24; fee windows in real seconds bounding a 30-s sweep | (c) | a claim term and a licence are one real day while the lease's dormancy week stays seven: the two "weeks" the design matched diverge 7×; the fee windows hold at any rate | no (`claims.rs`'s suites are fake-clock) |
| `port/buyout.rs:54-57` `BUYOUT_WINDOW_SECS` 3,600 real (lead ruling L11) | bounds a stuck settlement | (c) | holds at any rate | no |
| `surface/act.rs:318-326` `UNLOAD_MARKER_KEEP_S` 3,600 real (O-22; "a dozen markers an hour at rate 24") | the hopper ring's per-entry horizon | (c) | about 84 markers an hour against `MAX_MARKER_RING` 128 (`mush-inventory/src/record.rs:95`): holds; at 3600 the ring fills and `MarkersFull` waits | the cost lane's ring pin (fake clock) |
| `mush-inventory/src/record.rs:91, :95` `MARKER_RETENTION_SECS` one real day, a ring of 128 | the market's sale markers (doc 20 SM.11 as built: "about sixteen sold holds in one real day would" fill it; Q-18-74) | (c) | a hold sold every 90 real min fills it; a four-bot cohort at 168 does that | no |
| `port/stock.rs:974-977` `DRIP_RENEW_SECS` 86,400 real against the market's 90-real-day TTL (SM.5, reviewer F2) | a standing intake is renewed a real day before its deadline | (c) | holds | no |
| `mush-market/src/api.rs:44-46, :887, :1248` listing TTLs real (90 d max, 14 d default); `commands.rs:1450-1455` lot listings capped at `RESERVATION_TTL_IC_DAYS` 7 | listings live in real time, holds in IC time | (c) + aware (`lot.rs:1541` `hold_ttl_real_secs` = 7 IC days ÷ the live rate) | a lot listing lives at most one real hour | no |
| `mush-market/src/trade.rs:255-257, :2919`; `record.rs:1060-1062` the Draw/Deliver windows and a deal's leg cutoff in real seconds; `record.rs:249, :874, :1456`, `lot.rs:335, :762`, `projection.rs:295`, `port/map.rs:303`, `port/stock.rs:1031` stamps and deadlines on the wall clock | the hand-over windows and the listings' deadlines are real | (c) | a passage is 7× shorter in real time, so a window is 7× looser; below 24 it tightens | no |
| `port/orders.rs:233` a daily departure's `not_after_s` real; `port/when.rs:2, :17` "a daily UTC time or +Nh" | the captain's day is a real day (doc 18 §4.4) | (c) | a daily departure is every 7 IC days | `ic_slice_loop.rs:2663` ("a daily hour two real hours on") |
| `port/when.rs:170-200` `ic_at_utc`, `ic_at_real`; `port/voyage.rs:510` `real_until`, `:453` `real_span` | conversions through the timeline's generations | aware | correct at any rate | `ic12_a_rate_change_mid_passage_relabels_the_arrival` |
| `surface/act.rs:90, :6241, :6406, :9174` the finish's offer fixes the real second; the ring admits at the real second | replay guards on real time | aware | — | — |
| `mush-bot/src/mine/mod.rs:34-43` the tempo "as the server ships it at world rate 24"; `lane.rs:94-101` `HOP_MS` 15 min, `ACT_MS` 5 min, `RUN_MS` 15 min, `STAKE_RECORD_MS` 180 s; `:116` `SECOND_SOUNDING_AFTER_MS` 150 s "one IC hour at the play rate 24" (the heard budget, `act.rs:9014`); `lines.rs:136` | the bot's real waits are upper bounds sized to 24 | (c), the bot's own | all hold at 168 (the hop bound 15 min against 32 s; the sounding wait 150 s against a 21-s budget hour, over-waiting 129 s); they break BELOW about 6 (the hop) and below 24 (the second sounding is not heard) | `mine.rs:64` `RATE_READY` at boot; the cohort's heard windows (`SLACK_MS`) |
| `mush-core/src/clock.rs:76-85` the band `1..=3600`; `:941-943` a unit test at 24; `mush-server/src/timeline.rs:154-330` unit tests at 2, 3, 4, 99 | the band; persisted truth beats config | — | 168 is in band | — |

## 4. The trial: the bot canary at 24 and at 168

**Method.** Lead ruling L1's pattern, as `ic_slice_loop.rs` plays it: the rig boots at 24 (the Rate
line gates `+coop/enrol`), the pilot enrols, then God opens a new generation with `@tacsim/time
rate=168`, and the lane plays on. Built in this branch: `mine::rate_lift` takes the rig's own
default rate — `LIFTED_RATE` 168 for the canary since the owner's call of 2026-10-27, `BOOT_RATE`
24 (no lift) for the cohort — and `MINE_RATE` overrides it (unset or empty leaves the default; 24
is the rate-24 run), the staff act `mine::lift_rate` asserted on the verb's own reply, the drive
split in two phases around it (`mine::drive_lane_since`, one clock, one budget), in
`crates/mush-bot/tests/support/mine.rs` and `tests/mining_cohort.rs`. The lane's own
cadences (`crates/mush-bot/src/mine/lane.rs`) are untouched. Both runs on conclav.dev (32 cores),
the one pushed tree (`77d2f56c`), the ci profile, Redis on, `--success-output immediate` so the
`canary:` lines reach the job log; the 168 run carried `MINE_RATE=168` through a temporary `env`
dispatch input on gate.yml that is NOT in this branch (the measurement patch is in the session's
notes folder; PR #23, the conclav-dev runner change that owns gate.yml, merged since without it).

**The two runs** (conclav.dev, the `vps-gate-conclav-dev` queue, one after the other):

| | 24 throughout (the default then; `MINE_RATE=24` now) | 24, then 168 after the enrolment (`MINE_RATE=168` then; the default now) |
|---|---|---|
| Gate run | 40431777853 | 40431814456 |
| The canary (JUnit) | **528.0 s**, PASS | **363.0 s**, PASS |
| The lane's drive (its own clock) | 524 s | 359 s |
| Reached `Scanned` | yes (ArrivedSite, Launched, Scanned) | yes (the same three) |
| The job, checkout to artifact | 10 min 16 s (a warm relink of the bot's test binary) | 6 min 21 s |

**The flipped default, re-measured** (no `MINE_RATE`; the canary lifting to `LIFTED_RATE` on its
own; the tree merged with main at PR #23): gate run 40435825559, PASS, the canary **363.2 s**
(JUnit), the lift at +61 s, `ArrivedSite` at +346 s, `Scanned` at +358 s, the job 6 min 21 s — the
168 column of the trial, to the second.

**Where the wall goes**, beat by beat, from the `canary: +N s send …` lines (the lane's clock
starts at its first line, about three seconds after the boot):

| Beat | At 24 | At 24 → 168 | Bound by |
|---|---|---|---|
| The Co-op desk opens: `+coop/enrol` answered `NOT_OPEN`, retried every 10 s | +1 → +61 s (7 sends) | +1 → +61 s (7 sends) | (b): the stock arm's one act a clerk a fire seeds and lists both ports' stock on the 5-s port tick until the Pools and Listings lines read Ready |
| The lift: `@tacsim/time rate=168` as God — "World time generation 2 opens at 168 IC seconds per real second." | — | +61 s | — |
| Kit-up: refit, licence, lease, chart, the Exchange board | +61 → +70 s | +62 → +70 s | the lane's 500 ms line gap and the 2-s licence step |
| The pad opens: `+ship/plan ahuna` answered `SITE_CLOSED`, retried every 10 s | +71 → +252 s (19 sends) | +72 → +252 s (19 sends) | (b): the surface tick's one ground warm, then the site clerk's opening rows, one act a fire on the 5-s port tick |
| Bunkers: two Draws through the numeric wires, the re-plan | +252 → +259 s | +253 → +259 s | the lane's 5-s bunker round |
| The walk to the hull, `+ship/board`, `+ship/lift` answered "have not come aboard yet", retried every 30 s | +259 → +323 s (3 lifts) | +260 → +323 s (3 lifts) | (c): the market's 60-s real sweep settles the Draws' deliveries |
| **The hop** (the lift accepted → `ArrivedSite`) | **+323 → +481 s = 158 s** | **+323 → +346 s = 23 s** | the passage, about 3,800 IC s at the canary's load: IC time, so the rate |
| The witness map | +481 → +484 s | +346 → +349 s | — |
| **The launch** (`launch go` → `Launched`, 300 IC s) | **13 s** | **3 s** | IC time (12.5 s at 24, 1.8 s at 168) plus the 1-s surface fire |
| **The scan** (`scan go` → `Scanned`, 600 IC s) | **25 s** | **5 s** | IC time (25 s at 24, 3.6 s at 168) plus the fire and the lane's settle |

So the lift to 168 saves **165 s of 528 (31 %)**: the three rate-dependent beats fall from 196 s to
31 s, a 6.9× cut on the hop and the fire granularity on the two acts, and nothing else moves by a
second. The other **330 s are the floor no rate shrinks**: the desk opening (61 s) and the pad
opening (181 s) are per-fire budgets on real pulses — kind (b), the owner's — the goods wait (60 s)
is the market sweep's real period — kind (c) — and the rest is the lane's own retry and settle
cadences. A lift to 3600 would take the 31 s to about 10 (the pulses bound every act at one fire)
and the test would still run about 340 s. For the `#[ignore]`d cohort, which plays the whole loop,
the rate-dependent share is larger (the hop home, up to eight 75-s drill cycles, two 50-s soundings)
and the same floor applies per bot; its `SECOND_SOUNDING_AFTER_MS` (150 real s, one IC hour at 24)
over-waits at 168 by 129 s but still clears the heard budget's hour (21 real s at 168).

**The two mush-server stories the brief named were not trialled**, because they are not wall-bound
(§2): `sm_story.rs` and `ic_slice_loop.rs` drive `TestWorld::set_now`, so a rate changes which IC
instant a 6-s fake fire lands on, not how long the test runs; their 148 s and 149 s are landings
and ground builds. `ic_slice_loop.rs` already flies its lane at 3600 (L1) for the same reason this
trial lifts after enrolment: the fake clock's fire loops are shorter in IC terms, not in wall.

## 5. Recommendation

1. **Lift the bot rigs to 168 after enrolment; lift nothing else.** The canary and the cohort are the
   only always-wall-bound tests whose wall is game clock, and nothing on the mining loop after
   `+coop/enrol` reads the Rate line. The owner ruled on 2026-10-27: the canary's default is 168
   after enrolment (`LIFTED_RATE`), 165 s off every sweep's longest bot test, the floor of 330 s
   staying (§4); `MINE_RATE` overrides it, so `MINE_RATE=24` is the rate-24 run when a change to
   the enrolment or the passage wants the world as the deploy envs seed it. The cohort stays at 24:
   its four pilots enrol at different times (each site's second launches after the first's scan),
   so there is no one point after which the Rate line is behind every pilot — a lift there waits
   for `PLAY_RATE` as configuration (item 4) or a per-pilot gate. The sweep's tail on conclav.dev was
   the canary (the conclav-dev runner note), so the saving is wall off the gate, not test time. The mush-server stories and the tacsim suites gain nothing from any
   rate: their clock is `set_now`. The real-server tests (`paid_purchase`, the arena stories) wait on
   real-time things the rate does not touch.
2. **(b) at 168 survives by arithmetic, with three figures for the owner before any world runs there
   for long:** the dispatcher's one submission slot (the house's output an IC day falls 7×; whether
   that matters is doc 20's §3.4 question, not a test's); the emergency reading's real hour (one
   reading per 7 IC days: either the cadence becomes IC — `EMERGENCY_CHECK_IC_SECS` one IC day
   converted at the live rate — or it is accepted); and doc 20a §5.3.9's starvation figure, which the
   cost lane should print at 168 before anyone trusts the clerk's slot there. The per-fire budgets
   themselves (`PLAY_RATE`, 16 agents, one act a fire) stay as the owner ruled.
3. **(c) is design, not debt.** The dormancy week, the daily departure, the listing TTLs, the fee and
   buyout windows and the marker rings are real-time by decision (D-18-21, L11, O-22, Q-18-74, SM.5
   F2). At 168 each keeps its real meaning and its IC meaning shifts 7×; the one pair the design
   deliberately matched — the lease's dormancy week (7 real days) and the claim term and licence (168
   IC days, "seven real days at rate 24") — comes apart, and the market's sale-marker ring (128 a real
   day) is reachable by a cohort. Those two want the owner's ruling before 168 is anything but a
   test setting; the rest can stand.
4. **`PLAY_RATE` should become `[tacsim.time].play_rate`, one number, default 24** — read by the
   Rate line in place of the constant (`readiness.rs:37`, `:384`, `:400`'s text), with the two story
   assertions and `RATE_READY` reading the configured value. Not a range: the content (R3's "under a
   day", the dormancy week equal to the claim term, the drip's real-day economics) is sized to one
   rate, so a deploy at another rate is a campaign decision the operator should declare in the
   instance's config, where the gate then holds the world to the declared rate instead of to a
   constant the operator cannot see. `[tacsim.time]` already holds `rate` (inert since E7r1:
   generation one is `[game]`'s) and the catch-up policies, so it is the natural home; the deploy
   template renders `IC_RATIO` into `[game].ic_ratio` and would render the same number here. The
   test-world arm stays as it is. This is a small cut (a config field, three lines in readiness.rs,
   the two string assertions) and is NOT built here: `PLAY_RATE` is the owner's.

## 6. What this branch changes

`crates/mush-bot/tests/support/mine.rs` (`BOOT_RATE`, `LIFTED_RATE`, `rate_lift` with the rig's
default, `lift_rate`, `drive_lane_since`; `config` reads `BOOT_RATE`),
`crates/mush-bot/tests/mining_cohort.rs` (the canary's drive in two phases around the lift, 168 by
default; the summary line names the rate), this note. No game code, no constant, no gate file. The
one behaviour change: the canary's world runs at 168 from the enrolment on; `MINE_RATE=24` restores
the rate-24 run.
