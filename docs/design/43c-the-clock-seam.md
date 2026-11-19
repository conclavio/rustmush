# 43c. The clock seam — one time provider for the world task and the arena (TS.10, design note)

**Status:** Draft 0.2, 2026-11-18. This is the design note that doc 43 §7's TS.10 row asks for before any
build. Draft 0.1 (2026-11-04) was a read-only survey of main `7a535806`. Draft 0.2 re-verifies every anchor
and count against main `1e9b4117`, after TS.12 moved tacsim's tests into the four domain binaries and
PR #670 planned the CI cells by measured time. The corrections are made in place and listed in §8. TS.10a
is built in the same pull request (doc 43 cut notes, "TS.10"). The questions in §7 are the owner's. They
have not been ruled, and the build takes their defaults (§7).

**What it answers.** Doc 43 §5.8 found that the stories' real-time floor is three things: the world task's
ticks, the arena's windows (`[tacsim.field].disengage_s`) and the rigs' sleeps. In its words, "the world clock
is faked by the rig, the arena's is not, and no test can shorten the window except by the setting." TS.10 puts
`world_task`'s and `arena_task`'s `now` behind one provider. The in-process server, mushd and the field tests
can then fast-forward under tokio's paused clock (`start_paused`).

## 0. In brief

1. **The arena half is already done in the engine.** §5.8 is half wrong.
   - `arena_task` reads no wall clock of its own. Its base, its deadlines and its `elapsed_ms` are all
     `tokio::time::Instant` (`crates/tacsim/src/arena_task.rs:153`, `:1896`, `:3031`–`:3046`).
   - Its own tests already run on the paused clock: 53 `#[tokio::test(flavor = "current_thread", start_paused
     = true)]` in `tests/domain_arena/`: 20 in `arena_task.rs`, 30 in `cockpit.rs`, 2 in `minds_pool.rs` and
     1 in `match_lifecycle.rs`. Draft 0.1 counted 22: arena_task's 20 and minds_pool's 2.
   - The minds' wait goes through `spawn_blocking`, and tokio does not auto-advance a paused clock across one
     (`arena_task.rs:2438`–`:2454` says so).
   - So the arena's window is real time in the field tests **because of the rig, not the engine**.
     - At `1e9b4117`, 189 field tests in `tests/domain_field/` ran on `flavor = "multi_thread", worker_threads
       = 2` (`field_gc4.rs:114` and the rest). Such a runtime cannot pause. 76 files under `crates/tacsim/tests`
       use that flavour.
     - Their wait loop's deadline was a `std::time::Instant` (`support/field/field_host_rig.rs:328`). TS.10a
       replaced that loop.
2. **The world half is the real seam.**
   - `world_task` reads wall time from `SystemTime` through `mush_config::unix_now_s`/`unix_now_ms`. It feeds
     that reading to `Engine::set_now` (`world_task.rs:3884`–`:3893`) and to `fire_due` (`:4689`).
   - It arms its timer as "unix due minus unix now", slept on tokio (`sleep_until_unix_ms`, `:6114`).
   - On a paused runtime that sleep returns at once, the loop wakes, the unix reading has not moved, and nothing
     is due. The loop re-arms and spins virtual time forward without the world ever seeing it pass.
   - The provider has to make the unix reading and the tokio sleep **one clock**.
3. **The provider** (§2) is a cloneable `TimeSource` with two arms.
   - `System` is the default and production's arm: unix time from `SystemTime`, sleeps on tokio, exactly as
     today.
   - `Tokio { epoch_unix_ms, base }` computes unix time as `epoch_unix_ms + (tokio::time::Instant::now() −
     base)`, so under `start_paused` both readings advance together.
   - The world task takes it in `ServerDeps`, and the arena takes it in its spawn terms. Production builds only
     `System`.
   - The arena keeps its tokio `Instant` arithmetic. It gains the handle only so that a tape, a snapshot and a
     world tick all name one epoch.
