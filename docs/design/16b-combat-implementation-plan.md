# 16b — Combat implementation plan

Draft 0.2, 2026-09-16, for [doc 16](16-combat-arena.md) Draft 0.2 on `main` at `5da8d6a`, revised after
the three-lens review in §10. Thirteen cuts, each with its steps, verify lenses and size, in the order
the dependencies allow. Sizes are approximate lines of production code plus tests.

---

## 0. Build discipline

1. Every cut ends with the workspace green: `cargo test --workspace`, the contract gates blessed in the
   same commit that moves them (panic budget, state-slot owners, legacy-handler baseline, restriction
   lints census, module-namespace owners), all-target Clippy, and the story test and harness once they
   exist.
2. No second lane. A new capability extends the one that exists or replaces it in the same cut; nothing
   is kept "beside" for compatibility, because there is no data to be compatible with. The one shipped
   combat lane (`mush-sim::combat`) is retired by cut T (D-16-20).
3. No version ladders. A definition that changes is a new registry version with a migration chain; code
   carries one shape.
4. No fixture impersonates a production module. Tests compose the game crate's composition root, or drive
   `Arena::step` directly where no host is needed.
5. Each cut has a one-page design note in the ledger (§9) before it starts and an as-built entry when it
   lands, with the exact commands run. Reviews are performed by a reviewer that did not write the cut.
6. Panic policy: `panic → orderly exit`. The arena task's `catch_unwind` is the one place a panic is
   contained; it voids the match and commits nothing the faulted pulse touched, consistent with the
   world task's firewall.
7. Every new verb gets a non-baseline stanza in `crates/mush-contract-tests/legacy-handlers.toml` with a
   category and a note in the same commit; the pinned baseline only ever goes down.

---

## 1. The cut map

| Cut | Scope | Depends on | Size | Acceptance |
|---|---|---|---|---|
| **R-a** | Hoist, rename, composition move (no deletion) | — | Medium | Workspace green with `crates/tacsim` as the composition root and `mush-llm` owning the structured call |
| **R-b** | The one sheet, the deletions, the gate re-bless | R-a | Large (deletion ~63k, additions 6 to 10k) | Workspace green with no `ruleset-wod`, no `ruleset-shadowrun`, one sheet, one mechanic |
| **G** | Configuration and deploy plumbing | R-b | Small | New `[tacsim.*]` sections mapped in `config_ext`; example, template and dev configs edited; no ruleset selector anywhere |
| **P** | Integer `Pose` | R-b | Medium | Nine consumers on `x_cm, y_cm, z_cm, heading_mdeg`; `@spatial/place` wire renamed; legacy pose arm gone; atlas goldens unchanged |
| **H** | Harness skeleton and baseline | P, G | Medium | 300 placements on the arena window, 12 fake sessions, a JSON report, the D-H-8-shaped CI job |
| **A** | Arena core library | P | Large | `Arena::step` with substeps, tempo, motion, index, labels, hash, ledger ring; the cross-tempo property test; the harness drives it |
| **T** | Arena task, seam, custody, retirement of the old lane | A, G | Large | A match runs under `mushd` and under the gateway with custody in and out; restart mid-match reconciles; the no-store contract test passes; `+fire` and its lane are gone |
| **C** | Content and registry | A, R-b | Medium | The centre (OOC pack v4), the arena, the tanks, weapons, ammunition, sensors and resolution tables load through the latch and the registry on fresh and existing worlds |
| **K** | Cockpit and lobby | T, C | Medium | Two players `BOARD`, start up, drive, `VIEW`; lifecycle, abort, linkdead |
| **S** | Perception | A, C | Medium | Detection, classification, sharing, notices; the observer rule surface by surface |
| **W** | Weapons, resolution, damage | S, K | Large | **The story test passes**, in-process and through the gateway |
| **N** | Reflex AI and the cost run | W | Medium | The harness at 300 with all units driven; `Engaged` p99 under 25 ms; cost run available |
| **X** | Extensions | N | per item | Each §9 acceptance in doc 16 |

Critical path: R-a → R-b → P → A → T → K → W. G, C and H run beside A; S beside K.

---

## 2. Cut R: ruleset removal and base cleanup

**Goal.** One game, one sheet, one mechanic, one composition root; nothing that exists only to make a
second game possible.

### R-a (no deletion)

- R1. Hoist `mush-worldgen`'s structured-call machinery (`generate.rs` `generate_room` and
  `generate_brief`: forced tool, JSON-in-text fallback, repair round trip, typed validation,
  `prompt_hash`, priority, the `retryable | budget` error classification) into `mush-llm` as a generic
  `call_structured<T>`; re-point both worldgen sites to it. `ruleset-wod/src/st.rs`'s copy is not
  hoisted; it is deleted with its crate in R-b. `plan.rs`'s agentic loop is out of scope.
