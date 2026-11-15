# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

**Survey.** The typed word lived in one match arm in `field_commands.rs`, one in `directive_text`, six usage and refusal strings in `sortie/mod.rs` and `prepare.rs`, and two help pages (`field.md`, `detachments.md`). Nothing else parses the word: the mush-bot hits for `CONCENTRATE` are ore concentrate, a different thing. `FOCUS` collides with no other order word in the parser. `+focus` is a console command and never reaches the `ORDER` parser, whose verb is the second word of the line.

**Built.** `ORDER … <slot|ALL> FOCUS <contact>` does what `CONCENTRATE` did (R-47-1). `directive_text`, the usage and refusal constants (now `FOCUS_USAGE`, `FOCUS_UNIDENTIFIED`, `FOCUS_NO_CONTACT`, `FOCUS_NO_EXPIRY`) and both help pages say `FOCUS`. `CONCENTRATE` is still accepted, silently, as an alias, so nothing typed breaks; it is shown nowhere. The help's old paragraph on why the word was not `FOCUS` is replaced by a note that `+focus` is another command.

**Tests.** `co1_the_stored_variant_stays_concentrate`: a stored `{"kind":"concentrate",…}` reads back and re-encodes byte for byte. The `directive_text` unit test expects `FOCUS BK`. MW.14c's story (`field_mw14c.rs`) types `FOCUS`.

**Divergences.** None. The stored variant, `FieldObjective::Concentrate`, and the internal function and module names (`concentrate`, `Concentration`) keep their names: persisted bytes do not move, and an internal rename buys the player nothing.

**Not built.** Retiring the `CONCENTRATE` alias: it costs one comparison; drop it only if the owner wants the word gone.

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
| MW.7 `field_mw7` stories 2, 3 and 6 | already "`RETURN` answers" (AP.6) | unchanged, pass |
| MW.21 `mw21_completion` (dispatch) | the tape holds no shot of hers | a shot of hers is allowed only at a ship whose fire had arrived on her, never the corvette; her lock stays the bought mind's under the answer (`Reading` gains `fired_on_by`). On the branch the bought mind's ship does answer the raider by the house default: R-47-2 end to end |
| MW.13 `mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs` | passed on a shared, narrowed view | unchanged, passes because the sealed mind's view is its sensors' (below) |
| AP.10 (`domain_arena` cockpit), AP.11 (`field_ap11`), MW.8 (`domain_world` `voyage.rs`) | no assertion that `RETURN` fires nothing where fire arrived on it | unchanged; AP.11 passes on conclav.dev, all three in the branch's full dispatch |

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

**Mutations**, each alone on a throwaway clone at the branch head on conclav.dev (Linux), with
the in-source `space::`, `arena::absent` and `arena::weapons` tests and the stories
`field_ap6`, `field_mw7` (stories 2, 3) and `space_ap9`'s two `co2_`. Every one is caught:

| | Mutation | Caught by |
|---|---|---|
| M1s | `RETURN` given nothing to answer, in space (`answers_of` empty on `RETURN`) | `space::backup` `co2_what_a_hull_may_answer…`; the tick-level `co2_the_house_default_answers…`, `co2_a_sealed_minds_ship…`, `co2_beside_a_sealed_mind…`, `mw11_a_picket…`; story `co2_at_the_picket…` |
| M1g | `RETURN` given nothing to answer, on the ground (`absent_denies_fire` refuses on `RETURN`) | `arena::weapons` `co2_a_house_mind_on_return…`, `mw7_an_absent_players_mind…`, AP.6's guard test; stories `co2_the_same_rig_on_return…`, MW.7's 2 and 3 |
| M2 | `RETURN` stops when it answers (the landing lands the stand as thought) | story `co2_the_same_rig_on_return_answers_on_its_way_home` (it does not move between the two records). The unit test calls `keep_going_home` itself, so it is the story that holds the call |
| M3 | It fires first (the gate lets every shot through, nothing narrowed, ground and space) | 14 unit tests and 7 stories, among them AP.6's story 5 (a stance-declared Hostile) and `co2_a_ship_left_on_any_posture…` |
| M4g | It answers a party only marked or held hostile, on the ground | `arena::weapons` AP.6, AP.7, AP.10, MW.7 and `co2_` unit tests (7). The stories pass: the narrowed view keeps the house mind off such a party, as AP.6 recorded |
| M4s | It answers a party only held hostile, in space (`answers_of` takes in every hostile ship) | `mw13_two_sealed_minds…`; story `co2_a_ship_left_on_any_posture_does_not_answer_for_a_stranger` |
| M5 | It answers the customs ship (the `complying` filter gone) | `space::backup` `co2_what_a_hull_may_answer…`. The stories pass: the corvette never fires on a ship that complies |
| M6 | The sealed-mind ship's house default takes the helm and the lock | `co2_a_sealed_minds_ship_answers_by_the_house_default_and_keeps_its_sensors` (her lock), `co2_beside_a_sealed_mind_the_house_default_lands_its_shots_and_nothing_else` (her drive) |
| M7 | `GUARD`'s widening applied to `HOLD` (space and ground) | `space::backup` `co2_what_a_hull_may_answer…`, `arena::weapons` AP.6, AP.7, AP.10; story `ap6_the_same_rig_on_hold_does_not_answer_for_its_neighbour` |
| M8 | Nothing answers beside a sealed mind (AP.9's default 1 back) | `co2_a_sealed_minds_ship…`, `co2_beside_a_sealed_mind…` |

**Verified, and how.**

- Windows, the branch head: `cargo test -p tacsim --lib` (2,352 passed),
  `cargo clippy -p tacsim --all-targets -D warnings`, the lint job's M7 memory-size pass
  (with `.github/clippy-m7`), `cargo doc -p tacsim --no-deps` with and without private items
  under `-D warnings`, `cargo test -p mush-contract-tests` (nothing re-blessed), `cargo fmt`.
- conclav.dev: the `domain_field` stories named above plus AP.11's closing story, MW.13's
  undispatched ones and AP.9's other two; MW.21's dispatch story by `--ignored --exact`
  (passed, 174 s); the mutation table.
- The branch's full CI dispatch (PR jobs and every dispatch-only story), in the pull request.
- **`MIND_ABI` did not move** (`rhai=1.26.0;api=9;stock=3`), no seal, pin or descriptor
  moved, and no gate-harness baseline can: nothing here runs without an absent player, so the
  harness was not run.
- No mind API, store schema or persisted record changed shape. `Answer::house` lives only
  between a batch and its landing.

**Not verified.** Nothing ran against a live server or over SSH; nothing was deployed. The
ground's answer at ranges past the doctrine's stand range was not flown (no story's geometry
reaches it).

**Not built.**

- The doctrine's en-route stand range and health floor (above).
- AP.11's ship arm (CO.7) and a leader's order to an absent member (CO.3), which wait on this.

## CO.3

A leader's order to an absent member's unit (R-47-4; doc 47 §3, "A leader's order"). Paths are
under `crates/tacsim/src/` unless given.

**Survey** (written before the cut).

- **Where the refusal is.** Not in the order book's authority rule as such. `OrderBook::issue`
  (`mush-tactics`, `detachment/orders.rs`) answers `OrderError::Unauthorized` ("command
  authority does not cover the target unit") when the host's resolver refuses a recipient, and
  the field's resolver (`detachment/sortie/prepare.rs`, `message`, `Act::Issue`) asks two
  things of each unit: the mission command right over it (`allowed`, read from the unit
  authority owner) and `roster.member_available(slot)`. The second is what refuses a gone
  member: `field::reconcile` reads the member's `Presence` (`field::orders::presence`), and
  once the unit has passed to its mind (a quit, or five minutes after a lost link) the
  member's connection is `Disconnected` and the roster counts the slot unavailable. The
  command right is the member's lending to the mission's current leader and does not read the
  member's presence.
- **A second gate at the delivery.** The poll (`Act::Poll`) discards a delivered envelope's
  pending order when the slot is not available, by the same `member_available`. So admitting
  the issue alone would issue an order that never arrives.
- **The hand-off already works.** The carrying-out walk (`detachment/sortie/npc.rs`,
  `posture_order`) does not read presence: it writes the record, tells the player (queued when
  away) and answers WILCO as the player. A changed posture on a unit a mind flies lays the new
  word's standing order at once (`Arena::set_posture`, AP.6). So the cut is the admission.

**Built.**

- **One rule, read at the issue and at the delivery** (`detachment/sortie/field_orders.rs`,
  `takes`): a slot takes an order when the roster counts it available, or, for a `POSTURE`
  alone, when it is a player's unit that a mind flies because its player has gone
  (`field::orders::minded_for`: the field's presence record for that pilot says handed, the
  unit is in a running field, and it is not lost). The order then goes to the unit's mind
  through AP.4a's path unchanged. Every other order to a gone member's unit is refused as
  before.
- **What binds the mind is CO.2's**, untouched: `arena::absent::permits` (it answers a unit
  whose fire has arrived on it, never fires first) and `Arena::keep_going_home` (`RETURN`
  keeps going home while it answers).
- **Unchanged:** a present member's unit (the order is written while they hold the seat and
  the unit stays theirs), the leader check (`validate_command_at`), the command right, the
  relay and its jams, the OOC refusal, the hired-crew refusal, the unit answering for itself,
  the notice.
- Help: `docs/help/detachments.md` (POSTURE) and `docs/help/field.md` (*Setting it*, the
  leader's bullet). Every changed line is within 120 columns.

**Tests.** Stories in `tests/domain_field/field_ap4a.rs`:

1. **New**, `co3_a_leaders_order_after_the_grace_goes_to_the_members_mind`: two players on a
   roster in a three-party field. The leader orders the present member's rig to `HOLD`; it is
   written, answered WILCO, and the rig is still its player's (no presence record, not
   minded). The member's link drops; five minutes later the rig is its mind's and holds in the
   field. The member, who does not lead, is refused ("not this detachment's current leader").
   The leader's kernel `HOLD` to the gone member's slot is still refused ("command authority
   does not cover the target unit"). The leader's `2 POSTURE RETURN` is accepted, the rig's
   record reads `RETURN`, the rig answers WILCO, and its mind takes it out of the field having
   fired on nobody. **On base it fails** at the RETURN order: the leader's `ORDER ... 2 POSTURE RETURN` is refused, "command authority does not cover the
   target unit" (conclav.dev, main `6c86731fb` with only the tests applied). AP.4a's rewritten
   story fails on base at its step 5 the same way..
2. **Rewritten**, AP.4a's `ap4a_a_leader_orders_a_members_rig_to_guard_and_its_owner_can_set_it_back`
   step 5 held the refusal ("command authority does not cover the target unit") after the
   grace. It now orders `2 POSTURE HOLD` there and holds it accepted, written, answered WILCO,
   and the rig still its mind's in the field; steps 6 and 7 read the `HOLD` order (the
   member's report, the refused `NEGATIVE` against that order's sequence).
3. AP.4a's hired-crew story is unchanged.

**Mutations**, each alone on a throwaway clone of main with this branch applied, on
conclav.dev (Linux), with stories 1 and 2:

| | Mutation | Caught by |
|---|---|---|
| M1 | The issue's check back to availability alone (`takes` replaced by `member_available`) | both stories: the order is refused, "command authority does not cover the target unit" |
| M2 | The rule widened to every order (`sets_posture` dropped from `takes`) | story 1: the kernel `2 HOLD` to the gone member is queued, not refused |
| M3 | The delivery's check back to availability alone (the poll) | both stories: the order is issued, never delivered, and the record keeps its word |
| M4 | The hand-off skips a unit a mind flies (`posture_order` idles on `minded`) | both stories: the order is delivered and never written to the record |
| M5 | `Arena::set_posture` no longer lays the new word's standing order on a minded unit | **survives.** The field host asks a `RETURN` unit out at its ramp every second (`field/absent.rs`), and the rig stands near its carrier's ramp, so that, not the arena's order, takes it home here. AP.6's code, not this cut's; recorded |

**Verified, and how.**
- conclav.dev (Linux), a throwaway clone of main `6c86731fb` with this branch applied: AP.4a's
  three stories pass on the branch (147 s, 24 s and the new one 75 s); stories 1 and 2 fail
  on base; the mutation table.
- Windows, the branch: `cargo clippy -p tacsim --all-targets -D warnings`, the lint job's M7
  memory-size pass (with `.github/clippy-m7`), `cargo doc -p tacsim --no-deps` with and without
  private items under `-D warnings`, `cargo test -p mush-contract-tests` (nothing re-blessed),
  `cargo fmt -p tacsim`.
- No mind API, store schema, persisted record or `MIND_ABI` changed. Nothing ran against a
  live server; nothing was deployed.

**Divergences.**

