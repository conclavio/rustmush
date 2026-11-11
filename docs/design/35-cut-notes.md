# 35. Cut notes — the seam-first space train

One `## SF.Rn` heading per cut of [35-ship-free-flight-combat-and-mining.md](35-ship-free-flight-combat-and-mining.md)
§10.2 (`SF.R0`–`SF.R8`) and §10.3 (`SF.R9`–`SF.R16`, from [35b-seam-first-train-review.md](35b-seam-first-train-review.md) §8),
in the order they landed: what was built, the story test that proves it, and where the PR departed from
the plan's row. The initial nine cuts (`SF.R0`–`SF.R8`) merged on 2026-11-02 and were reviewed in `35b`; the follow-on
remediation cuts (`SF.R9`–`SF.R16`) merged on 2026-11-02..03 and were reviewed in
[doc 35c](35c-remediation-train-review.md); the second remediation train (`SF.R17`–`SF.R24`, planned in 35c §9)
merged on 2026-11-03..04 and was reviewed at `a76b1662` in [doc 35d](35d-sfr17-r24-train-review.md), whose §9
plans `SF.R25`–`SF.R31`. The story tests live in
`crates/tacsim/tests/domain_world/voyage.rs` unless a section says otherwise (they were in `shard_2/voyage.rs`,
then `shard_10/voyage.rs`, when the earlier cuts landed; section text that names `shard_10` records the path of
its day).