- R2. `git mv crates/ruleset-tacsim crates/tacsim`; add `composition.rs` and move into it:
  `server_modules`, `server_modules_with`, `tacsim_module`, `atlas_module`, `content_module`,
  `recovery_policy`, `run_module_migrations`, `SCHEMA_KEY`, `housing_title_issuer`,
  `market_title_issuer`, `org_capability_vocabulary` and `sim_routes.rs`; `mush-server/src/modules.rs`
  keeps host wiring and calls the game. Re-home `OnboardingConfig` into `mush-core`. Write down the
  new `tacsim` dependency list and add the `cargo tree -p tacsim` acyclicity test in the pattern of
  `mush-campaign` and `mush-industry`. Update `state_slot_owners.rs`'s `crate_family` (its
  `strip_prefix("ruleset_")` fallback) and every `ruleset_tacsim::` path in `config_ext.rs` (about 30).
- R3. `mush-ruleset` becomes a platform library: keep `schema`, `doc`, `store`, `chargen`, `render`,
  `vitals`, `interface`; delete the `Ruleset` trait's multi-implementation posture, `[rulesets].enabled`
  and `ruleset_modules`. `mush-tactics` keeps its `mush-ruleset` edge (it uses `pool_state`, `pool`,
  `vitals_frame`) and drops its dead `mush-dice` edge.

### R-b (the deletion)

- R4. The game sheet (`tacsim/src/sheet.rs`): `gunnery`, `piloting`, `sensors`, the character-first facts
  of doc 10 §17 to §19, the sheet verbs and chargen over the library. Size it against what
  `ruleset-wod/src/{schema, sheet_cmds, onboarding, state, roll, xp, agent, health, damage, chargen,
  chargen_cmds}.rs` provided (about 6k lines) and take only what the game needs now.
- R5. `mush-dice`: keep the expression evaluator for `+roll` and Rhai; delete `owod20` and `sr5`. The
  game's resolution (doc 16 §3.5) does not use it.
- R6. Economy and market: delete `SrCatalogSource`, `SrAvailability`, `SrDelivery`, `SrBoardObserver`,
  `vendor_caps`; give `housing_title_issuer` a game-owned id; rename `mush_housing::STORY_QUEUE` from
  `wod-story` to a neutral id and update `e5b.rs`; the `wod.usd` and `sr.nuyen` instruments go, the ten
  families arrive with cut C.
- R7. Delete `crates/ruleset-wod` and `crates/ruleset-shadowrun`; the `[wod]`, `[wod.economy]`,
  `[shadowrun]`, `[shadowrun.economy]` sections and their `Cfg` types; the WoD `ModuleAsync` arm in
  `world_task.rs`; `mush-market/src/record.rs`'s ruleset fields; `mush-npc`'s `wod_roll` and `sr_roll`
  abilities (one `check` ability over the game sheet replaces them) and the K8.4 providers
  `willpower_roll`, `willpower_set`, `condition_damage`, `condition_heal`.
- R8. Tests. Delete: `mushd/tests/{wod_e2e, wod_splats_e2e, wod_more_splats_e2e, shadowrun_e2e}.rs`
  and the two crates' suites. Re-point to the game sheet, never delete: the ruleset-specific cases in
  `mushd/tests/{intel_e2e, npc_e2e, onboarding_e2e, perception_e2e, persistence, power_use_e2e,
  web_e2e, modules_e2e, support/mod}.rs`, `mush-server/tests/{e7r2_proving_loop,
  h2c_ability_coordinator, market_issuer, production_action_migrations, boot_parity,
  provisioning_smoke, e4c_bridge, e6b_seal}.rs`, the four TUI goldens (`tui_sheet`, `tui_rolls`,
  `tui_damage`, `tui_vitals_two_feeds`) and the `mush-proto/tests/wire.rs` and `mush-core/src/lock.rs`
  fixtures. Rewrite as one-sheet tests: `mush-contract-tests/tests/{vitals_projection_clean,
  npc_ability_residual, legacy_handler_allowlist, state_slot_owners}.rs`.
- R9. Gates, in the same commit: remove the 49 ruleset stanzas from `legacy-handlers.toml` and lower
  `PINNED_BASELINE` accordingly; delete the `ruleset-tacsim` and `ruleset-wod` rows from
  `restriction_lints.rs` and correct its crate census; strip the `wod` and `shadowrun` writer rows and
  the two `jobs ->` edges from `module-ns-owners.toml`; delete `panic_budget/ruleset-wod.txt` and
  `panic_budget/ruleset-shadowrun.txt`; delete the `shadowrun_e2e` line from `.github/workflows/ci.yml`;
  re-bless the panic budget and state-slot gates.
- R10. Docs: README rows and superseded entries (done); docs 04, 08d, 08f, 08g and the example configs
  get a "retired, see 16 §11" banner (04 done) rather than edits.

