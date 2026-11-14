# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Absent units answer fire under attack (R-47-2, R-47-3; doc 47 §3). Paths are under
`crates/tacsim/src/` unless given.

**Survey** (written before the cut).

- **Where `RETURN`'s "never fires" lives.** Not in the gate. `arena::absent::permits` has no
  arm for a posture: it lets an absent unit's shot through at a unit whose fire has arrived on
  it under every posture, and `GUARD`'s widening is the only posture-read input (`guards`).
  - **On the ground `RETURN` already answers** (MW.7's stories 2 and 3, AP.6's
    `ap6_a_seat_left_at_fire_hold_does_not_bind_the_mind`). What it does not do is keep going
    home: the go-home order is `Withdraw`, and the Standard Doctrine's `Withdraw` body
    (`arena/stock_mind.rs`, `withdraw_stand`) answers by a **cornered stand**: it stops
    (`Goal(Hold)` and a facing `Heading`), fires, dwells `STAND_TICKS` thinks, and stands
    again at the next think while the unit that fired is inside `stand_cm`. So a `RETURN` unit
    under fire stops its return until its attacker leaves range or it is hurt below
    `stand_min_permille`. The doctrine is the native stock mind, the oracle its Rhai twin is
    held to byte for byte, under `STOCK_VERSION` (`stock=3` in `MIND_ABI`): changing its body
    moves the ABI, every seal and the twin lane.
  - **In space `RETURN` never fires**, in three places: `space/backup.rs` `answers_of` answers
    `None` on `RETURN` (AP.9's default 2), so the seat is `FireRule::Never` and the view is
    not narrowed; `standing` reads her player's other hulls only off `RETURN`; and the house
    default (`content/minds/linkdead_return.rhai`) asks to answer only when
    `v.posture != "return"`. The applier (`space/applier.rs`) has no posture in it.
  - **A sealed mind in space never answers**: `fire_rule(_, sealed)` is `Never` for a seal
    (AP.9 default 1).
- **The think host and a second source.** `space/thinks.rs` collects one `Ask` a hull with
  one job **per source** (sealed mind, store house default, compiled house default), and the
  pool runs **every** job: `settle` keeps the first clean answer and drops the rest. So the
  house default's think on a sealed hull is already run on the same view and contacts; a
  second source for fire costs no think, only keeping that answer (one more `Option` on
  `Answer`) and landing its fire channel. Whose lock stands: a bubble's fire commands carry
  their target (`fire_offensive_lasers(shooter, target)`), so the answer can be fired at the
  house default's chosen ship without moving her lock: the sealed mind's sensor lock stands.
- **The ABI and the baselines.** The cut needs no getter, no intent and no doctrine change:
  the ground's rule is a kernel rule at the landing (the arena lands a house mind's stop as
  the drive home), and the space script already reads `v.posture`. **`MIND_ABI` stays
  `rhai=1.26.0;api=9;stock=3`.** The house default's source changes, so its digest does; it
  is pinned nowhere (AP.9 found the same). No gate-harness baseline can move: no unit of the
  harness or of a practice match nobody practises in is flown for an absent player, and every
  change here is behind that flag (the arena) or the backup seat (space).

**Built.**

- **The one gate is unchanged** (`arena/absent.rs`, `permits`): it already had no posture arm.
  What moved is what each host hands it and what each mind is told.
