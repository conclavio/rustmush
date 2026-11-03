# 23. IC sorties — the seal epoch, the voyage, and the hosts that open and close one

> **2026-10-28 design update:** [doc 27](27-ic-field-operations-design-and-survey.md) aligns
> detachments, IC combat and salvage under the owner's new rulings. `+sortie` and “sortie” for
> the player's operation are approved, superseding Q-23-5's blanket wording ban; the technical
> hull epoch remains distinct. Recoverable disablement must not be equated with permanent
> destruction: accepted Q-H7/Q-H9 keep a recoverable unit's epoch open. The as-built epoch ledger
> below is preserved; this note does not claim that code has changed.

**Draft 0.9, 2026-11-03 — the owner accepted Q-23-7 to Q-23-20 at their defaults the same day, so D-23-11 to D-23-20 stand as ruled and S.4b-T may start; §17 custody for moving anchors (the S.4b precondition) and S.4b restated; S.0–S.4a as built (§14), S.4b open.** Draft 0.8 added §17, written read-only at `c83c2eff` (`origin/main`) under doc 38 Q-38-21: D-23-11 to D-23-20 and Q-23-7 to Q-23-20, and the cut S.4b restated with the host moved from the drone phase to the IC field (§17.10). Draft 0.3 was written read-only at `563259b7` (`main`, with 17g cuts C0–C2 merged);
Draft 0.1 was reviewed the same day against doc 17, doc 20, 17f, doc 18 and the IC-sortie code,
Draft 0.2 carried the review's corrections (§15), and Draft 0.3 carries the owner's rulings of the
same day (§16): Q-23-1 to Q-23-5 accepted as proposed, Q-23-6 ruled (the lapse hold runs to the
close of the top-level passage), risk 1 ruled (a match is a simulator concept, not an IC one), and
the narrowing of D-23-3 the Q-23-6 ruling implies. Nothing in the tree changed. This note
answers one question the owner asked on 2026-10-27 — *what do we need for IC sorties?* — and the
sharper one that followed it: a player might buy goods at Phobos, fly to a body and mine it for a
while, fly to an asteroid field and run drones, land on a second surface and shoot another
faction's miners, load back up and fight a ship on the way in, then dock at Ceres to sell and set
up milling orders. **Is that all one sortie?** It is not, and the reason it is not is the whole
design: doc 17's sortie is the custody epoch of **one hull** between two hangars, not the player's
trip. Everything below follows from taking that seriously.

Doc 17 §22 gates M8 (contraband and procurement) on "IC sorties exist", and 17g §4 adds House owner
attestation as an M8 prerequisite. This note says what "exist" has to mean for that gate, which
code already does the work, and the smallest cuts that make the first IC sortie real. It decides
nothing about fights on real ground: that is 16c H1's conversation and doc 20 §17's promotion, named
in §8 and left there.

The lead's proposals here are D-23-*n*; the owner's questions are Q-23-*n* with a default each. Per
the owner's standing word on provisional defaults, a default stands until ruled and never holds a
build.

## 1. Where the word came from, and why it looks simple

The arena is where "sortie" was first built, and there it coincides with a mission: the pod is the
hangar, `BOARD` then `STARTUP` is the hangar exit, the match fills the sortie, and custody returning
to the pod closes it (doc 17 §12; 17b cut M3.3). The arena is a proving ground for developing ground
combat while the rest of the world is built, so that shape was enough for it.

Doc 17's rewrite R-B already generalised the word past the arena (D-UM-45): the seal is per sortie,
"from the startup that takes a unit out of a hangar or base to its power-down in a hangar or base
(the same or another), or its destruction or recovery. A power-down in the field does not end it; a
pilot's exit and a licence lapse never do, and an engagement or a match on an IC map never ends it."
Three things in that sentence are what this note pins down: it is **per unit**; it is bounded by
**hangars**, which doc 17 never defines; and it is a **custody and record boundary**, not a thing a
player does. The arena hid all three because a match has one hangar, one unit per seat and one
reason to be out.

## 2. Terms

- **Seal epoch** — the engine's "sortie": one hull's custody epoch from its dispatch out of a
  hangar to its arrival back in one, its destruction, or its recovery. The seal (`MindSeal`, 21
  fields, 17b cut M3.3) is computed once at the dispatch and holds for the epoch; the black box
  opens it with `MindSealed` and closes it with `SortieClosed`
  (`crates/tacsim/src/arena/ledger.rs:579,607`); its id is a `SortieId`
  (`crates/tacsim/src/arena/mind.rs:293`). "Sortie" stays the code's word; this note says *epoch*
  where the distinction from the player's trip matters.
- **Voyage** — the player's trip: Phobos to Ceres with everything in between. Many epochs, on
  several hulls, carried on a ship. Not an engine object in this note; it is the shape a contract,
  a cover policy and a player's screen will want, and §8 names it as a later design.
- **Engagement** — a fight inside an epoch. It never opens or closes one (D-UM-45). In the arena the
  match lies inside the sortie and `MatchRecord.minds` carries the seal of the sortie it lies within
  (D-UM-46); on IC ground, doc 20 §17's promotion of a site into an arena is the same relation.