**Verify lenses.** No workspace crate depends on a deleted crate; `grep -r "wod\|shadowrun\|nuyen"
crates/*/src` returns nothing outside history banners; `mush-ruleset` has one implementor; every test
that proved a platform property with WoD content still passes with the game sheet; `onboarding_e2e`,
`perception_e2e`, `persistence` are green.

**Size.** R-a about 2k lines moved and 1k added; R-b deletion about 63k (two crates 60.7k plus 2k of
`mushd` tests plus their fixtures), additions 6 to 10k.

---

## 3. Cut G: configuration and deploy plumbing

- G1. `mush-config`: delete `WodCfg`, `WodEconomyCfg`, `ShadowrunCfg`, `ShadowrunEconomyCfg`,
  `[rulesets]`; add under `TacsimCfg` the `ArenaCfg`, `MatchCfg`, `SensorsCfg`, `ResolutionCfg`,
  `HarnessCfg` sub-sections with `deny_unknown_fields` and the defaults of §9.
- G2. `config_ext.rs`: delete `onboarding_ruleset`, `wod_economy_config`, `shadowrun_cadences`,
  `shadowrun_economy_config`; add the mappers for the new sections; `OnboardingConfig` now comes from
  `mush-core`.
- G3. Edit `mushd.toml.example`, `deploy/instance/mushd.toml.tmpl`, `deploy/dev1/mushd.toml`,
  `deploy/dev2/mushd.toml`: remove the ruleset sections, add the new ones. Delete
  `deploy/instance/instance.env.shadowrun.example` and its loop in the `provisioning` CI job.
- Verify: `provisioning_smoke` passes; a typo in a new section refuses boot.

**Size.** About 600 lines.

---

## 4. Cut P: integer Pose

- P1. `mush-sim/src/data.rs`: `Pose { x_cm: i64, y_cm: i64, z_cm: i32, heading_mdeg: u32, posture }`;
  `validate` bounds `heading_mdeg < 360_000`; `PoseWire` renamed keys; delete the `PoseWire::Legacy`
  arm.
- P2. Consumers, all nine: `mush-sim/src/{data.rs, commands.rs (the @spatial/place JSON and help),
  window.rs (place, pose_of, participants), journey.rs}`, `mush-atlas/src/{window.rs (pose_cm,
  pose_at, facing producers), frame.rs (locate), map_cmd.rs}`, `mush-atlas/tests/window.rs`,
  `mush-sim/tests/e7c6.rs`. Not consumers: `mush-npc`'s `NpcAction::Pose` (the emote) and
  `speech.rs`.
- P3. Authored packs keep metres (`Point2`, `Point3`) as input; `def.rs` converts through `cm()` at load.
- P4. Goldens: the atlas map goldens (`tests/window.rs` inline grids) and the TUI goldens render by cell
  and must not change.
- P5. Any `mushd` test that posts `@spatial/place` JSON is updated to the new keys.

**Size.** About 1.5k lines touched.

---

## 5. Cut H: harness skeleton and baseline

- H1. `crates/tacsim/tests/harness.rs`, `#[ignore]`, reading `[tacsim.harness]`; a generated 256 by 256
  relief with roads, woods and water; 300 placements on the window; 12 fake sessions through
  `mush-testkit`; the memory store; env-sized like `scale_harness` (`TACSIM_UNITS`, `TACSIM_ASSERT`).
- H2. Before cut A, the harness measures the current `sim.domain_pulse` with 300 placements and the
  output fan-out; the numbers are the baseline in §9.
- H3. CI per D-H-8: a `tacsim-harness` job on `workflow_dispatch` and `schedule`, report-only, keeping
  `harness.json` as an artifact; the target is named in `ci.yml` as `-p tacsim --test harness` so
  `ci_gated_test_inventory` passes; thresholds assert under `TACSIM_ASSERT=1`. The cost run (cut N) lives
  in the same target behind its own flag.
- H4. The report: per-pulse p50, p95, p99, max in each tempo; memory at start and end; events, traces,
  cell probes, path queries and share operations per pulse; ledger bytes streamed.

**Size.** About 1k lines.

---

## 6. Cut A: the arena core library

`crates/tacsim/src/arena/`:

- A1. `state.rs`: `Arena`, `ArenaDefinition`, `UnitState`, the epoch table, the shadow contact store, the
  due-event heap, the ledger egress ring.
- A2. `clock.rs`: tempo states, thresholds, hysteresis, `dt` clamping, `Lagged`, the substep length.
- A3. `intents.rs`: `Control` application, apply-time authorisation against the epoch table, continuous
  latest-wins, discrete sequenced and bounded.
- A4. `motion.rs`: doc 16 §3.4 per substep over `move_cost`; `index.rs`: the uniform grid.
- A5. `rng.rs`: `draw(seed, label)` and the enumerated labels; `hash.rs`: the per-substep state hash;
  `ledger.rs`: events, chunking, bounds, position summaries.
