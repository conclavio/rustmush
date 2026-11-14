# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Not built.

## CO.3

Not built.

## CO.4

Not built.

## CO.5

Not built.

## CO.6

Not built.

## CO.7

Not built.

## CO.8

Not built.

## CO.9

Not built.

## CO.10

**Survey.** What limits a ground tow, as built at `6cdc49a55`:

- **Mass, and nothing else.** `mush_tactics::field_ops::tow_mass_ratio_permille_capped` refuses a
  casualty whose laden mass passes the tug's cap (`ExceedsTonnageCap`). The cap is the tug's dry mass
  times its coupling's `TowRigProfile::cap_permille` (`Standard` and `Heavy` 2×, `RecoveryOptimized`
  3×, Q-41-27), or the definition's authored `logistics.tow_cap_t` when it has one (Tycho's crawler,
  400 t). The coupling is `logistics.recovery_profile`. No class or hitch rule binds: any mobile rig
  couples (`surface::tow::attach_facts`, `profile_of`, `tow_cap_kg_of`). The casualty counts laden
  (Q-41-29); a raider hull carries nothing the tow counts (`carried_kg`: no hopper, no hold).
- **Dry mass.** An authored `logistics.dry_mass_t`, else 10 kg a structural point
  (`dry_mass_kg_for_spec`). The line tank and its four variants author none: 2,990 points, **29.9 t**.
  The leased escort is the same tank authored at 8 t (doc 32 CF.6c, so it rides a Dray-P bay).