4. **What it buys.**
   - A field test whose body does nothing inside a 90-s window spends that window as CPU instead. At the rigs'
     tuned tempo (100 ms) that is about 900 pulses of a small arena: seconds, not 180 s.
   - §5.8's seven shard_1 tests carried at least 990 s by construction; TS.16 has already taken the cheap half
     of that. They are `domain_field` tests now, and TS.10a's measurement covers the rest (doc 43 cut notes,
     "TS.10").
   - The in-process mush-server and mushd stories lose their tick floors: the port tick (5 s), the surface
     tick (1 s), the market sweep (60 s), the economy sweep (30 s), the durable-decision tick (250 ms) and the
     schedule poll.
5. **What it cannot buy** (§3): anything across a real I/O boundary that the runtime does not own.
   - That means Redis, a child process (the mushd binary stories, the gateway and server generations) and a
     peer on another runtime. All of these keep wall time.
   - A paused test that awaits one of them fires its own timeouts early. The seam is therefore for
     **in-process rigs on the in-memory store**, and a guard (§5, TS.10c) refuses the paused arm when the store
     is Redis.
6. **Build plan** (§5): five cuts, TS.10a–e, about L in total, as doc 43 sized it. The first cut moves the
   field rig onto the paused clock. It needs **no engine change**, and its measurement is what the other four
   are judged on.

## 1. The clocks today

| Reader | Source | Where | Pausable today |
|---|---|---|---|
| World task: `Engine::set_now`, `fire_due`, the timer arm | `SystemTime` via `unix_now_s`/`unix_now_ms` | `mush-server/src/world_task.rs:1377`, `:3893`, `:4689`, `:6114`; `mush-config/src/lib.rs:64`–`:76` | no: unix time does not follow tokio's clock |
| World task: intervals, retries, shutdown grace, defer queue | `tokio::time` and `std::time::Instant`, mixed | `world_task.rs:977`, `:2943`, `:3437`, `:3453` (tokio); `:3884`, `:5373`, `:5427` (`std`); `DeferInstant` is `tokio::time::Instant` (`world_task/defer_queue.rs:314`) | the tokio ones yes, the defer queue among them; the `std` ones no |
| Other unix readers in mush-server | `SystemTime` directly | `staff.rs:349`, `world_task/staff_ops.rs:316`, `archive.rs:369`, `quarantine.rs:3157`, `config_ext.rs` (4 sites), `modules.rs:1634`, `process_fault.rs:216` | no |
| The minds bulk executor's worker day | `SystemTime` | `tacsim/src/minds/bulk/{executor.rs:304, world.rs:156, conduct_participant.rs:29, registry_participant.rs:143}` | no |
| Arena task: base, deadline, `elapsed_ms`, the `wall_now_ms` handed to `Arena::step` | `tokio::time::Instant` | `tacsim/src/arena_task.rs:1896`, `:2052`, `:3031`, `:3044` | **yes** (its tests already do) |
| Arena task: pulse cost, pool wait | measurement only, never fed to state: the pulse cost on tokio's `Instant`, the pool wait on `std`'s | `arena_task.rs:2274`, `:2453` | n/a: observability |
| Arena clock: tempo, last-engaged, last-shot, disengage | arena milliseconds, from `wall_now_ms` | `tacsim/src/arena/clock.rs:211`; `arena_task.rs:2106`–`:2119` | follows the task's base |
| `TestWorld` suites (all of tacsim's world-side tests, and mush-server's `commerce_composition`) | fake: `set_now` + `fire_due` | `mush-testkit/src/lib.rs:387`–`:393`; `support/field/driving_rig.rs:470`, `:727` | already fake, by fire count |
| The field rigs' wait loop | a `std::time::Instant` deadline, 25-ms tokio sleeps, and the world stepped one fake second every eighth beat | `support/field/field_host_rig.rs:320`–`:342`, `field_raiders_rig.rs:199`–`:220`, and five copies in modules (`crew_exit`, `field_ap1`, `field_carrier`, `field_crash`, `field_fr5`) | no: multi-thread runtime and a std deadline. TS.10a: `field_host_rig::Pace` |
| mushd, gateway and server process stories | the OS, across a process boundary | `mushd/tests/durable_process/*`, `mush-gateway` | never (§3) |

The table makes two consequences plain.

