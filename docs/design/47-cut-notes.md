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

Not built.

## CO.3

Not built.

## CO.4

Not built.

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
