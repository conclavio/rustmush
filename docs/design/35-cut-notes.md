# 35. Cut notes — the seam-first space train

One `## SF.Rn` heading per cut of [35-ship-free-flight-combat-and-mining.md](35-ship-free-flight-combat-and-mining.md)
§10.2 (`SF.R0`–`SF.R8`) and §10.3 (`SF.R9`–`SF.R16`, from [35b-seam-first-train-review.md](35b-seam-first-train-review.md) §8),
in the order they landed: what was built, the story test that proves it, and where the PR departed from
the plan's row. The initial nine cuts (`SF.R0`–`SF.R8`) merged on 2026-11-02 and were reviewed in `35b`; the follow-on
remediation cuts (`SF.R9`–`SF.R16`) closed every `35b` §3 hole. The story tests live in
`crates/tacsim/tests/shard_10/voyage.rs` (moved from `shard_2/voyage.rs` in #346) unless a section says otherwise.

Sources: the PR bodies of #297, #302, #309, #311, #313, #316, #317, #318 and #321. A PR body rarely says where it
left the plan, so "Divergences" lists what the body states or what its test text shows against §10.2's row;
lines marked **(audit)** come from the close-out audit in [doc 37](37-ic-phase-closeout-2026-11-02.md) §5.3, not
from a PR body.

The train replaced SF.1–SF.20 (PRs #266–#291), which [doc 35a](35a-space-flight-review.md) found to be an
island: no tick, RAM-only state, eleven authority holes.

## SF.R0 — foundation, fixes and island cleanup (#297, `aec12735`)

The doc 35a review (`35a-space-flight-review.md`), doc 35 Draft 0.3 and the README row, plus the fixes 35a §8.1–8.3
asked for before any seam work:

- **Sky (SF.1)**: `kirkwood_gap_3_1` moved to the 3:1 resonance at 2.50 AU; `outer_bodies.json` loads with
  `deny_unknown_fields` and `BodyDef` validation and is folded into `Sky::revision()`; the circular `velocity_mps`
  phase quantisation fixed; derivative and Galilean-moon velocity pins in `tests/shard_4/sky.rs`.
- **Pixel, bakes, web (SF.2, SF.13/14/20)**: Saturn's ring-plane projection and the ring's cloud-top shadow in
  `planet_shader.rs`; small-frame clamp bounds and the `pixel_model` lookup in `docking.rs`, `space_cockpit.rs` and
  `holotank.rs`; the hard-coded developer paths gone from both bake scripts (`RUSTMUSH_3D_ASSETS`), pure-Python and
  Tk sampling unified, `bake_params`/`pitch_deg`/`scale_factor` in the manifests, `docking_wireframe_mm`
  compacted in `ships/index.json`; golden hashes in `crates/mush-pixel/tests/space_golden.rs`
  (`tests/golden/space_hashes.json`); the sticky `spaceMode` in `web/src/lib/stores/picture.svelte.ts`.
- **The pure fire and kinematics library (SF.3)**: the railgun lead returns the world-frame intercept with
  mid-substep arrival interpolation; a sub-metre remainder `pos_rem_um: [i64; 3]` on `SpaceShipState`,
  `InFlightTorpedo` and `InFlightSlug`; torpedo proportional navigation and ballistic coast to `max_flight_ms`; a
  500 ms PDC burst cooldown and a target-lock check on decoys; `step_wall_ms_at(dt_wall_ms, ic_now_s)` with
  fractional-second carry; derelict `life_support_remaining_s` and `salvage_contest_window_s` countdowns; the
  `command_burn` `i64::MAX` clamp; `BubbleSnapshot` and `BubbleInputTape` replay.
- **Island deleted**: `commands.rs`, `stations.rs`, `pulse.rs`, `flight.rs`, `mining.rs`, `docking.rs`,
  `picket.rs`, `hulls.rs`, `content/space/hulls.json`, `autopilot_bot.rs` and `mush-bot/src/mine/space.rs`;
  `+helm`, `+tac` and `+mine` out of `legacy-handlers.toml` and `TacsimRuleset::register` until SF.R1–SF.R8
  re-cut them.

**Test.** No `sfr0_` story (the plan's row has none): the pins are `tests/shard_4/sky.rs` and
`mush-pixel`'s `space_golden`.

**Divergences.** The PR deleted the whole `docking.rs` module and `picket.rs`, which §10.2's row does not name
(the row lists `commands.rs`, `stations.rs`, `pulse.rs`, `flight.rs`, `mining.rs`, `hulls.rs`,
`autopilot_bot.rs`, `SpaceBook`); the module was rebuilt in SF.R7 and `picket.rs` in SF.R6. `custody` is named in the row's
module column but not touched here (SF.R2).

## SF.R1 — the tick consumer and the interdiction promotion seam (#302, `36658e7b`)

- **`mush-sim`** (D-35-3, D-35-5): a `CarrierJourney`'s post-interdiction leg timer pauses while
  `JourneyState::Promoted { domain }` holds (`with_promoted`, `with_paused_at`, `with_resumed`,
  `promote_carrier_journey`, `resume_carrier_journey`); `stand_down_domain` persists the `DomainSummary` and clears
  the `DomainActivation` when an interdiction bubble resolves.
- **`tacsim`**: the `tacsim.space` coarse world tick (`Registrar::tick`, `crates/tacsim/src/space/tick.rs`) and
  `SpaceBubbleBook`. An under-way carrier crossing an interdicted leg boundary opens a `LocalSpaceBubble` from
  canonical `UnitSpec` hulls (`PromotionTrigger::Interception`); active bubbles step on 1:1 wall time
  (`step_wall_ms_at` with live W-clock stamps, the input tape, snapshots); a clear resumes the journey.

**Test.** `sfr1_interdicted_leg_promotes_pauses_leg_timer_steps_wall_bubble_and_resumes_to_dock`: a chartered
pilot aboard the *Salt Moth* promotes at the Kirkwood boundary, leg 2 holds across 2,000,000 IC seconds while the
bubble steps on wall time, and the resume docks at Stickney Yards.

**Divergences.** None stated.

## SF.R2 — unit custody and the canonical hull (#309, `0f08e3d2`)

- `SpaceShipState` is populated from `UnitSpec::logistics` (`dry_mass_t`, `payload_ceiling_kg`,
  `propellant_cap_kg`, `rated_accel_mg`, `exhaust_kms`) and the live `TacticalUnit` cells of `dray.json`,
  `dray_p.json` and `halcyon.json` when a bubble opens (`check_out_space_unit` over
  `mush_tactics::custody::check_out`).
- On close, `check_in_space_unit` writes section internals, the five-facet armour, hull heat, section conditions
  (intact, damaged, destroyed, wreck) and component conditions (power → damaged on a drive or radiator crit) back
  to the `TacticalUnit`, so `+ship` reports the bubble's damage (`Damage: drive 190/250, hull 480/600.`).
- An open-epoch derelict stays open (D-35-12, FO-12, `FieldEpochDisposition::RemainOpen`); impacts go to the
  `SalvageClaimBook` black box; the 48 real-hour (172,800 s) `CustodyTitleStatus::PendingSalvageClaim` window opens.
- `mush-sim`'s `PlacementGate::perceives`: crew physically inside a checked-out carrier's multi-room interior
  still hear shipboard room narration.

**Test.** `sfr2_space_bubble_custody_checks_out_unit_persists_damage_on_checkin_and_opens_48h_salvage_window`
(composed world; it sets the post-duel damage and the drive's `internals = 0` directly before checking the
write-back, then checks the rear-laser strike path through `settle_disabled_carrier_bubbles`).

**Divergences.** The `PlacementGate::perceives` change in `mush-sim` is not in §10.2's row. **(audit)** The
hand-typed `SpaceShipState::new_dray` (`space/bubble.rs:290`, called at `space/custody.rs:201`) constants remain the base for every hull, so
D-35-1 is met for the three canonical hulls' mass and thrust only where `check_out` runs; the corvette and the raider
are `tacsim.unit.corvette`, which has no unit file and never goes through custody.

## SF.R3 — the authoritative `+helm` and `+tac` verbs (#311, `51f2f6be`)

- `+helm` (`/status`, `/station`, `/heading`, `/thrust`, `/emcon`, `/roll`, `/guidance`) and `+tac` (`/status`,
  `/scan`, `/lock`, `/beam`, `/torp`, `/railgun`, `/pdc`) registered authoritative (D-35-9). A caller must be
  physically aboard (`hull_aboard_for_actor`) and hold a bridge seat (`declare_station`, `assign_station`,
  `vacate_station`, `SpatialState.assignments`, `World::command_subjects_of`).
- Orders are validated and recorded as `mush_sim::tick::StandingOrder` (`Heading`, `Thrust`, `Posture`) and appended
  as `BubbleInputCommand`s to the active bubble's tape.
- Detection (D-35-10): `+tac/scan`, `/lock`, `/beam`, `/torp`, `/railgun`, `holotank_feed` and `cockpit_feed`
  go through `mush_propagation::resolve_detection` (`SignalKind::Infrared`: inverse-square plume plus hull heat plus
  EMCON against sensor aperture); light lag is stamped with `stamp_emission`.
- TUI (D-35-16): `LocalAction::SpaceViewToggle` (`PixelView::cycle_space_view`) on the lettered chart screen,
  cycling Chart → Holotank → SpaceCockpit → Docking without taking any of the 13 digit slots (`MAX_SCREENS`); `z`/`F1`
  fires the beam, `x`/`F2` the torpedo, `c`/`F3` the railgun in the holotank and cockpit views.

**Test.** `sfr3_authoritative_helm_and_tac_enforce_aboard_seats_write_standing_orders_and_gate_propagation_detection`.

**Divergences.** The key that cycles the space view is `v` (`mush-net-ssh/src/tui/screen.rs:1066`, label "view");
the plan's row names it `v` too, the PR body's text lost the character. Only the TUI is wired: no web key or
verb change shipped, though the row's module column lists "TUI/web views". **(audit)** `+tac/decoy` is referenced
in help but is not a switch.

## SF.R4 — the live propellant pool and the free-flight planner (#313, `b6ee2cb3`)

- Carrier `SpaceShipState` reaction mass and remaining delta-v come from the hull's `stow.propellant` vault pool in
  the `EconomyStore` (D-35-6). Single-kilogram free-flight and bubble burns accumulate in `HullPropellantLedger`;
  a `PendingSpaceBurn` goes out through `tacsim_port_burn` (`BurnProvider`) when a whole tonne is crossed.
- `port/passage.rs` gains `plan_passage_from(sky, pack, pos_km, vel_mps, target_place, …)` and
  `return_reserve_from(…)` over the ship's live 3D velocity; a burn below `RETURN_RESERVE` is refused with a warning
  unless `force` is passed.
- `space_tick` steps `SpaceFreeFlightRecord` on the live `WorldTimeline` IC delta with inverse-square solar gravity
  (Tier 1), a clean timed-burn expiry (`burn_remaining_ic_s`) and target-relative Intercept / MatchVelocity flip
  guidance (35a §5 #4–#5).
- `+helm/autopilot` re-enters a real `Cruising` `CarrierJourney` through `mush_sim::recouple_carrier_journey` and
  debits the planned passage's `burn_t` and `crew_days`.

**Test.** `sfr4_free_flight_decouples_debits_live_propellant_warns_return_reserve_and_recouples_autopilot`.

**Divergences.** None stated. **(audit)** The real-seconds fallback in the free-flight step is a literal `/ 24`
(`space/tick.rs:918`, `commands.rs:3468`), which breaks D-35-3; doc 37 lists it with hole 11.

## SF.R5 — asteroid field mining on the doc 20 engine (#316, `57f157eb`)

- The four orbital clusters (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`) lay
  up real `DARK` block place `Thing`s (`A`, `B`, `C`), block `Vault`s owned by `System(tacsim.sites)` and a cluster
  clerk `Thing`; `space_tick` has the clerk open each rock's `mush_industry::OreDeposit` through `industry_deposit_open`
  (`SiteOpener`), crediting conserved `metal_ore`, `ice_ore`, `carbon_ore` and `strategic_ore` (D-35-13, Q-35-3).
- Extraction and claims need a live prospecting licence from the Freeport Registry (2,500 cr, 168 IC days) through
  `licence_gate`. `+mine/stake <rock> [family]` and `+mine/contest <rock>` submit `industry_claim_tenure` through
  `SurfaceTenure`, staging `BlockTenure` records in `tacsim.claims` with `CLAIM_TERM_IC_SECS` (168 IC days) and
  `CONTEST_WINDOW_IC_SECS` (30 IC days). `pallas_inclined_belt` blocks A and B are seeded with `pallas_syndicate`
  claims; unleased or Syndicate-held frontier extraction raises a thermal bloom (+18 engineering heat) and a
  claim-jump flag. No scripted rival appears inside `+mine/laser` (35a §3 #9).
- `+helm/match <rock>` never teleports the ship: it needs a 3D Euclidean range ≤ 2,000 m and a squared relative
  speed under 5,000² (mm/s)² (|Δv| < 5 m/s) (35a §4, §5 #11).
- `+mine/laser` and `+mine/drones` start timed cycles that `space_tick` advances on `WorldTimeline`; up to
  `MAX_EXCURSIONS_PER_OPERATOR` (4) collector drones run under real `ExcursionId`s. A finished cycle submits
  `tacsim_site_work`, moving conserved ore from the rock's deposit vault to the hull's `stow.ore` (86,000 kg on
  `dray_p.json`) and updating the `PocketCursor` in `tacsim.pockets`.

**Test.** `sfr5_chartered_dray_p_buys_licence_matches_velocity_in_pallas_mines_into_stow_ore_and_sells_at_ceres_mill`
(the PR body quotes it as `voyage::sfr5_…`): refit, licence, burn to match, mine 20 t of `metal_ore`, stake and
contest, sell and mill at `ceres_port`.

**Divergences.** The plan's row says the 20 t mill to 8 t of `concentrate`; the PR body and the test say 8 t of
`metals` at the 5:2 ratio. The row says "stakes/contests Block `C`"; the seeded Syndicate blocks are A and B, and
the test's unlicensed stake is `+mine/stake C metals`. **(audit)** Mining at rate 24 fills the 86 t hold in about
20 real seconds (tuning, not a hole); `completed_space_mine_draws` is push-only (`space/mining.rs:1589`).

## SF.R6 — the Rhai customs picket and the ledger fine (#317, `b6f32a68`)

- The Kirkwood Customs Picket's corvette (`KIRKWOOD_PICKET_RHAI`) and the lurking Belt Raider
  (`BELT_RAIDER_RHAI`) run through `mush-tactics::mind::RhaiRunner` with persistent `this` memory and `trace`
  breadcrumbs (D-35-11).
- A three-party `LocalSpaceBubble`: the chartered carrier, the Republic Customs Corvette, and the Belt Raider
  drifting cold in `Ghost`. Hails are light-lagged radio through `resolve_detection` and `tacsim:notice`.
- `+helm/submit` inspects the carrier's live `stow.ore` and the `SurfaceTenure` / Registry licence. Unlicensed
  `strategic_ore` opens a public E4 fine contract in `mush-economy`, settled to `freeport_registry` on the 60 s
  sweep, and advances the `Customs Assertion` front in `mush-faction`.
- `+helm/run` builds `PicketOutcome::RanThePicket`, flips the corvette's relation to `Hostile` and advances the
  same front.
- When the raider lights its torch (`ActiveCombat`) and fires its two-torpedo salvo and a 120 km world-frame
  railgun shot, the corvette screens the torpedoes, the carrier jinks (`RailgunDodgedByJink`), and the raider ends
  as an open-epoch derelict with a 48 real-hour salvage window.

**Test.** `sfr6_kirkwood_customs_picket_story_1_end_to_end_rhai_doctrine_triad_combat_and_ledger_fine`; the PR
also ran `cargo test -p tacsim --lib -- space::` (12) and `mush-contract-tests`' `panic_budget`.

**Divergences.** None stated; the plan's row describes the cut at the level of "Story 1 end to end". **(audit)**
The story disables the raider by hand before `+tac/beam`, so doc 35 §9's acceptance is by fiat for that step;
`+helm/run` has a unit test only; story 2's claim-jumper duel, refit and Neptune/Triton burn have no test. NPC
hulls are `ObjId(900_000 + domain)` and contacts `hull + 1000` (hole 10).

## SF.R7 — client-side docking and server tape verification (#318, `902616a5`)

- `mush-sim::journey::dock_carrier_at_destination` merges a verified clamp outcome into `JourneyState::Arrived` and
  places the carrier at its port berth (`Berth::Docked { map, def }`).
- `tacsim::space::docking`: 60 Hz 6-DOF kinematics (`DockingStepperState`, `DockingRcsAuthority`),
  `DockingChallenge` and `DockingSession`, the RLE-coded `DockingInputTape` (1..=3,600 frames), deterministic
  server replay (`replay_docking_tape`), collar-scrape bounce detection (`CollarScrapeBounce`), `DOCK TRANS/ROT`
  order parsing, and `+helm/tow` for clamped derelicts.
- A hull with an active `DockingSession` is paused in `SpaceBubbleBook::step_active_bubbles`,
  `clear_resolved_bubbles` and `step_free_flight_records` (Q-35-5).
- `rcs_spent_kg` is debited from `stow.propellant` through `HullPropellantLedger` on a clamp or a bounce; a hard
  bounce (> 0.8 m/s) applies `collar_scrape_damage` to the Front facet armour.
- The client mirrors the fixed-point kinematics and the tape encoder (`mush-pixel::docking`
  `DockingClientSession`/`DockingClientInput`; `web/src/lib/picture.ts`).

**Test.** `sfr7_docking_session_pauses_ship_replays_valid_tape_into_berth_docked_rejects_forged_tape_and_tows_derelict`.

**Divergences.** The plan's row says "mush-pixel / TUI / web"; the PR body lists `mush-pixel` and the web
`picture.ts` only. **(audit)** No client submits a tape: `DockingClientSession` and `encodeDockingTape` have no
callers, and the TUI (`tui/pixel.rs:1338`) and web (`PictureView.svelte:814`) send bare `DOCK …` lines that no verb
accepts. The docking view pushes feeds only after a player command, not on the tick.

## SF.R8 — store-backed space persistence (#321, `f1ea7715`)

- `SpacePersistedState { format: SPACE_STATE_FORMAT, book: SpaceBubbleBook, salvage: SalvageClaimBook }` is stored
  in `TacsimStore` under `SPACE_STATE_KEY` (`tacsim:space:state`) on every `space_tick` and every mutating `+helm`,
  `+tac` and `+mine` command (`persist_space_state`), and restored at engine boot (`TacsimStore::load` and
  `space::boot_read` in `TacsimRuleset::on_world_loaded`).
- `Serialize`/`Deserialize` and `PartialEq`/`Eq` across `SpaceBubbleBook`, `ActiveInterdictionBubble`,
  `CompletedInterdictionBubble`, `SpaceDerelictRecord`, `SpaceFreeFlightRecord`, `HullPropellantLedger`,
  `ActiveClusterMiningSession`, `ClusterSiteState`, `ClusterTenureOrder` and `PendingSpaceMineDraw`, with
  `vec_tuple_map` for the non-string `BTreeMap` keys.
- `step_derelict_records` counts down `life_support_remaining_s` and `salvage_contest_window_s` on open-epoch
  derelicts on each world tick that the timeline is not frozen.

**Tests.** The unit test `space_persisted_state_round_trips_json_with_bubble_relations_standing_orders_and_salvage_book`
(`crates/tacsim/src/space/mod.rs`) and
`sfr8_space_state_persists_in_tacsim_store_and_survives_full_engine_restart_mid_bubble_free_flight_derelict_and_docking`:
story 1 (a mid-bubble triad encounter) and story 2 (mid-free-flight, an open-epoch derelict and a paused docking
session) restart through `reboot_with` with bit-identical `SpaceBubbleBook` and `SalvageClaimBook`.

**Divergences.** The PR persists the whole book on every tick, which the plan's row does not say; PR #324 (`1edc47b3`)
gates the write to real changes because three `cost_lane`/`drones` stories counted the
per-tick `tacsim:space:state` write and main went red. **(audit)** The document is still one unbounded value written
whole: `completed_space_mine_draws` is push-only, `derelicts` are never removed, `tape.entries` grows.

## SF.R9 — hold promoted interdictions across engine ticks and step bubbles on 1:1 wall time (#332, `6aa49514`)

- `mush_sim::journey::step_promoted_journeys` checks `DomainId("tacsim.space.interdiction")` alongside
  `tacsim.boarding` in `ActivationBook` so a `CarrierJourney` promoted into a `LocalSpaceBubble` stays in
  `JourneyState::Promoted` across full `WorldEngine::tick_world` calls until the bubble clears (`35b` §3.1, `D-35-5`).
- `ActiveInterdictionBubble::last_wall_step_ms: Option<u64>` (`#[serde(skip)]`) tracks the last wall-clock pulse
  timestamp per active bubble; `SpaceBubbleBook::step_active_bubbles(w, dt_ic, now_ic, now_ms)` steps each bubble by
  `dt_wall_ms = (now_ms - prev_ms).min(MAX_BUBBLE_TICK_STEP_MS)` instead of multiplying `dt_ic` by 1,000 (`35b` §3.2,
  `D-35-3`).

**Test.** `sfr9_promoted_interdiction_holds_across_full_engine_ticks_and_steps_on_wall_clock`: a promoted ship stays in
its bubble across three `tick_world_at_wall_ms` calls at `dt_ic = 3_600` (`500 ms` apart in wall time) and advances
`1_500 ms` of bubble wall time (`3_000 ms` total including the initial 500 ms step on promotion).

**Divergences.** None.

## SF.R10 — emit real propagation events and push space panel keyframes (#333, `c13ab990`)

- Bubble hulls and sites register in `mush_propagation::PropagationBook` (`ensure_bubble_propagation_locations`) and
  emit `PropagationEvent`s (`torch_burn`, `weapon_flash`, `transponder_ping`) with source position, magnitude, and
  emission timestamp (`35b` §3.3, `D-35-10`).
- `resolve_detection_for_hull` reads `PropagationBook::perceived_events` for the observer's location at the current
  perception horizon so a remote torch burn or weapon flash is invisible before `d / c` and detected after light-lag
  arrives.
- `push_space_panels` (`crates/tacsim/src/space/commands.rs`) pushes `space.holotank` and `space.cockpit` keyframes on
  every `space_tick` and command step via `Ctx::push_panel` (`35b` §5 #7, `D-35-18`).

**Test.** `sfr10_propagation_book_perceives_events_across_light_lag_and_tick_pushes_space_panel_keyframes`: a distant
torch burn is invisible to `+tac/scan` before `d / c` and visible after; `drain_panel_events` yields `space.holotank`
and `space.cockpit` keyframes on a tick with no player command.

**Divergences.** `push_space_panels` in `SF.R10` covered Tier-2 active bubbles; Tier-1 free-flight and orbital cluster
panel pushes (`build_tier1_space_panel_payloads`) landed in `SF.R16`.

## SF.R11 — wire `+helm/decouple`, free-flight entry and mid-space `+helm/autopilot` recouple (#335, `b157a098`)

- `+helm/decouple` transitions a `Cruising` `CarrierJourney` into a `SpaceFreeFlightRecord` interpolated from the leg's
  current sky positions and velocities (`35b` §3.5, `D-35-6`).
- `+helm/autopilot <place>` from `SpaceFreeFlightRecord` plans a passage from the hull's current `[pos_km, vel_mps]` via
  `plan_passage_from` and opens a `CarrierJourney` whose leg runs for the planned duration (`recouple_from_free_flight`),
  removing the free-flight record cleanly.

**Test.** `sfr11_pilot_decouples_from_cruise_burns_in_free_flight_and_recouples_from_arbitrary_coordinates`: a non-GOD
pilot types `+helm/decouple` mid-leg, burns off-corridor in free flight, and recouples via `+helm/autopilot stickney`
with zero test calls to `module_state_mut::<SpaceBubbleBook>()`.

**Divergences.** None.

## SF.R12 — end-to-end derelict lifecycle and split bounded `TacsimStore` keys (#336, `611d754c`)

- When a hull's drive is disabled in a `LocalSpaceBubble`, `SpaceBubbleBook::clear_resolved_bubbles` checks in custody
  with the epoch kept open (`check_in_space_ship_preserving_epoch_if_disabled`), creates the `SpaceDerelictRecord`, and
  opens the 48 real-hour (`172,800 s`) `SalvageClaimBook` window without requiring a test helper (`35b` §3.6, `D-35-12`).
- Space persistence splits the monolithic `tacsim:space:state` document into dirty-tracked per-entity keys
  (`tacsim:space:bubble:<id>`, `flight:<hull>`, `derelict:<hull>`, `cluster:<site>`, `docking:<hull>`, `ledger:<hull>`,
  `meta`), caps `BubbleInputTape` at `MAX_TAPE_ENTRIES` (`4,096`), prunes resolved derelicts and completed bubbles, and
  retains a one-time boot fallback from `tacsim:space:state` (`35b` §3.9, `D-35-19`).

**Test.** `sfr12_live_bubble_combat_disables_raider_opens_derelict_and_persists_per_entity_store_keys`: live bubble
combat disables a raider, opens the derelict and salvage claim on bubble resolution without calling
`record_disabled_derelict`, and writes only the touched per-entity `tacsim:space:*` keys.

**Divergences.** None.

## SF.R13 — close authority and lifecycle bypass holes (#337, `56b963d4`)

- `+mine/stake` and `+mine/contest` verify aboard presence, `command`/`helm` seat authority, and `has_claim_authority`
  on the caller's party; `+helm/bail` requires `command` or `helm` seat authority (`require_helm_or_command_seat`);
  `+helm/dock abort` verifies `session.hull == hull` (`35b` §3.4, `D-35-9`).
- Removed the player-callable `+tac/hail clear` branch so only the Rhai customs picket doctrine (`hail_clear_ship`) or
  combat resolution can clear a hail.
- `+helm/dock tape <session_id> <rle>` requires a matching active single-use `DockingSession` for the caller's hull and
  consumes the session on any terminal outcome (`Clamped`, `OutOfGate`, `CollarScrapeBounce`, `Aborted`); stale sessions
  expire after `MAX_DOCKING_SESSION_WALL_S` (`60 s`) in `expire_stale_docking_sessions` (`D-35-14`).

**Test.** `sfr13_unseated_and_off_ship_callers_cannot_stake_bail_clear_hail_or_forge_docking_tape`: unrostered and
off-ship callers are refused across `+mine/stake`, `+mine/contest`, `+helm/bail`, and `+helm/dock abort`;
`+tac/hail clear` is rejected; a forged `+helm/dock tape` without an active session is rejected; a stale docking session
expires after 60 wall seconds.

**Divergences.** None.

## SF.R14 — author clusters, pickets and doctrines in the lane pack (#338)

- Added `SpaceClusterDef` and `SpacePicketDef` to `crates/tacsim/src/pack.rs` (`clusters.json`, `pickets.json`, and
  `doctrines/*.rhai` in `packs/solar_2240/`) and removed the inline cluster and picket tables from `mining.rs` and
  `doctrine.rs` (`35b` §3.7, `D-35-13`, `D-35-17`).
- Added content-studio JSON schemas (`clusters.schema.json`, `pickets.schema.json`) and studio validation for cluster
  sites, ore pools, syndicate tenure seeds, picket casts, and Rhai doctrine scripts.

**Test.** `sfr14_lane_pack_authors_clusters_pickets_and_rhai_doctrines_under_studio_validation`: `solar_2240` loads and
validates all 4 orbital clusters and the Kirkwood customs picket doctrine from pack files with zero inline Rust tables.

**Divergences.** None.

## SF.R15 — canonical hull fidelity, multi-contact combat and linkdead policy (#339)

- Mapped 3D attack bearings onto canonical `UnitSpec.facets` (`front`/`left`/`right`/`rear`/`top` on 5-facet ships,
  4-facet fallback on ground units) and routed bubble weapon hits through `mush_tactics::combat::resolve_attack`
  (`35b` §5 #8, `D-35-1`).
- Scaled ship linear acceleration from canonical `UnitSpec.tons`, live `stow.ore` + `stow.propellant` cargo mass
  (`cargo_mass_kg`), and live `drive` section integrity (`drive_cell_ratio`) (`35b` §5 #9).
- Generalized `ActiveInterdictionBubble` sides (`challenger_hulls: Vec<ObjId>`, `defender_hulls: Vec<ObjId>`) for N-ship
  patrols and multi-contact target selection (`+tac/lock <contact>`) (`35b` §5 #10).
- Enforced linkdead thrust cut (`D-35-7`): when all crew aboard a hull in a bubble or free flight have disconnected
  (`!w.Connected(actor)`), main thrust cuts to zero (ballistic drift) while keeping point defense armed (`35b` §5 #12).
- Upgraded `Sky::velocity_mps` to return full 3D orbital velocity (`[vx, vy, vz]`) including orbital inclination `i`
  (`35b` §5 #13, `D-35-4`).

**Test.** `sfr15_canonical_hull_fidelity_multi_contact_combat_and_linkdead_ballistic_cut`: a `Dray-P` loaded with 80 t
of ore accelerates slower than an empty `Dray`, a damaged drive scales thrust proportionally, a 2-ship patrol engages in
one bubble, and linkdead crew cut thrust to ballistic drift.

**Divergences.** None.

## SF.R16 — client docking game, per-session challenge and Neptune canopy

- Shared 60 Hz 6-DOF client docking kinematics and session driver across `mush-pixel::docking::DockingClientSession`,
  the SSH TUI (`crates/mush-net-ssh/src/tui/pixel.rs`), and the web client (`web/src/lib/picture.ts` +
  `web/src/components/PictureView.svelte`), stepping 6-DOF RCS inputs at 60 Hz (`wasd` + `r`/`f` translation,
  arrow/`ijkl` + `q`/`e` attitude/roll, `x` damp, `c`/`Space` guidance clamp-assist) and submitting
  `+helm/dock tape <session_id> <rle>` on collar contact (`35b` §3.8, `D-35-14`, `D-35-16`).
- Added deterministic per-session start pose (`DockingChallenge::session_pose_from_seed`, `standard_port_for_session`,
  `derelict_collar_for_session`) and distinct per-hull `DockingRcsAuthority` (`rcs_authority_for_hull`, with
  `tacsim.unit.dray_p` distinct from `tacsim.unit.dray`) emitted in the `space.docking` panel feed (`35b` §3.8,
  `D-35-14`).
- Extended `DockingSession` pause coverage (`is_hull_paused_for_docking`) to orbital cluster sessions
  (`step_clusters_on_tick` in `mining.rs`) and `Cruising` `CarrierJourney` legs when initiating derelict docking
  (`helm_dock` in `commands.rs` and `expire_stale_docking_sessions` in `tick.rs`) (`35b` §5 #14).
- Extended `push_space_panels` (`build_tier1_space_panel_payloads`) and `+helm/autopilot <body_or_site>` to Tier-1
  free-flight and orbital cluster hulls, projecting `neptune` and `triton` (`default_canopy_bodies_for_site`) onto the
  `space.cockpit` canopy during outer-system expeditions (`35b` §8 `SF.R16`, `D-35-15`, `D-35-18`).
- Added contract lint `tacsim_integration_tests_never_mutate_space_bubble_book_directly` in
  `crates/mush-contract-tests/tests/arena_library.rs` pinning that zero tests in `crates/tacsim/tests/` call
  `module_state_mut::<SpaceBubbleBook>()` (`35b` §5 #15).

**Tests.** `sfr16_client_docking_game_randomised_pose_and_neptune_canopy` (`crates/tacsim/tests/shard_10/voyage.rs`),
`tui::pixel::tests::sfr16_tui_docking_client_session_plays_60fps_and_submits_rle_tape`
(`crates/mush-net-ssh/src/tui/pixel.rs`), `picture.test.ts` (`web/src/lib/picture.test.ts`), and
`tacsim_integration_tests_never_mutate_space_bubble_book_directly`
(`crates/mush-contract-tests/tests/arena_library.rs`).

**Divergences.** None.