- **The field rigs run two clocks at two rates.** The world advances one fake second per eight 25-ms beats,
  which is 200 ms of real time, while the arena it hosts runs at real time. A field test's world is therefore
  five times faster than its arena. Any assertion that pairs a world-second count with an arena window is
  pairing two different clocks. Under the seam they become one clock, and the tests that leaned on the ratio
  will show up in TS.10a's run.
  - *Draft 0.2:* the ratio lives in the beat count, not in the deadline, so a paused runtime alone does not
    remove it. Eight 25-ms beats are 200 ms of virtual time as well as of real time. TS.10a's `Pace`
    therefore gives the world one second per second of virtual time on a paused clock. On a running clock it
    keeps the eighth-beat rule, which the dispatch stories still need.
- **The world task's `std::time::Instant` uses are not all equal.**
  - Some are measurement: `iteration_start` (`:3865`) and the pulse costs. These stay `std`, so latency
    metrics stay true wall time even in a paused test.
  - Some gate behaviour: the defer queue's `DeferInstant`, the retry deadline (`:2943`, already tokio) and the
    shutdown grace (`:5373`, `:5427`, `std`). Every gating use moves to the provider's instant.
  - *Draft 0.2:* `DeferInstant` is already `tokio::time::Instant` (`world_task/defer_queue.rs:314`), so the
    defer queue pauses today. Of the gating reads, only the shutdown grace is `std`.

## 2. The provider

```rust
// mush-config (beside unix_now_s/unix_now_ms, which every crate already imports), or mush-core: Q-43c-1.
#[derive(Clone, Debug, Default)]
pub enum TimeSource {
    /// Production: unix time is the OS clock, sleeps are tokio's (real time).
    #[default]
    System,
    /// Tests: unix time is derived from tokio's clock, so `start_paused` moves both together.
    Tokio { epoch_unix_ms: i64, base: tokio::time::Instant },
}
impl TimeSource {
    pub fn tokio_at(epoch_unix_ms: i64) -> Self;        // base = tokio::time::Instant::now()
    pub fn unix_ms(&self) -> i64;                       // System: unix_now_ms(); Tokio: epoch + elapsed
    pub fn unix_s(&self) -> i64;
    pub fn instant(&self) -> tokio::time::Instant;      // both arms: tokio::time::Instant::now()
    pub async fn sleep_until_unix_ms(&self, due: i64);  // System: today's code; Tokio: sleep_until(base + (due − epoch))
}
```

The rules:

- **One handle per process, made at composition.**
  - `ServerDeps` (`world_task.rs:1173`) carries it into the `spawn_persist_with_gateway*` family (`:1425`–`:1692`).
  - `mushd`'s `inprocess.rs` and mush-server's `boot.rs` build `System`; a rig builds `Tokio`.
  - The arena receives a clone in `spawn_arena`, `spawn_field` and `resume_arena` (`arena_task.rs:819`,
    `:859`, `:953`; `resume_arena_recorded` at `:970`), through the bridge. The bridge already threads config
    into every spawn, and it takes the runtime with `Handle::try_current` (`arena_bridge.rs:2062`, `:2242`,
    `:2380`).
- **No global.**
  - A `static` or thread-local provider would let two rigs in one process see each other's clock. nextest runs
    one test per process, but `cargo test` does not.
  - A global would also hide which call sites were converted. A parameter is greppable.
  - A contract lint (TS.10e) then bans `unix_now_s`, `unix_now_ms` and `SystemTime::now` in `world_task*` and
    `arena_task*` outside the provider's own module, with the measuring sites allow-listed.
- **The arena's arithmetic does not change.**
  - It keeps `base: tokio::time::Instant` and `elapsed_ms(base)`.
  - The handle is there for the readings that leave the task: the tape's `Restart`, a snapshot's wall stamp
    and the bulk executor's worker day. A resumed or replayed arena then names the same epoch as the world that
    resumed it.
  - `ReplayClock` (`arena/clock.rs:260`) is untouched: replay restores the recorded wall state and reads no
    provider.