- **The ground: `RETURN` keeps going home while it answers** (`Arena::keep_going_home`, called
  by `Arena::mind_land` ahead of the intents' apply). For a unit the Standard Doctrine flies
  for an absent player, flown on `RETURN` under the go-home order, a think's `Goal(Hold)` and
  its facing `Heading` (the cornered stand's stop) land as `Goal(Cell)` to the cell the
  doctrine itself drives `Withdraw` to (its party's entry cell, `motor::entry_cell` on the
  tracked row, as the doctrine reads it), one intent for one so the author trace still joins
  outcomes to the think's intents by position. The lock and the shot land as thought, through
  the gate at the shot; a ground shot needs no facing, so the unit fires as it drives. A
  sealed mind, a pilot's think, `HOLD`, `GUARD` and a think that stops nothing are untouched.
- **Space: `RETURN` answers** (`space/backup.rs`). `answers_of` answers `Answers` under every
  posture (no more `None` on `RETURN`); `GUARD` alone still widens. `standing` reads her
  player's other hulls under every posture, so a ship of her own player is never answered on
  `RETURN` either. The seat's `fire_rule` is `Answer` for the house default under every
  posture and `Never` for a sealed mind. The house default script
  (`content/minds/linkdead_return.rhai`) asks its lock and its shot inside a bubble under
  every posture; its helm is as it was, so on `RETURN` she heaves to when stopped, coasts,
  and is on her filed passage again when the bubble lets her go.
- **Space: the house default answers beside a sealed mind** (R-47-2).
  - `space/thinks.rs`: `Answer::house` keeps, when the sealed mind's answer is the clean one,
    the first clean answer of the sources after it. The pool already ran them: no think is
    added.
  - `space/backup.rs` `collect_in_bubble`: a sealed mind reads the bubble as her sensors hold
    it (the view **not** narrowed: it has no fire channel, and the narrowing is the fire
    channel's), and the house default reads the view narrowed to what she may answer. Both
    views number contacts off the one book, so one map names the ships of both. This costs one
    more view a step for a hull with a sealed mind, and nothing for any other.
  - `space/applier.rs` `apply_answer`: the house default's fire channel alone. Its target is
    the ship its last `lock` names; its `fire`s go through the gate (`Answers::permits`) and
    are fired at that ship by commands that carry their target, so her lock is not moved.
    Helm, emission posture and lock are dropped: they are the sealed mind's. The mount
    firing is one helper (`shoot`) shared with `apply_intents_answering`.
  - `land_in_bubble` lands the sealed mind's think as before (its seat `Never`, its lock its
    own), then the house default's answer; `BACKUP_RETURNS_FIRE` is said the first time
    either fires.
- **Help**: `docs/help/ships.md` ("Her posture": `RETURN` answers and resumes; a sealed mind
  has the house default answer beside it), `docs/help/field.md` ("Postures": `RETURN` does not
  stop to answer). The absent-unit notice lines are not touched (CO.11's D3/D4 wait on them).

**Tests.**

In source (all of `tacsim`'s 2,352 unit tests pass on Windows):

- `arena::weapons::tests::co2_a_house_mind_on_return_answers_on_its_way_home`: the landing
  lands a stand as the drive home and the lock and shot as thought; the gate answers the unit
  that fired and refuses a first shot; `HOLD`, `GUARD`, a non-doctrine mind and a present
  pilot are untouched.
- `space::backup::tests::co2_what_a_hull_may_answer_is_read_off_the_bubbles_record_by_her_posture`
  (was AP.9's `ap9_…`): `RETURN` and `HOLD` answer their own attackers, `GUARD` also its
  player's; none answers a stranger's quarrel, her player's own hull or the ship she
  complies with, under any word.
- `space::backup::tests::ap9_a_mind_sealed_aboard_is_never_given_the_answering_seat`: the seat
  rule by source, not posture.
- `space::tick::thinks_tests::co2_the_house_default_answers_the_raider_that_fired_on_her_under_every_posture`
  (was `ap9_the_house_default_answers_…_by_her_posture`): the picket's three parties, all
  three words answer the raider, never first, never the corvette.
- `space::tick::thinks_tests::co2_a_sealed_minds_ship_answers_by_the_house_default_and_keeps_its_sensors`:
  `KEEPER` sealed aboard; under each word her beams go at the raider only after its fire
  arrived, her lock stays the sealed mind's (none), every trace word is the sealed mind's.
- `space::tick::thinks_tests::co2_beside_a_sealed_mind_the_house_default_lands_its_shots_and_nothing_else`:
  one landing by hand: the sealed mind's cut drive stands, her lock does not move, the house
  default's beam goes at the raider; the same house think as first source burns and locks.

Stories (`tests/domain_field/`):

- `field_ap6.rs` story 3, `co2_the_same_rig_on_return_answers_on_its_way_home` (was
  `ap6_the_same_rig_on_return_goes_home`), four parties: on `RETURN` the rig does not answer
  its neighbour's attacker; fired on itself, wherever it has got to, it answers twice, after
  the first arrival, at the intruder only; it moved between those two rounds (the field's
  position record); and it is put aboard its carrier.
- `space_ap9.rs` `co2_at_the_picket_a_ship_answers_the_raider_that_fired_on_her_under_every_posture`
  (was `ap9_at_the_picket_…_by_her_posture`), four parties: on every word she answers the
  raider and nobody else, never first; on `RETURN` and `HOLD` she is on her filed passage again
  after the picket, and after a restart.
- `space_ap9.rs` `co2_a_ship_left_on_any_posture_does_not_answer_for_a_stranger` (was
  `ap9_a_ship_left_on_hold_or_guard_…`): `RETURN` flown beside `HOLD` and `GUARD`.

**Stories that held "`RETURN` never fires", and what each became.**

| Story | Held | Now |
|---|---|---|
| AP.6 `field_ap6` story 3 | fired on through its neighbour only; fired nothing | renamed and extended above |
| AP.9 `space_ap9` picket story | on `RETURN` she answers nobody | she answers the raider; `RETURN` resumes her passage |
| AP.9 `space_ap9` stranger story | `HOLD` and `GUARD` only | `RETURN` too |
| AP.9 tick test | `RETURN` answers nobody | answers |
| MW.11 `mw11_a_picket_is_the_same_whatever_the_workers_timing` | her last act `HeavingTo` | `ReturningFire`, after the raider's fire arrived on her |
| MW.11 `mw11_every_think_of_a_bubble_runs_on_a_pool_worker` | 30 thinks in ten steps | her beam now takes the raider out within ten steps, so it thought 8 times: the test is about worker threads, so her mounts are removed and it counts 30 again |
| MW.7 `field_mw7` stories 2, 3 and 6 | already "`RETURN` answers" (AP.6) | unchanged |
| AP.10, AP.11, MW.8 (`voyage.rs`), MW.21 | no assertion that `RETURN` fires nothing where fire arrived on her | unchanged; run, results below |

**Divergences.**

- **The ground answers by the doctrine's cornered stand.** En route the Standard Doctrine
  stands only for a unit inside `stand_cm` (a third of its longest band, at most its first:
  1,000 m for the 105 mm cannon, a hundred site cells) and while it holds `stand_min_permille`
  (175) of health. Past either, a `RETURN` unit's answer is its drive home. Lifting it means
  changing the doctrine (`STOCK_VERSION`, `MIND_ABI`, every seal, the twin lane) or a kernel
  that chooses targets, and it bites at none of the field's ranges in play. Recorded for the
  owner, not built.
- **The author trace keeps the think's own intents.** A stand landed as the drive reads
  `Hold`, `Heading` with outcome applied.
- **A sealed mind's view is no longer narrowed** on `HOLD` and `GUARD` (AP.9 narrowed it,
  because one view was shared). Its sensors are its own (R-47-2), and the narrowing belongs to
  the fire channel it does not carry. MW.13's `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs`
  is what showed it: narrowed on `RETURN`, the rule program's `HOSTILES >= 1` term read false.
- **"Stops" has no space arm.** The seat's helm in a bubble never read the answer, and a ship
  on a passage is not thought for outside one, so there is no code to mutate for "`RETURN`
  stops when it answers" in space; the picket story pins that she resumes her passage.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | The ground's home for the landing is the doctrine's own `Withdraw` goal (the party's entry cell), not a carrier's ramp the host names | Low: one call |
| 2 | Beside a sealed mind the house default fires at the ship its own think locked; her lock stays the sealed mind's | Low: `apply_answer` |
| 3 | The house default's answer beside a sealed mind is the first clean house source after it (the store's, else the compiled one) | Low |

**Not built.**

- The doctrine's en-route stand range and health floor (above).
- AP.11's ship arm (CO.7) and a leader's order to an absent member (CO.3), which wait on this.

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

Not built.

## CO.11

Not built.
