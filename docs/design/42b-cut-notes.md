# 42b. Cut notes — surrender

One `## SU.n` heading per cut of [42b-surrender.md](42b-surrender.md) §7 (`SU.0`–`SU.6`), in the order
they landed: what was built, the gate that proves it, where the PR departed from §7's row, and the cell's
summed-minutes delta ([doc 43](43-testing-strategy.md) rule 2.3 #14).

## SU.0 — the trait on the record

Q-42b-5. No behaviour a player can reach changes: nothing writes the field until SU.1's seed, and nothing
reads it until SU.3's check.

**The record** (`crates/tacsim/src/field/crew.rs`).
- `CrewRecord.resolve_permille: Option<u16>`, 0 to 1,000 per mille, `serde(default,
  skip_serializing_if = "Option::is_none")`. `None` never yields: every player record and every record
  written before SU. `CREW_FORMAT` stays 1, as `Extraction::forced` did.
- Carried unchanged by every writer. The three that build a record afresh take the prior record's
  value: `check_in`, `on_frames` (a pilot incapacitated in the hull) and the eject
  (`field/eject.rs`). `capture`, the handover, the release and the extraction already rebuild with
  `..record`.
- `status_line` is untouched: the trait is never shown.

**Gate.** The in-source test `su0_resolve_on_the_record_decodes_under_the_same_format`. An old record
reads `None` and re-encodes byte-for-byte. A set value round-trips. A capture carries it. An unknown
neighbour field is still refused (`deny_unknown_fields`). The existing CR.0 decode test is unchanged.

**Departures from §7's row.** §7 names only `field/crew.rs`. The eject's writer (`field/eject.rs`) and the
custody rig's literal (`tests/support/field/custody_rig.rs`) also build a `CrewRecord` field by field, so
each names the new field.

**Cell delta (rule #14).** None. The one new test is an in-source unit test in the tacsim lib; no domain
binary or shard cell changes.

## SU.1 — raider crews

Q-42b-1, Q-42b-2, Q-42b-6, Q-42b-7. Raider hulls now carry a character to capture. Nothing reads the
resolve yet (SU.3).

**The content** (`crates/tacsim/content/field/raiders.json`, schema 5).
- `resolve_permille` on each raider: Skulk raiders 400, Reach raiders 750 (provisional).
- An optional `resolve_permille` and `crew` (a name) on each unit. Neither shipped raider names its
  crew, so they read "Skulk One crew" and so on.
- `raiders()` takes schema 5 only. It refuses a resolve over 1,000 and a blank crew name. A missing
  value reads `DEFAULT_RESOLVE_PERMILLE` 500 (`RaiderDef::resolve_of`: the unit's, else the
  raider's, else 500).
- The brief grows its schema-5 sentence. The pack is in neither the lane nor the relay digest, so
  nothing is re-signed.

**The seed** (`field/raiders.rs`, `crew`). For each armed hull, never the carrier (Q-42b-3):
1. A fresh `ObjKind::Npc`, GOD-owned, created inside the hull. It goes where the hull goes: aboard,
   launched, wrecked.
2. A `cockpit` station on the hull (`CREW_STATION`, the cockpit channels, no console room), and the
   NPC assigned to it through `mush_sim::commands::assign_station`. `parties::seated` reads that
   seat, so the field admits the hull with a `Pilot` row. `admit` keeps an NPC seat's hull
   `Controller::Mind`, so the ROM seal flies it as before.
3. A crew record `Unit { unit: hull }`, `Fit`, the leader as organizer, the authored resolve.

A re-seed (refused while a hull stands) makes fresh crew. The director seeds through the same
`seed`, so director raids carry crew too.

**The check-in** (`field/crew.rs`). `check_in` now takes a player, or an NPC whose record carries a
resolve. The raider seed is the only writer of a resolve. A hired NPC pilot is still left to its
contract (Q-42b-2, Q-42-26), even after `on_frames` has written it incapacitated.

**Gate.**
- The in-source test `su1_schema_5_authors_each_crews_resolve` covers the shipped values, the
  fallbacks, the default name and the refusals.
- CF.6's raider story (`field_raiders::cf6_…`) now also asserts:
  - after the seed, each hull holds one seated NPC with a `Unit` record, the leader as organizer
    and resolve 400, and the carrier has none;
  - after the fight, each crew member checked in from their wreck with the resolve carried.
- `field_raiders::`, `field_director::` (both gc11 stories), `field_fr2::`, `field_gc10::`,
  `field_boarding::`, `testimony::` and `field_story::` are green with crew aboard.

**Departures from §7's row.**
- `on_frames` is not widened: it already writes any seated pilot it finds incapacitated, NPCs
  included. Since SU.0 it carries the prior record's resolve.
- The `check_in` widening keys on the resolve rather than on any NPC with a record, so a hired
  pilot's `on_frames` record stays inert. That is Q-42b-2's rule.

**Cell delta (rule #14).** No story is added. `field_raiders::cf6_…` gains two read-only assertion loops
over the seeded crew (no extra pulses), and the tacsim lib binary gains one in-source test of well under a
second. No domain binary's summed minutes move beyond run-to-run variance.

## SU.2 — the reach count

Q-42b-4's odds term and Q-42b-11's read. Nothing calls the read yet. SU.3's `lift_check` is its first
caller, so no behaviour a player can reach changes.

**The read** (`crates/tacsim/src/arena/state.rs`, beside `party_holds_armed_hostile_in_reach`).
- `Arena::armed_hostiles_in_reach(id: ObjId) -> u32` counts the contacts hull `id` holds that pass four
  tests. The contact is Hostile to the hull's party. Its target came armed or has fired. It is not read
  dead. It was seen inside the hull's own `fight_inner_cm`. The store keeps one contact id per target
  (`ShadowContacts::id_of`), so the count is of units.
- A wrecked hull, a hull with no reach and an id the arena does not hold all read `0`.
- It is a read, never a step: no state moves and nothing is hashed.
- The per-contact test is now one private helper, `armed_hostile_held_in_reach`. The party boolean and the
  count both call it, so the two cannot drift. The boolean's answer is unchanged: the same four tests run in
  the same order behind the same `reach != 0` and wreck filters.

**SU.3's odds term** subtracts the raider's own standing hulls from this count (Q-42b-4). That subtraction
is SU.3's to make, in `lift_check`, not the kernel's.

**The gate.**
- The in-source test `arena::state::tests::su2_the_reach_count_counts_armed_hostiles_in_the_hulls_own_reach`
  holds one RED line tank and five contacts. A teammate is not counted. A never-armed barge is counted only
  once it has fired. A plot past the reach is not counted. A contact read dead stops counting. A wreck, a
  gunless hull and a stranger read `0`. At every step the party boolean equals "some hull of the party
  counts more than none".
- The harness kind on conclav-dev (doc 43 rule 13), because the cut edits `arena/state.rs`. It was green at
  `a5348999`, run [42059596120](https://github.com/conclavio/rustmush/actions/runs/42059596120).
  `tacsim_harness_delta_against_the_committed_baseline` passed against `arena.linux.json` (blessed at `a2ef4075`)
  with every deterministic leaf equal and the four `minds` blocks equal. The timing gate is live on that host, and
  every compared timing stayed inside 20%.

**Doc 40 §4a's pins.** Expected unmoved, as Q-42b-11 says, and none moves. The harness baseline is unmoved (above),
and the pin tests run in ci.yml's tacsim shards on this PR:
- The cut adds no draw, no `UnitState` or snapshot row, no pack byte and no API bump.
- The only kernel behaviour touched is the boolean, and its body is the same predicate moved into a helper.
- No pin is re-blessed. This cut shares no batch with a rebless (Q-36-56). FR.5's rebless is merged, and
  no other is open.

**Departures.** None from §7's row.

**Cell delta.** One in-source test of about 0.2 s in the tacsim lib binary.

## SU.3 — the check and the yield

Q-42b-4, Q-42b-8 to Q-42b-12, Q-42b-15, Q-42b-16. A stranded raider's crew can now yield to whoever holds
its cell, and its hull leaves the field as a casualty. No kernel step changes, and nothing is hashed.

**The check** (`field/raiders.rs`, `morale`). It runs inside `lift_check`, after the decode and before any
carrier's withdrawal reading, at the 10 s `LIFT_CHECK_MS` pace.
- **Candidates.** Each raider hull in the field (never the carrier) whose seated crew record carries a
  resolve under 1,000. With none, nothing is decoded.
- **Pressure** (`Pressure`, the five provisional terms of Q-42b-4):
  - carrier lost (a wreck, or not in the field): 400;
  - `Arena::armed_hostiles_in_reach` (SU.2) less the raider's own standing hulls: 150 for each one of
    excess, at most 450;
  - the raid's lost share × 300, yields counted lost;
  - `(1000 − health_permille) × 3/10`;
  - leader gone: 100.
- The sum is clamped to 1,000. The gate is carrier-lost or excess.
- **Holder.** `eject::holding_parties` reads the eject's (a) and (b) on any hull of the decoded arena:
  each party Hostile to the hull's party, with a live armed unit that has the hull inside its own reach,
  nearest first. `eject::holder_of` (the eject's `holder`, made shareable) then wants a carrier of its own
  in the field. With no holder, nothing is drawn and no step is spent (Q-42b-9).
- **Steps.** The pass draws once for each new 100-per-mille step (`steps_due`). The draw is
  `eject::yield_draw`, the eject's FNV-1a over `(site, ordinal, hull, step)`. It yields when the draw is
  under `pressure × (1000 − resolve) / 1000` (`yield_chance`). The highest step drawn is kept on the mark
  as `morale_step`.
- **Withdrawal first.** A hull under the leader's WITHDRAW is checked only once it is not under its own
  power: the arena's withdraw rule, mirrored on the decoded snapshot (Q-42b-16). A stranded hull has had
  its WITHDRAW spent and is checked like any other.

**The yield** (`stand_down`, `ask_yield`, `exit::Then::Yield`).
- Before the ask, the mark gains `yielded: { carrier, ordinal, to }` and loses `carrier` and `withdraw`.
  Every `carrier_of` reader drops the hull from then on (aboard, launch, recall, rejoin, `wreck_aboard`,
  `lift_check`'s `launched`/`exited`), so the latent exit of §3a never reads it as boarded (Q-42b-14).
- The no-ramp exit is asked as the leader, else the crew, with no carrier named. A busy field, or an
  ask a restart lost, is asked again at the next pass.
- `on_withdrawn` checks the hull in through the IC condition plan. `check_in` writes the crew `Unit` or
  `Stranded` in it. The hull never boards, even after a restart.
- `raiders::on_yielded` tells the site "The {hull} stands down: its crew yield to party {X}. It is out of
  the fight, and anyone may take it in." Once no hull of the raider stands un-yielded in the field,
  `lift_marks` drops the leader's party's `stance_hostile` rows from the live record and clears each of
  its marks in the kernel (`ArenaMsg::Mark { mark: None }`), so the field can end by disengagement
  (Q-42b-10).
- `lift_check` counts the raider's yielded hulls (`yielded_of`) as launched and lost for the director's
  `RaidReading` (Q-42b-16). A carrier whose other hulls are aboard or down lifts and leaves the yielded
  hull behind.

**The casualty** (Q-42b-12, Q-42b-13).
- `close::demote_units` ends with `raiders::keep_yielded`. Every yielded hull standing on the site's
  ground that no row holds yet is written as a `Casualty` at its pose, so every tow rule for a casualty
  applies once the field demotes.
- A yielded hull is clutter afterwards:
  - a later field's admission lays it as a held wall unless it is in tow;
  - `site_population` never counts it;
  - a running field never joins it.
- `@tacsim/site` lists it "casualty, yielded".

**Gate.**
- In-source tests:
  - `su3_pressure_opens_on_odds_or_the_carrier_and_sums_its_terms`;
  - `su3_resolve_weights_the_chance_and_steps_are_drawn_once`;
  - `eject::su3_the_yield_draw_is_keyed_by_the_step`.
- Stories in `domain_field/surrender.rs` (`support/field/surrender_rig.rs`), each at 256 (Ahuna Flank)
  and 512 (Urvara Reach). The content's raider is seeded and launched through `@field/raiders`, its barge
  is gone, and a stranger marks it Hostile with the Red Hound west of the ramp and the Red Moth on the
  pad:
  - **the yield:** the crew at 0 (hull stripped) yields to the stranger's party. The mark keeps its
    carrier and step, the hull leaves the field, the crew record stays in the hull, the crew at 1,000
    never draws, and after the demotion the hull is a casualty listed "yielded";
  - **(s2)** both crews at 1,000: nothing yields;
  - **(s3)** the Red Moth gone too: no holder, no yield, no step;
  - **(s5)** a crash right after the yield is marked: the mark is unchanged across the restart, and the
    resumed field lets the hull out with the step unchanged.
- Green with morale live at the shipped resolves:
  - the existing raider stories in `domain_field` (`field_raiders::`, `field_director::` both gc11
    stories, `field_fr2::`, `field_gc10::`, `field_boarding::`, `testimony::`, `field_story::`,
    `crew_custody::`): 36 passed.

**Departures from §7's row.**
- `left_condition` is unchanged. The yielded hull's casualty row comes from `keep_yielded` at the close.
  Its exit's own check-in writes nothing to the record (an exit never does), so the row has to be laid
  where the close lays its units.
- The holder read is host-side over the decoded arena's public state, not a new arena read. `eject_read`
  refuses a hull that is not doomed, so it cannot be reused. Writing the read in `field/eject.rs` keeps
  SU.3 off the hash-pinned arena files.
- The site line goes to the site's seated pilots and rig operators (`tell_site`). The cockpit draws
  nothing new (Q-42b-13).
- Also touched: `field/admit.rs`, `field/carrier.rs` and `field/parties.rs` (the clutter rule);
  `field/boarding.rs` and `field/home.rs` (the new `Then` arm); `surface/site_record.rs` (the listing).
  `crew::set_resolve_for_tests` is a `#[doc(hidden)]` seam for the stories, never called by the game.

**Cell delta (rule #14).** Measured from the JUnit `<testcase time>` sums of #491's run 42075225787. The
eight `surrender::` stories add 11.5 summed minutes to `domain_field`, spread by nextest's partition over
six shards:

| Shard | Stories | Minutes |
|---|---|---|
| shard_1 | the yield at 256 (88.5 s) | 1.47 |
| shard_2 | (s2) at 512 (76.1 s) | 1.27 |
| shard_4 | (s5) at 256 (67.9 s) | 1.13 |
| shard_6 | (s5) at 512 (105.2 s) | 1.75 |
| shard_7 | (s3) at 512, (s2) at 256, the yield at 512 | 4.46 |
| shard_8 | (s3) at 256 (86.5 s) | 1.44 |

shard_7 is the run's longest at 49.8 summed minutes. The lib's three in-source tests are well under a
second.

## SU.4 — capture and the NPC handover

Q-42b-17, Q-42b-18, Q-42b-19. The capture itself needed nothing new: CR.1's `captured_loaded` captures
a yielded hull's crew (no contract, so `npc_exit_loaded` passes it by), and `capture` keeps the resolve.
A raider is Hostile to its captor, so `capture_standing` writes nothing (Q-42b-17).

**The NPC branch** (`field/crew.rs`). A *raider's crew* (`npc_captive`) is an NPC whose record carries a
resolve. The raider seed is the only writer of one, so a hired pilot is never caught by these rules.
- `hand_over` writes `Processing` as for anyone. The NPC hears nothing. The captor hears
  `HANDED_OVER_NPC_CAPTOR`, "At {port} the {authority} take {who} into custody. You are paid nothing
  for it."
- `levy_release` returns at once for an NPC, so `charge` stays `None` and no obligation is created
  (Q-42b-18).
- `custody_pass` takes a raider's crew out of play (`leave_play`):
  - at `release_due_ic` while in processing;
  - when set down at a port;
  - when stranded in a hull that is no raider's (its captor's wreck, CR.3's `wrecked_captive`) on a
    site with no field running (Q-42b-19).
  `leave_play` deletes the record, then `World::destroy` (marked `GOING`, out of the world) and an
  `ObjDestroyed` event. A raider's own wreck or yielded hull still holds its crew for somebody to load.
- `+ship/release` refuses a raider's crew anywhere but a port: "{who} has no way off this ground; hand
  them over at a port." At a port it sets them down, and they leave play at the next crew tick.

**Gate.**
- The in-source test `su4_an_npcs_record_is_done_at_a_port_or_its_captors_wreck`.
- The story (s1), `surrender::su4_a_yielded_crew_is_taken_in_and_handed_over` at 256 and `…_at_512`
  (`surrender_rig::taken_in`):
  1. The raider's first hull stands powerless beside the stranger's ramp, its crew at 0. Its barge and
     its second hull are off the ground, so nothing fires on the Red Moth on its pad. The crew yields,
     and the field settles with the hull a casualty.
  2. The stranger's tug, launched after the demotion, tows and loads it.
  3. The crew is `Captured` aboard the Red Moth with the resolve kept, listed by `+ship/captives`, and
     refused `+ship/release` on the site.
  4. The Red Moth lifts for Ceres. The crew goes into processing with `charge: None`, and at release
     the record is gone and the object `GOING`. The captor holds nobody.

**Departure from §6's sketch, and a gap found.** No shipped rig can tow a raider's hull. A line tank
is 30 t laden. The tugs are the 6 t prospector (12 t cap), the 7 t tender and the 8 t escort (16 t). So
Q-42b-14's "anyone may take it in" cannot be reached with the shipped raiders: every yielded line tank
would sit out SU.5's window. The story's staff hand rebuilds the yielding hull as a drone
(`@unit/create #<hull>=tacsim.unit.drone_mk1`), a combat hull light enough for the 6 t tug. A scout is
16 t, still over the cap. A rig design such as the escort would read as a rig without a hopper and fail
the captor's departure. Closing the gap is a content
or ruling question (a heavier tug, a lighter raider hull, or a tow cap that counts a yielded hull
differently) and is flagged to the owner. §7's (s1) also sketches the players wrecking a director raid's
carrier. The story stands the raider's barge off the ground instead, the same "carrier lost" term.

**Cell delta (rule #14).** Measured from the JUnit `<testcase time>` sums of #492's run 42081753056.
The two (s1) stories add 1.69 summed minutes to `domain_field`: the 512 run (56.5 s) in shard_3 and
the 256 run (45.0 s) in shard_4. The lib's one in-source test is well under a second.

## SU.5 — the captor's window

Q-42b-20. A yielded hull nobody loads waits the owner's 28-IC-day floor. Then its crew leaves play, and
the hull stays a casualty for doc 41's salvage loop.

**The knob.** `[tacsim.field].yield_hold_s`, default 2,419,200 (28 IC days).
- `FieldConfig::check` holds it to `FIELD_YIELD_HOLD_S`, 2,419,200 to 7,776,000. The floor is the
  minimum, so no deployment can undercut it.
- It is mirrored in `mush-config`'s `TacsimFieldCfg` (with its table row) and mapped in
  `mush-server`'s `config_ext`.
- The `player_wait_floors` guard pins the default, the band's floor and mush-config's default
  (`the_captors_yield_window_is_at_least_28_ic_days`, item 9).

**The lapse** (`field/crew.rs`, `lapse_pass`, at the end of the crew tick's `custody_pass`). A raider's
crew still `Unit` or `Stranded` in a yielded hull leaves play (`leave_play`) once `yield_hold_s` has
passed since its casualty row's `left_ic`, which is the demotion that left it. A site with a field
running holds the lapse back.
- The yield row now carries its `site` (`raiders::yielded_on`), so the pass finds the record without
  a field.
- A loaded hull's crew is `Captured` and never read here. The hull's row and its mark are untouched.

**Gate.**
- `surrender::su5_an_unloaded_yield_lapses_after_28_ic_days` at 256 and `…_at_512` (`Case::Lapse`):
  1. The yield story runs to the demotion, the hull listed "casualty, yielded".
  2. The IC clock is moved to a minute short of `left_ic + yield_hold_s`, and the crew still waits.
  3. At the window the crew's record is gone and the object is `GOING`, and the hull is still a
     `Casualty` on the site record.
- The `player_wait_floors` guard, item 9.

**Departure from §6's sketch.** "(s4) with the rate knob": the story moves the IC clock with the rig's
`to_ic`, the same jump CR.3's bound story makes, rather than a rate setting.

**Cell delta (rule #14).** Measured from the JUnit `<testcase time>` sums of #493's run 42084940080.
The two (s4) stories add 3.06 summed minutes to `domain_field`: the 512 run (112.9 s) in shard_2 and
the 256 run (70.6 s) in shard_6. Each jumps the IC clock 28 days with `to_ic` and pumps no field
through the jump.

## SU.6 — docs, and the close-out

§5's amendments rode each cut's PR. The close-out PR adds the last three, and the programme closes
with doc 42b at Draft 0.4, as built.

- **Doc 40 §4a** records SU.2's re-run: the harness kind dispatched on conclav-dev (run
  42059596120), every pin unmoved, nothing re-blessed.
- **Doc 42 §7's** closing line points at doc 42b as built.
- **Doc 42b's** header lists every cut's PR, the review fixes beside the chain, where each of §5's
  amendments landed, and the open findings. The README row is updated with it.

**Beside the chain** (the 38a/38b review, the same files):

- **#497.**
  - F-14: `crew::release_fee_due` puts the fee due at `retrieval::debt_due`. It is pinned in
    `player_wait_floors` item 10, and CR.2's paid story pays at the due instant.
  - Astra #7: `vacate_seat` vacates every station a captive holds. Tested by the review's probe.
  - Astra #12: the post-field eject finds the hull up the location chain (`eject::seated_unit`).
  - F-22b: `+mine` refuses a captive anything but `/status`, and `+sortie/unseat` is held like
    `/board`.
- **#498, F-22a.** `mush-comsys` gains a game-free speak gate, `transit::SpeakGate`.
  - The composition registers it on `TransportComsysModule::with_speak_gate`, and both IC speaking
    paths ask it before `transit::submit`.
  - Tacsim's `DetachmentChatGate` answers `crew::refusal(Gate::Chat)` for `dt:` keys only.
  - A captive is gagged, not removed, so release restores the radio with no write.
  - It is proved end to end in `split_boundary`'s `f22a_a_captive_is_gagged_on_the_roster_channel_until_released`.

**Open after the close-out.** No shipped rig can tow a raider's 30 t line tank (SU.4's departure).
That needs an owner ruling or content before Q-42b-14's "anyone may take it in" can happen with the
shipped raiders.

## SU.7 — the yielded hull drives itself aboard

The owner's rulings of 2026-11-05 on the tow-cap finding (doc 42b Draft 0.5): Q-42b-14's yielded
hull boards under escort, and a hull the bay cannot take leaves its crew aboard and itself claimed
on the site. Built in `surface/tow.rs` alone. No content, bay, tug or raider changes.

**The escort.** `escorted(w, casualty, assessment)`: a raider hull whose crew yielded
(`raiders::yielded`), that can move, and is no wreck.
- `attach_facts` reads such a hull as `Disabled` for the library (towable), and weighs the coupling
  as a rig at least the hull's mass, so no tow rating binds.
- A sound hull that never yielded still meets `TargetNotTowable`.
- The tow book carries the escort exactly as a tow, so the rig keeps the hull within reach for the
  whole drive.
- Every line a player reads for it says it drives itself:
  - `ESCORT_PREVIEW`, `ESCORT_FILED` and `ESCORT_SET` (the attach landed);
  - `ESCORT_LOAD_PREVIEW` and `ESCORT_LOAD_FILED`;
  - `ESCORT_LOADED` (the site line once it stands in the bay).
- A yielded hull that fits the bay then loads by the ordinary path: custody with a claim owed, and
  CR.1's capture.

**Crew aboard, hull titled** (`crew_aboard_hull_titled`). `+tv/<n> load` of an escorted yielded
hull whose laden mass or volume the bay cannot take gets a different preview: "will not fit the
{carrier}'s bay … titled to you as salvage of the unowned". With `go`, three things happen:
1. the hull is titled to the captor (the carrier's custodian, else the loader) by
   `mush_economy::issue_title` under the registry issuer;
2. its crew are captured into the carrier's bay room by `load_crew` (CR.1/SU.4's branch);
3. the site hears `ESCORT_LEFT_TITLED`, and the escort is released (`release_cmd`).

The hull stays on the site's index as a casualty. A hull that already carries a title gets the bay's
ordinary refusal.

**Departure from the ruling's wording.** The ruling said "claim hull … through RS.3's claim path".
A raid's seed makes its hulls with no title, so `+salvage/claim` refuses them ("carries no title the
registry can name"), and `+salvage/retrieve` needs a title holder. The hull is titled instead, as
RS.6c-1's owner ruling titles an ownerless wreck to its recoverer, salvage of the unowned. That
gives the captor what the ruling wants (the hull is theirs to retrieve later) without minting raider
titles at the seed. Minting them would change the house sweep's ownerless rule.

**Gate.** `surrender::su7_a_yielded_line_tank_drives_aboard_under_escort` at 256 and `…_at_512`
(`surrender_rig::escorted_aboard`):
1. The content's line tank, stripped but mobile and never rebuilt, yields to the stranger, and the
   field settles.
2. The stranger's 6 t tug previews "under escort … drives itself" and sets the escort.
3. At the Red Moth's ramp the load previews "will not fit … salvage of the unowned". With `go`, the
   crew are `Captured` aboard the Red Moth with their resolve and listed by `+ship/captives`. The
   hull stands on the site titled to the stranger, and the escort is released.
4. `+salvage/retrieve` posts its job, and at the acceptance window's end Tycho Mutual takes it
   (`RetrievalStage::House`).

The existing tow, salvage, house, custody and surrender stories are green with the change.