- **Production behaves exactly as it does today.**
  - `System::unix_ms` is `unix_now_ms()`.
  - `System::sleep_until_unix_ms` is today's `sleep_until_unix_ms`, NTP caveat included (`world_task.rs:6110`).
  - No production config key selects `Tokio`. Only a constructor that tests call can reach it, and the §5
    guard enforces that too.

Why not inject a trait object (`Arc<dyn Clock>`)?

- The two arms, production's wall clock and tokio's virtual one, are the whole space.
- A trait would invite a third, hand-advanced clock whose sleeps tokio knows nothing about. `select!` over
  `sleep_until` would then need its own timer wheel.
- `start_paused` is tokio's own fast-forward, and it already drives the arena tests.
- It is the only design under which every `tokio::time::sleep` in a task moves together without each one being
  found by hand. doc 43 counts 251 of them in the tests; mush-server's source has 86 tokio sleep, interval and
  `sleep_until` sites. At Draft 0.2 the tests of tacsim, mush-server and mushd hold 220 `tokio::time` sleep
  and interval lines, and mush-server's source 91 sites.

## 3. Runtime constraints (what a paused test can and cannot contain)

1. **`start_paused` needs `flavor = "current_thread"`.**
   - tokio refuses it on a multi-thread runtime, so the field rigs' `worker_threads = 2` must go.
   - The arena tests show that current-thread works for the task itself.
   - The open question for the field rigs is whether anything in the rig **blocks the runtime thread**. A
     synchronous `TestWorld` call is fine; a `std::thread::sleep` or a `block_on` is not.
   - `crates/tacsim/tests/support/field/` has neither today (grep at `7a535806`, and again at `1e9b4117`).
     TS.10a confirms it by running.
2. **Auto-advance happens when the runtime is idle.**
   - **`spawn_blocking` work holds the clock, which is correct.** tokio inhibits auto-advance while a blocking
     task runs, and the arena's mind waits rely on that (`arena_task.rs:2131`).
   - **A bare `std::thread::spawn` that the test then awaits through a channel does not hold the clock.**
     Virtual time jumps to the next timer while that thread works.
   - The arena's own unit tests contain one such thread (`arena_task.rs:5255`, a gate opener that sleeps
     150 ms of real time).
   - Any rig helper that hands work to a std thread must route it through `spawn_blocking`. Otherwise it will
     see its timeouts fire "instantly".
3. **Real I/O that the runtime does not own breaks the model.**
   - A paused test awaiting a Redis reply, a child process, or a socket served by another runtime sees
     `timeout(STEP_TIMEOUT, …)` fire at once: the runtime is idle, and the next timer is the timeout.
   - Loopback sockets served **in the same runtime** usually work, because the I/O driver is polled before the
     clock advances. That is not guaranteed, though: tokio documents paused time as unsuitable for real I/O.
   - So the seam's scope is:
     - **In scope:** `TestWorld` field rigs; mush-server's world-task tests on `InMemoryStore`; mushd's
       in-process composition (`inprocess.rs`) with the memory store and no network client; the arena.
     - **Out of scope, wall time by design:**
       - every `REDIS_URL` story;
       - the `durable_process` binaries (process boundaries);
       - the gateway generations (doc 33);
       - the bot's SSH stories (`mush-bot/tests/*`, a separate process over a real socket);
       - `paid_purchase`'s real-time Registry, which is real time by decision (docs/testing-rate.md §2);
       - `web_e2e`, with its real reqwest and tungstenite clients. It is possible in principle (same runtime),
         but not worth the flake risk.
4. **Determinism is not the goal; speed is.**
   - A paused current-thread runtime is more deterministic than today's two workers.
   - The minds' real worker threads still finish in real time, though. Arena outcomes that depend on which pulse
     a think lands in are as nondeterministic as they are today.
   - `spawn_blocking`'s inhibition only guarantees that virtual time does not run ahead of them.

## 4. What each suite gets

