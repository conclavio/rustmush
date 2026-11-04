# 43. The testing strategy — as practised, as reviewed, and what changes at two million lines

Draft 0.4, 2026-11-03, docs only. **Draft 0.4 adds §10, the fit with doc 38 §9's programme map: six
amendments (TS.9's job, a new story's shard and TS.16's order, the tier rule, the lane fixtures, a per-cut wall
budget as rule 2.3 #14, TS.21 before FR.5), none changing a ruling.** **Draft 0.3 records the owner's rulings of
2026-11-03 on every question in §8: Q-43-6 amended (the timing baseline is re-blessed on conclav.dev, not
conclav.io), every other default accepted as written; the cuts in §7 are binding, and TS.21 is added for the
re-bless.** Written against `origin/main` at `c624cff1` (doc 42 merged) and main's run
41683232696 at `f146734e` (the JUnit reports of its eighteen test cells, downloaded and summed). Two outside
reviews of the testing strategy arrived the same day, both answering the owner's one question — where do we
undertest, where do we overtest, and what structural changes should the growth to 2M lines bring. Both are
stored verbatim beside this document, the doc 28 precedent: [doc 43a](43a-gemini-4-argon-review.md) (Gemini 4
Argon) and [doc 43b](43b-chatgpt-astra-6-review.md) (ChatGPT Astra 6). Six read-only lanes verified every
claim of both against the tree; §3 and §4 are the triages. §2 is the strategy as it has been practised,
written down for the first time in one place. §5 is what the lanes and this document's own reading found that
neither review names. §6 is the verdict, §7 the cuts (binding since the rulings), §8 the questions with
their rulings, §8.1 the consequences and §9 the decisions. Draft 0.1, earlier the same day, folded the first
review alone; Draft 0.2 the second.

A claim is CONFIRMED only against a line of the tree or a figure in the run; PARTLY where a real finding sits
inside an overstated mechanism; BY-DESIGN where a ruling or a comment in the file chose the behaviour, cited;
WRONG only against the line that defeats it. Line numbers are the tree's at `c624cff1`; main was merged in at `5d393bf9` (SF.R16 and four doc drafts) after the rulings, and nothing cited moved except as §3.1 2B notes.

## 0. In brief

- **The tree.** 52 crates, 2,026,368 lines of Rust in 2,549 files (the `tests/` directories 614,045 of them),
  12,624 `#[test]`/`#[tokio::test]` attributes, about 50 real `#[ignore]`s. Main's run 41683232696 ran
  **12,549 tests, all green, in 19 min 45 s of wall and 341 runner-minutes across 29 jobs; their summed test
  time was 615.7 minutes.** On 2026-10-26 the same sum was 389.5 (11,039 tests, run 40371568756); the Rust
  source grew 19 percent by bytes in between. **Test time is growing three times as fast as the tree.** It
  grows where both reviewers say: the story suites (tacsim's ten shard binaries 427 summed minutes, knowing
  alone 54; mush-server 36 + 20; mushd 32; the bot's three long stories 24), not the unit tests (rest-libs runs
  5,767 tests in fifteen summed minutes). **328 tests over 30 s carry 56 percent of the time; 15 tests over
  180 s carry 11 percent.**
- **The strategy, in one line.** Fast unit tests in every crate; integration stories through the production
  command paths in a few group binaries per crate; two process shapes proved by twin lenses; a deterministic
  harness with a blessed baseline; an external bot as the acceptance driver; a crate of tripwires that reads the
  tree and the workflow and refuses drift; nextest everywhere; the pull-request matrix sized to a twenty-minute
  wall, the long stories on the nightly and on dispatch, the owner's boxes for the whole sweep.
- **The reviews, scored.** The first (43a) makes 26 distinct claims: 8 confirmed as written, 11 partly, 4 by
  design or already ruled, 3 wrong — an `android/` client that exists on none of this repository's 507 refs, a
  "triplicated topology" that is the designed twin pattern, a doorway it calls internet-facing that sits
  behind a Unix socket. The second (43b) makes 14: 12 confirmed, 2 partly, none wrong; it read the live run
  and its reports, and every figure it quotes (12,549 tests, 341 runner-minutes, the four duplicate jobs at 36
  minutes, 328 tests at 56 percent, two regressions at 211 s) reproduces exactly. Where they agree — the cost
  profile, the duplicate jobs, the dispatch tier and the nightly nobody watches, the fiat and source-spelling
  tests, the missing browser — the point is settled. The lanes found eight things neither names (§5).
- **What changes (§7, binding since the rulings of 2026-11-03).** Twenty-two cuts, most small. The cheap ones first: a hygiene pass over the stale text
  the lanes found; three CI jobs folded and superseded pull-request runs cancelled; the two regressions that
  wait 90 real seconds twice for a timer their bodies never use, retuned to the rig's default; the gated-test
  inventory given tiers and configuration. Then the structural ones: hostile-input suites where untrusted bytes
  reach code that can take the world down; a seeded, shrinking generator over the economy's lifecycle and the
  fault matrix's sequences; a per-file budget for state written by fiat in tests; a contract that every string
  a client emits is a verb the server registers; a release-runtime smoke after the nightly's release build; a
  real-browser smoke for the web client. Two experiments before any decision: optimised dependencies, and
  hash-partitioned domain binaries in place of resharding by hand. The arena clock seam is the one large item,
  and a design note comes before it. The timing baseline is re-blessed on conclav.dev (D-43-6, TS.21), the
  one amendment the owner made.

## 1. Why this document exists

The testing strategy has been built cut by cut — T.0 to T.3 in the last week of October, the nextest config,
the VPS gate, the group binaries, three reshards, the pull-request wall set, the overheads cut, the ten-shard
split — and written down where each cut landed: the headers of `ci.yml`, `.config/nextest.toml`,
`.github/tacsim-integration-shards.txt`, the shard roots, docs/ops.md's gate and group-binary sections,
docs/testing-rate.md, the 16b load-sensitive inventory, and the owner's working notes. Nothing states the whole.
At two million lines, with two outside reviews on the table and the next phase's programmes (docs 39–42) about
to add stories at two field sizes each, this document states it: what the layers are and why, what they cost,
what the reviews say, what the lanes found, and what to change. It is a survey in the form doc 38 used: the
owner questions in §8 each carry a default and an alternative.

It is not a bug list. The defects the lanes found in passing (stale comments, an undeclared-module hole, a stale
lint path, a quiet skip in a deploy script) are cut TS.0's; nothing else here re-triages what doc 36a, doc 37
§5.1 or the fix thread hold.

## 2. The strategy as practised

### 2.1 The layers

| Layer | Where | What it proves | Size and cost (run 41683232696 unless dated) |
|---|---|---|---|
| **Unit tests** in `src/` | every crate; 1,760 in tacsim's lib alone, 818 in mush-core, 339 in mush-roll | one function or one module, with no world | rest-libs: 5,767 tests in 15 summed min; tacsim-lib 37 summed min (one bulk-scenarios test 194 s of it) |
| **Integration group binaries** | tacsim `shard_1..10` + `harness` + `cost_lane`; mush-server `commerce_composition`/`split_boundary`/`world_task`; mushd `durable_process`/`sim_lenses`/`world_journeys`; mush-gateway `split_halves` (T.3, 2026-10-26/27) | a story through the production composition (`compose_server_modules`, `TestWorld`), by the house rule through the production command paths (doc 36 §7; the allowed seeds per story in doc 36a §8) | tacsim shards 427 summed min over 2,118 tests (shard_4 61, shard_1 58, shard_2 11 + cost_lane 4 alone); mush-server 36 + 20; mushd 32 (sim_lenses 29); gateway 5. Every binary links the whole engine (560–740 MB), which is why they are grouped |
| **Twin lenses** across the two process shapes | mushd `sim_lenses` (observer_rule 9,501 lines, 32 tests) and `mush-gateway/tests/split_halves` (`arena_custody_split`, `arena_story_split`, `front_door_split`, `tui_split`) | the same story drives the all-in-one binary and the gateway+server pair; the assertions are divided between the twins (the store claims stay with mushd, the output path is the split's: doc 16 §1, `arena_story_split.rs:5-8`) | each lens boots the production stack over real SSH and web sockets |
| **The deterministic harness** | `crates/tacsim/tests/harness.rs` (13,354 lines, 33 tests) + `harness_baseline/arena.<os>.json` | the deterministic block (world, substeps, counters, totals, peaks, end states) is bit-identical to the blessed baseline; the timing gate (20 ms wake, 40 ms engaged p99) is live only on the host that blessed it | nightly `tacsim-harness` in release (~4 min build + 3.5 min lanes); `scripts/harness-weekly.sh` by hand; re-blessed by a record, never blind (`ade71092`, three records, 24 fields, after the 2026-11-02 bisect) |
| **The long stories** | M7 `mind-commerce-acceptance` (6 cells), M8 `m8-gate-stories` (3 cells), the bot's `mining_cohort` (three stories: 590, 504 and 336 s), `arena_mind_isolation`, `field_scale`, the leak battery, the house doctrines, the M6 author/replay acceptance | a whole programme's acceptance, `#[ignore]`d or long, run by name | nightly at production pace (`MIND_PULSE_MS=1000`, 3 h) or dispatched fast (50 ms, ~29 min); six jobs dispatch-only; debug builds, `-j 2`, 360-minute timeout |
| **The external bot** | `mush-bot` (71k src lines, 740 tests): an SSH line-mode player with goals, budgets, recall, regression gates (`docs/bot/regression.md`), adversarial playtests (`docs/bot/playtesting.md`) | the game as a player meets it, from outside the process; the mining canary (nine minutes of game clock at rate 24, 168 after enrolment since 2026-10-27) | two matrix cells (`bot` 12 summed min, `bot-stories` 26); `bot-win-rate` and `bot-muster-rate` dispatch-only (75–120 min) |
| **Live acceptance scripts** | `scripts/relay-r*-{live.mjs,ssh.py,fresh.py,world-fixture.py}` (44 files), `scripts/client-*-live.*` (4), their evidence under `docs/design/evidence/` | one relay or client cut proved against dev1 on the day, with the transcript kept | by hand, once; the evidence is the record |
| **The tripwires** | `mush-contract-tests`: 33 binaries, 13,294 lines; the registries `state-slot-owners.toml` (138 rows), `panic_budget/<crate>.txt` (35 files), `MEASURED` in `restriction_lints.rs` (51 crates × 6 lints) | the compiled production registry (exit words, macro names, help, projections, RNG routing, serialization policy) and the source tree (panic surface, lint floors, slot ownership, feature census, gated-test reachability, shard completeness, matcher call sites, grid vocabulary, toolchain pin) | seconds each; the four guards run before every push (feature census, panic budget, restriction lints, state-slot owners) |
| **The web client** | `web/`: 80 vitest files, ~876 cases, 9 mounted-component tests (jsdom), 71 lib-module tests, 4 files of source probes; svelte-check over every component in the build | the TypeScript controllers and stores (only 3 hand-written lib modules have no importing test); component logic lives in tested lib modules | the `web` job, 1.4 min; no browser, `environment: "node"` with 11 files opting into jsdom |
| **Shell** | `scripts/tests/{reset-instance,gateway-swap,check-web-bundle}.sh` against stub `systemctl`; `bash -n` over nine deploy scripts; `shellcheck` over one | the swap and reset scripts' control flow; the three instance files render and pass `--check-config` on debug binaries | the `provisioning` job, 9 min |
| **Static analysis** | CodeQL (actions, JS/TS, Python on every push; Rust weekly, 50 min, 425 noise alerts excluded) | the usual | `codeql.yml` |

### 2.2 The runners and their tiers

| Tier | Trigger | What runs | Wall |
|---|---|---|---|
| **Pull request / push to main** | `ci.yml` on every non-docs push; no cancellation of a superseded run | the `test` matrix of 20 cells (tacsim-lib, tacsim-integration-1..10, tacsim-minds, lint, rest-libs, rest-mushd, rest-server, rest-server-boundary, rest-gateway, bot, bot-stories) under `cargo nextest run -P ci` with `cargo test --doc` beside each sweep; plus features, documentation, provisioning, kernel-contracts, redis-faults, definition-wire-fixtures, direct-rand-policy, truth-serialization-policy, web; 21 jobs, 29 with the matrix | 19 min 45 s on run 41683232696 (the cells 9–20 min, shard_4 the wall); 341 runner-minutes; the hosted machines vary ~2× run to run |
| **Nightly** (04:17 UTC) | the schedule | everything above plus the release-profile build check, `scale`, `tacsim-harness`, M7 at production pace, M8 | hours; **nobody watches it** (`ci.yml:59` says so in its own words); red on 10-28, 29, 30, 31, 11-01, 02 and 03, green last on 10-27 |
| **Dispatch** | `gh workflow run CI` (one input, `mind_pulse_ms`) | everything, including the six dispatch-only jobs | hours |
| **The VPS gate** | `scripts/vps-gate.sh [--wip] [--packages …]` → `gate.yml`, `workflow_dispatch` only, never on a PR (a fork's PR must never reach the owner's boxes); inputs kind, runner, packages, extra, redis — no `env` | the whole workspace sweep against a private Redis, the bot in the background; or the harness or the cost lane on conclav.io | conclav.dev warm ~19 min at 32 threads; conclav.io ~81 min |
| **The box by hand** | conclav.dev as `ghrunner` | Linux-only `cfg(unix)` stories (the Windows box cannot run them); clock-bound stories as parallel variants (`variant-build.sh`/`variant-run.sh`); the story checkpoint (`M7_STORY_CHECKPOINT`); the nightly bisect (`nightly-run.sh`, `bisect.sh`) | minutes to an hour |
| **Locally** | the owner's Windows box: `-j 8`, one workspace build at a time, `cargo fmt -p` per crate (`--all` fails on the path length), `debug = "line-tables-only"` (full debuginfo hit the PDB ceiling) | only what the change touched, plus fmt, clippy, strict rustdoc and the four guards; the broad sweep goes to the gate once | the cut loop's rule since 2026-10-26: one gate, sized to ~25 min |

### 2.3 The rules we live by

1. **Test what changed; one broad gate.** Builders and fixers run the tests for what they touched; the gate
   runs the sweep once, on the VPS; the PR's CI is the rest (2026-10-26, after M7 cut C1 spent 70 percent of
   eighteen hours on repeated Windows sweeps).
2. **The four guards before any push**: `feature_matrix_inventory`, `panic_budget`, `restriction_lints`,
   `state_slot_owners` (2026-10-28, after a feature-gated PoC broke main).
3. **A new test is a `mod` line** in the group binary that fits (for tacsim, the shard with the most room);
   `ci_tacsim_shards` refuses a top-level target in no shard. A module moves with its `#[path]` siblings, its
   subdirectory and its state-slot rows; `crate::<mod>` importers pin a module to its binary.
4. **Through the production command paths.** Every cut's story runs the verbs a player runs; a seed is allowed
   when named in the as-built section and the thing claimed is still reached by command (doc 36 §7; the
   allowed-seed table doc 36a §8). The space train's eight direct `SpaceBubbleBook` writes were the breach that
   made this a rule with a lint (doc 35b:142).
5. **Stories grow, at both sizes.** The cut that first makes a step pass adds the step; the story's last cut
   runs it whole at 256 and at 512 (doc 36 §7).
6. **Nextest, one process a test.** The in-binary locks are restated in `.config/nextest.toml` (five binaries
   run alone; every `REDIS_URL` reader in a group of one; the load-sensitive suites two at a time with one
   retry reported FLAKY, never silently); a hung test dies at 20 min; JUnit per cell is the data for the next
   rebalance.
7. **A flake is investigated, not discarded** (16b cut L: "passed alone twice" is the rule for the list, and the
   first failure is preserved). The record bears it out: the `Locked on` and `CONTACTS` races were a stale-row
   class in the harness, the paid_purchase Busy was the first Timer wake after a boot, the M7 story's missing
   line was the engine's busy back-pressure at production pace, and f6 is a real race the field tick has
   (doc 37 §3.1).
8. **Re-bless by a record, never blind.** A harness delta is a merged kernel cut that did not re-bless; the
   baseline edit carries the fields, the previous values and the reason; `TACSIM_BASELINE_BLESS` never off the
   VPS.
9. **Numbers are provisional** (2026-10-24): a default is taken and tuned later; a build is never held on one.
10. **Never redeploy without the owner.** The boxes build release binaries from a checkout; CI's release job
    uploads nothing.
11. **Linux-only tests run on conclav.dev before a push** that renames a log line or a signal path the
    `cfg(unix)` stories await (GG.2 shipped a red cell after a green Windows sweep).
12. **Clock-bound stories iterate as parallel variants with a checkpoint**, never one 85-minute run per
    hypothesis (2026-10-31).
13. **A cut that touches a hash-pinned file dispatches the harness kind before it is marked ready** (5.4;
    TS.0 writes it into docs/ops.md). Doc 40 §4a's inventory names the files; its FR.5 and doc 23's S.4b are
    the first cuts it binds.
14. **Every cut's note records its cell's summed-minutes delta** from its PR run's JUnit, and a cut that
    would set a cell's wall moves its story to the shard with the most room (#3) or lands at the rig's default
    tempo, never a copied window (5.8; §10.5). The deltas are the step audit's (TS.3b) data.

### 2.4 The numbers, dated

| | 2026-10-22 | 2026-10-26 (T.0–T.2) | 2026-10-27 (T.3, six shards) | 2026-10-31 (sixteen cells) | 2026-11-03 (ten shards, run 41683232696) |
|---|---|---|---|---|---|
| PR wall | 57 min (3 jobs) | 28 | 18–23 | 27–34 (rest-server the wall) → 18–21 | 19.75; cells 9–20 (shard_4 19.7, shard_1 19.2) |
| Runner-minutes a run | | ~265 | | | 341 over 29 jobs |
| Test cells / jobs | 3 | 10 | 15 | 16 | 20 / 21 |
| Integration binaries | 373 (110 GB on the VPS) | 373 | 11 (tacsim 6 + 2; the servers 7): 8 GB | | tacsim 10 + 2 |
| Tests run / summed minutes | | 11,039 / 389.5 | tacsim 274.6; shards 45–49 each | tacsim shards 34–66; mush-server 105 | 12,549 / 615.7; tacsim shards 427 (knowing 54, claims 41, match_lifecycle 26, cockpit 26, mining 23, drones 21, plans 18, unloading 17, field_gc4 15 over six tests) |
| Tests over 30 s | | 13 binaries carried 219 of 389 min | | | 328 of 12,549 carry 345 min (56 %); over 60 s: 84 / 27 %; over 180 s: 15 / 11 % |
| Tree (.rs bytes) | | 66.2 MB | | | 78.8 MB (+19 %) |

The reshard cadence is the cost to watch: four shards on 10-26, six on 10-27, the shard-2 rebalance the same
day, the knowing/match_lifecycle swap on 10-31, ten shards with the field-rig hoist on 11-03 — five layout
changes in eight days, each a `git mv` plus `mod` lines plus run lines plus state-slot rows, each judged on
several runs because one hosted machine is twice as fast as another. The other cost nobody measured: of the
last 100 pull-request runs (2026-11-02/03), **18 were overlapped by a newer push to the same branch and ran to
the end anyway**, about 260 wall-minutes of superseded runs across up to 29 jobs each, because `ci.yml` has no
`concurrency` block (`codeql.yml` has one with `cancel-in-progress: true`).

## 3. The first review (Gemini 4 Argon, doc 43a), triaged

### 3.1 Score

| § | Claim | Verdict | One line |
|---|---|---|---|
| 1 | ~1.87M lines, 52 crates, 10,600+ tests | PARTLY | 2.03M lines, 12,624 tests at `c624cff1`; stale by 8–16 %, the shape right |
| 1 | "the pyramid is inverted; integration minutes scale faster than the code" | **CONFIRMED** (by time, not by count) | +58 % test minutes against +19 % source in eight days (§0); by count the base is wide (mush-core 818 unit vs 240 integration; rest-libs 5,767 in 15 summed min) |
| 1 | tacsim ~410 summed min, mush-server ~105 | PARTLY, now low | ~410 was the ten-shard *target* (41 × 10, cost_lane weighted fourfold); the measured figure on 11-03 is 427 over the ten shard binaries; 105 includes the lib's 14 |
| 2A | proptest in 7 files of 6 crates | **CONFIRMED** | the list is exact |
| 2A.1 | mush-ipc: no truncated-frame / hostile-length test | **WRONG** | `tests/roundtrip.rs:202/231/240` cut every prefix, sub-prefix and an oversized length; `MAX_FRAME_LEN` 16 MiB checked before allocation (`codec.rs:170`). Gaps: a corrupt body after a valid prefix; `FrameReader`'s mid-frame EOF (`io.rs:319-334`). The peer is the sibling process |
| 2A.2 | sshdoorway: internet-facing, server.rs untested | PARTLY, framing wrong | the proto runs doorway→instance over a Unix socket (`proto/lib.rs:3-5`); its 8 tests feed `u32::MAX` lengths, EOF mid-frame, five garbage bodies, a version mismatch; `tests/e2e.rs` drives server.rs with a real russh client through 33 tests (stalled kex, over-cap slots, keystroke storms, dial caps); the crate denies every panic-shaped lint. Only generative input at the username/menu level is missing |
| 2A.3 | mush-mind: no generative test of memory/budget | PARTLY | memory is integer-only (a float is a type fault, tested); 18 malformed decodes refused; every cap tested. Missing: any test that feeds deeply nested source to either Rhai engine (the depth guard is only checked as a config value, `engine.rs:198`) — a native stack overflow aborts the process and `catch_unwind` cannot catch it |
| 2A.4 | pixel/content decoders: no hostile-byte tests | mostly **WRONG** | `.mupk` decode is cut at every byte 0..400 and refuses a wrong version (`bake.rs:1220-1253`); packs are operator-baked and never player-supplied; sixel and kitty are encode-only; the content pack is strict JSON with a 1 MiB cap from shipped or operator paths. The one untrusted decoder is `caps.rs` (terminal DA1 and cell-size replies, 4 example tests) |
| 2A.5 | mush-economy: 791 tests, no stateful proptest | PARTLY | 791 exact; one hand-rolled generative test exists (`e2a.rs:503`: 2,000 steps, two-line transfers of one instrument, zero-sum after each, one fixed seed, no shrinking); conservation is enforced at validate (`engine.rs:785`) and by a boot tripwire that freezes the economy (tested). Holds, liens, reservations, title and custody (invariants 10–26) and checkpoint-vs-replay identity are example-only. **Real** |
| 2B | acceptance by fiat; "hundreds of exemptions" | PARTLY | the breach was eight `SpaceBubbleBook` writes in sfr1–8, ruled and closed (0 today; the lint is `shard_10/voyage.rs`'s self-check, its doc path stale; the `arena_library` test the review cites landed on main after this document's base, with SF.R16 as PR #353 at `f1b2a1e4`, and forbids one spelling of one slot). 81 real `module_state_mut` sites remain in tacsim's tests (AccountOracle 32, ArenaRegistry 16, MatchBook 9), 210 across every crate's tests. `state-slot-owners.toml` is a cross-crate ownership register of 138 rows (78 under `tests/`), not an exemption list, and same-crate writes are outside it, which is the hole. `TestWorld::run` already drives verbs (`mush-testkit/src/lib.rs:257`); the 35b `DOCK` dead end is the strongest example in the review |
| 2C.1 | `android/`: zero CI, rotted test | **WRONG** | no `android/` at HEAD or on any of the repository's 507 refs; no Kotlin or gradle file was ever committed; the review's links point at the `antigravity` checkout. If a prototype exists there untracked, it is outside the repository (Q-43-1) |
| 2C.2 | web: 44 of 53 components untested; four stores untested; no browser | PARTLY | the count is exact (48 of 57 with widgets) but the studios' logic lives in tested lib modules (`corridor-editor`, `npc-editor`, `quest-editor`, `catalog-editor`, `content/store`), several components mount inside tested parents, and svelte-check types every one in the build; the four named modules are all imported by tests; of 138 lib modules only `rhai-language.ts`, `storage.ts` and `stores/chart.svelte.ts` have none. **No browser: confirmed** (`environment: "node"`, 11 files opt into jsdom, no Playwright) |
| 2D | nightly red, unwatched; six dispatch-only jobs; inventory accepts a dispatch-only job | **CONFIRMED**, and the inventory is weaker than claimed | the six are exactly those (`ci.yml:157/188/206/224/1135/1171`); no workflow has a failure step and the token is `contents: read`; the inventory counts a test covered if its name appears **anywhere** in `ci.yml`, comments included (`ci_gated_test_inventory.rs:597`): five `#[ignore]`d tests are covered by comments that say no job runs them, deliberately (`:1149-1150`). Doc 37 §3.1 and §7.10 already list it |
| 2D | release-profile check off PRs and pushes | BY-DESIGN, risk real | `ci.yml:803-817`: 27 min that uploaded nothing and set main's wall; "a release-profile or LTO break now surfaces within a day"; the boxes build from a checkout, never from CI |
| 2E | shellcheck on one script; three deploy scripts in no list | **CONFIRMED** | `ci.yml:967-973` states the rule: "a gate is added with its findings fixed, not with a backlog attached"; `deploy/sshdoorway/{swap,check-logging}.sh`, `deploy/vps-runner/provision.sh`, `scripts/vps-gate.sh`, `scripts/unit-bake/bake.sh` are named nowhere; `scripts/tests/` holds three shell tests the review missed |
| 2E | 40+ Python/.mjs scripts with no unit tests | misread | 44 of the 58 are the relay cuts' one-shot live acceptance drivers (they *are* tests, with their evidence kept); 4 are client live scripts; the rest are operator bake and extract tools whose product is committed and whose reproducibility from a committed script is the 2026-10-29 ruling |
| 3A | five standalone jobs duplicate the matrix | PARTLY: 3 of 5 | `direct-rand-policy`, `truth-serialization-policy`, `definition-wire-fixtures` are pure re-runs (the last one's TS-drift step byte-identical to rest-libs'); `kernel-contracts` is a re-run but for one unique step (the mock LLM cost run, `TACSIM_LLM_COST_RUN=1`, which otherwise returns early in shard 1) and the inventory's pins on its literal lines; `provisioning` is not a duplicate at all (`render.sh` over three instance files, `--check-config` of three binaries, three negative controls, the shell tests). All five recompile mush-server and tacsim; none is a required check (main has no branch protection). The four together were 35.8 runner-minutes on run 41683232696 (14.4 + 12.1 + 5.0 + 4.3) |
| 3A | redis-faults re-runs non-Redis tests | PARTLY | only `persistence::` (24 of its 33 tests are not gated); the module filter is deliberate so a new `*_redis_gated` test is covered without a line; the comment's "seven" is nine |
| 3B | story tests used for narrow edge cases, "100× faster as unit tests" | UNPROVEN as stated; the cost profile is right | the cost sits in a dozen modules (§2.4). Nobody has audited which of their steps are real-time waits, which are world boots, and which are assertions a unit could carry; the second review's lane did it for one family (§5.8) and found the timer. The house rule (2.3 #4) is why they are stories; the audit is TS.3b |
| 3B | M7/M8 in debug builds | **CONFIRMED**, consequence overstated | debug, `-j 2`, 360-min timeout, no comment choosing debug (`ci.yml:122/152`). But the M7 story is **clock-bound** (load ~0.3 on 32 cores; the nightly's 3 h is the owner's production-pace rule, the fast dispatch 29 min; the worker's CPU is ~2 s a rating step), so optimisation buys M7 little; M8 and the tacsim shards (CPU-bound, load 5–6 on four vCPUs) are the candidates |
| 3B | wall-clock sleeps; the 23-entry load-sensitive list | counts **CONFIRMED**; remedy PARTLY applicable | 251 `tokio::time::sleep` in tests (mushd 72, tacsim 102, gateway 31, bot 28, server 18), mostly 200–500 ms polls; 51 `start_paused` tests exist already (tacsim 48). The world task reads `SystemTime` (`world_task.rs:3866/4670/6089`) and the **arena task anchors its clock to `Instant::now()`** (`arena_task.rs:1718/1848`) under multi-thread runtimes, so paused time would spin, not fast-forward: a clock seam is the prerequisite (TS.10). The lenses use real sockets and child processes and stay on real time by nature |
| 3C | triplicated topologies | mostly **WRONG** | the twin pattern is the design (doc 16 §1: the split twin "because the output path differs"); each twin pair divides its assertions; `split_boundary` has no arena, observer, lobby or reconnect lens at all. Four pairs share a story (arena_custody, arena_story, front_door, tui_split). The cost of a lens is real; the duplication of rules is not there |
| 3D | state-slot-owners: path-keyed, hundreds of test exemptions, breaks on every reshard | PARTLY | 67,997 bytes, 138 rows, exact-path key (`state_slot_owners.rs:599`); 34 rows under the tacsim shard dirs; the ten-shard reshard rewrote 21, the swap 6. A move fails **loudly** (a missing row and a stale row, `:688`), never silently. The churn is real; "hundreds" and "exemptions" are not |
| 3D | six hand-rolled lexers, no shared visitor | PARTLY | two real lexers (`panic_budget.rs:157`, `matcher_call_sites.rs:152`) that are drifted copies of each other; four line heuristics; 14 files own a `repo_root`, 19 own a walker; no `syn`. The duplication is wider and shallower than claimed |
| 3D | hex_vocabulary scans every file; the "sixteen-hex" failure | PARTLY | it walks `crates/` and `docs/` only, 28 substrings, six whole files allowed; the incident is real (`bb31b139`: doc 17c's plan was reworded after a grid phrase tripped it, found by M5.1's local gate, not CI) |
| 3D | exact counts of tutorial hints / fn names pinned | PARTLY | nine (file, fn) pairs and the gated census of 14 are pinned in the inventory; the tutorial count (13) is `help_commands.rs:447`, by design (the hints load at boot) |
| 4.1 | `opt-level = 2` for hot dependencies | UNMEASURED | `Cargo.toml:290` measured dependency `opt-level 1` on 2026-10-18 for **link time and disk**, never for test time; the "5–15×" is generic. Worth one measured trial (TS.3); every warm target dir rebuilds its dependencies once |
| 4.2 | fold the five jobs; coalesce the 33 contract binaries | ACCEPT 3 of 5; the merge was planned | T.3c (contract binaries → two groups) was in the T.3 plan and deferred by the owner on 2026-10-27 (TS.11) |
| 4.3 | domain binaries + nextest partitioning | WORTH AN EXPERIMENT | the reshard pain is real (2.4); but a domain binary is not a cell (the field cluster alone is 62 summed minutes), and a cell that hash-partitions a binary must link that binary, so the trade is link minutes per cell against reshards per week (TS.12) |
| 4.4 | paused time everywhere in-process | PARTLY | see 3B; the seam first |
| 4.5 | no new fiat; command-driven builders | ACCEPT in the narrower form | the builder exists; the ratchet is a per-file budget of same-crate test-side `module_state_mut` (TS.6), not a ban |
| 4.6 | proptest on ipc, doorway, mind, pixel, content, economy | ACCEPT 2 of 6, redirected | the lanes' ranking by who supplies the bytes and what a panic costs (§5.1): the command line, the TUI key parser, the Rhai depth guard, the economy's stateful model; mush-ipc's two small gaps; the doorway, pixel and content items are moot |
| 4.7 | android job; web Playwright; schedule and tier the dispatch-only jobs | ACCEPT 2 of 3 | no android; the browser smoke (TS.9); the weekly slot and inventory tiers (TS.2) |

### 3.2 What the review got right and why it matters

- **The cost profile.** Test minutes outrun the tree by 3×. The reviewer's "inverted pyramid" is the right
  shape by time; the reason is not that we test edge cases with stories (unproven) but that every programme's
  acceptance is a story at two sizes through the production paths, by rule, and the rule is correct for a game
  whose failures are wiring failures (§5.2). So the lever is not "write units instead" but: measure what each
  story minute buys (the step audit), stop paying for real-time waits (the field tempo now, the clock seam
  later), and keep the hosted wall by layout tricks that need no reshard.
- **The nightly nobody watches, and the six jobs nobody runs.** Both already on the owner's list (doc 37
  §3.1, §7.10). The inventory's coverage rule being "the name appears somewhere in the file" makes the tripwire
  a formality for anything dispatch-only or by-hand. Tiers fix it (TS.2).
- **shellcheck.** Five scripts in no list at all, three of them the doorway and runner provisioning that run as
  root on the boxes. The rule in the file is right (fix the findings, no backlog), so the cut is "run it, fix
  it, add it" (TS.13).
- **Fiat.** The 35b breach is the sharpest finding in the review and it is already a rule; the gap is that the
  rule has no ratchet for same-crate writes.

### 3.3 What it got wrong and why it matters less

- The `android/` client does not exist here. Everything built on it (a rotted assertion, a mock repository, a
  gradle job) is about files outside the repository.
- The doorway's proto is not the internet-facing side; russh is, and the doorway's end-to-end suite is the most
  adversarial in the tree. The review weighted a line count (server.rs, 0 inline tests) over the 33-test e2e
  file beside it.
- The "triplicated topology" is the twin pattern the arena design requires; the fourth leg does not exist.
- Three of the five "duplicate" jobs are duplicates. Hosted minutes are free on a public repository, so the
  cost of the other two was never money; it is noise in the checks list and a stale comment calling the test
  job "cross-platform".
- "Hundreds of exemptions" is 138 ownership rows of which 78 are test rigs, and a reshard breaks them loudly.

## 4. The second review (ChatGPT Astra 6, doc 43b), triaged

Shorter, and it read the live run, its JUnit reports and the repository's settings rather than the tree alone.
Every number it quotes reproduces from run 41683232696's artifacts and jobs (`gh run view`, the eighteen
`nextest-junit-*` reports): 12,549 passing tests, 19 min 45 s, 341 runner-minutes, the four named jobs at 36,
328 tests over 30 s carrying 56 percent, 211.6 and 211.0 s for the two regressions. The one lane it needed
confirmed all six of its code-level claims.

### 4.1 Score

| Claim | Verdict | One line |
|---|---|---|
| the run's figures (12,549 tests, 18 reports, 19m45s, ~341 runner-minutes, 80 frontend test files) | **CONFIRMED** | exact (§2.4) |
| no real-browser runner exercises the built SPA against a server | **CONFIRMED** | as 43a 2C.2; `vite.config.ts:35-38` documents the per-file jsdom opt-in; no Playwright |
| the accounting generator uses one fixed seed; the fault matrix exercises one failure point per case on a fresh store | **CONFIRMED** | `e2a.rs:514` `seed = 0x9E37_79B9_7F4A_7C15`, no override, no shrinking, one instrument, three unbounded accounts, two-line plans, and its "near i64 bounds" comment is false (amounts stay within ±2^40.6); `gate5_fault_matrix.rs:537-656`: one mode per case on a fresh `InMemoryStore`, healed after one fire, over seven bundles; not covered: two faults in one case, faults on outbox attempts, the Redis backend |
| the release job stops at compilation; provisioning exercises debug binaries | **CONFIRMED** | `ci.yml:833-838` builds the SPA and the three release binaries with tls and runs nothing; provisioning builds debug without tls and runs `--check-config` (`:872/:894`); the only TLS test is `mush-net-web/tests/tls_serve.rs` in the `features` job; no built binary has ever served TLS in CI. And `render.sh:111-113` **skips a binary that is not built with a stderr line**, so a drift between its list and the build line is quiet (TS.0) |
| `FIELD_RAID=director` is a story variant no workflow selects; the close-out records missing evidence | **CONFIRMED** | `mining_cohort.rs:924`; `director` seeds and wears the raider, never launches it, skips the leader assertions; the test runs in `bot-stories` in the staff-raid form only; `gate.yml` cannot pass env. The lane's census found **eleven more env-switched variants no workflow selects** (§5.9) |
| four named jobs repeat proofs in the sweeps, ~36 runner-minutes | **CONFIRMED**, with 43a 3A's nuance | 35.8 measured; three are pure re-runs, `kernel-contracts` carries one unique step and the inventory's pins |
| two regressions each settle two fields on a 90-s disengagement setting, 211 s each; "328 tests over 30 s = 56 %" | **CONFIRMED** | `field_gc4.rs:114/:446`, each calling `field_settles` twice (`:579-596`, up to 20 s to open, 180 s to demote); `lane3_tune` (`:569-576`) sets `disengage_s = 90` "GC.4's tempo", copied from the gate whose body needs the field open; **these two bodies do nothing inside the field.** Arena time is wall time (`arena_task.rs:1718/1848`); the rig moves the world clock by hand (`field_host_rig.rs:247-249`) but cannot speed the window; `stocked_tuned` validates nothing and `field_gc10.rs:200` already runs at 5 s, the rig's default is 15 (`field_host_rig.rs:33`). §5.8 has the family |
| frontend tests regex-match Svelte source; one says no component harness exists although mounted tests do | **CONFIRMED** | `unit.test.ts:935-1002`: seven cases on loop variables, the `{#if}` chain, `onkeydown` and markup; the comment at `:936-937` ("no component harness exists on this client") dates from 2026-09-07, the first jsdom mount from 2026-09-22 (`4e004857`); 4 of 80 files read source as text (about 22 structural cases and 11 vocabulary bans); `game.test.ts:1776-1778` and `roomband.test.ts:72-73` carry the same stale sentence; nine mounted-component files exist |
| the inventory accepts mentions, comments included; make it a manifest of execution | **CONFIRMED** | as 43a 2D; the manifest is TS.2's shape, with configuration added (§5.9) |
| separate domain organisation from scheduling | PROPOSAL, consistent with TS.12 | the same trade (link minutes per cell against reshards); the field-rig hoist it praises is PR #344 |
| consolidate fixture infrastructure; prefer immutable historical fixtures over stripping today's manifests | PARTLY | the strip chains are real and long: eleven `pub fn strip_*` in `tacsim/tests/support` (`sites.rs:76/124/141`) plus `relay_registry.rs:4277` `strip_urvara`, a duplicate; but the lane, relay and Ceres rebuilds **pin the published digest** (`:4338`), which is the point of a fixture. The ones that pin nothing are the gap: `mush-atlas/tests/interior.rs:180-187` `ooc_v3` ("Version 3's bytes are not in the tree any more"), `e2b.rs:1114-1122` (format 3 by deleting `title_lineage`), `shard_1/replay.rs:526-528`, `relay_registry.rs:2796`. The immutable pattern already exists: `mush-ipc/tests/golden/v12/*.frame`, "never edited by hand" |
| main unprotected, no rulesets; no cancellation of superseded runs | **CONFIRMED** | §5.5; `ci.yml` has no `concurrency`, `codeql.yml` and `gate.yml` do; 18 of the last 100 PR runs were overlapped by a newer push (§2.4) |
| seven preceding scheduled runs red; the baseline repair on main; evidence freshness as a maintained output | **CONFIRMED** | the schedule: red 10-28 → 11-03, green last 10-27; `ade71092` is the three-record repair; "manual lanes should record their last successful revision" is the `by-hand:` tag of TS.2 |
| preserve the expensive production-paced stories but give them a deliberate periodic role | AGREED | 2.2's nightly tier is that role; TS.2 writes it down per story |

### 4.2 What it adds beyond the first review

- **It measured.** The 211-second pair, the 56 percent, the 36 runner-minutes and the 18 superseded runs are
  the figures this document's cuts are sized on; the first review estimated.
- **Configuration is a coverage dimension.** A test name "covered" while its variant never runs is the same
  hole as a comment counting as coverage; the lane's census (§5.9) turns one example into twelve.
- **The release runtime.** Nothing in CI has ever started a release binary, served the embedded SPA, or
  handshaked gateway to server over TLS; the boxes do that for the first time on each deploy, under the owner.
  A smoke after the build the nightly already pays for is cheap (TS.17).
- **Spelling versus behaviour.** The source-probe tests were a stopgap for a harness that now exists; three
  comments still say it does not (TS.19).
- **The fault matrix's shape.** One fault per case on a fresh store is a matrix, not a sequence; Astra's
  "submit → retry → lose acknowledgement → restart → cancel → advance time" with conservation, sole custody and
  recovered-equals-live after every step is what TS.5 should generate.
- **Cancellation.** Eighteen superseded runs in two days is the cheapest fix in this document.

What it got less right: the "four named jobs" are three duplicates and one with a unique step (as 43a), and
"prefer immutable fixtures" is already the practice where a digest is pinned; its own ordering ("deduplicate CI
and cancel superseded runs; a browser smoke; the real-time waits; then generated lifecycle tests and the
manifest") is close to §7's.

## 5. What neither review names (the lanes' findings and this reading's)

### 5.1 Where untrusted bytes meet code that can stop the world

The lanes ranked every parser by who supplies its input and what a panic there costs. The panic strategy is
unwind; a panic in the world task is caught by `guard()` (`world_task.rs:1316`) and **the world stops frozen**;
a panic in a per-connection task loses one connection. There is no `fuzz/` directory anywhere.

| Parser | Input from | Runs in | Tests | Generative? |
|---|---|---|---|---|
| mush-core `cmd.rs` (`ParsedArgs`, `dispatch_line`) — every player line | the player | **the world task** | 12 inline + behaviour tests | **no**; the crate still allows `expect` (117 sites) and indexing (94) |
| mush-net-ssh `tui/keys.rs` `KeyParser` — raw PTY bytes, a hand-written state machine with index rewinds (`:203`, `:244-251`) | the player, over the internet | a connection task | 7 | **no** (`tui_line_proptest` uses a fixed key alphabet); the crate allows indexing (28 sites) |
| mush-pixel `caps.rs` (`DeviceAttributes::parse`, `CellSize::parse`) — terminal replies | the player's terminal | a connection task | 4 | no |
| mush-script / mush-mind Rhai engines — expression depth | builders; players' mind source (capped at 32 KiB) | the world task / the minds pool | config value only | **no test feeds nested source**; a stack overflow is an abort |
| mush-economy's op set (holds, liens, reservations, title, custody; checkpoint vs replay) | every verb | the world task; corruption persists in the store and is caught at the next boot by the freeze | examples; one two-line generative test with a fixed seed | **narrow** |
| mush-net-web `ws.rs` `ClientCommand` | the browser | a connection task | 44 + `decode_proptest` | yes (oversize, binary, malformed); `client_command_strategy` omits `Query::NarrativeBacklog` and later variants (docs/interface/05-look-back.md:45) |
| mush-net-ssh `LineEditor`; mush-core `parse_lock`; mush-dice; mush-npc `parse_actions` | player / builder / the model | — | proptest | yes |

The order of the real gaps is therefore: the command line (one panic freezes the world), the key parser and
the caps replies (raw internet bytes, backtracking indices), the Rhai depth guard (an abort, not a panic), the
economy's stateful model. Those are TS.4 and TS.5. mush-ipc's two gaps are cheap and go with TS.4.

### 5.2 The failure class the tests do not see: wiring

The 35b `DOCK` dead end (both clients emitted a string no verb parsed; `DockingClientSession` had no production
caller; every space and pixel test green) is one instance of the class the owner's 2026-09-30 survey of
built-but-unwired features found 148 times: the module proves itself and nothing proves the seam. The tests
are organised by crate; the game is organised by verb. Two cheap contracts close most of the class at the
client edge:

- **Every command string a client emits is a verb the production registry parses.** The TUI's key bindings and
  the web client's command strings are enumerable (the TUI's in `mush-net-ssh`, the web's in `web/src/lib`); a
  contract test over `production_composition()` in the style of `exit_command_words.rs` would have refused
  `DOCK` before it shipped (TS.7).
- **Every production handler has a production caller** is not a test (dead code is clippy's), but every
  *session type* with no production constructor is: a `pub struct` in a net crate constructed only under
  `cfg(test)` is the `DockingClientSession` shape; a small scanner over the net crates finds it (TS.7's second
  half).

### 5.3 A hole in the shard guard

`ci_tacsim_shards.rs:55-97` builds the target set from `crates/tacsim/tests/*.rs` only. A file under
`tests/shard_N/` that no root declares compiles nowhere and fails nothing; there are none today, and nothing
keeps it so. The suffix-collision rule ("no module name a suffix of another's in its group", which `cargo test`'s
substring filter needs) lives in the out-of-tree merge script, not the guard. Both are a dozen lines (TS.0).

### 5.4 The harness delta is blind on pull requests

The deterministic block is deterministic across hosts, so the nightly's "delta against the committed baseline"
could run on a kernel cut before it merges; it does not, and the 2026-11-02 triage bisected three movers (CF.0,
CF.6b, GC.2) after the fact. The hosted cost is a release build of the harness binary; the VPS gate already has
`--kind harness`. A rule, not a cell: a cut that touches a hash-pinned file (doc 40 §4a's inventory) dispatches
the harness kind before it is marked ready (2.3 gains a thirteenth rule; TS.0 writes it into docs/ops.md).
The timing gate, separately, is **dormant everywhere**: its baseline host no longer exists and conclav.io has
eight CPUs now, so the 20 ms / 40 ms ceilings are asserted by nothing automatic (Q-43-6; ruled D-43-6: re-blessed on
conclav.dev, TS.21).

### 5.5 Main is unprotected, and a superseded run is never cancelled

There is no branch protection and no ruleset on `main` (checked 2026-10-27, restated in `ci.yml:255`). Every
CI signal is advisory; the owner merges on a green run by habit, and twice merged with the runs still in
progress. That is a workflow choice and it has worked; it is named here because every "required check" argument
in the reviews (and the fold of the named jobs) assumes a requirement that does not exist (Q-43-2). And the
workflow has no `concurrency` block: a second push to a pull-request branch starts a second run beside the
first, and both finish (18 of the last 100 PR runs; §2.4). `codeql.yml` already cancels (TS.15).

### 5.6 Stale text, found in passing

`ci.yml:39` "all 18 jobs" (21); `:1021-1022` "the cross-platform `test` job's `--workspace`"; `:755-757` "with its
default features" (rest-libs adds four); `:1087` "seven" `*_redis_gated` (nine); `:1192-1193` the canary under
`cargo test -p mush-bot` (nextest, bot-stories); the scale harness's `#[ignore]` reason "weekly" (daily);
`.github/tacsim-integration-shards.txt:42` "six shard roots"; `state-slot-owners.toml:~1218` "the 37 `[[site]]`
rows" (138); `shard_10/voyage.rs:5677` names `tests/shard_2/voyage.rs`; `mind-author-acceptance` and
`mind-full-replay` carry no comment saying why they are dispatch-only; `web/src/lib/stores/unit.test.ts:936`,
`game.test.ts:1776` and `roomband.test.ts:72` say no component harness exists; `mush-economy/tests/e2a.rs:529`
"near i64 bounds" (±2^40.6); `deploy/instance/render.sh:111-113` skips an unbuilt binary quietly. All TS.0.

### 5.7 What the local box cannot do

Windows cannot run the `cfg(unix)` stories, cannot `cargo fmt --all`, caps at `-j 8` and one workspace build,
and links a test binary in 3 GB with line tables only. Every one of those shaped a rule in 2.3; none is written
in the README's test section, which still presents `cargo nextest run --workspace` as the local sweep. The
honest local recipe is 2.2's last row (TS.0).

### 5.8 The real-time wait families

The second review found the timer in one pair; the lane found the family. `FieldConfig.disengage_s`
(`config.rs:1993`, default 120, validated 10..=3,600) is counted in arena milliseconds inside the arena task,
whose clock is `Instant::now()` under `tokio::time::sleep_until` on a multi-thread runtime: **the world clock
is faked by the rig, the arena's is not, and no test can shorten the window except by the setting.** The field
rigs' default tune sets 15 s (`field_host_rig.rs:33`); `field_gc10.rs:200` runs at 5 (`stocked_tuned`
validates nothing); and a handful of tests copied a gate's 90:

| Test | Setting | Fields settled | Wall by construction | Measured (run 41683232696) |
|---|---|---|---|---|
| `field_gc4::gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift` | 90 (the gate: the field must stay open while the wingman drags) | 2 | ≥ 180 s | 213.2 s |
| `field_gc4::lane3_a_casualty_keeps_its_row_when_its_tug_falls_irrecoverable` | 90, copied ("GC.4's tempo"); the body does nothing inside the field | 2 | ≥ 180 s | 211.6 s |
| `field_gc4::lane3_a_later_field_never_restarts_the_write_off_clock` | 90, copied; nothing inside the field | 2 | ≥ 180 s | 211.0 s |
| `field_gc5::gc5_review_the_captors_field_close_keeps_the_abandoned_extraction` | 90 (`quick`, `:38`) | 2 | ≥ 180 s | 213.9 s |
| `field_gc4::lane3_a_loaded_combat_casualty_never_darkens_the_lift`, `lane3_a_load_into_a_strangers_carrier_is_refused`; `field_gc5::gc5_a_non_friend_load_takes_custody_and_abandons_the_crew` | 90 | 1 each | ≥ 90 s | 115.5, 119.1, 123.2 s |
| `escort_lease::cf6c_…` (shard_8) | 90 (`:223`) | | | 201.8 s |
| `field_carrier` (shard_10), `field_director` (shard_9) | 600, to hold a field open | | | |

Seven tests in shard_1 spend at least 990 s of wall on that one timer by construction, of the shard's 58
summed minutes; shard_1 was the run's second-longest cell. The cheap half (TS.16): the tests whose bodies do
nothing inside the field take the rig's default; one representative journey per family keeps the gate's tempo.
The structural half (TS.10) is the clock seam, which is now known to need the **arena** task's clock as well
as the world task's. The same shape explains the bot's three long stories (590, 504, 336 s): the mining canary's
330-s floor is per-fire budgets and the market's 60-s sweep (docs/testing-rate.md §0), not work.

### 5.9 Configuration is a coverage dimension

The lane's census of `std::env::var` in the bot's and tacsim's tests, against `.github/`:

| Variable | What it switches | Selected by a workflow |
|---|---|---|
| `FIELD_RAID=director`, `FIELD_IDLE_S` | the captain/escort story's director-raid form (`mining_cohort.rs:924/:945`) | no (doc 38 §K: "no recorded run") |
| `MINE_RATE` | the mining canary and cohort's rate (`support/mine.rs:365`) | no (the default lifts to 168; 24 is the rate-24 run) |
| `GC12_SIZES`, `GC12_ARENA_S`, `GC12_ASSERT` | `field_scale` (ignored) | no |
| `TACSIM_MINDS_VARIANTS`, `TACSIM_SNAPSHOT_OFF`, `TACSIM_SIGHT_ORIGINS`, `TACSIM_HOLD_LANE` | harness lanes | no |
| `MUSH_BOT_WIN_RATE_*`, `MUSH_BOT_MIND_RATE_*`, `MUSH_BOT_LIVE_FROM_MUSTER`, `MUSH_BOT_LIVE_COMMANDER` | the win-rate, muster and live-commander stories (ignored) | no; one named in a comment only (`ci.yml:1151`) |
| `RELIEF_PREVIEW`, `MUSH_CAMPAIGN_START` | a content preview path; the campaign stories | no |
| `TACSIM_SEED`, `TACSIM_SENSOR_CELLS`, `TACSIM_SHARE_CELLS` | harness cells | named in `ci.yml` comments only (`:1220`, `:1255`) |
| `TACSIM_COHORT_TVS`, `TACSIM_COHORT_IC_DAYS`, `TACSIM_COST_JSON` | the cost lane | via `scripts/cost-lane.sh` on the gate |
| `TACSIM_ASSERT`, `TACSIM_JSON`, `TACSIM_LLM_COST_RUN/_MOCK/_MINUTES`, `MIND_PULSE_MS` | asserting lanes, the mock cost run, the stories' pulse | yes |

A variant nobody selects is a configuration with no test, under a test name the inventory calls covered. TS.2's
manifest keys coverage by (test, configuration); the gate gains an `env` input so a dispatch can select one
(the `MINE_RATE` patch already exists in a side-session note).

## 6. Where we undertest and where we overtest

**Undertested, in order of what a miss costs:**

1. The client-to-verb seam (5.2): no contract says a client's strings are verbs; the bot drives line mode only,
   so the TUI and the web client have no external driver beyond golden tests and vitest.
2. Untrusted bytes into the world task (5.1): the command line and the economy's stateful invariants, whose
   one generator has a fixed seed and two-line plans.
3. Untrusted bytes into a connection task: the key parser and the caps replies.
4. The Rhai depth guard, for an abort no `catch_unwind` reaches.
5. Lifecycle sequences: the fault matrix fires one fault per case on a fresh memory store; two faults, faults
   on outbox attempts, the Redis backend and a restart mid-sequence are unexercised.
6. The nightly and dispatch tiers: six stories with no automatic owner; a tripwire that accepts a comment as
   coverage; twelve env-switched variants no workflow selects (5.9); no failure channel.
7. The release runtime: no built release binary has ever started, served the embedded SPA, or handshaked over
   TLS in CI; the boxes find out at deploy time.
8. The web client in a browser: the WASM canvas and the socket round trip are exercised by nobody automatic.
9. The deploy and provisioning scripts: five in no list, nine syntax-checked only, one that skips quietly.
10. The harness delta before a merge (5.4); the timing ceilings at all; the reconstructions that pin no digest.

**Overtested, or paid for twice:**

1. Eighteen of the last hundred pull-request runs ran to the end after a newer push superseded them.
2. Three CI jobs that re-run tests the matrix ran, each recompiling mush-server and tacsim (36 runner-minutes
   a run; noise, not money).
3. Real-time waits inside stories: seven shard_1 tests on a 90-s window their bodies mostly do not use (≥ 990
   s by construction), 251 sleeps, ticks of 1–60 s, the canary's 330-s floor — minutes that measure the clock,
   not the game. The field tempo is the cheap fix; the clock seam the structural one; until it exists the
   stories' real-time share is known for one family only (the step audit, TS.3b).
4. Twenty-two web test cases that pin the spelling of Svelte source under a comment that explains a harness
   does not exist.
5. The 33 contract binaries, each linking the engine: ~20 GB on the VPS for seconds of tests (T.3c).
6. The two drifted copies of one lexer, 19 private directory walkers, and two copies of the lane strip chain.

**Not overtested, though it looks it:** the twin lenses (parity is the point), the story suites as such (the
house rule is right; their *waits* are the cost), the state-slot register (it is loud, and it is the only
cross-crate ownership record the tree has), the strip chains that pin a published digest (that is a fixture).

## 7. Cuts

Each cut is one PR with its own gate, **binding since the rulings of 2026-11-03** (§9); §7.1 says which run
together. TS.0, TS.1, TS.15, TS.16 and TS.21 may start now.

| Cut | What | Why | Size |
|---|---|---|---|
| **TS.0 hygiene** | the stale text of 5.6; the orphan-module and suffix checks in `ci_tacsim_shards` (5.3); the voyage.rs lint path; `render.sh` fails on an unbuilt binary; the harness-before-ready rule and the honest local recipe in docs/ops.md and the README | cheap, and the guards are only as good as their text | S |
| **TS.1 CI fold** | delete `direct-rand-policy`, `truth-serialization-policy`, `definition-wire-fixtures`; move `kernel-contracts`' mock LLM cost step into `tacsim-integration-1` as a named step (the inventory pins its name) and delete the job; `provisioning` stays | 36 runner-minutes a run for nothing; the checks list reads true | S |
| **TS.2 inventory tiers and configuration** | every gated test declares its tier (`pr`, `nightly`, `weekly`, `dispatch`, `by-hand`) and, where it has one, its configuration; a comment-only coverage needs a `by-hand:` tag with the last run's revision; the six dispatch-only jobs get a weekly schedule (doc 37 §7.10); `gate.yml` gains an `env` input; a failure channel for the nightly (Q-43-3); a cut that adds a story declares the story's tier in the same PR, and a programme story on the `dispatch` or `by-hand` tier is named in the Sunday slot or carries its `by-hand:` revision in the cut note (§10.3) | the tripwire counts a comment as coverage today; twelve variants run nowhere | M |
| **TS.3 optimised dependencies, measured** | one draft PR with `[profile.dev.package."*"] opt-level = 1` (and a variant at 2 for `rhai`, `serde_json`, `rmp-serde`, `sha2`, `blake3`, `image`, `flate2`), the twenty cells and the gate on each; compare summed test minutes per cell against main's last three runs | the shards are CPU-bound; nobody has measured runtime, only link time | S to measure; the decision is Q-43-4 |
| **TS.3b the step audit** | for the dozen modules of §2.4, classify each story's minutes: boot, real-time wait, work, assertion; one table (5.8 is the first row) | the only honest basis for "unit instead of story" | M, read-only |
| **TS.4 hostile-input suites** | proptest never-panic over arbitrary lines for `dispatch_line`/`ParsedArgs` in a `TestWorld`; arbitrary bytes into `KeyParser` and `caps.rs`; nested source into both Rhai engines (the depth guard engages, no abort); mush-ipc's corrupt-body-after-valid-prefix and mid-frame EOF; `client_command_strategy` made exhaustive over `ClientCommand`/`Query` variants | 5.1 | M |
| **TS.5 generated lifecycle** | a proptest state machine over accounts, holds, liens, reservations, title and custody with the engine's invariants 10–26 as the model, random histories whose checkpoint boot equals their whole replay, and the fault matrix extended to sequences (submit → retry → lost ack → restart → cancel → advance time; two faults a case; outbox attempts; the Redis backend in the `redis` group) with conservation, sole custody and recovered-equals-live after every step; `e2a.rs`'s fixed-seed generator replaced by one that shrinks; minimised failures kept as regressions | the generator covers one of twenty-six invariants; the matrix is not a sequence | M–L |
| **TS.6 fiat budget** | `fiat_budget/<crate>-tests.txt`: a per-file count of test-side `module_state_mut` (and `pair_mut`, path-qualified spellings), baselined at today's 210, ratcheted like `panic_budget` (a raise carries a reason naming the as-built section that allows the seed); `TestWorld::exec_ok` for the common case | the house rule has no ratchet for same-crate writes | S |
| **TS.7 client verbs are registered** | a contract test over `production_composition()`: every command string the TUI and the web client emit resolves to a registered verb; and a scanner for session types in the net crates with no production constructor | 5.2; it would have refused `DOCK` | S–M |
| **TS.8 one scanner, one strip chain** | `mush-contract-tests/src/scan.rs`: the walker, the comment-and-literal stripper, the `cfg(test)` blanker, `repo_root`; the two lexers become one; the test-rig rows in `state-slot-owners.toml` keyed by `<crate>/tests::<module>` rather than path; `strip_urvara` folded into the support chain | 19 walkers, two drifted lexers, 21 rows rewritten per reshard, two strip chains; lands before doc 41's lane v21 (RS.1, RS.6) so the second lane bump re-pins one chain, not two (§10.4) | M |
| **TS.9 web in a browser** | one Playwright smoke in the job that already builds `mushd` (`rest-mushd`, or TS.17's nightly release job), not the npm-only `web` job (1.4 min, no cargo; §10.1): boot `mushd` on the in-memory store, open the SPA in headless Chromium, log in, pick a character, run a verb, one typed action, reconnect, see the WASM canvas paint | the only automatic run of the real canvas and socket path | M |
| **TS.10 the clock seam** | a design note first: `world_task`'s and `arena_task`'s `now` behind one provider so in-process server, mushd and field tests fast-forward ticks and the disengagement window under `start_paused`; `PLAY_RATE` as `[tacsim.time].play_rate` (docs/testing-rate.md §5) | 251 sleeps, the ticks and the arena windows are the stories' real-time floor | note S; build L |
| **TS.11 T.3c** | the 33 contract binaries into `ci_policy` and `engine_contracts` (the T.3 plan's manifests) | ~20 GB on the VPS; deferred 2026-10-27 | S |
| **TS.12 partition experiment** | a branch where tacsim's tests are four domain binaries and the ten cells run `--partition hash:k/10` over them; measure link minutes per cell and the balance over three runs against main | ends resharding if the link cost is under the reshard cost | M to measure; Q-43-5 |
| **TS.13 shellcheck the rest** | run it over the five unlisted and the nine `bash -n` scripts, fix the findings, add them to the gate | the rule in the file | S–M |
| **TS.14 release check on PRs** | `cargo check --release -p mushd -p mush-server -p mush-gateway --features tls` in the lint cell | catches `cfg(not(debug_assertions))` errors on the PR; LTO stays nightly | S; Q-43-10 |
| **TS.15 cancel superseded runs** | `concurrency: { group: ci-${{ github.event_name }}-${{ github.ref }}, cancel-in-progress: ${{ github.event_name == 'pull_request' }} }` in `ci.yml`: a newer push to a PR branch cancels the older run; pushes to main keep every run (the bisect and the rebalances read main's JUnit) | 18 of the last 100 PR runs ran superseded | S |
| **TS.16 the field tempo** | the two lane3 regressions in `field_gc4` (and `gc5_review…`, the two one-field gc5 tests, `cf6c`) take the rig's default `disengage_s` where the body does nothing inside the field; one representative journey per family keeps 90; measured on shard_1's JUnit (expected −12 summed minutes, the cell off the wall); lands before doc 42's CR.1 and doc 41's RS.1, which rewrite `field_gc5`'s and `field_gc4`'s gates (§10.2) | 5.8 | S |
| **TS.17 release-runtime smoke** | in the nightly release job, after the build it already pays for: `--check-config` on the three release binaries; boot mushd on the memory store with a self-signed cert and fetch the embedded SPA over TLS; boot gateway + server and see one handshake and one verb; exit | nothing in CI has ever run a release binary | S–M; Q-43-13 |
| **TS.18 variant identities** | `FIELD_RAID=director`, `MINE_RATE=24`, the GC12 sizes and the harness variants become named entries (a matrix cell, a `by-hand:` row with its last revision, or a distinct test id) in TS.2's manifest; the director-raid run recorded once | 5.9 | S (with TS.2) |
| **TS.19 mounted assertions** | the 22 structural source-probe cases in `unit.test.ts`, `game.test.ts` and `roomband.test.ts` become mounted assertions or go; the 11 vocabulary bans stay; the three stale comments go | spelling is not behaviour | S–M |
| **TS.20 pinned reconstructions** | every rebuilt-old-content test pins a digest or reads a committed fixture: `ooc_v3`, the format-3 body, the pre-O.1 replay, the R4 relay shape; the `golden/v12` pattern as the template | a reconstruction that pins nothing can drift from the version it claims to be | S |
| **TS.21 the timing baseline on conclav.dev** | re-bless the timing block of `harness_baseline/arena.linux.json` on conclav.dev (`scripts/vps-gate.sh --kind harness --runner conclav-dev`, then `TACSIM_BASELINE_BLESS` on that box as `ghrunner`, with a `harness_timing_update` record naming the host, its core count and the reason); `vps-gate.sh` and `gate.yml` default the `harness` and `cost-lane` kinds to `conclav-dev`; docs/ops.md's "which box" paragraph and the harness section say the timing gate is live there; the cost lane's timing gate follows by the same reason (§8.1) | D-43-6: the gate has been dormant since the baseline host went away; conclav.io carries dev1 and dev2's play, which the framework cannot control; conclav.dev has no other load and is close enough for timing; lands before doc 40's FR.5, whose rebless needs a live timing baseline to re-bless against (§10.6) | S |

### 7.1 Order and parallelism

Astra's order (deduplicate and cancel; the browser smoke; the real-time waits; then generated lifecycle and the
manifest) and this document's agree on the shape. Proposed: **wave 1** TS.0 ∥ TS.1 ∥ TS.15 ∥ TS.16 ∥ TS.13 ∥ TS.21
(different files; a day); **wave 2** TS.2 + TS.18 (after TS.1, both edit the named jobs) ∥ TS.3 measurement ∥
TS.3b audit ∥ TS.4 ∥ TS.19 ∥ TS.20; **wave 3** TS.5 ∥ TS.6 ∥ TS.7 ∥ TS.8 (after TS.0) ∥ TS.9 ∥ TS.17 ∥ TS.11;
**then** TS.10's note and TS.12's measurement, and the decisions Q-43-4/5 rest on them; TS.14 in wave 2 (D-43-10). Nothing here blocks the programmes of docs 39–42: doc 38 §9's wave 0 and this wave 1 write no file in
common, and the edges between the two trains are §10's — TS.16 before CR.1 and RS.1, TS.2 before the first
programme story on the dispatch tier, TS.8 before lane v21, TS.21 before FR.5.

## 8. Owner survey

Each question carries a proposed default and an alternative. **(expensive to reverse)** marks one whose
answer is costly to undo, in doc 38's narrow sense.

**Ruled 2026-11-03 (owner):** every default accepted as written, except Q-43-6, amended below. §9 records the
decisions and §8.1 what they change elsewhere.

- **Q-43-1 The android prototype.** It is not in the repository. Default: it stays out, and this document
  records that no mobile client exists here; the review's findings about it are void. Alternative: track it,
  in which case it arrives with its own CI job in the same PR or not at all.
- **Q-43-2 Branch protection.** Default: none, as now — merges stay the owner's call and the checks advisory;
  the fold in TS.1 then costs nothing in required-check names. Alternative: a ruleset requiring the `test`
  matrix and `web` on `main`, which would also stop the merge-while-running habit.
- **Q-43-3 The nightly's channel.** Default: GitHub's own failed-workflow email to the owner (a personal
  notification setting, no workflow change, no secret). Alternative: a `failure()` step posting to a Discord
  webhook, which needs a secret and a `permissions` change.
- **Q-43-4 Optimised dependencies.** Default: run TS.3's measurement; adopt `opt-level = 1` for all
  dependencies only if a cell's summed minutes fall by a fifth or more; every warm target dir pays one
  dependency rebuild. Alternative: leave the profile as measured on 2026-10-18.
- **Q-43-5 Resharding.** Default: keep resharding by hand until TS.12's measurement says the partition's link
  cost is lower; the reshard recipe is scripted and documented. Alternative: adopt domain binaries and hash
  partitioning now, accepting a few minutes of link per cell.
- **Q-43-6 The timing gate's host.** Default: dormant until the first routine redeploy, then re-blessed on
  conclav.io at eight CPUs with a `harness_timing_update` record. Alternative: re-bless on conclav.dev (32
  cores, unrepresentative), or retire the timing gate and keep the deterministic delta alone.
  **Ruled: conclav.dev.** The owner: it has no other load, and it is close enough to conclav.io for timing;
  a run on conclav.io could be skewed by heavy play on dev1 and dev2, which the test framework cannot
  control. The default's "after the first routine redeploy" falls with it: nothing on conclav.dev waits on the
  redeploy, so the re-bless is a cut that may start now (TS.21).
- **Q-43-7 T.3c now.** Default: TS.11 in this train (small, deferred once). Alternative: leave the 33 binaries
  until the VPS disk forces it.
- **Q-43-8 A browser in CI.** Default: TS.9's one smoke, Chromium only. Alternative: none, and the bot grows a
  web-client driver later (larger, and the bot's charter is line mode).
- **Q-43-9 The fiat budget.** Default: TS.6 as written, baselined at today's count, a raise needing a reason.
  Alternative: extend the state-slot register to same-crate test writes (one row per site, 210 rows today).
- **Q-43-10 Release check on PRs.** Default: TS.14 (`cargo check --release`, minutes). Alternative: the
  nightly alone, as ruled on 2026-10-31.
- **Q-43-11 The weekly slot.** Default: Sunday 04:17 UTC for the six dispatch-only jobs at the fast pulse;
  the gate.yml harness kind stays dispatch-only on the owner's boxes. Alternative: fold them into the nightly
  on alternating days.
- **Q-43-12 The step audit's bar (expensive to reverse).** TS.3b will name story steps that are real-time
  waits. Default: no story is rewritten as units; waits are removed by the field tempo now and the clock seam
  later, stories keep the house rule. Alternative: allow a story's combinatorial branches to move to unit
  tests in the owning crate when the audit shows a step boots a world to test one refusal — 43a §3B as policy.
- **Q-43-13 The release smoke's certificate.** TS.17 needs a TLS certificate in CI. Default: a self-signed
  one generated in the job (`openssl req` to the runner's temp, never committed). Alternative: skip the TLS
  leg and smoke plain HTTP behind the embedded-SPA fetch, which leaves the tls feature compiled but never
  served.
- **Q-43-14 Cancellation on main.** TS.15 cancels superseded pull-request runs only. Default: keep every main
  run (the nightly bisect and the shard rebalances read main's JUnit per commit). Alternative: cancel on main
  too, trading per-commit evidence for runner time.

### 8.1 Consequences of the rulings

- **D-43-6 moves the timing host.** The baseline's host record (`linux/4/srv1930616`, the box that no longer
  exists) becomes conclav.dev's; the 20 ms / 40 ms ceilings and the per-window lenses of docs/ops.md's harness
  section are re-measured there and re-blessed by a `harness_timing_update` record, never by a blind bless.
  `scripts/vps-gate.sh` and `gate.yml` default `--kind harness` and `--kind cost-lane` to `conclav-dev` (today
  `conclav-io`); docs/ops.md's "Which box" paragraph ("the timing gate is live on the host that blessed the
  baseline") and "The weekly harness by hand" ("the run that measures is still the owner's, by hand, on that
  box") name conclav.dev. The cost lane's timing gate (16c T-109's other lens, `scripts/cost-lane.sh`) follows
  by the ruling's own reason — the load on conclav.io is the same for both — and is re-blessed in the same cut;
  the owner says so if the cost lane should stay. The harness's `TACSIM_ASSERT=1` absolutes (per-wake and
  per-window milliseconds) are what moves; the deterministic block is host-independent and does not.
- **D-43-11 adds a second cron.** `ci.yml` gains `- cron: "17 4 * * 0"` beside the daily one, and the six
  dispatch-only jobs' `if:` admits the Sunday schedule at the fast pulse (`MIND_PULSE_MS` empty); TS.2 writes
  it. The nightly's channel (D-43-3) is the owner's GitHub notification setting, outside the tree.
- **D-43-10 adds minutes to the lint cell.** `cargo check --release` of the three binaries with `tls` compiles
  every dependency once in the release profile's metadata; measured on TS.14's own PR run, and moved to a cell
  of its own if it sets the wall.
- **D-43-14 fixes the concurrency expression.** `cancel-in-progress: ${{ github.event_name == 'pull_request' }}`;
  pushes to main, the schedule and dispatches keep every run.
- **D-43-4 and D-43-5 wait on measurements** (TS.3, TS.12); nothing changes until the figures are in.
- **D-43-12 keeps the house rule**: TS.3b's audit may name a step that boots a world to test one refusal, and
  the answer is the clock seam or the field tempo, not a unit test in its place.

## 9. Decisions

Ruled by the owner on 2026-11-03.

- **D-43-1** No mobile client exists in this repository; a prototype outside it stays outside; doc 43a's
  findings about `android/` are void.
- **D-43-2** No branch protection and no ruleset on `main`; CI stays advisory; merges are the owner's call.
- **D-43-3** The nightly's failure channel is GitHub's own failed-workflow email to the owner; no workflow
  change, no secret.
- **D-43-4** Optimised dependencies are measured first (TS.3); `opt-level = 1` for all dependencies is adopted
  only if a cell's summed minutes fall by a fifth or more.
- **D-43-5** Resharding stays by hand until TS.12's measurement shows the partition's link cost is lower.
- **D-43-6 (amended)** The timing baseline is re-blessed on conclav.dev, now, by TS.21, with a record; not on
  conclav.io and not after the redeploy. The cost lane's timing gate follows (§8.1).
- **D-43-7** T.3c (TS.11) is in this train.
- **D-43-8** One Playwright smoke, Chromium only (TS.9), in the job that builds `mushd` (Draft 0.4, §10.1: the
  ruling named the `web` job, which builds no binary); the bot's charter stays line mode.
- **D-43-9** The fiat budget as TS.6: per file, baselined at today's 210, a raise carries a reason naming the
  as-built section that allows the seed.
- **D-43-10** `cargo check --release` of the three binaries on pull requests (TS.14); LTO stays nightly.
- **D-43-11** The six dispatch-only jobs run weekly, Sunday 04:17 UTC at the fast pulse; the gate.yml harness
  kind stays dispatch-only on the owner's boxes.
- **D-43-12** No story is rewritten as unit tests on the step audit's word; waits go by the field tempo
  (TS.16) and the clock seam (TS.10); the house rule of 2.3 #4 stands.
- **D-43-13** TS.17's TLS leg uses a self-signed certificate generated in the job, never committed.
- **D-43-14** Cancellation applies to superseded pull-request runs only; every run on main is kept.

## 10. The fit with the programme map (Draft 0.4)

Doc 38 §9 reads the 42 cuts of docs 39–42 and doc 23 §17 as one set; this section reads this document's
twenty-two against them. Six amendments, none a ruling's. Checked at `535efcb6`: `ci.yml` has no `concurrency`
block and one cron (TS.15 and D-43-11 stand); the field rigs are under `tests/support/field/` (PR #344) while the
two-`Ground` table is still `shard_9/field_story.rs:56` (doc 40 FR.0's hoist is that table alone); the `web` job
runs `npm` only.

1. **TS.9's job.** The ruling named the `web` job; it builds no Rust (1.4 min). Booting `mushd` there adds a
   full build to the shortest cell. The smoke rides `rest-mushd`, which already builds the binary, or TS.17's
   nightly release job. D-43-8 is read that way.
2. **Where a new story goes.** Rule 2.3 #3 is the shard with the most room; doc 42 §7a names
   `shard_1/crew_custody.rs`, and shard_1 is the run's second-longest cell (58 summed minutes), carrying the
   990-second timer family of 5.8. A wave-0 cut follows #3 (shard_2 at 11 minutes has the room) unless a
   `crate::` import pins it, and says so in its note; TS.16 lands before doc 42's CR.1 and doc 41's RS.1, which
   rewrite the `field_gc5` and `field_gc4` gates TS.16 retunes.
3. **The tier rule.** The programmes' whole-story cuts (IN.11, FR.7, RS.9, the CR stories) land "on gate.yml"
   with "the bot over SSH on conclav.dev" — the dispatch and by-hand tiers, where 5.9 found twelve variants with
   no recorded run. TS.2's manifest requires the cut that adds a story to declare its tier in the same PR; a
   programme story on those tiers is named in the Sunday slot (D-43-11) or carries its `by-hand:` revision in
   the cut note. Without it the next phase's acceptance evidence decays the way `FIELD_RAID=director`'s did.
4. **The lane fixtures.** Doc 39 bumps the lane pack to v20 (IN.1, IN.4, IN.5, IN.10) and doc 41 to v21 (RS.1,
   RS.6), and the strip chains pin the published digest (§4.1, TS.20): each bump re-pins across
   `support/sites.rs`, `relay_registry.rs` and their siblings. Those cuts budget the re-pin; TS.8's one strip
   chain lands before v21 so the second bump re-pins one chain.
5. **A per-cut wall budget** (rule 2.3 #14). Forty-two cuts add stories at two sizes against a twenty-minute
   wall where shard_4 sits at 19.7. The levers exist (TS.16, TS.12, TS.10); the rule makes each cut show its
   cost: the cell's summed-minutes delta from the PR's JUnit in the cut note, and a cut that would set a cell's
   wall moves its story or lands at the rig's default tempo. The deltas are the step audit's (TS.3b) data.
6. **TS.21 before FR.5.** Doc 40's one rebless re-blesses the timing block too and needs a live baseline to
   re-bless against; and rule #13 (the harness kind before ready) binds FR.5 and doc 23's S.4b, the two cuts
   that touch hash-pinned files. Doc 38 §9.3 carries both edges.

## 11. Cut notes

Doc 43 has no cut-notes file; each cut's note lands here.

### TS.14 release check on pull requests (D-43-10)

**Built.** The `lint` cell gains one step after Clippy: `cargo check --locked --release -p mushd -p
mush-server -p mush-gateway --features mushd/tls,mush-gateway/tls`, the release job's package and
feature set. A `cfg(not(debug_assertions))` break now fails the PR. Thin LTO and the real SPA embed
stay with the nightly release job. The embed guard (`mush-net-web/build.rs`) refuses a release build
with no `web/dist/index.html`. The lint cell builds no SPA, so the step writes a one-line placeholder
when the file is missing: the check is of the Rust, not the bundle.

**The cost (rule #14, §8.1).** MEASURED_PENDING

### TS.16 the field tempo (§5.8, §10.2)

**Built.** Test-only, two tunes in shard_1. `field_gc4::lane3_tune` and `field_gc5::quick` drop their
`disengage_s = 90` and take the rig's default (15 s, `field_host_rig.rs`'s `tune`); their other stage timings
stand. That moves the six tests whose bodies only wait for a field to settle and act after it: the four lane3
regressions in `field_gc4` (two settle two fields each, two settle one) and `field_gc5`'s two field tests,
`gc5_review_the_captors_field_close_keeps_the_abandoned_extraction` (two fields) and
`gc5_a_non_friend_load_takes_custody_and_abandons_the_crew` (one). `gc5_review…` reads the live record right
after its second field opens, well inside 15 s. `field_gc5`'s other two tests open no field. No assertion
changed.

**Kept at 90, one journey per family.** `field_gc4::gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift`
(GC.4's gate: the wingman drags while the field is open) and `escort_lease::cf6c_a_leased_escort_flies_out_fights_withdraws_and_goes_back`
(shard_8, CF.6c's gate: the escort drives home while the field stays up). §7's row names `cf6c`, but its body
acts inside the field, so it is the family's representative rather than a candidate.

**Rule #14, shard_1's summed minutes.** Runner speed moved the cell more than the cut did. The like-for-like
pair is two runs taken under the same parallel wave-1 load, where the 214 untouched tests sum within 3 %
(60.2 and 62.0 minutes):

| Run | Head | Cell, summed | The six | The 214 others | Wall |
|---|---|---|---|---|---|
| main [41754225803](https://github.com/conclavio/rustmush/actions/runs/41754225803) | `c0d123a9` | 78.3 min | 1,086.2 s | 60.2 min | 23 m 54 s |
| PR [41757602966](https://github.com/conclavio/rustmush/actions/runs/41757602966) | `51722d5e` | 68.9 min | 416.6 s | 62.0 min | 21 m 40 s |

The delta is **−9.4 summed minutes**, with the six tests saving 11.2. A quiet main run
([41749664818](https://github.com/conclavio/rustmush/actions/runs/41749664818) at `f94b1e26`) summed 52.3
minutes, with the six at 965.4 s and a wall of 16 m 16 s. Its 214 others ran 1.7× faster, which puts the six
at about 245 s at that speed and the cell near 40 summed minutes, the §7 estimate of −12. Locally (Windows,
nextest) the six take 300.8 s. The wall under load is still the queue's, not this cell's: TS.10's clock seam
is the structural half.