- **Hangar** — one of two placements, the only two in which a hull is "in": a vehicle or drone
  placed `Interior` to a bay compartment of its carrier (`SpatialParent::Interior`,
  `crates/mush-sim/src/data.rs:697-724`), or a carrier hull berthed at a **hangar port**
  (`Berth::Docked` at one of the corridor's five ports — Ceres Freeport, Stickney Yards, Aitken
  Highport, Voltaire Anchorage, Ligeia Terminal — as `port::ship::berth_of` decides over
  `mush_sim::pose_of`; a docked or settled hull is `Direct` on the port map's domain, so no
  container test can find it, and the port entry's declared kind decides). Content declares the
  bays and which port entries are hangars, and the set grows: the five ports and the bays are the
  hangars **today**, and player-built hangars and bases and further ports join it as content
  declares them (the owner, 2026-10-27). A mining or field site is a port entry for the port
  map's sake (doc 20) but **not a hangar**: a Dray-P settled on its pad is a hull landed in the
  field, and D-UM-45's "a power-down in the field does not end it" applies to it.
- **The field** — anywhere else: a lane leg, a site's ground and the carrier settled on its pad, a
  rock in a field, a landed hull's own ramp. A power-down there is a pause, not a close.
- **Recovery** — a hull brought into a hangar by other means than its own drive: towed, carried,
  salvaged, or walked home by staff (`surface::recovery`'s `Home`).
- **Abandonment** — a hull in the field whose hangar is gone: its carrier destroyed, or its
  excursion closed by staff with the rig left out. A state inside the epoch, never a close (§4).

## 3. The rule — D-23-1

**A hull's seal epoch is open exactly while the hull is in neither hangar placement.** So:

- The epoch **opens** when the hull leaves its hangar, not when the order is given. For a vehicle
  that is the end of the launch (`LAUNCH_S`, 300 IC s, `crates/tacsim/src/surface/excursion.rs:72`)
  when the rig is placed `Direct` on the site's domain; an excursion that ends `NeverLeft` opens no
  epoch, and a scan, sounding or drive typed from the bay that implies a launch
  (`ActParams::launches`) opens one exactly as `+tv/launch` does. For a ship it is the port tick's
  launch arm calling `mush_sim::depart` (`crates/tacsim/src/port/voyage.rs`), not `+ship/undock`,
  which only burns and files the plan; a burn whose launch never lands lapses after
  `LAUNCH_GRACE_FIRES` and opens nothing. For a drone it is the drone bay's launch (doc 20 §17).
  The seal is computed there, ahead of the first startup (doc 17 §12), and the pool admission
  happens there (D-UM-36; §6).
- The epoch **closes** on arrival in a hangar, destruction, or recovery. The close kinds are the
  ones the IC claim protocol already defines (`IcCloseKind`,
  `crates/mush-roll/src/commerce/ic_claims/owner_protocol.rs:16-21`): `ReturnedToBase { base }`,
  `PoweredDownInBase { base }`, `Destroyed`, `RecoveredToBase { base }`. The arena's
  `SortieClosed.word` is `match_end` or `destroyed` today (`ledger.rs:601-613`); on an IC host the
  word is the close kind's. **This amends D-UM-45 in one respect**: a hull's epoch closes when it
  is *back in* the hangar (`ReturnedToBase`), not at a later power-down; `PoweredDownInBase` is
  kept for a hull that powers down where it already is. S.0 carries the amendment.
- Nothing else closes it. Not a pilot leaving the seat, not a licence lapsing, not an engagement, not
  a landing in the field, not a staff `Close` of an excursion that leaves the rig out
  (`surface/recovery.rs`), not the IC-return rule putting a player in the OOC area
  (`crates/tacsim/src/ic_return.rs`: a player who steps out leaves their hull where it is).

**Nesting is a record, not an object — D-23-2.** The engine keeps no tree of epochs. A `MindSealed`
line records the hangar the hull left (the bay's or the berth's id) and, when that hangar is a hull,
that hull's **open** `SortieId`; the close records the base it closed in, which the claim
protocol's `base` field already carries. A voyage can be reconstructed from the lines; the engine
never needs it. The carrier's epoch is open while its vehicles are out because a site is the field
(D-23-3), which is also what lets a lapsed licence hold across excursions (D-23-9).

**The declared hangars — today the corridor's five ports and every bay — are hangars; a site is
the field, and a ship landed in the field never ends its passage — D-23-3** (Draft 0.2 made every
port entry's berth a hangar; the owner's Q-23-6 ruling narrows it and the owner confirmed the
narrowing, §16; player-built hangars and bases and further ports join the declared set later). The five
ports (`ic_return.rs`) are hangars. The mining sites (doc 20: "a vendor-less port entry over a
2.56 km atlas region"; the Dray-P settles on its pad as `Berth::Docked`, and `+tv/launch` is legal
only while it is settled) and doc 20 §17's field sites are port entries for the port map's sake
and the field for the epoch's. So a vehicle's epoch is short — one excursion, from the launch to
the recover, bounded by SM.11's idle return (`IDLE_PARK_IC_SECS`, 24 IC h) unless the rig is
`Parked` and waits for staff (doc 20 §8) — and a drone's is bay to bay, while **a ship's epoch is
its whole passage between hangar ports**: one leg is under a day at the constant sky by doc 18 R3
and up to about 65 h on the coasting law (doc 18 §3 Q(5), D-18-50), and a Belt voyage strings
several legs and several sites into one epoch of IC weeks. A body with no port entry is the field
too, and today a hull cannot arrive at one anyway (doc 20 §2: "a hull arriving anywhere without a
port map is stranded").

## 4. The owner's trip, by epoch

| Leg of the trip | Hull | Epoch | Opens | Closes |
|---|---|---|---|---|
| Buy goods at Phobos | — | none | | |
| Fly to a body and settle on its site | Dray-P | 1 | `depart` from Stickney's berth | stays open: the site is the field |
| Surface-mine "for a while" | TV | 2, 3, … one per excursion | the rig leaves the bay | `+tv/recover` into the bay |
| Fly to an asteroid field | Dray-P | 1, still | — | — |
| Drone-mine in space | each drone | one per launch | the drone bay | the drone bay, or `Destroyed` |
| Land on a second surface, shoot the other faction's miners | Dray-P; then the TV | 1; then the next TV epoch | —; bay | —; bay — the fight is an engagement inside the TV's epoch |
| Load up, fight a ship on the way to Ceres | Dray-P | 1 | — | `ReturnedToBase` at Ceres — the fight is an engagement inside epoch 1 |
| Sell at Ceres, set milling orders | — | none | | |

The trip is one ship epoch and one epoch per excursion or drone launch — ten or more on three
kinds of hull — and one voyage. For a vehicle or a drone every bound in doc 17 that assumed "hours"
(§23 Long sorties: a lapsed licence holds for the rest of the sortie, memory grows to its cap, a
fault-dropped hull stays on the ROM until a base) is bounded by one excursion. For the ship they are
bounded by the voyage, which is what the owner ruled for the licence (D-23-9) and is harmless for
the Dray, which fits no core; a cored ship on a multi-week voyage would carry a fault-dropped mind
on the ROM until its next hangar port, which §13 names. The IC resume rule (§13) keeps the sortie
across a `MIND_ABI` change and strands nothing at all; what lasts until the next hangar is the drop
to the ROM. The epoch count is also why the black box, not the engine, carries the voyage: a
contract that pays on "the run" reads lines, it does not hold an open object across days.

**Abandonment — D-23-4.** Today a carrier cannot leave with a rig out: `TV_OUT`, `TV_BUSY` and
`HOPPER_LOADED` refuse `+ship/plan`, `+ship/undock`, `+ship/lift` and a queued `depart` (doc 20 §4,
D-20-7; SM.11). This note keeps that rule. Abandonment therefore arises only when the hangar is
destroyed or when staff `Close` an excursion with the rig out; the rig's epoch stays open, is
flagged `abandoned` with the instant, and still closes only by the four kinds of §3. The owner is
told once. Lifting D-20-7 so that "a TV left out is a choice" is doc 20 §17's fight-phase
consequence and needs an owner ruling and a cut of its own; Q-23-2 asks whether to take it now.

## 5. What an epoch fixes, and what it does not

An epoch is bookkeeping. For its length it fixes:

- **the seal** — the mind, its card and profiles, the core, the licence class and state, the warrant
  principal and the station channels (doc 17 §13). A mind cannot be updated mid-epoch; a profile can
  be switched (§12). A pilot who boards a hull already out takes it under the seal it carries.
- **the pool admission** — counted at the dispatch, never refused mid-epoch (D-UM-36; §6).
- **the black box pair** — `MindSealed` … `SortieClosed`, with every engagement, haul and shift
  recorded between them and citing the sortie (doc 17 §13).
- **the custody claim, for a hull flying a held mind** — the four-owner IC Open at the start and
  Close at the end (`crates/tacsim/src/minds/ic_sortie/`, §7). The Open needs a licence and a filed
  holding (`IcHost::enqueue_start(permit, licence, profiles)`, `host.rs:273-277`;
  `Execution::capture` resolves `FiledExecutable` from a `PersonalFiledHolding`,
  `execution.rs:89`, `participant.rs:265-272`), so a coreless hull — the Dray and the TV, "no core:
  Class 0" — has no claim, only a seal and a pair. Hull cover's claim window and M8's custody
  inspection read the claim where there is one and the pair everywhere.
- **the law's touch points** — the two ends are where an inspection, a renewal attestation and a
  seizure happen (M8's five evidence channels, D-UM-42). A short epoch means the law can touch a hull
  at every berth, which is what the Republic's pickets at Kirkwood need (doc 11 §6, Customs
  Assertion).

It fixes nothing about money, cargo, contracts or orders: those are the voyage's and the economy's,
and they already run on the IC clock without it.

**The principal — D-23-5.** The seal's warrant principal is the actor of the plan the hull leaves
under: whoever filed the passage (`+ship/undock`, `+ship/lift`, or IC11's queued `depart`), whoever
typed `+tv/launch` or the act that implied it, or the dispatcher principal for a house's rigs
(SM.12b). A captain change mid-passage does not move it; who may *order* the hull is the `ORDER`
path's own rule (doc 17 D-UM-2, 17b cut L), and who may *board* it is the station's. The seal
records who sent the hull out; that is all a seal should say.

**The lapse hold — D-23-9 (the owner's ruling on Q-23-6).** A licence that lapses mid-epoch is in
grace until the **top-level hull reaches a hangar port**: for a ship, the close of its own epoch;
for a vehicle or a drone, the close of its carrier's open epoch, not of its own excursion. Inside
that passage every launch from the bay seals under the held licence, and the fall to doctrine —
doc 17 §22 M7's gate says "a lapse falls to doctrine at the next seal" — happens at the first seal
after the hangar. Mechanically the seal at a bay launch reads the licence as lapsed-and-held under
the carrier's open `SortieId`, which the seal line names anyway (D-23-2), so the hold needs no
object of its own. S.0 amends doc 17 §22 M7 and §23 "Long sorties" to say "the next seal after a
hangar port".

## 6. The host: what opens and closes an epoch

Doc 17 §22 ("After M8") imagines **a region sim task** — one per-millisecond simulation over an IC
region map, with region-map `SortieClosed`, startup admission, long-sortie snapshots and the IC
resume rule; 17f §10 names "the IC region sortie driver" a missing dependency and 17g §4 calls it
"an IC region host". Doc 20 §2 (D-20-1) explains why the arena cannot be that task as it stands:
it integrates wall time, its domain is hidden from every world view, and its snapshot resume rewinds
the tail against IC minting.

**This note splits that task in two — D-23-6.** Its *custody half* (the seal at dispatch, the pool
count, the black box pair, the claim Open and Close where a mind flies, the resume rule) needs no
per-millisecond simulation and lands on the IC-clock pull ticks that already move hulls. Its
*simulation half* (contacts, line of sight, fire, the tempo) is the arena, promoted onto a site's
ground inside an epoch (doc 20 §17), and is not built here. The existing IC host's `resume` and
`IcRetainedExecution::install(world, &mut Arena)` (`host.rs:172-230`) are arena-shaped — they
install a retained execution into a match — and the custody hosts below do not call them. There
are three such hosts in the tree:

| Host | Tick | What it already has | What it lacks for an epoch |
|---|---|---|---|
| **The surface excursion** (doc 20; `crates/tacsim/src/surface/excursion.rs`) | `tacsim.surface`, 1 s Coarse pull on the IC clock | `ExcursionId (tv, launch_ic)`, which doc 20 §4.3 mints *as* the sortie identity "the bay being the hangar"; launch and recover from the Dray-P's bay; `TvState::{Bay, Out, Held, Parked}`; staff `Close` and `Home`; the 24 IC h idle return; a 32-per-site cap | the seal at the launch's end, the black box pair in a durable home, a `SortieId` |
| **The port voyage** (doc 18 cut IC9; `crates/tacsim/src/port/voyage.rs`) | `tacsim.port`, 5 s | the filed plan, `+ship/undock`, the launch arm's `mush_sim::depart`, arrival as a placement, the docked/departing/underway/approach lifecycle, `Berth::Docked` | the same three, plus the Dray's own black box |
| **The drone phase** (doc 20 §17, reserved; D-UM-47's cut) | the surface tick | the shape only | everything — and it is the first host where a mind thinks IC, so the first where the claim, the pool and the resume rule are exercised |

**The first IC sortie is a surface excursion — D-23-7.** It is the closest host: a bay that is a
hangar, a launch and a recover, staff `Close` and `Home`, an identity already named for the purpose,
a cost lane that measured the tick (SM.12c), and hulls that fit no core, so the seal is the ROM seal
the arena already computes for a coreless hull (`fallback: "no core: the ROM"`, 17b cut M3.3). No
mind thinks, so no pool is spent, no claim opens and no resume rule is exercised: the epoch is seal
and record. That is the first tier of the M8 gate (§10), not the whole of it. M8's own tests — an
ungoverned core learned only through the five evidence channels, a quirk claim confirmed only by an
inspection reading a seal line (doc 17 §22) — need a hull with a core and a mind, and so does M7's
deferred clause, "flying a bought mind on an IC hull's sortie" (doc 17 §22 M7). Those come with the
drone phase, the second tier. The ship's epoch follows the excursion on the voyage. Q-23-3 asks the
owner to confirm the order.

**What the host does at each end.** At the dispatch, after the pool count: compute the seal, mint
the `SortieId` (§7), and write `MindSealed` to the hull's black box; for a hull flying a held mind,
also produce the `StartPermit` and enqueue the four-owner Open, whose prepare computes the seal
itself (`Execution::capture` → `seal_with_policy(…, SortieId(sortie), …)`,
`crates/tacsim/src/minds/ic_sortie/execution.rs:102-123`). At the close: write `SortieClosed` with
the close kind; for a held mind, produce the `CloseEvent` with the kind, the hull's physical state
(`conditions: UnitMutationPlan`) and enqueue the Close. **The idempotency lives in the host's own
durable record**, on the journey tick's pattern ("the cursor is the idempotency key",
`journey.rs:1282-1290`): the excursion and the voyage each already move a cursor once per boundary,
and the epoch's open and close ride those moves, so a re-run tick or a reboot mints nothing twice.
It cannot live in `IcHost`, whose inbox is in RAM with random request ids (`host.rs:283,307`); the
claim book's one-active-open-per-unit (`state.active`) is the guard against a second Open.

**The pool at an IC dispatch — D-23-8.** Until a mind thinks on an IC host, admission is recorded
(the class, the host, the count) and never refused. When the drone phase lands, the region's pool
and its split — by owner, by organisation, by traffic authority — become real, and doc 17 §12
leaves that split to "the corridor's design". Q-23-4 asks who rations it.

## 7. Records

- **`SortieId` outside a match.** `SortieId::in_match(match_id, unit)` packs the match id in the
  high 32 bits and the object number in the low (`arena/mind.rs:299`), and doc 17 §13 says "a region
  map mints its own later". The IC claim path already mints one: `IcSorties::next_transition`, a
  global ordinal from 1 in the owner-local book (`participant.rs:265-272`; `state.rs`). It collides
  with the arena today — ordinal 4 is `in_match(0, ObjId(4))`, and match 0 is used
  (`arena/mind.rs:5134`). **The owner ruled (§16) that a match is a simulator concept, not an IC
  one — D-23-10**: an IC epoch's id is not a match-shaped number and never shares the arena's
  packing. S.1 keeps one mint for every IC epoch, claim or not (the claim book's ordinal), and gives
  `SortieId` a kind — the arena's packed `(match, unit)` and the IC ordinal — as a tagged value or a
  struct, the lead's call at the cut; the top-bit packing of Draft 0.2 is withdrawn. If the struct
  is chosen the arena's pinned tapes move once; the collision test runs the ordinal against
  `in_match(0, _)` until the kind lands.
- **The seal for a coreless hull.** `seal_with_policy` takes the holding's facts; `seal_for`
  (`crates/tacsim/src/match_/seal.rs:450`) takes an arena `CustodyUnit`, which needs a team, a pose
  and a pod slot. Neither fits a TV leaving a bay. S.1 adds the ROM seal over a `TacticalUnit` with
  no custody and no holding: the same bytes the arena writes for a coreless hull, minus the pod.
- **The black box.** Doc 17 §13 and doc 16 W16 ("a battle's canon is reconstructed from black
  boxes") want one per hull. Proposed: a per-hull `mush_core::Ledger` (the match ledger's own
  machinery) keyed by the hull, opened on the first `MindSealed` and bounded like the match ledger
  (`ledger.max_bytes`); engagements, hauls and shifts cite the open sortie as doc 17 says. It must be
  durable from S.1 on: doc 20 §8's trip-level lines are the 32-line `tacsim:notice` ring, which
  never queues a launch or a recover and cannot be read by an inspection.
- **The claim records.** `ic_sortie/records.rs` already keeps "durable execution and transition
  order"; `ic_sortie/state.rs` keeps "one owner-local bounded book" that "never treats absence as a
  close event". Nothing here changes them; a host supplies the transition they wait for when a mind
  flies, and the fixture boundary in `participant.rs` goes then.
- **The resume rule** (doc 17 §13): a resume across a `MIND_ABI` or rhai change keeps the epoch, its
  seal and its memory bytes, drops every mind hull on it to the ROM and journals `MindDropped`
  with the cause `abi`. It is built only when a mind flies IC (the drone phase) and is sized by one
  leg, not by hours.

## 8. What this note does not decide

- **The fight on real ground.** The owner has now settled the shared policy in [doc 27](27-ic-field-operations-design-and-survey.md)
  Q-H1–Q-H12. Field combat preserves exact casualties, admits real arrivals/withdrawals and runs
  human-paced alongside the rate-24 campaign. Outage recovery freezes the whole world through
  a broadcast countdown. A local scene cannot delay journeys. An IC engagement is not an OOC
  match, and a recoverable mission kill does not close the hull epoch. Doc 20 §17's promotion
  remains a candidate mechanism; the persistent state/authority adapter is still to be built.
- **Hull cover on an IC epoch.** Cover's close evidence (`CoverageCloseEvidence`) exists only for
  an Open enqueued through `IcHost::enqueue_withheld` with a `WithheldDeparture` and a zone
  (`host/coverage.rs`); `enqueue_start` sets `withheld: None` (`host.rs:294`). No cut here builds
  the withheld departure or its zone source; a covered hull's epoch is a cut after the drone phase.
- **The voyage as an object.** Contracts that pay on a run, cover priced per passage, and the
  player's screen all want it; the black box lines are enough to build it from, later.
- **The region sim task as doc 17 drew it.** Not needed for the first IC sorties (§6). If a
  per-millisecond region ever is, it inherits this note's epochs unchanged.
- **Lifting D-20-7 and salvage:** doc 27 Q-H10 accepts explicit leave-behind departure for H1;
  Q-S1–Q-S15 settle H2's recovery/title boundary. Existing code still needs those cuts. M8's
  broader programme is not a blanket gate on human-operated fighting or towing.
- **The Ceres Simulation Center** (D-UM-51): an IC venue on arena machinery, not an epoch host.

## 9. Cuts

Each through the established cut loop (builder, lenses, fixer, gate, judge; one commit and PR per
cut off `main`), ledger here in §14. Sizes are the lead's estimates.

- **S.0 — Terms and amendments (small).** Doc 17: D-UM-45's "hangar or base" becomes §2's two
  hangar placements and the close moves to the arrival (D-23-1), nesting and abandonment are stated
  (D-23-2, -4), §22's M8 gate is restated as §10 below, §22's "After M8" region sim task is
  split per D-23-6, and §22 M7's "a lapse falls to doctrine at the next seal" and §23 "Long
  sorties" gain "after a hangar port" (D-23-9). The port entries' hangar kind is declared in
  content (the five ports yes, every site no; *moved to S.1 and S.2, §14*). Doc 20 §4.3 and §17 gain the cross-reference. Doc 16 §11 "Before IC play"
  notes that per-pulse durability is the arena's and that an IC epoch's durability is its host's.
  The historical player-facing wording audit (Q-23-5) is in §14; S.0b's blanket rename is now
  superseded below. *As built: §14.*
- **S.0b — The word on the surfaces (historical proposal, superseded 2026-10-28).** The blanket
  ban on printing "sortie" is withdrawn. `+sortie` names the player's operation; technical hull
  epochs remain a distinct concept. Arena wording may still say "sealed for this match" where
  accurate, but no rename is required merely to remove the approved word. The original audit
  in §14 remains historical evidence, not a new implementation instruction.
- **S.1 — The first IC sortie, on the surface excursion (medium).** One `SortieId` mint for IC
  epochs (§7) with the collision test; the ROM seal over a `TacticalUnit` (§7); the per-hull black
  box ledger; `MindSealed` at the launch's end (and for an implied launch), `SortieClosed` with
  `ReturnedToBase` at `+tv/recover` and at staff `Home`, `RecoveredToBase` when a second carrier or
  staff bring a `Parked` rig in, the `abandoned` flag at staff `Close`; a `NeverLeft` excursion
  writes nothing; the pool count recorded (D-23-8); the open and close keyed to the excursion's own
  cursor. `Destroyed` was reserved here, with no destruction path; GC.3 gave it one (doc 36, "GC.3 as
  built"): a rig whose hull goes down irrecoverable leaves remains and its epoch closes `Destroyed`
  (`FieldProvider::close_destroyed`). `TvState` is `Bay`, `Out`, `Held`, `Parked`; `Stranded` is a
  plan end with the rig `Parked`. Tests: one excursion opens and
  closes with a restart between; a `Parked` rig leaves it open through the idle return; a re-run tick
  mints nothing; the seal line names the Dray-P and its open sortie (D-23-2); the SM.12c cost
  lane re-run with the two writes per excursion inside its budget. *As built: §14.*
- **S.2 — The ship's epoch on the port voyage (medium).** The same on `tacsim.port`: the seal and
  `MindSealed` when the launch arm's `depart` moves the hull, nothing for a lapsed burn, the close
  `ReturnedToBase` at `docked` at a hangar port — a settle on a site's pad writes nothing (D-23-3)
  — `Destroyed` is not written here: a carrier wreck (GC.3's table) writes nothing to `tacsim.ship_epoch`
  yet, so the ship's epoch stays open past the wreck (owed) — the Dray's own black box,
  the principal per D-23-5 for a filed, queued or `lift` departure. The IC-return stamp is untouched.
  Tests: Stickney to Ceres opens and closes; a passage across a server restart keeps the epoch; a
  lapsed launch writes nothing. *As built: §14 (the record is the typed `tacsim.ship_epoch`, not a
  `Voyage` field).*
- **S.3 — Abandonment and recovery (small).** The `abandoned` flag's lifetime and the owner's one
  notice; `RecoveredToBase` when a second carrier recovers another's rig and the black box names
  the recoverer; a carrier destroyed with rigs out (GC.3 strands the crew and wrecks the bay rigs, doc 36; the carrier's
  own ship epoch is not closed by it yet).
  *As built: §14 (the second carrier's close lands at the placement, not at the order).*
- **S.4 — The first IC mind: the claim, the pool and the resume rule, with the drone phase
  (medium; lands inside D-UM-47's cut, not before).** The `StartPermit`/`CloseEvent` producer on
  the surface tick for a drone flying a held mind, replacing `participant.rs`'s fixture boundary and
  `ic_integration_tests`' `fixture_start`/`fixture_close` with a production-host twin of its stories;
  admission against a site's pool at the drone's launch; the `abi` drop on resume; long-leg
  snapshots on the surface tick.
  *Split by the owner, 2026-10-28 (doc 17h Q-M8-1, and the M8e re-scope, option C).*
  - **S.4a** is built as doc 17h's cut M8e: a cored rig seals a held filed mind's line (the mind,
    the core, the class, the governed bit and the licence's real state); a lapse seals Doctrine and
    tells only the operator; a pending transfer refuses; M8c's seal mark is written in the finish's
    bundle; an ungoverned rig seals an unfiled draft with its admissions. No claim Open/Close and no
    pool seat: a four-owner IC claim cannot sit on a surface excursion as the tree stands. An IC
    base accepts only a fixed-domain anchor (`mush-sim ic_exclusion.rs:322`, `:341`), a ship's bay moves,
    and an active claim freezes the hull (`mush-tactics ic_custody.rs` `holds` `:164`, `movement_preimage` `:167`).
  - **S.4b** is the claim Open/Close (the production `HostTransition`), the pool seat, the `abi`
    drop on resume and the long-leg snapshots. It lands with the thinking mind and the drone phase
    (D-UM-47), after M8, together with a custody design for moving anchors. *Draft 0.8: the custody
    design is §17, and §17.10 restates S.4b with the IC field as its host (D-23-20) and adds S.4b-T,
    the testimony producer; this bullet is superseded by it.*
- **S.5 — The M8 gate check (small).** §10's two tiers as dispatch-only CI stories, folded into
  doc 17h's cut M8g.

## 10. The M8 gate, restated

"IC sorties exist" has two tiers, and M8 needs both:

**Tier 1 — the epoch (S.1–S.3).** On a production-shaped launch and under a restart:

1. a vehicle's epoch opens when the rig leaves the bay and closes at `+tv/recover` with a
   `SortieId` and a `MindSealed`/`SortieClosed` pair in a durable black box that an inspection can
   read;
2. a ship's epoch does the same from `depart` at one hangar port to `docked` at the next, staying
   open across every site between, and the vehicle's `MindSealed` names the ship and its open
   `SortieId` as its hangar;
3. a rig whose excursion staff closed with it out stays open and flagged, and a re-run tick mints
   nothing.

**Tier 2 — the first IC mind (S.4).** A drone flying a held mind opens with a real four-owner Open
and closes with a real Close, its seal line carries the mind, the core and the licence state, and
M8's own leak and quirk-claim tests have a seal line to read.

**Tier 2 as restated for M8 (doc 17h Q-M8-1, and the owner's option C of 2026-10-28).** On a
surface excursion, a cored rig seals a held mind's line: the mind, the core, the class, the governed
bit and the licence state. A lapse seals Doctrine and says so only to the operator, a pending
transfer refuses, the seal mark reaches the Roll, and the ungoverned rig's seal carries the draft
and the word. The four-owner Open and Close above moves to S.4b with the drone phase. M8's leak and
quirk-claim tests read the seal line.

No fight and no region simulation is required by either tier. M8's own cuts (the governed bit,
inspection verbs, seizure) then have a real epoch to read at each end.

## 11. Decisions proposed here

| # | Decision | Status |
|---|---|---|
| D-23-1 | A hull's seal epoch is open exactly while it is in neither hangar placement (`Interior` to a bay; `Berth::Docked` at a hangar port); opens when the hull leaves, closes on arrival in a hangar, destruction or recovery with the `IcCloseKind` words; nothing else closes it. Amends D-UM-45's close from the power-down to the arrival | lead's proposal; S.0 amends doc 17 |
| D-23-2 | Epochs nest by record, not by object: `MindSealed` names the hangar left and, when it is a hull, that hull's open `SortieId`; the close names the base | lead's proposal |
| D-23-3 | The declared hangars — today the corridor's five ports and every bay; player-built hangars and bases and further ports later — are hangars; a mining or field site is the field and a ship landed there never ends its passage; so a vehicle's epoch is an excursion and a ship's is its whole passage between hangars | **confirmed by the owner, 2026-10-27**; Q-23-1 |
| D-23-4 | D-20-7's refusal to depart with a rig out stands; abandonment arises only from a destroyed hangar or a staff `Close`, flags the rig and never closes its epoch; the owner is told once | lead's proposal, Q-23-2 |
| D-23-5 | The seal's warrant principal is the actor of the plan the hull leaves under; it does not move with the crew | lead's proposal |
| D-23-6 | Doc 17 §22's region sim task splits: its custody half lands on the IC-clock pull-tick hosts; its simulation half is the arena promoted onto a site (doc 20 §17), not built here | lead's proposal |
| D-23-7 | The first IC sortie is a surface excursion (ROM seal, no mind, no claim, no pool spent) — tier 1 of the gate; the drone phase brings the first IC mind and tier 2; the ship's epoch follows on the voyage | lead's proposal, Q-23-3 |
| D-23-8 | Until a mind thinks on an IC host, pool admission is recorded and never refused | lead's proposal, Q-23-4 |
| D-23-9 | A licence that lapses mid-epoch is in grace until the top-level hull reaches a hangar port; a hosted hull's launches inside that passage seal under the held licence; the fall to doctrine is at the first seal after the hangar | **the owner's ruling, 2026-10-27** (Q-23-6) |
| D-23-10 | A match is a simulator concept, not an IC one: an IC epoch's id never shares the arena's `(match, unit)` packing; `SortieId` gains a kind | **the owner's ruling, 2026-10-27** (risk 1) |
| D-23-11 to D-23-20 | Custody for moving anchors: the anchor, the production Open/Close, the custody re-scoped, the two seats, the `abi` drop, the carried memory, the testimony producer, the field as S.4b's host | **accepted by the owner, 2026-11-03** (Q-23-7 to Q-23-20 at their defaults); §17.11 |

## 12. Questions for the owner

| # | Question | Default |
|---|---|---|
| Q-23-1 | Is every port entry's berth a hangar, so that a ship's epoch is one passage? | **Accepted 2026-10-27**, as narrowed by the Q-23-6 ruling: the five hangar ports are hangars and a site is the field, so a ship's epoch is its passage between hangar ports and a Belt voyage is one epoch of IC weeks (the "hours" assumptions are bounded by the voyage for the ship, D-23-9; §13). The owner confirmed the narrowing (§16). |
| Q-23-2 | Lift D-20-7 now, so a carrier may leave with a rig out and "a TV left out is a choice"? | No, not yet: D-20-7 stands until the fight phase that doc 20 §17 ties it to. Abandonment is reachable only through a destroyed hangar or staff action, which S.3 covers. **Accepted 2026-10-27.** |
| Q-23-3 | First host order: surface excursion, then the ship, then drones? | Yes. The excursion is the closest to an epoch today and gives tier 1 soonest; tier 2 waits on the drone phase either way. **Accepted 2026-10-27.** |
| Q-23-4 | Who rations an IC site's compute pool once minds fly IC? | The region's de facto holder (doc 11 §5), published as an allotment; recorded, never refused, until the drone phase. **Accepted 2026-10-27.** |
| Q-23-5 | The player's words. | **Amended 2026-10-28:** `+sortie` and "sortie" for the player's operation are approved (doc 27 FO-7). Keep that operation distinct from the technical hull epoch. The 2026-10-27 blanket "Sortie never prints" ruling is superseded. |
| Q-23-6 | Does a licence that lapses mid-epoch still hold to the close (doc 17 §23)? | **Ruled 2026-10-27**: it holds to the close of the top-level passage, not each excursion — the licence is in grace until the hull reaches the next hangar. D-23-9. |
| Q-23-7 to Q-23-20 | The S.4b precondition's questions (the anchor, the Open's commit, one seal, the freeze, salvage under a claim, the compute seat, the ABI mismatch, the wall's exit, memory between fields, the testimony author and cut, the host, org-held licences, the acceptance) | **Accepted 2026-11-03, all at their defaults**; §17.12 |

## 13. Risks

- **The id collision is live.** The IC book's ordinal and `in_match(0, _)` already overlap; S.1's
  first test is that pin. The owner's ruling (D-23-10) takes the top-bit packing off the table, so
  the kind is a tagged value or a struct, and the struct moves the arena's pinned tapes once.
- **A ship's epoch can be IC weeks.** With a site as the field, the Dray's epoch runs from hangar
  port to hangar port across every site between. For the coreless Dray that costs nothing; a cored
  ship later carries a lapsed licence (by D-23-9), a memory at its cap and a fault-dropped mind on
  the ROM until its next hangar port. The owner has ruled the first; the other two are the price of
  the same rule and are named here so no one is surprised.
- **Two writes per excursion on the surface tick.** SM.12c's cost lane budgeted the tick (it ran at
  30 TVs under the per-party contract cap, O-19c); S.1 re-runs it with the black box writes.
- **The ROM seal on a mindless hull** makes a "mind" record for hulls that have none. It is the
  arena's existing behaviour for a coreless hull and the cheapest honest seal; the alternative, no
  seal for Class 0, would leave the Dray and the TV outside M8's evidence channels.
- **Tier 2 is further away than tier 1 looks.** The claim needs a filed, licensed mind on a cored
  hull; no such hull flies IC until the drone phase, and the drone phase is D-UM-47's own cut.
  Nothing in tier 1 should be presented as a passed claim story.
- **The ship's opening instant.** `depart` fires on the port tick after the burn; an epoch keyed
  to it, not to the player's verb, is right but will surprise a test that expects `MindSealed` on
  `+ship/undock`.
- **Doc 17's "After M8" ordering.** Doc 17 §22 puts the region sim task after M8 and gates M8 on
  IC sorties. D-23-6 resolves the circle by making the custody half an M8 prerequisite and leaving
  the simulation half after it; S.0 writes that into doc 17 so the ledger stops contradicting itself.

## 14. Ledger

### S.0 — terms and amendments, as built (2026-10-27, branch `claude/ic-sorties-s0`)

Docs only; no code and no pinned sentence moved. Built from `main` at `6f8b9d16` (PR #35, which
merged Draft 0.2 before the Draft 0.3 commit `88a7e9ce` reached its branch; S.0's branch carries
that commit first, so Draft 0.3's rulings land with this cut) by the lead in the session that wrote
the note, with the owner's rulings of the day already in hand (§16), so no planner or lens ran; the
diff is the review.

- **Doc 17.** §1 R-B, §12 "The flow" (the sortie and the dispatch), §13 "Records, the seal and
  replay" (the black box pair) and D-UM-45 carry the two declared hangar placements, the close on
  arrival, the four `IcCloseKind` words and the site-is-the-field rule (D-23-1, D-23-3), with the
  nesting record (D-23-2); §22 M7's "a lapse falls to doctrine at the next seal" reads "the next seal
  after a hangar" (D-23-9); §22 M8's note carries the two-tier gate of §10; "After M8" says the
  region sim task is split (D-23-6) and its custody half lands as S.1–S.4 before M8; §23 "Long
  sorties" carries the passage-bounded reading; §24 gains the amendment entry.
- **Doc 20.** §2 (D-20-1's "two L cuts"), §4.3 (the excursion as the first IC sortie: the epoch
  opens at the launch's end, closes at the recover or staff `Home`, names the carrier's open sortie)
  and §17 (the promotion stays out of doc 23; D-20-7 stands) gain the cross-references.
- **Doc 16.** §11 "Before IC play" says an IC sortie's durability is its host's and the arena is the
  fight's host inside a hull's open sortie.
- **The Q-23-5 audit.** "Sortie" prints today in seven player sentences — `match_/cockpit.rs`
  "Mind: …, sealed for this sortie" and "Sealed for this sortie: …", `match_/lobby.rs` "Sealed for
  this sortie", `minds/doctrine_library.rs` "running sortie", `mush-net-ssh`'s `mind_workshop.rs`
  "sortie seals", and `match_/mod.rs`' "held by IC sortie {sortie}" error — and six help lines
  (`cockpit.md`, `doctrine.md`, `mindcraft.md`, `minds.md` twice, `personal-minds.md` twice). All are
  the OOC arena's or M7's and each is pinned by a test, so they move in S.0b with a tacsim gate, not
  in a docs cut.
- **Lead rulings.** (1) S.0 is docs-only; the renames are S.0b (above). (2) "The port entries'
  hangar kind is declared in content" is not built here: the first reader of that declaration is
  S.1's seal at the launch and S.2's close at `docked`, so the declaration lands with S.1 and S.2.
  (3) The arena's wording after S.0b is "sealed for this match", which is true there (the match
  fills the sortie) and prints no engine term.
- **Not done.** No cargo ran (the C2e–C2h completion may own the workspace build); nothing under
  `crates/` or `docs/help/` changed.

### S.1 — the first IC sortie, on the surface excursion, as built (2026-10-27, branch `claude/ic-sorties-s1`)

Built from `main` at `a477e9c3` (PR #36) through the cut loop: the lead's brief with eight rulings
(`plan-data/ic-sorties/cuts/S.1/brief.md`), one builder, one lens, one fix round; the gate is the
cut's own tests on this box and CI's shards on the PR. Tier 1 of §10's gate for the vehicle is met:
a rig's epoch opens when it leaves the bay and closes at `+tv/recover` with a `SortieId` and a
`mind_sealed`/`sortie_closed` pair in a durable black box; a rig staff `close`d with it out stays
open and flagged; a re-run finish, at the same instant or across a reboot, mints nothing.

- **The id (L1, a recorded deviation from §7's first bullet).** `SortieId` is an enum of two kinds:
  the arena's `Packed(u64)` (`in_match`, `none`), serialised as the bare number it always was — every
  pinned tape, record and snapshot is byte-identical, `Display` unchanged — and the IC epoch
  `Ic { hull, at_ic }`, the hull and the IC instant its leaving began (`ExcursionId.launch_ic`; S.2's
  voyage will use the undock), serialised as `{"ic":{"hull":n,"at_ic":t}}`. Deterministic, no
  counter, and not the claim book's ordinal: `IcSorties` is one bounded book of `MAX_HISTORY` claim
  rows and an excursion-per-mint would exhaust it. The book keeps minting `Packed(ordinal)` for its
  own transition order and refuses the IC kind at load, insert and validation until S.4 makes it
  carry the host's id. The collision test runs `ic(#4, 0)` against `in_match(0, #4)`.
- **The seal (L8).** `match_::seal::seal_rom_for(id, &TacticalUnit, weapons, sortie,
  &IcSealConfig)` is `seal_with_policy` under the production policy with no `CustodyUnit`, match or
  holding: a rig with no core seals Class 0, governed, `fallback = "no core: the ROM"`, channels
  Helm and Sensors, and the test proves it equals `seal_for` of the same hull in a pod save the
  sortie and the principal. The surface fixtures' rigs do have a `TacticalUnit` (unit class
  `vehicle`).
- **The black box (L2, L7).** One `mush_core::Ledger` on `TacsimStore` (`black_boxes`), one log per
  hull, id `hull.<n>` so the documents are `log:hull.<n>` in tacsim's namespace, restored from
  history at boot (a refused restore pages and marks the boxes dark: no appends, chunk 0 never
  overwritten). The brief's assumption that the append could ride an `ActionTxn` did not hold (an
  `ActionTxn` writes no `log:*` chunk and a provider has no commit hook), so the builder followed
  mush-economy's `core_integration` pattern: a new `OwnerParticipant` (`black_box_owner`,
  `tacsim_black_box`, registered in `port_tick.rs`) whose `prepare_append` runs in the participant's
  prepare and whose `LedgerChunkSet` replay write rides the finish's or the receipt's bundle, bound
  and applied at the composed install — one commit for the record move and the line, and no second
  ledger mechanism. The two providers (`tacsim_surface_done`, the staff recovery) moved to
  `prepare_authoritative_with_context` for it. On a path with no participants (the typed-only
  harnesses) nothing is written and the record's `unrecorded` flag says so. Lines are a strict
  tagged enum `surface::black_box::BlackBoxLine`: `mind_sealed { unit, sortie, at_ic, host
  "surface", hangar { hull, slot, sortie: null }, site, principal, seal: sealed|absent, the seal
  facts (mind, source, card_digest, class, governed, licence, seal_digest, fallback), admitted
  { class, host, count } }` and `sortie_closed { unit, sortie, at_ic, close: <IcCloseKind's wire>,
  abandoned, unrecorded }` (the kind field is `close`, since `kind` is the line's own tag — the one
  deviation from L7's names). The read seam is `TacsimStore::{black_box, black_box_lines (with the count of
  undecodable entries, never dropped silently), black_box_len, black_box_dark}` and
  `black_box::history` over chunk documents; no verb.
- **The record (L3–L6).** `RigRecord.sortie: Option<OpenSortie { id, hull, slot, site, operator,
  opened_ic, abandoned, unrecorded, seal: Option<SealFacts> }>`, schema 1 kept. The epoch opens at
  the finish of any act that leaves the bay — a launch, or a scan, sounding or drive from the bay,
  by the hand, a plan's step or the idle return alike — when its excursion survives with the rig out
  of every bay (D-23-1: a rig out of its hangar is on its sortie, whatever act took it out); a
  `NeverLeft` opens nothing. The close: `+tv/recover`'s `Done` and staff `home` write
  `ReturnedToBase { base: hull }`; staff `home` on a parked rig writes `RecoveredToBase`; staff
  `close` keeps the epoch and sets `abandoned` (D-23-4). A start that would leave the bay with an
  epoch still open is refused `Conflict` ("that rig's last trip is still open; staff must bring it
  home first") and paged, with the finish's refusal as a backstop. The principal is the excursion's
  operator, never an implied act's `by` (D-23-5). `admitted.count` is the open surface epochs on the
  site including this one (the builder's reading of D-23-8; the cap's `counts.site` counts
  excursions, which differ by the abandoned and the mid-launch — accepted). `PoweredDownInBase` is
  unused on the surface; `Destroyed` was reserved when this was built and is written since GC.3, for a
  rig's irrecoverable hull only (not for a carrier wreck).
- **The cap.** `[tacsim.surface].black_box_max_lines`, default 4096 (provisional under the owner's
  economy-defaults rule, 1..=1,048,576). `mush_core::Ledger` has no retention primitive, so a full
  box refuses the append with a page (on every refusal, not once) and the act still completes; an
  open reserves room for its close (`len + 2 <= cap`) so a written open is never left without one.
- **Parked (L6).** The idle park is a projection, not a record fact, so a staff `home`'s kind is
  derived live at the offer (`act::parked_short`: the plan ended `Stranded` with no act since, or the
  idle return's own `recover_plan` + `leg_home` test refuses `LegShort`/`Far` for the operator) and
  covered by the revision hash; any other refusal reads `ReturnedToBase`.
- **The close-window case.** A staff `close` between the tick's placement and the finish's landing
  now opens the epoch abandoned (a launch) or closes it `ReturnedToBase` carrying the epoch's own
  `abandoned` (a recover) at the receipt, with the line on the receipt's commit, so D-23-1's "open
  exactly while out" holds through the window; both cases are driven by a test.
- **Gates.** No module-namespace or state-slot row was needed (the chunks land in tacsim's own
  namespace through the core's participant bind). The SM.12c write census had no exact pin for
  `done.launch`/`done.recover` (ceilings of 8); it gains `assert_black_box_pair`. The production
  roster's owner participants are thirteen under `MAX_OWNER_PARTICIPANTS = 16` (the lens counted
  them; `participant.rs`' comment corrected; `mush-contract-tests`' `macro_names`,
  `exit_command_words` and `help_commands` compose the full roster and pass). The `excursions.rs` and `cost_lane.rs` harnesses moved
  to the production composed prepare path so they can see a line; the other surface harnesses stay
  typed-only and write none.
- **Tests.** `arena::mind` (the id), `match_::seal` (the ROM seal), `surface::black_box` (the log
  id, the two strict shapes, the cap), `shard_4/ic_sortie_surface.rs` (one excursion across two
  reboots with the history read; the parked rig through the idle return and `RecoveredToBase`; the
  staff `close` flag then `home`; the re-run finish at the same instant and across a reboot; the
  `NeverLeft`; the close-window `close`), and the cost lane's census.
- **Not done / owed.** Retention for a full box (a `mush_core::Ledger` primitive, its own cut); a
  store-read completion for the history (no verb until M8's inspection); a second carrier's
  `RecoveredToBase` and the abandoned notice's lifetime (S.3); the parked derivation reads the
  operator's aboard-ness live; the 4096 default is the owner's to tune.

### S.2 — the ship's epoch on the port voyage, as built (2026-10-27, branch `claude/ic-sorties-s2`, on S.1)

Built from S.1 (`2f712d4f`, PR #41) through the cut loop: the lead's brief with seven rulings
(`plan-data/ic-sorties/cuts/S.2/brief.md`), one builder, one lens, one fix round in which the lead
amended ruling V2 (below). Tier 1 of §10's gate for the ship is met: a Dray's epoch opens when the
launch arm's `depart` moves it off a hangar port and closes `ReturnedToBase` when it is docked at a
hangar port again, staying open across every site it settles on; a lapsed burn writes nothing; a
re-run fire mints nothing; and a rig launched from a settled Dray-P names the ship's open `SortieId`
as its hangar (D-23-2; §10 item 2).

- **The hangar kind (V3).** One helper decides it: `port::ship::is_hangar_port(def)` is
  `def.site.is_none()` — a port entry with no `site` block (the corridor's five stations today) is a
  hangar, a site is the field (D-23-3). `Berth::hangar()` and `hangar_port_of(w, hull)` read it;
  nothing else in the tree decides "hangar". This is the content declaration S.0 moved here.
- **The mechanism (V1).** The ship's epoch is a pull reconciliation on the `tacsim.port` tick, not
  a write in the launch's outbox: the depart is a direct placement write and the black box line
  needs the owner participant's commit, so the two cannot share one commit. A new **epoch arm**
  (`port/epoch.rs`; `Arm::Epoch`, its own `ArmFireStats`) compares each programme hull's placement
  with its epoch record and submits at most one `tacsim_voyage_epoch` action as the principal
  through `Fire::as_principal` (`open` when the journey book says the hull departed and it is under
  way or docked at the journey's own site, `close` when it is docked at a hangar port with an epoch
  open, idle otherwise); the action's prepare re-runs the decision, refuses `Conflict` on any
  mismatch, appends the line through S.1's `black_box::append` and moves the record in one bundle.
  The window between the depart and the open's commit (one fire plus deferrals) is accepted; the
  id and the line's instant are the journey's `departed().at`, never the fire's. A hull whose epoch
  is due is withheld from the launch, recall and order arms that fire, so a close lands before the
  hull can depart again. Retries: at most three submissions per target per boot, 24 fires apart,
  then held and paged — and an exhausted hold withholds nothing, so a stuck epoch never wedges the
  hull. A principal who is gone (or GOD) falls back to the live helm; with none the hull is paged
  once and is not due.
- **The record (V2, amended at the fix round).** The brief put the epoch on the `Voyage` document;
  the lens showed that made the document a two-writer record (the tick's `put_voyage` and the
  participant) with a fail-stop race when a tick write landed while an epoch act was in custody. The
  lead ruled the epoch onto the typed family the core forced anyway (an `ActionTxn` refuses an
  empty typed transaction): **`tacsim.ship_epoch`**, one `ShipEpochRecord { hull, open:
  Option<ShipEpoch { id, from, base, principal, opened_ic, unrecorded, seal }>, last: Option<(op,
  id, at_ic, written)>, attempts }` per hull, replaced each time, strict, with the invariants in
  `check()` (every id is this hull's IC id; an open epoch's `last` is its own open). The participant
  writes only the line, exactly as S.1 left it. `Voyage` keeps `departed: Option<Departed { seq,
  pilot, depart_ic, to_venue, from }>`, written by the tick at the launch in the depart's outbox and
  cleared at a lapse — the durable principal (V5, D-23-5: the actor of the plan the hull leaves
  under, for a live `+ship/undock`, a `+ship/lift` or a queued departure). `VOYAGE_SCHEMA` stays 1.
  An open whose id the record's `last` already names is refused (the same id never opens twice,
  e.g. a staff placement back onto the journey's pad). The idempotency key carries the record's
  committed `attempts`, not a RAM counter.
- **The lines (V4).** `EpochHost::Port`; `Hangar` is an untagged enum `{ Bay { hull, slot, sortie },
  Port { entry, base } }` with S.1's bay JSON byte-identical; the ship's `mind_sealed` names the port
  left as hangar and site, the pilot as principal, `seal: sealed` ("no core: the ROM", the Dray's
  `TacticalUnit` through `seal_rom_for`, V6, re-checked against the unit's definition digest in the
  prepare) and `admitted { class, "port", 1 }`; the close is `ReturnedToBase { base: <the port's
  host room> }` at the journey's arrival instant. `Destroyed` is not written on the voyage (a carrier wreck closes nothing in `tacsim.ship_epoch` yet). An open whose `from`
  is a site (a staff-placed or pre-cut hull lifting from a pad) pages once and opens with that site
  as the port shape, recorded as it is.
- **The vehicle names its carrier (V7).** S.1's `open_epoch` fills the bay shape's `sortie` from
  the carrier's open `ShipEpochRecord` through a typed read at the surface offer; S.1's test 1 now
  asserts the ship's id (its harness flies the Dray-P, Ceres to Ahuna, so the ship is on its sortie
  — the brief's premise that it never flew was wrong) and a unit test covers the `null` case.
- **Pages, once per hull per boot.** An unwritable box, no helm, no `TacticalUnit`, an exhausted
  hold, an unreadable record, a from-the-field open, and a **missed epoch** (docked at a hangar
  with no open, `departed.depart_ic` equal to the journey's, and `last` not that close — a crash
  within a fire of the launch whose downtime outlasts the passage).
- **Gates.** The cost lane measures the act (`other.tacsim_voyage_epoch`, a constructed maximum of
  2: the record and the chunk) in the surface, plan and tenure censuses; `assert_black_box_pair`
  admits its kind. No surface count moved. The legacy-handlers `+ship` note is reworded; the
  baseline is unmoved. The typed-only harnesses now move the ship's epoch record themselves (marked
  `unrecorded`) and no longer draw resubmissions.
- **Tests.** `shard_2/ic_sortie_voyages.rs` (Ceres to Stickney opens and closes, with the hangar
  kind in-world; a passage across a restart keeps the epoch; a lapsed launch writes nothing; a re-run
  fire mints nothing and a replayed open after its close is refused), `shard_4/ic_sortie_surface.rs`
  (a rig launched on Ahuna's pad names its carrier's open epoch; the ship's epoch survives the rig's
  recover), unit tests for the hangar kind, the record's invariants, the two ship line shapes and
  the carrier null case.
- **Not done / owed.** Staff `close`/`home` for a ship, a second carrier and the abandoned flag's
  lifetime (S.3); the `Missed`, from-the-field and helm-fallback pages have no integration test; a
  retry of a target whose prepare was refused reuses its key (verified safe: the core remembers only a staged intent under a key — `ActionCoordinator::preflight`, `prepare_and_stage_resolved` — and a refused prepare is prepared again, as `mush-market`'s `trade.rs` already relies on); the typed-only shards 1, 3, 5 and 6 run on CI; retention.
- **The CI fix round (2026-10-27, after the PR).** Three cells the cut loop's local gate does not
  run went red. (1) The lint cell's M7 memory-size step (`-D clippy::large_stack_frames`, a
  65,536-byte threshold, run only on CI): `port_tick.rs`' registration function reached 71 KB with
  the epoch provider, and S.1's `RigRecord` had pushed a surface-tick unit test 452 bytes over.
  The epoch provider and the six providers that each clone the inventory families now register in
  their own frames (`register_epoch`, `register_stock`, `register_burn`, `register_site_work`,
  `register_relay`, `register_mill`, `register_dispatch`; the `register_surface_acts` pattern),
  and the test's last three lines are their own test (on S.1's branch). The M7 command joins the
  builders' gate in `common-s.md`. (2) Shards 1, 3 and 6, the typed-only harnesses: the arm takes
  a hull as its agent on the fire its close is due, so a harness that flies a hull to Stickney,
  steps the clock one real second at a time (never meeting the port tick's cadence) and then fires
  once and asserts on the queue found the close, not its order's leg, and the hull withheld.
  `fly_to_stickney` in `shard_3/orders.rs` and `shard_1/sale.rs` now settles the open after the
  launch fire and the close after the docking (one fire at rate 1 each, landed, the record asserted
  open, then closed); the two-crew flight showed why the open needs it too — the shorter passage
  docked before any fire the hour-long steps happened to meet, so it opened nothing and its arrival
  paged a missed epoch. (3) The SM.11 offline story's
  pinned IC span moved 46,297 → 46,897, one 600 IC s step of the queue's tail: the sell step after
  the docking back at Ceres waits the fire the close takes — V1's accepted window, now visible in
  a pin. Nothing in the arm or the record moved.

### S.3 — abandonment and recovery, as built (2026-10-27, branch `claude/ic-sorties-s3`, on S.2)

Built from S.2 (`8fb432ab`, PR #45 with its CI fix round) through the cut loop: the lead's brief
with six rulings A1–A6 (`plan-data/ic-sorties/cuts/S.3/brief.md`), one builder, one lens, one fix
round in which the lead ruled R1 (below). This is the last custody cut before the drone phase
(S.4): the `abandoned` flag has a lifetime and an instant, a home of an abandoned rig is a
recovery and the black box names the recoverer, a second carrier may bring another's rig in, and a
ship docked at a hangar its own journey did not bring it to closes recovered.

- **The flag's lifetime and the owner's one notice (A1, D-23-4).** `OpenSortie` gains
  `abandoned_ic: Option<i64>` (serde default, skipped when `None`; `RigRecord::check` refuses an
  instant without the flag): the IC instant of the staff `close` that abandoned the rig, set at
  both abandoning sites (the plain close with the rig out and the close-window case F3 that opens
  the epoch abandoned); a second close keeps the first instant. Nothing clears the flag but the
  epoch's close — no act takes an abandoned rig out (the command layer refuses `LAUNCH_OUT` from
  the ground; S.1's start refusal "that rig's last trip is still open; staff must bring it home
  first" is reachable only after a staff write back into the bay, and both are pinned) and the
  carrier cannot lift with it out (D-20-7, the departure guard, untouched). The close receipt's
  line to the operator is the one notice, reworded: *Staff closed rig #n's excursion and cancelled
  its work. It stands where it is, abandoned, until staff bring it home; its hopper is unchanged.*
  — told once through `recovery_told`, not again after a reboot, and carried onto a replacing
  receipt when the replaced one was an untold abandoning close (`RecoveryReceipt.abandoned` means
  "this receipt owes the abandonment notice"); a close with no epoch open keeps S.1's sentence.
- **A home of an abandoned rig is a recovery (A2, A3).** The own-hull home's kind rule is
  `RecoveredToBase { base: hull }` when the epoch is abandoned or the rig is parked (S.1's L6),
  `ReturnedToBase` otherwise (a home mid-excursion with the charge to come home); a recover act
  keeps `ReturnedToBase` even on an abandoned epoch (the rig comes in under its own drive). The
  `sortie_closed` line gains `abandoned_ic` and `recovered_by: Option<ObjId>` (both skipped when
  `None`; S.1's and S.2's JSON byte-identical), the recoverer set through
  `BlackBoxLine::recovered_by(staff)` exactly when the close is `RecoveredToBase` on a surface
  epoch; `check` refuses a recoverer of 0, equal to the unit, or on any other kind. S.1's
  close-then-home tests now assert `RecoveredToBase` (listed in the builder's report).
- **A second carrier recovers another's rig (A4; R1 at the fix round).** `@tacsim/tv home #<rig>
  #<hull>`, staff only: the rig's epoch must be open and abandoned (*That rig is not abandoned; home
  it into its own hull.*), its lease must stand (*That rig's lease must stand to recover it into
  another hull.* — the tick's lease fence would otherwise wait for good), the named hull must be a
  carrier with a readable bay settled on the site the rig stands on (*That hull is not settled on
  the rig's site with a readable bay.*) with room in the compartment (*That hull's bay has no room
  for the rig.*); any such hull — another player's, a house's — bounded in practice to the
  programme and yard rows. The recoverers are server facts read at the offer (one hull list per
  offer, `Seek`), never a submitted choice; the revision facet is `staff_<actor>_into_<hull>` so a
  hull landing on the site fences nothing but an order naming it; the idempotency key carries the
  recoverer. The receipt gains `into: Option<ObjId>` (`hull` stays the assigned hull; the bay
  fields are the recoverer's). **The lens found that closing the epoch at the receipt, as the
  own-hull home does, left no way out when the recovering hull lifted or its bay filled before the
  placement: the black box would name a bay the rig never reached and no verb could re-target it.
  The lead ruled (R1): a second-carrier home closes the epoch at the completion** — the receipt
  leaves the record's `sortie` open and abandoned and writes no line; the tick's completion, the
  staff actor's typed finish (the same provider's prepare, which already appends through
  `black_box::append`), takes the `sortie` and appends `RecoveredToBase { base: <the recoverer> }`
  with `recovered_by: <the staff actor>`, `abandoned: true`, `abandoned_ic`, in the bundle that
  confirms the placement. So, while it waits, `home #rig #B2` (or `#B` again) replaces the receipt
  through the existing gate, and `close` cancels it in place with the rig still abandoned, as S.1
  says. The wait notice for this home names its remedy, `@tacsim/tv home #n #hull`, and three new
  wait reasons (the recovering hull no longer settled on the rig's site; its bay changed; the rig
  cannot be placed in its bay). The own-hull home keeps its receipt-time close (its base is fixed);
  a comment at both sites says why they differ. `project()` reads a rig in a foreign bay as `Out`
  (its own bay `Bay`), agreeing with the tick's projection. **The lease is untouched**: the yard
  still leases the rig to its lessee on the first hull; the recovering hull lifts with it aboard
  (the departure guard walks leased rigs); the lessee's `+tv/n launch` is refused today with
  `LAUNCH_NOT_SITE` because its own hull is off the pad, not because of the foreign bay. The
  operator's notice names the recoverer (*Staff brought rig #n into hull #m's bay. …*).
- **The ship's recovery (A5).** The epoch arm's close kind is `RecoveredToBase { base: <the port's
  host room> }` when the journey book has no arrival at this port's host — exactly the binding
  `epoch_due` already makes for `at_ic` (which stays `now` there), the endpoints being host rooms
  wherever they are compared and an arrived journey kept and persisted — else `ReturnedToBase`;
  `recovered_by: None` (a staff placement is a GOD write and no verb records an actor on a hull
  yet). `EpochDue`/`EpochOffer` carry `recovered` (skipped when false; the S.2 offer JSON
  byte-identical) and the composed prepare requires equality. A recovered close pages once a boot
  per hull (`EpochWarn::Recovered`, counted in `epoch_pages`) so a misread own-drive docking
  would be visible. **There is no staff `close` for a ship**: its hangar is a port, never destroyed
  or closed, and D-23-4's abandonment is the rig's; `@tacsim/port clear` stands down a filed
  clearance before any epoch and is untouched.
- **Reserved (A6).** A carrier destroyed with rigs out: no destruction path exists; one comment
  where the surface close kinds are chosen names `Destroyed` for it.
- **Gates.** `hull_view_berth` is `pub(crate)`. The `@tacsim/tv` help line and the
  legacy-handlers note carry the second form; the baseline is unmoved. No cost-lane count moved
  (`cost_lane.rs` drives no staff recovery, so the second-carrier finish's chunk write is in no
  census). The panic budget is unmoved. `docs/ops.md`'s staff rig recovery section carries the
  abandonment and the second-carrier home.
- **Tests.** `shard_4/ic_sortie_surface.rs`: an abandoned rig keeps its flag and instant across a
  restart (the two refusals, a second close keeping the instant, the reworded notice once and the
  plain one once); a staff home of an abandoned rig is a recovery (S.1's test 3 amended); a home
  mid-excursion still returns; a second carrier recovers an abandoned rig across a reboot between
  the order and the completion (the receipt's `into`, the rig `Interior` in the recoverer's bay,
  exactly one close line naming the recoverer, the lessee's launch refused, the recoverer's lift
  filed); the same home waits while its hull is gone (the exact wait sentence once, the epoch still
  open and abandoned, the completion after the hull is placed back, the line's base the hull the
  rig landed in); a home into a full bay, into another hull without a lease, on a rig that is not
  abandoned, and into a hull off the site are refused with nothing written; a replaced abandoning
  close still tells the owner; a ship placed into Stickney's berth closes recovered at the fire's
  `now` with the journey book untouched and one page. The strict-shape unit test gains the
  `abandoned_ic` and `recovered_by` cases in a frame of its own (the M7 limit); the S.1 and S.2
  cases are byte-identical. The builder's test 4 flies the second carrier in before the first is
  taken off the pad (the Co-op refuses enrolment with a programme hull unplaced); the recovery is
  exercised as specified.
- **Not done / owed (doc 23 §8 unless said).** What the lessee or the recoverer may do with a rig
  in another's bay (the finish sets `charge.in_bay = true` there, so it recharges from a foreign
  hull; `into_berth.ready` accepts the recoverer on any site once the rig is already in its bay);
  salvage, seizure, who holds an abandoned hull; the lessee's own hull held by `TV_AWAY` while
  another carries its rig (D-20-7); a carried-forward abandonment notice on a pending home that
  never completes is never told; no test drives the idempotency key's recoverer segment, the
  not-fenced-by-a-landing-hull revision, or a re-target to a third carrier end to end (the harness
  cannot enrol a third pilot with a programme hull unplaced); a destruction path; retention.

### S.4a — the sealed mind aboard, as built (2026-10-29, doc 17h cut M8e, PR #93, `913e29dd`)

S.4 was split by the owner (doc 17h Q-M8-1, then option C). S.4a is built.
- `+tv/<n> launch mind=<filed:N|draft:name> [org=<org>]` seals a held filed mind aboard a cored
  rig with the licence's real state, for paid and owner licences.
- A lapsed or revoked licence seals Doctrine, and only the operator is told.
- A funded transfer refuses, with D-UM-40's sentence, which the owner approved.
- M8c's seal mark is written in the finish's bundle for a mind that flew.
- The ungoverned rig seals an unfiled draft with its `admissions`.
- The cored-rig excursion harness was built, with fitted rigs and commerce together.

No claim Open/Close, pool seat or `abi` drop: an IC base needs a fixed-domain anchor
(`mush-sim ic_exclusion.rs:322`, `:341`), and an active claim freezes the hull
(`mush-tactics ic_custody.rs` `holds` `:164`, `movement_preimage` `:167`). Those move to **S.4b**, with the drone phase and a custody
design for moving anchors. *Draft 0.8: that design is §17, and S.4b is restated there with the IC
field as its host.*

S.5's dispatch-only stories are folded into doc 17h's M8g. The leak battery is on main (#106,
#132).

## 15. The review of Draft 0.1, and what it changed

An independent read-only review on 2026-10-27 (one Opus lens over doc 17, doc 20, 17f, doc 18 and
`crates/tacsim/src/minds/ic_sortie`) found eight must-fix errors in Draft 0.1, every one a place
where the tree is further along than the draft assumed. Draft 0.2 carries them:

1. The close words: the arena's are `match_end`/`destroyed`, and `IcCloseKind` already defines the
   IC set with a `base` field (§3; D-23-1 now amends D-UM-45 explicitly).
2. A coreless hull cannot take the four-owner Open (it needs a licence and a filed holding), so the
   claim is tier 2's, not tier 1's (§5, §6, §10).
3. The claim path already seals (`seal_with_policy` inside `Execution::capture`) and mints
   (`next_transition`), and the mint collides with `in_match(0, _)` today (§6, §7).
4. A docked ship is `Direct` on the port map, so "Interior to a hangar" never closed a ship's epoch;
   the hangar is now two placements (§2, D-23-1).
5. A settled carrier is docked whenever its rigs are out, so nesting records the carrier's most
   recent sortie, not an open one (D-23-2, §10). *Superseded in Draft 0.3: with a site the field
   (D-23-3 narrowed), the carrier's epoch is open while its rigs are out and D-23-2 names it.*
6. D-20-7 already refuses a departure with a rig out; abandonment is narrowed to a destroyed
   hangar or staff action and Q-23-2 defaults to keeping D-20-7 (§4).
7. Cover's close evidence needs a withheld departure and a zone that no cut here builds; cover
   leaves the gate (§8).
8. Doc 20 §8's trip-level lines are a 32-line notice ring, not a record; the black box must be
   durable from S.1 (§7).

Its should-fixes (the excursion's own cases — `NeverLeft`, implied launches, staff `Close` and
`Home`, `Parked`; the ship's `depart` versus `+ship/undock` and the lapsed burn; the corrected bounds
— the 24 IC h idle return, the 65 h coast; the misread resume rule; the arena-shaped `resume`/`install`
and the RAM inbox; `ic_integration_tests` not being ignored; three mis-cited sources) are folded
where they apply, and its nits (`SpatialParent`, `MindDropped.cause`, three kinds of hull, the
pool split's wording, the Customs Assertion front, the 30-TV lane) likewise.

## 16. The owner's rulings of 2026-10-27, and what they changed

On reading Draft 0.2 the owner ruled, in their words:

- **Q-23-6**: "the license that lapse mid-epoch holds to the close of the top-level passage, not
  each individual excursion .. the license is in a grace period until the hull reaches the next
  hangar." → D-23-9. For this to mean anything a carrier settled on a mining site must still be on
  its passage, so Draft 0.3 narrows D-23-3: a site is the field, the hangars are the corridor's five
  ports and the bays, the carrier's epoch is open while its rigs are out, and D-23-2 names that
  open sortie again. **The owner confirmed the narrowing the same day**: "hangars are the
  corridor's five ports and bays today, but player-built hangars/bases, and additional ports will
  come online .. but the point is exactly correct, a ship that is landed in the field does NOT end
  its passage." D-23-3 says so, and the hangar set is a declared one that grows.
- **Risk 1**: "the 'match' concept is a simulator/arena concept, not an IC concept." → D-23-10.
  The IC epoch's id is its own kind; the top-bit packing into the arena's `(match, unit)` number is
  withdrawn.
- **Q-23-1 to Q-23-5**: "all other questions defaults accepted as proposed." Q-23-1 is accepted as
  narrowed above.

## 17. Custody for moving anchors (the S.4b precondition)

**Draft 0.8, 2026-11-03, written read-only at `c83c2eff` (`origin/main`); rulings the same day: the
owner accepted Q-23-7 to Q-23-20 at their defaults, so every decision below stands as ruled
(Draft 0.9).** Doc 37 §5.5 names S.4b
the blocker of docs 32, 36 and GC.1 and says its precondition, "custody for moving anchors", has no
owning document; doc 38 Q-38-21 (accepted at its default) rules that the note is written here, as a
section of this doc, before S.4b's cut, from the questions §9's S.4b bullet lists — the claim
Open/Close over a carrier anchor that moves, the pool seat, the `abi` drop on resume and the
long-leg snapshots — and that S.4b's cut is then restated with the note's answers folded in (§17.10).
The questions are the owner's, in doc 38's form (Q-23-7 to Q-23-20, §17.12); the lead's proposals are
D-23-11 to D-23-20 (§17.11). Nothing in the tree changed. The note's premise is the one doc 17h
recorded when the owner split S.4 (17h l.584–596, option C): the M8e builder found that a four-owner
IC claim cannot sit on a surface excursion for three reasons — an IC base accepts only a
fixed-domain anchor, the Open needs the hull inside the base, and an active claim freezes the hull —
and every one of the three is a property of the claim code's OOC shape, not of the claim. This
section says what the claim is for an IC hull, and lets the three go.

### 17.1 What the tree holds (verified at `c83c2eff`)

- **The four owners and their Open.** A claim is one commit prepared by four owners in order
  (`mush-roll` `ic_claims/writer.rs:190–305`): the Roll's admission reader (`:198–203`,
  `IcAdmissionFacts`, `reader.rs:93–133` — the holding, the hull's title fence, the capacity,
  `PaidSeats { seats }` or `OwnedDispatch`), tacsim's seal participant (`:206–208`), tactics'
  custody (`:224–233`), sim's exclusion (`:251–259`) and the Roll's root (`:297–303`), the
  participants fixed as `retention, seal, custody, exclusion` (`config.rs:40–50`). The seal
  participant's Open re-seals the mind (`execution.rs:102–126`, `seal_with_policy` under
  `SortieId::Packed(next_transition)`, the book's ordinal, `:106–108`), compiles it before admission
  (`:134–137`) and refuses an IC epoch's id outright (`:148`; the book likewise at load,
  `state.rs:36–44` — S.1's L1 deferred "the book carries the host's id" to S.4). The book keeps one
  active open per unit (`state.rs:109`) under `MAX_OPEN` 1024 and `MAX_HISTORY` 8192
  (`protocol.rs:4–5`).
- **The missing producer.** `HostTransition { _private: () }` is private and non-Clone
  (`participant.rs:21–26`), minted only by `#[cfg(test)]` fixtures (`:499–547`, and `:552–613` for
  cover), with `StartPermit { unit: TacticalUnit, actor }` (`:27–31`) and `CloseEvent { actor, open,
  unit, seal, sequence, kind, at_ic, conditions: UnitMutationPlan, coverage }` (`:32–43`) carrying
  it. `IcHost::enqueue_start`/`enqueue_close` take the permits into a RAM inbox under random request
  ids (`host.rs:273–315`, `:288`, `:307`), one pending Open per actor (`:247–249`); `submit` must
  run from a real actor dispatch (`:321–328`), and the provider offers the ticket as a self-action
  of the actor, "Apply owned IC transition" (`provider.rs:59–72`, `:98–160`). The seal
  participant's prepare demands that the action being prepared be one of the claim's own two
  (`participant.rs:95–102`), so the Open cannot ride another provider's bundle.
  `IcRetainedExecution::install(world, &mut Arena)` and `IcHost::resume(world, open)`
  (`host.rs:172–232`) install a retained execution into a running arena — the integration story
  does exactly that and the mind thinks (`minds/bulk/ic_integration_tests.rs:1075–1091`,
  `think_seq > 0`), survives a reboot with its seal (`:1100–1140`), and holds its licence's seat
  against a second hull (`:1095–1098`).
- **The fixed base.** The sim's exclusion admits an Open only for a hull `Interior` to a
  `ConfiguredIcBase` (`mush-sim` `ic_exclusion.rs:645–660`), whose space must be
  `SpaceAnchor::FixedDomain` (`:341`, `:366`); it refuses a hull under a deployed mark (`:604`)
  or on a journey (`:622–636`). Its boot check requires the held hull's placement to equal the
  Open's image (`:960–975`, "held IC hull placement requires explicit recovery"), its journey
  check says in words "these fixed bases are not travelling carriers" (`:1002–1003`), and
  `check_plan` refuses every placement write and projection of a held hull and any retire of the
  base's domain (`:1021–1047`). A bay is the other anchor kind, `MovingObject { host: hull }`
  (`data.rs:793–797`; the Dray-P's bay `port/hull.rs:869`, a rig's cab `port/vehicle.rs:723`).
- **The freeze.** Tactics' `is_ic_custodied` (`ic_custody.rs:265`, `holds` `:164`) refuses the
  field's check-out (`custody.rs:160–163`, "held by dedicated IC custody"), component replacement
  (`component_replacement.rs:243`), every mutation ("held by an IC sortie", `:272–277`) and the
  sim's movement custody (`mush-tactics` `movement_custody.rs:95`; `mush-sim`
  `movement_custody.rs:110`); the sim reads IC custody as custody (`deployed.rs:248–250`,
  `:300–302`). The Close's identity check requires the live unit to equal the Open's image
  (`participant.rs:310–316`, `current_unit` `:272`, `:317`) and projects every physical change as
  the Close's `conditions` plan over that image (`:324–337`; `ic_custody.rs:218`): the OOC shape, in
  which nothing touches a held hull but its Close.
- **The field.** Admission partitions on `held_out` (`field/admit.rs:338–341`, `:518–521`:
  `is_ic_custodied`), lays each held hull on a copy of the record as a one-cell object
  (`with_held_walls`, `:527–554`; the record is never written, `:526`), tells the site `HELD_OUT`
  (`:512`), and admits everyone else; the check-out is all or nothing (`:393–407`). Every entrant,
  cored or not, is sealed on the ROM over its epoch (`custody_unit`, `:647–700`, `seal_rom_for`
  `:677–690`; `Controller::Mind` with no pilot, `:668`): the field never reads S.4a's dispatch seal
  (no `DispatchSeal` reader under `field/`). The join refuses a `SealEngine::Scripted` seal
  (`arena/join.rs:130–131`), the task says "an IC field carries no licensed mind until doc 23
  S.4b" (`arena_task.rs:742`) and installs runners from `sources` the field never passes
  (`:753`, `:765`; none under `field/boot.rs`). An arrival excludes held-out hulls
  (`field/carrier.rs:257`) and is sealed by the same `custody_unit`. A field resumes from its live
  record and snapshot (`field/boot.rs:145–290`; re-joining `arrived` rows the snapshot predates,
  `:185–195`), voids from the last snapshot (`:293–302`), snapshots every
  `[tacsim.arena].snapshot_ms` of arena time (`arena_task.rs:101–124`; `field/mod.rs:338–367`),
  and at demotion checks every unit in through the IC condition plan (`field/close.rs:280`) —
  carrying no mind memory home (no `memory` under `field/`). Since doc 36a §11 row 1 the walls are
  known to be rebuilt only at admission; PR #340 (open) puts them on the live record
  (`FieldLive::held`, `admit::ground_with_held`).
- **The surface and the ship.** S.4a seals the held mind at the launch's dispatch
  (`surface/dispatch.rs:363–446`, `SortieId::ic(tv, launch_ic)` `:398`; `seal_held` `:303`),
  carries it on the act (`excursion.rs:728–734`) and the open epoch (`OpenSortie.seal`,
  `:1283`), and leaves it dormant: "no IC claim opens — the claim book's Open and Close and the
  pool seat land with the moving-anchor custody design" (`dispatch.rs:13–15`). The dispatch's
  `Admitted { class, host, count }` records the pool admission (`black_box.rs:138–152`, D-23-8).
  The only cored hull is a rig (M8b cores the TV; the Dray is Class 0, `dray.json:200`; "a cored
  ship" waits, 17h §4), so the ship's epoch (`port/epoch.rs`) seals the ROM and carries no claim.
- **The `abi`.** `MindSeal.abi` (`arena/mind_seal.rs:80`) records `MIND_ABI`
  (`minds/mod.rs:202`, `rhai=1.26.0;api=7;stock=3`). The OOC resume refuses a differing seal by
  name (`match_/resume.rs:676–700`, `ResumeRefusal::MindAbi` `:683–688`); `DropCause::Abi` is
  produced "by NOTHING in the arena" and carried for the region map (`arena/mind.rs:243–250`), with
  its player sentence ready (`arena_bridge.rs:5787`). `MindState.memory` is the mind's bytes
  (`arena/mind.rs:586–590`).
- **The pool.** `pool_for`, `load_of` and `admit_side` (`match_/pool.rs:59`, `:71`, `:143`) are
  read at the lobby and a match's check-out only; nothing under `field/` reads them. The think
  pool is one per process (`minds/pool.rs:1–35`).
- **Testimony.** `EarnedContacts` is a per-author ring of 256 (`minds/earned_contacts.rs:18`,
  `admit` `:39–46`, `has_earned` `:51–56`), registered with the claim
  (`registry_inspection/mod.rs:249`), "no producer yet", fed in design "from the author's own
  `Detected { viewer == me }` in an IC fight, persisted beside its RAM mirror" (`:4–11`); the only
  consumer is `@mind/claim`'s observer rule (`registry_inspection/claims.rs:53`, `:99–103`). The
  field's ledger already writes `Detected` and `Classified { viewer, contact, target }`
  (`arena/ledger.rs:168–190`).

### 17.2 The anchor — D-23-11, D-23-12

**The claim's anchor is the hull's hangar as §2 defines it, named by id, and the claim's lifetime
is the hull's epoch — D-23-11.** For a rig that is the bay of the carrier it launched from — the
`Hangar::Bay { hull, slot, sortie }` the `mind_sealed` line already names (S.1's L7, S.2's V4) — and
for a cored ship later it will be the port's berth. A claim is a custody record over an epoch, not
over a place: it opens when the epoch opens (the launch's end, D-23-1) and closes with the epoch's
close, by the four kinds the two protocols share (`IcCloseKind`, `EventKind`,
`records.rs:55–60`), so it needs no image of where the hull stands and no domain to retire.
"Moving" is therefore not a property the claim tracks; it is the list of what may happen to the
hangar and the hull while the claim is open, each already ruled elsewhere and each leaving the claim
where it is:

- **A voyage leg.** A rig's epoch is one excursion (D-23-3) and a carrier cannot lift with a rig
  out (D-20-7, D-23-4), so no claim is open on a rig riding in a bay. The one exception is doc 32
  F-6's live leave-behind lift (`field/carrier.rs:25–27`, `exit.rs:22–28`): the carrier leaves the
  field and the site; the rig stays in the field with its epoch and claim open, as a rig staff
  `close`d with it out does (S.3), and the claim closes when the rig's epoch does — `RecoveredToBase
  { base: <the recoverer> }` when a second carrier homes it (S.3's A4), or `Destroyed`.
- **An F-6 arrival mid-field.** The arriving carrier's rigs are `Interior` to its bay with no epoch
  open, so no claim; the carrier itself has no core (17h §4) and joins on the ROM as today (doc 36
  §11). Nothing in the claim path moves.
- **A lift-out by F-6 departure** is the first bullet seen from the carrier: its own ship epoch
  stays open (S.2) and carries no claim.
- **A wreck.** A carrier wrecked with a rig out leaves the rig's epoch open (S.3's A6; GC.3 wrecks
  the bay's rigs, `field/close.rs:383`, and closes nothing of a rig on the ground); the claim
  stays open with it and closes `Destroyed` only when the rig's own hull is irrecoverable (GC.3's
  `close_destroyed`) or by recovery. A rig's own wreck inside a field closes its epoch `Destroyed`
  in the close's bundle (S.1 as amended by GC.3), and the claim's Close follows it (§17.3).
- **The site** is never the anchor: a site is the field (D-23-3), and the field moves the hull
  under its own custody mark (§17.4).

**S.4b opens claims for rigs only — D-23-12.** The ship's epoch seals the ROM and carries no claim
until a cut cores a ship (17h §4); when one does, D-23-11 holds unchanged with the port's berth as
the hangar, which moves less than a bay. A carrier's F-6 arrival and departure, and its wreck,
therefore touch no claim of its own in S.4b.

### 17.3 The production Open and Close — D-23-13, D-23-14

**Who produces `HostTransition`.** The surface host — the same composed provider that opens and
closes the epoch (`tacsim_surface_done` and the staff recovery, S.1's L2) — and nobody else: not the
voyage (no claim rides a ship, D-23-12) and not the field (a field admits a claim already open and
never opens one, §17.4). The `StartPermit` is minted in the prepare of the launching act's finish,
from the act's `DispatchSeal` (`excursion.rs:728–734`) and the rig's live `TacticalUnit`, when and
only when the finish opens the epoch (S.1's L3: the excursion survives with the rig out of every
bay; a `NeverLeft` mints nothing); the `CloseEvent` is minted in the prepare of each finish that
closes the epoch — `+tv/recover`'s `Done` and staff `home` (`ReturnedToBase`/`RecoveredToBase`,
S.1, S.3), the second-carrier completion (S.3's R1), GC.3's `close_destroyed` (`Destroyed`) — with
the kind that finish writes, the hull as it stands, and a `sequence` that is the epoch's close
ordinal (1). `fixture_start`/`fixture_close` and `ic_integration_tests`' stories stay as the claim
path's own gate; `participant.rs` exports the constructor to the surface host under a `pub(crate)`
seam that only the surface's prepare can reach (the module's own rule, `:21–23`: "a later host
must supply its actual transition before any permit is minted").

**The Open and the Close are the host's own commits as the operator, on the fire after the finish
that opened or closed the epoch, and the epoch record is their cursor — D-23-13.** The seal
participant admits only its own two actions (`participant.rs:95–102`) and the provider's ticket is a
self-action of the actor (`provider.rs:59–72`), so the Open cannot ride the finish's bundle; it is
submitted the way S.2's epoch arm submits `tacsim_voyage_epoch`: at most one claim transition per
rig per fire, as the operator through `Fire::as_principal` (the principal per D-23-5: the
excursion's operator), the ticket read off the provider's offer. The ordering is fixed: **the epoch
first, the claim second.** The finish lands the `mind_sealed` line and `RigRecord.sortie`
(`excursion.rs:1242`) in one bundle, with a new field `OpenSortie.claim: ClaimSeam` — `Owed`,
`Open { open: CommitToken }`, `Closing { open, kind }`, `Closed { open, close: CommitToken }` (serde
default `None` for a rig with no held mind, so schema 1 stands and every S.1–S.3 record decodes byte
for byte) — and the next surface fire finds every `Owed` and `Closing` seam, enqueues the permit it
re-mints from the record, submits, and moves the seam at the claim's commit, in the claim's own
bundle through the finish's participant. A fire that finds the book already holding an active open
for the hull (`state.rs:109`) or a used request (`request_used`) moves the seam without
submitting; `IcHost`'s one-pending-per-actor guard (`host.rs:247–249`) and `MAX_PENDING` are
capacity, never state, and a refused enqueue is retried next fire. So a re-run fire mints nothing
twice, a reboot between the finish and the Open re-owes it from the record (the RAM inbox is
empty at boot by design, `ic_sortie/boot.rs:11–12`: "no closure is inferred from a reboot"), and the window
between the finish and the claim's commit is V1's accepted window (S.2), one fire plus deferrals.

**What a restart does.** (i) Between the finish and the Open: the record says `Owed`; the rig stands
dormant on the surface; the next fire opens the claim. (ii) Between the Open and the next field
snapshot, with the rig on the surface: the claim is durable at its commit (the Roll's row and the
four owners' documents) and the excursion resumes as S.1's restart test already proves; nothing to
re-do. (iii) With the rig in a field whose live record names it and whose snapshot predates its
join: `boot::resume` re-joins it (`field/boot.rs:185–195`), installing the retained execution
(§17.4); with no snapshot at all the field voids from the check-out as it went out
(`:293–302`) and the rig stands on the surface, dormant, its claim open, until the next promotion.
(iv) Between an epoch's close and the Close: the record says `Closing`; the rig is in its bay; the
next fire closes the claim. The licence's seat is held through every window, which is the point of
a durable cursor.

**One seal per epoch — D-23-14.** D-UM-45 says the seal is per sortie and S.4a already takes it at
the dispatch; `Execution::capture` seals the mind a second time at the Open (`execution.rs:102–126`)
because the OOC fixture had no earlier seal. S.4b captures the claim's `Execution` from the
dispatch's `MindSeal` and the holding the dispatch resolved (`DispatchReads`), so the claim's
`seal_digest` equals the open line's and an inspection reads one seal; `Execution::validate` and
the book accept `SortieId::Ic { hull, at_ic }` as the seal's sortie (`execution.rs:148`,
`state.rs:36–44`; S.1's L1 deferral), while the book's transition ordinal stays its own counter and
the Roll's `SortieNumber` (`writer.rs:276`) keeps carrying that ordinal — the Roll's row formats do
not move. The compile-before-admission step stays (`:134–137`).

### 17.4 Physical custody while the hull moves — D-23-15

**IC custody means "moved only by the epoch's hosts", not "frozen in a base" — D-23-15.** The
exclusion's Open records the hangar (the bay's hull and slot) and the epoch's id in place of the
`BaseImage`, and admits an Open for a hull `Direct` on the domain the hangar's carrier is settled
on — the rig is already on the ground when the finish runs, which was the M8e builder's second
reason — with no `ConfiguredIcBase` at all for a rig (the `ic_base` registration and `MAX_IC_BASES`
stay for the fixture and for a port hangar later). Its freeze is re-scoped from every placement
write to **re-anchoring**: `check_plan` (`ic_exclusion.rs:1021–1047`) admits a pose write on the
hull's current domain and a re-parent `Interior` into the claim's own hangar or into a recoverer's
bay (S.3's second carrier), and refuses every other change of `SpatialParent` (a staff teleport, a
transfer's placement, a load into a stranger's carrier) and any retire of the domain the hull
stands on; the boot check (`:960–995`) requires the hull to stand on a domain or in a bay the record
allows, not at the Open's pose. On the tactics side the field's check-out admits an IC-custodied
hull whose claim is the IC host's (`custody.rs:160–163` relaxed for a `DeployedMark::field`,
`deployed.rs:64–84`), and the two custodies coexist: the IC claim says who the hull's mind is and
who holds its seat for the epoch; the field mark says who moves it now. A field is the lawful mover
while its mark stands (the arena's poses come home at check-in, Q-CF-31), the surface tick while the
rig is out of a field (its drives and legs, `mush_sim::place` on the site's domain), and the
recover's finish for the one re-parent into the bay. Mutation under the claim is the hosts' own
(the IC condition plan at check-in, `field/close.rs:280`; repairs in a bay are the epoch's close and
beyond), so **the Close's physical image is the live hull**: the Close carries an empty
`UnitMutationPlan` fenced on the live revision (as the fixture does, `participant.rs:538–542`), the
identity check compares the live unit's object and lineage, not its bytes, with the Open's
(`:310–316`), and custody's `project_close_unit_digest` (`ic_custody.rs:218`) projects over the
live unit. The Open's refusal of a hull under a deployed mark (`ic_exclusion.rs:604`) stands: a
claim opens at a launch, never inside a field.

**Salvage under a claim.** A hull under an open claim that a stranger's tug loads (GC.4/GC.5's
salvage custody row, `surface/claim.rs`) is the re-parent the freeze would refuse. The default is
to admit it and close the claim at the load's completion `RecoveredToBase { base: <the carrier> }`
naming the recoverer, exactly as S.3's second-carrier home closes a rig's epoch at the placement
(R1): the IC claim is a custody record, not a title, and the salvage row and the Registry's
inspection at the next berth are where a stolen mind is found (M8's channels). Q-23-11.

### 17.5 The pool seat — D-23-16

Two seats are called "the seat" in this doc's history and they are different things. **The licence
seat** is the Roll's: a paid licence's `PaidSeats { seats }` or an owner's `OwnedDispatch`
(`reader.rs:127–133`), bound by the claim from Open to Close, one hull per seat (the story at
`ic_integration_tests.rs:1095–1098`). S.4b gets it with the Open and holds it through every window
of §17.3; a second launch under the same licence while a claim is open is refused at the dispatch
with the Roll's word, before any rig leaves the bay. **The compute seat** is D-UM-36's: a hull
loads its mind's class ceiling against a deterministic allotment, "admitted at the startup in the
hangar or base, never mid-sortie, a hull already out never evicted", with a region's split left to
"the corridor's design" and, by Q-23-4 (accepted), rationed by the region's de facto holder as a
published allotment, recorded and never refused until a mind thinks IC.

**The compute seat is the site's, counted at the launch, held for the epoch, and charged only in a
field — D-23-16.** The dispatch already records `Admitted { class, host, count }` (`black_box.rs:138–152`);
S.4b makes the count a sum of `load_of(class)` over the site's open epochs that carry a held mind
(`match_/pool.rs:71`) against `pool_for` of that site's open epochs (`:59`, the stock keys
`config.rs:798–808`, read per site: the lead's reading of "a region's pool", a site being the
region a field runs on), and **refuses at the launch with the arithmetic** (`PoolRefusal`'s
wording) once the sum would pass it — D-UM-36's "refused there before the hull leaves", which S.4b
is the first cut able to honour. At the stock keys no site refuses below a dozen Class III minds,
so the gate bites only under an operator's lowered pool or a holder's rationing, and the holder's
rationing verb is not built here (a later cut; the allotment is a provisional default under the
owner's economy rule). The seat is held while the hull is out, dormant or not, and spent only while
its hull is in a field: the field task's think pool is process-wide (`minds/pool.rs`) and its
admission counts nothing (no `pool_for` under `field/`), so a field never refuses a mind for the
pool and a hull already out is never evicted. **On the carrier between fields** there is nothing to
hold: a rig in its bay has no epoch, hence no claim and no seat; a rig out on the surface between
two fields on one excursion holds its seat and spends nothing. Q-23-12 asks whether the refusal
starts now or stays recorded-only.

### 17.6 The `abi` drop on resume, and Q-36-44's wall as the fallback — D-23-17

Doc 17 §13's IC rule is unchanged: across a `MIND_ABI` or rhai change a sortie keeps its epoch,
seal and memory, every mind hull on it drops to the drive ROM, and `MindDropped` is journalled with
the cause word `abi`, the sealed record keeping the old ABI. S.4b says *where* that happens, because
an IC hull thinks only inside a field: **the drop is decided per hull at the moment its retained
execution is installed into a field's arena — admission, an F-6 join, or a field's resume — never
at boot and never by voiding the field — D-23-17.** `IcRetainedExecution::install`
(`host.rs:172–189`) gains the check the OOC resume makes by name (`match_/resume.rs:683–688`): a
seal whose `abi` is not this build's, or whose source the store no longer compiles, installs the
hull on the ROM — Standard Doctrine as this build ships it, which is what every un-piloted field
hull flies today (`admit.rs:668`) — sets `MindState.dropped = Abi`, and journals one `MindDropped {
cause: abi }` on the hull's black box in the field's segment, with `dropped_line`'s sentence to the
operator (`arena_bridge.rs:5787`); the claim, the licence seat and the compute seat stay, the
epoch's record carries `dropped: abi` so a second field on the same epoch does not journal it
again, and the next seal at a hangar is on the new ABI. A field's own resume across the change is
therefore never refused for a mind (the field's units are the kernel's, Q-CF-33/35, and a void
would demote every party for one hull's engine); the OOC refusal by name stays the OOC rule.

**Q-36-44's wall is the fallback for an execution that cannot be installed at all**, which is not
the ABI case: a retained open the four-owner audit refuses (`audited_state`, `host.rs:191–210`), a
closed-or-crossed claim (`:182–186`), a record the book cannot read. Such a hull is `held_out` as
GC.1 built it (`admit.rs:338–361`; the walls on the live record once PR #340 lands) — an inert
obstacle, its claim untouched, the site told `HELD_OUT` — and is paged once a boot. **It leaves the
wall state when the field ends, or earlier when the hull leaves the site's ground** (a recover or a
staff home, which the surface tick may still drive under D-23-15); there is no staff verb that
closes a claim, since the four kinds are the epoch's close and a wall is evidence of a store that
needs an operator, not of a hull that needs a word. A rig whose claim is still `Owed` at admission
(the §17.3 window) is neither walled nor refused: admission waits one tick for it, as it waits for a
ramp handoff in flight (`admit.rs:301–313`). Q-23-13 and Q-23-14.

### 17.7 The long-leg snapshots — D-23-18

Doc 17 §22 sized "long-sortie snapshots" for a region sim task running a mind for hours; in the
S.4b shape a mind runs only inside a field, whose snapshot is the kernel's at
`[tacsim.arena].snapshot_ms` of arena time and carries every `MindState` row whole — `think_seq`,
the pending answer, the memory bytes (D-UM-23, `arena/mind.rs:580–605`) — beside the live record
(Q-CF-33: "the snapshot cadence is the kernel's"), and outside a field the mind is dormant with
nothing to snapshot beyond its seal (17h §4). What a long leg loses today is the **memory between
fields**: demotion and exit check a unit in through the condition plan and drop its `MindState`
(no `memory` under `field/`), so a mind that fought at dawn and again at dusk on one excursion
remembers nothing of the morning, against §13's "keeps … its memory bytes". **The long-leg
snapshot is the mind's memory carried on the epoch's record — D-23-18.** At every check-in (exit,
demotion, a void's return from the last snapshot) the host writes the unit's `MindState.memory`
and `think_seq`, and its `dropped` cause, onto `OpenSortie.mind: Option<CarriedMind>` (serde
default; bounded by `memory_bound`, the per-class cap doc 17 §23 names) in the check-in's own
bundle; the next install on that epoch seeds the row from it, so the mind resumes its own memory
and its `think_seq` is monotone across the epoch; a field's own cadence stands and no second
snapshot mechanism is built. The record is bounded by one rig's memory cap, which the M7 stack-size
lint will see (the rig record rides many a tick's frame, `excursion.rs:730–733`: boxed, as the
dispatch seal is). Q-23-15 asks whether memory is carried or forgotten at demotion.

### 17.8 The testimony producer — D-23-19

**A field contact is earned when the author's own hull classified the target in an IC field, and
the field host feeds the index at each unit's check-in — D-23-19.** `EarnedContacts::admit(author,
target)` (`earned_contacts.rs:39–46`) is called by the host at exit, demotion and a void's return
for every `Classified { viewer: <the unit>, target }` line (`arena/ledger.rs:182–190`) in the
field's segment of that unit's black box — a `Detected` blip is not "saw" (D-UM-55's observer rule
is about what the author's sensors told apart) — with the author the unit's seated pilot at the
line's instant, read from the live record's row and the seat epochs; for a hull flown by its mind
with no pilot, the author is the seal's warrant principal (D-23-5), whose instruments the mind's
sensors are and whose box read is the only other channel that would learn it. The index is
persisted beside its RAM mirror as the module's head asks (`:10`): one small document per author,
`tacsim:earned:<author>`, the ring of 256 (`:18`), written in the check-in's bundle and restored at
boot. What M8g needs is exactly this: `@mind/claim` refuses every hull today with the one sentence
(`claims.rs:99–103`), so the tier-2 remainder 17h M8g lists "To S.4b" — the pending-transfer
refusal and the IC audit over a draft claim — has no claim to run through; with the producer, an
author whose rig classified the suspect hull in a field may file, and the two stories run over a
real claim. The producer needs no licensed mind and no claim: any IC field writes the lines today.
It is therefore its own small cut (§17.10's S.4b-T), buildable now, beside S.4b rather than inside
it. Q-23-16 and Q-23-17.

### 17.9 Defect thread versus S.4b

**The defect thread's** (fixed or owed there, not S.4b's): the walls rebuilt only at admission
(doc 36a §11 row 1; PR #340's `FieldLive::held` and `admit::ground_with_held`, open); S.2's owed
"a carrier wreck writes nothing to `tacsim.ship_epoch`" (doc 37 §5.5); the `LeftUnit` instants and
the party-letter collision (36a §11 rows 9 and 2, fixed in #340 and lane 5); GC.1's untested
"admission run with a held hull on the ground" (doc 36 §9, the gates: "no tacsim rig turns on the
four-owner IC custody") stays untestable until S.4b's harness exists and is then S.4b's first gate,
not a defect.

**S.4b's** (this section, built as §17.10 says): the production `HostTransition` and the
`OpenSortie.claim` cursor (D-23-13); the one-seal capture and the book's IC id (D-23-14); the
exclusion's re-scoped freeze, the custody coexistence and the live-hull Close (D-23-15); the
compute seat's count and refusal at the launch (D-23-16); the install-time `abi` drop and the wall
as fallback (D-23-17); the carried memory (D-23-18); the field admitting a held mind's seal in place
of the ROM (`custody_unit` reading `OpenSortie.seal` and installing the retained execution; the
join admitting it; the task's `sources`/`install_runners` seam); the dispatch's seat refusals
(licence and compute); M8g's "To S.4b" stories once S.4b-T lands. **S.4b-T's** (its own cut, may
start now): the testimony producer and its document (D-23-19). **Neither's** (named here so nobody
builds them by accident): the holder's rationing verb for the compute pool (after S.4b); the
org-held licence's claim (M8c B3's `org=`; the Roll's reader admits the actor's own holding,
`IcAdmissionRequest::own`, and an org path is the Roll's cut, Q-23-19); a cored ship's claim over a
port hangar (17h §4); hull cover's withheld departure and zone (§8, unchanged); a staff verb over
claims (none, §17.6).

### 17.10 S.4b restated

**S.4b — the thinking mind in the field: the claim Open/Close, the seat, the `abi` drop and the
carried memory (medium-large; after S.4b-T or beside it; the host is the IC field, D-23-20).**
Through the cut loop (§9), one PR off `main`, the owner merging.

- **Host.** The IC field of docs 32 and 36 is where a licensed mind thinks IC; doc 20 §17's drone
  phase (reserved, not built, `20-surface-mining.md:2922`) is no longer S.4b's gate and D-UM-47's
  "lands inside the drone phase's cut" is amended by this section. Q-23-18.
- **The producer and the cursor** (D-23-13): the surface host mints `StartPermit`/`CloseEvent` in
  the finishes that open and close an epoch for a rig whose act carries a `DispatchSeal`;
  `OpenSortie.claim` and the surface fire's claim arm; the Roll's seat refusal at the dispatch.
- **One seal, the IC id** (D-23-14): `Execution` captured from the dispatch seal; `SortieId::Ic`
  accepted by `execution.rs:148` and the book; the collision test of S.1 extended to the claim's
  digest equalling the open line's.
- **Custody** (D-23-15): the exclusion's hangar-and-epoch image and re-scoped `check_plan`/boot
  checks; the field check-out admitting the IC host's own claim; the live-hull Close; the salvage
  load closing `RecoveredToBase` (Q-23-11).
- **The field takes the mind**: `custody_unit` seals from `OpenSortie.seal` when the rig's claim
  is `Open`, the host installs the retained execution through `IcHost::resume(open)?.install`
  at admission, join and resume (the `sources` seam carrying the retained source), the join's
  scripted refusal becomes "a scripted seal with no retained execution"; `arena_task.rs:742`'s
  sentence goes.
- **The compute seat** (D-23-16): the site sum at the dispatch, the refusal with the arithmetic,
  `Admitted.count` as the loaded ops.
- **The `abi` drop and the wall** (D-23-17): the install-time check, the journal line once an
  epoch, the wall for an uninstallable execution, paged once.
- **The carried memory** (D-23-18): `OpenSortie.mind` written at every check-in, seeded at every
  install.
- **Acceptance, through production command paths, at 256 and 512 (doc 38 Q-38-29).** One story in
  the field story's harness (`tests/shard_9/field_story.rs`: `driving_rig` on Ahuna Flank at 256,
  Urvara Reach at 512, `:405–416`) composed with M8e's fitted roster
  (`excursions::stocked_fitted`, `fit(RIG_I)`, `shard_9/ic_sortie_surface.rs:2553`) and the filed
  commerce world the licensed path needs (`:2794`; M8g's `LaunchComposition`), which is the cut's
  first task — PR #340 records that the shard_1 field harness composes no IC custody. The story: an
  operator's cored rig launches `+tv/1 launch mind=filed:<n> go` from a Dray-P settled on the pad;
  the finish opens the epoch and the next fire opens the claim (the Roll's row, the four owners'
  documents, `seal_digest` equal to the open line's, the licence's seat held: a second launch under
  the licence on another rig refused by name); a raider's mark promotes the site; the rig is
  admitted under its own seal, not the ROM, and thinks (`think_seq > 0` on its row), drives and
  fires under doctrine's orders, lands one hit and takes one; **a restart mid-field** resumes the
  field from its snapshot with the mind's `think_seq` continuing and the claim still active
  (`IcHost::resume` on the open token); the field demotes; the rig's memory is on its epoch record;
  a second promotion on the same ground re-admits it with that memory; `+tv/1 recover go` exits,
  closes the epoch `ReturnedToBase { base: hull }` and the next fire closes the claim, the seat
  released (the second launch now admitted); a reboot between the recover and the Close re-owes
  and lands it. Beside the story, in-source: the `abi` drop (a seal recorded under `OLD_ABI` as
  `store.rs:3584`'s test shapes one, installed → the ROM, one `MindDropped { cause: abi }`); the
  wall for a refused audit; the re-scoped `check_plan` admitting a surface drive and refusing a
  staff teleport; the Close over a damaged live hull. The bot types the player's half over SSH
  (Q-38-29's default; the bot's vocabulary gains `+tv/<n> launch mind=`). Q-23-20.
- **Gates.** The claim path's own stories (`ic_integration_tests`) unchanged; the OOC practice
  hashes unmoved (no OOC arena ever installs a retained execution); `mush-contract-tests`' census,
  panic budget and restriction lints; the SM.12c cost lane re-run with the claim arm's write
  (`other.ic_mind_transition`, two per epoch); the M7 stack-size lint on `RigRecord`.
- **Not in S.4b:** §17.9's "neither's" list.

**S.4b-T — the testimony producer (small; independent; may start now).** D-23-19:
`EarnedContacts::admit` fed at check-in from the field segment's `Classified` lines, authored by
the seated pilot else the warrant principal; `tacsim:earned:<author>` persisted and restored; then
M8g's "To S.4b" stories (the pending-transfer refusal and the IC audit over a draft claim) over a
real claim. Test: a rig that classified a raider in the field story's first field may `@mind/claim`
it after, a rig that only detected it may not, the ring survives a reboot, and a bystander who saw
nothing is refused with the one sentence.

### 17.11 Decisions proposed here

| # | Decision | Status |
|---|---|---|
| D-23-11 | The claim's anchor is the hull's hangar by id (a rig's bay: hull and slot; a ship's berth later) and the claim's lifetime is the hull's epoch, opening and closing with it by the four kinds; the claim keeps no image of where the hull stands; a site is never the anchor | **accepted by the owner, 2026-11-03** (Q-23-7) |
| D-23-12 | S.4b opens claims for rigs only; the ship's epoch carries no claim until a ship is cored, and D-23-11 then holds unchanged | stands with Q-23-7's ruling, 2026-11-03 |
| D-23-13 | The surface host mints `StartPermit`/`CloseEvent` in the finishes that open and close an epoch; the Open and Close are the host's own commits as the operator on the next surface fire (`Fire::as_principal`), with `OpenSortie.claim` as the durable cursor; epoch first, claim second; a reboot re-owes from the record | **accepted by the owner, 2026-11-03** (Q-23-8) |
| D-23-14 | One seal per epoch: the claim's `Execution` is captured from the dispatch's seal; the book and `Execution::validate` accept `SortieId::Ic`; the Roll's `SortieNumber` keeps the book's ordinal | **accepted by the owner, 2026-11-03** (Q-23-9) |
| D-23-15 | IC custody means moved only by the epoch's hosts: the exclusion records the hangar and epoch, admits an Open on the ground, freezes re-anchoring only; the field mark and the claim coexist; the Close's physical image is the live hull | **accepted by the owner, 2026-11-03** (Q-23-10, Q-23-11) |
| D-23-16 | The compute seat is counted at the launch against the site's allotment, held for the epoch, spent only in a field, refused at the launch with the arithmetic under a provisional stock allotment; never refused or evicted in a field | **accepted by the owner, 2026-11-03** (Q-23-12) |
| D-23-17 | The `abi` drop is decided per hull when its retained execution is installed into a field (admission, join, resume), to the ROM with one `MindDropped { cause: abi }` an epoch; it never voids a field; Q-36-44's wall is the fallback for an execution that cannot be installed at all, left when the field ends or the hull leaves the ground | **accepted by the owner, 2026-11-03** (Q-23-13, Q-23-14) |
| D-23-18 | The long-leg snapshot is the mind's memory carried on the epoch record at every check-in and seeded at every install; the field's cadence stands | **accepted by the owner, 2026-11-03** (Q-23-15) |
| D-23-19 | The testimony producer feeds `EarnedContacts` at check-in from the field's `Classified` lines, authored by the seated pilot else the warrant principal, persisted per author; it is its own cut, S.4b-T | **accepted by the owner, 2026-11-03** (Q-23-16, Q-23-17) |
| D-23-20 | S.4b's host is the IC field; the drone phase is no longer its gate; its acceptance runs at 256 and 512 with the bot per Q-38-29 | **accepted by the owner, 2026-11-03** (Q-23-18, Q-23-20) |

### 17.12 Questions for the owner

**All accepted at their defaults by the owner, 2026-11-03** ("Q-23-7 through 20 accepted as defaults").

**Q-23-7 — What the claim is anchored to.**
- **Decision (accepted default, 2026-11-03):** the epoch, with the hangar named by id (D-23-11): a claim per excursion,
  opened at the launch's end and closed with the epoch's close kind.
- **Alternative:** a field-scoped claim, opened at admission and closed at demotion, leaving the
  fixed-base code as it is.
- **Why:** doc 17 §13 and §5 make the claim the custody record M8's inspection reads at the two
  ends of a sortie and the thing that holds the licence seat for its length; a field-scoped claim
  would hold the seat only while shots fly and leave the hangar ends with nothing to inspect.

**Q-23-8 — Where the Open commits.**
- **Decision (accepted default, 2026-11-03):** its own commit on the surface fire after the finish, as the operator, with
  the epoch record as cursor (D-23-13; S.2's epoch arm pattern).
- **Alternative:** inside the finish's bundle, which needs the seal participant's action check
  (`participant.rs:95–102`) relaxed to admit the surface's action and the four owners prepared
  from a foreign provider.
- **Why:** the claim path's one-action rule is its security boundary (the module says so at every
  type), and the accepted V1 window already exists for the ship.

**Q-23-9 — One seal or two.**
- **Decision (accepted default, 2026-11-03):** one: the claim captures the dispatch's seal (D-23-14).
- **Alternative:** the Open seals again as today and the record carries two digests.
- **Why:** D-UM-45's seal is per sortie; two seals on one epoch would make the inspection choose.

**Q-23-10 — The physical freeze.** **(expensive to reverse)**
- **Decision (accepted default, 2026-11-03):** re-scope the exclusion to re-anchoring and let the epoch's hosts move the
  hull (D-23-15).
- **Alternative:** keep the freeze and drop the sim's exclusion from the IC composition for rigs
  (three owners), relying on tactics custody and the hangar id alone.
- **Why:** the exclusion is the owner that knows the spatial graph, and a claim that cannot say
  "not into a stranger's hold" is weaker than the one we have; four owners stay, their reading of
  "held" changes.

**Q-23-11 — A claimed hull loaded as salvage.**
- **Decision (accepted default, 2026-11-03):** admit the load and close the claim `RecoveredToBase` naming the
  recoverer at the completion (§17.4).
- **Alternative:** refuse the load while a claim is open.
- **Why:** a claim is custody, not title; refusing would make a licensed mind an unsalvageable
  hull, and the Registry's inspection at the next berth is where the mind's theft is found.

**Q-23-12 — When the compute seat refuses.**
- **Decision (accepted default, 2026-11-03):** at the launch, with the arithmetic, under a provisional per-site stock
  allotment (D-23-16); the holder's rationing verb later.
- **Alternative:** keep D-23-8's recorded-never-refused until the rationing verb exists.
- **Why:** D-UM-36's region rule says refuse in the hangar before the hull leaves; S.4b is the
  first cut that can, and at the stock keys nothing is refused.

**Q-23-13 — The ABI mismatch in a field.**
- **Decision (accepted default, 2026-11-03):** the hull is installed on the ROM and fights, one journal line (D-23-17).
- **Alternative:** the hull is walled (Q-36-44's rule applied to the ABI case too).
- **Why:** doc 17 §13 wrote the IC rule to strand nothing; a walled rig in a fight is stranded in
  the fiction's eyes, and the ROM is the fallback every other drop uses.

**Q-23-14 — When a walled hull leaves the wall.**
- **Decision (accepted default, 2026-11-03):** when the field ends or the hull leaves the site's ground; no staff verb
  over claims (§17.6).
- **Alternative:** a staff `@tacsim/claim close #hull` that closes the claim `RecoveredToBase`.
- **Why:** a wall is evidence of a store that needs an operator; a verb that closes claims is a
  verb that forges a close kind, which the module refuses to export for a reason.

**Q-23-15 — Memory between fields.**
- **Decision (accepted default, 2026-11-03):** carried on the epoch record at every check-in (D-23-18).
- **Alternative:** forgotten at every demotion; the mind starts fresh in each field.
- **Why:** §13 promised the memory bytes across a resume, and a mind that cannot remember the
  morning's raider is not the mind the operator licensed.

**Q-23-16 — Who earns a mind-flown hull's contacts.**
- **Decision (accepted default, 2026-11-03):** the seal's warrant principal when no pilot is seated (D-23-19).
- **Alternative:** seated pilots only; a mind-flown hull earns nothing for anyone.
- **Why:** the operator reads that hull's box anyway, and D-UM-55's point is that nobody else does.

**Q-23-17 — The testimony producer's cut.**
- **Decision (accepted default, 2026-11-03):** its own small cut, S.4b-T, started now beside S.4b.
- **Alternative:** inside S.4b.
- **Why:** it needs no licensed mind and unblocks M8g's remainder on its own.

**Q-23-18 — S.4b's host.**
- **Decision (accepted default, 2026-11-03):** the IC field (D-23-20); the drone phase is no longer S.4b's gate.
- **Alternative:** wait for a drone phase as §9's S.4 bullet and D-UM-47 planned.
- **Why:** doc 20 §17 is reserved and unbuilt; the field is built, reviewed and running stories at
  two sizes.

**Q-23-19 — Org-held licences.**
- **Decision (accepted default, 2026-11-03):** S.4b claims the operator's own filed licence; the `org=` claim waits for the
  Roll's org admission path.
- **Alternative:** both in S.4b.
- **Why:** the reader's request is `own(unit, licence)` and an org path is a Roll change with its
  own review.

**Q-23-20 — S.4b's acceptance.**
- **Decision (accepted default, 2026-11-03):** one story at 256 and 512 with the bot typing the player's half (doc 38
  Q-38-29).
- **Alternative:** one size, as GC's stories ran.
- **Why:** Q-38-29 is accepted and the field story already runs at both sizes.