| Suite | Today's floor | Under the seam |
|---|---|---|
| The field rigs (`domain_field`: field_gc4/gc5, `escort_lease`, `field_carrier`, `field_director`, `field_story`, the raiders, and the AP, MW, FR, CR and SU modules since) | the arena window in real time (`disengage_s` is the rig's `quiet_s()` by default, 20 since doc 49 CC.2; 10 in the raiders' rig and `field_director`; 90 in the gate journeys; 600 to hold a field open), plus the 25-ms beat loop | CPU only: pulses at the tuned tempo. The 90-s journeys keep their tempo **and** lose their wall time, so TS.16's "one representative journey keeps 90" costs nothing more |
| mush-server `world_task` tests on the memory store | the real tick cadences and 50-ms polls | the ticks fire as fast as the loop turns, and the polls become virtual |
| mushd in-process (`world_journeys` minus `web_e2e`) | the same ticks, through `inprocess.rs` | the same |
| `TestWorld` suites (most of tacsim, `commerce_composition`) | fire count (already fake) | nothing. Their cost is work, which TS.3 (opt-level) and TS.3b's audit address |
| Process, Redis, SSH and bot stories | wall time | unchanged (§3.3) |

`PLAY_RATE` as `[tacsim.time].play_rate` (docs/testing-rate.md §5 item 4) rides in TS.10's row, but it is
independent of the seam: a config field read in place of the readiness constant. It is listed as TS.10d only so
that it lands under the same row, not because it depends on the provider.

## 5. Build plan