Sources: the PR bodies of #297, #302, #309, #311, #313, #316, #317, #318, #321, #332, #333, #335, #336, #337, #338,
#339, #353, #374, #376, #386, #395, #401, #402, #404 and #409. A PR body rarely says where it
left the plan, so "Divergences" lists what the body states or what its test text shows against §10.2/§10.3's row
(or 35c §9's row for `SF.R17`–`SF.R24`);
lines marked **(audit)** come from the close-out audits in [doc 37](37-ic-phase-closeout-2026-11-02.md) §5.3,
[doc 35b](35b-seam-first-train-review.md), [doc 35c](35c-remediation-train-review.md) and
[doc 35d](35d-sfr17-r24-train-review.md), not from a PR body.

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

## SF.R9 — authority close-out (#332, `67ee3347`)

- `+helm/dock`: target must be a disabled derelict in the hull's current bubble or an open port at the end of the
  current leg (`MAX_DOCKING_INIT_RANGE_M`, `MAX_DOCKING_INIT_REL_SPEED_MPS`), refused while any operational contact in
  the bubble is `Hostile` or ordnance is in flight; mandatory `<session_id>` on `+helm/dock tape`, single-use session
  closed on every replay outcome (`Clamped`, `OutOfGate`, `CollarScrapeBounce`, `Aborted`), and a 60 s wall deadline
  (`DOCKING_SESSION_TIMEOUT_WALL_S`) enforced by `expire_stale_docking_sessions` on `space_tick`; an in-bubble port
  clamp runs `check_in_space_unit`, files the completed bubble record, and calls `mush_sim::stand_down_domain` (`35b`
  §3 #1).
- `+helm/cluster` is removed; orbital cluster transit routes through `+helm/autopilot <cluster_id>` with arrival
  promotion on `space_tick` (`35b` §3 #2); `+helm/approach` steps closure across ticks instead of snapping in one call.
- Timed free-flight burns clear `burn_remaining_ic_s` and zero `thrust_mmps2` when `burn_s == dt_ic_s` (`tick.rs`,
  `35b` §3 #3).
- Settling a disabled primary carrier calls `mush_sim::fail_carrier_journey` (`JourneyState::Failed`), and
  `ensure_hull_can_maneuver` refuses `+helm/decouple`, `+helm/burn`, `+helm/autopilot`, `+helm/dock`,
  `+helm/approach`, and `+helm/match` for a hull in `derelicts` (`35b` §3 #4).
- `+helm/run` is guarded by `picket.inspection_status`; player integer magnitudes use `unsigned_abs`; `+helm/thrust`
  surfaces `TankReserveWarning`; direct `module_state_mut::<SpaceBubbleBook>()` writes and `clear_interdiction_bubble`
  calls are removed from `sfr1`–`sfr8`.

**Test.** `sfr9_authority_closeout_gates_docking_removes_cluster_teleport_stops_1s_burn_and_locks_disabled_derelict`.

**Divergences.** **(audit, 35c §4, §5, §6)** `+helm/dock DOCK ABORT` (`order abort`) runs `unpause_hull_after_docking`
whether or not a session existed and resumes any `Promoted` journey (`35c` §4 #1); `+helm/autopilot <cluster|body>`
departs directly from `Berth::Docked` without `prepare_departure` (`35c` §4 #2); the outer-system expedition branch
caps `burn_t` and `crew_days` at the live tank and locker (`35c` §4 #3); re-initiating `+helm/dock` overwrites an open
session and resets `started_wall_s` (`35c` §4 #7); `holding_for_salvage` blocks bubble resolution for every carrier in
the domain (`35c` §4 #8); the port-at-leg-end dock branch tests `remaining_s > 60` IC seconds rather than 3D range and
speed, and the free-flight branch treats `< 2,000 m` from the body centre as `200 m` (`35c` §4).

## SF.R10 — tempo and persistence budget (#333, `21c67a91`)

- `ActiveInterdictionBubble::last_stepped_wall_s: i64` records the wall-clock second of the previous tick fire;
  `step_active_bubbles` steps active bubbles by measured wall elapsed seconds (with 250 ms substeps and no catch-up
  burst on restart) rather than multiplying by `adaptive_step_wall_ms` (`D-35-3`).
- Persistence caps on per-bubble snapshots (`MAX_BUBBLE_SNAPSHOTS = 6`, keeping the initial snapshot), input tape
  entries (`MAX_BUBBLE_TAPE_ENTRIES = 64`), and completed bubbles (`MAX_COMPLETED_BUBBLES = 8`), plus clearing
  `completed_space_mine_draws` when draws land (`mining.rs:1597`); no-op ticks skip writing `tacsim:space:state` (`35b` §5 #9).
- Unreadable `tacsim:space:state` documents are quarantined (`TacsimStore::is_space_state_held`) across boot,
  `space_tick`, and `+helm`/`+tac`/`+mine` (`persist_space_state`).
- `reacquire_custody_after_boot` re-checks out active space bubble hulls immediately after `field::boot::phase` so
  `DeployedMark::field` custody survives a normal engine restart (`35b` §5 #6).
- `RhaiRunner` and `ArenaDefinition` instances are compiled and cached by source hash across ticks.

**Test.** `sfr10_tempo_and_persistence_budget_steps_late_ticks_bounds_10m_heave_to_quarantines_corrupt_doc_and_keeps_custody_across_restart`.

**Divergences.** **(audit, 35c §6.2, §6.3, §6.6)** `last_stepped_wall_s` is persisted in whole seconds and clamped to
`1..=10` s (so a stall > 10 s loses the excess, and `elapsed == 0` advances 1 s), and `Registrar::tick` remains a fixed
`TickCadence::Coarse(1 s)` (`35c` §6.6); persistence is still one `tacsim:space:state` document cloned for equality on
every tick without `deny_unknown_fields`, and `derelicts`, `cleared_interdiction_legs`, `clamped_derelicts`,
`active_tows`, `cluster_tenure_orders`, `propellant_ledgers`, and `standing_orders` are never pruned (`35c` §6.2);
`reacquire_custody_after_boot` fails if the server reboots while the W-clock is frozen (`35c` §6.3).

## SF.R11 — one propellant ledger, rated drive acceleration, and post-burn reserve (#335, `3e4e2018`)

- In-bubble `CommandBurn` and substep burns debit reaction mass with the Tsiolkovsky rocket equation from `exhaust_kms`
  (`burn_kg_for_law_dv`) and sync into `HullPropellantLedger` / `PendingSpaceBurn` so bubble jinks debit
  `stow.propellant` and `fractional_burned_kg` persists across bubble close and re-entry without refilling (`35b` §1,
  `D-35-6`).
- `max_accel_mps2` is hydrated from `rated_accel_mg` via `flip_accel_mps2`, and `+helm/burn` is capped at the hull
  drive's rated acceleration.
- `return_reserve_from` is evaluated on the post-burn `(pos_km, vel_mps)` state (`35b` §5 #10), and `+helm/thrust`
  surfaces `TankReserveWarning` refusals.
- Tick-context space burns (`step_active_bubbles`, `step_clusters_on_tick`) submit as the hull's captain via
  `Fire::as_principal` (`35b` §5 #11).
- `+helm/decouple` records `prepaid_unflown_t` and `+helm/autopilot` recouple charges only
  `plan.burn_t.saturating_sub(prepaid_unflown_t)` (`35b` §5 #12).

**Test.** `sfr11_one_ledger_rated_drive_debits_bubble_jink_in_vault_persists_across_reentry_and_warns_burn_away_from_port`.

**Divergences.** **(audit, 35c §6.5, §6.10, §7)** `max_accel_mps2` rounds `flip_accel` up with `ceil` in integer
`m/s²` (turning `0.49–0.65 m/s²` into `1 m/s²`) and `command_burn` applies 1 s of Δv immediately on each thrust change
(`35c` §5 `sfr11`, §6.5); `+helm/approach`, docking RCS, and session-expiry RCS debit `HullPropellantLedger` without
updating the active bubble ship's private `propellant_kg`/`delta_v_mps` (`35c` §6.10); tick burns fall back to `GOD`
when no captain resolves and discard submission errors (`35c` §7).

## SF.R12 — domain bubbles, real NPC hulls, and contraband seizure (#336, `6410a8cc`)

- `SpaceBubbleBook::by_domain: BTreeMap<DomainId, ActiveInterdictionBubble>` with multi-carrier
  `carriers: BTreeMap<ObjId, BubbleCarrierEntry>` and `ship_hulls` so multiple hulls arriving at a waypoint share one
  domain bubble (`35b` §5 #2).
- Added `crates/tacsim/content/units/corvette.json` (`tacsim.unit.corvette`); NPC picket/interdictor hulls and the
  Raider (crewed by named NPC leader `"Varga Teodor"` from `raiders.json`) spawn as world `Thing`s with
  `TacticalUnit`s checked out through `custody::check_out` (`35b` §5 #7).
- Collapsed `LocalSpaceBubble::relations` onto `party_relations`; `apply_collar_scrape_damage` preserves `leg_cursor`
  (`35b` §5 #5); salvage claims resolve `prior_owner` from `RAIDER_ATTR` and `port_base_obj` from the waypoint's domain
  host (`35b` §5 #7).
- Added contraband seizure (`PicketInspectionOutcome::ContrabandSeized`, `PicketInspectionStatus::ClearedAfterSeizure`)
  debiting unlicensed `strategic_ore` from `stow.ore` via `tacsim_port_burn` when an indigent pilot cannot cover the
  customs fine (`35b` §1 `D-35-11`).

**Test.** `sfr12_domain_bubbles_and_real_hulls_share_kirkwood_bubble_tow_real_raider_thing_with_leader_claim_and_seize_indigent_contraband`.

**Divergences.** **(audit, 35c §4 #4–#6, §6.3, §6.4)** Shared-bubble lifecycle (`is_clear_to_resume`,
`is_carrier_disabled_and_settled`, `settle_disabled_carrier_bubbles`, and the port dock clamp check-in) still reads
only the primary `carrier_ship_id` (`35c` §4 #4, §6.3); all carriers share `CARRIER_PARTY` and a single
`picket.inspection_status` so one carrier's `+helm/submit` or `+helm/run` applies to all (`35c` §4 #5); `SpacePartyId`
and `SpaceRelation` remain separate from `arena::party::PartyId`; the space raider spawns from `raiders().first()`
(`tacsim.unit.dray` stamped with `RAIDER_ATTR` naming `tacsim.raider.ahuna_skulks`), blocking `@field/raiders`, and
GOD-owned NPC `Thing`s are never destroyed on stand-down (`35c` §6.4); `cleared_interdiction_legs` keys on
`departed.at`, which shifts under pause/resume (`35c` §6.4); `+helm/tow` still files on the private `SalvageClaimBook`
with millisecond/1970 timestamps (`35c` §6.4).

## SF.R13 — presence and feeds (#337, `b1fc0391`)

- `SPACE_TICK` pushes `space.holotank` and `space.cockpit` keyframes on bubble promotion and every bubble step to all
  aboard crew (`all_aboard_crew_for_hull`, `35b` §5 #17, `D-35-10`).
- `cockpit_feed` populates `bodies` and per-contact `pixel_model` (`freighter`, `corvette`, `raider`, `halcyon`,
  `drone`, `derelict`, `torpedo`), and `mush-pixel::ship_shader` adds the `halcyon` 3D procedural shader archetype.
- `holotank_feed` and `cockpit_feed` filter `torpedoes` through `mush_propagation::resolve_detection`
  (`is_bubble_torpedo_detected`, scaling IR plume signature by observer sensor aperture).
- `SpacePresenceHook` registered on `HookPoint::Connected`, `HookPoint::Linkdead`, and `HookPoint::Disconnected`
  (`D-35-7`, `35b` §5 #19): when all live-connected crew aboard a hull disconnect (`is_connected && has_live_conns`),
  main-drive thrust cuts to `[0, 0, 0]`, `pdc_auto` holds armed through the 120 s grace window, `queued_return_port` is
  recorded on `SpaceLinkdeadState`, and reconnect (`from_linkdead: true`) restores manual control.
- Restored `space.cockpit` and `space.docking` to TUI `Chart` Home-key view cycling and wired `l`/`b`/`t`/`r`/`p`/`m`
  tactical keys in TUI and web (`PictureView.svelte` / `picture.ts`) to `+tac/*` and `+helm/*`.

**Test.** `sfr13_presence_and_feeds_push_to_all_crew_filter_torpedoes_and_handle_linkdead_grace`.

**Divergences.** **(audit, 35c §6.8)** `queued_return_port` is a port-name string that never executes at grace expiry
(expiry only zeroes thrust), orbital cluster sessions are not covered by the linkdead hook or tick panel push, and
linkdead detection is event-only (`35c` §6.8); light lag is printed on hails but not applied to delivery timing (`35c`
§6.8); undetected contacts can still be named in `+helm/dock` refusals, `+helm/approach`, and torpedo `target_pos_m`
(`35c` §6.8); the Tier-1 cockpit feed fabricates `delta_v_mps: 4_500`, hull 100%, heat 0 (`35c` §6.8).

## SF.R14 — authoring for doctrines, clusters, outer bodies, and mind seals (#338, `e6f573c4`)

- Authored `kirkwood_picket.{rhai,json}`, `belt_raider.{rhai,json}`, `interdictor_patrol.{rhai,json}`, and
  `drone_mining.{rhai,json}` under `crates/tacsim/content/minds/`, registered in `minds/content.rs` and
  `content_studio.rs` (`D-35-17`, `35b` §5 #20).
- Authored `kirkwood_picket: true` and the four orbital cluster rock tables on `kirkwood_depot` and
  `hazard: 4, interdiction: 3` on `ceres_stickney` leg 0 in `crates/tacsim/content/lane/ceres_phobos.json`.
- Converted `crates/tacsim/content/sky/outer_bodies.json` into a signed `rules_definition` pack
  (`tacsim.sky.outer_bodies@1`) verified through `pack::load_pack`.
- Attached `MindSeal` (`SealEngine::Scripted`) records onto NPC space hulls (`corvette_hull`, `raider_hull`,
  `interdictor_hull`) and collector drones (`DroneSortieState`) resolving in `TacsimStore::mind_source`;
  `step_kirkwood_picket` reads `this.stage` from `picket.corvette_memory`, and `should_open_kirkwood_picket` checks
  whether the waypoint domain/host is authored with `kirkwood_picket: true`.

**Test.** `sfr14_authoring_loads_clusters_doctrines_and_outer_bodies_from_packs_opens_picket_only_on_kirkwood_and_seals_drone_minds`.

**Divergences.** **(audit, 35c §6.9)** Clusters remain a four-variant Rust enum (`OrbitalClusterId` in `mining.rs`)
whose rock tables load from compiled-in `LANE_JSON` through a process-global `LazyLock` rather than the applied lane
pack (with authored `zone`, `anchor_body`, and `label` ignored and all four clusters placed on `kirkwood_depot`);
doctrines run on a private cached `RhaiRunner::new` with `source_lookup(None, ...)` rather than the sealed runner host
and `MindPool` cadence, and the interdictor ignores its doctrine's lock/speed/fire intents; drones remain timers paying
a fixed 4 t with a hand-built `ExcursionId` and discarded mind intents; the picket cast remains in Rust (`35c` §6.9).

## SF.R15 — hull fidelity, facet split, heat cell, bay repair, and salvage bounty (#339, `50f7aedc`)

- `dray.json`, `dray_p.json`, and `corvette.json` define `"heat_max": 1000` and `"heat_dissipation_per_s": 30`
  (materializing the `"heat"` cell on `"hull"`), round-tripped through `build_space_damage_mutation_plan` and
  `space_ship_from_tactical_unit_with_spec` (`D-35-1`).
- `SpaceShipState::apply_damage` reads facet weights, `struck_through` armour, section internals, and consequences
  (`slow_75`, `disable_motion`, `destroy_unit`) from `UnitSpec`, and inflicts `SubsystemCrit::PdcOffline` and
  `SubsystemCrit::SensorDegraded` (`35b` §5 #16); `step_pdc_intercepts` skips non-operational PDC mounts.
- `+helm/tow` settles a 1,200.00 cr salvage recovery payout from `freeport_registry`, and `sfr15` exercises a doc 31
  `+bay/order #<hull>=all` repair at Stickney Yards.

**Test.** `sfr15_hull_fidelity_splits_rear_hit_per_dray_json_strips_pdc_and_refits_in_bay_after_salvage_payout`.

**Divergences.** **(audit, 35c §4 #6, §6.5)** `SpaceShipState::apply_damage` is still a private damage pipeline
separate from `arena/damage.rs` (splitting hits proportionally across sections instead of `choose_section`, and
compounding `Slow` on every hit while `hold` is at 0 instead of taking `min`); PDC and sensor crits come from hardcoded
internal-damage thresholds (`25`/`35`) rather than authored components and are not persisted on `check_in_space_unit`;
corvette and raider `max_accel_mps2 = 60` and `delta_v_mps = 9_500`, NPC `4,500`, and `DRAY_P_HOLD_CAPACITY_T = 86`
remain hardcoded Rust constants (`corvette.json` authors no drive law); the 1,200 cr tow bounty pays at clamp before
delivery and can be collected repeatedly (`35c` §4 #6).

## SF.R16 — client docking game, per-session challenge and Neptune canopy (#353, `020e567c`)

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

**Divergences.** **(audit, 35c §3, §5, §6.10)** `tacsim/src/space/docking.rs`, `mush-pixel/src/docking.rs`, and
`web/src/lib/picture.ts` carry three separate copies of the 60 Hz kinematics; both TUI and web bind `c`/`Space` to
`play_to_clamp` (auto-solver) and `solve_docking_tape` is `pub` in `tacsim::space`; `sfr16` drives the client through
`play_to_clamp` rather than key input and departs Ceres from the berth via `+helm/autopilot` (`35c` §4 #2); the
Neptune/Triton canopy (`default_canopy_bodies_for_site`) is a static bearing table keyed on the target name rather than
computed from the `Sky` ephemeris at the hull's live state vector (`35c` §6.10); the contract lint in `arena_library.rs`
matched only the unqualified `module_state_mut::<SpaceBubbleBook>()` spelling (`35c` §5); and the commit's edits to
`doc 35` and `35-cut-notes.md` described an earlier draft table (`35c` §3, corrected in `35c` §8).

## SF.R17 — hole close-out II (#374, `7f64450a`)

- `helm_dock` (`crates/tacsim/src/space/commands.rs`) requires an existing `DockingSession` on `DOCK ABORT` / `abort`
  (`"No active docking session to abort."`), debits `rcs_spent_kg`, persists space state on `OutOfGate`, refuses a
  second `+helm/dock` while a session is already open in `docking_sessions` without resetting `started_wall_s`, and
  `unpause_hull_after_docking` and `expire_stale_docking_sessions` (`crates/tacsim/src/space/tick.rs`) never call
  `mush_sim::resume_carrier_journey` (`35c` §4 #1, #7).
- `helm_autopilot` (`commands.rs`) refuses from `Berth::Docked` (`"Ship is docked at a berth; undock first (+ship/undock)."`),
  and `+ship/undock <cluster|body>` (`prepare_departure` in `crates/tacsim/src/port/voyage.rs` via
  `resolve_space_undock_destination`, `plan_space_undock_on`, and `promote_departed_hull_to_space_free_flight`) runs
  all berth departure and `hold_back` checks, submits `tacsim_port_burn`, and promotes the hull into
  `SpaceFreeFlightRecord` on `launch_step` with cluster arrival opening an `ActiveClusterMiningSession` on `space_tick` (`35c` §4 #2).
- Removed the `.min(live_tank_t.max(1))` and `.min(basis.locker)` clamps from `helm_autopilot`'s outer-system expedition
  branch so `+helm/autopilot <body_id>` refuses when tank or locker is short (`35c` §4 #3).
- `expire_stale_docking_sessions` calls `debit_hull_propellant_kg(ctx, hull, 15)` so crossing a whole-tonne boundary
  submits a `PendingSpaceBurn` (`tacsim_port_burn`) (`35c` §6.10, §9 `SF.R17`).
- `BubbleCarrierEntry::holding_for_salvage_until_wall_s` (`SALVAGE_HOLD_TIMEOUT_WALL_S = 60`) bounds per-hull salvage
  holds, and `clear_resolved_bubbles` (`tick.rs`) lets any ready carrier in a multi-carrier bubble where
  `is_combat_clear()` holds detach and resume its `CarrierJourney` immediately without waiting for another carrier's
  `holding_for_salvage` (`35c` §4 #8).
- `validate_docking_initiation_gates` (`commands.rs`) evaluates physical terminal-braking range (`<= 500 m`) and
  relative speed (`<= 15,000 mm/s`) on the port-at-leg-end branch (replacing `remaining_s > 60`) and removes the
  `dist_km <= 2 -> 200 m` clamp on the free-flight branch (`35c` §4).
- `step_active_bubbles` (`tick.rs`) steps active bubbles by `min(elapsed_ms, MAX_BUBBLE_TICK_STEP_MS)`
  (`MAX_BUBBLE_TICK_STEP_MS = 30_000`) and skips stepping when `wall_now_s <= last_stepped_wall_s` (`35c` §6.6).
- Hardened `tacsim_integration_tests_never_mutate_space_bubble_book_directly` (`crates/mush-contract-tests/tests/engine_contracts/arena_library.rs`)
  to forbid qualified/unqualified/aliased `module_state_mut` on space books, `clear_interdiction_bubble(`,
  `apply_bubble_command(`, `solve_docking_tape(`, and `solve_hard_bounce_tape(` across `crates/tacsim/tests/`, and
  updated `sfr5`, `sfr6`, `sfr7`, `sfr8`, `sfr9`, `sfr12`, `sfr15`, and `sfr16` (`35c` §5, §9 `SF.R17`).

**Test.** `sfr17_hole_closeout_ii_abort_undock_expedition_dock_gates_salvage_hold_and_30s_tempo`.

**Divergences.** **(audit, 35d)** The PR body records none; the review found these against `35c` §9's row:
- The cluster departure attaches an arbitrary corridor route to the voyage, and a later `+helm/autopilot <port>`
  recouples onto a one-leg route that is never interdicted: a customs bypass (`35d` §2 #1, the second half of
  `35c` §4 #2). `SF.R25` changed the recouple (`engage_port_autopilot`, `owed_interdicted_leg`); the departure
  still carries the arbitrary route.
- `+ship/undock <cluster|body>` records `prepaid_unflown_t = plan.burn_t` and nothing drew it down, so a divert
  was free (`35d` §2 #5). Closed in `SF.R25` by `SpaceFreeFlightRecord::prepaid_credit_t`.
- The re-initiation refusal holds, but `+helm/dock abort` with no pulse fired costs nothing and a new session may
  open at once; while any carrier had a session open the whole bubble's step was skipped (`35d` §2 #4, §3 #7).
  `SF.R25` holds only the docking hull (`docking_hold`); it adds no cooldown or charge.
- A leg-end `+helm/dock <port>` promotes the journey, and an abort or expiry leaves it Promoted until the hull
  docks or decouples (`35d` §3 #1).
- The expedition branch refuses when the tank is short, but first re-plans at 250‰ and then 180‰ throttle,
  below the `D-19-9` floor (`35d` §3 #3).
- The per-hull salvage hold exempted a clamped hull, so clamp-and-never-tow kept the bubble and its activation
  slot open (`35d` §3 #8). Closed in `SF.R25`.
- The leg-end dock branch still derives range and speed from `remaining_s` (a terminal-braking model); only the
  free-flight branch measures them (`35d` §3, P1 row).
- The tempo step is `min(elapsed, MAX_BUBBLE_TICK_STEP_MS)` with the excess dropped, not carried over; the source
  is `World::now()` in whole unix seconds, and a backwards step freezes the bubble until the wall clock catches
  up (`35d` §6.5). A frozen bubble repaying its frozen time as one 30 s step was fixed in `SF.R25`.
- `sfr17`'s 30 s late tick equals the cap, so nothing tests above it; the docstring's free-flight refusal at
  1 km is never run; the 1 t Neptune tank is never checked; the picket is disabled with every `+tac/beam` output
  discarded (`35d` §5).

## SF.R18 — the multi-carrier bubble (#376, `c0d123a9`)

Built on doc 40's FR.6 (#363, `fe212f4a`), which landed first (Q-35c-1): FR.6 deleted `SpacePartyId` and
`SpaceRelation` for the arena's `PartyId` and `Relation` + `Mark` (`admitted_party`, `set_party_relation`), and made
`Inspecting` a bubble-side flag on the picket's party (`set_inspecting`), keeping the persisted wire.

- Assigned each carrier in `ActiveInterdictionBubble::carriers` (`crates/tacsim/src/space/tick.rs`) its own distinct
  arena `PartyId` (`CARRIER_PARTY` for the primary carrier, `ActiveInterdictionBubble::next_available_carrier_party`
  for subsequent carriers joining the domain bubble) with `Friendly` inter-carrier relation and per-carrier relation
  entries against `CUSTOMS_CORVETTE_PARTY`, `BELT_RAIDER_PARTY`, and `INTERDICTOR_PARTY` (`35c` §4 #4, §9 `SF.R18`, `Q-36-48`, `doc 40` `FR.6`).
- Added `KirkwoodPicketState::carrier_inspection_status` and `carrier_inspection_outcome` (`crates/tacsim/src/space/picket.rs`)
  with per-carrier helpers (`inspection_status_for`, `inspection_outcome_for`, `set_inspection_status_for`,
  `set_inspection_outcome_for`, `is_carrier_customs_cleared`), and updated `helm_submit` and `helm_run`
  (`crates/tacsim/src/space/commands.rs`) and `step_kirkwood_picket_for_carriers` (`picket.rs`) so one carrier's
  `+helm/submit` or `+helm/run` updates only that carrier's inspection status and Corvette party relation (`35c` §4 #5).
- Evaluated `is_combat_clear`, `is_carrier_clear_to_resume`, `is_clear_to_resume`, `clear_resolved_bubbles`, and
  `settle_disabled_carrier_bubbles` (`tick.rs`) per carrier so a disabled escort carrier is settled into
  `SpaceBubbleBook::derelicts` (`JourneyState::Failed`) and checked in immediately while an operational cleared
  carrier remains in the bubble until combat clears and then resumes its `CarrierJourney` (`JourneyState::Cruising`) (`35c` §4 #4).
- Checked in every participating carrier and NPC contact (`interdictor_hull`, `corvette_hull`, `raider_hull`) across
  all bubble exit paths (`clear_interdiction_bubble`, `clear_resolved_bubbles`, `settle_disabled_carrier_bubbles`, and
  `helm_dock` port/derelict clamps), updated `SpaceBubbleBook::remove_carrier_from_active` to snapshot the docking
  hull's own `SpaceShipState` before removal, and stood down the domain activation when the last carrier leaves (`35c` §4 #4, §6.3, §9 `SF.R18`).
- Keyed `SpaceBubbleBook::cleared_interdiction_legs` by `(hull, leg_cursor)` (`BTreeSet<(ObjId, u32)>`, cleared on new
  departure in `launch_step`) so a carrier that cleared an interdicted leg is never re-promoted into that leg's domain
  when a subsequent hull arrives (`35c` §6.4).

**Test.** `sfr18_multi_carrier_bubble_per_carrier_party_inspection_escort_disabled_settlement_and_domain_stand_down`.

**Divergences.** **(audit, 35d)** The PR body records none; the review found:
- The interdictor's stand-down is still decided for every carrier at once, off a trace string
  (`INTERDICTOR_STAND_DOWN_CLEARED`) and the first-ranked hostile alone (`35d` §3 #5, §6.6). Owed in `SF.R29`.
- `+helm/decouple` on the first leg of `ceres_stickney` promoted the free-flyer into the Kirkwood domain and
  nothing resumed it, so that domain never stood down and kept one of `MAX_ACTIVE_DOMAINS` (`35d` §6.2). Closed
  in `SF.R25` (`any_bubble_journey_promoted_in`; `sfr18` gains the fourth hull).
- `sfr18` mints 51 t of contraband; nothing shows who disabled whom (60 empty ticks); the "not re-promoted" check
  looks vacuous, the miner being a leg past the waypoint (`35d` §5, inferred there).

## SF.R19 — one salvage path (#386, `0c1408c6`)

What SF.R19 built was changed by two programme cuts that landed after it: doc 41's RS.8 (#439, `f4ffa532`) and
doc 42's CR.6 (#453, `77de4a64`, stacked on RS.8). Their as-built records are in `35c` §12.1 and §12.2; the
bullets below say where each one changed this cut.

- Unified space derelict salvage with `GC.6`'s economy-contract salvage claim path (`mush_economy::file_salvage_claim`,
  `mush_economy::award_salvage_title`, `mush_economy::close_salvage_claim`, and `SalvageCustody`) (`Q-36-46`, `35c` §4 #6, §9 `SF.R19`).
  RS.8 made the tow put the derelict in the tug's custody (a `SalvageCustody` row, `ClaimSeam::Owed`) and file through
  `+salvage/claim`'s own code (`crate::surface::claim::tow_files_salvage`), with the title and witnesses pinned at the
  clamp (`pin_at_clamp`) and the rows held per recovery domain in `SpaceCustodyBook`, persisted as
  `tacsim:space:custody:{domain}`.
- Removed auto-filing of salvage claims on disablement in `clear_resolved_bubbles` and `settle_disabled_carrier_bubbles`
  (`crates/tacsim/src/space/tick.rs`): disablement records the open-epoch `SpaceDerelictRecord` with `salvage_title_status: None`
  until a tug clamps (`+helm/dock`) and attaches a tow line (`+helm/tow`).
- Added `real_now_secs(w)` (`w.now().max(1)`, never dividing by `1_000` or `now.at / 24`), `salvage_claim_window_secs(w)`
  (today `[tacsim.recovery].claim_window_ic_days` at the world's rate, through
  `crate::surface::claim::claim_window_real_secs`), and `ensure_space_hull_titled` (`crates/tacsim/src/space/custody.rs`), and
  issued `EconomyStore` titles on spawned NPC picket/raider/interdictor hulls (`picket.rs`, `tick.rs`) (`35c` §6.4).
  RS.8 retired the derelict's own contest countdown; life support keeps its 48 h as `DERELICT_LIFE_SUPPORT_S`.
- Enforced single-claimant collar clamp and tow gates across `validate_docking_initiation_gates`, `helm_dock`, and
  `helm_tow` (`crates/tacsim/src/space/commands.rs`) so two ships racing for the same derelict cannot double-clamp or
  double-tow it, and added `+helm/tow release` to release a tow line and collar clamp.
- Moved the Freeport Registry salvage recovery bounty from deep-space tow attach to port delivery
  (`deliver_active_tow_at_port` and `step_delivered_tows_at_port` in `tick.rs`) with an idempotency set so a
  release and re-tow never paid twice (`35c` §4 #6). **RS.8 deleted the bounty**: the 1,200 cr payout, its contract,
  its constants and the idempotency set are gone. `deliver_active_tow_at_port` now checks the derelict in, docks
  her at the tug's port, appends the black-box `EpochClosedInBay` entry and delivers a retrieval the tug's helm
  holds (`crate::surface::retrieval::deliver`).
- Wired `SpaceBubbleBook::space_custody` into `surface::salvage::custody_of` and `surface::claim` (`+salvage`,
  `+salvage/inspect`, `+salvage/report`, `+salvage/contest`, and `claim::tick`) and cleared `SpaceBubbleBook::derelicts`
  when a salvaged hull is repaired at a port workshop (`crates/tacsim/src/port/bay_work/mod.rs`).
- CR.6 added the crew to the space check-in (`check_in_space_crew` at the end of `check_in_space_unit`) and the
  derelict clamp's capture (`capture_at_hard_dock`): crew aboard a derelict hard-docked by a non-friend are taken
  captive into the tug, and the `+helm/dock` port clamp hands them over (`crate::field::crew::hand_over`).

**Test.** `sfr19_one_salvage_path_unifies_space_derelict_claims_with_gc6_and_economy` (rewritten by RS.8 as its
acceptance; RS.8 also added `rs8_a_space_retrieval_job_is_delivered_by_the_tug_that_docks_her`, and CR.6
`cr6_a_crewed_derelict_hard_docked_by_a_stranger_is_captured_and_handed_over_at_port`).

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- `+helm/tow release` removed the tow and the clamp only; the custody row and its filed claim stayed, and the claim
  tick awarded on the window alone, so a released derelict could still be titled to the tug (`35d` §2 #3). Closed
  in `SF.R25` (`withdraw_undelivered_custody`; the claim tick awards a space custody row only once she is docked).
- There is no physical tow: a towed derelict "lands" at whatever port the tug next docks at, from any distance,
  and adds nothing to the tug's mass, tank or Δv; there is no load act and no capacity check beyond twice the tug's
  dry mass (`35d` §2 P1 line, §6.3). Owed in `SF.R28`.
- The claim's base is the first lane port's host whatever port the tug docks at, and every in-space custody row is
  filed under the ground site `freeport` (`35d` §6.3).
- A refused `check_out` still falls back to an uncustodied copy, and a check-in with no marker clears `deployed`
  and drops the damage (`35d` §6.3).
- `+helm/dock derelict` still writes `Promoted` with a `DomainId(1)` fallback (`35d` §6.3).
- "A repaired hull manoeuvres again" and the attach/tow/load capacity check are absent from `sfr19`; every clamp is
  `play_to_clamp` on a challenge read out of the book (`35d` §5).

## SF.R20 — one damage table and canonical drives (#395, `9dedc1c0`)

- Bridged `ShipSectionId`, `ShipSectionState`, and `SpaceShipState` onto `crate::carrier_damage::CarrierPart` and
  `CarrierDamage` (`Q-36-47`, `D-35-1`, `crates/tacsim/src/space/bubble.rs`), and routed `Slow` consequence capping through
  `crate::arena::damage::apply_slow_consequence` (`crates/tacsim/src/arena/damage.rs`) so `Slow` takes `min` instead of
  compounding across hits after `hold` reaches `0` internals (`35c` §6.5).
- Persisted space subsystem crits (`PdcOffline`, `SensorDegraded`, `RadiatorDamaged`, `DriveDegraded`), stripped PDC mounts,
  and `slow_pct` across `check_in_space_unit` and `space_ship_from_tactical_unit` via the `tacsim.space.subsystems`
  capability (`crates/tacsim/src/space/custody.rs`), and restored `tacsim.space.subsystems` on workshop bay repair
  (`crates/tacsim/src/port/bay_work/jobs.rs`) (`35b` §5 #16, `35c` §6.5).
- Authored `"drive": { "class": "patrol_corvette", "law": { "flip": { "accel_mg": 500, "exhaust_kms": 12000 } } }` in
  `crates/tacsim/content/units/corvette.json` and re-signed its content digest; computed `max_accel_mmps2`, `max_accel_mps2`,
  and `delta_v_mps` from `UnitSpec` and the Tsiolkovsky rocket equation across `SpaceShipState::new_dray`, `new_corvette`,
  and `apply_unit_spec_to_space_ship` (`bubble.rs`, `custody.rs`), removing the `60` $\text{m/s}^2$, `9_500` $\text{m/s}$,
  `4_500` $\text{m/s}$, and `DRAY_P_HOLD_CAPACITY_T = 86` constants (`mining.rs`, `commands.rs`).
- Added `thrust_mmps2` and `max_accel_mmps2` (`mm/s²` kinematics) on `SpaceShipState`, `LocalSpaceBubble::command_burn_mmps2`,
  and `BubbleInputCommand::CommandBurnMmps2`, and wired `+helm/thrust`, `+helm/burn`, and `+helm/approach` to scale throttle
  permille against `max_accel_mmps2` so an empty `Dray` and an `80 t`-loaded `Dray-P` accelerate at distinct sub-$1\text{ m/s}^2$
  flip-drive rates (`35c` §6.5).
- Replaced `raiders().first()` and `RAIDER_ATTR` in `spawn_kirkwood_picket_ships_in_world` (`crates/tacsim/src/space/picket.rs`)
  with a dedicated space raider unit (`tacsim.unit.raider_heavy`), added `reuse_or_create_npc_hull` and non-derelict NPC
  `Thing` cleanup in `clear_resolved_bubbles` (`crates/tacsim/src/space/tick.rs`), and hydrated `SpaceShipState::pixel_model`
  directly from `SpaceUnitOverlay::pixel_model` (`35c` §6.4–§6.5).

**Test.** `sfr20_one_damage_table_and_canonical_drives_sub_1mps2_kinematics_pdc_checkin_and_no_leaked_npcs`.

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- `SpaceShipState::apply_damage` is still a private proportional splitter; only the Slow consequence and the
  disable test are shared with the arena, and section choice does not go through `choose_section` (`35d` §6.4).
- Slow is inverted on bubble entry (the entry path multiplies by `100 − slow_pct`, while `recompute_drive_accel`
  reads `slow_pct` as the surviving share), and it still compounds: every Slow pushes `DriveDegraded`, which halves
  the cap again in both burn paths and survives a restart as a damaged `power` component (`35d` §6.4).
- PDC and sensor losses are still hard thresholds, not authored components, though they now persist (`35d` §6.4).
- `ceil` on the rating survives in three places; NPCs and free flight still use whole m/s²; the interdictor still
  burns a fixed vector (`35d` §6.4). Only player thrust in a bubble is in mm/s².
- There is no space raider unit file: `tacsim.unit.raider_heavy` is a Rust overlay over `tacsim.unit.corvette`
  (`custody.rs`, named from `content/space/picket_cast.json`), and `hull_hold_capacity_kg` falls back to the Dray-P's
  capacity (`35d` §6.4).
- Each promotion still creates new NPC Things with location `None` and new economy titles, which the close freezes
  (`35d` §6.2).
- `sfr20` sets ROYALTY with a direct `world_mut()` flag edit and mints 80 t; the rear split is an NPC's hits checked
  by inequality; "still stripped after check-out" and Slow-`min` are direct library calls on a copy (`35d` §5).
- All of this is owed in `SF.R28`.

## SF.R21 — the prepared bubble mechanic and the per-entity store (#401, `3fb3e861`)

- Added the prepared bubble input action (`BUBBLE_INPUT_ACTION = "tacsim_space_bubble_input"`, outcome
  `BUBBLE_INPUT_OUTCOME = "tacsim_space_bubble_input_outcome"`, typed family `BUBBLE_TAPE_FAMILY = "tacsim.space_bubble_tape"`)
  via `BubbleInputProvider` and `submit_bubble_input` (`crates/tacsim/src/space/tick.rs`, `crates/tacsim/src/port_tick.rs`),
  and wired in-bubble `+helm` (`/heading`, `/thrust`, `/emcon`, `/roll`, `/submit`, `/run`) and `+tac` (`/lock`, `/beam`,
  `/torp`, `/railgun`, `/pdc`) commands (`crates/tacsim/src/space/commands.rs`) to submit `tacsim_space_bubble_input` via
  `Ctx::submit_authoritative` (`D-35-9`, `35b` §3b, `35c` §4b, §6.1).
- Moved `StandingOrder` persistence out of `SpaceBubbleBook::standing_orders` onto `mush-sim`'s `StandingOrderBook`
  (`mush_sim::tick::standing_order`, `record_standing_order`, `clear_unit_standing_orders`, `standing_orders_snapshot`,
  and `restore_standing_orders` in `crates/mush-sim/src/tick.rs`), and wired `step_active_bubbles` (`crates/tacsim/src/space/tick.rs`)
  to read `StandingOrderBook` and apply `OrderChannel::Thrust` (`Scalar { value: 0 }` heave-to) and `OrderChannel::Posture`
  (`Emcon`) on each tick (`D-35-9`, `35c` §6.1).
- Replaced the monolithic `tacsim:space:state` write with dirty-only per-entity keys (`tacsim:space:bubble:{domain}`,
  `tacsim:space:flight:{hull}`, `tacsim:space:derelict:{hull}`, `tacsim:space:docking:{hull}`, and
  `tacsim:space:ledger:{hull}`) backed by `#[serde(deny_unknown_fields)]` wire structs (`PersistedBubbleDoc`,
  `PersistedFlightDoc`, `PersistedDerelictDoc`, `PersistedDockingDoc`, `PersistedLedgerDoc`) and fail-closed quarantine
  (`quarantined_bubbles`, `quarantined_hulls`) on decode failure (`crates/tacsim/src/space/tick.rs`,
  `crates/tacsim/src/store.rs`) (`35c` §6.2). What the per-entity keys do not carry stays in the auxiliary
  `tacsim:space:state` (`SPACE_STATE_KEY`, the `SpacePersistedState` document: the rest of the book, the salvage book
  and the standing-order book); RS.8 later added `tacsim:space:custody:{domain}` (`PersistedCustodyDoc`).
- Bounded `SpaceBubbleBook::completed` (`MAX_COMPLETED_BUBBLES` = 8), `pending_space_burns` (drained oldest-first
  to 16), `propellant_ledgers` (64), and `SalvageClaimBook::black_box_tapes` (`<= 64` hulls in
  `crates/mush-tactics/src/field_ops/salvage.rs`), and unified propellant debits (`debit_hull_propellant_for_dv` and
  `debit_hull_propellant_kg`) so in-bubble `SpaceShipState::propellant_kg` and `HullPropellantLedger` stay synchronized
  (`35c` §6.2, §6.10). The bounty idempotency set this cut also capped was deleted by RS.8 with the bounty.
  `pending_space_mine_draws` has no count cap; it is pruned as draws land.

**Test.** `sfr21_prepared_bubble_input_mush_sim_standing_orders_and_per_entity_store_with_quarantine`.

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- The `authoritative` label is still a relabel (`35d` §4). Every in-bubble verb changes the bubble in its handler
  through `apply_bubble_command` and then calls `submit_bubble_input`; `+helm/submit|run` move the fine, the front
  and the relation in the handler; `+helm/dock`, `+helm/tow`, `+tac/hail` and `+mine/ping|laser` submit nothing;
  all three dispatchers still end in `persist_space_state`.
- `submit_bubble_input` returns without submitting outside an active bubble and when the actor already has an
  authoritative action pending; the prepare checks only that the pending entry matches the invocation, appends a
  row to `tacsim.space_bubble_tape` (which the bubble never reads and nothing deletes) and stages drone records. It
  re-reads no seat, location or detection (`35d` §4, §6.1).
- The input sequence (`next_bubble_input_seq`) is RAM-only and restarts at 1 under the idempotency key
  `tacsim.space.input/{hull}/{seq}/{switch}` (`35d` §4).
- The tick reads the standing-order channels for Posture and for Thrust at or below zero only; heading and positive
  thrust are still the handler's. The private map is deleted (`35d` §4).
- There is no dirty flag: `persist_space_state` clones the book, the salvage book and the standing-order book on
  every tick and every command and diffs them against the last write. The auxiliary document is unbounded
  (`derelict_pins`, `cluster_tenure_orders`, the salvage and standing-order books) and is rewritten whole every tick
  while any hull mines; a derelict's countdown writes once a second for 48 h; `deny_unknown_fields` is on the
  envelopes only (`35d` §6.1).
- A quarantined bubble is not fenced against the next interdiction at that domain; the `ObjId`-order caps can evict
  a live hull's ledger or burn (`35d` §6.1).
- `sfr21` has no "a gunner's beam is one submission" case; the heave-to has no before/after contrast; the
  dock-open assertion ORs two strings, one of which a refusal also matches (`35d` §5).
- The mechanic is re-cut in `SF.R26` and the store budget in `SF.R27`.

## SF.R22 — authoring II: clusters from the pack, sealed minds, drone records (#402, `1801b880`)

Built against `35c` §9's `SF.R22` row (`D-35-17`), stacked on #401.

- **Clusters.** The `OrbitalClusterId` enum is gone. A cluster is an `OrbitalClusterDef` (id, label, zone, anchor
  body, rocks as `ClusterRockDef`) in `LocationDef::clusters` (`crates/tacsim/src/pack.rs`), validated when the pack
  loads (`place_clusters_fault`), so a malformed cluster fails `load_pack`. All four are authored on
  `kirkwood_depot` in `crates/tacsim/content/lane/ceres_phobos.json`; `all_orbital_clusters` (`space/mining.rs`)
  resolves them from the store's packs as `ResolvedOrbitalCluster`s.
- **The picket cast.** The corvette, raider and interdictor cast is `crates/tacsim/content/space/picket_cast.json`
  (`PicketCastBook`, read through `picket_cast_book()`), compiled in with `include_str!` behind a `LazyLock`.
- **Sealed minds.** `run_space_rhai_think_with_seal` (`space/picket.rs`) installs the unit's sealed runner through
  `crate::minds::sealed::install_sealed_runners` and runs the think on `MindPool::shared`; the private runner is
  gone. NPC hulls and drones are sealed by `seal_space_unit_in_world`.
- **Drone records.** `+mine/drones` seals each drone (`drone_mining`), runs one think (`run_drone_rhai_think`), and
  stages one `RigRecord` per drone in `tacsim.excursion` through `submit_drone_excursions`, the
  `tacsim_space_bubble_input` action's prepare. The yield is still the `DroneSortieState` timer in the cluster
  session (`DRONE_SORTIE_IC_SECS` = 120, `DRONE_SORTIE_YIELD_T` = 4).

**Test.** `sfr22_authoring_ii_clusters_from_pack_sealed_minds_and_drone_excursions`.

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- Clusters are not doc 20 §17 field-kind sites, and all four sit on one host. When no applied pack has clusters,
  `all_orbital_clusters` falls back to the compiled-in lane and re-parses it on every call (per hull per tick); a
  load failure does not refuse boot, and a world cannot author clusters away (`35d` §6.6).
- Both live think paths pass `store = None`, so `TacsimStore::mind_source` is never read and only compiled-in
  digests resolve; there is no MindPool cadence, each think being a synchronous wait on the world thread every tick,
  per ship (`35d` §6.6).
- The interdictor's lock, speed and fire intents are not honoured; its stand-down is keyed off the
  `INTERDICTOR_STAND_DOWN_CLEARED` trace string (`35d` §6.6).
- The picket cast is not in the pack: no digest, silent on a parse failure (`.ok()`), with Rust fallbacks and GOD
  as the fallback commander (`35d` §6.6).
- Drones are not excursion sorties: the `RigRecord`s are inserted and never closed, the yield is the fixed timer,
  the excursion id is hand-built, and the mind's intents are discarded (`35d` §6.2, §6.6). The PR body's
  "excursions" overstates this (`35d` §7).
- `sfr22` opens no second cluster on another body and has no interdictor bubble, so "stands down on heave-to" is
  untested; it calls `picket_cast_book()` and `all_orbital_clusters()` directly (`35d` §5).
- Minds are owed in `SF.R29`; sites and drones in `SF.R30`.

## SF.R23 — mining and sensing remainders (#404, `d3a30948`)

Built against `35c` §9's `SF.R23` row (`35b` §5 #15, `D-35-10`, `D-35-7`), stacked on #402. Paths are under
`crates/tacsim/src/space/`.

- **The clerk.** `rock_uncommitted_pool_tonnes` (`mining.rs`) subtracts unlanded `pending_space_mine_draws` from a
  rock's pool, `next_space_mine_work_row` clamps a draw to what is left, and exhausted draws are pruned, so two hulls
  taking 4 t each from a 5 t rock both finish. `resolve_cluster_host_room` picks the room for the cluster clerk,
  block places and deposit openers.
- **Sessions.** `ActiveClusterMiningSession` records `last_activity_ic_s` and an idle session expires after
  `CLUSTER_SESSION_IDLE_TIMEOUT_IC_SECS` (7 IC days) into free flight. `+mine/laser` and `+mine/drones` need a Dray-P
  (`is_prospector`). Out of a cluster the refusals point to `+helm/autopilot <cluster_id>`.
- **Tenure and customs.** `actor_holds_cluster_block_claim` reads the claim book, so a holder mining her own staked
  block is not flagged for claim-jumping; `resolve_hull_customs_principal` makes the fine payer the hull's helm
  holder or title owner; `inspect_hull_manifest` checks `strategic_ore` per block against `mined_strategic_blocks`.
- **Hails and leaks.** `+tac/hail` is new: `queue_or_deliver_light_lagged_hail` (`picket.rs`) delivers at once or
  queues in `pending_light_lag_hails`, and `step_pending_light_lag_hails` delivers on the tick. `+helm/approach`,
  `resolve_docking_target`, `validate_docking_initiation_gates` and the holotank's torpedo target position are
  gated by `is_bubble_ship_detected`.
- **Linkdead.** `handle_crew_linkdead` and `expire_linkdead_grace_for_hull` (`tick.rs`) cover cluster sessions,
  and the grace expiry runs `execute_linkdead_return_journey`. `build_tier1_space_panel_payloads` fills the Tier-1
  feed's Δv, heat and hull from the hull, and cluster sessions get the tick's panel push.

**Test.** `sfr23_mining_and_sensing_remainders`.

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- The linkdead return recoupled onto a hand-built route with a fixed hour-long leg, an origin of any other open
  port, and no Δv, tank check or debit: a free trip home by dropping the link (`35d` §2 #2). `SF.R25` sends it
  through `engage_port_autopilot` (the planner, the tank check, the debit); a hull that can afford no port stays.
- `+tac/hail` took any aboard player, in or out of a bubble, with a typed `range_m=`, onto an uncapped persisted
  queue; light lag applied only from one second (300,000 km), so every in-bubble hail was immediate and the only
  queued one was the player's own range (`35d` §2 P1 line, §6.8). `SF.R25` removed `range_m=`, measures the range,
  and caps the queue (`MAX_PENDING_HAILS_PER_HULL`). Q-35d-1 is open.
- The dock gate skips an undetected operational hostile where it should refuse without naming it, while the
  ordnance check still refuses on undetected torpedoes (`35d` §2 P1 line). `SF.R25` changed the hostile gate.
- The clerk is still head-of-line for a draw that fails for another reason; clerks are created as GOD and the host
  room falls through to the first placed object of any applied pack and then `ObjId(0)`; rates are unchanged
  (4 t per 60 s laser, 4 t per 120 s drone); bought or transferred `strategic_ore` falls back to "any one claim";
  any fine error still seizes, and clearance is granted before the seizure lands (`35d` §6.7). The PR body's clerk
  placement and its "delivered at now + d/c" overstate the tree (`35d` §7).
- `sfr23`'s hail distance is the player's own `range_m=`; the torpedo mask is a hand-built off-engine bubble that
  asserts a key the feed never emits; the ghost-dock assertion passes on any refusal (`35d` §5). `SF.R25` changed
  step 7b.
- The rest is owed in `SF.R30`.

## SF.R24 — one docking kinematics and the canopy (#409, `7a535806`)

Built against `35c` §9's `SF.R24` row (`D-35-14`, `D-35-15`).

- **One kinematics.** `mush_pixel::docking::step_docking_kinematics_60fps` is the one 60 Hz integrator. The server
  (`crates/tacsim/src/space/docking.rs`) steps through it, the TUI (`crates/mush-net-ssh/src/tui/pixel.rs`) runs
  `mush_pixel::docking::DockingClientSession`, and the web client steps the session in `mush-pixel-wasm`
  (`mp_docking_step`, `mp_docking_state`) from `web/src/lib/picture.ts`; the TypeScript copy is gone.
- **No auto-solve key.** Neither client binds `play_to_clamp` to a key. `solve_docking_tape` and
  `solve_hard_bounce_tape` are `#[cfg(test)]`, and so is their re-export.
- **Canopy.** `compute_ephemeris_canopy_bodies` (`space/bubble.rs`) starts from `default_canopy_bodies_for_site` and
  re-bears it from the lane's `Sky` at the hull's position; the Tier-1 panel builder calls it, and the panels are
  pushed on the tick.

**Test.** `sfr24_one_docking_kinematics_and_ephemeris_canopy`.

**Divergences.** **(audit, 35d)** The PR body records none; against `35c` §9's row:
- No client key tape reaches the server in any test: `sfr24`'s "human key tape" is `next_guidance_input()` reduced
  to one key a frame, the "wasm" replay is the same Rust type twice, and no test pins that a client cannot reach the
  auto-solver. `play_to_clamp` and `next_guidance_input` remain `pub` in mush-pixel, which both clients compile
  (`35d` §5, §6.8). Owed in `SF.R31`.
- The canopy's body list is still the `default_canopy_bodies_for_site` table; the ephemeris rewrites the first
  body's and the sun's bearing only, a moon sits at a fixed offset, and inside a bubble the list is set once
  (`35d` §6.8). The expected value in `sfr24` comes from production's own function. The PR body's "computed from
  the live Sky ephemeris" overstates this (`35d` §7). Owed in `SF.R30`.
- The docking pose seed is still `seq + hull × 131` into fixed tables (`35d` §6.8).

## SF.R25 — hole close-out III (doc 35d §2, §3, §6.2)

Built against [doc 35d](35d-sfr17-r24-train-review.md) §9's `SF.R25` row, with Q-35d-2, Q-35d-3 and Q-35d-4 at
their defaults. Paths are under `crates/tacsim/src/`.

- **Customs (35d §2 #1).** `engage_port_autopilot` (`space/commands.rs`) is the one port recouple; `+helm/autopilot
  <port>` and the linkdead return both call it. It keeps the interdicted leg the journey carries, as before, and
  otherwise asks `owed_interdicted_leg`: the first interdicted leg on the lane's route from the port the hull last
  left (`Voyage.departed.from`) to the destination's venue, unless the hull has an entry in
  `cleared_interdiction_legs` since that departure. A hull that left by `+ship/undock <cluster>` therefore meets the
  picket on her way to a port across it.
- **The prepaid burn (35d §2 #5).** `SpaceFreeFlightRecord::prepaid_credit_t(now)` (`space/tick.rs`) prorates
  `prepaid_unflown_t` over the time left to `cluster_arrival_ic_s`; a record without an arrival instant (a
  `+helm/decouple`) keeps its share. The three autopilot branches read the credit, not the field.
- **The linkdead return (35d §2 #2, Q-35d-2).** `execute_linkdead_return_journey` tries the queued port and then every
  open port nearest first (`return_ports_nearest_first`, by range in the lane's sky from the hull's Tier-1 position)
  through `engage_port_autopilot`: the planner's passage, the tank and locker checks, the debit, the owed leg. A hull
  that can afford none stays where she is; the grace-expiry line says which happened. `nearest_return_port_for_hull`
  now reads the hull.
- **A released tow (35d §2 #3, Q-35d-4).** `+helm/tow release` calls `withdraw_undelivered_custody`: when the tug holds
  the derelict's space custody row and she is not docked, the claim's contract is closed through
  `mush_economy::close_salvage_claim` and the row removed; a refusal from the registry keeps the line on. The claim
  tick (`surface/claim.rs`, `adrift_in_space_custody`) awards a space custody row only once the derelict is docked.
- **A docking session (35d §2 #4, Q-35d-3).** `SpaceShipState.docking_hold` (`space/bubble.rs`), set from the open
  sessions on each bubble step, cuts the docking hull's drive as `crew_linkdead` does; the bubble is no longer skipped
  while a carrier docks. The hostile gate refuses on an operational hostile to the hull's party whether or not she
  has detected it, and names it only when she has (`DOCK_REFUSED_UNSECURED`).
- **A hail (35d §2).** `+tac/hail` takes no `range_m=`. In a bubble it needs a detected contact and measures the range;
  outside one, from free flight or a cluster, it needs an open port and measures the range in the lane's sky
  (`hail_range_to_port_m`). `MAX_PENDING_HAILS_PER_HULL` = 4 in flight per hull.
- **Leaks (35d §3 #8, §6.2).** The stand-down gate (`any_bubble_journey_promoted_in`) ignores the parked journey of a
  hull in free flight or at a cluster. A clamp sets a hold of `SALVAGE_HOLD_TIMEOUT_WALL_S`; the hold no longer
  exempts a clamped hull, and a clamp with no tow line is let go when it lapses. The per-tick prune drops the tow and
  clamp of a tug that no longer exists (withdrawing its custody), a derelict record whose hull no longer exists, a
  `derelict_pins` entry with neither a record nor custody behind it, and destroys an NPC hull whose derelict record
  lapsed with nothing holding it. `+helm/dock derelict` resolves only a derelict in the hull's own bubble's domain or
  the one she holds.
- **The port clamp (35d §6.1).** `dock_carrier_at_destination` runs before the free-flight record and the bubble are
  torn down, so a refused dock leaves the hull where she was.
- **Tempo (35d §6.5, one line of `SF.R29`).** A frozen bubble advances `last_stepped_wall_s`, so frozen time is not
  repaid as one long step.

**Tests.** `sfr25_hole_closeout_iii_customs_prepaid_linkdead_released_tow_docking_hold_and_hail` (new; parts A–F).
`sfr18_…` gains a fourth hull that decouples on the Kirkwood leg while the picket's domain is live, and still ends
with the domain stood down. Changed because they asserted the old behaviour: `sfr9_…` (the under-fire dock was passing
on the port-in-a-bubble refusal, the hostile being undetected), `sfr19_…` (a release now withdraws the claim and a
re-tow files a new one), `sfr23_…` step 7b (a typed `range_m=` is refused in a bubble; a hail to the corvette arrives
at once). Local: `cargo test -p tacsim --test domain_world` 255 passed; `cargo test -p mush-contract-tests` green;
clippy `-D warnings` and rustdoc `-D warnings` on tacsim clean.

**Divergences from 35d §9.**
- No cooldown and no charge on abort: with the session holding one hull, an abort loop stops nobody else.
- The session holds the docking hull, not "the pair": the derelict is already inert. The hull coasts; she is not
  frozen in place.
- The hostile gate is against the hull's own party's hostiles, not every hostile pair in the bubble.
- The cluster departure still carries an arbitrary corridor route; only the recouple was changed.
- The leg-end range gate is unchanged: a corridor journey has no position, and the terminal-braking model stands.
- A flight engaged by `+helm/autopilot <cluster|body>` still records no prepaid credit, so a divert from it is charged
  in full. Conservative; `SF.R26` can make the credit uniform.
- Q-35d-2's retrieval job for a drifting hull is not built.

**Not pinned by a story.** The award gate on an undelivered derelict (the release path removes the row first); the
lapsed clamp; the destroyed-tug prune; the reordered port clamp; "nearest first" among several affordable ports.

**Checked while building, not a hole.** Free flight is never interdicted (`promote_due_interdictions` skips it), so
a hull that never recouples might seem able to cross to a port under `+helm/burn` and dock from there. She cannot:
the free-flight branch of the dock gate (`validate_docking_initiation_gates`) takes only a hull whose flight record's
`target_place` is that port, and the only way to hold such a record is to decouple on the corridor leg into the port,
which lies past the interdicted leg.

## SF.R26 — the bubble input contract, re-scoped (doc 35d §9a, Q-35d-6)

Doc 35d §9's `SF.R26` row asked for every bubble verb's state change to move into a prepared action's apply. Read
against `mush_core`'s provider trait that is not a cut: `prepare_authoritative` sees typed families only, the bubble
is module state in RAM, and the ground arena takes its orders through its own queue. Doc 35d §9a sets out the two
readings and asks the owner which (Q-35d-6); the default is that in-bubble helm and tactical inputs are bubble orders
on the bubble's own tape.

Built, because it holds under either reading:

- `submit_bubble_input_with_drones` (`crates/tacsim/src/space/tick.rs`) floors `next_bubble_input_seq` on the wall
  clock (seconds × 1,000), so a sequence, and the idempotency key `tacsim.space.input/{hull}/{seq}/{switch}` built from
  it, never repeats one issued before a restart. Before this the allocator restarted at 1 with the server.

**Test.** `sfr21_…` reads the sequence and key of an input before its reboot and of one after, and requires the later
one to be greater and the keys to differ.

**Not built, waiting on Q-35d-6.** Under the default: file, debit, then grant for `+helm/autopilot`, `+helm/approach`
and the dock's RCS; one submitted action each for `+helm/submit`, `/run`, `/dock` and `/tow`; a decision to keep the
staged tape family as an audit trail or delete it. Under the other reading: the family-backed bubble. Also still as
doc 35d §4 found them: an input typed while the actor has an action pending is applied and not recorded on the staged
tape, and `+mine/drones` in that state adds the drones without staging their `RigRecord`s.

## SF.R27 — the store budget, a first part (doc 35d §6.1)

A subset of doc 35d §9's `SF.R27` row: the steady-state writes and the unbounded or unsafe collections that could be
fixed without the dirty-flag rewrite, which waits on Q-35d-6. Paths are under `crates/tacsim/src/space/`.

- **A cluster session is its own document.** `tacsim:space:cluster:{hull}` (`SPACE_CLUSTER_PREFIX`,
  `space_cluster_key`, `PersistedClusterDoc`, `tick.rs`), written and deleted by `persist_space_state` as a flight
  record is, read by `load_space_kv_entry` with the identity rule and per-hull quarantine, and cleared from the
  auxiliary copy by `without_per_entity_maps`. A hull at a cluster drifts every tick; before this each tick rewrote
  the whole `tacsim:space:state` document, salvage book and standing orders included. An auxiliary document written
  before this cut still carries its sessions and is read as before.
- **A derelict's window is stepped by the minute.** `SpaceDerelictRecord.life_support_checked_wall_s` and
  `DERELICT_WINDOW_STEP_WALL_S` = 60: `step_derelict_records` subtracts real elapsed seconds a whole step at a time,
  so the record is written once a minute, not once a second, a stalled tick or a restart loses nothing, and frozen
  time is not counted. Before this it fell by one a tick, whatever time had passed.
- **A mining session that did not move keeps its stamp** (`mining.rs`), so a matched or stationary hull writes
  nothing.
- **The ledger cap** (`MAX_PROPELLANT_LEDGERS` = 64) takes only a ledger with no unlanded and no part-tonne debit
  whose hull is in no bubble, flight or cluster session; with none such it takes nothing. Before this it took the
  lowest hull id, live or not.
- **A tenure order lapses** after `CLUSTER_TENURE_ORDER_TTL_IC_S` (one IC day); `cluster_tenure_orders` was never
  pruned.
- **A quarantined domain takes no new bubble** (`promote_due_interdictions`): the interdiction is skipped and
  logged, so nothing is written over the quarantined `tacsim:space:bubble:{domain}` document.

**Tests.** `sfr21_…` counts the derelict key's writes: none across ten one-second ticks, one on the tick that
completes the minute, the window sixty seconds or more shorter. `sfr25_…` part F counts `tacsim:space:state` writes
over five ticks of a hull drifting at her cluster (none), then restarts and reads her session back from its own key.
`sfr8_…` now flies its second leg at the cruise rate: flown whole at rate 1 it is eleven real days, and the raider's
48 hours of life support rightly ran out. Local: `domain_world` 255 passed, contract tests green, clippy and rustdoc
clean on tacsim.

**Not built from the row.** No dirty flags: `persist_space_state` still clones and compares the whole book on every
tick and command. The bubble document is still rewritten every combat second. The auxiliary document still carries
the salvage book, the standing-order book, hails, tows, clamps and pins. `deny_unknown_fields` is still on the
envelopes only. The black-box and `pending_space_burns` caps still evict oldest-first. The staged tape family is
never deleted. A bad auxiliary or custody document still holds the whole space state.

**Not pinned by a story.** The ledger cap's rule, the tenure order's lapse, the quarantined domain's skipped
interdiction.

## SF.R28 — hull fidelity and the tow line, a first part (doc 35d §6.3, §6.4)

A subset of doc 35d §9's `SF.R28` row: the verified drive bugs and the tow's missing physics. The damage table and
the authored components wait (see below). Paths are under `crates/tacsim/src/space/`.

- **Slow is read the right way round at bubble entry.** `build_carrier_ship_state` (`tick.rs`) multiplied the hull's
  acceleration by `100 − slow_pct`; `slow_pct` is the surviving share, as `recompute_drive_accel` reads it. A hull
  that checked in at Slow 75 entered her next bubble at 25 %. The line now calls
  `SpaceShipState::slowed_accel_mmps2` (`bubble.rs`).
- **A Slow no longer also raises `DriveDegraded`** (`SpaceShipState::apply_damage`, and the check-out in
  `custody.rs`), which halved the drive a second time in both burn paths.
- **`+helm/burn` flies at the drive's true rating.** The rating for the hull's mass is taken in mm/s²; a burn asked
  above it is flown at it, and the line says so. It was rounded up to a whole m/s², so a Dray rated about 0.76 m/s²
  burned at 1, and a burn above the rounded figure was refused.
- **A derelict on the line is mass.** `towed_mass_kg` (`commands.rs`) adds the towed hull's dry mass to the payload
  the autopilot planners (port, cluster and body), `+helm/burn`'s rating and burn, and the carrier's bubble ship are
  computed from; `shift_bubble_tow_payload` changes the tug's bubble ship the moment the line locks or lets go.
- **The base a delivery closes at** (`resolve_nearest_port_base_obj`, `tick.rs`) is the port the tug is docked at; it
  was the lane's first port whichever port she docked at.
- **A unit that will not check out gets no bubble.** `build_carrier_ship_state` no longer falls back to an
  uncustodied copy of the unit, whose damage a check-in then dropped. `release_uninterdicted_carrier` resumes the
  journey and marks the leg cleared, at both call sites; before this the join path left a refused carrier parked in
  `Promoted`.

**Tests.** `sfr25_…` part D reads the tug's acceleration before the tow, with the derelict on the line (lower) and
after the release (as before). `sfr19_…` requires the black box's `EpochClosedInBay` base to be Stickney's host.
`sfr20_…` checks `slowed_accel_mmps2` against the Slow it has just taken. `sfr4_…`, `sfr9_…` and `sfr11_…` change
where they pinned the rounded rating: the burns now fly at the hull's rating and say so, and `sfr11_…`, whose
"burn above the rating is refused" was the rounding itself, orders day-long away burns until one would eat the
reserve. Local: `domain_world` 255 passed, clippy clean on tacsim.

**Divergences and limits.**
- Removing the `DriveDegraded` push changes little in play yet: a hit that takes a section to zero internals also
  crosses the hard 30-internal threshold that raises the same crit (doc 35d §6.4). The thresholds go with the
  authored components, which are not built.
- The tow's mass reaches the planners and the bubble ship, not a corridor journey already under way: a tug that
  resumes her leg with a derelict on the line flies the plan she filed. The return-reserve estimate
  (`compute_hull_return_reserve`) does not count the tow either.
- The derelict's own position still does not follow the tug; she is placed at the port when the tug docks.
- No load act and no bay or cradle capacity check beyond twice the tug's dry mass.

**Not built from the row.** Section choice through the arena's `choose_section`; PDC and sensor losses as authored
components; `ceil` on `max_accel_mps2` for NPC ships and the interdictor's fixed burn; a raider unit file; NPC hulls
reused per domain with one title each; space custody rows off the `freeport` ground site.

**Not pinned by a story.** The refused check-out and the carrier it releases.

## SF.R29 — tempo and the doctrines' sources, a first part (doc 35d §6.5, §6.6)

A subset of doc 35d §9's `SF.R29` row, and one place where building it changed the plan. Paths are under
`crates/tacsim/src/space/`.

- **The doctrines read the store.** `step_active_bubbles` (`tick.rs`) lifts the `SpaceBubbleBook` out of the world
  for the step and puts it back after, so the `TacsimStore` can be borrowed beside it. The picket
  (`step_kirkwood_picket_for_carriers_with_store`) and the interdictor (`step_interdictor_picket`) now pass that
  store down to `run_sealed_mind_on_pool`, where `crate::minds::content::source_lookup` tries the store's
  `mind_source` before the compiled-in sources. Both were passing `None`, so a doctrine sealed to a digest the binary
  does not carry never ran (doc 35d §6.6, `D-35-17`).
- **A clock that steps back re-anchors the bubble.** `last_stepped_wall_s` is set to the new time and the bubble
  steps again on the next tick. It used to wait, stepping nothing, until the wall clock passed the old value.

**The cap stays, on purpose.** Doc 35d §6.5 called the time a fire cannot step "lost" and §9 asked for it to be
carried over ("a 45 s stall advances 45 s"). Built that way, a stall was repaid as thirty seconds of bubble a tick
until the debt cleared: a fight fast-forwarded past crews who had no chance to act in it, and `sfr6_…`'s torpedo
exchange resolved before its pilot's next command. That is worse than the drop. One fire still steps
`min(elapsed, MAX_BUBBLE_TICK_STEP_MS)` and the rest of a stall is not simulated, as an outage is not. Doc 35d's
"true catch-up" is withdrawn.

**Test.** `sfr10_…` fires a tick 45 s late and requires 30,000 ms of bubble, then a tick a second later and requires
1,000 ms: stalled time past the cap is not repaid.

**Not built from the row.** A sub-second cadence while ordnance flies (the Registrar tick is still one second, so the
500 ms cadence is unreachable); a monotonic time source; `[tacsim].space_pulse_ms` (still parsed and inert); the
derelict approach on the tape; a think cadence off the world thread (each think is still a synchronous wait, every
tick, for every doctrine ship); the interdictor steering by its intents and standing down per carrier on state.

**Not pinned by a story.** The store-sourced doctrine (a story would need a doctrine sealed to a store-only source)
and the backwards clock (the test world's tick does not fire for a time earlier than its last).