- The ruling's limits are CO.2's: this cut adds no limit and no story of its own for the
  mind answering fire after the order. AP.4a's story 1 step 4 holds a guard answering; CO.2's
  `co2_the_same_rig_on_return_answers_on_its_way_home` holds `RETURN` answering on its way
  home. The new story holds "fires on nobody first" and "goes home" after the order.
- Only `POSTURE` reaches a gone member's unit. The ruling speaks of "the order"; the other
  roster orders (`HOLD`, `ENGAGE`, `ADVANCE`, `FOCUS` ...) are objectives for the player's own
  hand and the mind flies a posture, not an objective, so they stay refused. A default; low to
  reverse (`takes`).

**Not built.**

- A gone member's unit that has already been taken out at its ramp takes no order
  (`minded_for` needs a running field). Nothing flies it.
- Not pinned by a story: a lost (destroyed) unit refused the order (`minded_for`'s `!lost`);
  a member who quit rather than lost their link (the same presence record, `handed`).

## CO.4

Removed parts to storage (R-47-5). Built 2026-11-14 on `claude/co4-removed-parts-storage-3f6d14`,
cut from `6c86731fb`. Stored and fitted again are built; the sale moved to CO.13 (below, "Not
built").

**Survey.**

- **Where a displaced core went.** Back to its owner already. The core refit's Install is one
  closed exchange (`mush-core` `core_exchange`, industry's `ExchangePolicy::facts`): the
  incoming core goes into the hull and the displaced one to the requester (`return_to`), still
  titled to them and unencumbered. MW.5's story 4 pinned that. The kernel's head for it
  (`industry_refit_assets`) reads `detached`, carrying the order that displaced it, the hull it
  came from and a digest of the component it came out as (`history::detached`).
- **Why it was never fitted again.** Four places held "a second install cannot reuse it"
  (doc 17f §9), each on its own:
  1. the kernel's Accept heads (`core_refit/transition.rs` `admission_heads`, and the boot
     replay `history::heads`) admitted an incoming core only with no head or `released`;
  2. industry's Accept heads (`families.rs` `accept_heads`) and boot audit (`audit.rs`) the same;
  3. the economy's refit twin (`core_refit/leg.rs`): `exclusive` held a core installed once for
     ever, and `conflicts` refused a fresh Accept for it unless its last order was cancelled;
  4. the Tactics replacement lane (`component_replacement.rs` `Binding::definitions`) took only
     an incoming component equal to a fresh one of the target (condition, cells,
     capabilities), so even past the first three a damaged core could go in only as new.
- **What identity and condition it carries.** The core is the same `ObjId` and the same
  managed title. Its condition is the `SavedComponent` the displacing order's offer captured
  (`offer.displaced`: condition word, cells, capabilities), kept in the order's acknowledged
  image (`CoreProductionState` refit projection) and digested into the `detached` head. Nothing
  wrote it anywhere else.
- **Which storage.** The owner's own custody: the core is a titled world object in their hands,
  which they may stow wherever they keep things. No new store is needed, and none is built.
- **Bay parts.** Two paths. The player's is the port's `+bay` (doc 36 GC.8, doc 39 IN.2/IN.7):
  `+bay/strip` takes a surviving part (intact, damaged or jammed) off into a one-unit lot in the
  owner's hold with its `PartProvenance` row (condition included); `+bay/fit` fits it carrying
  that condition (`bay_work::jobs::work`, "comes in the condition its provenance records");
  `+market/sell <bid>=1` sells the lot into a yard bid (IN.11's story); a destroyed part is
  replaced (`replace`) and nothing comes back. That path already meets R-47-5, and its stories
  pin it (`in2_lifecycle`, `in7_part_recipes`, `in11_matter`). A part has no title: it is an
  inventory lot, so the managed-title wall below does not apply to it. The industry `bay_refit`
  (doc 31 UD.4b) has no player route: no command offers it; only the hardware flow's rig
  (`hardware/flow/bay.rs`) drives it. Its removed part is minted into **the bay operator's
  stores** as a lot at grade 9 (intact) or 3 (anything else), untraced. See "Not built".

**Design taken.** The smallest change that lets the existing exchange's output be fitted again:
open the `detached` head to Accept everywhere it was shut, and carry the saved condition into
the fitting. A destroyed core is the one refused.

**Built.**

- **Kernel** (`mush-core`, `core_refit/transition.rs`, `history.rs`): `admissible` reads a head
  `released` or `detached`; the Accept heads and the boot replay use it.
- **Economy** (`mush-economy`, the refit twin): `Leg::conflicts` now sees the retained histories,
  not one. The core's rule: a fresh Accept for a core conflicts while an order for it is open,
  or while its last installation by the journal put it in a hull; once a later installation
  displaced it, it replays. `exclusive` holds a core while its order is open, as the bay holds
  its hull. The twin stays one (CT.7): the bay's `conflicts` takes the same map and answers
  `false`. The seal census is unchanged.
- **Industry** (`core_production/refit/`): Accept heads and the boot audit admit `detached`.
  `policies::displaced_by` finds the order that last displaced a core in the acknowledged
  image (a core in hand was displaced by its latest displacing order: a later one could only
  displace it after a fitting mounted it again, and a mounted core leaves its hull by no other
  route). `policies::destroyed` reads that order's saved component: destroyed when it came out
  `destroyed`, or without the capabilities its own fitting put in (the arena's `destroy_core`
  revokes the core capability). The offer refuses a destroyed core; the fitting
  (`replacement_from`) puts any other in carrying its saved cells and condition.
- **Tactics** (`component_replacement.rs`): the lane checks the incoming component's shape
  against the target's (definition, mount, cell set and ranges, capabilities), not its state.
  The state is the trusted policy's: an intact component for a fresh core, the saved one for a
  stored core. `replace_component` already checked shape only.
- **Doc 47**: a CO.13 row after CO.4, "then CO.13" in the order line, CO.4's done-when amended.

**Defaults taken (provisional).**

1. **A damaged or degraded core goes in as it came out.** Its cells and its condition word are
   carried; it is not repaired by being fitted. Repairing a core is a separate job, not this one. **Cheap to
   reverse.**
2. **"Destroyed" is the condition word `destroyed`, or a capability missing** against the root it
   came out of. The arena's two core words map onto it: `degrade_core` lowers the class cell
   (carried), `destroy_core` revokes the capability (refused). **Cheap to reverse.**
3. **A destroyed core still comes out into its owner's hands.** The exchange returns whatever it
   displaces, and making it vanish is a new exchange shape. It is inert: no fitting takes it.
   Selling it, or writing it off, is CO.13's to decide (a buy-back must refuse it).
4. **The Tactics lane no longer insists on a fresh-condition component.** A policy that hands it
   a damaged state now installs one. The only policy is the core refit's (trusted composition
   only); the comment's "never a damage reset of unrelated cells" still holds (cells outside
   the component are the hull's own). **Flagged: a security-shaped change to a kernel seam**,
   cheap to reverse in code.

**Persisted shapes.** None move. `AssetPosition::Detached` already existed; no record gains a
field; CT.0's `refit_core.snap` and `refit_bay.snap` pass unchanged. The economy fold's replay
rule widens (a history set it refused, a core installed twice after a displacement, it now
accepts), which matters only for data that does not exist yet (pre-launch).

**Tests.** Each fails on `6c86731fb` with only the test files applied, and passes here:

- `mush-industry` `core_production::…::refit_tests` (the composed refit rig, under a second):
  `co4_a_displaced_whole_core_is_stored_and_fitted_again` (stored in the owner's hands, its head
  `detached`; a third fitting puts it back and stores the second core in turn; a cold boot
  reads the history), `co4_a_damaged_core_is_fitted_again_carrying_its_condition` (it comes
  out `damaged` with its class cell at the floor and goes back in so),
  `co4_a_destroyed_core_is_never_fitted_again` (the offer refuses it). On base all three are
  refused at Accept; the destroyed one's offer is admitted there, so its assertion fails too.
- `mush-economy` `refit_twin::tests::co4_a_displaced_core_is_accepted_again`: a core in its hull
  conflicts with a fresh Accept, the same core displaced by a later installation does not. The
  shared twin bodies now read `INSTALLED_HOLDS` beside `EXCLUSIVE_WHILE_OPEN` (both legs hold
  while open; only the core holds once installed).
- `mush-tactics` `component_replacement::tests::co4_a_carried_component_goes_in_as_it_came_out`:
  the lane installs a core whose class cell is under its maximum and whose condition is not the
  spec's, as given.
- **The story**, `tacsim` `domain_field` `c4_hardware::co4_a_displaced_core_is_stored_and_fitted_again`
  (the launch world, about 12 s): a Class II core takes harm in the first rig, is displaced by a
  Class I and stored in the pilot's hands, titled and unencumbered; fitted to the second rig it
  goes in `damaged` for the fitting fee alone; displaced again it is stored again; a destroyed
  Class I core displaced from the first rig is in the pilot's hands and the shop's offer of it
  is refused, nothing charged. Staff do the M8 world's standing three things (the wallet top-up,
  titling the leased rigs, standing things in the shop) and mark the harm (`@unit/damage`, as
  story 4's dent); nothing is minted for CO.4's legs.
- **MW.5's story 4, step 6, moved to the ruling**
  (`c4_story_4_a_class_three_core_is_fitted_read_after_a_restart_and_replaced`): the fitted core
  is still not offered; the displaced Class III core now goes into the second rig (its root the
  rig family's Class III, the fee paid), where it pinned the kernel's refusal; the third, fresh
  core is gone from the story, and the second restart reads the displaced core in that rig.

**Mutations**, each caught (`co4-muts.py`, in this group's notes folder):

| # | Mutation | Caught by |
|---|---|---|
| M1 | store: the displaced core is returned into the hull, not to its owner | all three industry tests |
| M3 | kernel admits `released` only | whole and damaged |
| M4 | the economy twin holds a core installed once for ever | `co4_a_displaced_core_is_accepted_again` |
| M5 | industry Accept admits `released` only | whole and damaged |
| M6 | the boot audit admits `released` only | whole (its cold boot) |
| M7 | the Tactics lane takes a fresh-condition core only | the tactics test |
| M8 | the carried condition is dropped | damaged |
| M9 | the carried cells are dropped | damaged (survived until it degraded the class cell; it does now) |
| M10 | the offer's destroyed check removed | destroyed |
| M11 | the `destroyed` word reads as not destroyed | destroyed |

There is no sell path in this unit to mutate: it moved to CO.13.

**Not built.**

- **The sale (CO.13).** A stored core cannot be sold with its title by any existing path. A
  governed core's title is a managed title, and `mush-economy/src/managed_title.rs`
  (`require_unmanaged_mutations`) refuses every generic `TransferTitle`, `Title`, `Encumber`,
  `Release`, `SetCatalog`, `Abandon` and `WriteOff` on one: "a future managed transfer must carry
  a separate private closed plan proof". Only charter transfer, seizure and coverage have such a
  plan. Built and run here, then reverted: three market catalogue entries for the governed
  cores (`asset`, at `CoreClass::finished_minor`), the fitting shop's integrator as a
  `+market/sell` vendor at 500 permille, an `Accept` party cap for that org, and a
  `TitleHeld` + `TitleTransfer` arm in `+market/sell` for an owner's unencumbered asset. On the
  launch world the contract reached both conditions met (the core in the vendor's vault, the
  title held by the pilot) and went `disputed` at settlement: the settle plan's `TransferTitle`
  meets `require_unmanaged_mutations`. A custody-only sale (the object arm as it stands) works,
  but leaves the pilot holding the title to a core the vendor holds; the lead ruled it out.
  CO.13 needs: a closed managed core-title sale family on the charter-transfer pattern (kernel
  protocol, economy plan with its closed proof, audit and boot replay); the core catalogue
  entries and the buy-back vendor as its first users; a refusal for a destroyed core; and the
  vendor's `Accept` standing (the port party-cap source names no fitting shop today).
- **The industry `bay_refit`'s removed part.** It has no player route, and three gaps, each
  owed with that route (with IN.2's owed `CommissionSource::BayRefit` producer):
  1. a destroyed part comes back as a worn lot (`ShippedParts::grade` returns grade 3 for any
     condition but intact). Fixing it moves the kernel's offer rule "a removed part is always
     returned" (`bay_refit/wire.rs` `validate_offer`, `validate_installation`), which is a
     kernel shape change for a path nobody reaches;
  2. the lot lands in the bay operator's stores, not the hull owner's, so the owner cannot
     sell it;
  3. the lot is untraced (no `PartProvenance` row) and carries only a grade, so `+bay/fit`
     refuses it and a second industry bay refit would fit it as new.
- **The capability arm of "destroyed"** is built but no test drives `destroy_core` into a
  displaced core; the tests use the condition word.

## CO.5

Hired crew in armed rigs (R-47-6). Built 2026-11-14 on `claude/doc47-close-out` (`6cdc49a55`).

**Survey.**

- **What the hand-off is for.** `mush_sim::handoff` is the K6 control-lease machine: one lease
  per `(unit, channel)`, a request staged as a pending handoff with one durable schedule and one
  job, applied by the scheduler at its boundary. A physically seated NPC (`physical_npc_station`:
  an NPC in the console's room, holding the billet) asks through `station::request_npc_control`,
  which `+sortie/seat` calls after it has placed the NPC and written the billet. Launch and recover
  (`detachment/sortie/runtime.rs`) witness the **helm** lease held by the slot's operator, so a
  hire with no helm lease cannot launch.
- **Why one channel.** Cut 5 leased per channel, and the offer (`project_station_offer`) pins one
  `channel` and one `expected_revision`; `station_channel` answered only a console of exactly one
  channel, so a console of several had no offer at all. Cut 16b K.2 made the proxy seat skip the
  request for the same reason. Nothing deeper: the lease, schedule and job are each per channel
  already, so several channels are several leases.
- **The escort tank's console.** Doc 32 cut CF.6c gave an armed rig one pilot console carrying
  every cockpit channel (`port/vehicle.rs` `console_channels`: helm, sensors, fire); an unarmed
  rig's carries the helm alone. A player flies an armed rig without any K6 lease on sensors or
  fire: the cockpit's gate (`arena_bridge` `authorise_seat`) refuses only a lease **another**
  principal holds, so a vacant lease passes.
- **How a hired NPC fires in a field.** Not through the cockpit. Its hull is checked out as
  `Controller::Mind` (`field/admit.rs`), and the field host offers each delivered order to the
  kernel (`field/orders.rs`, `offer_field_detachment`); the mind fires unless the order's
  `hold_fire` binds it (`Arena::holds_fire`). Nothing on that path read a channel or a lease.
- **The existing rule for a hire whose contract does not cover combat.** An activation has one
  duty (`+sortie/activate …=mining|combat|recovery`) and every hired NPC's contract must carry it
  (`activation/preflight.rs`; checked again at the seat, at launch and at the field's
  reconcile). A combat contract never mines (`sortie/npc.rs`, doc 27b FC.1). There was **no rule
  about fire**: no hire could sit at a console that fires at all, because the hand-off refused
  every console of more than one channel. AP.8's story stood a combat hire in a refitted
  prospector by a staff hand.
- **Absent-player rules.** None apply: the NPC is the operator, never absent; the leader's
  absence is AP.8's lapse, unchanged.
- **Persisted records.** Nothing versioned or schema-bearing changes. A seated NPC in an armed rig
  now holds two more rows of the existing control-lease family (sensors, fire), with their
  schedules and jobs, each in the family's existing format. The request's parameters gain an
  optional `also` list (`#[serde(default)]`); a one-channel offer carries no `also` and is the
  offer it was. **Cheap to reverse.**

**Built.**

- **`mush_sim::handoff`.** `npc_channels` is the set a physically seated NPC leases: the
  station's channels as its billet grants them (for a one-channel console exactly
  `station_channel`'s answer). `physical_npc_station` admits any console whose billet grants a
  non-empty set. A physically seated NPC at a console of several channels gets its own offer
  (`project_npc_console_offer`): every channel not yet its own, in one invocation (`channel` the
  first, `also` the rest), vacant leases only; a lease another principal holds or any pending
  handoff on the set projects no offer. The release gives back every lease of the set the NPC
  holds. `prepare_request` stages each lease with its own schedule and job (`request_lease`, the
  old body); `validate` refuses `also` on anything but an NPC console's request and on a repeated
  channel. Every other station keeps cut 5's one-channel offer, untouched.
- **`mush_sim::station::request_npc_control`** answers "already observed" only when the NPC holds
  every lease of its set.
- **`+sortie/seat`** (`detachment/sortie/seating.rs`): a console that carries the fire channel
  takes a hire only under a combat activation (`facts::console_fires`, `facts::fires_by_duty`);
  otherwise it refuses with `NPC_SEAT_NO_FIRE`. Launch checks the same rule again
  (`facts::launch_person`), so a console that came to fire after the seat does not launch.
- **The field's fire gate** (`field/orders.rs`, `may_fire`, `offer`): every order the host offers
  for a hired NPC goes through one door, which sets `hold_fire` when the NPC sits at a console
  that carries the fire channel and does not hold that channel's lease. A console with no fire
  channel is not the gate's.
- **Help**: `docs/help/detachments.md` says an armed rig's console takes control of all three
  channels and that only a combat duty sits at a console that fires.
- **Test rig**: `roster_rig::Billet::HiredArmed` leases the slot's rig `+tv/lease escort go`
  (the treasury pays the captain the difference by a staff mint, since an escort beside a
  prospector is more than a new First Captain holds); `roster_rig::staged` runs `fly`'s steps 1
  to 5 and stops before the seat; `seat` is `pub(crate)`.

**Tests.** `crates/tacsim/tests/domain_field/field_co5.rs`, on AP.8's field scaffold:

1. `co5_a_combat_hire_is_seated_in_the_escort_tank_and_fires_under_its_contract`: Rook Adeyemi
   (combat contract) in the leased escort tank under a combat roster, seated by `+sortie/seat`,
   holds the helm, sensors and fire leases; the roster launches into a field of three parties;
   on `+field/mark` and `2 ENGAGE` the tank presses the order and its rounds go at the intruder.
   No refit, no staff seat.
2. `co5_a_hire_on_a_mining_duty_is_refused_the_escort_tanks_console`: Ilya Mendes (mining
   contract) planned into the escort tank of a mining roster is refused (`NPC_SEAT_NO_FIRE`) and
   holds no billet and no lease.
3. `co5_a_prospector_seat_leases_the_helm_alone`: Mendes in a prospector is seated and holds the
   helm lease and no other.

`mush-sim` `handoff::tests::co5_one_npc_request_stages_every_console_channel`: one request stages
nine operations (three leases, schedules and jobs) and three schedules come due; `also` is refused
on a non-NPC reason and on a repeated channel.

On conclav.dev at the branch head: the three stories, all of `domain_field` (305 passed, 16
ignored), and the `mush-sim` library (86). Mutations, each alone, against the three stories:

| Mutation | Caught by |
|---|---|
| the hand-off drops the fire channel (the NPC console offer skips `fire`) | story 1 |
| a non-combat hire gets it (`+sortie/seat`'s refusal removed) | story 2 |
| the one-channel console path breaks (`npc_channels` answers nothing for one channel) | story 3 |

Locally: clippy on every target of `mush-sim` and `tacsim` under `-D warnings`, CI's M7
memory-size pass on `tacsim`, rustdoc for both, `mush-contract-tests`.

**Divergences.**

- **A hire not on a combat duty is refused the console, not seated on a narrowed one.** The
  billet's narrowing (`StationAssignment::granted`) could have withheld the fire channel and
  seated them, but a boot clears every narrowing (`boot.rs`, E7 cut 10's A6), and the mind of a
  hull checked out to a hire is bound by no `hold_fire` until an order stands, so a narrowed seat
  would not be sure to hold its fire through a restart or before its first order. Refusing the seat is the
  rule that cannot leak. The owner may object: R-47-6 reads "hired crew may be seated in armed
  rigs", and this seats only crew on a combat duty in them.
- **The fire channel follows the activation's duty, not the contract alone.** A contract carrying
  both mining and combat, activated for mining, is refused too: the duty a plan is activated for
  is what the hire was engaged to do on it.

**Not built.**

- **A mind with no order standing.** The field gate binds orders the host offers. A hired NPC's
  hull with no order yet is the mind's, as before CO.5; with the seat's rule only a combat hire
  can be in an armed rig, so no hire not on a combat duty is exposed to it.
- **A staff refit of a hired prospector** (AP.8's stand-in, still used by its stories) leaves a
  one-channel console on an armed hull: the gate does not apply and the hull fires as it did.
  `port/vehicle.rs`'s console boot warns of such a console as changed and leaves it; nothing in
  play makes one.
- **A cancel at a console of several channels** is not offered: the NPC's request applies at the
  instant it is made, so there is nothing to withdraw.

## CO.6

Not built.

## CO.7

AP.11's ship arm: the space half of doc 45's closing story, flown under R-47-3 as CO.2 built
it. Tests and this note only; no production code moved. Paths are under `crates/tacsim/`
unless given.

**Survey** (written before the cut).

- **Two rigs, one lane.** The closing story (`tests/domain_field/field_ap11.rs`) runs on
  `driving_rig`'s keyed world; AP.9's helpers (`space_ap9.rs`) take `excursions::Rig`. Both
  compose the same shipped game content, and the Kirkwood customs picket is lane content, so
  the arm flies in the closing story's own world with the owner's own carrier, after his
  field and its restart. The helpers read only the world, so `space_ap9::read_at_the_picket`
  and `drop_link_in` were split out of `ship_at_the_picket` and `drop_link`, which now wrap
  them; nothing else in `space_ap9.rs` moved.
- **The route.** Ahuna Flank clears only for Ceres ("No passage leaves Ahuna Flank for that
  port; Ahuna Flank clears for Ceres Freeport"), and the one shipped lane with a picket is
  Ceres to Stickney Yards by Kirkwood Depot (two legs, the bubble at the end of the first).
  So she lifts for Ceres first, by `+ship/lift ceres`, as `escort_lease`'s lessee does.
- **Her tank.** At Ceres she holds 18 t of reaction mass; the passage to Stickney burns 41 t.
  No route in this rig puts reaction mass in a tank (the chandlery sells it as cargo), so the
  rig's stores fixture `load` tops her up, as `bay_gc8` does before the same passage and as
  the story's own staging does for the passage out.
- **What "rounds read from the frame" is in space.** A bubble's frame is its step
  (`ActiveInterdictionBubble::steps`), and its input tape (`tape.entries`) holds every fire
  command with its shooter and target (`FireOffensiveLasers`, `FireRailgun`,
  `FireTorpedoSalvo`), the raider's script's and hers alike; `fired_on` holds the arrivals.
  The arm reads both, so every bound on fire is in steps and rounds, and the only wall bound
  is the outer one on the picket letting her go.
- **What AP.11 owed AP.9.** The staged write of the ship's posture (the record's one writer
  under a staff context) is gone: AP.9 built `+ship/posture`, and the owner types it from
  aboard her before he leaves. The second marker ("what the ship did on her word goes
  here") is the arm.

**The arm** (step 10 of `ap11_an_absent_players_outfit_through_a_raid_and_their_return`,
`ship_arm`). After the restart the owner's armed rig is stood beside the ramp and he takes
CF.5's exit from its cab (`+tv/1 recover go`), steps into the bay (`+tv/1 leave`), lifts for
Ceres, sets `+ship/posture return` from aboard her ("The Salt Moth holds RETURN.") and files
`+ship/undock stickney`. His connection drops and the link-loss grace is jumped at the play
rate, so the house default has her helm; the story takes her to the end of her first leg,
where the picket's bubble opens on her beside the customs corvette and the belt raider, and
reads her a second at a time (`Watch`, AP.9's reading plus what the tape gained) until the
picket lets her go.

**Each clause of the row, and the assertion that holds it.**

| "Done when" | Read from | The assertion |
|---|---|---|
| She flies her posture | Her record, the reply | `+ship/posture return` answers the set line; after the passage she still reads `RETURN` |
| She heaves to for the ship she complies with | AP.9's reading, each second | Her seat's act reads `HeavingTo` (her mind's own burn), customs read her cleared, and no reading ever says she ran the picket; and she was cleared at a step before the raider's first round at her |
| She never fires first | The tape and the readings, by step | No round and no lock of hers is at or before the step whose reading first shows the raider's fire arrived on her (step 6 in a run; her answer lands at step 7, from the think made on what step 6 showed). The raider, which her party holds hostile from the bubble's opening, lit its torch the step before it opened, so a mind that fired first would lock it by step 6 (M2) |
| She answers fire that arrived on her | The tape, by step | She fires, and her first round is within 3 of the bubble's steps of the raider's first round at her, with no more than 6 of its rounds fired at her by then (a run: the next step, after 3) |
| She never answers the ship she complies with | The tape | Every round of hers is at the raider (the only other ship there); no lock of hers names the corvette |
| She keeps going home while she answers | The berth, the passage's end | The picket lets her go within the outer bound (about 35 s in a run); she is under way on her filed passage; stepped an IC day at a time she is docked at Stickney Yards, the end of the passage she filed |

**What a run shows** (Linux, the probe that led to the arm): at the bubble's first step her
mind burns 750 mm/s² against her drift and she reads hove to; at the third she is cleared;
at the fifth the raider goes to active emission, at the sixth it locks her and three rounds of
its are on the tape at her, and the corvette locks the raider and fires; at the seventh she
locks the raider and fires, a round a step from then on; at the tenth she blows coolant. At
about 33 wall seconds the picket lets her go, and she is under way.

**Where the arm departs from the row, and why.**

- **"Home" in space is her filed passage.** The seat never thinks for a hull on a corridor
  (Q-44-45): `RETURN`'s movement there is the passage her commander filed, and inside the
  bubble her drive is cut once customs have let her go, under every word (CO.2). So "keeps
  going home while she answers" is: answering does not stop the picket letting her go, and
  she flies the passage to its end. The passage is for Stickney Yards, not a port of her own,
  because that is the one shipped lane with a picket; a ship left on `RETURN` at a cluster
  makes for port, which AP.9's cluster story holds.
- **The closing story's ship flies `RETURN` only.** `HOLD` and `GUARD` at the picket are
  `space_ap9`'s `co2_` stories, the same three parties under every word. On her pad in the
  field she holds `HOLD`, set by the owner; the field flies a carrier on her pad as `HOLD`
  whatever her record says (AP.7), so the field holds her to the record only, as before.
- **The arm's raider is the picket's scripted belt raider**, not the field's raider player:
  no route gives a player a hull that can fire in a bubble (AP.9's note: carriers in a bubble
  are marked Friendly to each other).

**Each use of staff fiat added.** No `module_state_mut` is added to a test file, so the fiat
budget is not re-blessed.

| What | How | Why no route in play reaches it |
|---|---|---|
| The armed rig beside the ramp | `driving_rig::to_cell` | The story's existing placement fixture; the drive is in play and the placement saves it |
| Her tank for Stickney | `driving_rig::load`, 60 t and 40 crew-days | No route puts reaction mass in a tank in this rig (above) |
| The passage's rate | `@tacsim/time rate=3600`, and 24:1 to jump the grace | The lane stories' rate; the link-loss grace is jumped at the play rate, not waited |
| The lost link | the connections dropped by the engine (`space_ap9::drop_link_in`) | AP.9's way: a lost link is a connection that goes down |

And one removed: **the ship's posture**, staged by AP.11 through `posture::record::set`
because no verb set it, is now the owner's `+ship/posture hold`.

**Mutations.** Each applied by exact text on its own copy of a throwaway clone on conclav.dev
(Linux), the closing story run, the copy discarded; the five run side by side, the story
unmutated beside them. Every one is caught, each by the assertion it was aimed at, which is
why the arm asserts in the order below.

| | Mutation | Fails at |
|---|---|---|
| M1 | `RETURN` given nothing to answer in space (`space/backup.rs` `answers_of`: an empty set on `RETURN`) | "she answers the raider": its three rounds are on the tape at step 6, the corvette fires on it each step after, and she locks nothing and fires nothing |
| M2 | She may answer any ship in the bubble (`answers_of`: every ship but her own, her player's and the one she complies with, whether its fire arrived or not) | "she locks and fires on nothing until the raider's fire has arrived on her (step 6)": she locks the raider and fires on it in step 6, on the think she made when it lit its torch |
| M3 | She may answer the ship she complies with (`answers_of`: that ship added), alone | **Survives**, as CO.2's M5 did: her view shows her a neutral ship as neutral (`space/view.rs`), so her mind never picks the corvette. The view is the second lock |
| M3b | M3, and her view shows her every ship she may answer as hostile, whatever its relation | "she answers the raider and nobody else, never the corvette": she locks the corvette at step 1 and fires on it every step |
| M4 | The house default does not heave her to (`content/minds/linkdead_return.rhai`: the hailed and held branches off) | "she heaves to by her mind's own burn, is cleared and never runs the picket": she coasts, customs still wait on her heave-to after the 600 s bound, and the raider never lights |
| M5 | Answering stops her return (`space/tick.rs` `is_carrier_clear_to_resume`: a carrier whose fire has arrived on any ship is never let go) | "the picket lets her go": cleared, answering the raider, still held at the bound |

M5 adds a stop rather than removing a line: no code in space stops a `RETURN` ship when she
answers (CO.2's divergence, "'Stops' has no space arm"), so the mutation is the defect the rule
forbids. Its file is CO.12's; it was mutated on the throwaway copy only.

What the clauses that are not the rule rest on: `+ship/posture` and the record are AP.9's
(its S1); that the raider opens on her once customs have finished with her is the picket's
script (`belt_raider.rhai`), the story's premise, asserted so a broken premise reads as one.

**Tier, and run time.** Unchanged: not gated, one `mod` line in `tests/domain_field.rs`, so
the pull request's `tacsim-integration` cells run it by nextest's hash partition. Alone, in a
test build on Linux (conclav.dev), the story with its arm took 121.4, 123.3 and 121.8 s in
three runs; the same story before the arm took 119.2 s in the same clone (AP.11 recorded 119 s
on Linux and 120 s on Windows). The arm costs about 2 s: about 33 one-second fires at the
picket and a handful of IC jumps. No window is waited out (the grace is jumped, the legs
are jumped to their ends, the arrival is stepped an IC day at a time).

**Verified.** On conclav.dev (Linux), a throwaway clone at main `6c86731fb` with this
branch's two test files, the final tree: the closing story and `space_ap9.rs`'s four stories
(5 passed, 122 s, the four AP.9 stories over the split helpers unchanged at 14 s);
`cargo clippy -p tacsim --all-targets` with warnings denied; the lint job's memory-size pass
(`.github/clippy-m7`); `cargo doc -p tacsim --no-deps` with warnings denied; `cargo test -p
mush-contract-tests --no-fail-fast` (all passed, nothing re-blessed); the mutation table.
`cargo fmt -p tacsim` on Windows. No production code moved, so `tacsim`'s unit suite, the
mind ABI and every pin are untouched.

**Not verified.** Nothing ran against a live server or over SSH; nothing was deployed. The
arm was not run on Windows.

**Found, and not fixed.** Nothing new. The picket holding her shifts her arrival past her
plan's own end (by the time she was held): her ticket's plan is not re-timed, so a story that
waits on `depart_ic + total_s` finds her still under way. Not a defect of this unit's rule;
noted for whoever reads arrivals off a ticket.

## CO.8

**Survey.** A hull whose launch runs late passes an interdicted waypoint unmet. Three ticks take part. The port's
tick (`port::voyage::launch_step`) departs her with leg times frozen from the plan's own `depart_ic` (doc 18 lead
ruling L3), so a late launch is back-dated and her first boundary is already due. Sim's journey tick crosses one
boundary a fire (R13). At an interdicted boundary it calls the carrier hook (`TacsimCarrierHooks::boundary`,
`composition/sim_routes.rs`), then `raise_interception`, which raises an interception only into a domain that
is already there at the waypoint; with none it logs "presents into NO domain". The space tick
(`promote_due_interdictions`) opens the bubble only where that interception stands. The waypoint's domain is made
lazily, by the hook alone, and since doc 35 cut SF.R14 (`e6f573c45`) only when the boundary is crossed within
3,600 IC s of its due time. A later crossing made no domain, so nothing was raised. The cursor moves past a
boundary once, so nothing was raised later either: she flew her second leg unmet.

Reproduced on conclav.dev with probes on each tick, on MW.10's story with its `fire_quietly` deleted, at
3,600:1 (one real second is one IC hour). The launch lands 1,464 IC s past the boundary. When the journey tick
fires after the port's tick in that slice, the crossing is 1,464 s late and the bubble opens. When the port's
tick is last in its slice, the crossing comes one slice later, 5,064 s late, and the space tick then reads no
domain for 30 s.

In play at 24:1 the order in a slice does not matter; the crossing being an IC hour (150 real seconds) late
does. For a launch, on the shipped Ceres–Stickney lane (leg 0 is 326,136 IC s), the launch arm must run about
3 h 49 min of real time after `depart_ic`. That happens after an outage or restart between the burn landing
and the launch, or when staff release a held launch (`launch_hold`: pilot ashore, lease ended, ticket unreadable)
that much later. She is then past Kirkwood straight away, and neither the customs picket nor the interdictor
stops her.

Only the first hull ever to reach the waypoint could pass: once any hull has opened its domain, Sim raises into it
at any lateness, and every later hull is stopped however late she crosses. So the gate was never a rule the game
kept (PR #586 ruled the same lazy domain a production fault for a restart). It came in the same commit that
interdicted the shipped Kirkwood leg, and the clock-jumping trade-run and docking stories met no picket only
through it.

**Built.** The hook opens the waypoint's domain however late the boundary is crossed: the 3,600 IC s clause is
removed. The crew gate (a connected player aboard, or a standing link-loss record) is unchanged.

**Tests.** `domain_world::voyage::a_launch_that_runs_late_still_meets_the_picket`: the burn lands and the port's
tick first runs two IC hours past leg 0's frozen end. The picket's bubble opens within three seconds and her
journey waits `Promoted` at cursor 1. On the code before it fails at "the picket's bubble opens on her", in every
slice order (the crossing is more than an IC hour late in each). Stories moved to a lane whose Kirkwood leg is
not interdicted, because they step the clock past Kirkwood to reach Stickney and the picket is not their subject:
`sfr16_client_docking_game_...`, `sfr24_one_docking_kinematics_...` and `sfr17_hole_closeout_ii_...`'s docking rig
(`lane_without_the_kirkwood_interdiction`, `domain_world/voyage.rs`); every story of `domain_economy/orders.rs`
and `domain_economy/sale.rs` (their `lane_variant`; `orders.rs`'s `stocked()` now seeds it, as `sale.rs`'s
did). With the fix and without the move, 3 `domain_world` and 29 `domain_economy` stories fail, at "the
tactical bubble is not secure", "GOD never submits from the port tick" and "the 395's epoch closed at Stickney".
One mush-server story, `commerce_composition::ic_slice_loop::ic12_a_rate_change_mid_passage_relabels_the_arrival`,
stepped an IC hour past the Kirkwood boundary and failed at "docked at the frozen instant": the picket's pause moves
the arrival. It boots the production root on the shipped content, where a seeded lane is refused at the shipped
version and not taken at the next, and the only route to Stickney passes Kirkwood. So it now flies to Luna's
Aitken Highport, whose one leg passes no interdicted waypoint, outfitted for that plan (`outfit_for`). Rheasilvia,
the other long single leg, needs a leg home this hull's tank cannot hold.

**Divergences.** None from doc 47. `orders.rs` was composed on the shipped game content (`Roster::bare`) and now
boots a seeded lane, as `sale.rs` does; `Roster::bare` is removed.

**Not built.** A first-ever crossing caught up at boot (`journey::advance_due(Source::Boot)`) still raises
nothing, because the boot pass never calls the hook (R9). The thorough fix is to make the domains of interdicted
waypoints before any hull can reach them, at lane apply or at launch. That would also make the crew gate moot, so
a hull that sails with nobody linked and no link-loss record would be held at Kirkwood for good: MW.13's open
owner call. It waits on that call.

## CO.9

Hulls for Houses (doc 47 §4, the gap MW.1 left: "No House can be given hulls on a live world").

**Survey.**

- **How a player comes to own a hull today.** Three routes, all ending in an E4 asset title
  (`mush_economy`, `EconomicAsset { title_owner, scope }`): a Co-op charter (the Trust keeps the
  title; `+coop/buyout` passes it to the captain), the yard (`+yard/order <design>`, the bill
  delivered from a docked hold, `+yard/collect`: a `YardCommission` that lays the hull up, titles
  it to the order's beneficiary and writes the owned-hull register row,
  `tacsim:owned_hull:{hull}`, IN.9), and staff (`@asset/title`, `@asset/transfer`). Who reaches a
  hull's helm is one function, `port::coop::helm_of`: a fleet assignment, a ferry, a live charter,
  a programme row titled to the pilot, then a register ship `port::owned::titled_for` the pilot.
  `+ship/board`, `+ship/undock`, `+ship/lift`, the launch tick's re-check and `+ship/order depart`
  all ask it and nothing else.
- **What already exists for a House.** The register is keyed by the hull, not the owner, and its
  readers already read an organization's title: `titled_for` takes an `Organization` owner
  through `mush_economy::acts_for` (`economy.account.post` on its root scope, an officer, or
  staff). `+yard/order <design>=org:<org>` books for an organization an officer acts for and the
  commission titles the hull to it (IN.9's step 7, a Co-op escort). Doc 41's recovery house
  (`port::house`, RS.6a) seeds Tycho Mutual's two hulls from lane content as yard commissions with
  no work order and no kit, at `Step::Laying`, titled to the operator. Procurement's cap counts
  the commissioned ships (`surface::recovery::world_hulls` includes `owned::ships`). The scoped
  right a member flies a unit under already exists too: the unit-authority vocabulary
  (`mush_org::unit_authority`, DT.2a), whose `unit.deploy` resolves "by the title or by the
  organization path: a grant of the capability word on the org that holds the title, on a scope
  holding the title's" (`reader::authorize_in_world`; the bay's `unit.maintain_refit` is read the
  same way). `@org/grant` grants it; `@asset/list owner=org:<org>` is the House's holdings, read by
  a holder of `asset.title.transfer` or `asset.assign` on the title's scope.
- **What was missing.** Nothing makes a hull for a House. A House has no docked hold, so it cannot
  deliver a yard bill; its procurement buys mind licences, not hulls; the Trust's pool
  (`port::pools`, IN.10) restocks the Co-op alone; and the Houses are made by the bring-up's
  `@org/create`, so they are on no port map and lane content cannot name them. Three smaller holes
  behind it: (a) `commission::scope_of` finds an organization's root scope only on a port map, so a
  hull commissioned for a staff-made organization was titled with no scope, which no scoped grant
  can reach; (b) only an officer (`economy.account.post`, the treasury's spending right) reaches an
  organization's hull, so a House could not let a member fly without letting them spend; (c)
  `helm_holder` answers nobody for an organization's register ship, so a House's ship under way
  has no commander for the readers that ask (the voyage's clearance notice, space's backup seat,
  the custody and tow readers), and two members could each board her before either files.
- **What the bring-up would seed.** MW.1 lent three Co-op Drays to one House for its purchase and
  handed them back (a Dray titled to a House closes `+coop/enrol` for everybody). Each House needs
  three titled hulls for its first licence (the floor of 400 per mille).

**Built.** The smallest route: staff seed a House's hull through the yard's own commission arm,
a member flies it by a scoped `unit.deploy` grant, and the bring-up seeds every House.

- **`@tacsim/port seed <port> <design> <mark>=org:<organization>`** (staff;
  `commands::port_seed`, `port::commission::seed`). One `YardCommission` with no work order and no
  kit (the recovery house's seed identity, so the yard's `next_mark` steps past it) at
  `Step::Laying`, titled to the organization, at the named port; the commission arm's next fires
  lay her up, dock her, title her, make her hold the organization's, empty her magazines and
  write her provenance and register row. Ships only (a design with a drive). The mark is the
  caller's, `[A-Z0-9-]{1,16}`, and a mark already a commission's or a register row's is refused,
  so a bring-up typed twice seeds once. Seeds take pool indices from 128 (`SEED_POOL_BASE`, past
  the recovery house's 64), at most 128 a world (`MAX_SEEDS`). The hand-over line names the
  organization ("titled to Meridiani"), where it said "titled to you" for every commission.
- **The root scope for a staff-made organization** (`commission::scope_of`): a port map's row
  first, as before, else the organization's own record's root scope. A yard order `=org:` for such
  an organization is titled on its root scope too now, where it had no scope.
- **`owned::may_take_out(w, hull, pilot)`**: `titled_for` (unchanged: the pilot, or an officer),
  or, for a hull titled to an organization, `unit.deploy` by the organization path
  (`mush_org::unit_authority::reader::authorize_in_world` with the economy's live facts). The
  resolver's own checks are the owner key: the grant's organization must be the title's, the
  grant's scope must hold the title's, the title must stand.
- **`owned::org_flyer(w, hull)`**: who flies an organization's register ship, read from her
  sailing plan (`tacsim:voyage:{hull}`): the pilot of her filed passage, else, while she is not
  docked, the pilot of the passage she left under. Nothing new is stored.
- **`owned::helm_hulls`** filters by `may_take_out` and skips an organization's ship another
  member is flying, so the second member's helm is the House's next free hull (or none), never
  one already filed. **`coop::helm_holder`** answers an organization's register ship with her
  flyer, while they still may take her out.
- **The bring-up** (`deploy/instance/launch-bringup.py`): in the Active boot, after the NPCs, #1
  seeds every House `--house-hulls` (3) ships of `--house-design` (`dray`) at Ceres Freeport, marks
  `MER-01`..`03`, `COP-01`..`03`, `TYC-01`..`03`, and waits until `@asset/list owner=org:<House>`
  lists them ("each House holds 3 hulls of its own"). The seed step does the same before the
  purchase instead of lending Drays; the lend is gone and `--keep-house-hulls` with it.
  `return_hulls` stays, so a store an older run lent Drays on hands them back. The acceptance
  script and the bot minds lane (`scripts/bot-minds-lane.sh`, which runs the bring-up with
  `--no-systemd`) now check the line. `deploy/README.md` says so.

**Defaults taken** (provisional, the owner's economy rule):

- **Three Drays a House, docked at Ceres Freeport**, seeded at the bring-up, not bought. A House
  that should pay for its fleet is a later cut (a House yard order needs a hold to deliver from).
- **`unit.deploy` on the title's scope is the House's pilot's right.** An officer keeps reaching
  the House's hulls as before. Neither the bring-up nor this cut grants `unit.deploy` to anybody:
  who flies a House's hull is the House's (staff's `@org/grant` today).
- **One member flies a House ship at a time**, from filing to docking; at the dock she is free
  again for any member with the right.
- **The seed verb is staff's and takes the mark.** A House-facing verb would need a price.

**Expensive to reverse.** Nothing new is persisted: a seed is an ordinary `YardCommission`
document with the fields every commission has, and the flyer is read from the existing sailing
plan. The one choice with a tail: a seed is known by `work_order == 0` and an index from 128, the
recovery house's identity, so `next_mark` must keep skipping both.

**Tests.**

- **The story**: `tacsim` `tests/domain_economy/co9_house_hulls.rs`,
  `co9_a_house_is_seeded_a_hull_and_its_pilot_takes_her_out` (about 7 s; the composed lane,
  `driving`'s stocked rig). Every actor but staff is a player character; staff only create the
  Houses, seed the hull, make the grants and title the captain's locker, all by registered
  commands. In order: a Co-op captain enrols and staff title them a thing; `@org/create Meridiani`
  and `Coprates`; `@tacsim/port seed ceres_port dray MER-01=org:Meridiani`, and the same line again
  refused ("already a hull's"); the port's fires take the commission to `Done`, and the Dray is
  docked at Ceres Freeport, titled to Meridiani on its root scope, a register row under `MER-01`;
  `@org/grant` gives the pilot `unit.deploy` on Meridiani, the officer `asset.title.transfer` on
  Meridiani, a rival `unit.deploy` on Coprates; the officer's `@asset/list owner=org:Meridiani`
  lists the Dray; the deckhand (no grant) and the rival (the right word, the wrong House) have no
  helm and are refused `+ship/board`; the captain's charter is untouched; the pilot's helm is the
  Dray, they board, file `+ship/undock stickney`, are her `helm_holder` under way while the
  deckhand still has no helm, and she docks at Stickney Yards; ashore, the pilot's `+market/sell`
  of a part in her hold to the Yards' bid is offered no House hold; every title a character holds reads
  as before, and the captain's charter hull is titled as before. (The rig's clock runs the passage
  at rate 1, past the Co-op's seven real days of dormancy, so the captain's idle charter ends by
  the clock: the story asserts it ended `Dormant` or not at all.)
- **Mutations**, each on conclav.dev in a throwaway clone, each caught by the story:
  - **the title check** (`may_take_out`'s organization path made `false`): fails at "the pilot's
    helm is the Dray" (`None`);
  - **the owner key** (the seed's `beneficiary` the requester, `Object(#1)`, not the House): fails
    at the title owner;
  - **the owner's scope** (no root-scope fallback in `scope_of`): fails at "titled on the House's
    root scope" (`None`), before the grant could be tried;
  - **the hold** (`may_trade_hold` made `true`, run locally): fails at "a pilot without office is
    offered no House hold to sell from" (the reply becomes "No such listing.").
- Beside it, unchanged and green: `yard_in9`, `tycho_house_rs6a`, `in10_pools` (the yard's
  commissions, the recovery house's seeds and the Trust's pool, which share `next_mark` and
  `scope_of`), `tacsim --lib port::`, `mush-bot`'s `the_walks_are_the_worlds_rooms_and_exits`
  (reads the bring-up script), `mush-contract-tests`, clippy `-D warnings`, CI's M7 memory-size
  pass, rustdoc with `-D warnings`.
- **The bring-up** is not run end to end by this cut on a box. Its seeding runs in the Active boot,
  so the bot minds lane (`scripts/bot-minds-lane.sh`, a dispatch job of `ci.yml`, which brings the
  launch up with `--no-systemd`) types it and now fails unless every House holds its three; the
  branch's dispatch run is the proof (below).

**Divergences.**

- **Seeded, not bought.** The unit's row says "a House can own and field a hull through the routes
  a House uses"; a House's own route to buy one (a yard order paid from its treasury and delivered
  without a hold) is not built. A House is given its hulls at the bring-up by staff, the way
  doc 41's recovery house is given its pair by content.
- **`helm_of` is wider for the pilot.** A `unit.deploy` holder's helm is the House's ship, and
  every reader of `helm_of` follows it: `+ship` status and `/posture`, the plan and burn verbs, the
  market's hold reads, `+yard`, `+works`, the ship mind. The story proves the flight: her burn
  draws on the House's hold. **The market is closed to the pilot** (`owned::may_trade_hold`, read
  by `PortVenueGate::endpoint_for` and `hull_admits`): a House's register ship trades her hold
  only for one who holds her title (an officer), so a pilot without office is offered no House
  hold to sell from. (With that check removed the story's sale is still refused, later and as "No
  such listing.", by a step not traced here.) **Not checked**: `+yard`, `+works`, `+claims`, the
  passenger verbs and the ship mind, each of which takes `helm_of`'s hull; whether each asks a
  right of its own before it moves the House's goods or money is the first thing a review of this
  cut should read, and the narrow fix for any that does not is the market's. The rig title views (`title_view_for`) and the bay's rights (`bay_work::right_of`) still read
  `titled_for` alone: a pilot without office does not board the House's rigs by title or refit
  her.

**Not built.**

- A House buying a hull itself (above), and a House-facing route to grant `unit.deploy` (today
  staff's `@org/grant`; a House officer delegating it is doc 27 Q-A1's).
- Rigs: the seed makes ships only.
- **Not pinned by a story**: two members of one House each with `unit.deploy` (the second's helm
  skipping the ship the first has filed, `org_flyer` in `helm_hulls`); a House's ship in a bubble
  (the space readers that ask `helm_holder`); the House's procurement buying with seeded hulls on a
  launched world (MW.1's purchase step, now without the lend: run by the acceptance script, hours,
  not run here).

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
  (`[(819, 0), (821, 0)]`), and it fails the same way on base's own numbering, without lane
  v34's two rigs (`[(813, 0), (815, 0)]`); with it, it passes, and GC.11 passes unchanged. A probe on the
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

The defect sweep (doc 47 §5). Taken in the order the unit was given: D1, D2, D5, D6, D7, D8.
D3 and D4 wait for CO.2 and are not taken. Paths are under `crates/tacsim/` unless given.

**Survey.**

- **D1, the tonne for 3 kg, is the ledger's rule, not a defect.** The burn is charged in
  `space::tick::HullPropellantLedger::record_burn_kg`. The hold's `stow.propellant` pool is
  whole tonnes, and the ledger keeps it at `floor(live_kg / 1_000)` (`SF.R4`, `D-35-6`): the
  first kilogram of a fresh tonne debits it whole and the other 997 kg stay live in the
  ledger, which the bubble reads (`effective_live_kg`) and which survives a bubble's close
  and a restart (`SF.R11`, `SF.R21`; the cap never drops a ledger with a part-tonne owed,
  `SF.R27`). MW.21 read the pool, 9 t to 8 t; her live mass was 8,997 kg. No rounding error
  and no unit mix-up. What a player could call a defect is that the hold reads 8 t while the
  ship holds 8,997 kg; that is the integer-tonne inventory, and changing it is a design call.
- **D2.** `+ship/mind`'s read of a sealed hull was one sentence of 154 columns for the Salt
  Moth (`port::ship_mind::MIND_ABOARD`). Filled at a 32-byte hull name (`MAX_HULL_NAME_BYTES`),
  a 24-byte mind and a ten-digit licence, five more of the verb's lines passed 120:
  `MIND_PREVIEW` (215), `MIND_HOLDS_HER` (139), `MIND_VOIDED` (137), `MIND_UNFIT` (131),
  `MIND_OFF_PREVIEW` (130), `MIND_SEALED` (128), and the refused read, which joined two
  sentences on one row. A mind's name has no cap of its own, so rewording alone cannot hold
  the floor. **The absent-unit notices** (120 to 196 columns) are in `field/absent.rs`, a file
  CO.2 is changing: they are not touched here (below).
- **D5.** The retry list is `Minding::homebound` in `field/absent.rs`, a module state the host
  fills at `on_out` and never writes to the store. Every line of a fix is in that file, which
  is CO.2's.
- **D6: the flake is the test's, not the Registry's.** In `minds::bulk::registry_service`
  the ack that says "Registry work finished." sets `Phase::Finished`, and `continue_owned`
  drops the entry in the same command under the same lock; from then `holds_rating` is false
  until a new job is queued. So a `RATING_BUSY` after "finished" means a new job exists. The
  test made it and missed it: `Running::command` returns every line recorded since the
  command was sent, and on a loaded box late lines share the drain (the engine's `Queued.`
  and the driver's `Done.` for the earlier `@mind/save`, an inventory provider's "Dissolved
  lot#…", an obligation's line). The re-rate loop compared the whole drain with
  `["Registry preparation queued."]`, so a reply of `["Done.", "Registry preparation
  queued."]` did queue a job and was not counted; every later rate met that job and was
  refused `RATING_BUSY` until `STEP` (120 s) ran out. Both 160 s timeouts reproduced open with
  exactly that reply and then about 600 refusals. The test's other exact matches failed the
  same way, and a loaded world sometimes sheds a line ("The world was busy too long for that
  line; please retry in a moment."), which broke the cancel step.
- **D7.** `docs/design/45-cut-notes.md` held `## AP.11` twice. Commit `e45e47669` ("the cut note
  names the second merge of main") appended a whole new copy rather than editing the first;
  the merge of main that followed left the copies in the other order. The two agreed for 210
  lines. The first copy (the newer) had the second merge's bullet, but its last 13 lines were
  AP.10's (its CI bullet naming `mush-campaign`'s census, and a "Not verified" list naming
  `end_practice`), pasted over AP.11's own. Nothing links to the second copy's anchor
  (`#ap11-1`); doc 45 links `#ap11`, which is the one kept.
- **D8.** MW.19's item 3: the author's session reads the engine's `Queued.`
  (`mush_core::engine`, `drain_authoritative_submit`) and the world task's fallback `Done.`
  (`mush-server` `world_task.rs`, `CommandSubmitDrive::Staged`) for every submission a module
  makes as that player from a tick, the Registry's steps among them (4,294 lines a rating).
  Item 4: `+tv/<n> launch mind=filed:<licence> go` answers as a plain launch does, and nothing
  names the mind the claim seals aboard.

**Built.**

- **D2: `+ship/mind` at the floor.** A `fit` in `port::ship_mind` wraps each row at the
  crate's 118 columns (`port::site_map::wrap`, continuation rows indented two) and is applied
  to everything the verb and its arm say that carries a player's name: the read, both
  previews, and the arm's four notices (sealed, voided, refused, unsealed). The sentences
  break their own rows where a sentence ends (`MIND_ABOARD`, `MIND_UNFIT`, `MIND_PREVIEW`,
  `MIND_OFF_PREVIEW`, and the refused read), so the bot's needles each sit in one row.
  `MIND_HOLDS_HER` is said unwrapped by the yard and the refit (`bay_work/command.rs`,
  `refit.rs`, CT.7's files), so it is reworded to fit as it stands: "The {name} is held as she
  is while a mind is sealed aboard: +ship/mind off, at her berth." The bot's needles for
  `MIND_UNFIT`, `MIND_OFF_PREVIEW` and `MIND_HOLDS_HER` follow (`mush-bot`,
  `mine::minds::ship_mind`).
- **D7.** One `## AP.11`: the newer copy, its "Main moved twice" bullet kept, with AP.11's own
  CI bullet and "Not verified, and not covered" list put back in place of AP.10's lines.
- **D8, item 4: the operator is told which mind is aboard.** The surface tick's claim arm says
  `surface::excursion::CLAIM_ADMITTED` to the operator ("The Registry admits licence
  {licence}'s claim on #{tv}." and, on its own row, "{mind} is sealed aboard at Class {class}
  until the rig is home.") on the visit that acknowledges the claim's Open and submits the
  queued launch behind it (`surface::mind_claim::step`). It is said once: the Open's ticket is
  removed on that visit, and the line is said only when one was removed. Live when the
  operator is connected, queued for their login when not (`NoticeLevel::Trip`, as
  `CLAIM_REFUSED` is). A launch refused after it is still told `CLAIM_CANCELLED`. The bot's
  claim table has a row for it.

- **D6: the test matches the line it needs.**
  `mush-server` `player_market_commands::rate_queues_and_cancels_a_genuine_registry_preparation`
  reads the author's own lines (`command_seen`), looks in each reply for the one line it is
  checking, resends a line the world shed (`admitted`, within `STEP`), and waits for "retained
  and queued" and "finished" with `until_own`, which counts a line inside the command's own
  reply. Test-only; no production line moved.

**Tests.**

- **D1** `domain_world::voyage::co11_a_part_tonne_burn_debits_a_whole_tonne_and_keeps_the_rest_live`
  pins the rule with MW.21's numbers: 3 kg off 9 t debits one tonne, the live mass is 8,997 kg
  before and after the debit lands, the next 997 kg debit nothing and the kilogram after them
  opens the next tonne. It passes on main: there was nothing to fix.
- **D2** `port::ship_mind::tests::co11_every_ship_mind_line_fits_120_columns`: every sentence
  the verb says, filled at the worst case above, is at most 120 columns a row (through `fit`
  where the verb wraps, as it stands where it does not); the Salt Moth's read is the two rows
  MW.21 should have read; a 60-byte mind still wraps. On main the Salt Moth's read is one row
  of 154 and the test fails. MW.13's story (`domain_field::mw13_ship_mind`) still matches its
  lines byte for byte.
- **D8, item 4** m8_world's `claimed_launch` (the helper the licensed-launch stories share:
  `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`,
  `s4b2_a_claim_given_up_does_not_hold_the_next`, and MW.5's filed-mind story in
  `c4_filed_mind_field`) asserts the operator is told `CLAIM_ADMITTED`, filled with the
  claim's own seal, exactly once across the fires that open the claim, said or queued, and
  that each row fits 120. On main no such line exists and the assertion fails.

- **D6** on conclav.dev, at `bb6a24e8` (`claude/doc47-close-out`), 24 to 32 copies of the test
  binary at once (`--exact`), load average 40 to 89 with other sessions' builds running:
  before, 251 of 264 passed (13 failed, 4.9 %: two the reported 160 s `RATING_BUSY` timeout,
  the rest stray lines in exact matches); with the fix, 192 of 192 passed. The harness keeps
  its store in memory (`PurchaseFaultStore`), so no Redis was needed. The failure is a race of
  wall time, not a logic the fix could fail on main deterministically; the before and after
  counts are its test.

**Divergences.** None from doc 47's rows. D1 is closed by a reading and a pinning test, not a
fix (the unit's row allows "named with the reason").

**Not built.**

- **D6's neighbours.** The same stray-line flake can hit the shared `world()` launch setup
  (one run failed "one acknowledged wallet creation … got []" with the shed line in the
  reply) and every other test in `player_market_commands.rs` that matches a whole
  `command()` reply. Not touched; the same `admitted`/`command_seen` idiom fixes each.

- **D2's absent-unit notices** (`field/absent.rs`, 120 to 196 columns). CO.2 is rewriting those
  notices (R-47-2, R-47-3); a width test written now would be written against sentences about
  to change. Left for CO.2 or for a CO.11 follow-up after it lands: the same check, each
  notice filled at a five-digit unit and the longest posture word, at most 120 a row.
- **D3 and D4** wait for CO.2 (doc 47 §6, "Collisions to watch").
- **D5, the recover owed across a restart.** Left: the list and every line of a fix are in
  `field/absent.rs` (CO.2's). The fix is one of two. (a) Write `Minding::homebound` to the
  store with the field's other records and read it back at `at_resume`; or (b) rebuild it at
  `at_resume` from what survives: a unit out of the field, standing beside its carrier's
  ramp, whose rig record names an absent pilot and no recover act. (b) needs no new key and
  is the smaller; it is the one this note would take. Either wants MW.7's restart story
  extended by a reboot between `on_out` and the recover's landing.
- **D8, item 3: the engine's `Queued.` and `Done.` for a submission the player did not type.**
  Left, as too large for a sweep. The fix is in the engine's submit path: mark an
  `AuthoritativeSubmit` made outside a command dispatch (`Ctx::submit_authoritative` with no
  dispatch stamp), skip the receipt `Queued.` for it in `drain_authoritative_submit`, and skip
  the world task's fallback `Done.` (keeping a provider's own summary). `AuthoritativeSubmit`
  is a public struct with about fifty literals across nine crates' tests, the change reaches
  every story that reads a tick's lines, and MW.19's nightly job is built to go red the day the
  lines stop (`scripts/bot-minds-lane.sh`, "KNOWN DEFECT GONE"), so the marker comes out in
  the same change. It wants its own unit, with the stories' and the bot's runs.

## CO.11b

CO.11's leftovers, now that CO.2 has merged (doc 47 §5; CO.11's "Not built"). Taken in the order
given: D2's absent-unit notices, D3, D4, D5, CO.3's surviving mutation, D6's neighbours. Paths are
under `crates/tacsim/` unless given.

**Survey.**

- **D2.** Filled at a five-digit unit, `RETURN` (the longest posture word), "its pilot was away
  when the field opened" (the longest `Why::clause`) and a 32-byte station, eight of the fourteen
  notices in `src/field/absent.rs` passed 120 columns: `UNATTENDED` (198), `UNATTENDED_ON` (184),
  `BACK_IN_SEAT` (152), `LEFT_AT_RAMP` (149), `ADMITTED_AWAY` (148), `TAKEN_HOME` (124),
  `ADMITTED_ON` (123), `GONE_HOME` (122). Every name in them is the game's (a unit number, a
  posture word, a reason, a station), so unlike `+ship/mind` no fill is a player's and the rows
  can be fixed in the text itself. A ninth line of the same family, the field's restart line in
  `src/field/boot.rs`, carried a run of 22 spaces where a line continuation had been lost when
  the file was first written (`cbf9f7697`), about 160 columns.
- **D3: the order is the admission's.** At admission (`field::admit`, the open path) the seats
  nobody is at pass to their minds (`seat_absent`) and the units nobody sits in
  (`absent::unattended`), and each queues its notice then. The field's own opening line was
  queued later, by the stakes' read at the field's first snapshot (`field::notice::fold` from
  the host's snapshot branch). So a player away read each rig's line above the line that says a
  field opened at all (AP.11's queue, `[guarding, homing, opened]`).
- **D4: nothing told the aboard.** The host's pass (`absent::retry_recovers`) drops a unit from
  its owed list once its recover has put it aboard, with a log line and nothing for its player.
  `on_present`'s `WENT_HOME` is said only for a unit still owed (out, not yet aboard) whose
  pilot sits in it, so a rig nobody sat in was never told before the close's fate line, and
  neither was a seated pilot whose unit went aboard before they came back.
- **D5: what survives a restart.** `Minding::homebound` is RAM. Its `then` and its pilot were
  also held by the exit's own ask (`field::exit::ExitBook`, RAM): the exit's first try of the
  recover runs once the exit's K3 write has landed. A restart between the kernel's answer and
  that try loses both. What the store keeps: the live record's `owed_exits` (written with the
  answer, resubmitted at boot) and then the site record's `exits`; the unit's pose on the site's
  ground beside the ramp, where the check-in left it; its rig record (the excursion's slot and
  operator, and no act in flight); its seat (`mush_sim` assignments). Not kept: whether the mind
  or the player asked it out.
- **CO.3's M5 is caught; CO.3 did not run the test that catches it.** CO.3's mutation table ran
  AP.4a's two stories alone. `arena::weapons::tests::ap6_hold_and_guard_stay_where_their_mind_took_them`
  holds "Set to RETURN while its mind flies it: the way home, at once" (`objective_of` is
  `Withdraw` straight after `set_posture`).
- **D6's neighbours** (`mush-server` `player_market_commands.rs`). CO.14 is changing the
  engine's `Queued.` and the world task's `Done.` for submissions nobody typed (D8 item 3), which
  are the stray lines most of these whole-reply matches trip on, and is editing the world task's
  tests beside this file.

**Built.**

- **D2: the notices break their rows where a sentence ends.** Each of the eight is one row a
  sentence or less, the words kept except where one sentence was itself too long:
  `TAKEN_HOME`, `ADMITTED_AWAY` and `UNATTENDED` say "its mind has the controls and makes for its
  carrier's ramp." and then "It fires only on a unit that fires on it." on the next row;
  `LEFT_AT_RAMP` is "... but it could not be put aboard." then "It stands beside your carrier's
  ramp: recover it yourself."; `BACK_IN_SEAT`'s `+focus` sentence, `GONE_HOME`'s and
  `ADMITTED_ON`'s "It will not fire first." and the two `UNATTENDED` lines' plan sentences go on
  a second row. The restart line in `field/boot.rs` is two rows, its `+focus` sentence the
  second. **The reconnect queue holds one row a line** (`port_tick::NoticeQueue::push` turns a
  line's breaks into spaces, by its schema), so `absent::notify_rows` hands each row to
  `notify_budget` on its own: a two-row notice is said, or queued, as two lines in order. The
  bot's needles follow (`mush-bot` `mine::minds::absent`: `TAKEN_HOME`,
  `BACK_IN_SEAT`, `LEFT_AT_RAMP`, `ADMITTED_AWAY`, `UNATTENDED`, each inside one row), and the
  help's quotes (`docs/help/field.md`).
- **D3: the opening line is queued at admission, first.** `field::admit` reads the stakes
  (`notice::fold(ctx, site, true)`) on the live record it has just written, before
  `seat_absent` and `unattended`. The first snapshot's read then finds every unit known and says
  nothing. A connected stakeholder elsewhere hears the opening a pulse sooner; the site's own
  line is unchanged.
- **D4: `PUT_ABOARD`.** "Its mind took #{unit} out of the field at its carrier's ramp, and it is
  aboard her now." When the host's pass finds an owed unit aboard it tells the player it ran the
  recover as (`notify_budget`, trip level: said to one who is connected, queued for one who is
  not), seated pilot or operator alike. The close still reports the unit home. The bot has a row
  for it.
- **D5: CO.11's option (b).** `absent::rebuild_homebound`, called at the end of
  `field::boot::resume`, owes a unit its recover again when it left the field at the ramp (the
  live record's `owed_exits` or the site record's `exits`), is out of the live record and on the
  site's ground, is not a carrier, has a recover (`recover_of`: the DT recover of a roster's unit
  or the rig's own act) and no act in flight on its rig record, and its player (the pilot who
  sits in it, else `posture::player_of`) is not connected. From there the pass is unchanged:
  tried again every ten seconds, behind the boot's resubmitted exit (`exit::leaving`),
  `PUT_ABOARD` when it lands, `LEFT_AT_RAMP` after thirty tries.

**Tests.** Each failed before its fix. The stories' "before" is the branch with the three
production fixes taken out (the D3 `fold`, the D4 notice, the D5 rebuild), its tests kept, on
conclav.dev (Linux), a throwaway clone of main `aa3e1e4c3`.

- **D2** `field::absent::tests::co11b_every_absent_unit_notice_fits_120_columns`: every one of
  the fifteen notices, filled at the worst case above, is at most 120 columns a row. On main it
  fails at the first: "124 columns: While you were away #99999 passed to its mind, which makes
  for its carrier's ramp and fires only on a unit that fires on it." AP.11's story holds the
  queued rows to 120 as well, which the unit test cannot: its first run on the branch failed
  there, with every row flattened back to one line by the queue (above).
- **D3 and D4** AP.11's closing story (`domain_field::field_ap11`), step 5: what is queued for
  the owner is, in order, the field's opening, the guard's two rows, the prospector's two rows
  and `PUT_ABOARD` for the prospector, and he reads exactly those rows under the heading. With
  the fixes out it fails there: queued `[guarding, homing, opened]`, the opening last and no
  aboard line.
- **D5** MW.7's new story 7,
  `field_mw7::co11b_a_restart_between_the_ramp_and_the_recover_still_puts_the_unit_aboard`: a
  clean QUIT, the mind takes the escort out at its ramp, and the story reboots at the kernel's
  answer, with no act on the rig record yet; after the boot the host puts it aboard and the
  pilot reads `PUT_ABOARD` once at login. With the fix out it fails: "the host puts the escort
  aboard its carrier after the restart: pose Some((129, 125)), act None".
- **CO.3's M5** (`Arena::set_posture` no longer calls `lay_standing`), on Windows: caught by
  `arena::weapons::tests::ap6_hold_and_guard_stay_where_their_mind_took_them`
  (`weapons.rs:3130`, `objective_of` after `set_posture(RED, Return)`), the only one of the
  733 `arena::` and `field::absent` tests to fail. No new test: the existing one fails under
  the mutation, and CO.3's table ran only AP.4a's stories.
- AP.6's, AP.7's (whose two login checks read a notice whole and needed the row-by-row queue)
  and MW.7's other stories pass unchanged. `mush-bot`'s census, in step and distinct, passes
  with the new needles (28 tests).

**Verified, and how.**

- conclav.dev, the branch at its head: all of `domain_field` (312 passed, 16 dispatch-only
  ignored, 780 s at 12 threads), the stories above alone, `tacsim --lib` (2,357 passed),
  `domain_world` (1,132) and `domain_arena` (273).
- Windows: `cargo clippy -p tacsim -p mush-bot --all-targets -D warnings`, the lint job's M7
  memory-size pass (with `.github/clippy-m7`), `cargo doc -p tacsim -p mush-bot --no-deps` with
  and without private items under `-D warnings`, `cargo test -p mush-contract-tests`
  (199 passed, nothing re-blessed), `cargo fmt -p tacsim -p mush-bot`.
- No mind API, store schema, persisted record or `MIND_ABI` moved. Nothing ran against a live
  server; nothing was deployed.

**Divergences.**

- **D5 cannot tell whose exit it was.** A player who took their own unit out at the ramp and
  logged off before recovering it is, after a restart, recovered by the host as them, into the
  bay slot it launched from, which is where their own recover takes it; before the restart the
  host would have left it. A default: low to reverse (store the asker on the live record's
  `OwedExit`), and it would need a new field on a persisted record.
- **A two-row notice costs two of the queue's 32 lines.** The notice budget's count in
  `field::notice` is per line, so an absent player's units now spend up to twice what they did
  (the queue drops the oldest past 32). Low to reverse: one notice a line again, at the cost of
  the floor.
- **The rebuilt reason** is `LinkLost` for a seated pilot and `Unattended` for a unit nobody
  sits in: what a hull put back into the field is handed over under (`owed_why`) reads it. The
  true reason is not kept.

**Not built.**

- **D6's neighbours.** Left for after CO.14 (above): the stray `Queued.` and `Done.` it removes
  are most of what these matches trip on, and rewriting the matches now would be written against
  drains about to change, beside CO.14's open edits to the world task's tests. The same
  `admitted`/`command_seen` idiom fixes each that still fails after it.

## CO.12

Saving an open bubble costs what changed (doc 44 MW.20, "Found, not fixed" item 1).

**The short of it.** The space tick with eight bubbles open, back to back, went from 5.3 ms
at the median to 1.2 ms, and its store writes from 487 KB a tick to 152 KB. They are more
documents and not fewer: 8.1 a tick before, 23.7 after, because a stepped bubble now writes
its record and the one snapshot the step took, and deletes the one its ring let go. The store
layout for an open bubble changed; a store written by main reads as it did, and its first save
after writes it in the new shape.

### What was measured first

The MW.20 space lane (`minds_at_size::mw20_eight_bubbles_think_on_the_pool`), with one thing
added: it now counts the space book's writes (`tacsim:space:*`) and their encoded bytes
apart from every module write. On the base (`6c86731fb` with that counter only), one bubble's
record is 63,437 B; a clone of it takes 33 µs and `serde_json::to_value` 311 µs. Eight
bubbles step every second, so the tick spent about 2.5 ms encoding and 0.27 ms copying. The
encoding was the cost, not the clone, which ruled out a dirty flag alone: every open bubble
steps every second, so every record is dirty every tick.

What the record is made of explains it. `ActiveInterdictionBubble` holds the live bubble,
its tape's opening snapshot, and a ring of up to six snapshots, and each step pushes a
snapshot (`push_bounded_snapshot`). Eight copies of the bubble, seven of which do not change
once taken. So the record is written apart from its snapshots, and each snapshot once.

### What was built

- **`crates/tacsim/src/space/save.rs`** (new). `save_bubbles` diffs the live bubbles against
  the last save's copies in place and writes:
  - the record without its snapshots at `tacsim:space:bubble:{domain}` (the key it had),
    when the record changed or its snapshots' slots did;
  - each snapshot not yet saved at `tacsim:space:bubble_snapshot:{domain}:{slot}`, once;
  - a delete for each slot the record no longer names.

  A live snapshot keeps the slot of an equal one already saved (the hash first, then the
  whole), so the ring sliding by one is one set and one delete. A new slot is never one the
  saved record still names, so no write of a batch touches a document the stored record
  reads. The comparison and the copy name every field of the record (`same_core`,
  `copy_core`): a field added later is a compile error there until it is handled.
- **`persist_space_state`** (`space/tick.rs`) no longer clones the bubbles. It lifts them
  out of the book while the rest is copied, takes the last save's bubbles out of the cache,
  and hands both to `save_bubbles`. The cache is still the whole book as the store holds it
  (`TacsimStore::space_state`), so voyage's "`persisted.book == book_before_1`" holds.
- **`#[serde(skip_serializing_if = "kept_apart")]`** on `ActiveInterdictionBubble::snapshots`
  and `BubbleInputTape::initial_snapshot`. `kept_apart` reads a thread-local that is set
  only while `save_bubbles` encodes a record, so every other encoding is whole, as before.
- **Reading.** `load_space_kv_entry` holds a bubble document that names its `parts` and
  each snapshot document; `finish_space_kv_load`, called after the loop in both loaders
  (`boot_read` and `store.rs`'s), puts each snapshot back into its record's JSON and
  decodes the record as before. A record with a snapshot missing or unreadable holds its
  domain (`quarantined_bubbles`), as an unreadable record always did. A snapshot key that
  does not parse holds the space state, as for any other `tacsim:space:*` key.

### The layout, plainly

| Key | Shape |
|---|---|
| `tacsim:space:bubble:{domain}` | `PersistedBubbleDoc { format, domain, bubble, parts }`. `bubble` has no `snapshots` and its `tape` no `initial_snapshot`. `parts` is `{ initial: slot, snapshots: [slot, ...] }`, the ring's slots in order. Two equal snapshots may name one slot. |
| `tacsim:space:bubble_snapshot:{domain}:{slot}` (new) | `PersistedBubbleSnapshotDoc { format, domain, slot, snapshot }`, `deny_unknown_fields`; `snapshot` is a `BubbleSnapshot` as before. |

`SPACE_STATE_FORMAT` stays 1. A store written by main has the whole record under the
bubble key and no `parts`; it decodes as it always did, and the next save writes it apart
(every snapshot, the record with its parts). Read the other way, a binary from main would
hold the space state on the first `bubble_snapshot` key it met (an unknown `tacsim:space:*`
key does that today), so a server rolled back across this change needs those keys cleared.
Nothing is persisted that players depend on.

### Figures

conclav.dev, release, as `ghrunner` in a throwaway clone, `MW20_PACE_MS=0 MW20_POOLS=2`,
three runs each of 120 ticks with all eight bubbles open, one-minute load average 7 to 10
(other sessions were building). ms p50/p95/p99/max.

| | Space tick, run 1 | run 2 | run 3 | Space writes a tick | Bytes a tick |
|---|---|---|---|---|---|
| before (`6c86731fb`) | 5.33/8.36/9.05/11.79 | 4.94/6.90/7.50/7.71 | 5.61/7.78/9.82/11.85 | 8.1 | 486,544 |
| after | 1.40/2.29/2.48/7.73 | 1.22/1.36/1.68/2.04 | 1.14/1.23/1.60/2.06 | 23.7 | 151,978 |

MW.20 read the base's tick at 4.0 ms at `fb39c2b5`; the 5.3 here is the same lane a week
of code later on a busier box. The rest of the second's ticks are unchanged (2.2 to 2.6 ms).
Steps 960 of 960, thinks 1,920, held steps 0 in every run, before and after.

### The stories

- `voyage::co12_a_restart_between_two_steps_restores_the_saved_bubble`: two customs
  pickets open (`flown_into_pickets`), stepped until the rings are full and sliding,
  restarted over the kept store (`reboot_with`); the bubbles read back equal the world's,
  thinks owed, snapshots and tape included. Again after the crew's `+helm/heading` between
  two steps (a change to the record with no snapshot taken), and again after one more step.
  Each round checks the store holds exactly the snapshot documents the records name.
- `voyage::co12_a_store_written_before_the_cut_reads_and_is_written_apart`: the record put
  back whole, as main writes it, with the snapshot documents removed; it reads back the
  same, its first save writes it apart, a step's save after that is two sets and a delete
  and under half the whole record's bytes, and a restart brings it back.

Mutations, each run on conclav.dev against both stories:

| Mutation | Result |
|---|---|
| M1: a new snapshot's document is never written | both fail (the restart reads no bubble back) |
| M2: a changed record is not marked dirty (`core_changed` only for a new bubble) | the restart story fails at round 2: the heading order's tape entry is lost |
| M3: a slot the ring let go is never deleted | both fail (a stray snapshot document; `(2, 0)` for `(2, 1)`) |

M2 survived the first form of the restart story, which gave no order: a step always changes
the slots, which writes the record anyway. The order between two steps is why the story has
one.

Every existing tacsim test passes unchanged (`cargo test -p tacsim`, conclav.dev).

### Found, not fixed

1. **A closed bubble's summary is still copied and compared every tick.**
   `SpaceBubbleBook::completed` keeps up to eight `CompletedInterdictionBubble`s, each with its
   whole tape and snapshots, and they ride the auxiliary document. `persist_space_state`
   clones the book (now without the open bubbles) and `without_per_entity_maps` clones it
   twice more for the auxiliary compare. On a server that has closed eight bubbles that is
   about 64 bubble copies cloned three times a tick, and the auxiliary document is written
   whole when any of it changes. The MW.20 lane closes no bubble, so this is not measured.
   The same treatment (summaries in documents of their own, compared in place) fits.
2. **A step's record still carries the live bubble that its newest snapshot also holds.**
   The snapshot is taken at the end of the step, so the two are equal unless an order lands
   after; a record that named the snapshot for its live bubble would halve a step's bytes
   again. Not done: the record would then depend on its newest snapshot to read at all.

## CO.13

Selling a stored core (R-47-5's "or sold"). Built 2026-11-14 on
`claude/co13-managed-core-sale-0231e0`, cut from `26a539b5d`.

**Survey.**

- **Why no path sold one.** A governed core's title is a managed title (its birth is a native
  core integration, `managed_title::ManagedTitleIndex::core`). Every generic entry point,
  contract settlement included, reaches `require_unmanaged_mutations`, which refuses
  `TransferTitle` (and Title, Encumber, Release, SetCatalog, Abandon, WriteOff) on one. Only a
  closed plan proof under its own `PlanGuard` arm (`managed_mutations`) is exempt: the charter
  transfer, the pending charter and the Registry seizure. CO.4's `disputed` contract was the
  generic settle plan meeting that wall.
- **The pattern.** The charter transfer's economy leg (`registry_filing::charter_transfer`) is
  one owner-participant action with a closed request, a private proof (exact plan, before-title,
  birth, op shape), a native install that writes complete journal/account/title images in the
  coordinator union, and a receipt audit folded from the journal and replayed at boot. The
  Registry seizure (`registry_filing::seizure`) is the same shape on a managed **core**, with a
  tacsim producer (`minds::registry_inspection::seizure`); this unit follows the seizure's
  structure and adds the charter transfer's two money lines.
- **What the charter transfer has that a core needs too, and what it lacks.** A charter is
  title only. A core is also a world object in the seller's hands, and no native economy install
  moves an object (custody effects are refused there). Only the kernel writes world objects in
  a union (`core_delivery`, `core_exchange`, `physical_transition`, each a `BuiltinObjects`
  image). The completed-core delivery is the nearest: three object pre-images, the real carry
  hooks, a record family. The sale is that delivery run the other way.
- **Who buys, and from what money.** The fitting shop's integrator (`CorePaymentPolicy::
  integrator`: Basin's factory scope and account in the M8 world). Its till holds only the
  integration and fitting fees it has earned; on the launch world, after the story's four
  integrations and three fittings, 2,000cr against a 4,000cr buy-back.

**Built.**

- **Kernel** (`mush-core`, new `core_sale.rs`): the fixed action `tacsim/sell_stored_core` v1;
  `CoreSalePolicy` (the trusted policy names the core, the seller and the shop, read at prepare
  and again before custody: a fence refuses unless both reads agree); `SaleObjects` (the core
  from the seller's contents into the shop's, three exact pre-images plus both ancestries, the
  registered carry hooks); a `core_sales` record family keyed by the operation, with `audit`
  for the producer's boot pass. One variant on the closed `BuiltinObjects` list and one
  `PrepareCtx::sell_stored_core` beside `deliver_core_output`.
- **Economy** (`mush-economy`, new `registry_filing/core_sale{.rs,/install.rs,/receipt.rs}`):
  `CoreSaleConfig { action, vendor: ScopedCoreAccount }`; the closed request `RegistryCoreSale
  { core, title_revision, seller_account, price }` (the seller is the actor); the plan (the
  vendor's account pays the price, the seller's own credit account receives it, one
  `TransferTitle` to the vendor org); `ClosedCoreSalePlan` (exact plan, before-title, core
  birth, exactly those two lines and that one op, no hold or custody op); the native install
  (the charter transfer's, with the seizure's fold); `CoreSaleReceiptAudit` (receipts by
  commit, observed per journal entry, finished and checked against the managed-title index at
  boot, checkpoint coverage as the seizure's). One variant each on `EconomyRequest`,
  `EconomyOutput`, `EconomyPatch`, `BoundEconomy` and `PlanGuard` (the lead told CT.9).
  `EconomyModule::with_core_sale`; `EconomyStore::{is_managed_core, core_sale_receipt,
  core_sale_receipts}`. `require_unmanaged_mutations` is not touched.
- **Industry**: `CoreRefitFactoryConfig::stored_core_destroyed` (CO.4's `displaced_by` and
  `destroyed`, read-only).
- **Tacsim** (new `hardware/sale.rs`): `+core/sell <core>` (the quote) and `+core/sell
  <core>=go` (the sale), one registered action whose provider binds the economy leg and the
  kernel image in one union; the offer projected to a character standing in the shop; the
  receipt for the seller. The one read (`quote`) refuses, each in its own line: not at the shop,
  not a governed core, fitted (the core is a hull's fitted core), not held (anywhere else),
  not the seller's title alone (another's, a claim, impounded), destroyed, no catalogue price,
  the shop's till short, no credit account. The boot pass (`boot_audit`, `on_all_loaded`) pairs
  the kernel's records and the economy's receipts one to one by operation, core and seller.
  Composed by `HardwareDeployment::buy_back_permille` (and `HardwareCoordinates`, skipped when
  absent): `GovernedHardware::configure_economy` and the roster policy add the economy leg,
  `configure_tacsim` the provider and the command's slot.
- **Market**: the three core catalogue entries (pack `tacsim.cores`: `tacsim.core.doctrine`,
  `.line`, `.command`, `asset`, baselines `CoreClass::finished_minor`), loaded by the port
  applier beside the goods. `+market/sell` (and `mush_market::sell`) refuses a managed core
  outright (`MANAGED_CORE_SALE`): with a catalogue price a core would otherwise go into the
  custody-only offer the lead ruled out.
- **Contract tests**: `+core` pinned `authoritative` in `legacy-handlers.toml`.

**Defaults taken (provisional).**

1. **The buy-back rate is 500 permille** of the catalogue baseline (`BUY_BACK_PERMILLE`): 1,500cr,
   4,000cr and 12,500cr for Class I, II, III. **Cheap to reverse** (a constant, or the
   deployment's number).
2. **The buyer is the integrator**, paying from the account the integration and fitting fees go
   into, and the sale refuses when that till is short. Nothing refills the till but the shop's
   own fees. **Cheap to reverse.** Flagged: a shop that has done little work buys little.
3. **A damaged core sells at the full buy-back rate.** The condition is carried with the core
   (CO.4) but does not price it. **Cheap to reverse** (a factor in `quote`).
4. **The core goes into the shop's completed-work container** (`CorePaymentPolicy::
   output_container`), titled to the integrator's org in no scope. What the shop does with it
   next (refit it into a customer's hull, sell it on) is not built.
5. **The seller stands in the shop** (the factory object, as every fitting).

**Persisted shapes.** No existing record changes and no `.snap` moves. Three additions:
the economy fold (the checkpoint body) gains `core_sale_receipts`, skipped while uninitialized,
the seizure and bay audits' form. As theirs, the boot closes it on every world, so every
checkpoint written after this lands carries it (`{"receipts":{},"complete":true,...}` where
nothing was sold); a checkpoint without it still loads (serde default), and `e2b`'s format-3
pin strips it as it strips those two. The fold's charter-transfer audit is now boxed (its JSON
is unchanged) to hold `load_boot_with_coverage` under the M7 stack bound, which the new field
had taken 87 bytes over. The kernel gains the tacsim-owned `core_sales` action-state family,
written only by a sale. `HardwareCoordinates` gains an optional field, skipped when absent, so every
deployment that names none keeps its bytes and digest.

**Tests.**

- `mush-economy` `registry_filing::core_sale::tests` (engine level, as the seizure's):
  `co13_the_closed_plan_refuses_every_mismatch` (nine plan changes, a moved title, three wrong
  births); `co13_no_generic_transfer_touches_a_managed_title` (the exact sale plan through the
  generic guard is refused; `require_unmanaged_mutations` refuses its transfer and passes the
  same transfer of an unmanaged object); `co13_a_sale_posts_replays_and_a_forgery_is_refused`
  (money and title through the engine, the receipt, the replay, a forged price, a third line, a
  repeated commit, the seller paid elsewhere); `co13_another_holders_or_a_claimed_title_is_refused`;
  `co13_the_prepare_read_refuses_what_the_seller_may_not_sell` (the participant's own read: another
  holder's core, a claimed core, a payout account not the seller's, a zero price).
- `tacsim` `hardware::sale::tests::co13_the_lines_fit_and_the_entries_are_the_class_prices`.
- **The story**, `tacsim` `domain_field` `c4_hardware::co13_a_stored_core_is_sold_with_its_title`
  (the launch world, about 11 s): four cores built and fitted as CO.4's story does; the fitted
  core, the destroyed core and a core another character holds each refused (quote and `=go`),
  nothing paid or moved; refused away from the shop; refused while the till is short, then
  staff top the till up from the world's faucet (the M8 world's standing practice, as the
  wallet's); the stored Class II core quoted and sold for 4,000cr: the title is Basin's,
  unencumbered, one revision on; the core is in the shop's container; the pilot is paid and
  the till pays; one economy receipt and one kernel record; `+market/sell` of a core refused;
  a restart replays both audits, the title and the core's place read as before, and the sold
  core is not the pilot's to sell again.
- **On base** (`26a539b5d` with only the test files applied) the story does not compile: no
  `HardwareCoordinates::buy_back_permille`, no `EconomyStore::core_sale_receipt(s)`, no
  `tacsim::hardware::sale` (11 errors). The economy tests are a new module over new code.
- CO.4's and MW.5's stories beside it pass unchanged.

**Mutations**, each run on conclav.dev (`co13-muts.py`, in this group's notes folder):

| # | Mutation | Caught by |
|---|---|---|
| P1 | the closed check drops plan equality | `co13_the_closed_plan_refuses_every_mismatch` (another actor, no revision fence; survived until those two changes were added) |
| P2 | the sale's `PlanGuard` arm removed (it falls to `require_unmanaged_mutations`) | the story: the sale is refused |
| P3 | the vendor's auth admits another holder's title | `co13_another_holders_or_a_claimed_title_is_refused` |
| A1 | the fold ignores the family | `co13_a_sale_posts_replays_and_a_forgery_is_refused`, the story |
| A2 | the receipt does not check the seller's line | the forgery test (the seller paid elsewhere; survived until that forgery was added) |
| A3 | the boot never closes the sale audit | the story (the restart reads no receipt) |
| R1 | a destroyed core is bought | the story |
| R2 | a fitted core is not told apart | the story (it is refused as "not holding") |
| R3 | the holder check is gone (a core in anyone's hands is offered) | the story (the kernel image refuses it, unsaid) |
| R4 | the economy's read admits another holder's core | `co13_the_prepare_read_refuses_what_the_seller_may_not_sell` (survived until that test was added: the engine's auth and the quote both refuse first) |

**Divergences.**

- **The buy-back vendor is the integrator by composition, not a market `Vendor` record.**
  CO.4's attempt registered Basin as a `+market/sell` vendor by staff (`@market/vendor create`,
  a vault, a venue). The closed family names its one buyer in `CoreSaleConfig`, from the
  hardware's own payment policy, so nothing staff stand up decides who may buy a managed title.
  The market's part is the three catalogue entries, which price the core.
- **`+market/sell` now refuses a governed core.** Not asked for, but the catalogue entries would
  otherwise give a core a price there, and that path is the custody-only sale the lead ruled out.
- **The tacsim side lives in `hardware/sale.rs`**, beside the hardware composition that owns the
  integrator, the shop and the refit it reads, not under `minds/`.

**Not built.**

- **What the shop does with a bought core.** It holds it, titled, in its container. Fitting it
  into a customer's hull needs the shop to be the refit's requester and titleholder (the refit
  offers a core to its owner); reselling it needs a market listing for a managed title, a second
  closed family. Neither is asked for here.
- **A sale to anyone but the fitting shop.** The family names one vendor by composition.
  A player-to-player core sale is the charter transfer's two-step offer/accept shape over this
  plan, not built.
- **The industry `bay_refit`'s removed parts** stay as CO.4 left them (no player route).