| Cut | What | Engine change | Size |
|---|---|---|---|
| **TS.10a the field rig on the paused clock** | `until_with`'s deadline becomes a `tokio::time::Instant`; one field_gc4 journey and one short field test move to `current_thread, start_paused`; the world is stepped by the same virtual second the arena sees, so the 5:1 ratio of §1 goes | **none** | S, measured from `domain_field`'s JUnit (Draft 0.1: shard_1's). If it holds, the field family follows module by module, each module's minutes recorded per rule 2.3 #14 |
| **TS.10b `TimeSource`** | the type (§2); `ServerDeps` and the arena spawn terms carry it; `world_task`'s gating reads (`set_now`, `fire_due`, `sleep_until_unix_ms`, the defer queue, the shutdown grace) go through it; the other mush-server unix readers of §1 that gate behaviour (`staff.rs:349`, `staff_ops.rs:316`) follow; measuring reads stay `std` | mush-server, tacsim (bridge plumbing), mush-config | M |
| **TS.10c paused rigs** | mush-server's memory-store `world_task` tests and mushd's in-process journeys build `TimeSource::tokio_at(epoch)` under `start_paused`; a boot guard refuses `Tokio` with a non-memory store (§3.3) | the guard | M, module by module |
| **TS.10d `play_rate`** | `[tacsim.time].play_rate`, default 24 (docs/testing-rate.md §5.4) | tacsim readiness | S |
| **TS.10e the lint** | a contract test in `mush-contract-tests`, on TS.8's scanner: no `unix_now_*` or `SystemTime::now` in `world_task*`/`arena_task*` outside the provider, with an allow-list for measurement | none | S |

TS.10a comes first, and alone, for three reasons:
- It needs no engine change.
- It answers the one unknown in §3.1: does a field rig block its runtime thread?
- Its delta on `domain_field` tells us whether TS.10b and TS.10c are worth their size before anyone builds them.

## 6. Risks

- **Hidden blocking in the field rigs** (§3.1).
  - If a rig helper blocks, a current-thread runtime deadlocks: the arena never runs while the test thread sits
    in a synchronous wait on the arena's channel.
  - Detection is immediate: a hang, which the 20-min nextest slow-timeout kills.
  - The fix is local: `spawn_blocking` or an `await`.
- **Tests that leaned on the 5:1 world-to-arena ratio** (§1) will fail when the ratio goes to 1:1. They are bugs
  in the tests, not in the seam, but TS.10a will surface them, and each one is investigated (rule 2.3 #7), not
  retuned blind.
- **Virtual time racing a std thread** (§3.2). A test that hands work to a std thread and then awaits a
  `timeout` reads as a flake. The lint in TS.10e cannot see this case; review of each converted module must.
- **Nothing in production changes, but `ServerDeps` grows a field.** Every rig and every in-process composition
  that builds `ServerDeps` gains `TimeSource::System` (or `Default`). That is a wide, mechanical diff, and it
  touches no state-slot-owner rows.

## 7. Questions (the owner's)

- **Q-43c-1 Where the type lives.**
  - Default: `mush-config`, beside `unix_now_s`/`unix_now_ms`, which every crate already imports. `tokio`
    becomes a dependency of `mush-config`; it is already a workspace dependency. *Draft 0.2: `mush-config`
    already depends on `tokio` (`crates/mush-config/Cargo.toml:24`).*
  - Alternative: `mush-core`. It has no tokio dependency today and should keep none, which argues against it.
- **Q-43c-2 The scope of TS.10c.**
  - Default: in-process memory-store rigs only, with the guard refusing `Tokio` with Redis.
  - Alternative: also convert `web_e2e`'s same-runtime loopback clients, accepting tokio's documented caveat
    about real I/O.
- **Q-43c-3 The order against TS.12.** TS.12 partitions tacsim's tests into domain binaries, and TS.10a edits
  shard_1's field modules. *Moot since 2026-11-05: TS.12 was adopted (D-43-5), and the field modules are
  `domain_field`'s.*
  - Default: TS.10a goes after TS.12's measurement branch has been read. That branch is a measurement, never
    merged, so neither re-pins the other's files.
  - Alternative: TS.10a first, since it is S and engine-free.

## 8. Draft 0.2's corrections (main `1e9b4117`, 2026-11-18)

Each was made in place above; this list is what moved since `7a535806`.

- **The layout.** `tests/shard_N/` is gone (TS.12, D-43-5). The field modules are under
  `crates/tacsim/tests/domain_field/`, the field rigs under `tests/support/field/`, and the arena's paused
  tests under `tests/domain_arena/`. The CI cells run a plan by measured time, not a shard (PR #670,
  `.github/tacsim-integration-shards.txt`).
- **The counts.**
  - The arena's paused tests: 22 → 53 (cockpit's 30 and one of match_lifecycle's have joined them).
  - The field tests on `multi_thread, worker_threads = 2`: 49 → 189 in `domain_field`, in 76 files across
    tacsim's tests.
  - The test sleeps: 220 `tokio::time` lines in tacsim, mush-server and mushd tests.
  - mush-server's tokio sleep and interval sites: 86 → 91.
- **The world task.** Most anchors moved by 15 to 25 lines. `set_now` is at `:3893`, `fire_due` at `:4689`,
  `sleep_until_unix_ms` at `:6114`, and the shutdown grace at `:5373` and `:5427`. The defer queue's
  `DeferInstant` is `tokio::time::Instant`, so it was already pausable: Draft 0.1 counted it among the gating
  reads still to move.
- **The arena task.** The loop's base is at `:1896`, `sleep_until(due)` at `:2052`, `deadline`/`elapsed_ms` at
  `:3031`–`:3046`, the think wait at `:2438`–`:2454` and the std gate-opener thread at `:5255`. The spawn
  functions live in `arena_task.rs` (`:819`, `:859`, `:953`, `:970`), and the bridge calls them through
  `Handle::try_current`.
- **Unchanged.** `ServerDeps` at `:1173`, `world.set_now(unix_now_s())` at `:1377`, `mush-config`'s
  `unix_now_s`/`unix_now_ms` at `:64`–`:76`, the minds bulk executor's four `SystemTime` reads, mush-server's
  other unix readers (`staff.rs:349`, `staff_ops.rs:316`, `archive.rs:369`, `quarantine.rs:3157`,
  `config_ext.rs`'s four, `modules.rs:1634`, `process_fault.rs:216`), and `ReplayClock` at `clock.rs:260`.
- **The rigs' window.** The field rigs' default `disengage_s` is `quiet_s()` (the slowest fielded cycle plus
  five seconds, 20 since doc 49 CC.2), no longer 15.
- **The 5:1 ratio.** It is in the beat count, so it survives a paused runtime unless the rig steps the world
  by virtual seconds (§1). TS.10a's `Pace` does that.
- **Q-43c-1** takes no new dependency: `mush-config` already depends on `tokio`. **Q-43c-3** is moot.
