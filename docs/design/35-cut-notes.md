# 35. Cut notes — the seam-first space train

One `## SF.Rn` heading per cut of [35-ship-free-flight-combat-and-mining.md](35-ship-free-flight-combat-and-mining.md)
§10.2 (`SF.R0`–`SF.R8`) and §10.3 (`SF.R9`–`SF.R16`, from [35b-seam-first-train-review.md](35b-seam-first-train-review.md) §8),
in the order they landed: what was built, the story test that proves it, and where the PR departed from
the plan's row. The initial nine cuts (`SF.R0`–`SF.R8`) merged on 2026-11-02 and were reviewed in `35b`; the follow-on
remediation cuts (`SF.R9`–`SF.R16`) merged on 2026-11-02..03 and were reviewed in
[doc 35c](35c-remediation-train-review.md). The story tests live in
`crates/tacsim/tests/shard_10/voyage.rs` (moved from `shard_2/voyage.rs` in `44aac0fd`, PR #344) unless a section says otherwise.

Sources: the PR bodies of #297, #302, #309, #311, #313, #316, #317, #318, #321, #332, #333, #335, #336, #337, #338,
#339 and #353. A PR body rarely says where it
left the plan, so "Divergences" lists what the body states or what its test text shows against §10.2/§10.3's row;
lines marked **(audit)** come from the close-out audits in [doc 37](37-ic-phase-closeout-2026-11-02.md) §5.3,
[doc 35b](35b-seam-first-train-review.md), and [doc 35c](35c-remediation-train-review.md), not
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

## SF.R17 — hole close-out II

- `helm_dock` (`crates/tacsim/src/space/commands.rs`) requires an existing `DockingSession` on `DOCK ABORT` / `abort`
  (`"No active docking session to abort."`), debits `rcs_spent_kg`, persists space state on `OutOfGate`, refuses a
  second `+helm/dock` while a session is already open in `docking_sessions` without resetting `started_wall_s`, and
  `unpause_hull_after_docking` and `expire_stale_docking_sessions` (`crates/tacsim/src/space/tick.rs`) never call
  `mush_sim::resume_carrier_journey` (`35c` §4 #1, #7).
- `helm_autopilot` (`commands.rs`) refuses from `Berth::Docked` (`"Ship is docked at a berth; undock first (+ship/undock)."`),
  and `+ship/undock <cluster|body>` (`prepare_departure` in `crates/tacsim/src/port/voyage.rs` via
  `resolve_space_undock_destination`, `plan_space_undock_on`, and `promote_departed_hull_to_space_free_flight`) runs
  all berth departure and `hold_back` checks, submits `tacsim_port_burn`, and promotes the hull into
  `SpaceFreeFlightRecord` on `launch_step` with cluster arrival opening `ActiveClusterSession` on `space_tick` (`35c` §4 #2).
- Removed the `.min(live_tank_t.max(1))` and `.min(basis.locker)` clamps from `helm_autopilot`'s outer-system expedition
  branch so `+helm/autopilot <body_id>` refuses when tank or locker is short (`35c` §4 #3).
- `expire_stale_docking_sessions` calls `debit_hull_propellant_kg(ctx, hull, 15)` so crossing a whole-tonne boundary
  submits a `PendingSpaceBurn` (`tacsim_port_burn`) (`35c` §9 `SF.R17`).
- `BubbleCarrierEntry::holding_for_salvage_until_wall_s` (`SALVAGE_HOLD_TIMEOUT_WALL_S = 60`) bounds per-hull salvage
  holds, and `clear_resolved_bubbles` (`tick.rs`) lets any ready carrier in a multi-carrier bubble where
  `is_combat_clear()` holds detach and resume its `CarrierJourney` immediately without waiting for another carrier's
  `holding_for_salvage` (`35c` §4 #8).
- `validate_docking_initiation_gates` (`commands.rs`) evaluates physical terminal-braking range (`<= 500 m`) and
  relative speed (`<= 15,000 mm/s`) on the port-at-leg-end branch (replacing `remaining_s > 60`) and removes the
  `dist_km <= 2 -> 200 m` clamp on the free-flight branch (`35c` §4).
- `step_active_bubbles` (`tick.rs`) steps active bubbles by `min(elapsed_ms, MAX_BUBBLE_TICK_STEP_MS)`
  (`MAX_BUBBLE_TICK_STEP_MS = 30_000`) and skips stepping when `wall_now_s <= last_stepped_wall_s` (`35c` §6.6).
- Hardened `tacsim_integration_tests_never_mutate_space_bubble_book_directly` (`crates/mush-contract-tests/tests/arena_library.rs`)
  to forbid qualified/unqualified/aliased `module_state_mut` on space books, `clear_interdiction_bubble(`,
  `apply_bubble_command(`, `solve_docking_tape(`, and `solve_hard_bounce_tape(` across `crates/tacsim/tests/`, and
  updated `sfr5`, `sfr6`, `sfr7`, `sfr8`, `sfr9`, `sfr12`, `sfr15`, and `sfr16` (`35c` §5, §9 `SF.R17`).

**Test.** `sfr17_hole_closeout_ii_abort_undock_expedition_dock_gates_salvage_hold_and_30s_tempo`.

**Divergences.** None from the `35c` §9 `SF.R17` specification.

## SF.R18 — the multi-carrier bubble

- Assigned each carrier in `ActiveInterdictionBubble::carriers` (`crates/tacsim/src/space/tick.rs`) its own distinct
  arena `PartyId` (`CARRIER_PARTY` for the primary carrier, `ActiveInterdictionBubble::next_available_carrier_party`
  for subsequent carriers joining the domain bubble) with `Friendly` inter-carrier relation and per-carrier relation
  entries against `CUSTOMS_CORVETTE_PARTY`, `BELT_RAIDER_PARTY`, and `INTERDICTOR_PARTY` (`35c` §4 #4, §9 `SF.R18`, `Q-36-48`, `doc 40` `FR.6`).
- Added `KirkwoodPicketState::carrier_inspection_status` and `carrier_inspection_outcome` (`crates/tacsim/src/space/picket.rs`)
  with per-carrier helpers (`inspection_status_for`, `inspection_outcome_for`, `set_inspection_status_for`,
  `set_inspection_outcome_for`, `is_carrier_customs_cleared`), and updated `helm_submit` and `helm_run`
  (`crates/tacsim/src/space/commands.rs`) and `step_kirkwood_picket_for_carriers` (`picket.rs`) so one carrier's
  `+helm/submit` or `+helm/run` updates only that carrier's inspection status and Corvette party relation (`35c` §4 #4).
- Evaluated `is_combat_clear`, `is_carrier_clear_to_resume`, `is_clear_to_resume`, `clear_resolved_bubbles`, and
  `settle_disabled_carrier_bubbles` (`tick.rs`) per carrier so a disabled escort carrier is settled into
  `SpaceBubbleBook::derelicts` (`JourneyState::Failed`) and checked in immediately while an operational cleared
  carrier remains in the bubble until combat clears and then resumes its `CarrierJourney` (`JourneyState::Cruising`) (`35c` §4 #5).
- Checked in every participating carrier and NPC contact (`interdictor_hull`, `corvette_hull`, `raider_hull`) across
  all bubble exit paths (`clear_interdiction_bubble`, `clear_resolved_bubbles`, `settle_disabled_carrier_bubbles`, and
  `helm_dock` port/derelict clamps), updated `SpaceBubbleBook::remove_carrier_from_active` to snapshot the docking
  hull's own `SpaceShipState` before removal, and stood down the domain activation when the last carrier leaves (`35c` §6.1, §9 `SF.R18`).
- Keyed `SpaceBubbleBook::cleared_interdiction_legs` by `(hull, leg_cursor)` (`BTreeSet<(ObjId, u32)>`, cleared on new
  departure in `launch_step`) so a carrier that cleared an interdicted leg is never re-promoted into that leg's domain
  when a subsequent hull arrives (`35c` §4 #6).

**Test.** `sfr18_multi_carrier_bubble_per_carrier_party_inspection_escort_disabled_settlement_and_domain_stand_down`.

**Divergences.** None from the `35c` §9 `SF.R18` specification.

## SF.R19 — one salvage path

- Unified space derelict salvage with `GC.6`'s economy-contract salvage claim path (`mush_economy::file_salvage_claim`,
  `mush_economy::award_salvage_title`, `mush_economy::close_salvage_claim`, and `SalvageCustody`) (`Q-36-46`, `35c` §4 `SF.R19`).
- Removed auto-filing of salvage claims on disablement in `clear_resolved_bubbles` and `settle_disabled_carrier_bubbles`
  (`crates/tacsim/src/space/tick.rs`): disablement records the open-epoch `SpaceDerelictRecord` with `salvage_title_status: None`
  until a tug clamps (`+helm/dock`) and attaches a tow line (`+helm/tow`).
- Added `real_now_secs(w)` (`w.now().max(1)`, never dividing by `1_000` or `now.at / 24`), `salvage_claim_window_secs(w)`
  (`store.config().recovery.claim_window_s`), and `ensure_space_hull_titled` (`crates/tacsim/src/space/custody.rs`), and
  issued `EconomyStore` titles on spawned NPC picket/raider/interdictor hulls (`picket.rs`, `tick.rs`).
- Enforced single-claimant collar clamp and tow gates across `validate_docking_initiation_gates`, `helm_dock`, and
  `helm_tow` (`crates/tacsim/src/space/commands.rs`) so two ships racing for the same derelict cannot double-clamp or
  double-tow it, and added `+helm/tow release` to release a tow line and collar clamp.
- Moved the `1,200.00 cr` Freeport Registry salvage recovery bounty settlement from deep-space tow attach to port
  delivery (`deliver_active_tow_at_port` and `step_delivered_tows_at_port` in `tick.rs`) with idempotency tracking in
  `SpaceBubbleBook::paid_salvage_bounties` so releasing and re-towing never pays the bounty twice (`35c` §4 #11).
- Wired `SpaceBubbleBook::space_custody` into `surface::salvage::custody_of` and `surface::claim` (`+salvage`,
  `+salvage/inspect`, `+salvage/report`, `+salvage/contest`, and `claim::tick`) and cleared `SpaceBubbleBook::derelicts`
  when a salvaged hull is repaired at a port workshop (`crates/tacsim/src/port/bay_work/mod.rs`).

**Test.** `sfr19_one_salvage_path_unifies_space_derelict_claims_with_gc6_and_economy`.

**Divergences.** None from the `35c` §9 `SF.R19` specification.

## SF.R20 — one damage table and canonical drives

- Bridged `ShipSectionId`, `ShipSectionState`, and `SpaceShipState` onto `crate::carrier_damage::CarrierPart` and
  `CarrierDamage` (`Q-36-47`, `D-35-1`, `crates/tacsim/src/space/bubble.rs`), and routed `Slow` consequence capping through
  `crate::arena::damage::apply_slow_consequence` (`crates/tacsim/src/arena/damage.rs`) so `Slow` takes `min` instead of
  compounding across hits after `hold` reaches `0` internals (`35c` §6.5).
- Persisted space subsystem crits (`PdcOffline`, `SensorDegraded`, `RadiatorDamaged`, `DriveDegraded`), stripped PDC mounts,
  and `slow_pct` across `check_in_space_unit` and `space_ship_from_tactical_unit` via the `tactics.space.subsystems`
  capability (`crates/tacsim/src/space/custody.rs`), and restored `tactics.space.subsystems` on workshop bay repair
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

**Divergences.** None from the `35c` §9 `SF.R20` specification.

## SF.R21 — the prepared bubble mechanic and the per-entity store

- Added the prepared bubble input action (`BUBBLE_INPUT_ACTION = "tacsim_space_bubble_input"`, outcome
  `BUBBLE_INPUT_OUTCOME = "tacsim_space_bubble_input_outcome"`, typed family `BUBBLE_TAPE_FAMILY = "tacsim.space_bubble_tape"`)
  via `BubbleInputProvider` and `submit_bubble_input` (`crates/tacsim/src/space/tick.rs`, `crates/tacsim/src/port_tick.rs`),
  and wired in-bubble `+helm` (`/heading`, `/thrust`, `/emcon`, `/roll`, `/submit`, `/run`) and `+tac` (`/lock`, `/beam`,
  `/torp`, `/railgun`, `/pdc`) commands (`crates/tacsim/src/space/commands.rs`) to submit `tacsim_space_bubble_input` via
  `Ctx::submit_authoritative` (`D-35-9`, `35b` §3b, `35c` §6.1).
- Moved `StandingOrder` persistence out of `SpaceBubbleBook::standing_orders` onto `mush-sim`'s `StandingOrderBook`
  (`mush_sim::tick::standing_order`, `record_standing_order`, `clear_unit_standing_orders`, `standing_orders_snapshot`,
  and `restore_standing_orders` in `crates/mush-sim/src/tick.rs`), and wired `step_active_bubbles` (`crates/tacsim/src/space/tick.rs`)
  to read `StandingOrderBook` and apply `OrderChannel::Thrust` (`Scalar { value: 0 }` heave-to) and `OrderChannel::Posture`
  (`Emcon`) on each tick (`D-35-9`, `35c` §6.2).
- Replaced the monolithic `tacsim:space:state` write with dirty-only per-entity keys (`tacsim:space:bubble:{domain}`,
  `tacsim:space:flight:{hull}`, `tacsim:space:derelict:{hull}`, `tacsim:space:docking:{hull}`, and
  `tacsim:space:ledger:{hull}`) backed by `#[serde(deny_unknown_fields)]` wire structs (`PersistedBubbleDoc`,
  `PersistedFlightDoc`, `PersistedDerelictDoc`, `PersistedDockingDoc`, `PersistedLedgerDoc`) and fail-closed quarantine
  (`quarantined_bubbles`, `quarantined_hulls`) on decode failure (`crates/tacsim/src/space/tick.rs`,
  `crates/tacsim/src/store.rs`) (`35c` §6.3).
- Bounded `SpaceBubbleBook::completed` (`<= 64`), `paid_salvage_bounties` (`<= 256`), `pending_space_burns` (`<= 64`),
  `pending_mine_draws` (`<= 64`), and `SalvageClaimBook::black_box_tapes` (`<= 64` hulls in
  `crates/mush-tactics/src/field_ops/salvage.rs`), and unified propellant debits (`debit_hull_propellant_for_dv` and
  `debit_hull_propellant_kg`) so in-bubble `SpaceShipState::propellant_kg` and `HullPropellantLedger` stay synchronized
  (`35c` §6.3–§6.4).

**Test.** `sfr21_prepared_bubble_input_mush_sim_standing_orders_and_per_entity_store_with_quarantine`.

**Divergences.** None from the `35c` §9 `SF.R21` specification.

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