- A6. `step.rs`: the five phases of doc 16 §3.1 in order, each documented with its snapshot rule.
- A7. Tests: unit tests per module; the cross-tempo property test (one scripted intent and control log at
  5,000, 1,000 and 500 ms yields one event list and one hash sequence, crossing an impassable band and an
  occupied cell); a test that a tempo change mid-run changes no outcome; a test that adding a draw label
  shifts no existing draw.

**Verify lenses.** The arena module has no `use mush_store`, `tokio`, `GatewaySink` or `&mut World`;
`step` is `&mut self` over the inputs and nothing else; the harness at 300 moving units stays under
5 ms per pulse before perception and weapons exist.

**Size.** About 3.5k lines plus 2.5k tests.

---

## 7. Cut T: the arena task, the seam, custody, and the old lane's retirement

- T0. Disposition of the shipped lane (D-16-20): delete `mush-sim/src/combat.rs`'s verbs (`+fire
  +launch +heading +thrust +posture +dc +dock +eject`), the `tactical_intent` staging, the placeholder
  resolution in `tick.rs`, `s9_fire_gate.rs`, and the `+fire` stanza in `legacy-handlers.toml`
  (lowering the baseline). The `domaintick` pulse and `DomainTickCommit` remain for world-side
  non-combat domains; add a test that a `deployed` unit is never a participant of a world-side pulse.
- T1. `arena_task.rs`: owns an `Arena`, the inbox (`intents.max_pending_per_unit`), the egress ring
  (`ledger.max_unflushed_chunks`, the per-pilot frame slots), a timer driven by `next_due_ms`, a
  watchdog (`tempo.stall_ms`), `catch_unwind` around `step`, a `watch::Sender<bool>` stop signal, a
  `JoinHandle` on `PersistHandles` joined in both shutdown tails.
- T2. Custody. Check-out on `Boarding → Running` as one authoritative action: the `deployed { arena,
  snapshot }` marker on each unit record; refusals added with one shared helper at
  `TacticsDestroyGate::check`, `TacticsDetachGate::check`, `unit_damage`, `unit_remove` and the
  `@spatial` move commands; the world placement left frozen; `CustodyOut` sent. Check-in per unit,
  fallible: cells through `apply`, pose through `mush_sim::place`, `custody_lost` for a vanished unit;
  marker cleared; tank reset; seats, leases, focus and pods released. `Void` restores snapshots.
- T3. Routing: the cockpit verbs (cut K) validate focus, lease and epoch on the world task and forward
  `Intent`s; `Control` on seat activation, deactivation, revocation, linkdead onset and revival, `EXIT`,
  `SIM ABORT`; `Stop` on abort, timeout and void.
- T4. Output: the world task drains the arena's frame slots each iteration and emits them through its
  ordinary output path as `SendTarget::Char`; the arena has no `GatewaySink` and no session view.
  Ledger chunks are appended by the world task to `log:match.<id>` through `mush_core::Ledger` on its
  lane.
- T5. Boot reconciliation in the tactics recovery pass: `deployed` markers with no live arena are
  restored from snapshot, cleared, and their seats, leases, focus and pods released, with an
  `aborted(restart)` result.
- T6. `mush-contract-tests`: the arena library has no path to `ObjectStore`, `WriteBatch`,
  `GatewaySink` or `&mut World` (a source scan in the style of `restriction_lints`); the draw-label set
  is pinned; the custody markers are the only unit writes during a match.

**Verify lenses.** A `mushd` test and a gateway test board two fake pilots, start a match, drive, and end
by abort; unit records are byte-identical before check-out and after check-in when nothing happened,
and a unit driven ten cells has its world placement at the arena's final pose; a panic injected into
`step` voids the match with reason and the world keeps serving commands; a server restart mid-`Running`
leaves the units mutable and the pods free.

**Size.** About 3k lines plus 2k tests.

---

## 8. Cuts C, K, S, W, N, X

**Cut C, content and registry.**
- C0. The atlas `item` decoder gains `interior: bool`, applied as an interior space at pack apply; the
  eight pod items use it.
- C1. Typed decoders in `tacsim/src/definitions/` for units, weapons, ammunition, sensors, resolution
  tables and arenas; registration through the C1 registry as `RulesDefinition` entries.
- C2. `neighborhood.ooc_area.json` to version 4 in `mush-atlas`: `room_budget.maximum` raised to 10,
  the simulator lobby, red room and blue room, their exits, the selector item and the eight pods; the
  digest pin in `starter.rs` re-blessed. A test applies the upgrade on a world at version 3 and on a
  fresh world.
- C3. `content/arenas/proving_ground.json`: the 2.56 km arena with authored tiles, entry zones, key
  spots, the waypoint graph and the eight tank objects' initial placements; the tanks are created once
  by the arena pack's applier.
- C4. `content/resources.json`: the ten commodity families as `InstrumentDef` rows, registered at boot.
- Verify: the packs load on a fresh world and on a world at `ooc_area@3`; every definition has an exact
  version and validates its references.

**Cut K, cockpit and lobby.**
- K1. `match/` state machine with the named `Boarding → Running` trigger, `SIM` verbs, slots, `SIM
  START`, `SIM ABORT`, `match.max_duration_ms`, atomic release, the reset to `Idle`.
- K2. `BOARD` through `Transition::Place Interior`, the tank's station assignment,
  `request_seat_activation` with `helm`, `sensors`, `fire`, focus; `EXIT`.
- K3. Cockpit verbs as contextual selectors; `STARTUP` with the `power` capability and its due event;
  `VIEW`, `STATUS`, `WEAPONSTATUS`, `CONTACTS` frames.
- K4. Linkdead onset and revival delivered as `Control`; reclaim at a pulse boundary with a new epoch;
  the idle substitution recorded.
- Verify: two fake pilots complete `Selection → Boarding → Running → Aborted → Idle` twice in a row; a
  third account cannot claim a taken slot; a pilot who types `FIRE` then `EXIT` in one burst fires
  nothing; a pilot who disconnects mid-match reclaims on reconnect.

**Cut S, perception.**
- S1. Sensor and signature definitions; the pass (`arena/perception.rs`) as a due event over the index
  and the atlas trace with per-unit `Eye` heights; classification with the gap rule; team sharing.
- S2. The shadow contact store, id minting, the one refusal; contacts mirrored into the pilot's frame.
- S3. Notices with hysteresis; the world-side exclusion of the arena domain from `visible_rooms`,
  `units_for`, `+map`, `look` and unit listings; `unit.status` for deployed units.
- Verify: the observer rule surface by surface (cockpit, `+map`, `look`, `@unit/status`, notices, TUI,
  web); the harness's candidates, traces and probes per pulse.

**Cut W, weapons, resolution, damage.**
- W1. Weapon and ammunition definitions, `LOCK`, `FIRE` with cycle and ammunition, the percentage.
- W2. Resolution per doc 16 §3.5 over `direct_fire_path` and the silhouette tables; scheduled impacts as
  due events; revalidation; the blocked shot as a costed miss.
- W3. Damage as `UnitMutationPlan` through `apply`; consequences; wrecks; team elimination ending the
  match.
- W4. **The story test** in `mushd/tests/arena_story.rs` and its gateway twin.
- Verify: the story test; a shot at a target behind full cover costs a round and a cycle and misses; a
  moving target that leaves the predicted cell misses with reason; ammunition is conserved across a
  match; the money rule.

**Cut N, reflex AI and the cost run.**
- N1. The state machine controller and utility target choice in `arena/reflex.rs`; the waypoint graph;
  `[atlas].path_max_cells` raised for the arena; the `unreachable` edge; path queries bounded per pulse.
- N2. The harness drives all 300 units through it; `Engaged` p99 under 25 ms.
- N3. The deliberation hook: the order type, squad grouping, the decision pipeline request with a TTL,
  the doctrine fallback; the cost run mode behind `TACSIM_LLM_COST_RUN=1`.
- Verify: the harness thresholds; the cost run produces its table when the owner runs it.

**Cut X, extensions.** One cut per item of doc 16 §9, each adding its acceptance test and touching no
existing test.

---

## 9. Constants and the cut ledger

| Constant | Value | Home |
|---|---|---|
| `tempo.quiet_ms / alert_ms / engaged_ms` | 5,000 / 1,000 / 500 | `[tacsim.arena]` |
| `tempo.alert_hold_ms / engaged_hold_ms` | 30,000 / 10,000 | `[tacsim.arena]` |
| `tempo.alert_range_cm` | 2 × max weapon range | `[tacsim.arena]` |
| `tempo.substep_max_ms` | 250 | `[tacsim.arena]` |
| `tempo.stall_ms` | 2,000 | `[tacsim.arena]` |
| `max_catch_up_steps` | 4 | `[tacsim.arena]` |
| `intents.max_pending_per_unit` | 8 | `[tacsim.arena]` |
| `ledger.summary_ms` | 5,000 | `[tacsim.arena]` |
| `ledger.max_bytes` | 64 MiB | `[tacsim.arena]` |
| `ledger.max_unflushed_chunks` | 256 | `[tacsim.arena]` |
| `match.startup_ms` | 10,000 | `[tacsim.match]` |
| `match.board_timeout_ms` | 120,000 | `[tacsim.match]` |
| `match.max_duration_ms` | 3,600,000 | `[tacsim.match]` |
| `match.max_slots_per_side` | 12 | `[tacsim.match]` |
| `sensors.pass_ms` | 1,000 | `[tacsim.sensors]` |
| `sensors.max_signature_factor` | 1.5 | `[tacsim.sensors]` |
| `sensors.visual.max_range_cells` | 60 (600 m) at factor 1 | content |
| `sensors.classify_after_ms` | 3,000 | content |
| `comms.share_range_cells` | 30 | content |
| `weapons.cycle_ms` floor | 1,000 | content, asserted |
| `[atlas].path_max_cells` for the arena | 65,536 | config |
| `harness.units / fighting / weapons` | 300 / 24 / 150 | `[tacsim.harness]` |
| `harness.engaged_p99_ms` | 25 | `[tacsim.harness]` |
| `harness.max_traces_per_pulse` | 4,000 | `[tacsim.harness]` |
| `harness.max_path_queries_per_pulse` | 64 | `[tacsim.harness]` |
| `harness.max_ledger_bytes` | 64 MiB | `[tacsim.harness]` |

**Cut ledger** (filled as cuts land; one row per cut: design note date, commit, tests run, numbers,
deviations from this plan and why).

| Cut | Design note | Landed | Evidence | Deviations |
|---|---|---|---|---|
| R-a | | | | |
| R-b | | | | |
| G | | | | |
| P | | | | |
| H | | | | |
| A | | | | |
| T | | | | |
| C | | | | |
| K | | | | |
| S | | | | |
| W | | | | |
| N | | | | |

---

## 10. Review (2026-09-16, three independent lenses on Draft 0.1)

Three reviewers read Draft 0.1 of docs 16 and 16b against the tree: an architecture and consistency
lens, a feasibility and sequencing lens, and an adversarial lens. Thirty-five findings; every one is
dispositioned below and, where adopted, folded into Draft 0.2 of both documents. Nothing was rejected
outright; three items are owner questions or deferred.

### Critical

| # | Finding | Disposition |
|---|---|---|
| 1 | The in-RAM ledger with per-pulse `moved` events would reach 100+ MB in an hour and the custody-in bundle would exceed the 256 KiB record and 16 MiB bundle ceilings, failing the whole match commit (adversarial, architecture) | **Adopted.** Ledger streamed as 16 KiB chunks through the world task to `mush_core::Ledger`; `moved` replaced by `position_summary` every 5 s; `ledger.max_bytes`, `max_unflushed_chunks`, `match.max_duration_ms`; the result record holds the digest and log id; A2 restated as "memory flat with the ledger streamed". Doc 16 §3.1, §4; cut A5, T4 |
| 2 | The arena transmitting frames through `GatewaySink` cannot resolve pilots in the split server (no `SessionManager` there) and a full channel tears down the whole gateway boundary (adversarial, feasibility, architecture) | **Adopted.** Frames are a seam message: latest-wins slots drained by the world task and emitted through its normal output path as `SendTarget::Char`; the arena has no sink; T's acceptance includes the gateway twin. Doc 16 §3.1, §5; cut T4 |
| 3 | One-jump motion tests only the destination cell, so units tunnel through buildings and water at `Quiet` and `Alert` (adversarial, architecture) | **Adopted.** The pulse is a sequence of substeps bounded to one cell of the fastest unit; per-cell tests; the invariant "no unit occupies an untested cell"; the cross-tempo property test crosses an impassable band. Doc 16 §3.1, §3.4; cut A2, A4, A7 |
| 4 | The world's `+map` frame lists every unit in the arena domain by room visibility, and staff see everything (adversarial) | **Adopted.** A deployed arena's domain is excluded from `visible_rooms`, `units_for`, `+map`, `look` and unit listings for every viewer; A1 asserts the observer rule surface by surface. Doc 16 §1, §3.6; cut S3 |
| 5 | The contact ledger cannot be "reused unchanged": it is a durable action-state family the arena cannot write, its ids are owner-derived not viewer-scoped, and the arena would silently create a second identity space (architecture, adversarial) | **Adopted.** Arena contacts are an in-RAM shadow with their own id type minted from `draw(seed, contact:<viewer>:<target>)`, never crossing to the durable family; one byte-identical refusal; the reuse claim restated as vocabulary and projection rules. Doc 16 §3.6, D-16-10; cut S2 |
| 6 | Cut R's re-point list missed eight test targets and four contract tests that import the rulesets (feasibility) | **Adopted.** R8 lists them; the four contract tests are rewrites. |
| 7 | Cut R's gate step omitted the legacy-handler stanzas and pinned baseline, the restriction-lints rows and census, the module-namespace owners rows and edges, two panic-budget files and a CI line (feasibility) | **Adopted.** R9. |

### High

| # | Finding | Disposition |
|---|---|---|
| 8 | Queued intents survive a seat release because the seam has no revocation and the arena never learns the current epoch; a pilot fires from a pod they left (adversarial, architecture) | **Adopted.** `Control` message; per-`(unit, channel)` epoch table; authorisation at apply; purge on bump; K's verify lens. Doc 16 §3.1, §3.7; cut A3, T3 |
| 9 | Committing post-panic arena state through custody-out inverts the workspace's panic firewall (adversarial) | **Adopted.** A caught panic voids the match; `CustodyIn{Void}` restores check-out snapshots and commits no arena cell. Doc 16 §3.1; §0.6 |
| 10 | Stranded `deployed` markers after a crash or routine hot restart brick every unit that was in a match; no boot sweep specified (adversarial, architecture) | **Adopted.** The marker carries the check-out snapshot; boot reconciliation in the tactics recovery pass restores, clears and releases; a restart-mid-match test. Doc 16 §3.1; cut T5 |
| 11 | The `deployed` refusal was placed at `UnitAuthorityAdapter::prepare_mutation`, which does not exist on `main` (a Codex-branch name); the real admission sites let `@destroy` and `@unit/damage` through mid-match, and one vanished unit would fail the whole check-in (adversarial) | **Adopted.** Real sites named (`TacticsDestroyGate::check`, `TacticsDetachGate::check`, `unit_damage`, `unit_remove`, `@spatial` moves); check-in per-unit fallible with `custody_lost`. Doc 16 §3.1; cut T2 |
| 12 | Check-in wrote back cells but no poses, so world placements stayed at the entry cell for the whole match and after it (adversarial) | **Adopted.** `CustodyIn` carries `final_pose`, applied through `mush_sim::place` in the same action; the placement is documented as frozen while deployed. Doc 16 §3.1; cut T2 and its verify lens |
| 13 | Checking a unit out without keeping it placed drops the pilot's channel grants (`controls_are_live` requires a placed unit), and stations keyed by unit cannot live on the pod (architecture) | **Adopted.** The world placement stays, frozen; stations and grants are on the tank; `unit.status` for a deployed unit specified. Doc 16 §3.1, §3.3, §3.6 |
| 14 | The lifecycle never named the `Boarding → Running` trigger, had no boarding verb, and put check-out in two different places (architecture) | **Adopted.** `BOARD` verb; the trigger (all seated or `match.board_timeout_ms`); check-out on that transition; pre-custody aborts release slots and pods only. Doc 16 §3.7, §3.8; cut K1, K2 |
| 15 | "Wreck" had no M32 representation, and the tanks' provenance and reset were unspecified, so "a second match can start" had no mechanism (architecture) | **Adopted.** Wreck is `hull` condition `wreck` plus a declared `RevokeCapability` set; eight persistent per-pod tanks created once by the arena pack and reset by plan at check-in and `Idle`. Doc 16 §3.3, D-16-6; cut C3, K1 |
| 16 | The plan's harness-on-push CI job contradicts D-H-8 and re-implements `scale_harness`; `ci_gated_test_inventory` would fail unless the target is named in `ci.yml` (feasibility, architecture) | **Adopted as D-16-19.** D-H-8 shape (on demand and scheduled, report-only, `TACSIM_ASSERT=1`); the target named in `ci.yml`; the cost run inside the same target. **Owner question:** if a gating harness in CI is wanted, D-H-8 is superseded and the decision is recorded in D-16-19. Cut H3 |
| 17 | No cut plumbs configuration and deploy files; the live instance and example configs would keep advertising a selector that no longer exists (feasibility) | **Adopted.** Cut G. |
| 18 | The arena is a second intent lane, resolution and RNG beside the shipped `+fire` lane, and the plan's §0.2 forbids exactly that without saying which one dies (feasibility) | **Adopted as D-16-20.** T0 retires `mush-sim::combat`, `tactical_intent` staging, the placeholder resolution, `s9_fire_gate` and the `+fire` stanza; `domaintick` stays for world-side domains with a test that a deployed unit is never in one. |
| 19 | The centre's rooms cannot be authored as a `tacsim` pack requiring `ooc_area` (room budget is full, the atlas owns and digest-pins the pack, first-boot apply cannot reach a `tacsim` pack without a cycle), pods need an interior field that does not exist, and cut C depends on A's `ArenaDefinition` (feasibility) | **Adopted.** The centre lives in `ooc_area.json` version 4 with the budget raised to 10; C0 adds `interior: bool` to the atlas item decoder; C depends on A and R-b. Doc 16 §4; cut C |

### Medium

| # | Finding | Disposition |
|---|---|---|
| 20 | Sensor passes batched at pulse end evaluate one instant, so a dash across open ground inside a `Quiet` pulse is never seen; several draws were unkeyed; A7 compared runs within a tempo only (adversarial) | **Adopted.** Passes are due events applied at their timestamps between substeps; the classification gap rule; every draw labelled; A7 is the cross-tempo test. Doc 16 §3.1, §3.5, §3.6; cut A7 |
| 21 | A per-pulse LOS cache keyed by cell pair can almost never hit (one unit per cell, per-unit eye heights) and W4's cache assertion is unmeetable; the reflex controller's path budget is one sixteenth of the arena and `path()` has no production caller (adversarial, architecture) | **Adopted.** No cache; per-observer traces bounded per pulse (`harness.max_traces_per_pulse`); W4's assertion restated; `path_max_cells` raised for the arena; a waypoint graph in the arena pack; the `unreachable` edge; path queries bounded. Doc 16 §2, §3.2, §6; cut N1 |
| 22 | The flight-time formula was off by 100×, and the range the model needs was never defined (the trace's `range_cells` is Chebyshev) (architecture) | **Adopted.** `× 1000`; one Euclidean-centimetre range function; the trace supplies cover and blocking only. Doc 16 §3.2, §3.5 |
| 23 | Only the hit draw had a label; facet, burst and classification draws were undefined, and a long-lived stream violates doc 09 C5 (architecture) | **Adopted.** `draw(seed, label)` with an enumerated, contract-pinned label set; `provider_rng_policy` exemption recorded. Doc 16 §3.5; cut A5, T6 |
| 24 | The candidate cull at `max_range_cells` made any signature factor above 1 unreachable (architecture) | **Adopted.** Cull at `max_range × sensors.max_signature_factor`, declared in config and asserted. Doc 16 §2 W1, §3.6 |
| 25 | R's additions were underestimated by 2× and R needs a green commit between the move and the deletion (feasibility) | **Adopted.** R-a and R-b; sizes restated. |
| 26 | R2's composition move was one bullet for a change that multiplies `tacsim`'s dependency fan-out and orphans `OnboardingConfig` (feasibility) | **Adopted.** R2 lists what moves, re-homes `OnboardingConfig`, moves `sim_routes.rs`, adds the acyclicity test. |
| 27 | R1 hoisted the weaker copy (`st.rs`) out of a crate the same cut deletes; worldgen holds the richer original (feasibility) | **Adopted.** R1 reversed. |
| 28 | Cut P's consumer list had three omissions and two name collisions, and renaming the pose units is a wire change for `@spatial/place`; `PoseWire::Legacy` is a kept-beside arm (feasibility) | **Adopted.** P1, P2, P5 corrected. |
| 29 | `mush-tactics`'s `mush-dice` edge is already dead and its `mush-ruleset` edge is vitals rendering that R3 keeps; the grep lens could not pass as written; `mush_housing::STORY_QUEUE` and four TUI goldens are WoD-flavoured (feasibility) | **Adopted.** R3, R6, R8 and the lens corrected. |
| 30 | A6's phase list had no fire-resolution or impact phase and no per-phase snapshot rule (architecture note) | **Adopted.** Doc 16 §3.1 lists the five phases with their snapshot rules; A6 mirrors it. |

### Confirmed sound by the reviewers, no change

Contextual cockpit verbs on a focused station exist (`CommandReach::Operated`, `resolve_seat`,
`channel_for`) and none of the planned verbs collide with a registered global verb; the `JoinHandle`
and two-shutdown-tail pattern is accurate (`PersistHandles`); `catch_unwind` works because `panic =
"abort"` is deliberately refused by a pinned test; the deletion estimate for the two ruleset crates
(about 63k lines) checks out.

### Deferred or open

| # | Item | Status |
|---|---|---|
| 31 | Whether the harness gates CI (supersede D-H-8) or stays report-only | **Owner question** under D-16-19; the plan proceeds report-only |
| 32 | The 10 km arena (tile side per driver, tiled composite) | Deferred to doc 16 §11's "before IC play" list; the arena pack is authored at 2.56 km |
| 33 | Per-pulse durability and resume on an arena writer lane | Deferred to before IC play (D-16-3) |
| 34 | AI takeover of a dropped human slot (doc 07 §1.1's fallback controller) | Deferred to after cut N; the idle substitution is recorded in doc 16 §11 |
| 35 | `Full` cover and `LOCK` on unclassified contacts as gameplay policy | Adopted as written (blocked shots cost like any shot; `lock.requires_classified` defaults false); revisit with the first playtest |

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| Cut R breaks platform tests that used WoD content as a convenience | R8 re-points, never deletes, every test that proved a platform property |
| The world task's output path becomes the arena's bottleneck at 24 pilots and 500 ms | Frames are latest-wins per pilot and dropped when stale; the harness attaches 12 sessions from the first baseline and measures fan-out |
| Custody races (a pilot leaves the pod during check-out, a staff destroy mid-match) | Check-out is one authoritative action; boarding is locked from `Boarding`; the real admission gates refuse deployed units; check-in is per-unit fallible |
| Rate independence silently violated by a later cut | A7's cross-tempo property test is mandatory for every later cut |
| The ledger stream falls behind the world task | The egress ring is bounded; `Lagged` then `Void` are named outcomes; the harness asserts bytes streamed |
| The 2.56 km ceiling is hit by content before the 10 km path lands | The arena pack is authored at 2.56 km; the 10 km addition is designed in doc 16 §11 and re-run through the harness first |
| The centre's rooms in the atlas-owned pack couple the game to `mush-atlas` content ownership | Acceptable under the single-game direction; the tanks and everything mechanical stay in `crates/tacsim/content` |