- **Who tows what.** The 6 t prospector (standard) 12 t; the 8 t escort (standard) 16 t; the 7 t
  tender (recovery) 21 t; Tycho's 130 t crawler 400 t, flown only by Tycho Mutual's retrieval. So no
  rig a player can lease reaches a line tank, which is the gap doc 42b's SU.4 recorded. SU.7's escort
  (the owner's ruling on Q-42b-14) closes it only for a yielded hull that can still drive; a yielded
  hull that cannot (power or tracks gone) needs a real tow, and SU.4's story stood in a drone rebuilt
  by staff hand for it.
- **The bay.** A Dray-P's bay takes two rigs, 20 t and 70 m³. A line tank never fits it, so a tow
  to her ramp ends in SU.7's "crew aboard, hull titled", which the load offered to an escorted hull
  only.

**The fix chosen: a tug, not a limit.** The limit is not the bug: 2× and 3× are rulings, and the
line tank's 29.9 t is the same measure every unearned hull takes. Raising the tender's cap (an
authored `tow_cap_t` of 30 t on 7 t) would put it at 4.3×, past where the recovery speed factor
reaches its floor (1‰: the pair would not move); making the tender heavier breaks Q-41-1's "7 t
rides beside an escort in a 20 t bay". So Occator gets a third lease design, the **wrecker**, doc 27a
§10's "mobile recovery rig" at escort scale.

**Built.**

- `crates/tacsim/content/units/tv_wrecker.json` (`tacsim.unit.tv_wrecker`, pack
  `tacsim.pack.tv_wrecker`): the tender's chassis widened and ballasted. **Figures, all provisional:**
  12 t dry, `recovery_optimized` (3×, **36 t laden**), tracked at 250 cm/s (the tender's 300), one
  teleoperated cab, a 1 t / 2 m³ `stow.bulk` tool locker and no `logistics.tender` stores, 36 m³. It
  rides a Dray-P bay beside a prospector (18 t, 60 m³) or an escort (20 t, 68 m³). Class `vehicle`,
  never fielded in an arena; no core. Coupled to a line tank it moves at 39% of its speed.
- `definitions::content`: `TV_WRECKER`, `TV_WRECKER_PACK`, `EXTENSIONS_WRECKER`, fielded last so no
  earlier table moves.
- **Lane v34 / relay v25.** Occator's `vehicles.designs` gains `wrecker`: pool 2, `OWP-WR-`, 2,200 +
  9,000 for 168 IC days (provisional, between the tender's 1,800 + 7,000 and the escort's 2,500 +
  15,000), the battery tool, no gate. `+tv/lease wrecker go` signs it. `lane_chain::strip_wrecker`
  is the strip chain's new head.
- `surface::tow`: the load's "crew aboard, hull titled" branch now takes any yielded raider hull that
  is no wreck (`yielded_hull`), escorted or in tow, so the wrecker's tow ends as SU.7's escort does.
  `profile_of_spec` and `tow_cap_kg_of_spec` split out of `profile_of` and `tow_cap_kg_of` so a test
  reads the shipped rigs through the attach's own functions.
- `docs/help/field.md`: the wrecker, beside the tender's coupling.

**Tests.**

- `field_ops::tests::co10_the_wrecker_takes_a_line_tank_and_refuses_a_heavier_hull` (in source):
  the wrecker's 12 t and coupling read from content; it takes every line tank, the scout, the drone
  and every leased rig; it refuses the heavy, both bipeds and a Dray-P; the prospector, escort and
  tender still never reach a line tank; the tender still takes a prospector and the prospector a
  drone; 36,000 kg passes and 36,001 kg is refused.
- The story, `surrender::co10_a_yielded_line_tank_is_towed_in_by_a_leased_wrecker` at 256 and
  `…_at_512` (`surrender_rig::towed_in_by_the_wrecker`), every step a registered command but SU.4's
  staging:
  1. The stranger leases a prospector and the wrecker at Occator (`+tv/lease wrecker go`) and sails
     to the site. A third captain's armed hull stands there too.
  2. The content's line tank, never rebuilt, stands powerless at the ramp, its crew at 0. The
     stranger's mark opens the field; three parties stand in it; the crew yields; the field settles.
  3. The prospector's `+tv/1 tow` is refused ("more than the 12 t a 6 t tug can tow"). The wrecker's
     `+tv/2 tow` previews a tow (29 t, no escort), and `go` lands it in tow.
  4. `+tv/2 load` previews "will not fit the Red Moth's bay … salvage of the unowned"; with `go` the
     crew are `Captured` aboard the Red Moth, the hull stays on the site titled to the stranger, and
     the wrecker lets it go.
- Lane v34: `co10_strip_wrecker_rebuilds_the_published_v33` and the pins that move with a lane
  (lane and relay digests and versions in `domain_economy`, `cost_lane` and `split_boundary`).

- **Mutations of the limit, each caught** (run on conclav.dev, the in-source test and the 256
  story; the unmutated tree passes both):

  | Mutation | Caught by |
  |---|---|
  | `RecoveryOptimized` cap 3× to 2× | both |
  | the wrecker's 12 t to 9 t (re-signed) | both |
  | the wrecker's coupling dropped to standard (re-signed) | both |
  | `profile_of_spec` maps recovery to standard | both |
  | the cap check removed | both (the prospector's refusal, the heavy hull) |
  | the cap made exclusive (`>=`) | the in-source test (36,000 kg at 12 t) |
  | the titled branch back to escort only | the story (the load's preview) |
  | an authored `tow_cap_t` of 29 t on the wrecker (re-signed) | both |

- **The pins lane v34 moves.** Occator's shed holds two more rigs, so
  `domain_world::vehicles`' readiness line and its 14-rig refusal, and `domain_arena::content`'s
  four unit counts and its 22-entry core table, move with it.
- **AP.6's story 1, loosened by exactly one move** (`field_ap6.rs`, CO.2's file, with the parent
  session's agreement). The two shed rigs move every later object's id, and with them whether
  the intruder's sensor clock (`classify_after_ms`, an elapsed-time rule that runs whoever flies
  the rig) comes due inside the story's twelve-second wait. The intruder's row of the rig then
  reads `Classified` where it read `Unknown`. The check now pins the row's bearing, range,
  motion, cell, aspect and facing (`seen_where`), and allows identity only the classify move:
  `Unknown` to `Classified` as the rig's own party, under that party's designator, its
  provisional number kept as its alias. Mutations planted in the contact row for a mind-flown
  target (range +1 m, bearing +5°, cell +1, alias dropped) each fail story 1; all five AP.6
  stories pass.

**Found and fixed: a raid that joins a running field never took its leader's WITHDRAW.**

- **The defect.** A director raid that arrives while a field runs joins it (doc 36 GC.11). Its
  hulls' crews are seated (doc 42b SU.1), so the arena raises each hull's helm epoch at the join.
  `field::raiders::on_joined` never mirrored that epoch into the world's `ArenaRegistry`, as a
  launch's join does (`field::carrier::seat_joined_pilot`). Every order the host stamped for a
  joined hull carried epoch 0, and the kernel refused it (`Refusal::Epoch`), the leader's
  WITHDRAW (`raiders::order_home`) among them. On base both of GC.11's hulls are refused, and the
  story passed only because the doctrine happened to bring both beside the ramp. Under lane v34's
  ids one hull chased its last kill nineteen cells east, stopped there, and was never ordered
  home: the field ran on to its idle demotion, about 2,600 IC seconds.
- **The fix** (its own commit): `raiders::on_joined` seats the joined crew through
  `carrier::seat_joined_pilot`, as a launch's join does.
- **The test**: `field_director::co10_a_raid_that_joins_a_running_field_mirrors_its_crews_helm`,
  on GC.11's ground: once the kernel's answer to the join is in, the world mirrors each joined
  hull's helm at its seat's epoch (1, as `field::admit` seats every crew). It reads no position
  and no id, so it holds under any numbering. Without the fix it fails on both hulls
  (`[(819, 0), (821, 0)]`); with it, it passes, and GC.11 passes unchanged. A probe on the
  kernel's intent loop (not kept) showed the WITHDRAW refused `Epoch` for both hulls on base and
  applied for both with the fix. The ledger line it writes (`DetachmentCommanded`) is not what
  the test reads, since a ledger chunk reaches the world only when it fills or the field ends.

**Divergences.**

- SU.7's ruling 2 ("crew aboard, claim hull") was worded for an escorted hull; it is read here as the
  rule for any yielded hull the bay cannot take, since a towed one meets the same 20 t bay.
- No yard bill (`industry/hulls-v2.json`) and no hull-kit good: the wrecker is leased, not built.
  CO.9 owns the yard content this wave.

**Not built.**

- The wrecker's look (`unit_looks` lists it lookless, as the tender).
- A yard bill and hull kit for the wrecker.
- Lane v34 collides with any other wave unit that bumps the lane: whichever lands second re-signs on
  top (v35 / relay v26).

## CO.11

Not built.
