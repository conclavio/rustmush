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

RESULTS_PENDING

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
