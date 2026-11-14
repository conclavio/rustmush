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

## CO.12

Not built.
