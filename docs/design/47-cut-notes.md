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

Not built.

## CO.11

Not built.

## CO.12

Not built.
