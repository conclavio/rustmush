# 35. Cut notes — the seam-first space train

One `## SF.Rn` heading per cut of [35-ship-free-flight-combat-and-mining.md](35-ship-free-flight-combat-and-mining.md)
§10.2, in the order they landed: what was built, the story test that proves it, and where the PR departed from
the plan's row. All nine cuts merged on 2026-11-02, each as one PR; the story tests are in
`crates/tacsim/tests/shard_2/voyage.rs` unless a section says otherwise.

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
