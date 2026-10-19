# 18 — The trade run: the first IC vertical slice

**Draft 0.4 — 2026-10-17, "as built" through IC12.** The slice's cuts have landed on `main`: IC0
`c87d78f7`, IC1 `07232d09`, IC2 `484647bc`, IC3 `ea52634d`, IC4 `3c9cded5`, IC5 `10b89d58`, IC6
`3bf2bc90`, IC7 `d8427081`, IC8 `ee1d831f`, IC9 `1137c373`, IC10 `7435f954`, IC11 `26e3b1fa`, and IC12
with this revision (the story test on the production root, the full readiness gate, the help topics,
the station packs Ceres v2 / Stickney v3, and this docs pass). IC13 (buyout, graduation, dormancy) and
IC14 (the residual sweep) are open. The record of each cut — what landed, what it deferred and why, the
defaults it took for the owner — is [18-cut-notes.md](18-cut-notes.md), and Draft 0.4 folds it in as
**"As built (IC*n*):"** notes beside the text they amend: every Draft 0.3 sentence is kept, a Draft 0.3
number the tree contradicts is left standing with the as-built number in its note, and a "verbatim" text
a cut changed is quoted by its shipped constant. As-built cites are file:line at `26e3b1fa` plus IC12's
working tree; every older cite keeps the commit it was pinned at (below). The owner's rulings (R1–R9)
and the P-A brief (§4.8) are untouched; the defaults the cuts took are re-listed in §8 as questions the
owner may override (Q-18-42 on).

**Draft 0.3 (history).** **Draft 0.3 — 2026-10-15**, revised in place at `main` `1fc40d01` (Draft 0.2's commit; IC0 landed at
its parent `c87d78f7`, Draft 0.1 at `8a32734a`). Only documents changed between `c87d78f7` and
`1fc40d01`, so every file:line below holds at `1fc40d01`: the A–I cites at `8a32734a`, the six files
IC0 moved (`mush-market` `trade.rs` +28, `lot.rs` +78, `commands.rs` +132, `module.rs` +34, `api.rs` +16;
`mush-sim` `commands.rs` +22) re-pinned at `c87d78f7` where marked, and the sky cites of §4.8 taken
fresh. Draft 0.1's nine-beat map (§3 A–I) and Draft 0.2's seven-beat map (§3 J–P), their designs and
reviews stand; this draft adds two beats (§3 Q the sky, R volume) condensed from a third verified map,
two designs with their judges (the sky seam; volume beside mass), and folds the owner's **second-round
rulings of 2026-10-15 (R7–R9, §1)** over Draft 0.2. Decisions are §7, D-18-1 to D-18-38 re-stated with
the replaced ones marked (D-18-28 replaced by R7), every accepted default now **binding (owner,
2026-10-15)**, plus D-18-39 to D-18-51; questions are §8, every Draft 0.2 question closed in one line,
Q-18-21 reversed, and only what the two designs could not settle left open (Q-18-32 to Q-18-41). The
companion content brief is [18a-trade-run-stations-and-ship.md](18a-trade-run-stations-and-ship.md),
revised with this draft. **IC0 has landed** as `c87d78f7`; its record is [18-cut-notes.md](18-cut-notes.md),
written by the IC0 session, which never writes this document. The IC5 proposal is pending.

**The second-round rulings (2026-10-15), in the owner's words.** *"Q-18-21 .. both volume and mass are
critical components of capacity .. that's part of the trick is balancing the payload from station to
station to utilize both the available volume and mass .. other defaults are accepted. .. i had said that
i wanted to lock in the geometry of the solar system for constant flight paths, but after watching
belter-charts i want to change my mind because the map is fascinating, and it adds a whole extra degree
of strategy to the game .. a trading path that was optimal last month might be less optimal as Pallas
gets further from the ecliptic plane .. and Pallas might be willing to pay more money for emergency
supplies when they're in an awkward position relative to the rest of the inners .. we can flag this for
implementation after the current slice, but i want to make sure we dont make any decisions during this
slice that we'll regret when we implement the astronomy engine."* Read as three rulings: **R7** volume
capacity is in the slice and the hold binds on tonnes and cubic metres, so a captain balances each leg's
payload to fill both (§4.3; D-18-28 replaced); **R8** every other Draft 0.2 default is accepted and
binding (§7, §8); **R9** the sky will move after this slice — a real, three-dimensional ephemeris on the
IC calendar, route optimality that drifts over months, prices that read position — and the slice must
make no decision the astronomy engine would undo (§4.8; post-slice cut **P-A**; doc 10 §22 T17 amended).

**What the owner ruled (2026-10-15).** Start on IC0 while the details settle. **R1** The pilot charters;
a required purchase is refused; owning a hull is an aspiration "a couple of weeks of good luck away", so
a buyout exists in the design and the start does not make it trivial. **R2** No one-time advance and no
revolving line: a corporation invests in new pilots up front on a profit-sharing basis because it wants
pilots who can manage runs for it (the default lay of §4.1 shares **trading margin** over posted
reference prices, not net profit — fuel, stores and port costs stay the pilot's — which departs from the
word "profit"; Q-18-13, closed under R8: the net share with a run book is P9). **R3** A passage under a day and around a day; travel is primarily
offline; a pilot who logs in 30–60 minutes at about the same hour each day runs a productive career by
queuing orders for the time they are away (departure times, loading, the captain's minutiae) and logging
off. **R4** Seed an initial finite world stock; nothing from thin air. **R5** A five- or six-room ship —
an airlock on a central passage with the bridge, a crew area and the cargo hold off it — because the
pilot may be aboard for long stretches. **R6** Flesh out the two stations and the flavour of each. The
ports (Q-18-1) stand. Where a ruling overrides a Draft 0.1 default the text says so and §7 marks the
decision "replaced by R*n*". **Second round (2026-10-15; the words are in the header):** **R7** volume
beside mass (§4.3; D-18-28 replaced by D-18-39/40/41); **R8** every other Draft 0.2 default accepted —
Q-18-3, 5, 6, 7, 10, 11, 12, 13–20 and 22–31 "as recommended", their decisions binding; **R9** the
frozen sky superseded *after* the slice by an astronomy engine, the slice flying a constant sky but
making no decision the engine would undo (§4.8; D-18-42..51; cut P-A).

Reading order for someone new: §0 (the run in one paragraph), §2 (two days, beat by beat), §4.1–§4.4
(the backer, the ship, the goods, the captain's day), §5 (the cuts), §8 (the questions).

**The slice in one paragraph.** A new account connects, makes a pilot, and walks out of Limbo through
the `Ceres Freeport <C>` door into Port Watch Arrivals. At the Registry & Co-op desk they sign the Belt
Carriers' Cooperative's First Captain programme: the Co-op charters them a Co-op Dray, the *Salt Moth*,
stakes them 25,000 cr of working cash, and takes a quarter of the trading margin on every sale until the
pilot buys the hull out at a price fixed the day they sign. At the Exchange Hall they buy 20 t of metal
concentrate and 80 t of water ice into the Moth's hold — the concentrate weighs the hold out, the ice
cubes it out, and that load fills both the 100 t and the 120 m³ — and at the chandlery, 42 t of reaction
mass and 21 crew-days of provisions. They board through the Visiting Berth Gallery, walk the six rooms,
check the plan on the Bridge (payload, burn, ETA about twenty hours at this season's constant sky), queue
the day's orders — depart now; at Stickney sell everything, fill the tank and the locker, buy crated yard
fittings and alloy stock with what is left, and leave again at 19:30 tomorrow — and log off in a bunk. The ship flies while they sleep: the burn settles, the Moth
casts off, passes Kirkwood, docks itself at Stickney Yards, and the queue sells to Kasei Metals, buys,
and waits for its hour. The next evening the pilot logs in docked at Stickney, reads what the queue did
and what the Co-op took, walks the Yard, re-files the Ceres orders and logs off again. Two weeks of good
runs buys the hull. Every act is the player's own verb, or the queue executing the pilot's own signed
orders; no staff command and no NPC stands on the path.

**As built (IC0–IC12):** the loop runs as written, at the play rate of 24 IC seconds per real
second (doc 18 §4.4, R3; the production readiness report refuses enrolment at any other rate,
`tacsim/src/port/readiness.rs:37`). On lane v3 at the constant sky (rate 0, epoch 2351-05-13) the
doc's full outbound load flies 959,361 + 735,600 IC s, about 19.6 h real, and the full return 653,642 +
852,473 IC s, about 17.4 h (IC9 L1, the laden step FLOOR); the two legs' timings move with the load and,
after P-A, the season. The money renders `25000.00cr` (IC0's render, IC6 (5)). The queue's return
depart in the story test is `at=+10m` (the shipped time grammar has no seconds form); `at=daily 19:30`
is the shipped form of this paragraph's "19:30 tomorrow". Buyout (IC13) is not built.

---

## 1. What the owner asked, read against the setting

The ask (2026-10-15, verbatim): *"build out an IC world to interact with, a number of real IC maps to tie
things together .. a vertical slice that allows a player to acquire a new ship, get an initial credit
line, purchase tradeable goods, along with consumable resources like food and fuel, then fly their ship
to another port, dock, and sell those goods. Maybe Tycho to Pallas .. mush-room maps for both stations,
laid out like a station would be .. stat out the starter ship .. add a small set of the resources we
identified previously."*

**The corridor as seeded.** Doc 11 builds exactly one region deeply: the Mars–Phobos–Ceres corridor.
Ceres Freeport holds the market, the Registry, Ceres Clearing (the Belt's bank, 11 §4.8) and Occator
Water & Propellant; Kirkwood Depot is the Co-op's transfer node with Republic customs pickets from 2347
(11 §2); Phobos holds Stickney Yards and Kasei Metals' one refinery, which turns volatiles into
propellant and metals into alloys (11 §4.3; §3.2 only names "its Phobos refinery"). Ceres exports water
and propellant; Phobos needs volatiles.
That flow is the corridor's reason to exist and the pair the slice's constant-sky epoch is chosen for
(10 §22 T17, as amended by R9: honoured for the slice, superseded after it by cut P-A). The
ephemeris already has bodies `ceres`, `mars` and `earth` as pure data
(`crates/tacsim/src/fixture/tacsim_corridor_v2.json`), and the money is the credit, cleared through Luna.

**What "Tycho" and "Pallas" are in the setting.** Tycho is **Tycho Mutual Assurance & Salvage**, a Luna
insurer with agents at Ceres, Kirkwood, Phobos and Isidis that owns the wrecks it pays out on and runs
the Re-float Program, leasing a Dray or a Cairn under `Encumbrance::Lease` (11 §4.7). It is an
institution, not a place. A Tycho port on Luna is unplaceable today: the ephemeris is heliocentric,
circular and coplanar with no parent bodies (`crates/tacsim/src/ephemeris.rs:1-10,44-52`), so a Luna
port sits on Earth's point, and doc 11 §5 keeps Earth/Luna "strategic map only in E7". **Pallas** is a
one-line off-corridor stub, the Pallas Syndicate (11 §4.10), with no body in any pack and no relay node
under doc 15's recommended topology (`15-comms-relay-backbone.md:319-322` is an open question whose
option A reads "none at Venus, Titan, Pallas or Vesta", so "months away" is a recommendation, not a
settled fact).

**Recommended ports.** Ceres Freeport (start) to Stickney Yards on Phobos (sale), by Kirkwood Depot as an
intermediate leg. It is the only pair with institutions, a trade reason, an ephemeris and fixture places
to promote, and the content built for it is what the faction, front and campaign cuts will consume
(11 §10). All three designs and all three judges converged on it. **The owner confirmed the pair on
2026-10-15** (Q-18-1 closed); the pasted proposal uses it.

**Disposition of the two names (Q-18-1).** Tycho is honoured as an agency in both station maps (the
insurer and the hull-registry board that makes ownership visible); under R1/R2 the starter hull comes
from the Co-op's programme pool, not Tycho's loaner pool (Draft 0.1's reading). Pallas is honoured as
a thread: a `pallas` body entry (orbit 2.773 AU) so distances exist, a Syndicate agent at the Ceres
exchange speaking the outer-claim lore 11 §5 already seeds, and "Ceres–Pallas" named as the first
off-corridor extension. Swapping port B to Pallas later costs one location, one station pack and one
route pair, all data.

---

## 2. The run, beat by beat: two days

Verbs whose status is **new** do not exist today; their names are binding under R8 (Q-18-12, "names ok"). Evidence is
file:line at `c87d78f7` (HEAD, IC0 landed) for the six files IC0 moved and at `8a32734a` everywhere
else (identical there). Every typed exit word below is an alias 18a authors on that room's exits
(exit matching is exact over `;`-segments and aliases, `mush-core/src/commands/movement.rs:24-28`), and
IC12's story test walks these paths against the packs. "cr" is `tacsim.credit`, scale 2; IC0's one render prints `200.00cr/t` (the symbol flush
against the digits, [18-cut-notes.md](18-cut-notes.md)) and this document writes it that way. Prices
and quantities are per tonne (D-18-13). Day 1 is the live set-up and the first departure with the queue
filed; day 2 is the offline turnaround at Stickney and the pilot's second session.

**Day 1 — the pilot is online for about forty minutes.**

| # | Step | The player types | The player sees | Mechanism | Status | Evidence |
|---|---|---|---|---|---|---|
| 1 | Log in, make a pilot | `ssh play@host` · `create Vex <pw>` | Limbo with `OOC Area <O>`, `Staff <S>`, `Ceres Freeport <C>` | Gateway creates account + character in Limbo; chargen gate inert on dev1 | exists | `mush-gateway/src/accounts.rs:135-148`; `mush-server/src/world_task.rs:9228` |
| 2 | The IC door | `c` | Port Watch Arrivals, Ceres Freeport: a 2350 desc, the customs counter, notices, the stair down | `exit.limbo_arrivals` in `neighborhood.ceres_freeport`, **unlocked**: tacsim's `OnboardingGate` already refuses every exit line for an unapproved character, so no approval marker is needed; the IC-return hook (§4.5) passes a character with no recorded IC position | new | `tacsim/src/sheet/onboarding.rs:97-112`; `mush-core/src/commands/movement.rs:24-42` (Limbo exits are global) |
| 3 | Read the programme | `registry` · `+coop` | The lay board: stake 25,000.00cr; 250‰ of margin over the Co-op's reference prices; buyout 90,000.00cr fixed at signing; 8 of 8 Drays free (a fresh world: the applier lays up every hull with no lease); one enrolment per account | Reads the port entry's `programme` block and the live leases (the `next_loaner` shape) | new | `mush-campaign/src/recovery.rs:935-948` |
| 4 | Sign | `+coop/enrol Salt Moth` | "Ceres Clearing opens account #41 in your name. The Co-op charters you FPC-D-0401, laid up as the *Slack Water*, now the *Salt Moth*, cradled at the Visiting Berth Gallery. 25,000.00cr staked to you. Buyout 90,000.00cr. Ilse Oduya-Brandt countersigns." | One command: `open_account` if absent; lowest-id Trust Dray with no live lease (`next_loaner`'s `min_by_key`, so the first pilot on a fresh world gets 0401); the pilot's name replaces the Trust's laid-up name; `offer_lease` + in-verb `accept_contract` (the market's offer-plus-accept precedent); the stake `post` as `Organization(coop_trust)` keyed by the lease; `create_vault(SpatialInterior(stowage))` owned by the pilot; the enrolment document; refused on a second enrolment from the same gateway account | new | `mush-economy/src/contract_live.rs:4516` offer_lease; `mush-market/src/trade.rs:647-666`; `mush-campaign/src/recoverycmd.rs:621-661,701`; `mush-campaign/src/seatstate.rs:142-146` |
| 5 | Check the ship | `+ship` | "*Salt Moth* (Dray): docked Ceres Freeport, Berth Gallery. Hold: cargo 0/100 t · 0.0/120 m³ · tank 0/80 t · locker 0/120 crew-days. Programme: 0 sales; buyout 90,000.00cr." | Reads the enrolment, the live lease, `pose_of`/JourneyBook, the hold pools through `scan_action_state` | new | `mush-sim/src/window.rs:362`; `mush-core/src/module.rs:561,573` |
| 6 | Buy cargo | `arrivals` · `concourse` · `exchange` · `+market/list` · `+market/quote 4=20` · `+market/accept 11` · `+market/quote 5=80` · `+market/accept 12` | "Metal concentrate 200.00cr/t, 4,000 t · Water ice 100.00cr/t, 9,600 t · WANTED structural alloy 520.00cr/t · WANTED yard fittings 560.00cr/t"; "20 t of metal concentrate are loaded into the *Salt Moth*'s hold. Margit Søndergaard-Achebe countersigns."; "80 t of water ice … Rosalind Qureshi-Mbeki countersigns." Hold: cargo 100/100 t · 120.0/120 m³ (both full) | The lot path with the port seams: `market=` and `to=` default to this venue and the berthed hold (VenueGate `Presence::Here`); the ask is `Supply::Stock + Representation::Lot` whose matter leg is `LotOp::Draw`, a pool-to-pool `transfer` staged at activation, with the listing quantity as the synchronous availability counter (no reservation, no hold); the vendor org accepts through the content-resolved party-cap source; capacity (mass **and** packed volume, R7) pre-checked synchronously and again in the Draw prepare; an 81st tonne of ice would be refused on volume with nothing charged | partial | `mush-market/src/trade.rs:573` (`booked_loss` pre-check), `:701-716` (the lot leg staged at activation, `LotOp::Consume` at `:706`), at `c87d78f7`; `mush-inventory/src/pool.rs:413-528`; `mush-economy/src/contract_live.rs:5151-5175`; `tacsim/src/composition/mod.rs:361` |
| 7 | Fuel and stores | `chandlery` · `+market/quote 6=42` · `+market/accept 13` · `+market/quote 7=21` · `+market/accept 14` · `+ship/hold` | "42 t of reaction mass are bunkered." "21 crew-days into the stores locker." Hold: cargo 100/100 t · 120.0/120 m³ · tank 42/80 t · locker 21/120. `+credit`: available 11,110.00cr | Same path; the goods table's stowage tag routes each good to its pool (`stow.bulk`, `stow.propellant`, `stow.provisions`); ice into the tank is refused | partial | `crates/tacsim/content/goods.json` (new, §4.3) |
| 8 | Board and walk the ship | `exchange` · `concourse` · `arrivals` · `lobby` · `gallery` · `+ship/board` · `in` · `up` · `look` · `down` · `crew` | "You cycle the lock into the *Salt Moth*." Airlock; Central Passage; Bridge (course, tank and hold readouts, the status line "Aboard the *Salt Moth*, docked at Ceres Freeport"); Crew Compartment | `+ship/board` in the berth gallery places the lease tenant `Interior{airlock space, berths}`; the member-room boundary rule lets an Interior actor take an exit whose destination is another member of its own space, with no placement write per step; projection keeps the member location across a restart | new | `mush-sim/src/window.rs:324-341`; `lib.rs:293-303`; `graph.rs:167`; `data.rs:832-835` |
| 9 | Plan | `+ship/plan stickney` | "Cargo 100 t of 100 t (mass full) · 120.0 m³ of 120 m³ (volume full). Payload 142.2 t of 181 t. Burn 41 t (tank 42 t; 1 t left). Stores 21 crew-days. Passage 19.6 IC days ≈ 19 h 38 m real; arrives about 15:08 UTC tomorrow. Nothing was written." | One pure tacsim `plan_passage` over the sky at departure, the route's legs as body pairs, the Dray's drive law and the hold pools' mass (§4.4, §4.8); the capacity function for the two slacks; the `@journey/plan` write-nothing precedent | new | `mush-sim/src/route.rs:794-836` (the laden arithmetic it calls); `mush-sim/src/commands.rs:2605-2614` (`c87d78f7`) |
| 10 | Queue the orders | `+ship/order depart stickney at=now` · `+ship/order sell metals all min=250` · `+ship/order sell volatiles all min=150` · `+ship/order buy propellant fill max=60` · `+ship/order buy provisions fill` · `+ship/order buy industrial_parts 49 max=400` · `+ship/order buy structural_alloys 55 max=400` · `+ship/order depart ceres at=daily 19:30` · `+ship/orders` | Eight numbered orders with states and due times in UTC, IC and real time; "Order 1 fires on the next tick." The bulky good is ordered first because cash will bind at Stickney (§4.3, "why you carry both") | The `tacsim:orders:<hull>` document, written in the command's batch; a closed vocabulary (buy, sell, depart, stop); fuel and stores are queued **before** cargo so the executor's clamp can reserve the next departure's needs (§4.4) | new | §4.4; `mush-propagation/src/commands.rs:1252-1279` (`+orders` is taken, hence `+ship/order`) |
| 11 | Log off aboard | `QUIT` from the Crew Compartment | (in every room, a moment later) "The *Salt Moth*'s lock seals. Berth clamps release; the drive lights." | The disconnected body stays placed aboard (only seat focus lapses, and there is no seat); the tacsim tick's order arm runs the depart step **as the pilot**: lease re-check, undock checks, `plan_passage` at this instant, then one authoritative `tacsim.port.burn` keyed `tacsim.port.burn/<hull>/1/1` (debits the propellant and provisions pools, writes the ticket with the manifest and the whole passage plan: per-leg distance and seconds, burn, crew-days, the sky revision); the next fire launches it through `pub mush_sim::depart` with the plan's frozen `leg_s`; narration loops over `space.rooms` | new | `mush-sim/src/seat.rs:736-770`; `mush-npc/src/module.rs:1496-1502` (actor swap); `mush-sim/src/journey.rs:11-35,398-408` |

**As built, day 1** (notes keyed by step; the table is Draft 0.3's):

- **Step 3 (IC6).** `+coop` prints three lines: the registrar's line (`REGISTRAR_LINE`,
  `tacsim/src/port/coop.rs:84`), the terms (`Terms: a 25000.00cr stake; 250‰ of trading margin over the
  Co-op's reference prices; buyout 90000.00cr, fixed at signing (a new Dray from the Yard is
  120000.00cr); one enrolment per pilot, ever.`) and `Drays free: 8 of 8.` (`coop.rs:745-754`). Money
  renders without grouping (IC6 (5); IC11 L22).
- **Step 4 (IC6).** The enrol reply is three lines (`coop.rs:1315-1336`): "Ceres Clearing opens account
  #*n* in your name." (only when the account is new), "The Co-op charters you FPC-D-0401, laid up as the
  Slack Water, now the Salt Moth, cradled at the Visiting Berth Gallery.", "25000.00cr staked to you.
  Buyout 90000.00cr. Ilse Oduya-Brandt countersigns."
- **Step 5 (IC6 L19).** `+ship`'s first line is `SHIP_DOCKED` (`tacsim/src/port/ship.rs:104`),
  "{name} ({class}): docked {port}, {gallery}." with the gallery's full name, and the hold and the
  programme follow on lines of their own (three lines in all).
- **Step 6 (IC7, IC8 L17).** The accept answers the market's generic line (it begins "Agreed."); the
  boards name each line's signatory (`tacsim/src/port/board.rs`), but an accept line naming the
  listing's signatory is not built (unowned; §5's IC12 list, §8). A quantity that will not fit is
  refused by `capacity_line` (`tacsim/src/port/market.rs:498`) before anything is charged.
- **Step 8 (IC6 L18; IC9).** The status line carries the registry mark: `STATUS_DOCKED`,
  "Aboard the {name} ({mark}), docked at {port}." (`ship.rs:85`), in every hull room; its other forms
  are `STATUS_UNDER_WAY`, `STATUS_CLEARED`, `STATUS_DOCKED_DEPARTS` and `STATUS_ELSEWHERE`
  (`ship.rs:90-100`). The story test asserts the Bridge render
  (`mush-server/tests/ic_slice_loop.rs::ic12_part_one_a_leg_by_hand`).
- **Step 9 (IC9).** `+ship/plan` (`tacsim/src/port/voyage.rs:1087`) prints, at the vertex with the tank
  bunkered to the planner's own burn: "Plan for Stickney Yards, by way of Kirkwood Depot:", the cargo
  line with both limits ("FULL: mass and packed volume"), the room by mass and volume per good on the
  board, "Payload 140.2 t of 181 t.", "Burn 40 t (tank 40 t; 0 t left). Every tonne aboard, reserve
  included, costs 105 kg of burn.", "Stores 21 crew-days (locker 21; 0 left).", the passage in IC days
  with its real time and UTC arrival, "Nothing was written." With the doc's 42 t aboard the burn is
  41 t (lane v3; IC9 (7)).
- **Step 10 (IC11; IC12 L12).** The grammar is `+ship/order depart <port> [at=now|HH:MM|daily
  HH:MM|+Nh|+Nm] [until=+Nh|+Nm]` (`tacsim/src/port/when.rs:13-15`; `SHIP_HELP` and `ORDER_USAGE`
  corrected to name `+Nm` in IC12, `ship.rs:62`, `orders.rs:715`); `at=daily 19:30` parses as written.
- **Step 11 (IC9, IC11 L7).** The queued depart's burn is keyed `tacsim.port.burn/<hull>/<order>/<attempt>`
  (here `/1/1`), the live undock's `tacsim.port.burn/<hull>/undock/<seq>` (`tacsim/src/port/burn.rs:52-58`);
  the launch is the tick calling `mush_sim::depart` with the plan's frozen leg seconds
  (`mush-sim/src/commands.rs:2793`), no submission.

**Day 1/2 — offline, about twenty hours.** The `CarrierJourney` advances on the world IC clock; at the
Kirkwood boundary every member room hears "Kirkwood Depot slides past: the Co-op's tank farm, a Republic
picket's transponder asking for your manifest. Leg 2 of 2." and a notice is queued (`journey.rs:1088-1172`;
today's narration reaches no one aboard, `journey.rs:1396-1409`, partial). Nothing can cross the hull's
boundary: the carrier seal refuses placements, the boundary rule refuses every non-member destination
(the global `o` and `s` exits included), and `+ship/leave` refuses under way.

**Day 2 — the turnaround runs offline at about 15:08 UTC; the pilot logs in at 19:00.**

| # | Step | The player types | The player sees | Mechanism | Status | Evidence |
|---|---|---|---|---|---|---|
| 12 | Arrival and docking | (nothing) | (as a notice) "15:08 UTC *Salt Moth* docked at Stickney Yards, Visiting Berth Gallery." | `apply_arrival` places the hull Direct on Stickney's passive domain, presented into P01; docking is automatic (D-18-18 kept): every hull at a port shares the gallery and is listed there | partial | `mush-sim/src/commands.rs:2207-2256`; `journey.rs:1340-1420`; `window.rs:195` |
| 13 | The queue sells | (nothing) | (notices) "Sold 20 t metal concentrate to Kasei Metals at 280.00cr/t: 5,600.00cr; Co-op share 400.00cr; you received 5,200.00cr. Jonas Achterberg-Nkemelu countersigns." "Sold 80 t water ice at 200.00cr/t: 16,000.00cr; Co-op share 2,000.00cr; you received 14,000.00cr." | The order arm, as the pilot under VenueGate `Presence::Ship(hull)`: the `+market/sell` lot arm against Kasei's `Supply::Wanted` bid; the buyer org's consideration escrowed by `accept_contract_as`; payouts `Exact(share)` to the Trust and `Exact(rest)` to the pilot from the `ProceedsPolicy`, gated on the inventory `transferred` Fact so money never moves before the matter; `LotOp::Deliver` = `transfer(hold pool → Kasei pool)`; one step per fire, verified by pull before it is marked done | new | `mush-economy/src/contract.rs:1886-1909,1418-1458`; `mush-inventory/src/facts.rs:44-82`; `mush-market/src/trade.rs:1023-1036` (one payout today) |
| 14 | The queue buys | (nothing) | (notices) "Bought 36 t reaction mass at 60.00cr/t (2,160.00cr): tank 37/80 t, the burn for the 73 t your orders can load." "Bought 19 crew-days (190.00cr)." "Bought 49 t yard fittings, crated at 400.00cr/t (19,600.00cr)." "Bought 20 t structural alloy stock at 400.00cr/t (8,000.00cr); short of 55 t: funds." "Departure for Ceres Freeport set for 19:30 UTC." | `fill` projects the heaviest load the queued buys can make under both hold limits (49 t fittings, then alloys clamped to 24 t by volume: 73 t), plans the next depart's passage at its hour and tops the tank to that burn (37 t) and the locker to its crew-days (8 + 11 = 19 by the per-leg rule, §4.2); each cargo buy is then clamped to min(qty, listing, room by mass, room by volume, funds) **after** the departure's needs are met, logging which bound bit; `quote` then `accept_into` as the pilot, clamped before quoting because `quote` refuses a quantity above the listing | new | `mush-market/src/trade.rs:246` (`quantity > listing.quantity`), `:515` (`accept_into`), at `c87d78f7` |
| 15 | Log in | (connect) | "While you were off watch:" then the lines above, once; "Aboard the *Salt Moth*, docked at Stickney Yards; departs 19:30 UTC (in 31 m)." | tacsim reconnect notices on the housing pattern, rendered once on `Connected`; the look status line on the atlas `look_rendered` precedent | new | `mush-housing/src/notice.rs:1-20`; `mush-atlas/src/travel.rs:2673-2695` |
| 16 | Ashore at the Yard | `passage` · `airlock` · `+ship/leave` · `lobby` · `customs` · `concourse` · `+coop/statement` · `observation` | The gallery; Republic forms; the statement (readable anywhere, §4.1): 2 sales, margin 9,600.00, shares 2,400.00, cash 360.00 after the fittings and the alloys, buyout progress; the Guard's pad under Mars | `+ship/leave` in the airlock, docked: `unplace(keep)` + `move_to(gallery)`; `tacsim.last_port` stamped for the IC-return rule; the statement reads the enrolment and share records | new | `mush-sim/src/window.rs:346-358`; `tacsim/src/match_/cockpit.rs:2401-2418` put_out shape |
| 17 | Re-file and log off | `concourse` · `customs` · `lobby` · `gallery` · `+ship/board` · `+ship/order sell industrial_parts all min=500` · `+ship/order sell structural_alloys all min=480` · `+ship/order buy propellant fill max=40` · `+ship/order buy provisions fill` · `+ship/order buy volatiles 80 max=100` · `+ship/order buy metals 20 max=200` · `+ship/order depart stickney at=now` · `QUIT` | The Ceres orders queued behind the pending 19:30 depart; at 19:30 the lock seals and the return leg (17 h 27 m at 69 t: 1,507,276 IC s) lands about 12:57 UTC the next day | The same queue; `rev` bumps on every edit and the inflight step is untouchable | new | §4.4 |
| 18 | Day 3 and after | (nothing) · (connect at 19:00) | "Sold 49 t yard fittings, crated to the Belt Carriers' Cooperative at 560.00cr/t: 27,440.00cr; Co-op share 1,960.00cr; you received 25,480.00cr." "Sold 20 t structural alloy stock to Freeport Works at 520.00cr/t: 10,400.00cr; Co-op share 600.00cr; you received 9,800.00cr." Then the outbound bought; cash 21,840.00cr; the cycle repeats one leg per day | The Co-op's fittings bid and Freeport Works' alloy bid at Ceres are the return leg's buyers; from here a 30–60 minute session a day is the career; with full funds the best return is 55 t alloys + 45 t fittings (100 t, 120.0 m³) | new | — |
| 19 | Week two: buyout | `registry` · `+coop/buyout` | "The Registry writes your name over the Co-op's mark. The *Salt Moth* is yours: 90,000.00cr paid; the charter ends; no further share." | Pilot-issued asset contract (`TitleHeld` + `TitleTransfer`, the market's shape); the Trust accepts by party cap; `end_lease` before settlement so invariant 12 holds; the observer marks the enrolment graduated | new (IC13) | `mush-market/src/trade.rs:829-848`; `mush-economy/src/contract_live.rs:4710`; `engine.rs:1121-1125` |

**As built, day 2:**

- **Step 12 (IC9).** Docking is automatic; every member room hears tacsim's flavoured arrival line
  ("Salt Moth docks at Stickney Yards.") beside mush-sim's generic one (`REPLACE_GENERIC_ABOARD_LINES =
  false`, `tacsim/src/composition/sim_routes.rs:47`; IC9 (1)).
- **Step 13 (IC10).** The sale line is `SALE_LINE` (`tacsim/src/port/proceeds.rs:53`): "Sold 20 t of metal
  concentrate to Kasei Metals at 280.00cr/t: 5600.00cr; Co-op share 400.00cr; you receive 5200.00cr when
  the goods are in. Jonas Achterberg-Nkemelu countersigns." — "receive … when the goods are in", not
  "received": the money moves only once the inventory fact is true.
- **Step 14 (IC11).** The buy lines are `BOUGHT`, `BOUGHT_SHORT`, `BOUGHT_NONE`, `BOUGHT_FILL_TANK` ("Bought
  {qty} t {label} at {price}/t ({total}): tank {after}/{limit} t, the burn for the {cargo} t your orders
  can load."), `BOUGHT_FILL_LOCKER` and `DEPART_SET` (`tacsim/src/port/executor.rs:2909-2925`);
  a bound that bit is named in log order ("short of 55 t: funds").
- **Step 15 (IC11).** The reconnect heading is `NOTICE_HEADING`, "While you were off watch:"
  (`tacsim/src/port_tick.rs:100`), each line once; the status line is `STATUS_DOCKED_DEPARTS`
  ("…; departs {hhmm} UTC (in {span}).", `ship.rs:97`).
- **Step 16 (IC6 (1)).** `+ship/leave` is gated on placement alone (aboard, in the Airlock, docked), so a
  pilot whose charter ended aboard can always step ashore; `+ship/board` is gated on the lease and the
  placement (§7 D-18-34).
- **Step 17.** The story test's part 2 re-files nothing after the return (its queue ends at Ceres); the
  Ceres orders of step 17 are filed by hand in a session, as written.
- **Step 19.** IC13; not landed.

**Money check** (every figure reproduced by the volume design's judge; §4.3 carries the arithmetic).
Stake 25,000 − 4,000 (20 t at 200) − 8,000 (80 t at 100) − 1,680 (42 t at 40) − 210 (21 crew-days at 10)
= 11,110; hold 100 t / 120.0 m³, both limits full. Stickney: + 5,200 (5,600 less a 400 share on a 1,600
margin) + 14,000 (16,000 less 2,000 on 8,000) = 30,310; − 2,160 (36 t at 60: `fill` tops the 1 t left to
37, the burn for the 73 t the queued buys can load) − 190 (19 crew-days) = 27,960; − 19,600 (49 t
fittings at 400) − 8,000 (20 t alloys; the clamp's funds term gives 20, so "short of 55 t: funds") =
360. Hold 69 t / 118.7 m³; return payload 106,176 kg, burn 37 t (tank 0), crew-days 8 + 10 = 18 (1
left), 17 h 27 m. Ceres: + 25,480 (27,440 less 1,960 on 7,840) + 9,800 (10,400 less 600 on 2,400) =
35,640; − 1,600 (40 t at 40) − 200 (20 crew-days: 1 → 21) − 8,000 − 4,000 = 21,840. Steady state, both
hulls' limits full each way: an outbound nets about 5,390 and a return (55 t alloys + 45 t fittings)
about 7,740, 13,130 a round trip; the first return is cash-bound to 69 t and the second to 96 t (45 + 51).
At one leg a day the 90,000 buyout is crossed on day 13 and the buyout plus a 20,000 float lands on
**day 15** at full loads (about 4,000 to spare) and day 23 at 70 % loads (§4.1). Fuel is a decision: a
Ceres top-up to 80 t costs burn (105 kg per tonne carried) and still cannot cover both legs with a full
hold (out 45 t at a brimmed tank, back 37–40 t), so Stickney's 60 cr/t reaction mass is bought every
return. The timings are the slice's **constant sky** at its epoch (§4.8), never an invariant.

**As built (IC9, IC11 L23):** the figures hold at lane v3. The outbound's legs are 959,361 + 735,600
IC s with the burn 41 t and 21 crew-days; the return's 653,642 + 852,473 IC s with the burn 37 t and
18 crew-days — the laden step is `route_duration`'s exact checked arithmetic with a FLOOR, not a
ceiling (IC9 L1; `mush-sim/src/route.rs:863` `laden_leg_durations`). IC11's lens reproduced the
Stickney figures — 36 t of reaction mass to a 37 t tank, 19 crew-days, a 73 t projected load, the
alloys cut to 20 t by "funds", and with full funds 24 t bound by "hold volume" — and §2 step 7's 42 t /
21 crew-days leaving the tank at 1 and the locker at 0. No figure is stored in code or test (D-18-39):
the story test reads every one from the boards and the planner.

---

## 3. What the tree has, beat by beat

Condensed from the verified map. Each beat lists what exists, what is partial, what is missing, and the
corrections the verifiers made to the first reading, so the owner can see where earlier docs or surveys
were wrong. "Code wins" throughout.

**As built (IC0–IC12):** every *Missing* below that the slice needed is now built; the record is
[18-cut-notes.md](18-cut-notes.md); the cites in this section stay at the commits named in the header
(Draft 0.1's `8a32734a`, IC0's `c87d78f7`, Draft 0.2's `1fc40d01`). Where each beat lives at HEAD:

- **A** (the IC door): IC1, `crates/tacsim/src/ic_return.rs`.
- **B** (the wallet): IC3/IC6, `crates/tacsim/src/port/credit.rs`.
- **C** (acquiring a ship): IC4/IC6, `crates/tacsim/src/port/coop.rs`.
- **D** (the Dray): IC2 (v2 IC8), `crates/tacsim/content/units/dray.json`.
- **E** (goods, market, chandlery): IC0/IC7/IC8/IC10, `crates/mush-market/src/trade.rs`.
- **F** (flying): IC9, `crates/tacsim/src/port/voyage.rs`.
- **G** (station maps): IC1 (Stickney v2 IC8; Ceres v2 / Stickney v3 IC12),
  `crates/tacsim/content/atlas/neighborhood.ceres_freeport.json`.
- **H** (the content pipeline): IC1–IC3/IC8, `crates/tacsim/src/port/apply.rs`.
- **I** (NPCs, providers, help): IC1 (the cast, in the station packs), IC12 (the help), `docs/help/ports.md`.
- **J** (the six-room interior): IC4 (board/leave IC6), `crates/mush-sim/src/graph.rs`.
- **K** (the queued orders): IC5 (the tick), IC11, `crates/tacsim/src/port/executor.rs`.
- **L** (the backer): IC6 (the share IC10; the buyout IC13, open), `crates/tacsim/src/port/proceeds.rs`.
- **M** (finite stock, asks, bids): IC7/IC8/IC10, `crates/tacsim/src/port/stock.rs`.
- **N** (mass, volume, payload-dependent passage): IC2/IC9, `crates/tacsim/src/port/passage.rs`.
- **O** (the stations): IC1, `crates/tacsim/content/atlas/neighborhood.stickney_yards.json`.
- **P** (the pasted proposal's audit): history; nothing to build.
- **Q** (the sky): IC2 (the moving-sky flag and page IC12), `crates/tacsim/src/sky.rs`.
- **R** (volume beside mass): IC2/IC8, `crates/tacsim/src/port/capacity.rs`.

### A. Where the player starts, and the IC door

- **Exists.** Account + character creation (`mush-gateway/src/accounts.rs:135-148`; cap 3,
  `mushd.toml.example:102`); every Player created in Limbo, owner self, home Limbo
  (`mush-server/src/world_task.rs:9228`, `mush-core/src/world.rs:1026-1029`); chargen three verbs, 6 rating
  points, `pilot-chargen` job, approval stamps and calls `on_approved` (`tacsim/src/sheet/chargen.rs:41-48,
  411-439`); the onboarding gate refuses non-allowlisted verbs and exit lines only while `chargen_required`
  (`onboarding.rs:39-50,107,205-243`); three starter packs compiled in, digest-pinned, 17 rooms
  (`mush-atlas/src/starter.rs:1-46`); a never-latched pack applies at ANY boot and a refused first apply fails
  the boot (`mush-content/src/module.rs:112-149`); cross-pack exits with `portal.at`
  (`mush-atlas/src/def.rs:611-682`, `apply.rs:1462-1467`); exit locks `flag^NAME` match markers
  (`mush-core/src/lock.rs:266-274`); npc entries with `interfaces` (`mush-npc/src/content.rs:77-124`); eight
  interface providers (atlas, travel, help, request, rules, weather, market, build).
- **Partial.** `[onboarding].start_room` is a raw `#N` (`mush-config/src/config.rs:403-412`); approval sets no
  home, no marker, no account, and moves no one with chargen off; `NeighborhoodDef.setting` is inert text
  (`record.rs:483`); a seed `location` room has no `RoomGeo`, so no atlas exit can reach it
  (`apply.rs:1494-1498`); `RoomDef` cannot declare a function (`def.rs:366-390`).
- **Missing.** Any IC place content; an IC/OOC marker anything reads; a player path to an account; clearing/
  registry/berth providers; a marker set at approval; a Pallas body.
- **Corrections.** (1) "No berth concept" was **refuted**: mush-sim has hangar/bay interiors (`sim.interior`
  spaces with slots), a carrier journey can arrive `Interior{space, slot}`, and `InteriorCustodyProvider`
  backs vaults with them (`mush-sim/src/data.rs:778-795`, `journey.rs:1196-1215,1355-1372`,
  `custody.rs:43-50`); what is missing is wiring. (2) A **registered** marker is self-settable by `@set` because
  players own themselves (`building.rs:456-470`, `world.rs:1022-1024`): an approval marker must stay
  unregistered. (3) The engine already honours a finite overdraft floor (`mush-economy/src/record.rs:251-263`)
  and `NegativePolicy` is read nowhere; only a caller is missing. (4) Game code already resolves rooms by pack
  entry at runtime: `tacsim/src/match_/centre.rs:116-135`. (5) The Re-float loaner is a player-initiated lease
  plus stipend that a zero-asset pilot qualifies for, disabled by default (`recovery.rs:830-878`). (6) Bug:
  `connection.lot_tour`'s driver names `npc.aaron_samuels`, the entry is `npc.aaron`
  (`region.limbo.json:179`, `ooc_area.json:813`). (7) Doc drift: `mushd.toml.example:413-419` still names
  a WoD wizard and a `wod-chargen` job; `mush-config/src/config.rs:394-401` still says "guided wizard" but
  already says `pilot-chargen`; doc 14 §2.6 says two packs and fourteen rooms against three and seventeen.
- **Draft 0.2.** "A marker set at approval" (Missing) and correction (2) are superseded by O: the IC door
  stays unlocked because the `OnboardingGate` already bars unapproved characters; P9 is withdrawn.

### B. Wallet and the initial credit line

- **Exists.** `tacsim.credit` create-only at boot (`tacsim/src/economy.rs:53-79`); per-account `CreditPolicy`
  enforced on posted and available balances (`engine.rs:630-642,668-690`); invariant 26: a hold needs
  non-negative available funds (`engine.rs:691-697`); market purchases escrow a hold
  (`contract_live.rs:1934-1947`); `open_account` public, no validation but zero balance (`api.rs:1036-1047`);
  `create_system_obligation` is the in-code faucet, no actor check, capped **per obligation** (the amount
  ceiling, `spec.amount > cap`, re-asked at creation and at each settlement; not an aggregate per period)
  (`api.rs:1725-1818`, `:1766-1773`, cap `config.rs:299-324`, 50,000 default `deploy/instance/mushd.toml.tmpl:162`); `create_obligation`
  lets the debtor take on a debt with `Ruleset` cause (`api.rs:1940-2035`); declared cadences exist as a
  mechanism (`config.rs:124-141`); `@econ/pay` from an own account, `@account/show|list` (posted only).
- **Partial.** No verb builds a finite floor; the contract route mis-records security as a Lease
  (`contract.rs:2314-2329`); a lien is staff-only and an org-creditor lien can be released only by staff
  (`api.rs:422-440`); `Obligation` has no term field; recovery upkeep is RealUtc every 15 s, not the
  "campaign cadence" its header and 11 §4.7 claim (`recovery.rs:1049-1053`, `tick.rs:52`);
  `with_party_caps(Vec::new())` blocks every vendor's programmatic acceptance
  (`tacsim/src/composition/mod.rs:361`); all money verbs parse minor units.
- **Missing.** A pilot account opened by anything; a boot treasury; a data declaration of institutions; a
  loan seam with a term; `+wallet`; a default consequence.
- **Corrections.** (1) The whole feature was **built once and deleted**: the retired Shadowrun E6a layer
  (`56e877c4`) opened System faucets at boot with `open_account`, opened the character's account at approval
  with a one-shot attributed grant, ran upkeep on a declared cadence and shipped `+nuyen`. (2) A no-staff loan
  is code-reachable today: `post` admits an Organization actor on its own scoped account
  (`api.rs:370-375`; recovery's `post_stipend` does it, `recoverycmd.rs:621-661`), so the api.rs comment that
  `accept_contract_as` is the only producer of org actors is stale. (3) A finite term is a one-place engine
  change at the branch that already flips one-shots to Satisfied (`obligation.rs:660-667`). (4) No cadence is
  declared anywhere and `@obligation/create` cannot bind one (`commands.rs:4882-4886`). (5) A System-debit
  grant from the approval hook depends on the approver being economy staff (`mush-jobs/src/commands.rs:485-496`,
  `api.rs:345,360`). (6) mush-faction already creates an org with root scope and wallet from code
  (`mush-faction/src/econ.rs:391-447,851-900`). (7) Beat E's claim that `SystemOnly` stops a pilot account
  going negative is **wrong** by beat B's grep: `negative_policy` is never read; what stops the overdraft
  buying anything is invariant 26.
- **Draft 0.2.** The credit line this beat was mapped for is gone (R2; superseded by L): the account
  facts, the org-actor `post` precedent and the party-cap gap still carry the programme's stake and share.

### C. Acquiring a ship

- **Exists.** `EconomicAsset` title keyed by ObjId, separate from custody and operation
  (`record.rs:545-599`); `issue_title` under a registered issuer (`api.rs:2498-2580`; tacsim registers
  housing, market-if-on, industry, `composition/mod.rs:123-146`); `Encumbrance::Lease` via `offer_lease`
  after tenant accept (`contract_live.rs:4463-4516`); invariant 12 blocks title transfer while encumbered
  (`engine.rs:1121-1125`); the recovery provider end to end (`recovery.rs:162-198`,
  `recoverycmd.rs:545-608`); carrier root model (`journey.rs:1-9`, `transition.rs:975-1013`); stations with
  explicit channels and console reach (`data.rs:864-882`, `focus.rs:135-137`); departure by staff,
  `perm::controls`, or a `movement` billet (`commands.rs:2606-2652`); `TacticalUnit` generic, `create_unit`
  callable from appliers (`mush-tactics/src/module.rs:2715-2760`); weaponless units ship (`target_mk1`,
  `drone_mk1`); the arena applier creates, binds and places units at pack apply (`tacsim/src/content.rs:612-647`).
- **Partial.** No hull issuer; a market Asset sale moves title and delivers nothing (`mush-market/src/module.rs:384-387`);
  no catalog source in production (`composition/mod.rs:509-512`); the recovery lease grants paper, ignores tier,
  grants no billet (`recovery.rs:935-948`); atlas interiors are Unanchored only (`mush-atlas/src/apply.rs:1268-1296`);
  `maintenance_account` is read by nothing; `assignment` IS read as standing (`a.title_owner == me ||
  a.assignment…assigned_to == me`, `mush-economy/src/commands.rs:549`; rendered by `@asset/show`
  `:3744-3747`; compared at boot `boot.rs:1812-1813`), an existing operate-standing seam this slice weighs
  in §4.2 and does not use.
- **Missing.** Content that instantiates a ship; a title-to-operate bridge; a player embark verb; a hauler
  schema; drive-dependent journey time; multi-room interiors (reserved, `data.rs:832-835`); gangway exits.
- **Corrections.** (1) Every catalog unit is SIM UNIT-fieldable: `fieldable_ids` walks the whole catalog
  (`match_/lobby.rs:1600-1619`), so ship hulls need a lobby filter, not a different table (the "non-fielded
  table" idea was **refuted**: a unit outside the catalog cannot be made by `@unit/create` either). (2) No
  journey consumes propellant: `route.propellant()` has one production reader, a display line
  (`tacsim/src/commands.rs:421-428`). (3) Atlas `+travel` moves a character by a `spacecraft` SELF mode with no
  ship (`mush-atlas/src/travel.rs:89-90`). (4) Housing's doctrine keeps title and access separate
  (`mush-housing/src/access.rs:16-18`). (5) A windowed domain is walkable (`mush-sim/src/lib.rs:264-271`).
  (6) Lantern, Tycho and any Dray exist only in docs. (7) `request_seat_activation` claims the account's one
  foreground focus (`cockpit.rs:77-80`).
- **Draft 0.2.** "Multi-room interiors (reserved)" and "gangway exits" (Missing) are superseded by J
  (members built in IC4; gangway by verb, exits P-G); "a title-to-operate bridge" by L (the live lease);
  "drive-dependent journey time" by N (applied in IC9).

### D. Stat-ing out the Dray

- **Exists.** `UnitSpec` (`definitions/unit.rs:803-849`, `deny_unknown_fields`); closed consequences
  (`:331-385`), closed `ComponentRole` weapon/sensor/ecm/eccm/power/core (`:446-524`); mobility tracked/
  wheeled/legged only, bind hard-codes GROUND (`:623-634,1821`); optional fields added with
  `skip_serializing_if` keep digests stable (`:822-847`); M32 `unit_cells` exist and tacsim never writes them
  (`mush-tactics/src/unit.rs:1805-1822`; `unit.rs:1860`); `DriveProfile` and `route_duration(_at)` exist with
  no production caller (`mush-sim/src/route.rs:607-628,794-848`); `BASE_MEDIA` has `space`, locomotions have
  `thrust` (`mush-atlas/src/physics.rs:58-71`); `@vault/create <name>=interior:<obj>` (staff).
- **Missing.** A propellant tank, consumption, life support, hold capacity, a drive kind, Dray content, mass,
  authorable stations.
- **Corrections.** (1) The trade slice needs no `TacticalUnit` at all: departure, the interior vault and pools
  read no `unit:<obj>` (`commands.rs:2422-2470`). (2) The hull's MUSH owner can already depart it with no
  station (`perm.rs:53-65`), despite help saying "(staff)" (`commands.rs:3439`). (3) Doc 10 §8 models cargo,
  life support and engines as components. (4) `FAMILIES` is a closed `[&str; 10]` and the table must have
  exactly ten rows (`tacsim/src/definitions/resources.rs:34,94,107`; `fixture.rs:94-98`'s
  `COMMODITY_FAMILIES` is a separate test-only array); `fixture.rs:96`'s "changing them needs no code" is stale.
  (5) Doc 11 marks the Vigil as the torch hull while every fixture route is `fusion_torch` and
  `nominal_drive` picks the law by name (`store.rs:842-846`). (6) The `tracked max_speed 0` stand-in makes a
  Dray read as ground armour to any arena reader.
- **Draft 0.2.** "A propellant tank, consumption, hold capacity, mass" (Missing) are superseded by N
  (the goods table, `logistics.stowage`, the burn law on the drive); correction (1) stands (no
  `TacticalUnit` is read), with `interior` now read beside `logistics`.

### E. Goods, the market and the chandlery

- **Exists.** Lot listings reserve a tranche-rooted pool (`mush-market/src/api.rs:707-760`); accept moves the
  bought quantity into the destination endpoint as a lot after both acceptances (`trade.rs:576-593,673-688`;
  `mush-inventory/src/pool.rs:651-760`); commodity defaults to catalog code (`commands.rs:950-975`); a hold is a
  vault backed by a container or sim interior (`mush-inventory/src/record.rs:84-96`; `mush-economy/src/commands.rs:3976-3996`);
  venue is a `MarketId` string per vendor and listing (`ids.rs:60-77`); maker budgets (`record.rs:1059-1124`);
  the market composes unconditionally, `[tacsim.economy].market` gates only the title issuer
  (`composition/mod.rs:137-148,508-532`).
- **Partial.** `CatalogSource` has zero production implementors (`composition/mod.rs:510`); `deliver()`
  mis-pages `SettlementRefused` on every settled lot sale (`module.rs:386-412`); one lot listing sells exactly
  once (`trade.rs:708-720`); `+market/sell` is object-only (`trade.rs:931-1085`); stock enters endpoints only
  through industry and no pack has a volatiles or propellant deposit (`fixture/tacsim_corridor_v2.json:39-44`).
- **Missing.** An NPC buyer for a lot; a presence gate (`remote` only changes TTL, `commands.rs:431-446` at `8a32734a`, `:524-539` at `c87d78f7`); a
  stock seed path; restock; a food model; a departure debit.
- **Corrections.** (1) Lot reservations are **never expired** in production while listings default to 14 real
  days against a 7-IC-day hold, and `is_current` never checks the hold: a late accept takes the money and
  delivers nothing (`mush-inventory/src/record.rs:33-35`, `mush-market/src/commands.rs:1001-1008`,
  `trade.rs:90-131`; `lot.rs` has no compensation). (2) Endpoint choices are capped at 64 per actor and staff
  may take every vault, so staff seeding breaks past ~64 vaults (`mush-inventory/src/provider.rs:63,187-206`).
  (3) Grade is seller-declared and `dissolve` drops it (`pool.rs:1092,1216-1245`). (4) A container placed in a
  sim interior is sim-bound and cannot back a container vault (`mush-sim/src/data.rs:1241-1248`).
- **Draft 0.2.** "One lot listing sells exactly once", "a stock seed path" and "an NPC buyer for a lot"
  are superseded by M: port asks draw from seeded pools (`LotOp::Draw`, no hold), bids are `Supply::Wanted`
  with `LotOp::Deliver`; correction (1)'s hold hazard is closed by IC0 for player listings and avoided by
  construction for port asks.

### F. Flying

- **Exists.** CarrierJourney over an authored one-way `RouteDef`, crew-driven departure, arrival as a
  placement, the in-flight seal, interdiction raises (`journey.rs:1021-1172`; `commands.rs:2207-2256,2606-2650`);
  corridor locations become atlas sites and routes a `spacecraft` connection (`tacsim/src/content.rs:160-356`);
  atlas real-time caps, stake-outs, `+intercept`, service-vehicle doors; one global world rate
  (`tacsim/src/commands.rs:273-330`, `mush-core/src/clock.rs:74,83`).
- **Partial.** The atlas copy of a route is one leg with bands zeroed and no checkpoint
  (`content.rs:320-340`); no interception can fire on any shipped fixture leg; `@journey/depart` is staff-shaped;
  no port creates a domain; egress onto a passive domain strands the player (`lib.rs:263-276`); journey time
  ignores drive and payload; `+travel/status` lacks an aboard arm; production loads no corridor pack
  (`deploy/instance/mushd.toml.tmpl:197-199`).
- **Missing.** A dock/airlock model; a player board verb; a destination seal on static exits into a cruising
  interior (`lib.rs:293-303` ignores `dest`); a `LocationDef` dock field; a player undock word; an arrival
  fact provider; Kirkwood as a place.
- **Corrections.** (1) A **non-staff** pilot with place tokens gets the manual-route arm: any `#dbref` legs at
  1 s each, no propellant, effectively a teleport (`commands.rs` build_route; `route.rs:142`). (2) Departure has
  no presence check (`drives()` never reads location). (3) The arena's BOARD composes `declare_station`,
  `place_interior` and `request_seat_activation` from public calls (`cockpit.rs:1943,2062,2076`): a ship board
  verb is a re-composition. (4) The port domain host **must** be the location room (route endpoints are
  seed-placed location rooms, `origin_anchor` returns the domain host, one domain per host; `store.rs`
  materialize_route, `window.rs:195`); berths are poses or slots inside one domain. (5) A pack applier can open
  a domain (`content.rs:547-572`). (6) An Interior-placed actor cannot take ANY exit; multi-room interiors are
  reserved. (7) `unplace` and `open_window` are the release and walkable-ground seams (`mush-atlas/src/window.rs:902,1944`).
  (8) The fixture's `ceres_freeport` has no route; `ceres_phobos` leaves from `ceres_extraction`. (9) `dev1`'s
  `IC_RATIO=4` is only the generation-one seed; persisted truth or `@tacsim/time` decides the live rate.
- **Draft 0.2.** "Journey time ignores drive and payload" (Partial) is superseded by N and D-18-30
  (applied in IC9); "a dock/airlock model" and "a player board verb" by J (`+ship/board`/`+ship/leave`);
  correction (1)'s manual-route arm is closed by IC0.

### G. Station maps as atlas packs

- **Exists.** Pack wire, digests, caps, latch, boot pass (`mush-content/src/pack.rs:9-26,58-93,404-417`;
  `module.rs:108-160`); 14 atlas kinds, no portal or tile entry kind (`mush-atlas/src/apply.rs:56-71`);
  structures with floors, rooms on `{floor:[structure,i]}`, portal modes hatch/stair/ladder/door with
  `at`+`dz_m` (`def.rs:273-479`, `record.rs:1110-1124`); items with `interior:true`; features (counters);
  themes (`theme.rs:83-101`); the validator (`validate.rs:1-33`); the digest-pin and validate-clean test
  shapes (`tests/starter.rs:31-60`, `tests/validate.rs:138-152`).
- **Partial.** Neighborhoods in an orbital region are untested; a connection is single-region
  (`def.rs:114-174`); topology seeds are Limbo, World rooms and ground access points (`validate.rs:1958-2000`);
  `RoomDef` has no interior or marker field; the building agent is staff-only and its SCHEMA omits
  neighborhood/region (`mush-worldgen/src/plan.rs:39-60`); `known_modes` lacks `spacecraft` (`validate.rs:735-740`).
- **Missing.** A join from an atlas room to the corridor location room; a cross-frame connection; a live hatch;
  a pack-authored anchored ship interior; bank/registry providers; a digest CLI (the pin test is the tool).
- **Corrections.** (1) Cross-pack exits are a shipped pattern (staff_lobby → ooc_area). (2) `access_room` already
  honours an endpoint naming any roomgeo'd room (`travel.rs:750-758`), so the corridor-to-station join is S.
  (3) `+travel` duration is always the authored legs' `base_s` (`travel.rs:822`); the ephemeris pins only the
  drawn corridor and the carrier `RouteDef`. (4) Same-body places collapse to one point
  (`content.rs:144-149`). (5) No test or pack uses an orbital region; the tested path is a **surface** region on
  a body (`proving_ground.json`, `region.limbo`). (6) Region coordinates cap at 1,000 km (`geom.rs:13`), so a
  one-region lane is symbolic. (7) A timed exit can join regions at 1.4 m/s: a hack, ruled out. (8) `+dock` was
  retired by 16b cut T (`mush-server/src/modules.rs:1659-1664`).

### H. The content and seed pipeline

- **Exists.** One pack format; tacsim `RulesPack` with 15 owned kinds and foreign entries (`pack.rs:173-234,
  884-900,940-946`); the misplacement rule (`:1129-1155`); the latch's per-entry digest plan, honest partial
  apply, downgrade refusal (`mush-content/src/apply.rs:152-326`); boot packs hard-coded in `content_module_with`
  (`composition/mod.rs:762-787`); `PlaceApplier`/`RouteApplier`/`ArenaApplier` (`tacsim/src/content.rs:76-81,
  161-352,612-647`); `@tacsim/seed` through the latch; instruments created by the sheet module before the
  content pass (`sheet/mod.rs:346-352`); tacsim already depends on every economy, org, market, inventory and
  sim crate (`Cargo.toml:22-54`; `lib.rs:36`'s "no economy edge" is stale).
- **Partial.** Built-in packs never enter `TacsimStore.loaded` or the ledger (TAC-5, `lib.rs:208`), so
  `route_in_world` is blind to compiled-in routes; the legacy seed latch is live code but dead in production
  (`seed.rs:308-309`); `AppliedEntry` keeps only `Option<ObjId>` (`latch.rs:33-41`); a tacsim seed pack in
  `loaded` cannot `requires` atlas packs (`lib.rs:228-236`) while a latch-only pack can (`apply.rs:478`).
- **Missing.** `org`, `account`, `vault`, `vendor`, `good`, `hull`, `stock`, `drive`, `berth` kinds; a
  start room by reference.
- **Corrections.** (1) With an empty delivery table any non-asset purchase settles money and delivers
  nothing or mis-pages (`mush-market/src/module.rs:383-411`). (2) `NegativePolicy` unenforced means a System
  account with unbounded credit can mint commodities by post. (3) Vault backings are Container, SpatialInterior
  or Registered, never an atlas room (`record.rs:867-879`). (4) Tests pin that the market never mints and that
  extraction is the only `mint_origin` caller (`mush-market/tests/e7r2.rs:315-332`). (5) Titling needs no new
  issuer strictly (housing and industry issuers are registered); a `tacsim.registry` issuer is a naming
  choice. (6) `tacsim_corridor_v2.json` already models Ceres Freeport, Phobos Yard, Mars places. (7) The
  game's boot packs are also on the atlas shelf (`composition/mod.rs:690-707`). (8) Only `s9_corridor` and
  `boot_parity` compose the content module; `e7c1`, `e7r1`, `e7r2` and `e7r2_proving_loop` do not (verified by
  the buildability judge), so deleting the legacy latch ports four suites.
- **Draft 0.2.** Correction (4) is **wrong** as stated and superseded by M: work-order delivery also
  calls `mint_origin` (`mush-industry/src/provider.rs:2693-2875`), and the pin asserts only that
  mush-market never mints; the `tacsim.corridor` v1 id this beat proposed collides with the published
  fixture (P; D-18-36); the `stock` kind is now the `vendors[].stock` rows plus the receipt family.

### I. NPCs, providers and help

- **Exists.** The interface seam (`mush-core/src/module.rs:1053-1089,1167-1183`), eight providers composed,
  the asker-authority turn path (`mush-npc/src/module.rs:1413-1570`), fallbacks with command hints, npc content
  entries, secrecy tiers, cost controls; no NPC action moves money (`action.rs:38-78`; `market.buy` asserted
  Unknown, `mush-market/src/interface.rs:203-206`).
- **Partial.** Providers run under the `npc` module identity: a cross-namespace `ctx.persist` is dropped
  (logged once at error, `mush-core/src/world.rs:1677-1695`), so `request.file` through an NPC creates a
  RAM-only job (confirmed, `mush-jobs/src/api.rs:173-219`), and typed reads fail ForeignOwner; rule 2
  (read/propose) is convention; the economist replies to the ROOM after filtering as the asker
  (`interface.rs:89,113`) and prints minor units; `travel.next` lists atlas carriers only.
- **Missing.** `ledger`, `ship`, `port` providers; `market.depth` as an ability; any IC NPC; help topics
  economy/trade/ships/ports; per-NPC provider args.
- **Corrections.** (1) A hold is an economy Vault plus inventory pools, found through plain state; only pool
  quantities need the identity fix. (2) The tactics `ScalarCell` is the designed seam for a fuel cell, but this
  slice keeps propellant in inventory (§4.2). (3) `Ctx.module` and `typed_state` are pub fields
  (`mush-core/src/cmd.rs:332,336`), so the identity fix is small. (4) `MarketId` is a bare string with no place
  binding; the port map supplies one. (5) The crew gates a ship provider needs are `pub(crate)` in mush-sim.
  (6) Per-command help wins over topic pages for exact names (`mush-server/src/help.rs:4`).


### J. A six-room interior the crew walks (Draft 0.2)

- **Exists.** `InteriorSpaceRecord` already carries `rooms: BTreeSet<ObjId>` "reserved for cut 2's
  boundary rule", `compartments` (placed-occupant slots; only `maximum_items` enforced,
  `transition.rs:343-362`) and `egresses`, persisted with `serde(default)` so members need no wire change
  (`mush-sim/src/data.rs:824-839`); `Transition::Space` is the only writer (`{space}`, `transition.rs:596-608`)
  and the arrival scene-floor raise the only reader (`journey.rs:1429-1451`); the boundary rule is doc 06
  §7.5's text (`06-object-behaviors-spatial-domains.md:764`), never built; the carrier seal refuses
  placement crossings under way and leaves moves inside one subtree free (`transition.rs:396-445`); console
  reach is location == console room (`focus.rs:128-137`); a disconnected body stays placed
  (`seat.rs:736-770`); exits match before commands, so a room-local alias shadows a verb only there
  (`mush-core/src/cmd.rs:22-26`); mush-housing ships a destination-keyed `AccessHook` under MoveAttempt and
  CarryAttempt with the owner ruling "no lock, no exit rewrite" (`mush-housing/src/hooks.rs:1-40`, D-E5-4).
- **Partial.** `rooms` has no writer, no boot validation, no reverse index and no destroy arm; an
  `EgressSpec` can only land Direct on the effective domain, which strands an actor on a passive port
  domain (`commands.rs:2074-2112`; `window.rs:226-234`); `sim.interior` is Room-only at `@behavior/add`
  while atlas pods attach it to Things (`lib.rs:224-232`; `mush-atlas/src/apply.rs:1268-1296`); mush-sim
  creates no world objects anywhere (tacsim does, `content.rs:185,561`).
- **Missing.** The boundary rule itself (gate `lib.rs:293-303` ignores exit and dest; projection
  `graph.rs:167` pins an occupant to the space object, and boot reconcile would teleport walkers back,
  `boot.rs:604-624`); a member declaration with validation; a destination seal; a gangway; an interior
  template and per-hull instantiation; a passage manifest; an inventory seal on under-way backings; a
  ship status line in `look`.
- **Corrections to Draft 0.1.** (1) D-18-8's "one boardable Bridge" and its bay are **replaced by R5**:
  one record per hull whose `rooms` lists six walkable members, the airlock as the space object, and a
  separate unwalkable store space as the vault backing (the bay principle kept: a walkable room must never
  back a vault, `custody.rs:76-105`, `commands.rs:2224-2245`). (2) The alternative of six records, one per
  room, needs a placement write per step and makes every room a possible backing; unplaced riders (the
  atlas bus, `travel.rs:2595-2630`) lose the carrier seal and placement-derived locality; both refused.
  (3) A seat-reach re-check is owed only when a station exists; the slice declares none. (4) Six hulls
  with plain room names are ambiguous to staff, so the hull is named by a `look` status line, not by a
  room-name prefix.

### K. The captain's day: queued orders executed offline (Draft 0.2)

- **Exists.** `@wait` defers a stored line and `fire_due` dispatches it as its author
  (`mush-core/src/engine.rs:4312-4333,3213-3232`); module code can enqueue one for any ObjId
  (`mush-script/src/runtime.rs:1043-1049`); `submit_authoritative` stamps `ctx.actor` and the pre-ingress
  queue is one per actor, 64 actors, drained one per step (`module.rs:4844-4861`; `engine.rs:688-721,
  4043-4085`); ticks run as GOD and the campaign rule voids two GOD submissions in one drain
  (`mush-campaign/src/tick.rs:28-37`); `Ctx.actor` is pub and both mush-npc and tacsim's own seed swap it
  (`mush-npc/src/module.rs:1496-1502`; `tacsim/src/seed.rs:384-398`); the journey, housing and campaign
  pull ticks fix the budget shape (visit/work, rotation cursor, one unit per record per fire); journeys
  relabel IC stamps across a rate generation and never pause (`journey.rs:1230-1257`); all module ops of
  a step ride one WriteBatch (`mush-server/src/world_task.rs:7230-7262`); housing's notice queue renders
  once on `Connected` (`mush-housing/src/notice.rs:1-20`).
- **Partial.** `@wait` as the queue: RAM-only and re-armed empty at boot (`sched.rs:1-14`), invisible to
  the player (`@ps` is Wizard-only), 100 slots shared with the player's own waits, output to an offline
  connection, and two due lines for one pilot in one slice void each other; `schedule_source` can act as
  the pilot but writes typed families only (`journey.rs:13-20`); industry's WorldIc schedules pause on a
  rate change (`provider.rs:2977-2985`).
- **Missing.** A durable per-hull orders record and a pull tick executing it as the pilot; player verbs
  to view and edit it; a UTC time parser; a reconnect notice store (`mush_mail::deliver` is crate-private);
  a contract-keyed lot leg (today `market.lot.{op}.{listing}.{IC second}`, `lot.rs:708-745`); a
  ship-locality presence mode.
- **Corrections to Draft 0.1.** (1) `+orders`/`+order` are registered by mush-propagation in the
  production composition (`commands.rs:1252-1279`; `composition/mod.rs:247`), so the queue verb is
  `+ship/order`. (2) Typed-state reads inside market code called from a tacsim tick are scoped to the
  **tick's** module (`engine.rs:3285-3289`), and IC0's `is_current` refuses an unreadable hold, so tacsim
  must be an inventory quantity delegate before any queued buy. (3) Draw-backed port asks hold nothing, so
  the hold read applies only to player lot listings. (4) R3 binds the **rate** as well as the lane: dev1
  seeds `IC_RATIO=4` (`deploy/dev1/instance.env:81`), where a 20 h passage is five real days.

### L. The backer: a cooperative that stakes new pilots (Draft 0.2)

- **Exists.** `offer_lease` with an org lessor builds the parties, records the lessor's acceptance, sets
  no deadline and one `Attestation{Staff}` condition (`contract_live.rs:4516-4636`); activation places
  `Encumbrance::Lease` when an Obligation deliverable exists (`contract.rs:2314-2329`); `end_lease` ends
  it (`:4710`); an org may `post` from its own **scoped** account (`api.rs:385-393`; recovery's stipend,
  `recoverycmd.rs:621-661,701`); an outcome may pay a non-party account by `Share::Exact` splits that must
  sum (`contract.rs:1886-1909,1418-1458`) and settlement is atomic (`:2856-2873`); the market's asset
  purchase shape moves title once both parties accept (`trade.rs:829-848`); invariant 12 refuses an
  encumbered transfer (`engine.rs:1121-1125`); the live-lease query precedent (`recovery.rs:1000-1028`);
  a gateway account is reachable from a Ctx (`seatstate.rs:142-146`); the tacsim economy observer is wired
  (`composition/mod.rs:312`); doc 11:109-111 gives the Co-op the fleet and the motive.
- **Partial.** A lease requires a rent and amounts are fixed (`contract_live.rs:4463-4479`;
  `obligation.rs:372-376`): a 1-minor rent on `EconomicClock::Manual` never comes due; every sell path
  hard-codes one payout (`trade.rs:1023-1036,877-892`); `accept_contract_as` needs a party cap and tacsim's
  table is empty (`contract_live.rs:5151-5175`; `composition/mod.rs:361`); `page_stale_contracts` pages
  **every** live lease at 7 real days with no lease exemption (`sweep.rs:1597-1640`; `contract.rs:109`);
  the per-issuer cap of 16 binds before the party cap of 32, and `offer_lease` checks neither
  (`contract.rs:88`; `contract_live.rs:4987-4997`).
- **Missing.** A programme definition and enrolment record; a `ProceedsPolicy` seam; a bonded-hold rule;
  the buyout verb and graduation observer (10b:1753 designed a declared-price lease buyout, never built);
  dormancy and cooldown on a tacsim tick.
- **Corrections to Draft 0.1.** (1) D-18-5/6/7 (the Clearing loan) and D-18-9/10 (the Tycho charter
  record and pool) are **replaced by R1/R2**: the Co-op stakes and charters; Clearing opens accounts and
  lends nothing; Tycho stays the insurer (its Re-float is "no sale", 11:119, which contradicts a buyout);
  P1 leaves the slice because there is no debt. (2) The charter is the **lease now**, not a placeholder
  record: tacsim calls `offer_lease` directly, cheaper than Q-18-4's M–L repair of the recovery provider.
  (3) A separate programme org (the Co-op First Captain Trust) issues the charters so eight leases do not
  eat the Co-op's 16 issuer slots. (4) The pasted seed (about ten round trips in all) cannot carry the
  buyout timeline; stock is sized to the programme (§4.3).

### M. Finite seeded stock, asks that sell real lots, buyers that take them (Draft 0.2)

- **Exists.** `mint_origin` is the one creator of rooted matter and needs an `ActionTxn`
  (`mush-inventory/src/pool.rs:1280-1338`; `Lineage::Origin` is documented for "a seeded fixture",
  `record.rs:227-233`); `credit` creates an unrooted pool (`pool.rs:285`); `dissolve` is a shipped action,
  `inv.dissolve` (`provider.rs:762-772`); `transfer` moves pooled quantity and carries lineage
  (`pool.rs:413-528`); an `ActionTxn` exists only inside a registered provider and tacsim registers none
  (`mush-core/src/action.rs:3895-3912`); the content boot pass is one GOD drain (`engine.rs:2285-2299`);
  typed-state reads are delegation-gated (`action_state.rs:658-680`); the fact provider answers
  `inventory.transferred` (`facts.rs:44-82`); `MAX_QUANTITY` is 100,000 per listing and quote
  (`mush-market/src/record.rs:90-91`); the no-mint pins scan only mush-market files (`tests/e7r2.rs:310-333`).
- **Partial.** A `Supply::Lot` listing sells once and its remainder returns to the seller
  (`trade.rs:732-748`); market holds are never renewed or expired in production; the lapse sweep deletes
  an expired lot listing **without releasing its hold**, and held records count against availability
  regardless of deadline (`boot.rs:169-205`; `store.rs:657-668`; `mush-core/src/resource.rs:434-436`); at
  rate 24 a hold lasts 7 real hours (`record.rs:33-35`); `fund_from_faucet` is a 60 s drip (`api.rs:556-590`).
- **Missing.** A funded bid record and a matter leg into the buyer's endpoint; a seeding action with a
  durable receipt; a non-GOD driver; mass, volume and a unit per good (`CommodityDef` is label/in_use,
  `pack.rs:522-534`; the catalog wire silently drops unknown fields, `record.rs:328-348`).
- **Corrections to Draft 0.1.** (1) D-18-15's `LotOp::Issue`/`Retire` is **replaced by R4**: stock is
  minted once per issue into storefront pools and a port ask draws from the pool (`LotOp::Draw`, a
  `transfer`), so no shelf hold, no Renew, no stranded hold; the e7r2 pin amendment is withdrawn and
  `e7r2.rs:327-328`'s "exactly extraction's" comment is stale (work-order delivery also mints,
  `mush-industry/src/provider.rs:2693-2875`). (2) §3-H correction (4) is wrong for the same reason.
  (3) The applier must never submit (one GOD drain voids two port entries); a per-port clerk actor on a
  tacsim tick drives mint then `inv.dissolve`. (4) Units are tonnes, so the 100,000 cap and every pasted
  price fit without a market change; Draft 0.1's "200,000 t on the listing" exceeded the cap.

### N. Mass and volume: the Dray's limits and payload-dependent passage time (Draft 0.2)

- **Exists.** `route_duration` is a pure laden evaluator: each leg × 1000/thrust × (capacity + payload)/
  capacity, `Overloaded` above `payload_capacity` (`mush-sim/src/route.rs:794-836`); `DriveProfile::new`
  and `with_law` are public (`:637-660`); the recorded ruling counts propellant inside payload
  (`tacsim/src/store.rs:823-835`); doc 06 D3's `LoadProfile`/`CapacitySpec` shape (`data.rs:797-810`);
  `CarrierJourney` persists the `RouteDef` and derives every instant in `leg_due_at` from raw `base_s`,
  with serde-defaulted wire fields (`journey.rs:190-232,398-408`); commands read inventory typed state
  through `Ctx::action_state` (`mush-core/src/module.rs:561,573`; IC0's `hold_standing`); doc 10 T1 is
  binding: duration is a function of route, drive **and** payload (`10-tacsim-foundations.md:286-291`).
- **Partial.** No production caller of `route_duration`; `LotCensus` is a bare count and `ActionTxn` has
  no scan (`record.rs:917-921`; `pool.rs:56-63`); market delivery lands lots while `debit` works on pools,
  and dissolve-then-debit in one txn is a `DuplicateMutation` (`pool.rs:651-756,306,1092-1170`);
  `resources.json` is decode-tested, not digest-pinned (`tests/content.rs:313-322`).
- **Missing.** Per-good mass, packed volume, unit and stowage; an endpoint capacity on every write; per-leg
  laden times frozen on the journey; per-hull dry mass, stowage limits and drive; a burn law; a departure
  manifest.
- **Corrections to Draft 0.1.** (1) D-18-17's "payload does not vary flight time until P6" is a live
  breach of binding T1; Draft 0.2 applies the evaluator (D-18-30). (2) `payload_capacity` is both the
  Overloaded rail and the laden denominator (`route.rs:806-831`), so a binding ceiling and the time curve
  are one lever. (3) `RouteDef.propellant` is a payload-unit quantity in the ruling and in
  `tacsim/tests/e7c1.rs:1447`; it is not repurposed as permille (the burn law lives on the drive). (4) A
  capacity check inside `credit` would refuse `dissolve`, which credits while the lot is still counted
  (`pool.rs:1162,1169`). (5) Pools-only hull endpoints make the in-txn load figure five point lookups and
  retire the lot-consume blocker. (6) At the pasted 0.5/1/0.5 m³/t a 120 m³ hold can never bind; volume is
  deferred (Q-18-21).
- **Draft 0.3.** Correction (6) is **reversed by R7** (beat R): the packed volumes are re-authored so
  each leg has a dense and a bulky good and the hold binds on both; "a burn law" (Missing) is now the
  drive's tagged law with the coast rate as data (beat Q); (5)'s "five point lookups" is one lookup per
  good the compartment accepts (four for the hold, one each for the tank and the locker).

### O. Fifteen-room stations with decks, locks and a berth connection (Draft 0.2)

- **Exists.** Paired exits lock only `names[0]` (`mush-atlas/src/apply.rs:1500-1519`); pack locks test
  only the mover (`flag^NAME` matches markers, `mush-core/src/lock.rs:122-155,266-275`); module code may
  set any marker and a registered one is self-settable (`world.rs:1423-1440`; `building.rs:456-470`);
  decks are structure floors rising in index and z (`validate.rs:1679-1715`); `dz_m` may be negative;
  only Door/Gate/Open portals cut walls and only structure rings and `wall` features are walls
  (`raster.rs:412-458`); `+atlas` and the `+map` frame hide by **access** (`view.rs:38-46`); runtime exit
  pairs exist (patch doors, `travel.rs:1222-1290`); pack entries resolve through the latch
  (`travel.rs:960-967`); hydration defaults off (`def.rs:222-230`); a room with an outside exit leading in
  cannot be retired (`apply.rs:1687-1708`).
- **Partial.** `+move` refuses every non-staff walker into a Restricted room even after the exit lock
  admits them (`window.rs:2276-2287`); `ExitDef` has no `fail`/`succ`; `ItemDef` has no lock
  (`def.rs:509-532`; the atlas applier sets none, `mush-atlas/src/apply.rs:1205-1262`; `get` reads a
  Thing's Basic lock as its take lock, `mush-core/src/commands/items.rs:46-52`); every Limbo exit is
  global (`movement.rs:24-42`), so a Limbo IC
  door guarded only "from Limbo" still admits `o` → `x` → `c` from Stickney; there is no `lift` portal
  mode; theme banned words only warn.
- **Missing.** A topology seed for a room reached by carrier (D-18-19 kept); a gangway API; berth-marker
  sync; the IC-return rule; the `port` binding's room references; the four packs.
- **Corrections to Draft 0.1.** (1) The IC door needs **no** approval lock: tacsim's `OnboardingGate`
  already refuses every exit line for an unapproved character (`onboarding.rs:97-112`), so D-18-3's P9
  marker is withdrawn. (2) Adjacent public rooms with no exit are open floor on the scene map unless the
  partition is a `wall` feature; the packs author them. (3) `disembark` is already the sim's egress alias
  (`mush-sim/src/commands.rs:3428` at `c87d78f7`) and strands the player; no gangway alias may use it. (4) The
  four pasted freight rooms per station need marker locks, marker sync and the `+move` fix, which the loop
  never exercises: Ceres and Stickney ship 13 and 12 rooms (18a), the bonded hall, transfer junction and
  tank gallery deferred to cut P-F. (5) Dock-time object creation needs a `Ctx`, so docking runs on the
  journey tick or a command, never in an authoritative prepare.

### P. Audit of the pasted proposal's tree claims (Draft 0.2)

- Sections 7–9 of the owner's pasted proposal (R-set, §1) were opened against `8a32734a`. Of the twelve
  implementation-gap rows, every claim is confirmed or narrowed and none refuted; of the authority rules,
  all confirmed, with the readiness gate confirmed missing (nearest seams `--check-config`, `@tacsim/pack
  verify`, lobby `require_layer`); of the passage rules, the approach-then-dock phase is **unsupported**
  (only journeys move a carrier root: `commands.rs:2735`, `journey.rs:834,1357`) and under R3 docking is
  automatic.
- **Narrowings.** The metric-room binding primitive exists (`open_passive_domain(location, gallery)`,
  `window.rs:195`); only the record is missing. The route catalog is empty in production
  (`deploy/instance/mushd.toml.tmpl:195-199`). The market title issuer matters only when stocking an
  untitled object (`mush-market/src/api.rs:791-834`), so a charter needs none. A passive port domain
  **holds** no window or active-domain slot — its record is written `window: None`
  (`window.rs:226-234`), so `windows()` never counts it (`:279-285`) — but `open_passive_domain`
  itself **refuses while `MAX_WINDOWS` (= `MAX_ACTIVE_DOMAINS` = 8) windows are open**
  (`window.rs:190-194` doc comment; the guard at `:205-206`; `domain.rs:43`), and the applier runs in
  the first-boot pass where any refusal aborts the boot (`mush-content/src/module.rs:113-139`). The
  "eats into MAX_WINDOWS" risk is therefore half refuted: a passive domain takes no slot, yet a kept
  world with eight windows open at restart would fail the lane pack's first apply. IC3 lands the S
  mush-sim change that skips the window-count guard for passive opens (the guard protects window slots
  a passive domain never takes) and adds "eight windows open at restart" to its kept-world boot test
  (§9 risk 1).
- **Corrections to Draft 0.1 found by the audit.** (1) Draft 0.1's lane pack `tacsim.corridor` v1
  **collides** with the published test fixture id and digest (`tacsim/src/fixture.rs:34`; `store.rs:6`);
  the lane is `tacsim.lane.ceres_phobos` v1 (D-18-36). (2) D-18-8's Cargo Bay as the vault backing
  contradicts R5's walkable hold (resolved by the store space, J). (3) D-18-9's bypass rested on "one
  boardable room", which R5 removes; the lease is now the who-may gate and the pasted "live authorized
  operator" is read as current authority, not a connected session. (4) The journey's arrival is a Coarse
  tick on the legacy Outbox (`journey.rs:11-35`): the cross-lane join exists at both ends, and the burn
  ticket plus a pull tick is R3's natural seam. (5) "Disable automatic neighborhood expansion" is moot;
  none exists. (6) Three of the audit's "missing" items were already closed by the IC0 working tree
  (the Lot short-circuit, the manual-route arm, the listing/hold TTL); `spacecraft` in `SELF_MODES` is
  still owed (Q-18-6).

### Q. The sky: what the slice assumes frozen, and what a moving sky would undo (Draft 0.3)

- **Exists.** A circular, coplanar, heliocentric ephemeris with a **time-varying** query: `Body
  {orbit_uau, period_mdays, phase_mdeg}`, `phase_mdeg_at(id, t)` advancing by `rate_permille` of real
  motion per IC second (the authored phase at rate 0), `distance_km_at(a, b, t)` by the law of cosines
  in f64 rounded to km (`tacsim/src/ephemeris.rs:42-52,135-170,180-195`); the knob
  `[tacsim.time.ephemeris] {epoch, rate}`, "0 freezes the sky" (`config.rs:76-88`;
  `mushd.toml.example:190-192`; `deploy/instance/mushd.toml.tmpl:218-220`, both `epoch = 0`); two
  distance laws as **ratios to a pin**, torch ∝ √d and coast ∝ d, on `DriveProfile`
  (`mush-sim/src/route.rs:563-598,607-612`); a committed passage never re-reads the sky or the catalog —
  `CarrierJourney` embeds its `RouteDef` and stamps every leg instant from `departed + Σ base_s`,
  `reanchor` keeps `departed.at` across rate generations (`journey.rs:191-202,398-408,1231-1255`;
  `commands.rs:2306-2311` resolves the catalog only in `build_route`); the IC clock is a continuous
  calendar of IC Unix seconds (`mush-core/src/clock.rs:197-218,335`; `2350-01-01T00:00Z` =
  11,991,628,800; dev1 ≈ IC 2350.2 **if** ratio 4 held since unix 0, `deploy/dev1/instance.env:81-82` —
  the persisted timeline decides); doc 10 T1 is binding and written for an *aligned* sky
  (`10-tacsim-foundations.md:287-291`); doc 15's lag table is already ranged for a moving sky and D-15-11
  derives a ship's lag from journey progress (`15-comms-relay-backbone.md:87-111,312`).
- **Partial.** The rate knob is **inert in production**: `materialize_route` pins one `distance_km` per
  route from `pinned_km`, always the epoch (`store.rs:761-813`; `ephemeris.rs:168-170`); `reindex_routes`
  caches it and `route_in_world` re-materializes at the epoch (`store.rs:493-521,720-753`); atlas sites
  are drawn at the epoch phase with z = 0 and the corridor as a two-point chord of Σ authored `base_s`
  (`content.rs:130-149,306-340`); `route_duration` and `route_duration_at` have test callers only. The
  pin is the **direct chord between the endpoint bodies**, not the flown path, and the pack cannot author
  it (`pack.rs:849-871`): direct Ceres–Mars at the fixture phases is 1.293 AU while the lane's legs sum to
  1.302 AU. `BodyDef` is a strict three-field wire with a hand-written `Serialize` (`pack.rs:593-626`),
  so new fields must be `Option` skipped when absent or every digest moves. The epoch is server config
  while the phases are pack data tuned to no real date (`tacsim/tests/ephemeris.rs:66-86`); doc 10 T17
  puts the epoch in the pack (`10:762-764`), doc 14 §3.8 S7 put it in config
  (`14-spatial-implementation-plan.md:881-882`). `spacecraft` is still in `SELF_MODES`
  (`mush-atlas/src/travel.rs:90`; `content.rs:44`), so atlas `+travel` can fly the epoch projection.
- **Missing.** 3D positions with eccentricity, inclination, node, perihelion and parents; real data
  (grep `inclination|eccentric|horizons|kepler` outside the arena: none); a planner that takes the sky at
  a departure instant (`route_duration` takes neither, `route.rs:794-834`); a flip law with an absolute
  acceleration or a coast law with an absolute km/s (`coast_km` nowhere); a chart and a position-reading
  price policy (`@corridor/show`'s pin line is the only sky render, `commands.rs:432-436`).
- **Corrections to Draft 0.2.** (1) IC2's "the route's `distance_km` as the sum" cannot be produced by
  today's code; the field stays the chord, relabelled, and the truth lives in the passage plan (§4.8).
  (2) The 205 km/s rate as "a constant the test owns" is the one regret the engine would unwind; it
  becomes drive data and IC2's test checks the pack against the sky (D-18-45). (3) Mars moves 0.524°/day:
  over the laden Kirkwood→Mars leg (4.74 IC days unladen, 8.4 laden — not the route's 11.0) that is
  2.5–4.4°, and both legs being near-radial the intercept correction here is about **−4 %** per leg,
  not a 25 % under-statement; the planner still carries a fixed-count intercept for tangential lanes.
  (4) Kirkwood at 2.06 AU (2.96 yr, the 4:1 gap) cannot co-orbit Ceres (4.60 yr), and "on the
  Ceres–Mars lane" (doc 11 §2's 2320s row and §5; §1 never mentions Kirkwood) is a **three-body**
  alignment, not the Ceres–Mars one: the slice's legs put Kirkwood at ~135°, within ~5° of both bodies
  at the epoch, and per 3.18-yr synodic cycle Kirkwood's offset from the next conjunction longitude
  steps by 3.18 × 360/2.957 − 3.18 × 360/4.60 ≈ 27° − 249° ≡ +138°, a pseudo-random walk, so a ±5°
  via-Kirkwood season is hit on about 3 % of cycles (±15°: about 8 %) — once in roughly 40–110 years,
  never "70 days in every 3.18". The moment the sky moves, an independent 2.06 AU Kirkwood is off the
  lane for decades, which is the R9 regret Q-18-36 now exists to avoid. (5) Ceres–Mars spans 1.242–4.290
  AU (circular bounds): a coaster's full outbound stretches up to 3.30× (19.6 h → ~65 h), a flip law's
  1.82×, so R3's "under a day" holds for a coast Dray only about 16 % of the cycle — the owner's call
  (Q-18-35). (6) T17's "real geometry" was never checked against a real ephemeris; every slice figure
  is tuned fiction, cited as the constant sky's output, never as fact.

### R. Volume beside mass: the hold binds on both, and the mixed load is the optimum (Draft 0.3)

- **Exists.** mush-sim's `CapacitySpec.limits: BTreeMap<SmolStr, i64>`, stored and unused, the naming
  precedent for measure-keyed integer limits (`mush-sim/src/data.rs:797-810`); doc 06 D3's open measure
  map, "summed and compared, never floating point" (`06-object-behaviors-spatial-domains.md:1285-1300`);
  unit-less u64 pool quantities (`mush-inventory/src/record.rs:989-999`; `pool.rs:413-420`);
  `MAX_QUANTITY` 100,000 per listing (`mush-market/src/record.rs:91`), which keeps the ledger unit the
  tonne; the lot pre-check's slot between the destination check and `cannot_deliver`
  (`mush-market/src/trade.rs:608-632`); the Draw backstop's home beside `Consume` in
  `prepare_authoritative`, the families handle cloned at composition (`lot.rs:168-176,199-203,469-540`);
  `Pools::pool(txn, endpoint, good) -> Result<Option<Pool>, _>` (`mush-inventory/src/pool.rs:253-264`);
  `industrial_parts` already one of the ten families (`tacsim/src/definitions/resources.rs:34-45`;
  `content/resources.json:69-73`); 18a's empty `item.yard_spares` cage and the Stickney Yards service
  clerk who can sign a Yard ask (18a:1219-1230, :1265).
- **Partial.** Typed-state reads inside an `ActionTxn` are **delegation-gated like writes** and a refused
  read poisons the txn (`mush-core/src/action_state.rs:1934-1960,2043-2068`; only a second *mutation* is
  refused, `:2071-2076`): the market's Draw backstop is covered because the market is already a quantity
  delegate (`tacsim/src/composition/mod.rs:404`); the executor's clamp needs IC5's tacsim delegation; an
  unreadable pool must refuse, never read as zero. Draft 0.2's stowage `{limit, unit}` shape cannot carry
  two limits; its clamp names "capacity" with no measure; "three" and "five point lookups" disagreed.
- **Missing.** Any capacity function, measure registry, packed volume or stowage code (grep
  `CapacityProvider|volume_l|MeasureId|stowage` in `crates/*.rs`: none); a bulky Stickney export, so the
  return leg has a decision (alloys at 300 L/t use 30 of 120 m³).
- **Corrections to Draft 0.2.** (1) Draft 0.2's outbound margins tie at 100/t for metals and ice, so no
  balance exists until a price moves: Kasei's metals bid 300 → 280 (Q-18-33). (2) Q-18-21's 1.5 m³/t ice
  gives no whole-tonne load that fills both limits (72.7 t); 1.4 does (80 t). (3) "81 t of ice after 20 t
  of concentrate (120.05 m³)" is refused by mass too (101 t, 121.4 m³); the volume-only cases are 19 + 81
  (100 t, 121.0 m³) and 86 t of ice alone (120.4 m³). (4) The first cash-bound return is 49 t fittings +
  20 t alloys (118.7 m³), not 45 + 24; the executor clamps order by order and never optimises across
  orders. (5) Draft 0.2's "day 14–15" did not reproduce under its own numbers (day 17); this set lands
  day 15. (6) Side-slips: 1,696,268 IC s is 19 h 38 m, not 19 h 36 m; the 70 t return payload was
  111,176 kg, not 111,184; the propellant fixed point at a full hold is 40 (41 the greater); the 181,000
  kg rail is ≥ 180,680, not "=" it. (7) The volume design's own slips, fixed by its judge: the alloys-bid
  edge is about 421, not 452; the second cash-bound return is 96 t = 45 + 51, not 94 = 45 + 49; the LP
  test cannot run in IC2 (no port entry before IC3): it is IC8's.

---

## 4. The design

The spine is the thin design of the Draft 0.2 review (fewest new mechanisms; the daily loop runnable
earliest), chosen by the daily-loop and buildability lenses, with the fiction lens's grafts (the ship as
a place: the status line, per-room gauges, named hulls, the watch-log framing, the NPC voices, the
passenger rule) and the one-lane design's grafts (the executor's buy clamp, the readiness report, the
Fact-gated bid payout, the tick skeleton as its own cut, the receipt family as the mint guard, the
contract-keyed lot leg, a separate programme org). Where the judges disagreed — the share base, the
passage law, gangway exits, the freight decks — the text says which way it goes and why, and §8 carries
the alternative.

### 4.1 Money: the backer programme

**The wallet.** `+credit` renders posted, held and available, in major units (a port of the retired
`+nuyen`, `56e877c4`); Draft 0.1's obligation column (every obligation where the pilot is debtor) stays
dormant until a debt record exists, because under R2 there is none and "owed" would always read zero.
The account is opened **inside `+coop/enrol`** (D-18-6 amended): the Clearing holds the money and lends
nothing to pilots, which is R2 read with doc 11 §4.8 (Clearing is "the Belt's bank and the Freeport's
fiscal agent" and "holds Kasei's and the Republic's Belt debt": its lending is to institutions, never
quoted as "lends to governments", which §4.8 does not say). `+credit/advance` is deleted from the plan;
D-18-5 and D-18-7 are replaced, and P1 (terms, cadence, repayment) leaves the slice because there is no
debt. Doc 11 §4.8 also makes Clearing "the corridor's NPC market-maker"; the slice does not use that
role (the standing offers are the vendors' own books, §4.3) and leaves it to the faction cut.

**As built (IC3, IC6):** `+credit` prints the one line posted / held / available
(`tacsim/src/port/credit.rs:21`, `CREDIT_HELP`), in the economy's render without thousands grouping —
`25000.00cr`, not this document's `25,000.00cr` prose (IC0's render; IC6 (5); IC11 L22). The account
is opened inside `+coop/enrol` by Ceres Clearing, as written.

**The corporation (D-18-21).** The **Belt Carriers' Cooperative** runs the **First Captain programme**.
Doc 11 §4.5 gives it the Dray fleet, Kirkwood Depot and "the baseline hauling contracts that keep the
corridor's water moving when no player will": it needs captains, which is R2's motive word for word, and
an owner-operator scheme on shares is a cooperative's natural shape. The other institutions keep their
doc 11 roles: **Tycho Mutual** is the insurer and recovery provider (its Re-float is "no sale", 11 §4.7,
which contradicts R1's buyout); **Lantern Haulage** is where a graduate goes (joining it needs ECO-3
membership verbs with no producer, `mush-org/src/commands.rs:435`); **Ceres Clearing** opens accounts.
The programme is issued from a separate legal person, the **Co-op First Captain Trust** (org
`coop_trust`, faction `freeport`, `ChapterOf` the Co-op in the faction cut): every live lease has
issuer = lessor and `MAX_OPEN_CONTRACTS_PER_ISSUER` is 16 (`mush-economy/src/contract.rs:88`, counted by
`offer_contract` at `contract_live.rs:4987-4997`; `offer_lease` checks no cap at all), so eight charters
on the Co-op itself would starve the hauling contracts its fiction says it publishes. The Trust holds the
programme capital in an org-**scoped** `tacsim.credit` account (an org actor is admitted only on a
scoped compartment, `api.rs:385-393`) with a floor below which it refuses to stake (the
`pool_can_fund` shape, `recovery.rs:914-920`). The story hook survives: the Co-op is the Tether's
public front (11 §4.5–4.6) and the programme's dispatcher is placed `enabled:false` for the faction cut.

**Where the verbs work.** `+coop` and `+coop/statement` are readable anywhere (like `+ship/orders`:
the pilot reads the lay at Stickney and aboard, §2 step 16, §6 part 2); only `+coop/enrol`,
`+coop/buyout` and `+coop/release` are bound to C05.

**What the registrar says** (the fiction line, read at C05 and printed by `+coop`): *"The Co-op stakes
new captains: a Co-op Dray on a working charter, twenty-five thousand in working cash, and a quarter of
your trading margin to the Co-op until you buy the hull out at the price on your papers. Sell under the
Co-op's reference price and the Co-op takes nothing. Go a week without a departure and the Co-op takes
its Dray back."*

**The charter is `Encumbrance::Lease`, now** (D-18-21; replaces D-18-9/10's charter record and P2's
lease migration). `+coop/enrol <name>` at the Registry & Co-op desk runs, in one command and one batch:
(1) the eligibility checks — an open gateway account with no prior enrolment ever (`account_of` through
`mush_sim::AccountOracle`, `seatstate.rs:142-146`; an unknown account refuses; `max_characters` is 3), a
free Trust Dray (the lowest-id titled hull with no live lease, the `next_loaner` rule,
`recovery.rs:935-948`), programme capital above the floor; (2) `open_account(Object(pilot),
tacsim.credit, forbid)` if absent; (3) `offer_lease{lessor Org(coop_trust), tenants [Object(pilot)],
premises hull, rent ObligationTemplate{amount 1 minor, clock Manual}}` followed by `accept_contract` as the
pilot — the market's offer-plus-accept-in-one-verb precedent (`trade.rs:647-666`); activation places the
Lease (`contract.rs:2314-2329`) and invariant 12 then keeps the hull from being sold out from under the
pilot (`engine.rs:1121-1125`); the rent must exist (a lease needs an Obligation deliverable,
`contract.rs:2314-2329`). Whether a **zero** rent is refused is unverified: `offer_lease` runs only
`check_structure` + `check_offerable` and no amount check (`contract_live.rs:4604-4605`); the
positivity checks that exist are `spec.amount.is_zero() || spec.amount.is_negative()` in
`api::create_system_obligation` (`api.rs:1731`) and `api::create_obligation` (`:1978`), and whether the
activation path that materialises the rent template goes through one of them was not shown (Draft 0.1's
cite, `contract.rs:1361`, is `apply_split`'s doc comment and says nothing about it). The rent is
therefore one minor unit on `EconomicClock::Manual`, which never comes due, as belt-and-braces: if
IC6 finds zero admitted, the 1-minor rent and Q-18-27's optional-rent change are both unnecessary and
the cut note says so; (4) the stake, `post` with actor
`Organization(coop_trust)`, two zero-sum lines from the programme account to the pilot's, keyed
`tacsim.backer.stake/<lease contract>` so a replay re-posts nothing (`recoverycmd.rs:701`); (5)
`create_vault(SpatialInterior(stowage), owner Object(pilot))`, the hold; (6) the enrolment document.
The hull is renamed to the pilot's choice and keeps its registry mark (FPC-D-04*nn*, the Freeport
Registry's Belt mirror). **The operate gate reads the live lease**, never the enrolment document: issuer
`Org(coop_trust)`, premises the hull, the pilot a party, the `live_recovery_lease` shape
(`recovery.rs:1000-1028`); the lease is durable money state that staff attestation, `@contract/cancel`
and the buyout already understand. A tacsim `EconomyObserver` (wired, `composition/mod.rs:312`) folds
the lease contract's Settled or Cancelled into `revoked` unless the status is `buyout_pending`, so a
staff-ended charter is seen.

**As built (IC6, IC8, IC10):** the verb runs as written. The rent is 1 minor on the Manual clock,
**kept although `offer_lease` admits zero** (IC6 (2), L3), so Q-18-27's optional-rent change was never
needed. The enrolment document is `tacsim:coop:{account}` (`COOP_PREFIX`, `tacsim/src/port/coop.rs:68`),
schema 1 (`ENROLMENT_SCHEMA`, `:71`), shape `Enrolment` (`:184`): account, pilot, programme, pack, entry,
hull, mark, laid-up name, name, lease, vault, credit account, stake, share permille, buyout, enrolled
at, status, and the counters `sales`, `margin_minor`, `shares_minor`, which the proceeds policy folds
once per settled sale (IC10, its `settled` hook keyed by pilot = seller and vault = source); since IC8
`+coop/enrol` runs `Enrolment::check()` before the write. One enrolment per gateway account, ever:
`ENROLLED_ONCE` (`coop.rs:113`). The operate gate reads the live lease (`live_charter`), never the
document.

**Two engine facts fixed in IC6.** `page_stale_contracts` pages every Active contract with no deadline
and no condition progress after 7 real days, with no lease exemption (`sweep.rs:1597-1640`;
`STALE_CONTRACT_AGE_SECS`, `contract.rs:109`): every charter would raise a level-2 page mid-programme, and
recovery and housing leases share the defect. IC6 exempts lease-shaped contracts (Collateral by the
issuer plus an Obligation), S. And `offer_lease` sets one `Attestation{Staff}` condition
(`contract_live.rs:4504-4510`), which is how staff end a charter; the observer above handles it.

**As built (IC6, narrowed IC7):** the stale-contract exemption covers only a live **lease-shaped
contract whose own escrow list is empty** (`mush-economy/src/sweep.rs:1597`, `exempt_from_stale_page`);
an Active lease-shaped contract holding escrow is money sitting still and pages like any other (IC6's
residual, closed by IC7's lead note 2b).

**The stake.** 25,000 cr, an investment: no obligation, no principal, no repayment. It is the Trust's
capital at risk and cannot be clawed back (an org has no debit authority on a pilot's unscoped account,
`api.rs:370-394`); alt-farming is bounded by one enrolment per gateway account ever and by finite
capital. No top-up in the slice (Q-18-14's alternative).

**As built (IC3 (1), IC6):** the stake posts `25000.00cr` from the Trust's scoped account, keyed
`tacsim.backer.stake/<lease contract>` (`STAKE_KEY_NAMESPACE`, `coop.rs:75`). The `tacsim.credit` mint
cap the port capital obligations run under is contributed in code per **instrument**, 1,000,000
major (`PORT_MINT_CAP_MAJOR`, `tacsim/src/port/def.rs:80`), unless an operator sets a per-instrument
value — so every future `tacsim.credit` faucet obligation shares that ceiling (IC3 (1)).

**The share (D-18-24): 250‰ of trading margin over the origin port's posted ask, paid inside the
sale.** share = 250 × max(0, (price − reference[good]) × qty) / 1000, where **reference[good] is a live
read**: the programme block names, per good, the origin ask it is measured against (`reference`:
metals → the Freeport Exchange's ask, volatiles → Occator's, structural_alloys → Kasei's,
industrial_parts → the Yard's; 200 / 100 / 400 / 400 today) and the policy reads that listing's
**current** price at the moment it builds the payout, from the same listing data the applier writes and
P-A-4 will move. Only the share permille and the buyout price freeze at signing; a frozen reference
number was the one slice decision the astronomy engine's price policy would have had to undo (once an
ask moves with position, a share billed against a stale reference stops being a margin share), so it
is not a contract term. It is
computed at contract build by a tacsim `ProceedsPolicy` as an extra `Exact` payout to the Trust's account
beside the pilot's `Exact(offer − share)`: the engine already accepts a non-party payee and exact splits
that must sum to the escrow (`contract.rs:1886-1909,1418-1458`) and settles them atomically
(`:2856-2873`); only the seam is new, because every sell path hard-codes one payout today
(`trade.rs:1023-1036,877-892`). The policy fires when the sale's **source endpoint** is an enrolled hold,
never from command arguments, so a queued sale (R3) pays exactly as a typed one. Why margin over
reference and not gross or net: gross bills a losing run (250‰ of a 21,600 outbound gross is 5,400
against a 5,390 cash margin); net needs an M-size run book because lots carry no price and quotes no
destination (`mush-inventory/src/record.rs:222-252`; `mush-market/src/record.rs:771-815`); the reference
margin is deterministic at the constant sky, computable from the exchange board before the pilot logs
off, never bills a sale below the origin's posted ask as it stands at settlement, and leaves fuel and
stores to the pilot. It can bill a run whose fuel ate the margin, which is the honest cost, and once
P-A moves asks it bills on the ask at settlement, not the ask the pilot paid (a pilot who bought cheap
and sells after the ask rose is billed less; one who bought dear and sells after it fell, more —
`trade.md` says so) — and it is a **margin share, not the "profit-sharing" of R2's words**; §1 says
so where the ruling is restated; Q-18-13 is closed under R8 and the net share with a carried deficit
is P9, once the run book exists. Every sale line prints gross, share and net.

**As built (IC10):** the reference is read **at contract build** from the origin listing's current
price, not at settlement: the `Exact` payouts must sum to the escrow when the contract is offered, so
the share is fixed then (IC10 (6)); the drift between build and settlement is at most one window
(3,600 s), recorded as a residual. When the origin ask is gone the goods table's `reference_major` is
used and the sale's memo says so (`BASELINE_MEMO`, "baseline ref", `tacsim/src/port/proceeds.rs:68`).
The share is **floored** in minor units at the enrolment's frozen permille (`share_of`, `proceeds.rs:75`;
IC10 (5), in the pilot's favour). The seam is `ProceedsPolicy` (`mush-market/src/module.rs:340`: `proceeds`
at build, `settled` after settlement) with at most `MAX_PROCEEDS_CUTS` = 1 cut beside the seller
(`mush-market/src/record.rs:976`); tacsim's implementation is `CoopProceeds` (`proceeds.rs:98`), which
also renders the sale line (`SALE_LINE`, `proceeds.rs:53`) and folds the enrolment's counters once per
settled sale.

**The buyout (D-18-25; cut IC13).** The price, 90,000 cr, is posted in the programme block, frozen into
the enrolment at signing and shown by `+coop` from day one beside the Yard's new-Dray comparator
(120,000 cr). `+coop/buyout` at C05, hull docked at Ceres, status active: (1) a synchronous funds check;
(2) the pilot issues the market's asset-purchase contract shape with `offer_contract` — `TitleHeld{hull,
Org(coop_trust)}` + `Before(window)`, deliverable `TitleTransfer Org(coop_trust) → Object(pilot)`,
consideration the price, one `Exact` payout to the Trust (`trade.rs:829-848`; the pilot is the issuer,
`contract_live.rs:4979`, so the Trust's issuer cap is untouched); (3) `accept_contract` as the pilot
escrows the price; (4) `accept_contract_as(Org(coop_trust))` through the party-cap source (IC7); (5)
`end_lease` in the same command, before the sweep settles (`sweep.rs:1425-1430`), because invariant 12
refuses an encumbered transfer and settlement would go Disputed after eight attempts
(`contract.rs:90-92`); on `Retrying` the verb cancels its own contract and refuses; (6) the observer
marks Settled → `graduated` (the share hook stops; the gate reads title) and Cancelled or Expired →
re-offer the lease. 10b:1753 designed exactly this declared-price lease buyout and never built it.

**As built:** not built; IC13 is open. `+coop` already shows the buyout and the Yard's comparator
(`coop.rs:745-754`).

**Losses, dormancy and release.** A sale at or under reference pays the Trust nothing; the pilot's cash
bears operating costs. Dormancy: seven real days without a departure, judged by the tacsim tick; the
lease ends when the hull is docked at Ceres with an empty cargo pool, otherwise the queue is held and
staff paged (no confiscation of matter in the slice). Staff `@coop/revoke <pilot>=<why>` is attributed;
`+coop/release` at Ceres is voluntary and keeps the stake. All three land in IC13; before it a dormant
charter simply stays. Re-enrolment is refused for ever by default (Q-18-14's 30-day cooldown is the
alternative). Hull damage is out of scope; Tycho underwrites Trust hulls later (11 §4.7).

**As built:** a sale at or under the reference pays the Trust nothing (`share_of` floors a negative
margin to zero). Dormancy, `@coop/revoke` and `+coop/release` are IC13's and not built; a staff
`@contract/cancel` of the lease is seen by the observer and ends the charter (IC6), and a revoked hull
stays out of the free pool until IC13 re-homes its hold (IC6 (3)).

**What refuses when broke.** `+market/accept` with insufficient available funds answers the escrow
refusal; a queued buy clamps to funds and logs the shortfall; a queued depart short of fuel or stores
holds with nothing burned; `+coop/enrol` twice is refused; nothing is minted on demand.

**Numbers (Draft 0.3, R7's loads).** Per the §2 money check: a full outbound (20 t concentrate + 80 t
ice, both hold limits full) nets the pilot about 5,390 cr cash (21,600 − 2,400 share − 12,000 cargo −
1,600 fuel − 210 stores), a full return (55 t alloy stock + 45 t crated fittings) about 7,740 (53,800 −
3,450 − 40,000 − 2,400 − 210), 13,130 a round trip; the first return is cash-bound to 69 t (49 + 20) and
the second to 96 t (45 + 51). Simulated at one leg a day with a 20,000 float kept at Ceres and the captain
striking the best mix each leg: full loads cross 90,000 on day 13 and the buyout plus the float on
**day 15** with about 4,000 to spare ("a couple of weeks of good luck"); 70 % loads on day 23; the Trust
receives about 40,000 in shares plus the 90,000 sale against its 25,000 stake and the hull. With the
fittings bid at 540 instead of 560, or with no fittings at all, day 17. At 75,000 the buyout lands
about day 12, close to trivial; at 60,000 about day 9. Programme capital 8 × 25,000 + a 50,000 floor =
250,000 cr. All of it at the constant sky (§4.8).

### 4.2 The ship

**Object model (D-18-22; replaces D-18-8).** One hull Thing is the carrier root (`sim.participant`,
Direct on the port domain; its MUSH owner is the content owner, never the pilot, so `perm::controls`
cannot bypass the port verbs, `mush-sim/src/commands.rs:2606-2610`). It carries `tactics.unit` bound
from `tacsim.unit.dray`; the slice reads nothing from the unit but `logistics` and `interior`. **Seven
Rooms ride it**: six walkable members and one store. ONE interior-space record governs the six:
`space:<airlock>` = `{object: airlock, anchor: MovingObject{host: hull}, compartments: {berths:
{maximum_items: 2}}, rooms: {airlock, passage, bridge, crew, hold, engineering}, egresses: {}}`. The
airlock is the space object (it carries `sim.interior`; `project()` falls back to it for anyone placed
from outside, so entrants arrive through the lock) and crew are placed `Interior{space: airlock, slot:
berths}`. A second record, `space:<stowage>`, anchored to the hull with `rooms {itself}`, is a Room with
no exits and backs the ONE hold vault (`CustodyRef::SpatialInterior(stowage)`): Draft 0.1's bay
principle kept, so `interior_contents` (`custody.rs:76-105`) never counts a person as custody and
`arrival_parent`'s rule (`commands.rs:2224-2245`) holds. The six member rooms are never vault backings
(`backing_valid` answers false for a multi-room space). Title is an `EconomicAsset` keyed by the hull,
issued at pack apply to the Trust under `tacsim.registry`.

**As built (IC4, IC6):** as written, with one addition: a chartered hull Thing with no description
takes `HULL_DESC_TEMPLATE` ("A Dray-class bulk hauler, {mark}, cradled with its umbilicals slack: blunt,
grey and webbed with reaction-mass hoses.", `tacsim/src/port/coop.rs:102`; IC6 L10).

**The boundary rule (cut IC4, mush-sim, S: about 150 lines plus tests).** (a) The move hook
(`lib.rs:293-303`) gets a destination-aware reason: an Interior-placed actor may move only through an
exit (`exit.is_some()`, so `@teleport` and `home` stay refused, `building.rs:599`, `movement.rs:228`) to a
room in its own space's `rooms`; the held, quarantined and journeying arms still win first. (b)
`project()` (`graph.rs:167`) returns the occupant's current location when it is a member, otherwise the
space object — without this, boot reconcile (`boot.rs:604-624`) would teleport crew back to the airlock
on every restart. (c) `Transition::Members{space, rooms}`, a pure plan: every member a live Room; the
space itself a member; no room a member of another space, a space, a domain host or a presentation room;
at most 16; no occupied member removed; persisted through `set_spaces`; a derived `member_of` index
checked by `verify_indexes`; the destroy gate refuses destroying a member; `Transition::Space` refuses to
make a member a space; invalid members are dropped in RAM at boot and paged (fail closed into the
airlock). (d) Guards: `InteriorCustodyProvider::backing_valid` false for a multi-room space
(`custody.rs:149-156`); `egress` (alias `disembark`, `commands.rs:3407-3408`) refuses in a multi-room
space with "use +ship/leave", because it strands the actor on a passive domain. (e) Leg-boundary and
arrival narration loop over `space.rooms` for every space anchored to the carrier. **Not built, because
nothing in the slice needs it:** a destination seal (no exit leads into a member room; staff `@teleport`
is recovery), a seat-reach re-check (no stations), a BerthLink and gangway exits (boarding is a verb), a
Moved observer. Walking writes no placement, the carrier seal is untouched and keeps the occupant set
fixed under way, and the existing compartment cap enforces two berths (`transition.rs:343-362`).

**As built (IC4, IC8, IC9):** as written; the boundary's refusal is mush-sim's line "{who} is aboard
{host}; it moves only between {host}'s rooms, or leaves through the simulation or staff recovery
(@spatial)." (`mush-sim/src/lib.rs:333`), asserted for `o`, `s` and `home` by the story test's negative
checks. The hull template's **`interior.exits` are append-only by position** (`tacsim/src/port/hull.rs:22,
438`; IC4's residual, IC8's rule): an exit's position is its identity across a re-dress. IC9 adds the
apply-time refusal of a pack exit **into** a room aboard an anchored interior (`mush_sim::anchored_space_of`);
an exit out of one applies (IC12's T10, `mush-atlas/tests/anchored_exits.rs`).

**The airlock and the berth (D-18-34; D-18-18 kept).** Boarding is by verb, not by a gangway exit.
`+ship/board`, typed in a port's Visiting Berth Gallery, for the lease tenant of a hull Direct on that
port's domain and not under way: `place_interior(airlock space, Some(berths))` (`window.rs:324-341`), and
the projection shows the pilot in the Airlock. `+ship/leave`, in the Airlock, docked: `unplace(keep)` +
`move_to(gallery)`, emitting Moved (the arena `put_out` shape, `cockpit.rs:2401-2418`). "Only the airlock
gains a berth connection" holds because both verbs work only at that pair of rooms. Under way nothing
crosses: the carrier seal refuses placements both ways; the boundary rule refuses every non-member
destination, the global Limbo exits included (`movement.rs:24-42`); `+ship/leave` refuses. Real gangway
exits (a permanent outer-door exit relinked at dock, a station exit named by berth and hull, a
`BerthLink` record, a Moved observer filtered to Player/Npc, atlas `on_moved` ordering tests, boot
reconcile: an M cut) are post-loop cut **P-G** and Q-18-18; the verbs cost about forty lines and give the
same seal. No room in the slice carries an exit aliased `board` or `disembark`.

**As built (IC6 (1)):** `+ship/board` is gated on the live lease **and** the hull's placement;
`+ship/leave` on the placement alone (aboard, in the Airlock, docked), so a pilot whose charter ends
aboard can always step ashore — a proposed amendment to D-18-34's "both gated on the lease" (§7, §8).
While a clearance is filed, or burned but held, neither verb works (`LEAVE_CLEARED`, IC9 (3)). The
third soul refused at the berths slot reads mush-sim's "compartment `berths` is full (2 max)"
(`mush-sim/src/transition.rs:63`).

**Compartments.** Occupancy is the mush-sim `berths` slot, `maximum_items 2`, enforced at placement; in
the slice only the lease tenant may board, so the second berth is reserved for a passenger (Q-18-17).
Matter is the one hold vault with every quantity in **pools**, each compartment limited by doc 06 D3's
measure map (D-18-28 as replaced by R7): cargo (Σ `stow.bulk` goods ≤ 100,000 kg **and** ≤ 120,000 L of
packed volume), the tank (≤ 80,000 kg; tank-rated, mass only), the locker (≤ 480 kg and 1,200 L, which is
120 crew-days at 4 kg and 10 L), from `logistics.stowage`. Every delivery into a hull endpoint is a pool
credit — `LotOp::Draw` is a `transfer` (`pool.rs:413-528`) — so no loose-lot census is needed, the burn
debits pools directly (`pool.rs:306`), and the in-transaction load figure is one point lookup per good
the compartment accepts (four for the hold, one each for the tank and the locker), read by balance (hull
pools hold no reservations). **One pure tacsim function** (`port/capacity.rs`: `load`, `admits`,
`max_more`, §4.3) does the arithmetic everywhere it is needed: synchronously in the market lot leg's
pre-check through `Ctx::action_state` (`trade.rs:572`; refuse, nothing charged), as a backstop inside the
Draw prepare (fail closed on an unreadable pool), in `+ship/plan` and `+ship/hold`, in the room gauges,
and in the queue's buy clamp and `fill` (§4.4). The venue gate refuses any non-port listing that would
deliver into a hull endpoint, so no lot ever lands there. What the pilot sees of the compartments is a
`look_rendered` line per pool in the rooms `interior.gauges` lists (18a §3): the load cells in the
Cargo Hold, the tank gauges in Engineering Access, the locker tally in the Crew Compartment, and all
three on the Central Passage's manifest slate and the Bridge's displays, every figure rendered from the
pools at look time and none written into pack text (D-18-39: a hull's `details` never carry a load),
plus `+ship/hold`.

**Propellant and provisions as consumables (D-18-11 kept, extended).** Both are pools in the hold, bought
like cargo and spent by the burn. The burn is settled at undock, matter first, by ONE authoritative
`tacsim.port.burn` action submitted as the pilot (by `+ship/undock` live, or by the queue's depart step
offline): the command or tick computes the **passage plan** first (`plan_passage`, §4.4/§4.8) and passes
it as the action's input; the prepare recomputes payload from transaction quantities, recomputes the
burn and the crew-days from the plan's per-leg seconds and the drive table the provider holds (the
`MarketLotProvider` shape, families cloned at composition, `lot.rs:199-203`), refuses on mismatch,
`Overloaded`, tank < burn or locker < crew-days, debits the two pools, and writes the typed ticket
`{hull, route, route_pack, definition digest, manifest{cargo, tank, locker, occupants}, payload_kg, plan
{route, law, depart_ic, sky_rev, legs[{to, body, dist_km, secs, t0, t1}], total_s, burn_t, crew_days},
order, attempt, burned_at, launched: false}` in the same bundle. Only the sky-dependent seconds are taken
on trust from the plan, which is exactly T1's freeze; nothing after the burn re-reads the sky. The launch
is the tacsim tick's arm consuming the ticket through `pub mush_sim::depart(ctx, hull, route, leg_s)`
with `leg_s = plan.legs[].secs`; never an `ActionStateChanged` persist
(`engine.rs:2939-2942`), never a GOD-tick submission (`tick.rs:28-37`); boot reconcile launches
stragglers once. A refused departure consumes nothing. **Crew-days burned = occupants × Σ over the
route's legs of ceil(laden leg IC days)** — the one rule, used by `+ship/plan`, the burn and the
queue's `fill`: the full outbound is 12 + 9 = 21 (not ceil(19.6) = 20), the 69 t first return 10 + 8 =
18, a 100 t return 12 + 9 = 21.

**As built (IC9, IC11):** the ticket wire **gained `seq` and lost `launched`** — whether a ticket launched
is read from the journey, never stored (`Ticket`, `tacsim/src/port/families.rs:202`); the ticket id is
`{hull, seq}`, and the `seq` is the replay guard. The burn is the action `tacsim_port_burn` with outcome
`tacsim_port_burn_result`, keyed `tacsim.port.burn/{hull}/undock/{seq}` for a live undock and
`tacsim.port.burn/{hull}/{order}/{attempt}` for a queued depart (`tacsim/src/port/burn.rs:52-58`). The plan
does not travel as the action's input: the filed passage is the per-hull sailing-plan document
`tacsim:voyage:<hull>` (`Voyage`/`PendingVoyage`, `tacsim/src/port/voyage.rs:104`), read into the prepare as
a server-authenticated fixed parameter, so the submission names only the `seq` (IC9's lead ruling). The
launch is the tacsim tick calling `mush_sim::depart` directly at `plan.depart_ic` with the frozen leg
seconds (`mush-sim/src/commands.rs:2793`) — no submission at all, so D-18-11's "never a GOD-tick
submission" holds trivially. mush-sim's generic aboard lines are heard **beside** tacsim's flavoured ones
(`REPLACE_GENERIC_ABOARD_LINES = false`, `tacsim/src/composition/sim_routes.rs:47`; IC9 (1)), through
`CarrierHooks` (`refuse_manual_departure`, `boundary`, `arrived`; `mush-sim/src/journey.rs:592`).

**The helm.** There is no station (D-18-23). `+ship/plan`, `+ship/undock` and the queue are usable from
anywhere aboard; the Bridge carries the readouts and the status line. P2's helm station, `request_seat
_activation` and the `movement` channel return when a second crew member can hold the helm; under R3 a
seat lease that lapses 300 s after a disconnect (`seat.rs:736-770`) could never be the departure gate.

**As built (IC6 L18, IC9, IC11):** the status line shows in **every** hull room on `look`, with the
registry mark, in five forms (`tacsim/src/port/ship.rs:85-100`): `STATUS_DOCKED` "Aboard the {name}
({mark}), docked at {port}."; `STATUS_UNDER_WAY` "…, under way, leg {n} of {m}{astern}, ETA {hhmm} UTC.";
`STATUS_CLEARED` "…, cleared for {port}."; `STATUS_DOCKED_DEPARTS` "…, docked at {port}; departs {hhmm}
UTC (in {span})."; `STATUS_ELSEWHERE` "Aboard the {name} ({mark})." The gauge lines follow in the rooms
`interior.gauges` lists; the Engineering/Bridge burn-and-reserve gauge lines are not built (IC11 L21;
unowned, §5).

**The Dray definition (D-18-29).** `crates/tacsim/content/units/dray.json`, in `fielded_sources` with a
digest pin. Everything below is valid against today's schema except `class: "hauler"`, `logistics` and
`interior`, added as optional fields with `default` + `skip_serializing_if` so no shipped `DefinitionRef`
moves (`unit.rs:822-847`). The pasted scale replaces Draft 0.1's 2,800 t hold.

**As built (IC2, IC3, IC8):** `tacsim.pack.dray` ships at **version 2** (IC8): the cruise is
`kms` **226** (IC2's lead ruling, built in IC3: the real date 2351-05-13 at 205 km/s missed R3's "under
a day"), the Airlock's alias `lock` is **dropped** (it shadowed the cockpit verb LOCK in the Central
Passage; exits match before commands), and every room's prose is figure-free (no load, no engine word;
the figures are `look_rendered` gauges from `logistics`). The JSON below is Draft 0.3's and keeps 205
and `lock`.

```json
{
  "manifest": { "id": "tacsim.pack.dray", "version": 1, "kind": "rules_definition",
                "digest": "<computed by crates/tacsim/tests/content.rs; pinned there>", "requires": [] },
  "entries": {
    "tacsim.unit.dray": {
      "kind": "unit",
      "def": {
        "label": "Dray-class bulk hauler",
        "class": "hauler",
        "sections": [
          { "id": "hull", "facets": [ { "id": "front", "armor": 120 }, { "id": "left", "armor": 80 },
              { "id": "right", "armor": 80 }, { "id": "rear", "armor": 60 }, { "id": "top", "armor": 60 } ],
            "internals": 600, "consequences": [ { "kind": "destroy_unit" } ] },
          { "id": "hold", "facets": [ { "id": "left", "armor": 60 }, { "id": "right", "armor": 60 },
              { "id": "top", "armor": 50 } ],
            "internals": 400, "consequences": [ { "kind": "slow", "percent": 75 } ] },
          { "id": "drive", "facets": [], "struck_through": { "section": "hull", "facet": "rear" },
            "internals": 250, "consequences": [ { "kind": "disable_motion" } ] }
        ],
        "components": [
          { "id": "power", "section": "hull",
            "role": { "kind": "power", "capability": "tacsim.power",
                      "states": [ "off", "starting", "online" ], "initial": "off" } }
        ],
        "mobility": { "tracked": { "max_speed_cms": 0, "accel_cms2": 100, "brake_cms2": 200,
                                   "turn_rate_mdeg_s": 1000, "reverse_factor_pct": 0 } },
        "facets": [
          { "band": "front", "sections": [ { "section": "hull", "weight": 100 } ] },
          { "band": "left",  "sections": [ { "section": "hull", "weight": 50 }, { "section": "hold", "weight": 50 } ] },
          { "band": "right", "sections": [ { "section": "hull", "weight": 50 }, { "section": "hold", "weight": 50 } ] },
          { "band": "rear",  "sections": [ { "section": "hull", "weight": 50 }, { "section": "drive", "weight": 50 } ] },
          { "band": "top",   "sections": [ { "section": "hull", "weight": 40 }, { "section": "hold", "weight": 60 } ] }
        ],
        "silhouettes": [
          { "band": "front", "width_cm": 1800, "height_cm": 1600 },
          { "band": "left",  "width_cm": 6000, "height_cm": 1600 },
          { "band": "right", "width_cm": 6000, "height_cm": 1600 },
          { "band": "rear",  "width_cm": 1800, "height_cm": 1600 },
          { "band": "top",   "width_cm": 6000, "height_cm": 1800 }
        ],
        "sensor_height_cm": 1500,
        "logistics": {
          "dry_mass_t": 240,
          "stowage": {
            "cargo":      { "accepts": [ "stow.bulk" ],       "limits": { "si.mass_kg": 100000, "si.volume_l": 120000 } },
            "propellant": { "accepts": [ "stow.propellant" ], "limits": { "si.mass_kg": 80000 } },
            "provisions": { "accepts": [ "stow.provisions" ], "limits": { "si.mass_kg": 480, "si.volume_l": 1200 } }
          },
          "berths": 2,
          "occupant_allowance_kg": 100,
          "payload_ceiling_kg": 181000,
          "drive": { "class": "coast_hauler", "law": { "coast": { "kms": 205, "burn_permille": 105 } } }
        },
        "interior": {
          "space": "airlock",
          "store": "stowage",
          "berths_slot": { "id": "berths", "maximum_items": 2 },
          "rooms": [
            { "id": "airlock",     "name": "Airlock",            "kind": "walkable" },
            { "id": "passage",     "name": "Central Passage",    "kind": "walkable" },
            { "id": "bridge",      "name": "Bridge",             "kind": "walkable" },
            { "id": "crew",        "name": "Crew Compartment",   "kind": "walkable" },
            { "id": "hold",        "name": "Cargo Hold",         "kind": "walkable" },
            { "id": "engineering", "name": "Engineering Access", "kind": "walkable" },
            { "id": "stowage",     "name": "Hold Stowage",       "kind": "store" }
          ],
          "exits": [
            { "from": "airlock", "to": "passage",     "names": [ "Inner Hatch <I>;inner;in;passage", "Airlock <A>;airlock;lock;forward" ] },
            { "from": "passage", "to": "bridge",      "names": [ "Bridge <U>;bridge;up;u", "Passage <D>;passage;down;d" ] },
            { "from": "passage", "to": "crew",        "names": [ "Crew Compartment <C>;crew;galley;bunks;aft", "Passage <P>;passage;out;forward" ] },
            { "from": "passage", "to": "hold",        "names": [ "Cargo Hold <H>;hold;down;d", "Passage <U>;passage;up;u" ] },
            { "from": "hold",    "to": "engineering", "names": [ "Engineering <E>;engineering;eng;down;d", "Hold <H>;hold;up;u" ] }
          ],
          "gauges": { "cargo":      [ "hold", "passage", "bridge" ],
                      "propellant": [ "engineering", "passage", "bridge" ],
                      "provisions": [ "crew", "passage", "bridge" ] }
        },
        "note": "Stickney-built hauler (doc 11 section 7): slow, efficient, unarmed, Class 0 (no core: the drive ROM of doc 17 section 9; nothing separately titled). The port verbs read logistics and interior only; the combat anatomy is authored for later and never fielded (class hauler is kept out of SIM UNIT; tracked max_speed 0 is the immobile stand-in). Room descs are 18a section 3's; the airlock is FORWARD of the passage and the crew compartment AFT, and this JSON is the authority 18a section 3.1 matches alias for alias. The hold binds on 100 t AND 120 m3 (R7); stowage limits are doc 06 D3's measure map (a later measure is a data key). drive.law is an externally tagged enum, coast {kms, burn_permille} in the slice: kms is the UNLADEN cruise and burn_t = ceil((dry_mass_t + payload_t) * burn_permille / 1000) per departure is doctrine, distance-free; durations come from plan_passage, never from an authored number. payload_ceiling_kg is route_duration's rail and laden denominator, >= 180,680."
      }
    }
  }
}
```

**Numbers.** Dry 240 t; cargo 100 t and 120 m³; tank 80 t; locker 120 crew-days at 4 kg and 10 L each;
two berths at 100 kg; payload ceiling 181,000 kg, a rail at or above the stowage maxima plus occupants
(180,680 kg) that never refuses on its own; `coast_hauler` cruising at 205 km/s with a burn of 105‰ of
departure mass per departure. Full outbound (20 t concentrate + 80 t ice = 100 t / 120.0 m³, 42 t
propellant, 21 crew-days, one person): payload 142,184 kg, burn 41 t, legs 965,623 + 730,645 IC s =
1,696,268 (19.63 IC days, 19 h 38 m at rate 24). First return at 69 t (49 t fittings + 20 t alloys,
118.7 m³; 37 t aboard, 19 crew-days): 106,176 kg, burn 37 t, 1,507,276 IC s (17 h 27 m). A full return
(55 + 45): 140,184 kg with 40 t and 21 crew-days, burn 40 t, 19 h 31 m. Volume-bound loads are lighter
and fly faster (85 t of ice alone: 18.5 h), so the capacity function keeps mass and volume separate and
never collapses them into one "fullness". Per hull: 1 Thing, 7 Rooms, 10 Exits (5 pairs; the applier
opens `forward` and `back` as two objects, `apply.rs:1500-1512`), 2 space records: 18 objects, so eight
hulls make 144, far under `MAX_CARRIER_SUBTREE` 256 per hull. The JSON `note` is 991 bytes against
`MAX_NOTE_BYTES` 1,024 (`definitions/mod.rs:82`; `UnitSpec::check` calls `check_note`, `unit.rs:938-940`),
and IC2's pin decodes `dray.json` through `UnitSpec::check` so an over-long note fails the cut, never
the boot; the R7/R9 rationale lives in this prose, not the note. Every passage figure is the constant
sky's output at its epoch (§4.8), never an invariant.

**As built (IC9 (7)):** at 226 km/s on lane v3 the full outbound (42 t aboard, 21 crew-days) flies
959,361 + 735,600 = 1,694,961 IC s (19.6 IC days, about 19.6 h at rate 24) with a 41 t burn; the 69 t
first return 653,642 + 852,473 = 1,506,115 IC s (about 17.4 h) with a 37 t burn and 18 crew-days. The
laden step is FLOOR (`route_duration`'s exact arithmetic), not the ceiling Draft 0.3 implies.

**Schema additions.** (1) `UnitClass::Hauler`, excluded from arena class caps; `fieldable_ids` filters
it (`lobby.rs:1600-1619`) with a custody-seam refusal test. (2) `UnitSpec.logistics` and
`UnitSpec.interior` (optional), read by tacsim; `bind()` ignores both. (3) `tacsim.registry` title issuer.
(4) In mush-sim: `Transition::Members`, `member_of`, the gate and projection arms, `pub
declare_compartment` (the `@spatial/slot` write without the parser, `commands.rs:1029-1068`). Not added:
a thrust mobility mode, `ComponentRole` drive/hold/tank (Q-18-5's alternative), stations (P2), a
`BerthLink` (P-G).

### 4.3 Goods

**The resource set (D-18-13 amended by R7).** Six sale items, five from the ten families and one
aggregate: the sixth, `industrial_parts` as **yard fittings, crated**, is the bulky Stickney export that
makes the return leg a balance too (D-18-40; Q-18-32). Units are tonnes for bulk goods and reaction mass
and whole crew-days for provisions (D-18-28): every listing and quote is capped at `MAX_QUANTITY` 100,000
units (`mush-market/src/record.rs:90-91`), which is 100,000 t, so the cap is untouched and every price is
exact in minor units per tonne; a kg ledger would cap a listing at 100 t. **Packed volume is binding**,
chosen so each leg has one dense and one bulky good and the load that fills both limits falls on whole
tonnes (the integrality rule: (V − M·v_dense)/(v_bulky − v_dense) is an integer).

| Good (inventory id) | Family | Unit | Per unit (binding) | Stowage | Role | Ceres | Stickney |
|---|---|---|---|---|---|---|---|
| `metals` metal concentrate | metals | t | 1,000 kg; **400 L** (binned, ~2.5 t/m³: weighs out first) | `stow.bulk` | Ceres export (doc 10 §11), Kasei's refinery input | **Freeport Exchange** ask 200.00cr/t | **Kasei** bid **280.00cr/t** (was 300) |
| `volatiles` water ice | volatiles | t | 1,000 kg; **1,400 L** (1.09 m³/t solid plus insulated canister: cubes out first) | `stow.bulk` | Occator water to Phobos (11 §3.2) | **Occator** ask 100.00cr/t | **Kasei** bid 200.00cr/t |
| `structural_alloys` structural alloy stock | structural_alloys | t | 1,000 kg; **300 L** (banded bar and plate) | `stow.bulk` | Kasei's output (11 §4.3), the dense return cargo | **Freeport Works** bid 520.00cr/t | **Kasei** ask 400.00cr/t |
| `industrial_parts` yard fittings, crated | industrial_parts | t | 1,000 kg; **2,300 L** (pressure fittings, ducting, tank sections for the Co-op fleet and the Kirkwood tank farm, crated with dunnage, ~0.43 t/m³) | `stow.bulk` | the bulky return cargo (new) | **Co-op** bid 560.00cr/t | **Stickney Yards** ask 400.00cr/t |
| `propellant` reaction mass | propellant | t | 1,000 kg; 1,000 L (tank-rated: the tank limits mass only, so never summed) | `stow.propellant` (tank only) | burned at departure | **Occator** ask 40.00cr/t | **Kasei** ask 60.00cr/t |
| `provisions` voyage provisions | none (class `ship_supply`) | crew_day | 4 kg; 10 L (ten crew-days to the case, 18a C08) | `stow.provisions` | `occupants × ceil(days)` burned per leg | **Co-op chandlery** ask 10.00cr | **Yard Commissary** ask 10.00cr |

Provisions stay an opaque inventory commodity with no instrument; `FAMILIES` stays ten
(`tacsim/src/definitions/resources.rs:34-45,94-104`); Q-18-3 is binding. **The price chain, read
honestly.** At Ceres reaction mass (40) is cheaper than water ice (100): Occator's reaction mass is
the bulk, crude fraction of the same extraction and the refined water is the product Phobos pays for;
18a's lore line says so, never that reaction mass "costs more than the water". At Stickney Kasei bids
200 for ice and asks 60 for reaction mass, which no refining chain explains (electrolysis conserves
mass): the ice bid is **Phobos's demand for volatiles** — the refinery's process water, the habitat and
the Republic's reserve (doc 11 §3.2 wants the volatiles; "Phobos needs volatiles", §1) — and Kasei's
reaction-mass ask is a **Republic-posted price** (the "Republic prices" of 11 §4.3's refinery, a
strategic asset the Republic keeps fuelled for the Guard and the Yard), dearer than Occator's crude
and unrelated to the ice bid. Kasei's capital (600,000) carries the difference; the economist says
the Stickney prices, not the chain. **One price moves under R7**: Kasei's concentrate bid falls 300 →
280 because Draft 0.2's margins tied at 100/t on both goods and a tie is no balance (Q-18-33); the lore
is that Kasei's refinery is water-starved, so it bids concentrate down and pays the same 200 for ice.
The reference for the share stays the Ceres ask, read live at settlement (200 today). 40/60/100/200 are unchanged.

**Why you carry both (the captain's arithmetic).** Each leg is a small linear programme: earn Σ margin
× tonnes, under Σ tonnes ≤ 100, Σ packed volume ≤ 120 m³ and, early on, Σ cost ≤ cash. Outbound the
concentrate earns 80/t and the ice 100/t (bid − ask), but per cubic metre the concentrate earns 200 and
the ice 71: the bulky good pays more per tonne, the dense good more per cube, so neither alone is best
and the load where **both** limits bind wins. At 400 and 1,400 L/t that is 20 t concentrate + 80 t ice
= 100 t and 120.0 m³ exactly, 9,600 gross margin against 8,500 for 85 t of ice alone (volume-bound),
8,000 for 100 t of concentrate (mass-bound) and 8,600 for Draft 0.2's 70/30. Return: alloys 120/t
(400/m³), fittings 160/t (70/m³): 55 t alloys + 45 t fittings, 13,800 against 12,000 all-alloys. The
rule holds whenever 1 < margin_bulky/margin_dense < volume_bulky/volume_dense (outbound 1.25 inside
1–3.5; return 1.33 inside 1–7.67), a wide band: the outbound vertex survives a concentrate bid from 229
to 300 or an ice bid from 180 to 380, the return a fittings bid from 520 to 1,320 or an alloys bid from
about 421 to 560. When cash binds a third limit enters and per credit the bulky good earns more (0.40
against 0.30), so a short captain buys fittings first, up to the volume the affordable tonnage leaves,
then alloys: 49 + 20 on day 2, 45 + 51 on day 4. The executor clamps each order in turn and never
solves across orders (Q-18-34): the balance is the captain's skill, `trade.md` explains it, and
`+ship/plan` shows the room by mass and by volume per good and each queued buy's bound, never a
recommended mix. **No code, fixture, NPC line or test stores 20/80 or 55/45** (D-18-39): IC8's LP test
derives both vertices from `goods.json`, the hull and the posted port prices and asserts them optimal,
so when the astronomy engine moves prices the optimum moves inside the band or out of it — "optimal
last month, less optimal now" is the play, not a broken constant.

**The goods table.** All six live in one tacsim-owned compiled-in document,
`crates/tacsim/content/market/goods.json` (`deny_unknown_fields`, version 1, a **new** digest pin —
`resources.json` is only decode-tested, `tests/content.rs:313-322`): a top-level `measures` registry
(`si.mass_kg` → kg, `si.volume_l` → L: kg/L rather than D3's g example or Q-18-21's mL, by choice,
because every slice figure is a whole kilogram or litre), then per row id, label, family or null, class,
ledger unit, quote unit, `measures: {"si.mass_kg": 1000, "si.volume_l": 400}` (D3's `LoadProfile`
shape, so a later measure — reefer slots, hazmat — is a data key on both sides; `mass_kg_per_unit` is a
derived accessor for the burn and payload arithmetic), stowage tag, consumable role, a reference baseline,
a note. It is the one source for the market `CatalogEntry` rows (derived, representation lot, loaded by
the port applier through `api::load_pack`) and for the capacity check. Physics never goes into
`CommodityDef` (label/in_use, in the test-only `tacsim.core` fixture, dropped by `Catalog::build`) or
into the market catalog wire, which silently drops unknown fields (`record.rs:328-348`). **The
validator** (IC2): every limit key is a registered measure; every good whose stowage tag a compartment
accepts carries every measure that compartment limits (a `stow.bulk` row without volume refuses); where
a compartment accepts one tag and limits two measures (the locker) the limits agree on the unit count;
limits are positive and the largest stock × per-unit fits u64.

**The capacity function (D-18-28 replaced).** `crates/tacsim/src/port/capacity.rs`, pure, no World:
`Compartment {id, accepts, limits: BTreeMap<SmolStr, u64>}`; `load(goods, compartment, contents) ->
Load` (a measure → sum map, checked u64); `admits(goods, compartment, contents, (good, qty)) ->
Admitted(Load) | Refused(CapacityRefusal {compartment, bound: Vec<measure>, fits, room}) |
Error(UnknownGood | NotAccepted | MissingMeasure | Overflow)` (one enum, so no caller can `?` past a
refusal); `max_more(goods, compartment, contents, good) -> (units, bound)` = the minimum over the
compartment's limits of floor((limit − load)/per_unit), naming every measure that attains it. The
market hook (IC7) cannot name tacsim: `trait CapacityProvider { fn check(&self, endpoint, commodity,
qty, read_pool: &mut dyn FnMut(&CanonicalId) -> Result<u64, CapacityReadError>) ->
Option<Result<(), CapacityRefused {fits, line}>> }`, `None` for an ungoverned endpoint; `read_pool` is
backed by `Ctx::action_state` in the pre-check (IC0's `hold_standing`, `lot.rs:775-800`) and by
`Pools::pool(txn, ..)` in the Draw prepare (absent = 0, `Err` refuses: reads are delegation-gated,
`action_state.rs:2043-2068`). A Draw refused at its prepare after the pre-check passed (two typed
accepts staged before the first applies) cancels its contract and returns the escrow with no page — the
IC0 Lot short-circuit's rule carried to the Draw (IC7's verify). The tacsim implementation (IC8) maps
endpoint → hull → stowage → `compartment_for` → `admits`. UI: whole tonnes; m³ to one decimal,
**truncated**, never rounded up (every cargo volume is a multiple of 100 L, so it is exact; a formatter
test pins it); player text says "packed volume" or "m³", never "volume" alone (mush-market's "volume" is
a maker's trade budget, `record.rs:1117`). Refusals read, with nothing charged: "86 t of water ice packs
to 120.4 m³; the hold takes 120 m³. 85 t fits." · "The hold has 1 t of room by mass and 0.4 m³ by
volume: no more water ice fits." · "101 t of structural alloy stock weighs past the hold's 100 t rating
(30.3 m³). 100 t fits."

**As built (IC7, IC8):** the market seam is two calls, not one `check`: `CapacityProvider::plan(&World,
endpoint) -> Option<CapacityPlan>` at accept, `None` for an ungoverned endpoint, and
`check(&plan, commodity, qty, read_pool)` (`mush-market/src/module.rs:223`), the plan carried in the
action's fixed parameters (at most 2 KiB) so the prepare re-checks the same compartments. **A Draw refused
at its prepare is not cancelled**: its contract carries a `market.lot.drawn` Fact and expires with
`Expiry::Refund` at `DRAW_WINDOW_SECS` = 3,600 s, re-stage allowed up to a 600 s cutoff
(`mush-market/src/trade.rs:73,114`; IC7 (1)) — the engine can neither cancel an Active contract nor learn
of a prepare refusal. The refusal lines are `capacity_line`'s (`tacsim/src/port/market.rs:498`).

**Finite seeded stock (D-18-26; replaces D-18-15; R4).** Every unit of matter is minted once, before any
sale, into a storefront pool, with a receipt. **The mint action** is tacsim's first action provider,
`tacsim_port_stock`, one op: refuse if the receipt exists; otherwise `mint_origin(storefront vault, good,
qty, grade 50, ExternalRef{tacsim, port.stock, <pack>/<entry>/<org>/<good>, revision: issue}, LotId
derived from the issue id)` (`pool.rs:1280`; `Lineage::Origin` is documented for "a seeded fixture",
`record.rs:228-233`) and insert the receipt in the same bundle. **The receipt** is a typed family
`tacsim.port_stock`, `StockIssueId = hash(pack, entry, org, good, issue)`; it is the durable replay guard
(idempotency keys do not survive a reboot, `provider.rs:394-400`), and the tranche's absorbed set is only
a cross-check. **The pool step** is the shipped `inv.dissolve` (`provider.rs:762-772`), submitted as a
second bundle: a key mutates once per `ActionTxn` and a tranche must absorb a lot (`record.rs:619-624`).
**The driver** is the stock arm of the tacsim normal-lane tick (IC5): at most one step per vendor agent
per fire, acting as that agent, a dedicated per-port stock-clerk Thing holding an officer grant with
`vault.withdraw` in each vendor scope (so `live_endpoints` stays small, `provider.rs:177-208`), never GOD
(`sim.domain_pulse` already submits as GOD) and never the applier (the boot pass is one GOD drain,
`engine.rs:2285-2299`). The applier writes plan rows, vaults, vendors, grants and listings and submits
nothing. A pack update re-prices in place and never re-issues; **under Q-18-19's default a bumped
`issue` on an already-receipted stock row is refused by the applier and paged as drift** (R4: nothing
from thin air; a receipted staff top-up is Q-18-19's alternative, and only an owner ruling for it turns
the refusal into a new issue). The market-never-mints pin stays true (it scans only mush-market files,
`e7r2.rs:310-333`).

**As built (IC8):** as written, one clerk per port (IC8 L2). **Q-18-19 is taken stricter than its
default** (IC8 (2)): a port update refuses **any** change of issue, quantity or grade on a recorded stock
row, and any removal, as drift (paged), receipted or not — the applier cannot read receipts from the
content context, so it cannot tell a minted row from an unminted one; a top-up is a new issue number.
The receipt's golden id is pinned (`StockIssueId::derive(...)`, the IC5 residual).

**Asks that draw from the pool.** An ask is `Supply::Stock + Representation::Lot` whose matter leg is a
new **`LotOp::Draw`** = `transfer(storefront pool → hold pool)`, staged at activation where `Consume` is
today (`trade.rs:673-688`, about +28 in the IC0 tree). The listing's quantity is the availability counter:
set equal to the pool at seed and decremented synchronously in-command by `commit_quantity`, so buyers
serialise on the single-writer world task and a draw cannot oversell. There is no reservation, hence no
7-IC-day hold (7 real hours at rate 24, `record.rs:33-35`), no Renew, no `settle_part`, and no stranded
hold on lapse. **Source authority**: the buyer-actor's offer projection authorises endpoints from the
actor's own `live_endpoints` (`lot.rs:312-380`), which never include the vendor's pool, so the
`market_lot` action's `project_offer` projects the Draw's source from the live listing read off
`engine.world` — the seller-side projection beat M required for Deliver, built once for both. The
applier and the readiness report compare each listing's quantity with its storefront pool and page on
drift.

**As built (IC7):** an ask is a Draw listing (`Supply::Stock` on a lot entry); the fourth `market_lot` op
`draw` is a pool-to-pool transfer that writes a `market_lot_draws` receipt in its own bundle; the paid
legs are keyed by contract, `market.lot.<op>.<contract>`, and offered only over the actor's own paid
contracts through `market_lot`'s server-authenticated `paid` fixed parameter — which **closed IC0's
direct-consume hole** (any endpoint holder could take a live hold-backed listing's lot), an amendment to
IC0's cut (IC7 (2)). The Draw sale's payout is gated on the sixth fact namespace, `market`
(`market.lot.drawn`). The accept destination takes both spellings, `+market/accept <quote>=<vault>` and
`+market/accept <quote> to=<vault>` (one command), defaulting to the gate's endpoint (IC7 (3)). The
listing quantity is the in-command counter, checked against the storefront's available stock at
publication.

**Bids that take real lots.** A bid is **`Supply::Wanted{good, min_grade, price, remaining}`**, created by
the applier as a normal-lane market document. The `+market/sell <listing>=<qty> [from=<vault>]` lot arm
(beside the object arm, `trade.rs:931-1085`): the venue gate; synchronous checks (seller pool ≥ qty,
remaining ≥ qty, buyer funds ≥ total); the buyer org's consideration escrowed by `accept_contract_as`
through the party-cap source; payouts from the `ProceedsPolicy` (§4.1); **the payout gated on
`TypedCondition::Fact{inventory, inventory.transferred, id}`** with an offer-time-derivable marker id, so
money never moves before the matter (`facts.rs:44-82`; the one-lane graft over the Before window); a
staged **`LotOp::Deliver`** = an inventory-owned `transfer_marked(hold pool → warehouse pool, good, qty,
marker)` (write_marker is private, `provider.rs:380-410`, so the verb is exported by inventory);
`remaining` decremented at activation and restored on Cancelled or Expired. The `sell()` template has no
seller accept, and neither does this arm. Intake lands in the buyer's **pool**, which keeps Kasei under
the 512-lot cap (`record.rs:58`) and makes intake real, re-sellable stock.

**As built (IC8, IC10):** the bid is `Supply::Wanted{vault, commodity, min_grade}` with the price and
the remaining intake on the listing itself (IC8 (7), one truth; `mush-market/src/record.rs:505`). The
sell arm is `sell_into_bid` (`trade.rs:1701`), grammar `+market/sell 21=20` (bare digits on the left);
fourteen refusals run before any contract. **"No seller accept" as built:** the pilot accepts inside the
verb, before the house's `accept_contract_as`, because `fully_accepted` needs every party; the sale reuses
`Quote` with buyer = the house, seller = the pilot, `PaidKind::Deliver`, the window and cutoff the
Draw's (3,600 / 3,000 s); a deliver leg lost to back-pressure is re-staged by `+market/accept <quote>`
(`restage_deliver`, `trade.rs:2236`). The payout's Fact is inventory's own `inventory.transferred`, the
marker derived at offer time. An exhausted bid **stands at quantity 0** ("not buying") and is never
deleted by the sweep; the clerk re-posts a lapsed bid at its **full** authored intake (IC10 (4); D-18-27
"no restock" read so; re-posting at the remainder is P5). Undock refuses while a sale's goods have not
left the hold (`GOODS_LEAVING`, `tacsim/src/port/voyage.rs:259`) until the cutoff. Kasei's intakes are
4,000 t metals and 9,600 t volatiles, as written (IC8 (1)).

**Vendors and buyers.** One vendor per org (`store.rs:334-336`); a vendor may hold asks and bids.
Ceres (venue `ceres`): **Freeport Exchange** (`freeport_exchange`: metals ask; Freeport Works' alloy bid
is carried by the same org under the name Freeport Works), **Occator Water & Propellant** (`occator`:
volatiles and propellant asks), **Belt Carriers' Cooperative** (`coop`: the provisions ask **and the
yard-fittings bid** for its Dray fleet and the Kirkwood tank farm, 11 §4.5; a Wanted chit on the
chandlery's board, signed by the chandler). Stickney (venue `stickney`): **Kasei Metals** (`kasei`:
metals and volatiles bids at P07; alloys and propellant asks at P08), **Stickney Yards**
(`stickney_yards`, a vendor now, not only a lore org: the crated-fittings ask from the `item.yard_spares`
cage in P15, read at the service counter P11, signed by the Yard's service clerk), **Yard Commissary**
(`yard_commissary`: provisions). **The Freeport Exchange was not in doc 11
§4** and is added by this slice as 11 §4.12 (§10's doc 11 amendment): the Port Council's commodity floor, a
Freeport institution (11 §3.3's Council seats registered claim-holders), consigning the Belt claims'
metal concentrate (Occator's extractor drones and the smaller claim-holders, 11 §4.1; doc 10 §11's
Ceres export) under the Exchange's mark and taking the corridor's alloy intake under the Freeport Works
name; its capital is the slice's largest because it is the return leg's buyer. It has **its own officer
of record**, the Exchange's factor (18a: Margit Søndergaard-Achebe, `npc.sondergaard`, C07); the
Occator factor (Qureshi) signs **only Occator's lines** — a slice rule under the Warrant's principle
(11 §2 says only that every automated act has a liable principal and nothing automated may sign; the
one-house-one-signatory rule is this slice's, in that spirit): an officer of one org is never the
liable principal of another's offers. Storefront vaults are
`CustodyRef::Container` of a manifold or cage item placed in each station's staff-locked Maintenance
Spine (C15/P15), so no player can `get` it and no `ItemDef` lock is needed; buyer warehouses are the
same vaults. Each listing is a standing offer signed by the org's officer of record (the `signatory` in
the `vendors` block, **required** by the `port` schema: an entry with an unsigned listing is refused
before any mutation), the Warrant principal for every acceptance under it (11 §2; Q-18-10); the accept
line names them. Six signatories: the Exchange's factor, Occator's factor, the Co-op chandler (C08; the
provisions ask and the fittings bid), Kasei's receiving clerk (P07), the Stickney Yards service clerk
(P11; 18a: Wren Castillo-Haugen, `npc.castillo_haugen`, who signs only the Yard's ask, never Tycho's
business) and the Yard Commissary's storekeeper (P09; 18a: Hanne Okonkwo-Steyn, `npc.okonkwo`).

**As built (IC8):** as written; the Yard's fittings ask renders on both the P08 and the P11 boards,
and the Co-op's provisions ask on no board (it reads through `+market/list`; IC8 L15/(4)). The boards
name each line's signatory (`tacsim/src/port/board.rs`); the accept line does not (IC8 L17; unowned,
§5). **The hold-backed player sale path is still exposed to §9 risk 3** (a consume refused at prepare or
dropped by back-pressure leaves a Before-only contract that settles and pays the vendor for nothing):
no cut Fact-gates the hold path (IC7 (5); §8 Q-18-42). The sweep keeps a lapsed hold-backed listing
until its hold is Gone — a generic market change for every roster (IC8 (3)).

**Stock and capital (D-18-27, resized under R7).** A sizing basis of about 120 legs each way for one
cohort of eight to ten pilots over about three weeks, derived from today's vertices (80/20 out, 55/45
back) and expected to drift when the astronomy engine moves prices — a basis, never an invariant the
readiness report asserts. Ceres: volatiles 9,600 t (80 × 120), metals 4,000 t (20 × 120 plus a 1,600 t
reserve, so an ice sell-out degrades honestly to mass-bound concentrate loads), propellant 5,000 t,
provisions 2,600 crew-days; Freeport Works' alloy intake 7,000 t; the Co-op's fittings intake 5,400 t.
Stickney: alloys 7,000 t, fittings 5,400 t (45 × 120, on the Stickney Yards vendor), propellant 5,000 t,
provisions 2,600; Kasei's intake volatiles 9,600 t and metals 4,000 t. Capital by one-shot
`create_system_obligation` tranches of at most 800,000 cr, each tranche's receipt row written in the same
batch as its obligation (the economy does not dedup, `api.rs:1683-1818`), under a per-instrument
mint-cap override of 1,000,000 in the template: Kasei 600,000 (self-replenishing: it sells 22,000 of
alloys and 2,400 of propellant per full return against 21,600 paid per outbound, about +2,800 a round
trip); Freeport Exchange/Works 4,000,000 (a season budget: about 28,600 per full return, so its 7,000 t
intake binds first at about 127 returns); the **Co-op 3,100,000** (its fittings intake is 5,400 × 560 =
3,024,000; tranches 800k, 800k, 800k, 700k); the Trust 250,000; the rest 50,000 each. Exhaustion is
honest: a good with no listing quantity renders "SOLD OUT: seeded 9,600 t, 0 left" from the port map and
receipts (a listing cannot carry 0, `record.rs:634-636`); an unfunded or exhausted bid renders "not
buying". Restock is P5, by production only, and must land before a second cohort; money pools in the
Yard and the pilots while the Co-op and Freeport Works drain, so risk 9 matters twice over. `fund_from_faucet` is never used (a 60 s drip, `api.rs:556-590`); capitalization settles on the
next sweep, so the readiness report lists unfunded buyers until it does.

**Presence (D-18-12 kept, extended).** `MarketModule::with_venue_gate(Arc<dyn VenueGate>)`, consulted at
quote, accept and sell with (actor, venue, endpoint) and a presence mode: `Presence::Here` (the actor in
the port's rooms and the hold's hull Direct on this port's domain and not under way) for typed verbs;
`Presence::Ship(hull)` (the hull's locality and the author's live lease; the actor's room ignored) for
queued orders. The locality chain is backing → space → anchor host → hull Direct on the domain. With no
argument `market=` defaults to the port the actor stands in and `to=`/`from=` to the berthed hold.
Locality on **every** inventory path (`@inv/move` between player vaults) is deferred (D-18-37): the pilot
holds exactly one vault and only staff create vaults (`mush-economy/src/commands.rs:3973-3976`).

**As built (IC7, IC8):** `VenueGate` has five methods, `port_seller` added for the sell arm
(`mush-market/src/module.rs:138,167`); tacsim's `PortVenueGate` answers them. `@inv/move` stays
locality-free (D-18-37): a pilot acts on exactly one endpoint, the hull's hold, so a pilot's move out of
it has no endpoint to land in and is refused at the coordinator; staff moves are not gated (§6 as built).

### 4.4 The captain's day: travel and the queue

**The lifecycle.** Docked → departing → under way → docked, as player-visible phases over existing
authority with no parallel location field: docked = the hull Direct on a port's passive domain, presented
into the Visiting Berth Gallery (`graph.rs:151-166`); departing = a burned, not-yet-launched ticket (one
tick); under way = a `CarrierJourney` on the authored route (the hull projects to the in-transit room and
the crew walk the six rooms); docked again = arrival places the hull Direct on the destination's domain.
(In fiction these are the sailing plan's phases — filed, cleared, under way, docked — and the port's
boards show them; no character ever sees a "ticket".)
**Docking is automatic at arrival** (D-18-18 kept): the pasted approach-then-dock step needs a second
carrier move (only journeys move a carrier root, `commands.rs:2735`, `journey.rs:834,1357`) and under R3
the pilot is usually asleep at arrival, so a manual dock would idle the queue. There are no berth slots:
every hull shares the gallery and is listed there (berths, berth-holder markers and the freight decks are
post-loop cut P-F; Q-18-20). Each port entry opens ONE passive domain, host = the corridor location room,
presentation = the gallery (`open_passive_domain`, `window.rs:195`; the applier precedent
`content.rs:572`); a passive domain holds no window or active-domain slot (`window: None`,
`window.rs:226-234`, uncounted by `windows()`, `:279-285`), but `open_passive_domain` refuses while
eight windows are open (`:205-206`), so IC3 skips that guard for passive opens (§3-P, §9 risk 1).

**As built (IC9):** the clearance is two steps — `+ship/undock` files it and answers `CLEARANCE_FILED`
("Clearance filed for {port}: the plan burns {burn} t …; arrival in {icdays} IC days, about {hhmm} UTC
{day}.", `tacsim/src/port/voyage.rs:232`), and the launch on a later fire tells the pilot
`CLEARANCE_GRANTED` (`:216`). A live undock whose burn never lands lapses after 24 fires
(`CLEARANCE_LAPSED`, `LAUNCH_GRACE_FIRES`, `voyage.rs:81,218`; suppressed for a queued depart, IC11 L7). A
burned launch that cannot begin is held (`LAUNCH_HELD`, `:227`; staff paged once, the pilot told once),
and staff stand it down with `@tacsim/port clear #<hull>` — **nothing burned is refunded**, the ticket
is the record (`CLEARANCE_STOOD_DOWN`, `:229`); **there is no player abandon verb** (IC9 (3)–(5)). The
manual `@journey/depart` arm refuses every lane route for every carrier (`LANE_UNPLANNED`, `:224`) and a
programme hull on any route (`MANUAL_DEPARTURE`, `:221`); the pilot of a Trust hull is refused earlier,
by the permission gate, with "Permission denied." (IC9 (2); the story test's negative checks).

**Passage: the planner over the sky at departure (D-18-30 as amended by R9; D-18-44/45).** The lane
`ceres_stickney` / `stickney_ceres` runs via Kirkwood Depot; the lane pack authors each route as **body
pairs** (its places name bodies, `pack.rs:576-582`) with `PackLeg.base_s` kept, required and authored —
**540,800 + 409,200 = 950,000 IC s** unladen (11.0 IC days; legs of 0.741 and 0.561 AU at the epoch) —
as the pack's pin of the constant sky (T1/Q18A: a route's figures come from a published pack,
`route.rs:224-231`), while the **coast rate is drive data**, `dray.json` `drive.law.coast.kms = 205`,
never a test constant. Nothing in the slice reads `base_s` as a passage time. **Durations are
derived**: one pure tacsim `plan_passage(sky, pack, route, drive, dry_kg, payload_kg, occupants,
depart_ic) -> PassagePlan` (`port/passage.rs`, IC9; §4.8 has the shapes) walks the legs as body pairs,
and per leg from body A at t0 to body B iterates a fixed three times: d = `Sky::distance_km(A, t0, B,
t1)`, unladen = ceil(d / kms), laden = `mush_sim::laden_leg_durations([unladen], drive, payload)[0]`
(`route_duration`'s exact per-leg arithmetic, `route.rs:819-832`, factored out as a sky-free primitive
that takes per-leg unladen seconds; `route_duration` becomes its Σ over `base_s`, so its pins hold), t1
= t0 + laden; a leg between two places on one body uses its authored `base_s` through the same laden
step. At the constant sky one pass is exact and the plan reproduces every figure here. Draft 0.2's
`route_leg_durations(route, drive, payload)` is **not built**: it took no sky and no instant, and every
caller would have been rewired at the flip. **IC2's test** (`lane_legs_derive_at_epoch`) derives each
leg from the sky at the epoch and `dray.json`'s kms and refuses if the lane pack's `base_s` disagrees by
more than 100 s; the route's `distance_km` stays what `materialize_route` pins — the direct chord
between the endpoint bodies at the epoch (1.293 AU; `store.rs:800-813`), relabelled "direct chord at the
epoch" in `RouteDefWire` and `@corridor/show`, never the sum (Draft 0.2's line is withdrawn) and read by
nothing in the slice. **Payload changes flight time now**, because doc 10 T1 is binding
(`10-tacsim-foundations.md:287-291`) and Draft 0.1's deferral was a live breach: `CarrierJourney.leg_s:
Option<Vec<u32>>`, a serde-defaulted wire field on the `applied_revision` precedent
(`journey.rs:204-232,309-318`), validated by a fallible `with_leg_times`, read by `leg_due_at` so
`due_cursor`, `arrival_at`, interdiction and `@journey/show` inherit it; `pub mush_sim::depart` takes
`leg_s = plan.legs[].secs`; the burn ticket freezes the whole plan (D-18-46). The drive is the hull's
(`DriveProfile::new(class, 1000, 181,000).with_law(coast)`, `route.rs:637-660`; thrust nominal because
kms is absolute and is the **unladen** cruise), never `nominal_drive`, whose name-sniffing stays for the
fixtures only. **The laden law is doctrine, not physics**: leg_s = base_s × (181,000 + payload)/181,000
(`route.rs:819-832`) says a thrust-limited hauler accepts a slower cruise rather than a bigger burn
(18a's "a week longer on a full hold than on an empty one"), and the rocket equation is not consulted
(Q-18-41). So: near-empty (32 t fuel) 12.9 h, the 69 t first return
17.45 h (17 h 27 m), a full outbound 19.63 h (19 h 38 m), a brimmed 180.7 t hull 22.0 h at rate 24 —
all under a day **at the slice's constant sky**, with at least two hours of slack before a same-time
login (R3), and a light load lands earlier. No gate, test or help text asserts that bound outside that
sky (D-18-50; the R3-against-R9 collision is Q-18-35). The 181 t ceiling stays a non-binding rail: it is
also the laden denominator (`route.rs:806-831`), so a binding ceiling would slow every passage. **Rate
24** (doc 10 1A's one OOC hour per IC day) is a deploy step and a readiness line: dev1 seeds `IC_RATIO=4`
(`deploy/dev1/instance.env:81`), where a 20 h passage is five real days; `@tacsim/time rate=24` on the
next owner-approved redeploy (Q-18-16), and `+coop/enrol` refuses at any other rate outside test worlds.
Tests run at 3600 (a full leg in about 471 s). The real-time ETA is (arrival_ic − now_ic)/rate,
re-derived after a rate change; journeys relabel across generations (`journey.rs:1230-1257`). The
**sky rate** (`[tacsim.time.ephemeris].rate`, "sky motion" in docs to keep it apart from the world rate
and the inert `[tacsim.time].rate`) stays 0 throughout the slice.

**As built (IC2, IC3, IC9):** the lane is v3 (IC8), its legs authored at the real date 2351-05-13 and
`kms` 226 (IC2's lead ruling): unladen about 537,293 + 411,975 IC s; laden for the full outbound 959,361 +
735,600 and for the full return 653,642 + 852,473, with `laden_leg_durations` (`mush-sim/src/route.rs:863`)
factored out as written and its step a FLOOR (IC9 L1). The planner is `plan_passage`
(`tacsim/src/port/passage.rs:236`) with the fixed three-pass intercept. `CarrierJourney.leg_s` is
re-validated on the wire and read by `leg_due_at`; a rate change mid-passage keeps the arrival's IC
instant and re-derives its real-time ETA (`mush-server/tests/ic_slice_loop.rs::
ic12_a_rate_change_mid_passage_relabels_the_arrival`, IC9's T9). The production Rate line refuses
enrolment at any rate but 24 (`PLAY_RATE`, `tacsim/src/port/readiness.rs:37`); the story test boots at
`[game].ic_ratio = 24` and sets 3600 after the enrolments (§6 as built).

**The fuel function is the planner's output.** `passage_burn(law, dry_kg, payload_kg, legs) ->
Result<u64, PlanError>`, a tacsim pure function beside `nominal_drive` (`store.rs:838-850`): for
`DriveLaw::Coast {kms, burn_permille}` it ignores the legs and returns `ceil((dry + payload) ×
burn_permille / 1000)`, the slice's 105‰ and 41 t. Honestly stated: 105‰ is a **doctrine figure**,
distance-free by construction, not the rocket equation. A fixed burn fraction gives one Δv (2 × 205
km/s at an exhaust near 3,700 km/s) and so one cruise whatever the mass, which the laden law
contradicts by flying a 142 t payload 1.79× slower on the same fraction (at that exhaust a 115 km/s
cruise would burn about 6 %, not 10.5 %). The slice keeps both as data — a moving sky changes
durations and crew-days, never the 105‰ — and the engine must choose between a mass-dependent cruise
(keep the laden law, burn by doctrine) and a mass-dependent burn (the rocket equation, the laden law
dropped): **Q-18-41**, a P-A decision the slice does not pre-empt because both answers read the same
`DriveLaw` seam; for the engine's `Flip {accel_mg, exhaust_kms}` it returns a distance-dependent burn
behind the same seam. A validator refuses two fielded definitions naming one class with different law
blocks (one class, one law; the "drive table" is the index over fielded definitions, honouring Q-18-5
and §9's refusal of a `drive` rules kind), and the planner refuses a class with no law
(`UnknownDriveClass`) rather than falling back. `RouteDef.propellant` is authored 0 and **not**
reinterpreted (the `store.rs:823-835` ruling and `e7c1.rs:1447` treat it as a payload-unit quantity).
Fuel aboard is departure mass, so a reserve costs 105 kg per tonne carried; `+ship/plan` says so and shows
the remainder; no minimum reserve rule (Q-18-26, binding). The tank cannot fuel both legs with a full
hold (out 45 t at a brimmed tank, back 37–40 t), so Stickney's price matters.

**As built (IC9 (7)):** `passage_burn` as written; the 105‰ law counts the fuel's own mass, so §6's
"40 t against a 41 t burn" cannot be built: the tank case is **39 t aboard against a 40 t burn** (the
vertex with 39 t), as the story test's negative check builds it from the planner.

**As built (doc 19 cut A.2, 2026-10-18):** Q-18-41 is ruled the engine's way — the Dray flies
`DriveLaw::Flip { accel_mg: 50, exhaust_kms: 6000 }` (`dray.json` v3): a leg is `2√(d / a)` at the
hull's thrust-limited acceleration for its load (50 milli-g at the rated 421 t, more when lighter),
the burn the rocket equation over the passage's `Δv = Σ a·t`, so **both time and fuel move with the
sky**; the laden law stays mush-sim's for the coast law and same-body legs only. `passage_burn` takes
the `HullDrive` (law, rail and the passage's throttle, 250–1000 ‰). At the epoch sky the story's
outbound (100 t cargo, 53 t bunkered, 19 crew-days) takes 321,547 + 1,256,313 s (18.3 h at rate 24)
and burns 51–52 t; the return 1,195,660 + 306,023 s (17.4 h), 48 t, 18 crew-days — the slice's 42 t
tank no longer covers an outbound, so the story bunkers 53. The solar keep-out (0.3 AU) is a detour
cost in the leg's distance, never a closure (Q-18-40); `choose_route` keeps the fastest or the
cheapest listed route and honours `via=` as a waypoint filter; the throttle rides `+ship/plan`,
`+ship/undock` and the depart order. The figures above this note are the coast law's history
(D-18-50: none was an invariant); the cut note in [18-cut-notes.md](18-cut-notes.md) "A.2" has the
table.

**The queued-orders model (D-18-31).** Rejected carriers: `@wait` re-dispatch (it can act as the pilot,
`engine.rs:3213-3232`, but is RAM-only, invisible, capped at 100 shared slots, not atomic with any record,
voided by fan-out) and `schedule_source` (acts as the pilot, writes typed families only, cannot post money
or move a hull, `journey.rs:13-20`). **The model: a durable per-hull document executed by a pull tick**,
the shape of the journey, housing and campaign ticks (`journey.rs:11-35`; `mush-campaign/src/tick.rs:13-27`).
The record is a **legacy** `TacsimStore` document, `tacsim:orders:<hull>`, not a typed family, so it lands
in the same WriteBatch as the step's EconomyCarveOut and market post-image writes (`store.rs:545`;
`api.rs:105-110`; `world_task.rs:7230-7262`); a corrupt document fails closed for that hull and pages.

```json
{ "schema": 1, "hull": 5011, "author": 341, "rev": 17, "state": "running", "hold": null, "cursor": 3,
  "inflight": { "order": 3, "attempt": 1, "key": "tacsim.port.burn/5011/3/1",
                "refs": { "contract": null, "ticket": "5011:9" }, "issued_utc": 1760551800, "issued_ic": 11843200000 },
  "orders": [
    { "id": 1, "kind": "sell",   "good": "metals",            "qty": "all", "min_major": 250, "on_short": "partial", "state": "done" },
    { "id": 2, "kind": "buy",    "good": "propellant",        "qty": "fill", "max_major": 60, "on_short": "hold",   "state": "done" },
    { "id": 3, "kind": "depart", "to": "ceres", "not_before": { "utc_daily": "19:30" }, "not_after_s": 7200,
      "on_fail": "hold", "state": "inflight" } ],
  "log": [ { "utc": 1760551200, "ic": 11842800000, "order": 1,
             "text": "Sold 20 t metal concentrate to Kasei Metals at 280.00cr/t: 5,600.00cr; Co-op share 400.00cr; you received 5,200.00cr." } ] }
```

Bounds: 16 orders; 32 log lines of at most 240 bytes, FIFO; `kind` ∈ buy | sell | depart | stop;
`on_short`/`on_fail` ∈ partial | skip | hold; order `state` ∈ pending | inflight | done | skipped | held;
document `state` ∈ running | paused | held. The venue is implicit (the port the hull is docked at when the
step runs). **`fill` (D-18-41)** is a quantity for propellant and provisions computed together: project the
heaviest load the queued orders can make — the cargo still aboard after the queued sells plus each
queued cargo buy clamped by `max_more` in queue order, with no funds term (cash can only make the load
lighter, so the fuel is always enough) — then take the joint least fixed point at or above what is
aboard of burn = `passage_burn` and crew-days = occupants × Σ ceil(laden leg days) over `plan_passage`
at the depart's `not_before` with payload = cargo + fuel + provisions + occupants (six iterations,
asserted); hold if the tank or the locker cannot carry it. A full hold gives 40 t and 21 crew-days (Draft
0.2's 41 was the greater fixed point); the day-2 projection of 73 t gives 37 t and 19. Because there is
no funds term a cash-bound leg over-bunkers by a few tonnes and a crew-day or two, which stay in the tank
for the next fill and cost about 200 cr once; `trade.md` says so. A depart's `not_before` is a UTC
instant, a daily UTC time or `+Nh` (a real-world habit, immune to `@tacsim/time`), with an optional
`not_after` window after which the order holds rather than leave at an odd hour. **The fiction of the queue, one story in both documents (Q-18-24).** The orders document
is the **sailing plan**: a filed instrument the pilot signs at the dockmaster's (C14) or harbourmaster's
(P05) counter — sales, loading, bunkering, stores, the departure hour. The **port** executes its money
and matter legs under the pilot's signature: the berth loads and bunkers on the loading schedule, the
Exchange's and Kasei's clerks settle the filed sales against the pilot's signed lines, and the
dockmaster clears the hull at its hour. The Class 0 drive ROM (doc 17 §9, D-UM-32) **only flies the
cleared route**: it "drives, senses, holds and returns on its orders", runs the Registry's doctrine and
nothing else, and under the Warrant (doc 11 §2; D-UM-11/52) contracts for nothing and signs nothing —
exactly what 18a's Bridge pane says ("FLIES THE PLAN, DECIDES NOTHING"). The pilot is the liable
principal of every line, and every log line names the pilot as signatory. The executor is the port's
clerks in fiction and a tacsim tick in mechanism; the ROM is never the agent of a sale.

**As built (IC11):** the document is `tacsim:orders:<hull>` (`ORDERS_PREFIX`, schema 1;
`tacsim/src/port/orders.rs:56-58`), `Orders` (`:90`): `schema`, `hull`, `author`, `rev`, `state`,
`hold`, `cursor`, `next_id`, `inflight`, `orders`, `log`. Limits: `MAX_ORDERS` 16, `MAX_ORDER_LOG_LINES`
32 of `MAX_ORDER_LOG_BYTES` 240, `ORDERS_SHOWN_LOG` 8, `MAX_DEPART_CREW` 15, `MAX_PRICE_MAJOR` 1,000,000
(`orders.rs:60-70`); `at=`/`+Nh`/`+Nm` at most 7 days ahead and `until=` at most 24 h
(`tacsim/src/port/when.rs`; IC11 O6). The wire nests the order's kind (`What`: `sell{good, qty,
min_major, on_short}`, `buy{good, qty, max_major, on_short}`, `depart{to, not_before, not_after_s,
on_fail, crew}`, `stop`; IC11 L3) beside `id`, `state`, `attempt`, `refusals` (a `u8`, at most
`MAX_ORDER_REFUSALS` 3, counted apart from drops; IC11 L26), `retry_at`, `announced` and `reached`. The
sell order names a **good**, the bid resolved at the venue when the step runs (IC11 L2). `fill` is built
as written, the least fixed point at the depart's `not_before` (`fill_targets`,
`tacsim/src/port/executor.rs:481`), with the reserve for the next departure's needs. `seed_next_id` keeps
a replaced document from reusing recent order ids (IC11 L29).

**Verbs.** `+ship/orders` (the queue: cursor, each order's state, next due in UTC, IC and real time, any
hold reason, the last eight log lines; readable anywhere); `+ship/order sell <good> <qty|all> [min=]
[short=]`, `+ship/order buy <good> <qty|fill> [max=] [short=]`, `+ship/order depart <port> [at=now|HH:MM|
daily HH:MM|+Nh] [until=+Nh]`, `+ship/order stop`, `del <n>`, `move <n>=<pos>`, `clear`, `pause`,
`resume` (retries a held step), `skip <n>`; `+ship/plan [<port>]` (a dry run of the queue against live
listings, pools, funds, burn and ETA; writes nothing, the `@journey/plan` precedent); `+ship/undock
<port>` (live, at=now only: scheduled departure lives in the queue alone). Editing is allowed aboard the
hull or anywhere in the port area where it is docked (Q-18-28); edits bump `rev` and never touch the
inflight step. `+orders`/`+order` belong to mush-propagation (`commands.rs:1252-1279`), hence the
`+ship/` namespace (Q-18-22). The **real-time** ETA and departure clock (UTC) appear only in these
renders, in `+ship` and in the `look` status line — OOC-facing surfaces; room details and NPC speech
give IC time only, because the rate is the campaign's compression (doc 10 1A, T17), not a fact a
character knows.

**As built (IC11; IC12):** the nine sub-verbs as written, filed aboard or in the port where the hull is
docked (a port room by the venue gate's own rule; IC11's Q-18-28). The time grammar is `at=now|HH:MM|daily
HH:MM|+Nh|+Nm` and `until=+Nh|+Nm` (`when.rs:13-15`); `SHIP_HELP` and `ORDER_USAGE` omitted `+Nm` until
IC12 corrected them (`tacsim/src/port/ship.rs:62`, `orders.rs:715`). `+ship/undock` takes `at=now` only
(`AT_NOT_YET` otherwise) and `via=` is refused "not yet" (`VIA_NOT_YET`). Every rendered log line is
signed ", signed {pilot}" from the document's author — rendered, never stored; notices are unsigned (IC11
L27, Q-18-24). `stop` is a barrier that marks itself done and pauses the plan until `+ship/order resume`,
never discarding later orders (IC11 O5).

**Offline execution (D-18-32).** The order arm of the tacsim normal-lane tick (IC5's skeleton; period
5 s) rotates over hulls with orders (visit ≤ 64, work ≤ 16), takes at most ONE step per hull and ONE
authoritative submission per pilot per fire, and for a due step: (1) re-checks that the author still
holds the live lease; (2) evaluates the condition from World state (placement and JourneyBook,
MarketStore listings and bids, hold pools through `Ctx::action_state`, available funds); (3) swaps
`ctx.actor` to the pilot (`mush-npc/src/module.rs:1496-1502`; `tacsim/src/seed.rs:384-398`), calls ONE
closed typed function, and restores it — **buy** = `trade::quote` then `accept_into` against the port
counter, destination the hold vault, the quantity clamped first to min(qty, listing remaining,
`max_more` by **mass and by volume**, available funds **less the next depart order's unmet burn and
crew-days at this port's prices**) because `quote` refuses a quantity above the listing (`trade.rs:246`
at `c87d78f7`), the log naming every bound that bit in the fixed order stock, hold mass, hold volume,
funds ("Bought 20 t structural alloy stock … short of 55 t: funds"; "Bought 85 t water ice … short of
100 t: hold volume"; "Bought 0 t yard fittings: short of 10 t: hold mass and hold volume"); **sell** =
the Wanted lot arm; **depart** = the undock checks, `plan_passage` now, then `tacsim.port.burn` staged
as the pilot with the plan as its input under key `tacsim.port.burn/<hull>/<order>/<attempt>`, the same
tick launching the ticket on a later fire; (4)
writes the order's transition to `inflight` in the SAME Outbox as the step's market and economy writes;
(5) on later fires verifies completion by pull (lot in the endpoint and contract settled; ticket burned
and journey present, `journey.rs:506-520`) before marking it done, logging and queuing a notice. After a
reboot the inflight step is verified, never re-run. "Not admitted" is normal: the submit queue is
engine-wide, 64 actors, one per actor, drained one per step (`engine.rs:688-721`), shared with live players;
a clustered daily hour serialises through it and is retried by pull. Two prerequisites: tacsim is an
inventory quantity delegate (`composition/mod.rs:404`, IC5; typed-state reads are scoped to the tick's
module, `engine.rs:3285-3289`); the market lot leg is keyed by contract, `market.lot.<op>.<contract>`
(IC7; today's listing + IC-second key, `lot.rs:708-745` at `8a32734a`, about +78 lower at `c87d78f7`,
makes a re-stage a new request), so a re-staged
leg replays as a HIT and the executor never has to "hold and page" a paid-for step. Never orderable:
enrolment, buyout, boarding and leaving, walking, free text (the macros rule, `macros.rs:141-147`).

**As built (IC11):** one closed call per step as the pilot, as written; the departure is the shared
`prepare_departure` (`tacsim/src/port/voyage.rs:1368`), which returns a typed `DepartRefusal{kind, line}`
over 15 kinds. **A queued depart whose burn produced no ticket is re-filed under `attempt + 1` with a
fresh `seq`, never re-staged under the same key** (IC11 L7; replaces this paragraph's "the same tick
launching the ticket" retry and §4.4's "re-staged under the same key" for departs only): the launch arm's
lapse is the depart's "not admitted" (`NOT_ADMITTED`, logged, never notified), at most
`MAX_ORDER_ATTEMPTS` = 3 (`executor.rs:81`), and a fourth drop holds (`HELD_UNCONFIRMED`). A buy is done
when its quote's `drawn_at` or its settlement reads (IC11 L5). Market legs re-stage at `staged_utc +
ORDER_RESTAGE_SECS` (60 s) and a second miss is judged `ORDER_SETTLE_GRACE_SECS` (160 s) after the first
re-stage (`executor.rs:72,76`; IC11 L6). `at=` is the executor's due gate and the plan is made at the
step (IC11 L8). A daily depart resolves at its durable **reach**, `Order.reached{utc, venue}`, and a daily
`HH:MM` reached past `until=` rolls to the next day while a fixed instant past it holds (IC11 L9, O2).
A listed passenger is a person aboard other than the pilot when the depart is filed; one ashore at launch
is left behind and both are told (IC11 O3). The story test runs a whole queue on the production root
(§6 as built).

**Failure handling.** Buy: never above `max`; cargo fills partially and logs the shortfall; propellant and
provisions hold when the shortfall would fail the next depart. Sell: fills against the bid's remaining
intake and keeps the rest aboard; an unfunded or exhausted bid is skipped and logged. Depart refused
(pilot not aboard, tank below burn, locker below crew-days, overloaded, lease lapsed): holds with nothing
burned, re-checked silently each fire, logged once; past `until`, holds. A step unverified after twelve
fires is re-staged under the same key; after a second miss, held and paged. Every hold or skip queues a
reconnect notice (housing's `notice.rs:1-20`; `mush_mail::deliver` is crate-private) under the heading
"While you were off watch:", and the executor narrates to the hull's member rooms. IC-stamped conditions
relabel on a rate generation like journeys, never pause like industry (`provider.rs:2977-2985`). A pilot
who is ashore at a depart's hour holds the ship (Q-18-17); a listed passenger ashore is left behind with
a notice to both.

**As built (IC11 O4, L10):** every hold is one of four shapes — stopped, stopped over inflight, waiting,
waiting over inflight — and an inflight record is never discarded (`orders.rs` module doc). **Waiting**
holds clear themselves: a depart's transient refusals and a lapsed lease (`HELD_CHARTER`). **Stopped**
holds need `+ship/order resume`: `short=hold`, a consumable buy short for the next depart
(`HELD_SHORT_FOR_DEPART`, on the literal reading even when a later `fill` would cover it — IC11's default,
IC14 revisits the projection), an unmet `fill` (`HELD_FILL`, `HELD_FILL_PLAN`, `HELD_NO_DEPART`), a past
window (`HELD_WINDOW`), `NO_ROUTE` (stopped, a recorded deviation from "re-checked silently"), three
refused calls (`REFUSED_RETRY` logged once), three drops or a second unverified miss (`HELD_UNCONFIRMED`),
and a staff stand-down (`HELD_STOOD_DOWN`; `clear_cmd` bumps `attempt` and clears the inflight step). A late
hand-over past its cutoff logs `AWAITING_REFUND` and the order is tried again after the refund. A sale
that takes nothing names why (`UNDER_MIN`, `INTAKE_TAKEN`, `HOUSE_CANNOT_PAY_SKIP`, the short clause;
`sale_none_reason`, `executor.rs:2863`, factored in IC12). Where each is proven: §6 as built.

**What customs gets.** Unchanged from Draft 0.1: narration at the Kirkwood boundary and a queued notice;
pickets, holds and the Customs Assertion front are P4.

### 4.5 The stations

The room-by-room brief — themes, overviews, every room's descs, details, exits and items, the NPC cast,
the lore pools and the ambience — is [18a](18a-trade-run-stations-and-ship.md). This section fixes what
the packs must declare and what the loop uses (D-18-35; R6).

**Scope.** Ceres Freeport ships **13 rooms** on three decks and Stickney Yards **12 rooms** on two. The
owner's pasted fifteen-room grids are the source; per station the three freight-side rooms behind
berth-marker locks (Ceres C11 Freight Transfer Junction, C12 Bonded Freight Hall, C13 Tank Service
Gallery; Stickney P06 Bonded Freight Hall, P13 Freight Transfer Junction, P14 Tank Service Gallery) are
pruned to the post-loop freight-deck cut **P-F** (Q-18-23): each needs berth-holder marker locks, marker
sync from docking records, boot reconcile, the `+move` restricted fix (`window.rs:2276-2287`) and
`ExitDef` fail text, and the loop never walks them because cargo moves between vaults. They stay as
background seen through the dockmaster's window and the receiving office's bars. Ceres keeps a third
deck, the Port Authority Gallery, so doc 11's principals are placed `enabled:false` for the faction cut
(Draft 0.1's §4.6 kept); Stickney's principals sit in its Maintenance Spine the same way.

| Ceres Freeport (`neighborhood.ceres_freeport`, structure "Freeport Hub") | deck | the loop uses it for |
|---|---|---|
| C01 Visiting Berth Gallery | Public (1) | marker `port.dock`; the domain's presentation room; hulls listed; `+ship/board` (and where `+ship/leave`, typed in the airlock, lands) |
| C02 Pressure Lobby | Public | the only route between C01 and the port; doors to C01/C03 only; walls elsewhere |
| C03 Port Watch Arrivals | Public | the Limbo door lands here; the public stair down to C14 |
| C04 Freeport Concourse | Public | circulation; the map detail (`+atlas`); directory |
| C05 Registry & Co-op Desk | Public | `+coop/enrol`, `+coop/buyout`, `+coop/release` (C05-bound; `+coop` and `+coop/statement` read anywhere); the registry ledger; the staff ladder to C16 |
| C06 Ceres Clearing Office | Public | `+credit` |
| C07 Exchange Hall | Public | metals and volatiles asks, the alloy bid; `+market/list` defaults here |
| C08 Co-op Chandlery | Public | provisions and the reaction-mass counter |
| C09 Tycho Mutual Agency | Public | the hull-registry board; the 120,000 cr comparator; Re-float lore |
| C10 The Slack Line | Public | crew commons; where strangers meet |
| C14 Dockmaster's Office | Freight (0) | the departures board (LookRendered); the queue editable here for a docked hull |
| C15 Maintenance Spine | Freight, restricted, `lock: staff`, hidden | the storefront items backing the vendor vaults |
| C16 Port Authority Gallery | Office (2), restricted, staff ladder | Wen, Varga, Ndiaye, Osei-Ferrante, Holm `enabled:false` |

| Stickney Yards (`neighborhood.stickney_yards`, structure "Yard Rim") | deck | the loop uses it for |
|---|---|---|
| P01 Visiting Berth Gallery | Public (1) | marker `port.dock` (without it every public room fails `topology.unreachable`, `validate.rs:1958-2000`); presentation; boarding |
| P02 Pressure Lobby | Public | doors to P01/P03 only |
| P03 Republic Customs Hall | Public | arrival clearance as flavour; nothing collected |
| P04 Yard Concourse | Public | circulation; shift boards; the map detail |
| P05 Harbourmaster's Office | Public | the berth board; the queue editable here |
| P07 Kasei Receiving Office | Freight (0), public via the stair from P08 | Kasei's bids (`+market/sell`); the Warrant signatory |
| P08 Materials Exchange | Public | Kasei's alloy and propellant asks; the bids board |
| P09 Yard Commissary | Public | provisions |
| P10 The Shift Board | Public | crew commons |
| P11 Stickney Service Counter | Public | hull cards; the new-Dray price; Tycho's Phobos agent merged |
| P12 Yard Observation Gallery | Public | the quiet scene room |
| P15 Yard Maintenance Spine | Freight, restricted, staff, hidden | storefront items; Castellanos, Tanaka, Bask `enabled:false` |

**As built (IC1, IC8, IC12):** the two stations ship as written, 13 and 12 rooms. **Pack versions:**
`neighborhood.ceres_freeport` v1 (IC1) → **v2 (IC12)**; `neighborhood.stickney_yards` v1 (IC1) → v2 (IC8, the
P11 fittings card's text) → **v3 (IC12)**; the two region packs at v1; each a new version with its digest
re-pinned (`crates/tacsim/tests/stations.rs`), never edited in place. A detail name admits only
`[A-Za-z0-9_ ]` (`check_details`, `mush-atlas/src/apply.rs:1012`), so 18a's five hyphenated details shipped
only in IC12, as `hull registry board` (C09), `sailing plan forms` (C14), `sailing plan pad` (P05),
`yard time board` and `insurers mark` (P11) — IC1's lead ruling: rename, never widen the validator. Ceres
v2 also strikes three clauses that named a verb as meta-text (the C04 map's `+atlas`, the C07 price
board's `+market/list`, the C08 schedules' `+ship/plan`); every live market figure is on a board or a
listing, never painted, and the Dray's 120,000cr list price and the C14 board's "two to three weeks in a
good season" stay as fiction (IC12 O3). The Yard's fittings ask renders on both the P08 and the P11
boards and the Co-op's provisions ask on no board (IC8 L15); every board line names its signatory. Not
built and unowned: the C14 departures board and the C01/P01 berth boards as `LookRendered` lines (§5's
IC12 list). The validator now reports a room's `short_desc` over 256 bytes and a neighborhood's ambience
over 24 lines, blank or over 512 bytes as `budget.text` errors, and `topology.unreachable`'s hint names
`port.dock` (`mush-atlas/src/validate.rs`; IC12).

**What the packs declare.** Each station is a surface region on its body plus one neighborhood (the tested
path; no orbital region), one site, one structure with floors rising in index and z
(`validate.rs:1679-1715`): Freight Deck 0 at z 0, Public Deck 1 at z 6 (Ceres) / 7 (Stickney), Ceres' Office
Deck 2 at z 10.5. Cross-deck exits carry `portal.at` + `dz_m` (`mode: stair` for the public stairs,
`hatch` for the staff hatch, `ladder` for the Registry ladder; no `lift` mode exists). Every room is
`kind indoor` or `circulation`. Paired exits lock only the way in (`apply.rs:1500-1519`); the one
restricted room per station below the office deck uses `lock: staff` + `portal.visibility: hidden`
(the staff-lobby precedent). **Partitions between adjacent public rooms with no exit are `wall`
features** (`blocks.movement`), because only structure rings and Wall features are walls on the scene
map (`raster.rs:412-450`): Ceres C02|C10, C08|C10, C01|C10, C01|C08, C05|C06, C06|C09; Stickney P05|P08,
P08|P11, P11|P12, P03|P05, P09|P12, P02|P10, P01|P10 — every exitless shared seam of 18a §0.4's boxes,
re-cut so the Yard Concourse shares an edge with every desk it opens onto, because a plain `door`
between footprints that do not touch is `geometry.portal_gap`, an **error**, and the applier refuses the
exit outright (`validate.rs:1808-1840`; `apply.rs:1453-1457`), which on a boot pack aborts the first
apply; IC1's validate-clean test carries an adjacency check over the authored boxes (every plain door
shares an edge, every exitless seam is walled) rather than trusting the brief. Hydration
`{enabled:false, max_player_sites:0}` on both neighborhoods. Themes:
the 2350 base (banned words, pov rules, desc length) on each region, the flavour on each neighborhood
(theme fields have no `deny_unknown_fields`; a misspelled name is dropped, a wrong type fails decode).
Storefront items sit in the Maintenance Spines. `RoomDef.markers` with `port.dock` as the topology seed
is kept (D-18-19). Exit aliases avoid the kernel words and never `board`, `disembark` or `dock`.

**The IC door and the IC-return rule.** `exit.limbo_arrivals` from Limbo to C03 is **unlocked**
(D-18-3 amended): tacsim's `OnboardingGate` already refuses every exit line for an unapproved character
(`onboarding.rs:97-112`). Every exit located in Limbo is global (`movement.rs:24-42`), so a pilot at
Stickney could type `o`, `x`, `c` and skip the passage; a guard that only tests "standing in Limbo" does
not close that path. IC1 therefore adds an **IC-return rule**: `+ship/leave` and the Limbo-bound OOC exit
stamp an unregistered `tacsim.last_port` on the character; a tacsim `MoveAttempt` hook refuses the IC
door to anyone whose last port is not Ceres ("You left the corridor at Stickney Yards; `+ic` returns you
there"), and `+ic` moves the character to that port's berth gallery. Aboard, the boundary rule already
refuses `o` and `home`.

**As built (IC1):** `+ic` lands a Ceres pilot at **C03 Port Watch Arrivals**, where the Limbo door goes,
and a Stickney pilot in P01 — not "the berth gallery" for Ceres (IC1's owner default). The stamp is an
observer on `Moved`: a Player leaving a port room for a room in no port (`o`, `home`, the OOC exit) is
stamped with that port (`tacsim.last_port.<port>`); `+ship/leave` moves from a hull room into the gallery
and needs no stamp. The door's refusal is `left_at_line` (`tacsim/src/ic_return.rs:123`), "You left the
corridor at Stickney Yards; +ic returns you there.", with **no staff exemption**; standing in Stickney the
global `c` meets "There is no door from Stickney Yards to Ceres Freeport; the way there is by ship."
`+ic`'s refusals `NOT_A_CHARACTER`, `NO_CORRIDOR` and `ALREADY_IN` (`ic_return.rs:115-119`) and the
no-exemption door are tested since IC12 (`crates/tacsim/tests/stations.rs`, `ic12_ic_*`,
`ic12_the_ic_door_has_no_staff_exemption`).

**Visibility.** `+atlas` and the `+map` frame hide by access (`view.rs:38-46`): the two restricted rooms
per station are absent for everyone but staff and occupants, which is the intended texture ("the
Freeport's power is paperwork"). Non-staff `+atlas <place>` is limited to the viewer's region, so
Stickney's plan is not readable from Ceres or from aboard a hull.

**Authoring and review.** Hand-authored JSON compiled in, digest-pinned by a copy of
`crates/mush-atlas/tests/starter.rs:31-60`, validated clean by a copy of `the_starter_packs_validate_clean`
(`tests/validate.rs:136-152`), because boot packs bypass the validator and a refused first apply aborts
the boot of dev1's kept world (`mush-content/src/module.rs:113-139`). Appended to the boot list and the
atlas shelf. The tacsim `port` entries name rooms by `<pack>.<entry>` through the latch
(`travel.rs:960-967`); the applier refuses before any mutation if a room lacks `RoomGeo` or the gallery
lacks `port.dock`. Names below doc 11's principals are binding under R8 (Q-18-12, "names ok"; 18a §4).

### 4.6 NPCs

The cast, voices, knowledge bounds and fallback lines are 18a §1.4 and §2.4. The rules of Draft 0.1
stand: every NPC ships in the pack that makes its room (`mush-npc/src/content.rs:214-219`); interface
NPCs front only providers that exist (atlas, travel, help, market; the economist answers the asker, IC0);
no NPC fronts `request`; **no NPC performs a transaction** — every money and matter act is the player's
own verb or the queue executing the pilot's own signed orders, and dialogue is never the authority for
money, title, matter or movement. The six service NPCs are local employees: the Co-op desk officer
(C05), the Clearing teller (C06), the Occator factor (C07, the Warrant signatory of Occator's offers),
the Orbital Guard customs officer (P03), the Kasei receiving clerk (P07, the signatory of Kasei's bids)
and the Stickney service clerk (P11, who also sits Tycho's chair). Extras: the Exchange's factor (C07,
the signatory of the Exchange's ask and Freeport Works' bid), the chandler (C08, the signatory of the
Co-op's provisions ask), the Slack Line's barkeep (C10), the harbourmaster's clerk (P05), the Yard
Commissary's storekeeper (P09, the signatory of its ask), the Shift Board's cook (P10), and a Tycho
agent at C09 — thirteen NPCs in all, every listing with a signatory of its own org. Principals are
placed `enabled:false` in C16 and P15 and never voiced in the slice. Lore pools by
tag: `corridor_trade`, `port_ceres`, `port_stickney`, `coop_lay`, `tycho`, `tether`; facts quote doc 11
and sit in the public/guarded/secret tiers 18a lists. The stock-clerk actors that drive seeding are
dedicated objects, separate from the NPCs, so NPC submissions never collide with stock-arm submissions in
one drain.

**As built (IC1, IC3, IC6, IC8):** the thirteen NPCs ship in the station packs; the principals are
`enabled:false` and never wake (`crates/tacsim/tests/stations.rs::ic1_the_principals_never_wake`); the
fallback lines that name `+coop`, `+credit` and `+ship` now name live verbs (they answered "Huh?" until
IC3/IC6). The stock clerk is one dedicated Thing per port (IC8 L2), separate from the cast. The boards
name each line's signatory; no NPC performs a transaction.

### 4.7 The content pipeline

**Which vocabulary carries what.** (1) **Rules**, tacsim `rules_definition`, compiled in:
`tacsim.pack.dray` (unit + interior) and the goods table `goods.json`. (2) **Places**, mush-content atlas
packs: `region.ceres_freeport`, `neighborhood.ceres_freeport`, `region.stickney_yards`,
`neighborhood.stickney_yards` (existing kinds plus `RoomDef.markers`; npc and lore inside). (3) **The
lane and the ports**, one tacsim `campaign_seed` pack **`tacsim.lane.ceres_phobos` v1** (D-18-36; a fresh
id, because `tacsim.corridor@1` and `@2` are the published test fixtures with pinned digests,
`fixture.rs:34`, `store.rs:6`): the `sol` region; bodies `ceres`, `kirkwood`, `mars`, `pallas`; locations
`ceres_freeport`, `kirkwood_depot`, `stickney_yards`; routes `ceres_stickney` and `stickney_ceres` (legs
via Kirkwood, `drive_class coast_hauler`, `propellant 0`); and the `port` kind (D-18-20), joined to
`TacsimStore.loaded` through the narrow game-packs list as before.

**As built (IC2, IC3, IC8):** `tacsim.lane.ceres_phobos` is published at **v1 (IC2), v2 (IC3, the two
`port` entries and the 226 km/s legs) and v3 (IC8, the vendors, stock, asks and bids)** — each a new
version with its digest pin, never edited in place, because `boot_reconcile` wrote the v1 row on every
enabled world that booted IC2 (`LANE_VERSION`, `tacsim/src/definitions/content.rs:577`); the lane pack
requires nothing. `tacsim.pack.dray` is at v2 (IC8) and `goods.json` at v1.

**The `port` entry** (Ceres; the `clearing` and `charter` blocks of Draft 0.1 are gone):

```json
{
  "ceres_port": {
    "kind": "port",
    "def": {
      "location": "ceres_freeport",
      "venue": "ceres",
      "area": "neighborhood.ceres_freeport",
      "gallery": "neighborhood.ceres_freeport.room.c01_berth_gallery",
      "rooms": {
        "arrivals": "neighborhood.ceres_freeport.room.c03_arrivals",
        "registry": "neighborhood.ceres_freeport.room.c05_registry",
        "clearing": "neighborhood.ceres_freeport.room.c06_clearing",
        "exchange": "neighborhood.ceres_freeport.room.c07_exchange",
        "chandlery": "neighborhood.ceres_freeport.room.c08_chandlery",
        "dockmaster": "neighborhood.ceres_freeport.room.c14_dockmaster",
        "spine": "neighborhood.ceres_freeport.room.c15_maintenance"
      },
      "treasury": "tacsim.treasury",
      "orgs": [
        { "id": "ceres_clearing",   "name": "Ceres Clearing",                 "capital_major": 50000,   "faction": "freeport" },
        { "id": "coop",             "name": "Belt Carriers' Cooperative",     "capital_major": 50000,   "faction": "freeport" },
        { "id": "coop_trust",       "name": "Co-op First Captain Trust",      "capital_major": 250000,  "faction": "freeport" },
        { "id": "freeport_exchange","name": "Freeport Exchange",              "capital_major": 4000000, "faction": "freeport" },
        { "id": "occator",          "name": "Occator Water & Propellant",     "capital_major": 50000,   "faction": "freeport" },
        { "id": "tycho_mutual",     "name": "Tycho Mutual Assurance & Salvage","capital_major": 50000,  "faction": "concord" }
      ],
      "programme": {
        "id": "coop.first_captain", "org": "coop_trust", "office": "neighborhood.ceres_freeport.room.c05_registry",
        "floor_major": 50000, "unit": "tacsim.unit.dray", "issuer": "tacsim.registry",
        "pool": 8, "registry_prefix": "FPC-D-04",
        "hull_names": [ "Slack Water", "Cold Comfort", "Ninth Ledger", "Patience Wen", "Kirkwood Lamp", "Long Division", "Dry Season", "Tally Ho" ],
        "stake_major": 25000, "share_permille": 250,
        "reference": { "metals": "ceres_port/freeport_exchange", "volatiles": "ceres_port/occator",
                       "structural_alloys": "stickney_port/kasei", "industrial_parts": "stickney_port/stickney_yards" },
        "buyout_major": 90000, "dormancy_real_days": 7, "per_account": 1
      },
      "vendors": [
        { "org": "freeport_exchange", "storefront": "neighborhood.ceres_freeport.item.exchange_manifold",
          "agent": "freeport_exchange.clerk", "signatory": "neighborhood.ceres_freeport.npc.sondergaard",
          "stock": [ { "good": "metals", "qty": 4000, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "metals", "price_major": 200 } ],
          "bids":  [ { "good": "structural_alloys", "price_major": 520, "intake": 7000, "as": "Freeport Works" } ] },
        { "org": "occator", "storefront": "neighborhood.ceres_freeport.item.occator_manifold",
          "agent": "occator.clerk", "signatory": "neighborhood.ceres_freeport.npc.qureshi",
          "stock": [ { "good": "volatiles", "qty": 9600, "grade": 50, "issue": 1 },
                     { "good": "propellant", "qty": 5000, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "volatiles", "price_major": 100 }, { "good": "propellant", "price_major": 40 } ],
          "bids":  [] },
        { "org": "coop", "storefront": "neighborhood.ceres_freeport.item.chandlery_cage",
          "agent": "coop.clerk", "signatory": "neighborhood.ceres_freeport.npc.akintola",
          "stock": [ { "good": "provisions", "qty": 2600, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "provisions", "price_major": 10 } ],
          "bids":  [ { "good": "industrial_parts", "price_major": 560, "intake": 5400 } ] }
      ],
      "routes": { "stickney": [ "ceres_stickney" ] }
    }
  }
}
```

(The `coop` org's `capital_major` becomes 3,100,000 for the fittings bid; §4.3.) Ask quantities are
never authored: an ask lists the live pool. **`reference` names listings, never prices** (D-18-24):
`<port entry>/<org>` is the origin ask the share is measured against, read live at settlement, so the
applier's re-price and P-A-4 move the reference with the board; the validator refuses a reference whose
org has no ask for that good. **`routes` is a non-empty list per destination** (D-18-47):
at departure the planner plans every listed route and takes the fastest, ties to the earlier listing;
`+ship/order depart <port> [via=<place>]` parses `via` and refuses it "not yet" in the slice; an order
stores the destination, never a route or a duration. `hull_names` are the Trust's **laid-up** names,
replaced by the pilot's choice at `+coop/enrol` (the registry mark stays); no hull is pre-named for a
transcript. Stickney's entry has no `programme`, orgs `kasei` (capital 600,000), `stickney_yards`
(50,000, now a vendor) and `yard_commissary` (50,000), vendors Kasei (stock alloys 7,000 t and propellant
5,000 t; asks alloys 400 and propellant 60; bids metals **280** with intake 4,000 t and volatiles 200
with intake 9,600 t; signatory `neighborhood.stickney_yards.npc.achterberg`, the receiving clerk),
Stickney Yards (storefront `neighborhood.stickney_yards.item.yard_spares`, agent
`stickney_yards.clerk`, stock `industrial_parts` 5,400 t at 400; signatory
`neighborhood.stickney_yards.npc.castillo_haugen`) and the commissary (provisions 2,600 at 10; signatory
`neighborhood.stickney_yards.npc.okonkwo`, the storekeeper), `rooms` for the gallery, customs,
harbourmaster, exchange, receiving, commissary, **yard** (P11) and spine, and `routes: { "ceres":
[ "stickney_ceres" ] }`.

**As built (IC3, IC8, IC10, IC11):** the entry ships as written at lane v3. The reference is read **at
contract build**, not "live at settlement" (IC10 (6); §4.1 as built). `routes` is planned at departure
by `choose_route` over the port's list (IC9); `via=` is refused "not yet" (`VIA_NOT_YET`).

**The `PortApplier`** (tacsim/src/content.rs, phase 11, on the `ArenaApplier` pattern). Create, in order,
all as normal-lane writes and **submitting nothing**: (a) `open_passive_domain(location room, gallery)`
after checking the gallery's `RoomGeo` and `port.dock` marker; (b) the treasury System account if
absent; (c) per org: `mush_org::submit(CreateOrg)` + root scope, a scoped `open_account` (forbid),
capitalization by one-shot `create_system_obligation` tranches of ≤ 800,000 cr, each tranche's map row
written in the same batch as its obligation (the sole dedup, as in Draft 0.1); (d) the goods catalog,
once, from `goods.json`; (e) per vendor: `create_vault(Container(storefront item))`, `register_vendor
(venue)`, a dedicated stock-clerk Thing with an officer grant carrying `vault.withdraw` in the org scope,
plan rows `content.stock/<pack>/<entry>/<org>/<good>/<issue>` → {issue id, vault, good, qty, grade,
price}, `Wanted` documents for bids; the Draw listings are made by the tick once a pool exists; (f) the
pool: `pool` hulls, each a Thing (owner = content owner) with `create_unit`, seven Rooms and five exit
pairs from the interior template, `sim.interior` on the airlock and the stowage, `Transition::Space` and
`Transition::Members`, the `berths` compartment, placed Direct on the port domain (laid up: no lease),
`issue_title` to the Trust under `tacsim.registry`, named from `hull_names` with the registry mark. It
persists an applier-owned map `{domain, orgs, accounts, vaults, vendors, clerks, stock rows, bids,
hulls{hull, space, store, rooms, exits}}` in `TacsimStore` that fails closed when corrupt (the `tacsim:seed`
key rule, `store.rs:5-8`), and re-applies by map row, never by name. **Update** rewrites prices, intakes,
names and references in place and never re-capitalizes or re-issues; a bumped `issue` on a stock row
whose receipt exists is **refused and paged as drift** (Q-18-19's default; the alternative, a receipted
top-up, is the owner's to choose). **retire_check** refuses while a hull is leased or a vendor has live
contracts. **Retire** withdraws asks and bids; matter, receipts, orgs, accounts and hulls stand.

**As built (IC3, IC4, IC8):** the applier lives in `tacsim/src/port/apply.rs`. Its update semantics are
**"never re-capitalize, rename or re-host"**: org renames are refused as drift (mush-org has no rename
op; IC3 (2); renames are P5), and host drift or an already-hosted domain is refused in `put` (IC3's
residual). Q-18-19 is taken stricter (IC8 (2); §4.3 as built). Retire withdraws the Draw listings and the
bids. The map rows record the clerk (one per port, IC8 L2), the boards' rooms by role, the storefront
vaults, vendors, `vault.withdraw` grants, stock plan rows and Wanted bids, re-applied by map row with
every drift refused before any write.

**Boot-once acts and the readiness report.** Everything above runs inside the content module's boot pass
in pack order: starter (3), proving ground (2), the four station packs, `tacsim.lane.ceres_phobos`. A
never-latched id applies at ANY boot, including dev1's kept world; an anomaly aborts the boot. Nothing
per-player runs at boot. The stock arm then seeds every pool over the next ticks (one step per clerk per
fire, receipts as the guard, resumable across a reboot). **`@tacsim/port status`** (staff, read-only)
lists per port: rooms resolved, domain open, orgs capitalized (settled), pools seeded against their
receipts, listings equal to their pools, hulls titled and laid up, world rate. **The readiness gate on
`+coop/enrol` grows with the cuts** so no cut's verify waits on a later one: at IC6 it checks the
lines IC3 and IC4 bind (rooms resolved, domains open, orgs capitalized, hulls titled and laid up) and
the rate; IC8 adds pools seeded and listings equal to pools; IC12 lands the full report and the gate
over all of it. The rate line reads "24 outside test worlds", where a **test world** is a composition
built with the test flag the `tacsim` composition builder gains in IC3 (`test_world: bool`, set only by
the test harnesses' engine builder, never by any config key, so dev1 can never be one); at rate 3600 in
a test world the gate passes, on dev1's `IC_RATIO=4` it refuses.

**As built (IC3, IC4, IC6, IC8, IC10, IC12) — the readiness report and its gate.** `@tacsim/port status`
prints `readiness_in` (`tacsim/src/port/readiness.rs:413`), whose world-only half is `readiness`
(`:225`), in this order: **lane** ("lane: {pack}@{v} applied; this server ships @{v}." — flagged for an
older or unloaded version); **sky** ("sky: constant (rate 0), epoch 2351-05-13, rev {12 hex}" at the
shipped rate; flagged with "; {body} has no epoch_ic, so it moves from the server's epoch." when the sky
rate is non-zero and a lane body lacks `epoch_ic` — IC12, `body_without_epoch`, `readiness.rs:816`);
**world rate** (ready at 24 "world rate: 24 IC seconds per real second.", ready at any rate in a test
world, otherwise flagged "world rate: {r} IC seconds per real second; play runs at 24.");
**treasury**; **market facts** and **inventory facts** (IC8, IC10); then per port its **map** (only when
absent or unreadable), **rooms**, **domain**, one line per **org** (account and capital; IC3 L14),
**programme**, **hulls** (", {n} under way", ", {d} departing"), one **pools** line per vendor, one
**listings** line per ask and one **bids** line per bid (IC8); the summary last ("Ready: every binding
above is in place." only when every line is). The report's lane pack is the latched version when this
server loads it, else the shipped (IC3); `@org/list` shows the orgs only and capital reads here (IC3 (3)).
**The gate on `+coop/enrol` is every line but the sky** (IC12; `gated`, `tacsim/src/port/coop.rs:526`,
an exhaustive match, and `enrol_gate`, `:567`): §4.7's "the gate over all of it" is read with D-18-50,
which keeps the sky reported and never gated. Treasury and Bids gate since IC12 (IC8 L8 had reported
Bids without gating them): a missing treasury account, or a hand-withdrawn or apply-waiting bid, closes
enrolment; an exhausted bid reads ready. **As built (P-B, Q-18-60 R10):** the Bids line gates on Missing
alone (`shuts`, beside `gated`): a lapsed bid the clerk re-posts, or a bid whose house cannot fund one
unit ("not buying until its capital settles"), reads pending on the report, leaves the desk open, and is
paged once a boot by the port tick (`page_unfunded_houses`, `PortTickState.funds_pages`); the sales
against such a house skip with `HOUSE_CANNOT_PAY_SKIP`, as they did before IC12. The player never sees the failing line: `+coop/enrol` logs it and answers `NOT_OPEN`,
"The Co-op is not enrolling yet: the programme is still being set up." (`coop.rs:119`). A moving sky
over a lane body without `epoch_ic` is also paged once per boot by the port tick (`page_moving_sky`,
`tacsim/src/port_tick.rs:461`; `PortTickState.sky_paged`). **"At rate 3600 in a test world the gate
passes" is superseded for the story test** (IC12 O6): the story test boots the production composition
(`build_engine`) at `[game].ic_ratio = 24` and never threads `test_world` into a mush-server boot path —
IC3's deferral of that threading is closed as not needed; the production composition is never a test
world (IC3 L15).

**Idempotency.** As Draft 0.1, plus: stock issues are deduped by the `tacsim.port_stock` receipt (the
derived LotId refuses reuse while the lot lives, `pool.rs:1280-1292`; the receipt outlives the dissolve);
the stake is keyed by the lease contract; the burn by hull/order/attempt; the lot leg by contract.

**Config flips on dev1.** `[economy]` per-instrument mint-cap override for `tacsim.credit` = 1,000,000
(tranches of ≤ 800,000). `[tacsim.economy].market` stays false. `[onboarding].start_room` stays 0.
`[tacsim].pack_paths/seed_paths` stay `[]`. World rate to 24 by `@tacsim/time rate=24` **after the owner
approves a redeploy; never redeploy without the owner's ask** (memory rule).

**As built (IC3, IC12):** the `tacsim.credit` mint cap is contributed in code (1,000,000 major,
`PORT_MINT_CAP_MAJOR`) unless an operator sets a per-instrument value, so the `[economy]` override line is
optional (IC3 (1)); `[tacsim].enabled = true` with no pack or seed path is the whole game switch (the
story test's config). The rate-24 deploy line, the kept-world pack upgrade by `@atlas/pack plan` then
`apply`, and the pre-deploy blocker on selling are in `docs/ops.md`, "The trade run (doc 18)" (IC12).

### 4.8 The sky: the slice's constant sky, and the astronomy engine it must not foreclose (R9)

**What the owner wants from it.** belter-charts (`github.com/skylercole/belter-charts`, Apache-2.0;
fetched 2026-10-15: "unofficial navigator for The Expanse's solar system — real ephemerides, canon
locations, Epstein flight planner"; JPL Horizons state vectors packed per day for 2340–2365 in
heliocentric ecliptic J2000 Float32, regenerated by a `fetch-horizons` script; a 3D map of planets,
moons and asteroids; origin/destination/hull/acceleration inputs; a flip-and-burn plan with travel time,
flip point, peak velocity and light lag; an "honest physics" vs "canon-feel" toggle; a date scrubber)
is the inspiration, not a source: the engine imports public JPL data through its own script and takes
none of belter-charts' code wholesale and none of The Expanse's canon locations or names (D-18-51). What
it should give this game: a trading path that was optimal last month and is less optimal now (route
optimality drifting over months as the bodies move); Pallas at high ecliptic latitude being awkward to
reach and paying more for emergency supplies (prices that read position); light lag that reads real
positions (doc 15). **R9 does not ask the slice to move the sky.** It asks the slice to stop treating a
frozen sky as something the code may rely on.

**The regrets avoided** (each the design's finding, verified by its judge; each S):

| Draft 0.2 decision | Why the engine would undo it | Draft 0.3 |
|---|---|---|
| The coast rate 205 km/s as "a constant the test owns"; authored `base_s` the only truth (§4.4, IC2) | A moving sky has no one duration per leg; the engine would move 205 into drive data and rewrite the test | The rate is drive data (`drive.law.coast.kms`); `base_s` stays authored as the pack's pin of the constant sky; IC2's test derives each leg from the sky and refuses if the pack disagrees (D-18-45). The engine then stops reading `base_s`; it deletes nothing |
| `RouteDef.distance_km` "the sum of the legs" (IC2) | The code pins the direct chord (`store.rs:800-813`), the one cached distance a moving sky makes most misleading; a field with two meanings by route kind is a trap | The field stays the direct chord at the epoch, relabelled; the plan's per-leg `dist_km` is the truth; nothing in the slice reads `route_duration_at` |
| A mush-sim `route_leg_durations(route, drive, payload)` (IC9) | No sky, no departure instant, no per-leg bodies: every caller rewired at the flip; mush-sim must stay sky-free | tacsim `plan_passage` over the sky at departure; mush-sim keeps `laden_leg_durations(unladen, drive, payload)` (D-18-44) |
| `routes: {dest: one id}` in the port entry | Direct and via-Kirkwood routes coexist after the flip and the best changes by season (Kirkwood laps Ceres) | A list per destination; the planner takes the fastest; `via=` reserved (D-18-47) |
| The flat 105‰ burn as THE fuel function and `"law": "coast"` as a bare word | A flip law's burn depends on distance; a string law carries no parameters; `thrust_permille` is meaningless beside an absolute speed | `passage_burn(law, dry, payload, legs)` over a tagged `DriveLaw`; Coast returns today's figure (a doctrine fraction, distance-free by construction, not the rocket equation; whether mass slows the cruise or grows the burn is Q-18-41); one class, one law (D-18-45) |
| The ticket freezing only `leg_s`, `burn_t`, `crew_days` | The chart, doc 15 R5 and a cross-flip audit need what each leg assumed; seconds alone cannot be re-derived; adding fields later is a typed-family wire change | The ticket carries the whole `PassagePlan` (D-18-46) |
| The epoch as server config `= 0` while phases are tuned to no date (`config.rs:76-82`; `tests/ephemeris.rs:66-86`) | Under rate > 0 one pack gives a different sky per server, and `epoch 0` puts every body about 380 IC years off at IC 2350; doc 10 T17 itself puts the epoch in the pack | `BodyDef.epoch_ic: Option<i64>`, emitted only when present (digests hold), carried into `Body` and read by `phase_mdeg_at` in place of the config epoch when present (today `Body` has no epoch and every body advances from `self.epoch`, `ephemeris.rs:44-52,146`); the lane pack's bodies author it at a real IC date; Ceres, Mars and Pallas at their **true** heliocentric longitudes and **heliocentric distances** on that date, never mean elements (Ceres' equation of centre reaches 2e ≈ 9° and its radius ranges 2.548–2.984 AU, ±30 % on a 0.741 AU leg), so the circular sky is exact at the epoch and drifts only after it; Kirkwood's orbit is Q-18-36's (D-18-48). A date with the two true longitudes within ≈ 10.9° (a 1.302 AU via-Kirkwood lane) recurs each 3.18-yr synodic cycle for a season whose length the script measures on true positions, so one lies within about 1.6 IC years of dev1's persisted IC; inert at rate 0 |
| Kirkwood "on the Ceres–Mars lane" as a permanent fact (doc 11 §2/§5; 18a "two to three weeks") | At 2.06 AU (1,080 d) it cannot co-orbit Ceres, and a three-body via-Kirkwood season recurs once in decades, not once a cycle (§3-Q (4)); a depot on an independent orbit is off the lane the day the sky moves | Kirkwood authored at its epoch position; its orbit is the owner's (Q-18-36, lane-tied default; D-18-48 provisional); the slice keeps via-Kirkwood as its only route; the lore says "in this season" and "in a good season" |
| `spacecraft` still in `SELF_MODES` (`travel.rs:90`; Q-18-6 owed) | Atlas `+travel` can fly the epoch projection: a second, wrong travel path once the sky moves | Q-18-6 is binding under R8: IC1 removes it (and `options_from_corridor_room`) with a test that no player verb traverses a spacecraft connection, covering `CORRIDOR_MODE` (`content.rs:44`) so a renamed mode cannot reopen it; the atlas drawing is a projection no code reads as truth (D-18-49) |
| "All under a day" as a property of the lane (§4.4, the readiness gate, the story test) | A coast Dray is sub-day only while Ceres–Mars ≤ ~1.6 AU, about 16 % of the cycle; a gate or help text asserting it would lie at the flip | Stated "at the slice's constant sky"; the gate reports the sky and never checks a duration; help says passage time depends on load and, later, the season (D-18-50) |
| `fill` and the buy clamp computed from route constants (IC11) | Crew-days follow durations, which become seasonal | Both call `plan_passage` at the depart order's `not_before` (D-18-41) |
| "the pair the T17 epoch was chosen for" (§1), "T17 (the pinned sky)" (§7), `@corridor/show`'s "the sky is frozen at the campaign epoch" (`tacsim/src/commands.rs:435`), `route.rs:231`'s "a frozen sky has no departure windows" | They state as permanent a ruling R9 limits to the slice | "T17's constant sky, honoured for the slice and superseded after it by R9 (cut P-A)"; the UI string "constant sky at epoch <date> (T17)"; the comment "a constant sky has no departure windows; the planner computes per departure" (IC2) |

**The seams the slice builds, which the engine reuses unchanged** (D-18-43/44/45/46/47):

```rust
// crates/tacsim/src/sky.rs (IC2). An enum, not a trait object: serde-free, matchable, deterministic.
pub struct PosKm { pub x: i64, pub y: i64, pub z: i64 }          // heliocentric ecliptic J2000, km
pub enum Sky { Circular(Ephemeris) /* P-A: Kepler(ElementsSky), Tabulated(VectorSky) */ }
impl Sky {
    pub fn position_km(&self, body: &CanonicalId, t_ic: i64) -> Option<PosKm>;   // z = 0 while Circular
    pub fn distance_km(&self, a: &CanonicalId, ta: i64, b: &CanonicalId, tb: i64) -> Option<u64>;
    pub fn revision(&self) -> DefinitionDigest;   // the pack digest type over body data + model + epochs + {epoch, rate}
}
// crates/tacsim/src/port/passage.rs (IC9)
pub enum DriveLaw { Coast { kms: u32, burn_permille: u32 } /* P-A: Flip { accel_mg: u32, exhaust_kms: u32 } */ }
pub struct HullDrive { pub class: CanonicalId, pub law: DriveLaw, pub payload_ceiling_kg: u64 }
pub struct LegPlan { pub to: CanonicalId /*place*/, pub body: CanonicalId, pub dist_km: u64, pub secs: u32, pub t0: i64, pub t1: i64 }
pub struct PassagePlan { pub route: CanonicalId, pub law: CanonicalId, pub depart_ic: i64, pub sky_rev: DefinitionDigest,
                         pub legs: Vec<LegPlan>, pub total_s: i64, pub burn_t: u64, pub crew_days: u32 }
pub fn plan_passage(sky: &Sky, pack: &RulesPack, route: &CanonicalId, drive: &HullDrive,
                    dry_kg: u64, payload_kg: u64, occupants: u32, depart_ic: i64) -> Result<PassagePlan, PlanError>;
pub fn choose_route(sky, pack, routes: &[CanonicalId], drive, dry, payload, occupants, depart_ic, via: Option<&CanonicalId>)
    -> Result<PassagePlan, PlanError>;     // fastest total_s; P-A gives it a policy (fastest | cheapest) since a flip's fastest route burns most
pub fn passage_burn(law: &DriveLaw, dry_kg: u64, payload_kg: u64, legs: &[LegPlan]) -> Result<u64, PlanError>;
// crates/mush-sim/src/route.rs (IC9): sky-free
pub fn laden_leg_durations(unladen_s: &[u32], drive: &DriveProfile, payload: u64) -> Result<Vec<u32>, RouteDefError>;
```

In the slice `Sky::Circular.distance_km` **delegates to `separation_km`** (`ephemeris.rs:188-195`) so
fixture routes stay bit-identical and no second rounding path exists; the i128 vector distance arrives
with `Sky::Kepler`. `position_km` is new code with no slice caller but its tests. `plan_passage` plans
over the symbolic `RouteEntry` while `depart` flies the materialized `RouteDef` from `route_in_world`;
both are keyed by (pack, version, route id), so a v1-pinned consumer (Q18A) plans and flies one version.
`PlanError`: `UnknownRoute | UnknownDriveClass | UnknownBody | NoSkyLeg | Overloaded | Overflow`. Callers:
`+ship/plan` (depart_ic = now; writes nothing), `+ship/undock` and the queue's depart (now; the result is
the burn's input), `fill` and the clamp (the depart's `not_before`), `choose_route` over the port's list.
**Prices** stay listing data the applier writes (§4.7 "Update rewrites prices in place"); quotes freeze
their price; the programme's references are live reads of the named origin asks, so only the share
permille and the buyout freeze at signing (D-18-24); no trait is added — the engine's price policy is
a second writer with the applier's authority. **The atlas drawing** stays an epoch projection. **The
readiness report** prints "sky: constant (rate 0), epoch <date>, rev <digest>" and pages if the sky rate
is non-zero while any lane body lacks `epoch_ic`. T17's own coherence property — every pair sharing a
body changes together (`ephemeris.rs:11-15`) — is preserved by having one query.

**As built (IC10, IC12):** the references are read at contract build (IC10 (6)), not live at settlement.
The readiness sky line and its promise are built (IC12): "sky: constant (rate 0), epoch 2351-05-13, rev
{12 hex}" at rate 0; at a non-zero sky rate with a lane body lacking `epoch_ic` the line is **flagged**
and the port tick **pages once per boot** (`tacsim/src/port/readiness.rs:816`,
`tacsim/src/port_tick.rs:461`) — never a gate (D-18-50). **Since doc 19 cut A.6** the line names the
sky's model and motion — "sky: moving (rate 1000, real motion), kepler, epoch 2351-05-13, rev {12 hex}"
on a shipped server, "sky: constant (rate 0), kepler, …" at rate 0 — and the shipped templates set
`rate = 1000` (Q-18-37's default; the Rust default stays 0 for the pins).

**The calendar, in one place.** The IC clock is Unix seconds, continuous across rate generations
(`clock.rs:214-218,335`), so JD = ic_s / 86,400 + 2,440,587.5 and `2350-01-01T00:00Z` = 11,991,628,800.
A sky rate of 1000 makes the sky's date the IC date, which real data requires. Each body's `epoch_ic`
is the IC instant whose calendar date its elements were fetched for. dev1's current IC is read from the
persisted timeline (`mushd.toml.example:105`, "NOT reset at each boot"), never from `instance.env`'s
`IC_EPOCH`/`IC_RATIO`, which seed generation one only. Test worlds sit near IC 2030
(`tests/ephemeris.rs` epoch 1,900,000,000), where a 2340–2365 vector table is out of range while
per-body elements are self-referenced anywhere — one more reason for elements. At world rate 24 one IC
year is about 15.2 real days; the Ceres–Mars synodic period (3.18 yr) about 48.5 real days, best to worst
in about 24; Pallas's orbit (4.62 yr) about 70 real days and its latitude cycle about 35; belter-charts'
2340–2365 window would run out about 228 real days after IC 2350.

**As built — a correction (IC2's lead ruling R3; carried to IC12):** the tree's calendar is **not** Unix
seconds and not Gregorian. It is `mush_core::CivilDate` (`mush-core/src/clock.rs:618`, `from_ic_seconds`
at `:636`): 365-day years with no leap days, year 0 at IC 0. So 2351-05-13 = 74,152,540,800 IC s (the
lane bodies' `epoch_ic`) and 2350-01-01 = 74,109,600,000; there is no Julian-day mapping, and a real-date
import maps a Gregorian date to `CivilDate` by its fields (year, month, day). The test worlds' IC
1,900,000,000 is year 60, not 2030. The lane's legs are real-date legs at 2351-05-13 at the Dray's 226 km/s
(IC2's lead ruling; D-18-29/30/45 as built); Kirkwood is authored at 2.06 AU on the chord with a
placeholder period of 1,080 days (Q-18-36 open). **P-A-1's date mapping must read through `CivilDate`**;
the P-A brief below predates this correction and is left as written.

**Post-slice cut P-A: the astronomy engine** (P-SKY in the review; L overall; after IC12, with P5
restock for its price policy and P7 Pallas as a port for its showcase). Supersede T17 as amended: the
corridor's and the inner system's bodies move on the IC calendar in three dimensions from real orbital
data; the slice's one planner computes every departure from the sky at that instant under coast and
flip laws, with intercept and direct routes beside via-Kirkwood; a chart shows the sky; awkwardly placed
ports pay more; light lag reads positions; committed passages are untouched; the flip is one boot.
**P-A-1 data and model (M).** `BodyDef` gains `Option` fields skipped when absent (`ecc`, `incl_deg`,
`node_deg`, `peri_lon_deg`, secular rates, `parent` + `local_orbit_km`, and `a_au` with the mean
anomaly at `epoch_ic`); `orbit_au` stays the heliocentric **distance** and `phase_deg_at_epoch` the
**true** longitude at `epoch_ic`, exact at the epoch under both models, and with the Kepler fields
absent a body is circular at that radius, so every existing pack is a valid e = i = 0 sky; element
sets by era. `Sky::Kepler` solves Kepler's equation in f64 with a fixed Newton count,
rotated to ecliptic J2000 and rounded to km (platform drift is harmless: committed plans are frozen).
Bodies: mercury, venus, earth (+ luna), mars (+ phobos, deimos; Stickney keeps naming mars until a local
route needs the 9,376 km offset), ceres, pallas (i ≈ 34.8°), vesta, hygiea, jupiter, kirkwood (station
elements), later saturn/titan. A small committed script beside the fixture (no `xtask` crate exists)
regenerates osculating elements at era epochs and a quarterly heliocentric vector golden table for
2340–2400 from JPL Horizons (≈ 240 × ~10 bodies × 24 B ≈ 58 KB), asserted within ~0.5 %; packed daily
vectors as belter-charts ships them (state vectors ≈ 219 KB per body per 25 years; 110 KB positions
only) stay an optional `Sky::Tabulated` for a near window (Q-18-38). **P-A-2 laws (M).**
`DriveLaw::Flip` (secs = 2√(d/a), a scaled by the laden law; Δv = 2√(d·a); burn by the rocket
equation); coast optionally `exhaust_kms` (≈ 3,700 km/s reproduces 105‰ at 205 km/s for the
**unladen** hull only) and `max_kms` for a cruise choice, once Q-18-41 has chosen between a
mass-dependent cruise (the laden law kept) and a mass-dependent burn (the laden law dropped); the
slice's intercept loop (first-order on tangential lanes); a solar keep-out inside 0.3 AU as a detour
cost, never a closure (Q-18-40); direct route ids beside via-Kirkwood in lane pack v2; `choose_route`
with a policy; `via=` honoured. Full-outbound stretches: coast up to 3.30× (19.6 h → ~65 h), flip
1.82×. **P-A-3 chart (TUI S, web M)** on the one OutputEvent/Panel protocol: `+chart [port]` (AU,
light lag, laden ETA for the best route and the alternative, "next best window" = a scan of
`plan_passage` at 1-IC-day steps over 2 IC years, ≤ 730 evaluations), `+chart/body <id>` (heliocentric
distance, ecliptic latitude), a window line on `+ship/plan`, a web cockpit panel (top-down XY with a
z-exaggeration toggle, ports, ships under way interpolated along their tickets' frozen legs; the TUI at
120 columns); the atlas sites re-drawn at the flip or retired for the chart. **P-A-4 price policy
(M).** A once-per-IC-day arm on the `tacsim.port` tick writing listing and Wanted updates with the
applier's authority, each carrying its org's officer of record as signatory (in fiction the factor
re-prices the board each IC day; nothing automated signs, 11 §2, and the `port` schema already refuses
an unsigned listing): reach(port, good) = the minimum nominal-cruise passage seconds from the supplying
ports at now; a capped premium on demand-side bids as reach rises against a synodic median; emergency
Wanted bids for life-support goods when reach crosses a threshold, funded from the org's finite capital
(doc 10 7A; nothing minted) — the owner's Pallas paying more when awkward; quotes stay frozen, and the
programme's reference reads the moved ask, which is the point (Q-18-39; D-18-24). **P-A-5 comms (S).**
Doc 15 R5 from `Sky` positions and a ship's
frozen legs; the lag table a live render; D-15-11 honoured with the route's light distance from the
plan. **P-A-6 flip and migration (S).** Nothing to rewrite: in-flight journeys keep their embedded
`RouteDef` and `leg_s`, burned tickets their plan and `sky_rev`, orders their destinations and UTC
times, quotes their prices; one owner-approved boot with lane pack v2 and the sky knob collapsed to
`constant | real` ("sky motion"), `route_in_world`/reindex re-materializing while v1-pinned consumers keep
v1 (Q18A); a boot test that a mid-passage journey arrives at its frozen instant across the flip and a
burned-unlaunched ticket launches with its frozen `leg_s`; a golden test that the real sky is
**identical** to the slice's circular sky at the epoch (true longitude and distance authored, D-18-48)
and drifts from it afterwards only by the e/i terms, the drift per IC year measured and recorded in
the cut note; lane pack v2 re-authors every `base_s` from the real sky and IC2's
`lane_legs_derive_at_epoch` retires with v1 (the v1 pins stay as v1's). Open for the owner: Q-18-35 to
Q-18-41.

**The doc 10 §22 T17 amendment (owed, §10; the text).** Heading: *"22. Addendum — T17, the frozen sky
(owner, 2026-09-09, binding; amended 2026-10-15: the constant sky of the first slice, superseded after
it by the astronomy engine)"*; the 2026-09-09 body kept; appended: *"**Amendment (owner, 2026-10-15; doc
18 R9).** T17 now governs only the constant sky of the first IC vertical slice (doc 18). After that slice
it is superseded by the astronomy engine (doc 18 cut P-A): real ephemerides in three dimensions, with
eccentricity and inclination (Pallas far from the ecliptic); bodies moving on the IC calendar, so a
route's distance, duration and fuel depend on when it departs; the best route between two ports
changing over months; a port's position moving its prices. Until P-A lands `rate = 0` stays configured
and every slice figure is the constant sky's output at its epoch, never an invariant: no code, test or
help text may treat a distance, a duration or a price as constant beyond a frozen commitment. The slice
reaches the sky only through the seams the engine will reuse — one sky query (the distance between two
bodies at two instants) and one passage planner over the sky at departure, the route's legs as body
pairs, the drive's law and the departure mass — and derives durations rather than relying on authored
seconds. **What stands:** 1A's scales and compression (local orbit in hours, the inner system in days
to weeks, the Belt in weeks, the outer system in weeks to months, about one OOC hour per IC day); 1A's
rule that transfer conditions modify routes rather than closing destinations (no launch windows; every
port reachable at a cost); T1's freeze, no longer moot — a passage's per-leg distances, durations, burn
and crew-days are computed once at commitment and frozen into the ticket and the journey, and nothing
after the burn re-reads the sky; T17's coherence (every pair sharing a body changes together), kept by
the one query. **Superseded, after the slice:** 'No body moves. No transfer window opens or closes.
Nobody waits for an alignment' (bodies move and distances vary; still no window closes a destination);
the circular, coplanar model as the final model (it remains the e = i = 0 case of the engine's
elements); 1A's rejection of simulated ephemerides ('long non-opportunity' is Q-18-35 by another name,
re-opened knowingly). Travel time becomes the planner's output: the sky at departure, the drive's law
(coasting at a cruise speed, or a constant-acceleration flip) and the departure mass."* The epoch's
move from server config to the pack's bodies is not a doc 10 supersession (T17 always put it in the
pack) but doc 14 §3.8 S7's and `config.rs`'s, recorded in §10.

---

## 5. The cut plan

Sizes: S ≤ a day, M a few days, L a week-plus, honest after the buildability review. One commit and push
per cut, with the Fable trailer, as the 16b/16c loop does; each cut's record goes to
[18-cut-notes.md](18-cut-notes.md). Two lanes can run in parallel until **IC8, where they join** (the
first cut that needs both IC5's provider and tick and IC6's enrolment): a **content/sim lane** (IC1 →
IC2 → IC3 → IC4 → IC6: mush-atlas content, tacsim content and definitions, mush-sim's
gate/graph/transition) and a **market lane** (IC5 → IC7: mush-core/engine registration, mush-economy
`contract_live.rs`, mush-market `trade.rs`/`lot.rs`/`api.rs`/`module.rs`, mush-inventory). The lanes
are disjoint at the **crate** level for mush-atlas/mush-sim against mush-market/mush-inventory only;
both touch tacsim's `composition/mod.rs` (IC5's `with_quantity_delegates` and provider/tick
registration at `:398-404`; IC1's boot-pack list in `content_module_with`, `:762-787`, and the atlas
shelf, `:690-707`), tacsim's `lib.rs:398`/`commands.rs:69` registration (IC3's `+credit` and
`@tacsim/port status`, IC6's `+coop`/`+ship`, IC5's tick) and mush-economy (IC4/IC6 against IC7). So:
IC5's composition and registration edits land first as one small standalone commit, in a file of
their own (`tacsim/src/port_tick.rs`, registered from `lib.rs` in one line), and the content lane
rebases on it; the mush-economy touches are sequenced IC7 before IC6. IC0 has landed (`c87d78f7`), so
nothing rebases on it. **Cut numbering across drafts:** Draft 0.1's IC5 (the market seams, named in
IC0's test comments and cut note as "cut IC5's `with_party_cap_source`") is Draft 0.2's **IC7**; Draft
0.2's IC5 is the tick skeleton. **IC9** is the first player-runnable flight, **IC10** closes the loop
interactively, **IC11 is the first cut after which the full daily loop — queued orders included — is
runnable.**

| Id | Name | Scope | Crates | Size | Depends on | Verify lens |
|---|---|---|---|---|---|---|
| IC0 | Engine blockers first (**landed, `c87d78f7`**) | As Draft 0.1 and [18-cut-notes.md](18-cut-notes.md): Lot short-circuit; listing TTL ≤ hold TTL and the hold read at accept; non-staff manual route closed; major-unit money lines; economist answers the asker; the lot-tour driver; the IC0 gate test. `spacecraft` out of `SELF_MODES` still owed (Q-18-6) | mush-market, mush-sim, mush-atlas | M | — | per the cut note; the Draw path relies on the hold read only for player lot listings |
| IC1 | Two stations, the IC door and the IC-return rule (**landed**, `07232d09`) | The four atlas packs (13 + 12 rooms, three decks at Ceres; wall features on every exitless partition; themes; thirteen NPCs with fallbacks; principals `enabled:false`; lore pools); `RoomDef.markers` + the `port.dock` topology seed (`validate.rs:1958-2000`); unlocked `exit.limbo_arrivals`; the `tacsim.last_port` stamp, the door hook and `+ic`; boot list + shelf; validate-clean, digest-pin and kept-world boot tests; **`spacecraft` out of `SELF_MODES` and `options_from_corridor_room` (Q-18-6, binding), a sky prerequisite** | mush-atlas, tacsim | M | — | fresh and kept-world boots apply; `c` lands in C03; every public room walkable; Stickney validates with no foot route; the adjacency check over 18a §0.4's boxes: every plain `door` shares an edge (`geometry.portal_gap` is an error and the applier refuses it) and every exitless seam is a `wall`, walled on `+map`; `o`→`x`→`c` from a Stickney position refused with the `+ic` hint; starter digests unmoved; `no_player_verb_traverses_a_spacecraft_connection` (covers `CORRIDOR_MODE`); T-45 |
| IC2 | The Dray, the goods table, the lane and the sky seam (**landed**, `484647bc`) | `dray.json` (class hauler, `logistics` with D3 limit maps and the tagged `drive.law`, `interior`) in `fielded_sources` + pin; `UnitClass::Hauler`; the lobby filter + custody refusal test; `goods.json` (six rows, the `measures` registry, binding mass **and** volume per row) with its validator and pin, deriving the catalog; `port/capacity.rs` (`load`/`admits`/`max_more`) with its pure tests (volume binds on ice and mass on alloys; 86 t ice, 19 + 81 and 101 t alloys refused on the right measure; the m³ formatter truncates); `tacsim.lane.ceres_phobos` v1 (fresh id; bodies `ceres`, `mars`, `pallas` at their **true** heliocentric longitudes and distances on a real date, with `epoch_ic`, from the committed Horizons script; `kirkwood` tuned at its 2.06 AU epoch position with a placeholder period pending Q-18-36, D-18-48; three locations; two routes at 540,800 + 409,200 s, `propellant 0`; `routes` lists in the port entries) via the game-packs list; `sky.rs` (`Sky::Circular`, `distance_km` delegating to `separation_km`, `position_km`, `revision`); `BodyDef.epoch_ic: Option<i64>` **carried into `Body.epoch_ic` and read by `Ephemeris::phase_mdeg_at`** in place of the config epoch when present (today every body advances from `self.epoch`, `ephemeris.rs:44-52,146`; without this IC9's rate-1000 test cannot be written); the one-class-one-law drive validator and `drive_for_class`; `@corridor/show` per-leg AU lines worded "constant sky at epoch <date>"; the `route.rs:231` comment reworded; tests `lane_legs_derive_at_epoch` (each leg from the sky and `dray.json`'s kms, the pack within 100 s; refuses if `kirkwood_depot`'s body is missing; retired with lane v1 at P-A), `sky_circular_agrees_with_the_law_of_cosines` (within 2 km at the epoch and ±1 IC year at rate 1000; z = 0), `body_epoch_holds_its_phase_at_that_instant` (a body with `epoch_ic` set is at its authored phase at that instant under rate 1000), `fixture_digests_unmoved_by_optional_sky_fields`, `one_drive_class_one_law` | tacsim | M | IC1 | the five tests green; `@corridor/show ceres_stickney` prints two legs at about 0.741 and 0.561 AU and 540,800 / 409,200 s, never "the sky is frozen"; SIM UNIT never offers `dray`; `@tacsim/pack list` shows `tacsim.lane.ceres_phobos@1` beside the untouched `tacsim.corridor` fixtures; the goods validator refuses a family-less trade row and a `stow.bulk` row without volume; `tests/content.rs`'s pin decodes `dray.json` through `UnitSpec::check`, so a `note` over `MAX_NOTE_BYTES` (1,024) fails the cut, never the boot; shipped DefinitionRefs and fixture digests unmoved |
| IC3 | Ports, orgs, capital and readiness (**landed**, `ea52634d`) | `PackEntry::Port` + `PortApplier` a–c (two passive domains; treasury; the nine orgs with capital tranches, each tranche's map row in its batch; the Trust's scoped programme account; fail-closed port map; `signatory` required on every listing; `routes` validated as a non-empty list of routes from this location to that destination); mush-sim: `open_passive_domain` skips the `MAX_WINDOWS` guard for passive opens (`window.rs:205-206`, S); the composition's `test_world` flag; the template mint-cap override 1,000,000; `+credit` (posted, held, available); `@tacsim/port status` | tacsim, mush-sim, mush-server | M | IC1, IC2 | after boot and one sweep `@org/list` shows the orgs and capital; a double boot posts nothing; a corrupt map refuses re-creation; `@domain` shows both passive domains presenting into the galleries; **a kept world with eight windows open at restart still applies the lane pack**; the readiness report lists every binding and flags rate ≠ 24 |
| IC4 | Walking aboard: the boundary rule and the hull template (**landed**, `3c9cded5`) | mush-sim: `Transition::Members`, `member_of` + verify, the destination-aware gate (exit-only, member destination), projection returns the member location, the destroy-gate arm, `backing_valid` false for multi-room spaces, `egress` refused in them, narration over `space.rooms`, `pub declare_compartment`. tacsim: `PortApplier` part f instantiates 8 Drays (Thing + unit + 7 Rooms + 5 exit pairs + 2 spaces + Members + `berths`), titled to the Trust, named from `hull_names`, laid up on the Ceres domain | mush-sim, tacsim, mush-economy | M | IC2, IC3 | staff `place_interior` into an airlock, then `in`/`up`/`down` walk all six rooms with no placement write per step; `o`, `s`, `home` and `@teleport` by the occupant refused; a reboot keeps the walker in the Bridge; e7c6 and placement suites green; eight Drays listed in C01; `@asset/show` title Trust |
| IC5 | Tacsim's authoritative plumbing and the port tick (**landed**, `10b89d58`) | Tacsim's first action provider registration and typed families (`tacsim.port_stock` receipts, `tacsim.ticket`); tacsim joins `with_quantity_delegates` and gets the inventory handle cell (`composition/mod.rs:404`, pinned edge test); the `tacsim.port` normal-lane Coarse tick skeleton (5 s): rotation cursor, visit/work budgets, per-agent actor swap, at most one submission per actor per fire, the reconnect-notice store on `Connected` | tacsim, mush-server | M | IC0 | a test action submitted by an agent from the tick lands; two agents in one fire do not void each other; GOD never submits; tacsim reads inventory pools via `action_state`; the composition-edge pin updated |
| IC6 | The Co-op programme and boarding (**landed**, `3bf2bc90`) | `+coop`, `+coop/enrol` (account open + `offer_lease` + in-verb accept + keyed stake post + hold vault + enrolment doc; gated on the IC3/IC4 readiness lines and the rate only — pools and listings join the gate in IC8, §4.7), `+coop/statement` (readable anywhere); the lease-reading operate gate; the `page_stale_contracts` lease exemption (`sweep.rs:1597-1640`); the observer folding a staff-ended lease; `+ship`, `+ship/board`, `+ship/leave`, `+ship/hold`; the `look` status line (`look_rendered`) and per-room gauges | tacsim, mush-economy | M | IC4 | enrol once (25,000.00cr posted, lease Active, vault opened, hull renamed); a second enrolment from the same account refused; board and leave only by the tenant, only docked; after eight simulated days no stale-contract page; staff `@contract/cancel` revokes; `look` names the hull and the port |
| IC7 | Market seams for content ports (**landed**, `d8427081`) | `EconomyModule::with_party_cap_source` (after the frozen table, `contract_live.rs:5163-5169`); `Supply::Stock` + `Representation::Lot` with `LotOp::Draw` (seller-side source projection; synchronous quantity decrement); `with_venue_gate` with `Presence::{Here, Ship(hull)}` + `venue_of_room` + default `market=`/`to=`; the `CapacityProvider` hook (a `read_pool` closure returning `Result`, so absent = 0 and unreadable refuses) on the lot pre-check and the Draw prepare, a Draw refused at prepare cancelling its contract and returning the escrow with no page; refuse non-port listings into hull endpoints; the contract-keyed lot leg (`market.lot.<op>.<contract>`, e7r2 sweep). **Not here:** the lapsed-hold release — a hold's release is the `market_lot` action's release op under `listing_holder`, an authoritative submission the GOD lapse sweep may not stage (`tick.rs:28-37`; the IC0 cut note); it is IC8's tick arm | mush-economy, mush-market, mush-inventory | M | IC0 | production composition in miniature: a seeded pool, a Draw-backed ask; accept moves the pool quantity and the money once; the listing decrements; two buyers racing the last tonnes, the second refused before any contract; a gate refusal escrows nothing; a fake provider refusing on volume escrows nothing; a Draw that would exceed the limit at drain refuses fail-closed with the escrow returned; an unreadable pool refuses; a re-staged lot leg replays as a HIT; no `SettlementRefused` page |
| IC8 | Finite stock and port commerce (**landed**, `ee1d831f`) | `tacsim_port_stock` (mint + receipt) on IC5's provider; the stock arm (mint → `inv.dissolve` → list, one step per clerk per fire); `PortApplier` d–e (storefront items in C15/P15, vaults, vendors, clerks, grants, stock rows, Wanted docs); the tacsim `VenueGate`, party-cap source and `CapacityProvider` (endpoint → hull → stowage → `compartment_for` → `admits`, reading the hold's four pools); the resized port entries (Kasei metals bid 280; stock 9,600/4,000/7,000/5,400; the Stickney Yards vendor on `item.yard_spares` with P11 in the Stickney rooms; the Co-op's fittings bid and 3,100,000 capital; `industrial_parts` → the Yard's ask in the programme's `reference` map, read live by the policy); **the posted-price LP test** (both vertices computed from `goods.json`, the hull and the port entries' asks and bids and asserted optimal; never 20/80 or 55/45 by name); SOLD OUT and "not buying" rendering; the listing-vs-pool check and the pools/listings lines in the readiness gate; **the lapsed-hold release arm** on the `tacsim.port` tick: one lapsed player listing per fire, the tick acting as the listing holder by IC5's actor swap, the `market_lot` release op keyed by the derived reservation id, verified by pull (D-18-26; Q-18-31) | tacsim, mush-inventory, mush-market | L | IC3, IC5, IC6, IC7 (the two lanes join here) | after boot the ticks seed every pool exactly once (a kill and reboot mid-seed re-mints nothing; conservation from lineage and receipts); at C07 a no-argument list/quote/accept loads the hold pool; 101 t of alloys refused (mass), 86 t of ice and 19 + 81 refused (volume), each with nothing charged; from Limbo refused; into a hull docked at Stickney refused; the board decrements to SOLD OUT; the LP test green; a lapsed player listing's hold is released by the tick, never by the sweep; the applier submitted nothing |
| **IC9** | **Undock and fly — the first player-runnable flight** (**landed**, `1137c373`) | mush-sim `laden_leg_durations` (`route_duration` delegating to it) + `CarrierJourney.leg_s` + `pub mush_sim::depart(leg_s)`; tacsim `port/passage.rs` (`plan_passage` with the fixed three-pass intercept, `choose_route` over the port's list, `passage_burn` for Coast); `tacsim.port.burn` taking the `PassagePlan` as its input, the prepare recomputing burn and crew-days from the plan's seconds and the held drive table and the mass terms from the txn (pool debits, manifest, the ticket carrying the whole plan, order/attempt); the launch arm on IC5's tick + boot reconcile passing `plan.legs[].secs` to `depart`; `+ship/plan` (both hold slacks and the binding measure, room per good on this board, payload, burn, stores, ETA; never a recommended mix), `+ship/undock <port>` (at=now); automatic docking at arrival; narration to member rooms at the Kirkwood boundary and at `apply_arrival`; `+ship` ETA in IC and real time; `+travel/status` aboard arm; apply-time refusal of exits into anchored interiors | mush-sim, tacsim, mush-atlas | L (+1 S for the sky seam, IC2 and here) | IC5, IC6, IC8 | at rate 3600: undock burns 41 t / 21 crew-days, launches within a tick, the pilot in the Crew Compartment hears Kirkwood (~268 s) and the arrival (~471 s), the hull docks at Stickney and `+ship/leave` lands in P01; `plan_passage_reproduces_the_slice_figures` (payload 142,184 kg → legs 965,623 + 730,645 s, burn 41, crew-days 21; the 69 t return 1,507,276 s, burn 37, crew-days 18; Overloaded above 181,000 kg); `constant_sky_plan_is_departure_invariant` (rate 0: identical at depart_ic and +400 IC days, t0/t1 shifted); `moving_circular_sky_moves_the_plan_and_converges` (rate 1000 with per-body epochs: half a synodic period later is strictly longer, two runs bit-equal, Coast burn identical); `ticket_freezes_the_plan_across_a_sky_change` (re-materialize the catalog under rate 1000 or a changed epoch: `leg_due_at` and `arrival_at` unchanged, `plan.sky_rev` still the old sky); `journey_arrives_at_its_frozen_instant_across_a_reboot_with_a_new_sky`; `port_routes_are_a_list_and_the_planner_picks_the_fastest` (a bare string refused; a two-route test pack: the faster wins, ties to the earlier; `via=` refused "not yet"); refusals: tank 40 t, overloaded, not aboard, under way twice; a server-half restart between burn and launch yields exactly one journey; one hand-played smoke at rate 24 on local `mushd`, logged in the cut note |
| IC10 | Selling and the Co-op share — the loop closes interactively (**landed**, `7435f954`) | `Supply::Wanted`; inventory `transfer_marked` + the Fact-gated payout; `LotOp::Deliver`; the `+market/sell <listing>=<qty> [from=]` lot arm (gate, synchronous checks, buyer-org escrow by party cap, intake decrement/restore); `MarketModule::with_proceeds_policy` + the tacsim margin-over-reference policy; bids by the applier; the enrolment folds sales and shares; sale lines print gross/share/net | mush-market, mush-inventory, tacsim | L | IC9 | at Stickney, sell 70 t metals: Kasei −21,000 = pilot +19,250 + Trust +1,750 from journals; hold −70 t = Kasei pool +70 t from lineage; money moves only after the marker; a sale at reference pays share 0; over-selling, an unfunded bid and a cross-port sale refused before any contract. From here a tester plays one leg per login by hand |
| **IC11** | **Captain's orders — the FIRST CUT WITH THE FULL DAILY LOOP** (**landed**, `26e3b1fa`) | `tacsim:orders:<hull>` (fail-closed); the order arm on IC5's tick (rotation, one step per hull, actor swap, closed typed calls, the buy clamp by stock, hold mass, hold volume and funds less the next depart's unmet needs with every bound logged, `fill` as the joint least fixed point over the queued projection and `plan_passage` at the depart's `not_before`, inflight in the same Outbox, pull verification, relabel on a rate generation); `+ship/orders`, `+ship/order …` (`depart … [via=]` parsed and refused "not yet"), `+ship/plan` over the queue (each buy's clamp and bound, the affordable tonnage at this board); the UTC time parse/format helper; the watch-log reconnect notices; the pilot-aboard and passenger rules | tacsim, mush-core | **L** (risks 2, 3, 7, 8 live here and D-18-38 makes it the acceptance; if it must be split, IC11a = the document, the nine verbs, the time parser and `+ship/plan` over the queue, IC11b = the executor arm with its restart and clustering tests, M each) | IC10 | at rate 3600 with the pilot disconnected after queueing: depart → arrive → sell ×2 → buy ×4 → depart executes unattended, the log and notices matching; a reboot mid-step re-runs nothing (one sale, one burn); a sold-out buy goes partial, an unfunded bid skips, a depart short of fuel holds and burns nothing; the Stickney queue buys fuel and stores before cargo; a cash-bound return logs "funds", re-used 49/55 orders with full funds log "hold volume", `buy volatiles 100` logs "short of 100 t: hold volume", a full hold logs both measures joined; `fill` buys 36 t for the 73 t projection; `fill_and_clamp_plan_at_the_depart_instant` (a test sky at rate 1000 makes the crew-days at `not_before` differ from a stored constant); two pilots with the same due time both complete |
| IC12 | The story test, help and readiness (**landed** with Draft 0.4) | `ic_slice_loop.rs`: two simulated days at rate 3600 on the production composition (§6), its sky built explicitly at rate 0; `docs/help` ports, ships (the hold's two limits; passage time depends on load "and, later, the season", never "always under a day"), coop, orders, trade (the dense good weighs out, the bulky good cubes out, the best full load fills both; when cash is short buy the bulky good first up to the volume the affordable tonnage leaves); amendments to getting-started, travel, spatial, cockpit, npc; `ops.md`; the full readiness report ("sky: constant (rate 0), epoch <date>, rev <digest>"; never a duration) and the gate over all of it on `+coop/enrol` (§4.7); every §2 and §6 path walked against the packs' exit aliases; one hand-played two-session smoke at rate 24 on local `mushd`, logged | mush-server, docs, tacsim | M | IC11 | the story test green with "every hold write leaves both cargo limits met"; a fresh tester completes two legs from help alone; T-45 green; the smoke log shows a sub-day full leg at the constant sky and a docked hull at the next session |
| IC13 | Buyout, graduation and dormancy (**landed**, `94679739`) | `+coop/buyout` (pilot-issued asset contract, Trust accepts by party cap, `end_lease` before settlement with refuse-on-Retrying, observer graduation and re-lease on refund); dormancy recall on the tick; `+coop/release`; `@coop/revoke` | tacsim, mush-economy | M | IC10 | a pilot with 110,000 cr buys out: title to the pilot once, lease ended, the share silent on the next sale; a refunded buyout re-leases; seven idle real days recall a docked empty hull; a buyout while under way refused |
| IC14 | The residuals sweep (**landed**, `2f268ee3`) | the retention of `inventory_applied` (risk 19), the money-safety residuals (the full-queue sale, the re-stage after a landing, the seller's dispute, the boot-healed fold), the recall's convergence, the fail-closed sailing plan, the residual tests, Stickney v4, the small presentation residuals, the §10 amendments | mush-core, mush-inventory, mush-market, mush-economy, mush-sim, mush-atlas, tacsim, mush-server, docs | L (five passes) | IC13 | every residual dispositioned in the cut note with its file:line; the gate green |

**As built, by row** (the deviations each landed cut recorded; the cut notes hold the rest):

- **IC0.** As its row; `spacecraft` left `SELF_MODES` in IC1 (Q-18-6). IC7 later closed the
  direct-consume hole IC0 left on hold-backed listings (an amendment to IC0's cut, IC7 (2)).
- **IC1.** As its row. `+ic` lands a Ceres pilot at C03, a Stickney pilot in P01 (owner default). Five
  18a details could not ship under the detail-name rule; they shipped in IC12 under plain keys. `+ic`'s
  refusal branches and the door's no-staff-exemption were tested in IC12.
- **IC2.** The lane's legs moved to the real date 2351-05-13 with the cruise at 226 km/s (IC2's lead
  ruling, built in IC3; the row's 540,800 / 409,200 s and 0.741 / 0.561 AU are Draft 0.3's); lane v2 and v3
  followed as new versions. The calendar is `CivilDate`, not Unix seconds (§4.8 as built).
- **IC3.** As its row; org renames refused as drift (P5), `@org/list` shows orgs only and capital reads on
  `@tacsim/port status`, and the `tacsim.credit` mint cap is contributed in code (IC3 (1)–(3)).
- **IC4.** As its row; the Dray's exits are append-only by position (IC8's rule); no lane bump.
- **IC5.** As its row; the per-fire ledger's engine-drain seam (`Ctx::authoritative_pending`) followed in
  IC8 and IC10.
- **IC6.** As its row, money rendered `25000.00cr`; `+ship/leave` gated on placement alone (the proposed
  D-18-34 amendment); the 1-minor rent kept.
- **IC7.** The row's "a Draw refused at prepare cancelling its contract and returning the escrow" is built
  as **expiry with refund** at 3,600 s with a 600 s re-stage cutoff (IC7 (1)); the capacity hook is
  `plan` + `check`; the `to=` grammar takes both spellings.
- **IC8.** Kasei's intakes are 4,000 t metals and 9,600 t volatiles as the doc says (IC8 (1)); Q-18-19
  stricter; the Dray v2 and lane v3 republished; the accept line naming the signatory deferred (unowned).
- **IC9.** The row's lens figures are lane v1's: at lane v3 the outbound legs are 959,361 + 735,600 s and
  the 69 t return's total 1,506,115 s, Kirkwood at about 266.5 s and the docking at about 470.8 s at
  3600 for the full outbound; the tank refusal is 39 t against a 40 t burn; the ticket gained `seq` and
  lost `launched`; the launch submits nothing. **The hand-played smoke was not done** (no Redis on the
  build machine); owed to the owner.
- **IC10.** The row's "sell 70 t metals: Kasei −21,000 = pilot +19,250 + Trust +1,750" is Draft 0.2's
  load: at R7's vertex the sale is 20 t at 280: Kasei −5,600 = pilot +5,200 + Trust +400. The reference is
  read at contract build; the share floored; part 1 of the story test landed in `crates/tacsim/tests/`
  and moved to `crates/mush-server/tests/ic_slice_loop.rs` in IC12.
- **IC11.** A queued depart's lost burn is re-filed under `attempt + 1` with a fresh `seq`, never the
  same key (L7); the verify lens lived in `crates/tacsim/tests/orders.rs` and part 2 of the story test is
  IC12's.
- **IC12.** Crates touched: mush-server (the story test), tacsim, mush-testkit (`TestWorld::from_engine`),
  mush-atlas (the validator bounds, the T10 test), mush-sim (`@journey`'s help, comments), mush-npc
  (`@agent`'s help), mush-market (`+market`'s help, comments), mush-contract-tests (a state-slot row
  removed), and `docs/` (help, `ops.md`, this revision). The story test runs on `build_engine` at
  `[game].ic_ratio = 24` with every submission landed through the action coordinator, and sets 3600 as its
  one staff act after the enrolments (§6 as built; O6). **The hand-played two-session smoke was not done**:
  this machine has no Redis (port 6379 closed, checked 2026-10-17); the story test's rate-24 set-up half
  stands in for the smoke's set-up half, and the real smoke is owed to the owner.
- **Unowned presentation residuals** (deferred to IC12 by the cuts, and not built by it — not in the
  slice's acceptance; the figures are on `+ship/hold`, `+ship/plan`, `+ship/orders` and the boards; owner
  to place, IC14 or a content cut): the Engineering/Bridge burn-and-reserve gauge lines (IC11 L21); the
  C14 and P05 departure boards and the C01/P01 berth boards as `LookRendered` lines (IC9, IC11); P07's
  funded-balance board figure (IC8, IC10); the accept line naming the listing's signatory (IC8 L17).

Post-loop cuts, named and ordered: **P-B** the Bids readiness line gates `+coop/enrol` on a Missing bid
only, a funds-Pending house paged once a boot (Q-18-60, R10; S; **landed 2026-10-18 at `e682a7ec`**, the cut note in
[18-cut-notes.md](18-cut-notes.md) "P-B"); **P-G** real gangway exits (outer door relinked at dock, a station
exit named by berth and hull, `BerthLink`, the Moved observer, atlas `on_moved` ordering tests, boot
reconcile; M); **P-F** berth slots, berth-holder markers, the six freight-deck rooms, `ExitDef`
fail/succ, the `+move` restricted fix (M); **P-H** locality and the under-way seal on every inventory
path at offer projection, for the day a player holds a second endpoint (S–M); **P2** stations in the unit
spec, `+ship/helm`, `request_seat_activation` and the `movement` channel for a second crew member (M);
**P3** provider identity fix, `ledger`/`ship`/`port` providers (M); **P4** Kirkwood as a checkpoint, a
hold of a sim hull, the Customs Assertion front (M); **P5** production restock with conservation (Kasei
refinery work orders, Ceres deposits, a construction sink) before a second cohort (L); **P6** the
`LotCensus` quantity index only (S; its volume half is in the slice under R7 and is withdrawn here);
**P7** Pallas as a port (data); **P8** delete the legacy seed latch (M); **P9** the net share with a run
book and carried deficit (M); **P-A** the astronomy engine (§4.8; L in six sub-cuts: data and model M,
laws M, chart S+M, price policy M, comms S, flip S; after IC12, with P5 for its prices and P7 for its
showcase) — **designed as [doc 19](19-astronomy-engine.md) (2026-10-18) and built as its cuts A.1–A.6;
A.1 (the element tables, `Sky::Kepler`, lane v4) landed 2026-10-18**, the cut note in
[18-cut-notes.md](18-cut-notes.md) "A.1"; Q-18-35..41 are ruled by doc 19 §2's defaults, the owner
overriding by data.

**IC0.** As recorded in [18-cut-notes.md](18-cut-notes.md). `spacecraft` out of `SELF_MODES`
(`travel.rs:90`) and `options_from_corridor_room` is IC1's, with its test (Q-18-6, binding).

**IC1.** Author the four packs to 18a §1–§2 and §4.5. Add `markers: Vec<String>` to `RoomDef` with the
registry the validator consults; seed `port.dock` rooms in `validate.rs:1958-2000`. Write
`exit.limbo_arrivals` unlocked. Add the `tacsim.last_port` stamp (on `+ship/leave` in IC6 and on the
Limbo-bound OOC exit now), the door hook and `+ic`. Copy the starter digest-pin and validate-clean tests;
add the kept-world boot test.

As built (IC1): as written, but the stamp is on leaving a port room for a room in no port (`o`, `home`, the OOC exit), not on `+ship/leave`; `+ic` lands at C03.

**IC2.** Author `dray.json` (limit maps, the tagged drive law); add `Hauler`, `logistics` and `interior`
with `skip_serializing_if`; filter `hauler` in `fieldable_ids`. Author `goods.json` (six rows, measures)
with its validator and pin; derive the catalog rows; `port/capacity.rs` and its pure tests. Author
`tacsim.lane.ceres_phobos` with §4.4's legs, real-date true longitudes and distances with `epoch_ic`
(a small committed script beside the fixture; the date in the cut note) and Kirkwood at its epoch
position pending Q-18-36; `sky.rs`; `Body.epoch_ic` in `phase_mdeg_at`; the five sky tests; the
`@corridor/show` and comment wording.

As built (IC2, IC3): the legs at 2351-05-13 and 226 km/s (lane v2 in IC3); the calendar is `CivilDate`.

**IC3.** `PackEntry::Port(PortDef)` and the misplacement row. `PortApplier` a–c with the fail-closed map
and capital tranches. The passive-open guard skip in mush-sim. The `test_world` composition flag.
`+credit` with the major-unit formatter. `@tacsim/port status`.

As built (IC3): as written; the `test_world` flag is never set by a mush-server boot (IC12 closed that deferral as not needed).

**IC4.** The five mush-sim edits of §4.2 and their tests; `PortApplier` f from the interior template.

As built (IC4): as written.

**IC5.** Register the provider, the two typed families, the delegate and the handle cell; the tick
skeleton with its budget and the notice store; the composition-edge pin.

As built (IC5): as written; the engine-drain seam followed in IC8.

**IC6.** `+coop/enrol` and its siblings; the lease gate; the stale-page exemption; the observer; the
`+ship` verbs; the status line.

As built (IC6): as written; `+ship/leave` placement-gated only.

**IC7.** The six market/economy seams of §4.3 and §4.4, on IC0's `trade.rs`, `lot.rs`, `api.rs` as
landed.

As built (IC7): as written, the Draw refusal by expiry with refund.

**IC8.** `tacsim_port_stock`; the stock arm; `PortApplier` d–e; the tacsim gate, caps and capacity
provider; the lapsed-hold release arm; the rendering.

As built (IC8): as written, with Stickney v2, Dray v2 and lane v3.

**IC9.** The time half (`laden_leg_durations`, `plan_passage`, `choose_route`, `passage_burn`, `leg_s`,
`depart`); the burn with the plan as input and the ticket carrying it; the launch and dock arms; the plan
and undock verbs; the sky-change and reboot tests; the narration. Play it once by hand at rate 24.

As built (IC9): as written, except the hand-played run (no Redis).

**IC10.** The sell arm, `transfer_marked`, the Fact gate, `ProceedsPolicy`; `ic_slice_loop.rs` part 1.

As built (IC10): as written; part 1 moved to mush-server in IC12.

**IC11.** The orders document, the executor arm, the verbs, the time parser, the notices;
`ic_slice_loop.rs` part 2 with the pilot disconnected.

As built (IC11): the document, arm, verbs, parser and notices as written; part 2 is IC12's.

**IC12.** Help, the readiness gate, the smoke.

As built (IC12): the story test (both parts, the rate change, the negative checks, the plan's skips and holds) on `build_engine`; the help topics; the gate over every line but the sky; the station packs Ceres v2 / Stickney v3; the smoke owed (no Redis).

**IC13.** Buyout, release, revoke, dormancy.

---

## 6. Acceptance

**The story test**, `crates/mush-server/tests/ic_slice_loop.rs`, runs on the production composition
(`build_engine`) at `@tacsim/time rate=3600` (the only staff act) with no staff act after boot, in two
parts. In this listing `;` separates lines typed one at a time (there is no command separator; a
leading `;` is the semipose prefix, `mush-core/src/cmd.rs:1300-1304`). Timings are for the full outbound
load at 3600 at the test's own constant sky, built explicitly at sky rate 0 (a 19.63 h passage is about
471 s; Kirkwood at about 268 s).

```
--- part 1 (from IC10): interactive, the pilot online ---
create Vex pw                       -> Limbo; exits O, S, C
c                                   -> Port Watch Arrivals, Ceres Freeport
registry                            -> (C05 is north of Arrivals; the concourse has no exit to it)
+coop                               -> stake 25,000.00cr; share 250‰ of margin; buyout 90,000.00cr; 8 of 8 Drays free
+coop/enrol Salt Moth               -> "... account #41 ... charters you FPC-D-0401, laid up as the Slack Water, now the Salt Moth ... 25,000.00cr staked ..."
+ship                               -> docked Ceres Freeport, Berth Gallery; cargo 0/100 t · 0.0/120 m³; tank 0/80 t; locker 0/120
arrivals ; concourse ; exchange
+market/list                        -> metal concentrate 200.00cr/t 4,000 t · water ice 100.00cr/t 9,600 t · WANTED alloy 520.00cr/t · WANTED yard fittings 560.00cr/t
+market/quote 4=20 ; +market/accept 11     -> "20 t of metal concentrate are loaded into the Salt Moth's hold. Margit Søndergaard-Achebe countersigns."
+market/quote 5=80 ; +market/accept 12     -> 80 t water ice, 8,000.00cr; "Rosalind Qureshi-Mbeki countersigns."
chandlery
+market/quote 6=42 ; +market/accept 13     -> 42 t reaction mass, 1,680.00cr
+market/quote 7=21 ; +market/accept 14     -> 21 crew-days, 210.00cr
+ship/hold                          -> cargo 100/100 t · 120.0/120 m³ · tank 42/80 t · locker 21/120 ;  +credit -> available 11,110.00cr
exchange ; concourse ; arrivals ; lobby ; gallery ; +ship/board     -> "You cycle the lock into the Salt Moth."  (look: Airlock; status line)
in ; up ; look                      -> Bridge: course, tank and hold readouts (t and m³)
+ship/plan stickney                 -> cargo 100 t of 100 t (mass full) · 120.0 m³ of 120 m³ (volume full); payload 142.2 t; burn 41 t (1 t left); 21 crew-days; 19.6 IC days ≈ 19 h 38 m real
+ship/undock stickney               -> "Clearance granted. 41 t reaction mass and 21 crew-days committed ... arrival in 19.6 IC days."
down ; crew
   (~268 s)                         -> "Kirkwood Depot slides past ... Leg 2 of 2."     (heard in the Crew Compartment)
   (~471 s)                         -> "Salt Moth docks at Stickney Yards."            (heard in the Crew Compartment)
passage ; airlock ; +ship/leave     -> Visiting Berth Gallery, Stickney Yards
lobby ; customs ; concourse ; exchange ; down
+market/list                        -> Kasei Metals WANTS metal concentrate 280.00cr/t (4,000 t) · water ice 200.00cr/t (9,600 t)
+market/sell 21=20                  -> "Sold 20 t metal concentrate to Kasei Metals at 280.00cr/t: 5,600.00cr; Co-op share 400.00cr; you received 5,200.00cr. Jonas Achterberg-Nkemelu countersigns."
+market/sell 22=80                  -> "Sold 80 t water ice at 200.00cr/t: 16,000.00cr; Co-op share 2,000.00cr; you received 14,000.00cr."
+credit                             -> available 30,310.00cr

--- part 2 (from IC11): the pilot queues and disconnects before departure ---
(at Ceres, after loading, aboard)
+ship/order depart stickney at=now ; +ship/order sell metals all min=250 ; +ship/order sell volatiles all min=150
+ship/order buy propellant fill max=60 ; +ship/order buy provisions fill ; +ship/order buy industrial_parts 49 max=400 ; +ship/order buy structural_alloys 55 max=400
+ship/order depart ceres at=+600s ; +ship/orders ; QUIT
   (tick)                           -> plan; burn 41 t / 21 crew-days; launch; (~471 s) dock at Stickney
   (fires)                          -> sell 20 t metals (+5,200 net); sell 80 t ice (+14,000); buy 36 t fuel (tank 37); buy 19 crew-days; buy 49 t fittings; buy 20 t alloys (short of 55: funds); cash 360.00
   (+600 s)                         -> depart Ceres at 69 t; (~419 s = 1,507,276 IC s at 3600, the 17.45-day return) dock at Ceres
connect                             -> "While you were off watch:" every line above, once; "Aboard the Salt Moth, docked at Ceres Freeport"
+coop/statement                     -> 2 sales; margin 9,600.00; shares 2,400.00; buyout 90,000.00cr
```

**As built (IC12).** The story test is `crates/mush-server/tests/ic_slice_loop.rs`, five tests, each
booting its own engine through **`mush_server::boot::build_engine`** — the split server's root: the roster
of `boot::game_modules`, the help topics from `docs/help`, the world timeline seeded from `[game]`, the
account oracle the world task links, the durable key rings — over an in-memory store, with
`[game].ic_ratio = 24` and the sky's rate 0 set explicitly. Every queued authoritative submission lands as
the world task lands it (`CommandSubmitDriver::drive_one`): the step's legacy work persisted first, then
`ActionCoordinator::command_ingress` → `preflight` → the actor's projection and resolve →
`prepare_and_stage_resolved` → `take_staged_action` → reserve → reconcile → the announcing install →
checkpoint, so the typed state lives in the store.

- **`ic12_part_one_a_leg_by_hand`** (part 1, Vex): §2 day 1 and day 2's walk by every exit word; the sky
  line asserted in this section's form with no duration on it; the enrolment at rate 24; then **the one
  staff act, `@tacsim/time rate=3600`, after the enrolment** — the production Rate line refuses enrolment
  at any rate but 24 and the production composition is never a test world (IC3 L15; IC10's roster note),
  so "no staff act after boot" cannot hold here (owner item O6; §8). The Bridge's status line; the plan's
  burn and crew-days read from the planner (the burn is the bunkered fixed point; 41 t is the burn of
  the doc's 42 t); IC9's T7 (a fire while the burn is still queued in the engine defers the launch —
  `launch_step`'s `Step::Deferred`, read from the fire's `PortFireStats.deferred`, never the lapse grace:
  no journey, no `CLEARANCE_LAPSED`, `STATUS_CLEARED`) and the **same-boot HIT** (the drained burn landed
  twice in one process epoch is the coordinator's replay: one ticket, one debit); the Kirkwood line not
  heard by the fire a few real seconds before the first leg's frozen boundary and heard by the fire at
  it, and the docking likewise at the ticket's total (about 266.5 s and 470.8 s at 3600 for the doc's
  full outbound; the test's own load is a little lighter); the two sale lines exactly as `SALE_LINE`
  renders them; money conserved; every tonne traced to a seeded receipt; the five help topics answered by
  the booted registry.
- **`ic12_part_two_the_queue_runs_the_return`** (part 2, Wren, alone in its world so `+coop/statement`
  reads two sales): §2 step 10's queue with the return at **`at=+10m`** (the shipped grammar has no
  seconds form, so this section's `+600s` is `+10m`; lead ruling L12), the pilot disconnected; three
  restarts, each a real `build_engine` re-boot over the kept store with the typed partition hydrated
  before any module boots and a new process epoch: **(a)** between the depart's burn landing and its
  launch — the kept burn landed again in the new epoch **before any fire**, the hull still docked and the
  passage still filed, is refused by the provider's ticket guard ("that passage is already burned": the
  coordinator preflights it vacant, so the ticket at its `seq` is the guard) with one ticket and the tank
  unchanged; then one launch, one journey, no new burn under any key; **(b)**
  with the first sale in flight and its deliver leg lost — the leg re-staged once under
  `market.lot.deliver.<contract>`, one settled sale, one share, Kasei +20 t once, no second sale into
  the bid ("verified, not re-run"); **(c)** the kept world after the return — nothing submitted on the
  fires after it (every queued submission inspected and landed), no stock receipt minted, no listing
  moved, no stake re-posted, no capital re-issued, no object made. The queue runs unattended through
  Stickney (two sales; the fill's tank and locker lines with the targets recomputed from `project_cargo`
  and `fill_targets` over the hold as it docked, with the passage planner for the return route; the
  fittings at 49 t; the alloys "short of 55 t: funds", the affordable tonnage computed from the stake,
  the Ceres costs, the sales' nets and the earlier buys, and checked below the listing and the hold's
  room by both limits; each line asserted exactly) and the
  return docks at its frozen instant (about 418.4 s at 3600 for the doc's full return); on reconnect "While
  you were off watch:" once, every notice once in queue order; `+ship/orders` signed ", signed Wren"; the
  statement's margin and share computed from the posted prices; conservation over Wren, the Trust and
  every port org, and over every touched listing, pool and both legs' burns.
- **`ic12_a_rate_change_mid_passage_relabels_the_arrival`**: §6's rate change and IC9's T9 — at 1800
  mid-passage the arrival's IC instant is unchanged, the real-time ETA on the status line and `+ship`
  doubles from that point, a queued `daily HH:MM` depart keeps its UTC hour and its IC label is
  re-derived (never stored), and the hull docks at the frozen instant.
- **`ic12_negative_checks_refuse_in_fiction_and_move_nothing`**: each check asserts the shipped line and
  that nothing moved — `ENROLLED_ONCE` for a second character on the account; `NOT_OPEN` for an enrolment
  after the rate switch, with the staff view's flagged Rate line and `enrol_gate` naming `Binding::Rate`;
  `NO_CHARTER` for another player's `+ship/board`; mush-sim's "compartment `berths` is full (2 max)" for a
  third soul placed by staff; `NOT_IN_PORT` for an accept from Limbo and `HOLD_ELSEWHERE` for one at Ceres
  with the hull at Stickney (staff carried the pilot there: no player reaches Ceres without the hull);
  `capacity_line`'s refusals for 86 t of ice, 19 + 81, 101 t of alloys and one more tonne of ice with the
  cargo full and the tank empty ("ice into the tank": ice stows only as bulk); `NOT_ABOARD`, **`TANK_SHORT`
  with 39 t against a 40 t burn** (this section's 40-against-41 cannot be built under the 105‰ law) and
  `UNDOCK_UNDER_WAY` twice; `HELD_ASHORE` for a queued depart with the pilot ashore (nothing burned);
  `HOUSE_CANNOT_PAY_SKIP` (never the market's `UNFUNDED`) for a queued sale into a house whose account staff
  emptied, the cargo kept, and `UNFUNDED` for the same sale typed; mush-sim's boundary line for `o`, `s` and
  `home` aboard and "Permission denied." for `@teleport`; the IC door's `left_at_line` after `o` → `x` →
  `c` from Stickney; `SELL_SHORT`; and the pilot's raw `@journey/depart`, refused for the pilot by the
  permission gate ("Permission denied.") while staff read the lane's own `LANE_UNPLANNED`; IC9's T6 (a
  non-staff `@tacsim/port clear` refused "Permission denied."; a staff clear of a held launch stands it down
  with nothing refunded; the pilot's `+ship/leave` then lands in the gallery). **`@inv/move` out of the hold
  under way** is refused for the pilot at the coordinator — a pilot acts on exactly one endpoint, so the
  move has nowhere to land — and nothing leaves the hold; the test pins the same refusal for the same move
  while docked, so it proves the endpoint rule, **not** an under-way rule: none exists, a staff move out of
  a hold under way lands (D-18-37's deferral to P-H; Q-18-62), and this section's check as written stays
  open there. The unfunded-bid check drains the house to 100.00cr (under one tonne's price, not zero).
- **`ic12_the_sailing_plan_skips_and_holds`**: IC11's untested paths on the production root — `NO_BID`,
  `HELD_SHORT_FOR_DEPART`, `HELD_NO_DEPART`, `FILL_ENOUGH`, three dropped burns → `HELD_UNCONFIRMED`,
  `UNDER_MIN`, `NOTHING_ABOARD`, and `AWAITING_REFUND` after `DELIVER_TOO_LATE` and after `DRAW_TOO_LATE`
  (each late hand-over refunded within the hour: the Draw's contract `Expired`, the escrow — a hold, so
  read as available funds, never the posted balance — taken at the accept and released exactly).
  `REFUSED_RETRY` (no step on the shipped packs reaches
  `refused_call`: the planner pre-checks the house's funds, the stock and the hold), `INTAKE_TAKEN` (no 100 t
  hull exhausts a 4,000 t intake in a test), `HELD_FILL` and `HELD_FILL_PLAN` are pinned at the unit level
  (`tacsim/src/port/executor.rs`'s `ic12_*` tests) and the resume reset in
  `crates/tacsim/tests/orders.rs::ic12_resume_clears_the_refusal_count`.

IC9's T8 is `crates/tacsim/tests/voyage.rs::ic12_t8_goods_pending_aboard_releases_a_terminal_contract_and_holds_for_a_lapsed_listing`
and T10 `crates/mush-atlas/tests/anchored_exits.rs::ic12_t10_a_one_way_exit_out_of_a_room_aboard_applies`.
**The hand-played two-session smoke was not done:** the build machine has no Redis (port 6379 closed;
checked 2026-10-17), so the story test's rate-24 set-up half (enrolment, loading and queueing at the play
rate) stands in for the smoke's set-up half and is not called a smoke; the real smoke — two sessions at
rate 24 on local `mushd`, or a dev1 session after an owner-approved redeploy — is owed to the owner.

**Assertions.** Conservation from journals and lineage: Σ money over pilot, Trust and the orgs is
unchanged except the boot capitalizations; pilot + Trust = Kasei's payment on every sale; every tonne in
the hold traces to a seeded Origin receipt; seeded − sold − burned = remaining on every listing and pool;
Kasei's pool = delivered intake; every hold write leaves both cargo limits met. No `SettlementRefused`
and no stale-contract page. The test asserts "sky: constant" on `@tacsim/port status` and no passage
bound outside its own sky.

**Negative checks**, each refused with an in-fiction line and nothing moved: a second `+coop/enrol` on the
same gateway account; another player's `+ship/board` on the Moth; a third person placed into the berths
slot; `+market/accept` from Limbo, or into a hold docked at the other port; 101 t of alloy stock
(mass); 86 t of ice, and 81 t of ice after 19 t of concentrate (volume); ice into the
tank; `+ship/undock` with 40 t against a 41 t burn, not aboard, or under way twice; a queued depart with
the pilot ashore (holds, burns nothing); an unfunded bid (sale skipped, cargo kept); `@inv/move` out of
the hold under way; `o`, `s`, `home`, `@teleport` by an occupant; `o`→`x`→`c` from Stickney (the `+ic`
hint); selling more than held; the pilot's raw `@journey/depart`.

**Restarts.** A server-half restart between the burn and the launch yields exactly one journey; a kill
mid-order at Stickney yields one sale and one share on restart (the inflight step is verified, not
re-run); a kept-world reboot mints no stock, posts no stake and capitalizes nothing twice; a rate change
mid-passage relabels the arrival and the queue's IC stamps.

**The play-rate experience.** The 3600 test does not stand in for it. IC9 carries one hand-played
flight and IC12 one hand-played **two-session smoke at rate 24 on local `mushd`** — day 1 enrol, load,
queue, log off; day 2 log in to a docked hull, read the watch log, walk the Yard, re-file, log off —
logged in the cut note. The rate is a global knob that moves contracts and cooldowns (`clock.rs:74,83`;
dev1 seeds `IC_RATIO=4`), so dev1 is never flipped for a session. A dev1 redeploy with `@tacsim/time
rate=24` happens only on the owner's ask.

**The harness/CI lens.** `ic_slice_loop.rs` on the production composition; IC0's gate test; the four
station packs and the lane pack validate clean with a kept-world boot test; `tests/content.rs` pins every
digest including `goods.json`; the four IC2 sky tests, the six IC9 planner and freeze tests and IC8's LP
test; the capacity function's pure tests; the custody-seam test refuses a hauler; the
composition-edge pin covers tacsim's delegation; T-45 holds the help. The load-sensitive suites rule of
the 16b loop applies to the rate-3600 tests.

---

## 7. Decisions taken here (under the defaults of §8)

Draft 0.1's twenty decisions, re-stated. "Stands" keeps the number and the text; "replaced by R*n*"
records the owner's ruling of 2026-10-15 and points at the decision that takes its place. **Under R8
every decision below that rested on a Draft 0.2 question is now binding (owner, 2026-10-15)**; the
"(Rests on …)" notes are kept so the reader can see which ruling closed what. D-18-28 is replaced by R7;
D-18-39 to D-18-51 are new; D-18-13, 25, 27, 29, 30 and the §4.2 ticket (D-18-11) are amended as marked.

- **D-18-1 Ports.** Stands, confirmed by the owner (Q-18-1 closed): Ceres Freeport → Kirkwood Depot →
  Stickney Yards; Pallas a body, a lore thread and the named next port. Tycho is an insurer's agency at
  both ports, no longer the ship desk.
- **D-18-2 Blockers first.** Stands; IC0 landed unchanged as `c87d78f7`.
- **D-18-3 The IC door.** Amended: an **unlocked** Limbo exit (the `OnboardingGate` already bars
  unapproved characters, so no approval marker and no P9) plus an **IC-return rule** (`tacsim.last_port`,
  a door hook, `+ic`) because Limbo exits are global.
  **As built (IC1):** `+ic` lands a Ceres pilot at C03 Port Watch Arrivals (where the Limbo door goes),
  a Stickney pilot in P01; the stamp is the `Moved` observer on leaving a port room for a room in no
  port; no staff exemption at the door (§4.5 as built).
- **D-18-4 No staff act and no NPC act on the player path.** Stands, extended: the queue executes only
  the pilot's own signed orders, as the pilot, through closed typed calls.
- **D-18-5 The credit line.** **Replaced by R2** → D-18-21. There is no loan, advance, line or overdraft.
- **D-18-6 The pilot account.** Amended: opened inside `+coop/enrol` at the Registry & Co-op desk;
  Clearing holds it and lends nothing.
- **D-18-7 Repayment.** **Replaced by R2**: there is no debt; P1 leaves the slice.
- **D-18-8 The hull.** **Replaced by R5** → D-18-22. The bay principle (a walkable room is never a vault
  backing) survives as the store space.
- **D-18-9 No stations or billets; the charter record as the operate gate.** **Replaced** → D-18-23: no
  stations, but the live lease is the gate, and the C2A–C4A/F3 bypass is re-justified under R3.
- **D-18-10 Acquisition from Tycho's pool.** **Replaced by R1/R2** → D-18-21.
- **D-18-11 Consumables and the burn.** Stands, extended: pools only; the ticket carries the manifest,
  the whole passage plan (D-18-46; `leg_s` is its seconds), burn, crew-days and the order key; a queued
  depart stages the same action as the pilot; the prepare recomputes burn and crew-days from the plan's
  seconds and the held drive table and trusts only the seconds (T1).
  **As built (IC9):** the ticket gained `seq` (the replay guard) and lost `launched` (read from the
  journey; `tacsim/src/port/families.rs:202`); the plan reaches the burn through the per-hull
  `tacsim:voyage:<hull>` document as a server-authenticated fixed parameter; the launch is the tick
  calling `mush_sim::depart` with the frozen leg seconds — no submission, so "never a GOD-tick
  submission" holds trivially (D-18-46).
- **D-18-12 Presence and defaults.** Stands, extended with `Presence::Ship(hull)` for queued orders.
  **As built (IC7, IC8):** `Presence::{Here, Ship(hull)}` as written; `VenueGate` has five methods,
  `port_seller` added for the sell arm (`mush-market/src/module.rs:138,167`).
- **D-18-13 The resource subset.** Amended by R7: **six** goods (metals, volatiles, structural alloys,
  yard fittings as `industrial_parts`, propellant, provisions), five of the ten families; provisions
  opaque and instrument-less; `FAMILIES` ten; units tonnes and crew-days. (The sixth rests on Q-18-32.)
- **D-18-14 Vendor funding.** Stands in shape (finite one-shot capitalization, applier-map dedup);
  numbers resized → D-18-27; tranches ≤ 800,000 under a 1,000,000 cap.
- **D-18-15 Goods origin by issue/retire.** **Replaced by R4** → D-18-26; the E7 pin amendment withdrawn.
- **D-18-16 The travel record.** Stands (`CarrierJourney`; `spacecraft` leaves `SELF_MODES` in IC1 with
  a test, Q-18-6 binding).
- **D-18-17 Flight time by authored `base_s` only.** **Replaced** → D-18-30 (binding T1 applied), now
  through the planner (D-18-44).
- **D-18-18 Docking.** Stands: one passive domain per port; boarding and leaving are placement verbs;
  no gangway exits in the slice (P-G); docking automatic at arrival; no berth slots (P-F).
  **As built (IC9):** docking is automatic; every member room hears tacsim's flavoured line beside
  mush-sim's generic one (`REPLACE_GENERIC_ABOARD_LINES = false`,
  `tacsim/src/composition/sim_routes.rs:47`; owner flip, IC9 (1)).
- **D-18-19 `RoomDef.markers` / `port.dock`.** Stands.
- **D-18-20 The `port` kind.** Stands; its pack is `tacsim.lane.ceres_phobos` v1 → D-18-36; its
  `routes` value is a list per destination (D-18-47).

New decisions:

- **D-18-21 The backer.** The Belt Carriers' Cooperative's First Captain programme, issued by the Co-op
  First Captain Trust (`coop_trust`, a separate org for the 16-issuer cap): the charter is
  `Encumbrance::Lease` now (`offer_lease` + in-verb accept, a 1-minor Manual-clock rent); the stake is an
  org-actor `post` keyed by the lease; the share is paid inside each sale; the buyout is a pilot-issued
  asset contract with `end_lease` first. Clearing opens accounts; Tycho insures; Lantern is the graduate
  path. (Rests on Q-18-13, Q-18-14, Q-18-30.)
- **D-18-22 The ship interior.** One `InteriorSpaceRecord` per hull whose `rooms` lists six walkable
  members with the airlock as the space object, plus one unwalkable store space backing the single hold
  vault; the doc 06 §7.5 boundary rule (gate exit-only to a member; projection returns the member
  location); no destination seal, seat re-check, BerthLink or Moved observer in the slice. (Rests on Q-18-25.)
- **D-18-23 The operate gate is the live lease.** Who-may, checked at enrolment, at `+ship/board`, at
  queue time and at every executed step; departure requires the pilot Interior aboard, connection
  irrelevant; no station, seat or billet (P2). The C2A–C4A/F3 station-channel gate stays bypassed for the
  hull's departure, now because R3 needs departures while the pilot is disconnected and a seat lease
  lapses 300 s after a disconnect. (Rests on Q-18-17.)
  **As built (IC6 (1)):** the lease gates enrolment, `+ship/board` and every queued step (re-checked per
  step, IC11 L25); `+ship/leave` is gated on placement alone, so a pilot whose charter ends aboard is
  never stranded (the D-18-34 amendment below).
- **D-18-24 The share base.** 250‰ of trading margin over the origin port's posted ask, computed by a
  `ProceedsPolicy` from the source endpoint and paid as an `Exact` payout inside the sale; a sale at or
  under reference pays nothing. The reference is a **live read**: the programme's `reference` map names,
  per good, the origin listing (`<port>/<org>`) whose current ask the policy reads when it builds the
  payout, so only the share permille and the buyout price freeze at signing and the astronomy engine's
  price policy moves the reference with the board (the residual — billed on the ask at settlement, not
  the ask paid — is `trade.md`'s). It shares trading margin, not net profit (R2's word); the net share
  with a run book is P9. (Rested on Q-18-13; binding.)
  **As built (IC10):** the reference is read **at contract build** from the origin listing's current
  price, not at settlement (the `Exact` payouts must sum to the escrow at offer; drift at most one 3,600
  s window), falling back to the goods table's `reference_major` with the memo "baseline ref"; the share
  is floored in minor units (`tacsim/src/port/proceeds.rs:68-98`).
- **D-18-25 Programme terms.** Stake 25,000 cr; buyout 90,000 cr frozen at signing; one enrolment per
  gateway account ever; a pool of 8 Trust Drays; programme capital 250,000 with a 50,000 floor; dormancy
  seven real days; release and revoke in IC13. Numbers restated under R7: the buyout plus a 20,000 float
  lands on day 15 at full loads (about 4,000 to spare) and day 23 at 70 %, with the Trust's shares about
  40,000; Draft 0.2's "14–15 / 21–24" did not reproduce under its own numbers (day 17). (Rested on
  Q-18-14; binding.)
  **As built (IC6):** as written; money renders `25000.00cr` / `90000.00cr` (IC0's render, no grouping);
  the buyout itself is IC13.
- **D-18-26 Goods origin.** Finite seeded stock: minted once per issue by `tacsim_port_stock` with a
  `tacsim.port_stock` receipt as the guard, pooled by `inv.dissolve`, driven by a per-port clerk actor on
  the tacsim tick; asks are Stock+Lot with `LotOp::Draw` (pool to pool, the listing quantity as the
  availability counter, no reservation); bids are `Supply::Wanted` with `LotOp::Deliver` into the buyer's
  pool and a payout gated on the `inventory.transferred` Fact. The applier never submits, and refuses a
  bumped `issue` on a receipted row (paged as drift). A lapsed **player** listing's hold is released by
  the `tacsim.port` tick acting as the listing holder (IC5's actor swap), one per fire, keyed by the
  derived reservation id, never by the GOD lapse sweep. (Rests on Q-18-19, Q-18-31.)
  **As built (IC8):** Q-18-19 taken **stricter**: a port update refuses any change of issue, quantity or
  grade on a recorded stock row, and any removal, as drift, receipted or not (IC8 (2)).
- **D-18-27 Stock and capital sizing.** Resized under R7 as a sizing basis of about 120 legs each way
  for one cohort of 8–10 pilots over about three weeks: Ceres volatiles 9,600 t, metals 4,000 t; Stickney
  alloys 7,000 t, fittings 5,400 t; propellant 5,000 t and provisions 2,600 per port; Kasei intake
  9,600/4,000 t; Freeport Works intake 7,000 t; the Co-op's fittings intake 5,400 t; capital Kasei
  600,000, Freeport 4,000,000, the Co-op 3,100,000, the Trust 250,000; no restock in the slice (P5); no
  top-ups. A basis derived from today's vertices, never an invariant the readiness report asserts.
  (Rested on Q-18-19; binding.)
  **As built (IC8, IC10):** the intakes are 4,000 t metals and 9,600 t volatiles as written (IC8 (1));
  an exhausted bid stands at quantity 0 and a lapsed one is re-posted at its **full** authored intake
  under "no restock" (IC10 (4); the remainder is P5).
- **D-18-28 Units and capacity.** **Replaced by R7.** Ledger units stay tonnes for bulk goods and
  reaction mass and crew-days for provisions, in a tacsim-owned `goods.json` with its own digest pin
  deriving the market catalog; every good carries integer measures `si.mass_kg` and `si.volume_l` per
  unit, **both binding**, under doc 06 D3's open measure map (kg/L scale by registered choice); the cargo
  hold limits 100,000 kg **and** 120,000 L, the tank 80,000 kg (tank-rated, mass only), the locker 480 kg
  and 1,200 L (120 crew-days); one tacsim pure function (`capacity::load`/`admits`/`max_more`) runs in
  the lot pre-check (refuse, nothing charged), the Draw prepare (backstop, fail closed), `+ship/plan`,
  `+ship/hold`, the room gauges and the executor's clamp and `fill`; volume leaves P6.
- **D-18-29 The Dray.** 240 t dry, 100 t and 120 m³ cargo, 80 t tank, 120 crew-days, two berths at
  100 kg, a 181,000 kg payload ceiling that is a non-binding rail (≥ 180,680); `UnitClass::Hauler`,
  `logistics` (D3 limit maps) and `interior` as optional fields; no core. Amended by R9: the drive is a
  **tagged law**, `law: { coast: { kms: 205, burn_permille: 105 } }`, no `thrust_permille`, one class one
  law (D-18-45).
  **As built (IC2, IC3, IC8):** the cruise is 226 km/s at the real date 2351-05-13 (IC2's lead ruling),
  the pack at v2 (the `lock` alias dropped, the prose figure-free); at lane v3 the full outbound flies
  959,361 + 735,600 IC s and the full return 653,642 + 852,473, the laden step FLOOR (D-18-30/45).
- **D-18-30 Passage.** The laden evaluator applied to live flight now (T1) **through the planner**
  (D-18-44): `plan_passage` → `CarrierJourney.leg_s` → `pub depart(leg_s)`; the lane's unladen `base_s`
  is 540,800 + 409,200 s, the pack's pin of the constant sky checked against the sky and the drive's rate
  by IC2's test (D-18-45), so a brimmed hull lands in 22 h, a full load in 19.63 h and the 69 t first
  return in 17.45 h at rate 24 — at the slice's constant sky, never as an invariant (D-18-50); crew-days
  are occupants × Σ ceil(laden leg days); the rate is 24 for play and 3600 for tests; the sky rate is 0;
  `RouteDef.propellant` is 0 and not reinterpreted; `route_leg_durations` is not built. (Rested on
  Q-18-15, Q-18-16; binding.)
  **As built (IC9):** the planner as written; the laden step is `route_duration`'s exact arithmetic with
  a FLOOR (IC9 L1; `mush-sim/src/route.rs:863`); the lane's legs are lane v3's.
- **D-18-31 The queue.** A durable legacy `tacsim:orders:<hull>` document with a closed vocabulary (buy,
  sell, depart, stop; `fill`; UTC departure times), executed by the tacsim pull tick acting as the pilot
  through closed typed calls; `@wait` and `schedule_source` refused as carriers; the verb family is
  `+ship/order`. In fiction it is the **sailing plan** the pilot signs at the port counter, executed by
  the port's clerks and berth under the pilot's signature; the Class 0 ROM only flies the cleared route
  (doc 17 §9; the Warrant). (Rests on Q-18-22, Q-18-24, Q-18-28.)
  **As built (IC11):** the document as written, nested `What` wire (L3), `Order.refusals: u8` (L26), the
  four hold shapes (O4, L10); a buy is done when drawn or settled (L5); the sell order names a good
  (L2); `stop` is a barrier (O5); `seed_next_id` (L29).
- **D-18-32 Executor rules.** One step per hull and one submission per pilot per fire; the lease
  re-checked per step; inflight written in the step's batch; completion verified by pull; after a reboot
  verified, never re-run; the buy clamp reserves the next depart's burn and crew-days; deterministic keys
  (burn by hull/order/attempt; lot leg by contract); holds consume nothing and notify.
  **As built (IC11):** market legs re-stage at +60 s and a second miss is judged 160 s after the first
  re-stage (L6); **a queued depart whose burn produced no ticket is re-filed under `attempt + 1` with a
  fresh `seq`, never re-staged under the same key** (L7); `NO_ROUTE` holds stopped (L10, a recorded
  deviation from "re-checked silently").
- **D-18-33 The tick skeleton** (tacsim's first action provider, the two typed families, the quantity
  delegation and inventory handle, the `tacsim.port` normal-lane tick with its budget and the notice
  store) lands as its own cut, IC5, shared by the stock, launch, dock and order arms.
- **D-18-34 Boarding by verb.** `+ship/board` in the gallery and `+ship/leave` in the airlock (and
  nowhere else, in both documents), gated on the lease and the hull's placement; real gangway exits are
  P-G. (Rests on Q-18-18.)
  **As built (IC6), a proposed amendment for the owner (IC6 owner-critical default 1):** *"D-18-34
  Boarding by verb. `+ship/board` in the gallery and `+ship/leave` in the airlock (and nowhere else, in
  both documents); board is gated on the live lease and the hull's placement, leave on the hull's
  placement alone, so a pilot whose charter ends aboard can always step ashore; real gangway exits are
  P-G."* (Q-18-45).
- **D-18-35 The stations.** Ceres 13 rooms on three decks, Stickney 12 on two, as 18a; the six freight
  rooms pruned to P-F; one staff-locked spine per station holding the storefront items; principals placed
  `enabled:false`; wall features on every exitless partition; hydration off. (Rests on Q-18-23.)
- **D-18-36 The lane pack id** is `tacsim.lane.ceres_phobos` v1; the `tacsim.corridor` fixtures are
  untouched.
  **As built (IC2, IC3, IC8):** published at v1 (IC2), v2 (IC3) and v3 (IC8), each a new version with
  its digest pin, never edited in place (`LANE_VERSION`, `tacsim/src/definitions/content.rs:577`).
- **D-18-37 Locality on every inventory path and the under-way quantity seal** are deferred to P-H,
  because the pilot holds one vault and only staff create vaults; the venue gate covers the market; when
  built, the filter sits in offer projection (`world_task.rs:3187-3210`).
- **D-18-38 The daily rhythm is the acceptance.** The slice is done when IC11's queue runs a two-day
  loop unattended and IC12's hand smoke at rate 24 shows a docked hull at the next same-time session.

New in Draft 0.3 (R7, R9; the owner's second-round rulings):

- **D-18-39 The mixed load is the optimum (R7).** Packed volumes 400 / 1,400 / 300 / 2,300 L per t
  make each leg one dense good against one bulky good with a whole-tonne vertex (20 + 80 outbound,
  55 + 45 return); Kasei's metals bid moves 300 → 280 to break Draft 0.2's tie; the vertex condition
  1 < margin_bulky/margin_dense < volume_bulky/volume_dense is the design invariant, a band. No code,
  fixture, NPC line or test stores the best mix: IC8's LP test derives it from `goods.json`, the hull and
  the posted prices (the R9 guard). (Rests on Q-18-33.)
- **D-18-40 The return export.** `industrial_parts` "yard fittings, crated" at 2,300 L/t: the Stickney
  Yards ask 400 from `item.yard_spares`, signed by the service clerk at P11; the Co-op's bid 560 at Ceres
  (intake 5,400 t; capital 3,100,000). The return is a three-limit balance: mass, volume and, early on,
  cash. (Rests on Q-18-32.)
- **D-18-41 `fill` and the clamp under two limits.** `fill` = the joint least fixed point, at or above
  what is aboard, of the burn and crew-days (from `plan_passage` at the depart's `not_before`) for the
  heaviest load the queued orders can make under both cargo limits, no funds term; the cargo clamp =
  min(qty, listing, `max_more` by mass and by volume, funds less the next departure's unmet fuel and
  stores), every bound logged; the executor never optimises across orders. Supersedes §4.4's "41" (the
  greater fixed point; the least is 40). (Rests on Q-18-34.)
  **As built (IC11):** `fill` as written (`fill_targets`, `tacsim/src/port/executor.rs:481`, with the
  reserve for the next departure's needs); an explicit consumable buy short of the next depart holds
  under `HELD_SHORT_FOR_DEPART` on the literal reading even when a later `fill` would cover it (IC11 O4;
  its projection carries the order's own quantity; IC14 revisits).
- **D-18-42 The sky in the slice (R9).** A **constant** sky: `[tacsim.time.ephemeris].rate = 0`, doc 10
  §22 T17 as amended by the owner on 2026-10-15 (§4.8). Every figure here (950,000 s, 19.63 h, burn 41 t,
  21 crew-days, the day-15 buyout) is that sky's output, never an invariant. The astronomy engine is
  post-slice cut P-A; doc 10 §22 and the README superseded list are amended now.
- **D-18-43 The sky seam.** The slice reaches the sky only through tacsim `Sky` (an enum;
  `distance_km(a, ta, b, tb)` the one query, delegating to `separation_km` while Circular;
  `position_km(body, t)` in 3D integer km with z = 0, reserved for the chart; `revision()` as a
  `DefinitionDigest`). `Ephemeris::distance_km_at`/`pinned_km` stay for fixture routes only.
- **D-18-44 The one passage planner** (replaces D-18-30's `route_leg_durations`). tacsim
  `plan_passage(sky, pack, route, drive, dry, payload, occupants, depart_ic) -> PassagePlan` is the only
  source of per-leg distance, duration, burn and crew-days, with a fixed three-pass intercept per leg;
  its callers are `+ship/plan`, `+ship/undock`, the queue's depart, `fill` and the clamp (at the depart's
  `not_before`) and `choose_route` over the port's list. mush-sim keeps a sky-free
  `laden_leg_durations(unladen, drive, payload)`, `route_duration` its Σ over `base_s`; `depart(leg_s)`
  and `CarrierJourney.leg_s` stand as D-18-30 specifies.
- **D-18-45 The coast rate is drive data; the pack's seconds are checked.** `PackLeg.base_s` stays
  required and authored as the pack's pin (T1/Q18A); `dray.json` carries `law: { coast: { kms: 205,
  burn_permille: 105 } }`; IC2's test derives each lane leg from the sky at the epoch and the drive's kms
  and refuses if the pack disagrees by more than 100 s; `RouteDef.distance_km` stays the direct chord at
  the epoch, relabelled, read by nothing in the slice; `passage_burn(law, dry, payload, legs)` is the
  fuel function (Coast distance-free by doctrine: `kms` is the unladen cruise, the laden law and the
  105‰ are both data and their reconciliation under the rocket equation is Q-18-41); one class, one law.
- **D-18-46 The ticket freezes the plan** (extends D-18-11). The departure ticket carries the whole
  `PassagePlan` (route, law, depart_ic, sky_rev, per-leg to/body/dist_km/secs/t0/t1, total_s, burn_t,
  crew_days); `leg_s` is its seconds; nothing after the burn re-reads the sky. T1, load-bearing again.
- **D-18-47 Routes per destination are a list** (amends D-18-20). `routes: { <dest>: [<route id>, …] }`,
  non-empty; the planner takes the fastest at departure, ties to the earlier listing; `via=` reserved;
  orders store destinations, never routes or durations.
  **As built (IC9, IC11):** `choose_route` plans every listed route at departure; `via=` is parsed and
  refused "not yet" (`VIA_NOT_YET`) by `+ship/plan`, `+ship/undock` and `+ship/order depart`.
- **D-18-48 Per-body epochs and real-date positions.** `BodyDef.epoch_ic: Option<i64>`, emitted only
  when present, carried into `Body` and read by `phase_mdeg_at` in place of the config epoch; the lane
  pack's bodies author it at a real IC date near launch on which the **true** heliocentric longitudes
  of Ceres and Mars are about 10° apart, with Ceres, Mars and Pallas at their true longitudes and
  heliocentric distances on that date (never mean elements: Ceres' equation of centre reaches 9° and
  its radius ±0.22 AU, ±30 % on a 0.741 AU leg), so the circular sky is exact at the epoch and only
  drifts after it. Fictional Kirkwood is tuned at its 2.06 AU epoch position with a placeholder period,
  inert at rate 0, and its **orbit is provisional on Q-18-36** (the lane-tied default would put it on
  Ceres's orbit a few degrees astern): IC2 authors Kirkwood's orbit as data the owner has chosen, and
  lane pack v2 re-authors every `base_s` at the flip, retiring IC2's `lane_legs_derive_at_epoch` with
  v1. No slice figure is cited as "real geometry".
- **D-18-49 Prices and the atlas under R9.** Prices stay listing data the applier writes (no pricing
  trait; the engine's policy is a second writer with the applier's authority); quotes freeze prices; the
  programme's references are live reads of the named origin asks, and only the share permille and the
  buyout price freeze at signing (D-18-24); `fill`, the clamp, NPC lore and tests derive from live
  listings. The atlas drawing is a projection; `spacecraft` leaves `SELF_MODES` in IC1 with a test
  (Q-18-6, binding); no slice code reads site coordinates.
- **D-18-50 No passage bound as an invariant.** R3's "under a day" is stated at the slice's constant
  sky; the readiness gate checks the world rate and reports the sky but never asserts a duration; help
  says passage time depends on load and, later, the season. The bound under a moving sky is Q-18-35.
  **As built (IC12):** honoured. The readiness sky line is reported, flagged on a moving sky over a lane
  body without `epoch_ic`, paged once per boot by the port tick, and **never gated**:
  `gated(Binding::Sky)` is false (`tacsim/src/port/coop.rs:526`); the help says passage time "depends on
  the load and, later, the season".
- **D-18-51 belter-charts as inspiration only.** The engine imports public JPL Horizons data through
  its own committed script, vendors no belter-charts code wholesale and takes none of The Expanse's canon
  locations or names; its ideas (a date scrubber, a flip-and-burn planner, honest vs canon-feel
  acceleration, light lag) inform P-A's surfaces.

**Locked decisions this plan touches.** *Leaves:* README superseded 1–7; D-16-* (the arena untouched;
the hauler filter keeps it so); doc 17 D-UM-* (the Dray has no core: Class 0, the drive ROM, nothing
separately titled; the queue is the pilot's instrument, not a mind); T2 (no economic transaction in a
pulse); T4; doc 10 4A (ten families); doc 10 7A (finite capitalization); D-E5-4 (no lock, no exit
rewrite: boarding is a verb and gangways, when they come, use a hook). *Honours:* **T1** (passage time by
route, drive and payload, now applied, and its freeze load-bearing again: the ticket freezes the whole
passage plan); **T17 as amended by R9** (the slice's constant sky; durations derived from it by the one
planner; superseded after the slice by cut P-A); doc 11 §2's Warrant
(every listing has a signatory of its own org; every queued order is the pilot's signed sailing plan,
executed by the port under that signature, and the Class 0 ROM contracts for nothing); doc 17 D-UM-32
and D-UM-11/52 (the ROM flies, never trades); D-14-34 (the
Asker audience); doc 06 §7.5 (the boundary rule, built as written: staff not exempt). *Amends, on the
owner's answer:* C2A–C4A / doc 10 §18.1 / F3's station-channel gate for the hull's departure (D-18-23,
until P2); DL-11 / D-E6-15's frozen party-cap table gains a content-resolved source (Q-18-10); D-14-35's
wording (Q-18-6); doc 11 §4.5 gains the First Captain programme and §4.7 loses the starter-charter line
Draft 0.1 proposed; doc 10 1A is honoured as written for its scales and compression (one OOC hour per
IC day; a Belt passage of "weeks" IC in about a day real) and for "transfer conditions modify routes
rather than closing destinations", so the Draft 0.1 three-day amendment is withdrawn — while its
rejection of simulated ephemerides is superseded after the slice by R9; doc 10 §22 T17 gains the
amendment of §4.8; doc 14 §3.8 S7's placement of the sky epoch in server config moves to the pack's
bodies (D-18-48). *Withdrawn:* the E7 "market never mints" pin amendment (R4 makes it unnecessary).

---

## 8. Questions for the owner

**Answered on 2026-10-15, first round** (the ruling in one line): **Q-18-1** Ports: Ceres Freeport →
Kirkwood → Stickney Yards; Tycho an agency, not the ship desk. **Q-18-2** World rate 24 for play.
**Q-18-2b** Lane length: about a day real, offline, a 30–60 minute daily session. **Q-18-4** A charter,
never a required purchase; a buyout exists. **Q-18-8** Finite seeded stock; no issue/retire. **Q-18-9**
Finite one-shot vendor funding.

**Answered on 2026-10-15, second round (R8: "other defaults are accepted"), each as recommended and
now binding:** **Q-18-3** provisions opaque, crew-days, `occupants × ceil(days)`. **Q-18-5** optional
`UnitSpec.logistics`/`interior`, no `ComponentRole` drive and no `drive` rules kind (the drive table is
the index over fielded definitions). **Q-18-6** D-14-35 decided as two records with a hard boundary;
`spacecraft` out of `SELF_MODES` in IC1 with a test. **Q-18-7** the burn settles at undock, matter first,
the tick launching the ticket. **Q-18-10** party caps by content reference, conferring no money
authority. **Q-18-11** packs declare money as numbers under the cap. **Q-18-12** names ok (18a §4; the
sixth signatory and the fittings good take their names from 18a). **Q-18-13** the share is 250‰ of
margin over reference, paid inside the sale; net with a run book stays P9. **Q-18-14** stake 25,000,
buyout 90,000 fixed at signing, one enrolment per account ever, 8 hulls, no top-up. **Q-18-15** the
laden law applied now (through the planner, D-18-44). **Q-18-16** `@tacsim/time rate=24` on the next
owner-approved redeploy; never redeploy without asking. **Q-18-17** the pilot must be aboard; a passenger
ashore is left behind. **Q-18-18** boarding by verb; gangway exits P-G. **Q-18-19** stock for one cohort
(resized under R7, D-18-27); restock only by P5; no top-ups. **Q-18-20** no berth slots (P-F).
**Q-18-22** `+ship/orders` / `+ship/order`; 16 orders, 32 log lines; no repeating plans. **Q-18-23** 13
+ 12 rooms; the freight rooms P-F. **Q-18-24** the signed sailing plan executed by the port; the ROM
only flies. **Q-18-25** the six rooms plus the store, plain names. **Q-18-26** no minimum fuel reserve.
**Q-18-27** the 1-minor Manual-clock rent plus the stale-page exemption. **Q-18-28** the queue editable
aboard or in the port area. **Q-18-29** plain treasury capitalization for Kasei. **Q-18-30** the Trust a
separate org. **Q-18-31** the port tick releases a lapsed player listing's hold as the holder.

**Q-18-21 Volume capacity — REVERSED by R7 (owner, 2026-10-15):** *"both volume and mass are critical
components of capacity .. that's part of the trick is balancing the payload from station to station to
utilize both the available volume and mass."* Draft 0.2's mass-only default is replaced by D-18-28 as
rewritten and D-18-39/40/41: both limits bind, the packed volumes are re-authored so the balanced load
is the optimum on each leg, and the capacity function runs everywhere a hold is written or shown.

**Open — what the two Draft 0.3 designs could not settle** (each with a recommended default; "as
recommended" builds §5 as written):

- **Q-18-32 A bulky Stickney export, so the return leg is also a balance.** Default: yes,
  `industrial_parts` "yard fittings, crated" at 2,300 L/t, Stickney Yards ask 400, Co-op bid 560 at
  Ceres (D-18-40; a vendor, a sixth signatory, P11 in the port entry, 3.1M of Co-op capital; zero new
  NPCs, orgs or rooms). Alternatives: the return stays alloys-only and mass-bound (the m³ line reads
  slack on half the legs; the buyout slips to day 17); another bulky export (habitat consumables from
  the Yard Commissary) on the same numbers. Why: R7 says "from station to station"; alloys alone use 30
  of 120 m³ and the return has no decision.
- **Q-18-33 Which price breaks the outbound 100/100 margin tie.** Default: Kasei's metals bid 300 →
  280 (vertex 20 + 80; outbound net 5,390; day 15). Alternatives: Kasei's ice bid 200 → 220 (outbound
  6,890; the buyout on day 13 with fittings at 560, too easy against the accepted 90,000; day 15 with
  fittings at 540 but a return ratio of 1.17 near the band's edge); Occator's ice ask 100 → 90 (moves the
  share reference). Why: it keeps every reference and share term unchanged, touches 20 t a leg, and sits
  at 1.25 inside 1–3.5.
- **Q-18-34 Should `+ship/plan` or the queue solve the best mix for the captain?** Default: no;
  `+ship/plan` shows room by mass and by volume per good, the affordable tonnage and each queued buy's
  clamp and bound; the captain strikes the balance. Alternatives: `+ship/plan best` as printed advice;
  a `buy <a>,<b> balance` order the clamp solves. Why: the owner calls the balance "part of the trick";
  a solver turns it into a button; an advisor fits a later NPC or unit-mind product.
- **Q-18-35 R3 against R9: how does the daily passage survive a moving sky?** At nominal cruise a coast
  Dray makes the full Ceres–Stickney outbound in under 24 h only while Ceres–Mars ≤ ~1.6 AU, about 16 %
  of the 3.18-yr synodic cycle; at the worst season ~65 h. Default (at P-A, a data change behind the
  tagged law): the Dray's drive becomes a low-thrust flip law tuned so a brimmed 421 t hull's worst
  season is about 22–24 h and its best 12–13 h (about 0.07 g with exhaust near 10,000 km/s: a burn of
  7–13 % of departure mass; the lore cost is that a hauler sustaining 0.07 g for a day with 3 % c exhaust
  is no longer doc 11 §7's "slow, efficient" Dray against the Vigil's torch, and 18a's "two to three
  weeks" holds only by coincidence). Alternatives: a seasonal coast Dray (multi-day passages for weeks at
  a time; the queue copes, a session may find the ship under way); coast with a cruise choice (fuel
  buys time; the 80 t tank caps a full hold at roughly 2.7 AU for a sub-day passage, about 40 % of the
  cycle); a slower sky (Q-18-37). Why: R3 is the spine of the career, and 1A rejected simulated
  ephemerides for exactly this "long non-opportunity"; the owner is re-opening that trade knowingly.
- **Q-18-36 Kirkwood's orbit (re-presented; Draft 0.3's default rested on a wrong number).** The
  slice's legs (0.741 + 0.561 AU) put Kirkwood at about 2.06 AU and 135°, inside the 10° Ceres–Mars gap
  at the epoch, and at rate 0 that is all the slice needs; the orbit matters only at P-A. An
  independent 2.06 AU orbit (Draft 0.3's default, "on the lane about 70 IC days per cycle") is in fact
  on the lane once in roughly 40–110 years — a via-Kirkwood season is a three-body alignment whose
  offset walks 138° per synodic cycle (§3-Q (4)) — so the Co-op's "transfer node", the Republic
  pickets, P4's "every corridor ship passes Kirkwood" and 18a's job board all break the day the sky
  moves, and the engine would have to re-author the depot's orbit: the R9 regret. **Default,
  recommended: Kirkwood on Ceres's own orbit, trailing by a few degrees** (2.766 AU; 3° astern is
  0.145 AU, about two hours at cruise): the Ceres–Kirkwood leg is a constant short hop, Kirkwood–Mars
  is the seasonal chord, every Ceres-bound ship passes it, and via-Kirkwood is a steady ~9 % detour
  over direct (at the epoch 0.145 + 1.267 ≈ 1.41 AU against 1.293) that the depot's propellant and the
  pickets make worth taking. Its costs: lane pack v2 re-authors the legs and the slice's 0.741/0.561
  split is replaced rather than kept (a full outbound about 21 h at rate 24 and a brimmed hull about
  24 h at the epoch sky, so R3's bound is tighter and Q-18-35 bites sooner), and doc 11's "inner edge
  of the Belt" becomes Ceres's outer harbour. Alternatives: keep the independent 2.06 AU orbit, with
  lane pack v2 listing direct routes and the pickets reading routes actually flown (the depot is a
  Belt-edge waypoint that is sometimes on the way; the fiction above is rewritten); a powered or cycler
  depot with authored elements as a lore lever (no stable point exists between two bodies on different
  orbits, so it is a fuel budget the fiction must pay). Co-orbiting at 2.06 AU is physically
  impossible. Until ruled, IC2 authors Kirkwood at its epoch position with a placeholder period and
  nothing reads its orbit (D-18-48 provisional).
- **Q-18-37 The sky rate under world rate 24.** Default: 1000 (real motion), so the sky's date is the
  IC date, which real data requires; the corridor's season turns about every 48 real days and Pallas's
  latitude cycle about every 35. Alternative: a separate slower sky clock (e.g. 250, a ~194-real-day
  cycle), a second calendar every surface would have to explain.
- **Q-18-38 The engine's sky data.** Default: Keplerian element sets fitted from JPL Horizons at era
  epochs (per-body `epoch_ic`, sets valid ±5 IC years), validated against a ~58 KB quarterly golden table
  for 2340–2400; a few hundred bytes per body, self-referenced in any world, and Kirkwood is fictional and
  needs elements anyway. Alternative: belter-charts-style packed daily vectors as a content pack (higher
  fidelity; ~219 KB per body per 25 years; a 2340–2365 window lasts about 228 real days from IC 2350 at
  rate 24); or both, vectors as a near-window cross-check.
- **Q-18-39 How far the position-driven price policy reaches.** Default: a capped once-per-IC-day
  reach premium on demand-side bids plus emergency Wanted bids for life-support goods when a port's reach
  crosses a threshold, funded from the org's finite capital (doc 10 7A); quotes stay frozen and the
  programme's reference follows the moved ask (D-18-24), so the lay stays a margin share under the
  policy. Alternatives: premiums only; emergency contracts only; no policy (the sky moves only time
  and fuel). Why: "Pallas might be willing to pay more for emergency supplies when awkward" is a demand
  signal tied to reach, and finite capital keeps R4.
- **Q-18-40 Solar keep-out.** Default: a **cost, never a closure** — a chord passing inside 0.3 AU
  takes a detour penalty in the planner, so a conjunction-season direct route loses to via-Kirkwood or
  a waypoint, and a P-A validator rule keeps at least one non-refusable route in every port's list per
  destination; because `choose_route` picks only from the port's list (§4.8) and the kept 1A rule says
  transfer conditions modify routes rather than closing destinations, a planner that refused the only
  listed route would close a port for a season. Alternatives: refuse the chord outright (sound only
  with the validator rule); ignore it (the planner's distance is an abstraction anyway).
- **Q-18-41 Coast under the rocket equation: does mass slow the cruise or grow the burn?** The slice
  flies the laden law (a 142 t payload 1.79× slower at the same 105‰ burn) and calls both doctrine
  (§4.4); under the rocket equation a fixed burn fraction is one Δv and one cruise whatever the mass.
  Default (at P-A, data behind the tagged law): keep the laden law — a thrust-limited hauler accepts a
  slower cruise — and keep the burn a doctrine fraction, so every slice figure and 18a's "a week longer
  on a full hold" survive and `exhaust_kms` is documentation. Alternative: the physical coast — the
  burn from `exhaust_kms` and the cruise constant, so a full hold costs propellant rather than hours
  (the laden law dropped; passages lose their load-dependence, which T1 would have to re-read as
  fuel-dependence). Why it is the owner's: it decides what "slow, efficient" means for a Dray.

**Status at Draft 0.4 (IC0–IC12).** Every one of Q-18-32 to Q-18-41 is still open; the slice built on
the defaults where it had to:

- **Q-18-32** (a bulky Stickney export): built on the default — `industrial_parts`, "yard fittings,
  crated", at 2,300 L/t, the Stickney Yards ask and the Co-op's Ceres bid (IC2, IC8); open for the owner.
- **Q-18-33** (the outbound margin tie): built on the default — Kasei's metals bid at 280 in lane v3
  (IC8); open.
- **Q-18-34** (a solver for the mix): built on the default — no solver; `+ship/plan` shows the room by
  mass and by volume per good and each bound, never a recommended mix (IC9, IC11); `trade.md` explains
  the balance (IC12); open.
- **Q-18-35** (R3 against R9): open; the slice flies the constant sky and states no bound outside it.
- **Q-18-36** (Kirkwood's orbit): open and untouched (the placeholder 1,080-day period stands).
- **Q-18-37** to **Q-18-40** (the sky rate, the engine's data, the price policy's reach, solar
  keep-out): open; P-A's.
- **Q-18-41** (coast under the rocket equation): open and untouched.

**Raised by the cuts (IC0–IC12), each with the default the tree took (the owner may override).** Related
items are merged; the cut notes carry each default's full reasoning.

- **Q-18-42 The hold-backed player sale's Fact gate.** A consume refused at prepare or dropped by
  back-pressure leaves a Before-only contract that settles and pays the vendor for nothing (§9 risk 3).
  Default: no cut Fact-gates the hold path (IC7 (5)); candidate: IC14 or a P-cut, reusing the Draw's
  `Expiry::Refund` and the sale's Fact gate.
- **Q-18-43 The stale-contract exemption's reach.** Default: only a live lease-shaped contract with an
  empty escrow list is exempt (IC6's exemption narrowed in IC7).
- **Q-18-44 The rent.** Default: 1 minor on the Manual clock, kept although zero is admitted (IC6 (2)).
- **Q-18-45 The lease-gate reading (the D-18-34 amendment, §7).** Default: `+ship/board` gated on the
  lease and the placement, `+ship/leave` on the placement alone (IC6 (1)).
- **Q-18-46 `+ic`'s landing room.** Default: C03 Port Watch Arrivals for Ceres, P01 for Stickney (IC1).
- **Q-18-47 The accept line's signatory and the other unowned presentation residuals.** Default: not
  built — the accept line naming the listing's signatory (IC8 L17), the Engineering/Bridge
  burn-and-reserve gauges (IC11 L21), the C14/P05 departure boards and the C01/P01 berth boards (IC9,
  IC11), P07's funded balance (IC8, IC10); the figures are on `+ship/hold`, `+ship/plan`, `+ship/orders`
  and the boards; owner to place (IC14 or a content cut).
- **Q-18-48 A Draw refused by expiry locks cash for up to an hour.** Default: refund by expiry at
  3,600 s (the tree cannot cancel an Active contract); the listing under-reports the refused tonnes
  until then (IC7 (1)).
- **Q-18-49 Bids re-post at full intake.** Default: a lapsed or refunded bid is re-posted at its full
  authored intake under "no restock"; re-posting at the lapsed remainder is P5 (IC10 (4)).
- **Q-18-50 The reference read at contract build.** Default: at build, not settlement, so the payouts
  sum to the escrow at offer; drift at most one window (IC10 (6)).
- **Q-18-51 The explicit consumable buy's literal hold.** Default: a `buy propellant <n>` short of the
  next depart holds under `HELD_SHORT_FOR_DEPART` even when a later `fill` would cover it, and its need
  is projected with the order's own quantity aboard (IC11 O4; IC14 revisits).
- **Q-18-52 The generic aboard lines.** Default: mush-sim's generic boundary and arrival lines are heard
  beside tacsim's flavoured ones (`REPLACE_GENERIC_ABOARD_LINES = false`; IC9 (1)).
- **Q-18-53 The clearance rules.** Default: a filed or held clearance locks board, leave and loading; an
  unburned live clearance lapses after 24 fires; a held launch waits for staff; a staff stand-down
  refunds nothing; there is no player abandon verb (IC9 (3)(4)).
- **Q-18-54 Org renames.** Default: refused as drift on a port update; renames are P5 (IC3 (2)).
- **Q-18-55 The per-instrument mint cap.** Default: 1,000,000 contributed in code per instrument, shared
  by every future `tacsim.credit` faucet obligation (IC3 (1)).
- **Q-18-56 The lane's date and the Dray's cruise.** Default: the real date 2351-05-13 at 226 km/s (the
  alternative, the 2354-06-09 season at 205 km/s, about 15 h brimmed; IC2's lead ruling).
- **Q-18-57 The daily-depart roll and the queue's limits.** Default: a daily `HH:MM` reached past
  `until=` rolls to the next day (a fixed instant past it holds); `at=` at most 7 days ahead, `until=` at
  most 24 h, 15 listed passengers, prices at most 1,000,000 major (IC11 O2, O6).
- **Q-18-58 The listed passenger.** Default: a person aboard other than the pilot when the depart is
  filed; one ashore at launch is left behind and both are told (IC11 O3).
- **Q-18-59 Fiction figures in room prose.** Default: the Dray's `120,000cr` list price (C05, C09, P11)
  and the C14 departures board's "two to three weeks in a good season" stay; every live market figure is
  struck from details (IC12 O3).
- **Q-18-60 Treasury and Bids gate enrolment.** Default: every readiness line but the sky gates
  `+coop/enrol` — a missing treasury account or a missing or apply-waiting bid closes it, a lapsed bid
  for the fire or two the clerk takes to re-post (IC12 O2; D-18-50 keeps the sky out). **The reach the
  owner should rule on (IC12 review):** the Bids line also reads Pending while a house cannot fund one
  unit of its bid (`readiness.rs`, `board.rs::funds_one`), so ordinary play can close enrolment world-wide.
  Lane v3 capitalizes Kasei at 600,000cr against intakes worth 3,040,000cr (4,000 t × 280 + 9,600 t × 200);
  a story-shaped round trip pays Kasei about 21,600cr (20 t of concentrate, 80 t of ice) and returns it
  roughly 10,000cr in alloys and propellant (the fittings are the Yard's), so on the order of fifty round
  trips — sooner while sale escrows are held — leave Kasei unable to fund a tonne. From then every new
  player at the Registry desk hears `NOT_OPEN` ("the programme is still being set up") and the gate logs
  at `info`, never paged, until staff refund the house or players buy enough from it. Before IC12 the
  Bids line was reported, not gated: enrolment stayed open and the sales skipped with
  `HOUSE_CANNOT_PAY_SKIP`. The alternatives: gate the Bids line only when a bid is Missing (hand-withdrawn
  or apply-waiting), not on the funds-Pending or lapsed-reposting states; or keep the gate and page at
  `error` once a boot when a funds-Pending bid closes enrolment. Not built either way: it changes O2.
- **Q-18-61 The story test's rate wording.** Default: §6's "rate 3600 the only staff act, none after
  boot" is built as "boot at `[game].ic_ratio = 24`, enrol at 24, set 3600 as the one staff act after
  the enrolments" — the production Rate line refuses enrolment at any other rate (IC12 O6).
- **Q-18-62 A staff `@inv/move` out of a hold under way.** Default: not gated — the under-way quantity
  seal on every inventory path is D-18-37's deferral to P-H; a pilot's own move is refused because the
  pilot acts on one endpoint (the story test's negative checks).
- **Q-18-63 Q-18-19 taken stricter, and the sweep's kept lapsed listings.** Default: any change or
  removal of a recorded stock row is drift; a lapsed hold-backed listing stays until its hold is Gone, a
  generic market change for every roster (IC8 (2)(3)).
- **Q-18-64 A seller may dispute their own sale.** Default: allowed; it freezes the house's escrow and
  the bid's intake until staff resolve (IC10 (8); IC14 closes).
- **Q-18-65 Every player-facing text the briefs wrote.** Default: the briefs' wording in the house voice
  — IC6's enrol, board, leave and statement lines (IC6 (4)), IC7's market refusals (IC7 (4)), IC9's
  clearance and plan lines (IC9 (5)), IC10's sale line and refusals, which reuse the venue gate's
  load-worded lines (IC10 (1)(2)), IC11's verbs, renders and executor log lines (IC11 O1), and IC12's
  help topics and re-worded in-code helps (IC12 O1).
- **Q-18-66 Forfeiture of consumables at a recall.** Default: a recall (dormancy, revoke, or a
  voluntary `+coop/release`) re-homes the hold vault to the Trust with its remaining propellant and
  provisions and the next charterer inherits them; §4.1's "no confiscation of matter in the slice" is
  read as covering cargo only (IC13 O1).
- **Q-18-67 The full recall rule for revoke and release.** Default: both require docked at Ceres,
  empty cargo, nothing filed, in flight or moving; a charter ended away from Ceres leaves the hull out
  of the pool with nobody to fly her home; there is no staff "bring her home" verb (IC13 O2).
- **Q-18-68 The staff reason in the revocation notice.** Default: `@coop/revoke`'s `why` is shown to
  the pilot in `REVOKED_NOTICE` (IC13 O3).
- **Q-18-69 The laid-up name restored.** Default: the recall restores the Trust's laid-up name and
  frees the pilot's chosen one; the registry mark is untouched (IC13 O5).
- **Q-18-70 The pool shrinks per graduate.** Default: a graduated hull leaves the Trust's pool for
  ever; the slice buys no replacements, so the desk answers `NO_HULL` once all eight are chartered or
  graduated (IC13 O7).

**Answered on 2026-10-18, third round (R10), before the first dev1 playtest** (the owner, taken through
every open ruling in this document and in 16c §5; recorded here, binding):

- **Q-18-60 — gate on Missing only.** A hand-withdrawn or apply-waiting bid closes `+coop/enrol`; a
  funds-Pending or re-posting bid does not: the sales skip with `HOUSE_CANNOT_PAY_SKIP` as before IC12,
  and the port tick pages once a boot while a house cannot fund a tonne. **Built as the post-loop cut
  P-B, landed 2026-10-18 at `e682a7ec`** (on top of `29230ff5`; the hash written by cut AQ's commit, the
  next on the same branch, since a commit cannot carry its own) — after the deploy, as ruled: a playtest
  cannot reach the fifty round trips that drain Kasei. The record is
  [18-cut-notes.md](18-cut-notes.md) "P-B".
- **Q-18-71 A failed relet.** Six retries under a doubling backoff (2 min to 1 h), then the revoke IC13
  built; the pilot told `RELET_PENDING` once, the plan waiting under `HELD_RELET`; no `@coop/unload`.
  **Ratified as built (IC14 O4).**
- **Q-18-72 The `Markers` line never gates** `+coop/enrol`; only an actually incomplete mirror (the
  `InventoryFacts` line) closes it. **Ratified as built (IC14 O8)** — the same reading as Q-18-60's.
- **Q-18-73 An unreadable sailing plan** keeps that hull out of the pool and pages; it never closes
  enrolment world-wide. **Ratified as built (IC14 O9).**
- **Q-18-74 Marker retention past E7c2's D1.** The applied marker stays the reboot-surviving replay
  guard for at least one real day and at least the newest 32 per endpoint; a request replayed after
  both is no longer refused by it; a ring of 128 young markers refuses the next move from that endpoint.
  **Ratified as built (IC14 O7)**; the un-pruned rings of endpoints that stop writing (IC14 O10) stay
  the remaining bound of §9 risk 19.
- **Q-18-42 to Q-18-70 — every default as the tree took it is ratified**, including the three IC14
  changed: a seller cannot dispute their own sale into a bid (Q-18-64, O2); an explicit consumable buy
  holds under `HELD_SHORT_FOR_DEPART` only when the later consumable orders before the next depart would
  still leave it short (Q-18-51, O3); the accept-line signatory, P07's funded balance and the
  Engineering/Bridge burn line built, the C14/P05 departure boards and the C01/P01 berth boards to a
  content P-cut (Q-18-47, O5).
- **Q-18-35 to Q-18-41 stay open for cut P-A**; the slice flies the constant sky and states no bound
  outside it. Kirkwood's placeholder period stands in prose only (Q-18-36).
- **The playtest deploy (Q-18-16's "next owner-approved redeploy"):** dev1 on a **fresh store** at
  `IC_RATIO=24` (no pack applies by hand, no `@tacsim/time` flip, the staff account bootstrapped again),
  with the unit-minds programme through M6 merged first (main fast-forwarded onto
  `codex/unit-minds-through-m6` at `6fafaa11`).

---

## 9. Risks, and what this slice refuses

**Risks.**

1. dev1 keeps its world: every new boot pack id applies at the next restart and any refusal aborts the
   boot (`mush-content/src/module.rs:113-139`). IC1–IC3 carry validate-clean, digest and kept-world boot
   tests; no redeploy without the owner's ask. One known refusal path: `open_passive_domain` refuses
   while eight windows are open (`window.rs:205-206`) although a passive domain takes no window slot,
   so a kept world with eight open windows at restart would fail the lane pack's first apply — IC3
   skips that guard for passive opens and tests the case.
2. The burn-then-launch ordering: a ticket with no launch (tick stalled) leaves a burned hull at the
   berth until reconcile; IC9 asserts exactly one launch across a restart. The one `tacsim.port` tick now
   runs the stock, launch, dock and order arms, so a stall in one delays every pilot's offline day; each
   arm is budgeted and pull-idempotent, and IC11's restart test covers a mid-step kill.
3. The paid-for-nothing class survives on the **buy** side only: a Draw refused at drain after the money
   is in force (the contract-keyed lot leg makes the re-stage a replay HIT, and the executor re-stages
   under the same key before holding). The sell side is closed by the Fact-gated payout.
   **As built (IC7, IC10, IC11):** on the buy side a Draw refused at prepare is refunded **by expiry**
   at 3,600 s, not cancelled, so the buyer's cash is locked for up to an hour (Q-18-48); the sell side
   is closed by the Fact-gated payout as written; the **hold-backed player sale path stays open** (a
   consume dropped by back-pressure leaves a Before-only contract that settles for nothing; Q-18-42). A
   queued depart's lost burn is re-filed under a fresh `seq`, never the same key (IC11 L7).
4. The Draw arm relies on the listing quantity staying equal to the storefront pool; a staff `@inv/move`
   out of a storefront desynchronises it. The applier and the readiness report compare the two and page.
5. The share is computed only where a lot sale is built; any later sell path must consult the
   `ProceedsPolicy`, or cargo can leave an enrolled hold share-free. In the slice the only other route out
   of the hold is the burn.
6. The projection change (`graph.rs:167`) touches every Interior occupant: `custody.rs:212-220`'s comment
   goes stale and the e7c6 and placement suites that assume "Interior means the space object" need review.
7. Clustered daily departure times serialise through the engine-wide one-slot admission queue shared with
   live players (`engine.rs:688-721`); burns and lot legs are retried by pull, and a cluster of eight
   completes in seconds, but the executor's work budget must stay well under 64.
8. The actor swap is general power; the executor stays a closed set of typed calls with a lease re-check
   per step, and review refuses any arm that dispatches text.
9. Finite stock: at the sized seed one cohort saturates Freeport Works' bid in about three weeks; its
   4,000,000 is a season budget, after which alloy returns stop and the loop degrades to one way. The cut
   note for IC8 says so; P5 is owed before a second cohort. Money pools in seller orgs while the alloy
   buyer drains; a recapitalization path (attributed, receipted) is needed before the bid empties.
10. The Trust's issuer cap: a pool above 16 would starve it silently, because `offer_lease` checks no cap.
11. Rate 24 is a deploy decision; on dev1's `IC_RATIO=4` a full leg takes five days and R3 fails. The
    readiness gate refuses rather than misleads.
    **As built (IC3, IC12):** the readiness gate refuses enrolment at any rate but 24 on a production
    world (`PLAY_RATE`, `tacsim/src/port/readiness.rs:37`), and since IC12 at any non-ready line but the
    sky; a player sees only `NOT_OPEN`. The rate-24 deploy line is in `docs/ops.md` (IC12).
12. IC0 landed (`c87d78f7`) moving `mush-market` `trade.rs`, `lot.rs`, `commands.rs`, `module.rs`,
    `api.rs` and `mush-sim` `commands.rs`; this draft's cites into them are at `c87d78f7` where marked
    and at `8a32734a` otherwise, and IC0's test comments and cut note call Draft 0.2's IC7 "IC5"
    (Draft 0.1's numbering; §5 carries the mapping).
13. The stale-contract page: until IC6's exemption lands, any test that keeps a lease Active for seven
    real days pages; recovery and housing leases have the same latent defect today.
14. Doc drift a builder will hit, as Draft 0.1's risk 11, plus `e7r2.rs:327-328` ("exactly
    extraction's"), `custody.rs:212-220`, and 10b §3.11's issue/retire note, which is withdrawn; and
    the frozen-sky texts until §10's amendments land (doc 11 §2/§5 "Kirkwood on the lane", 18a's "two to
    three weeks", doc 10 1A's rejection of ephemerides, `route.rs:231`, `commands.rs:435`).
    **As built (IC12):** the in-code drift is fixed or found already true (§10 as built); the doc drift
    outside this cut's files is still owed (§10).
15. **R3 against R9 (Draft 0.3).** A coast Dray at nominal cruise makes a sub-day full-load passage
    for only about 16 % of the Ceres–Mars cycle (circular bounds). Until Q-18-35 is ruled, no gate, test
    or help text may encode "every passage < 24 h" (D-18-50), and R1's "two weeks of good runs" and
    D-18-27's sizing hold only in a good season.
16. **The vertex is a band, not a point, and cash-bound loads depend on order sequence.** An ice bid
    below 180 or a concentrate bid above 300 tips the outbound to a corner — once P-A's prices move, that
    is the play; the risk is a fixture, NPC line, help text or test that hard-codes 20/80 or 55/45 (IC8's
    LP test derives them). The clamp acts order by order, so a captain who files alloys before fittings
    when short, or re-uses yesterday's 49/55 orders with full funds (5,630 against 7,740), gets a worse
    load; `trade.md` and `+ship/plan`'s per-order bound make it legible. `fill` over-bunkers a cash-bound
    leg by a few tonnes (about 200 cr once).
17. **Delegation-gated reads and money pooling.** The Draw backstop and the executor read sibling pools
    inside an `ActionTxn`; a refused read poisons it (`action_state.rs:2043-2068`): the market is a
    delegate, tacsim needs IC5's before IC11, `read_pool` surfaces errors, never zeros. The Co-op pays
    about 25,200 per full return against 3.1M and Freeport Works' 7,000 t intake binds at about 127
    returns; two season budgets drain together, so risk 9's path (P5) matters twice over.
18. **The sky seam.** `sky_rev` on every plan and the ticket freeze keep a committed passage independent
    of `dray.json`, the sky config and f64 drift (the slice's Circular distance delegates to
    `separation_km`, so no new rounding path). Three "rate" knobs (world, the inert `[tacsim.time].rate`,
    the sky rate) invite operator error once the sky moves; P-A collapses the sky knob to `constant |
    real`. Whether the burn's prepare can read the pack and sky is unverified, so the plan is the action's
    input and the prepare recomputes only burn and crew-days from its seconds. A planner without
    intercept would understate tangential legs at P-A; IC9's rate-1000 test is the only guard. Kirkwood
    customs (P4) assumes every corridor ship passes Kirkwood, true under Q-18-36's lane-tied default and
    false for decades at a time on an independent 2.06 AU orbit once direct routes exist. The coast
    law's laden multiplier and its flat burn contradict each other under the rocket equation (Q-18-41);
    both are data behind one seam, so the slice pre-empts nothing. belter-charts bundles The Expanse's
    canon; only public JPL data is imported.

19. **(Draft 0.4) The IC10 pre-deploy blocker.** `inventory_applied` has no retention past
    `MAX_APPLIED_MARKERS` = 4,096; beyond it every sale into a bid lapses unpaid with a page each. No
    deploy that enables selling to a long-lived world before IC14's first item lands (`docs/ops.md`, "The
    trade run (doc 18)"). Risks 9 and 17 stand unchanged.
    **As built (IC14):** bounded, not closed. Each source endpoint's markers sit in a ring
    (`inventory.marker_ring`) that evicts only past 32 entries **and** past a real day of age, so no
    marker a sale's Fact, re-stage or a re-driven offer can still read is retired (E7c2 D1 holds inside
    the horizon; a replay after it is unguarded — an owner item); a ring of 128 young entries refuses the
    next move. The mirror's 4,096 and its world-wide fail-closed rule stand: the cliff is now roughly 128
    endpoints holding 32 or more, the `Markers` readiness line flags and pages at three quarters (never
    gating, proposed Q-18-72), and the rings of endpoints that stop writing are never pruned. Selling is
    no longer blocked before a deploy; the `Markers` line is the operator's watch. A sale still in
    flight past the horizon (a dispute held for days) may lose its marker, so the market reads an absent
    sale marker as "not moved" only within a real day of the deal's acceptance; past it the lapse is
    unconfirmed (paged, the bid never restored over goods that may have arrived).

20. **(IC14) What the residuals sweep leaves and changes.** It leaves, from IC13, the hand-played
    two-session smoke at rate 24 (the owner's, on dev1 after an approved redeploy), Q-18-66..70 (§8), and
    e3b's "writes nothing" checks through `fold_kv` (which drops `StreamAppend`) with the cross-epoch
    cutoff and the Done-depart arm proven by unit tests only. It changes two defaults: Q-18-51 (an
    explicit consumable buy holds only when the later consumable orders before the next depart still
    leave it short, §4.4's wording) and Q-18-64 (a seller cannot dispute their own sale into a bid). It
    raises four owner questions, numbered here for §8's next docs pass and recorded in the IC14 cut note:
    **proposed Q-18-71** (a failed relet retries six times, then revokes as IC13 built), **Q-18-72** (the
    `Markers` line reports and pages but never gates, departing from Q-18-60's "every line but the sky
    gates"), **Q-18-73** (an unreadable sailing plan keeps its hull out of the pool and pages, but does
    not close enrolment world-wide) and **Q-18-74** (the marker retention horizon of one real day and the
    hard cap of 128, against D1). Two §10 sentences stay as written because §10 is outside this cut's
    doc 18 edits: "the code default `mortal` is left for IC14" is dispositioned edge (the single-game
    cleanup), and "`18-cut-notes.md:109-110` is still owed" was done in IC12's note
    (`18-cut-notes.md:5848-5857`).

**Refused, deliberately.**

- A loan, a revolving line, an advance, interest, a lien or an overdraft (R2; invariant 26); a required
  hull purchase or a hull minted per pilot (R1); a market Asset sale of hulls before the buyout.
- Matter created at a purchase or retired at a sale (`LotOp::Issue`/`Retire`); any restock sweep; a
  listing whose quantity is authored rather than live; a reservation-backed port ask, Renew or
  `settle_part` in the slice (Draw avoids them).
- Any GOD-tick authoritative submission, staff act or NPC act on the player path; submissions from the
  applier or the boot pass.
- `@wait` or any command-text re-dispatch as the queue's carrier; free-text orders; unattended
  repeating plans; the `+orders` name.
- Six interior records, a second placement field, or unplaced riders; a walkable room as a vault
  backing; a takeable storefront item; a static exit to a specific hull or to the last station visited;
  gangway exits, `BerthLink`, berth slots and berth-holder markers (P-G, P-F).
- Stations, billets, helm seats and a seat-reach re-check (P2); a live-seat departure gate.
- Payload-independent passage time (T1 is applied); a minimum fuel reserve, passengers and a manifest
  beyond the tenant (reserved berth); port fees, Kirkwood customs or holds (P4).
- A mass-only hold, a fixed-field stowage schema, a "fullness" figure that collapses mass and volume,
  a stored best mix, or a verb that solves the balance for the captain (R7; Q-18-34).
- Moving the sky in the slice; authored seconds read as a passage time; a route-level distance read as
  the flown path; a pricing trait, a chart or a light-lag caller before P-A; any gate, test or help text
  asserting a passage bound outside the constant sky; belter-charts' code wholesale or The Expanse's
  canon locations and names (R9; D-18-42..51).
- Capacity or locality checked only on the market leg as the final shape (P-H); rewriting exit locks per
  crew change (D-E5-4).
- Reusing the `tacsim.corridor` id or editing the corridor fixtures or the starter packs; a Luna port, a
  Pallas station, Earth/Luna on the map.
- An 11th family, a thrust mobility mode, a `drive` rules kind, a `lift` portal mode.
- Redeploying dev1 without the owner's ask.

---

## 10. Supersessions and amendments owed

- **README.** The doc 18 row becomes:
  `| [18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) | **The trade run, the first IC vertical slice (Draft 0.3, 2026-10-15, at 1fc40d01; D-18-1..51, every Draft 0.2 default binding under the owner's rulings R1–R9, Q-18-21 reversed, Q-18-32..41 open)**: a pilot walks from Limbo into Ceres Freeport, signs the Belt Carriers' Cooperative's First Captain programme (a chartered Co-op Dray under \`Encumbrance::Lease\`, a 25,000 cr stake, a quarter of the trading margin to the Co-op inside every sale, a 90,000 cr buyout two weeks of good runs away), loads a hold that binds on 100 t **and** 120 m³ — 20 t of concentrate that weighs it out and 80 t of ice that cubes it out, the balance being the trick of the run — plus reaction mass and provisions from finite receipted stock, queues the captain's orders and logs off; the ship burns, flies the Ceres–Kirkwood–Stickney lane in about twenty hours at rate 24 under the slice's constant sky, docks itself, and the queue sells to Kasei, buys crated yard fittings and alloy stock and departs again at the pilot's hour; the verified map A–R, the design (backer, interior, goods and the capacity function, the passage planner over the sky at departure, the queue, stations, pipeline, the sky seam and the astronomy engine's brief as post-slice cut P-A), the Dray definition, fourteen cuts (IC0 landed at c87d78f7; IC9 first flight; IC10 closes the loop; IC11 the first daily-loop cut), the two-day story test |`
  and the 18a row becomes:
  `| [18a-trade-run-stations-and-ship.md](18a-trade-run-stations-and-ship.md) | **The trade run's places (content brief, Draft 0.1 revised with doc 18 Draft 0.3, 2026-10-15)**: Ceres Freeport (13 rooms, three decks) and Stickney Yards (12 rooms, two decks) room by room with themes, details, exits, items, the NPC cast with voices and fallbacks, lore pools and ambience; the Dray's six-room interior and its stowage in the same voice, its hold read in tonnes and cubic metres; the Yard's crated fittings and the Co-op's bid for them; how the brief becomes atlas packs (decks as floors, locks, wall features, the \`port.dock\` seed, the port entry's room references, the validator tests); the names accepted under Q-18-12 |`
  **As built:** done (IC12; see `docs/design/README.md`), the rows rewritten for Draft 0.4.
- **README "Superseded decisions".** The stale lead "Five choices" (seven are listed) becomes
  "Eight choices", and item 8 is added: **The sky will move, after the first IC slice.** Doc 10 §22 T17's
  frozen sky ("no body moves", `rate = 0` the ruling) and 1A's rejection of simulated ephemerides are
  superseded (owner, 2026-10-15, doc 18 R9). After the trade-run slice an astronomy engine (cut P-A)
  moves real three-dimensional bodies on the IC calendar, so route distances, durations, fuel and prices
  depend on when and where a ship sails. Until then the slice flies a **constant sky** through the same
  seams the engine will use — one sky query, one passage planner whose output every departure freezes
  (T1, load-bearing again) — and no slice figure is an invariant. 1A's travel scales and compression and
  "transfer conditions modify routes, never close destinations" stand. A row for D-14-35 as decided
  (Q-18-6, binding) is added beside it.
  **As built:** done with Draft 0.3 (`0ce0d0fa`: "Nine choices", items 8 and 9 in the README); IC12
  checked it and added nothing.
- **Doc 10 §22 T17.** The amendment of §4.8, verbatim: the heading gains "amended 2026-10-15: the
  constant sky of the first slice, superseded after it by the astronomy engine"; the 2026-09-09 text is
  kept; the amendment paragraph is appended.
  **As built:** done with Draft 0.3 (`10-tacsim-foundations.md:773`).
- **Doc 10 1A.** Honoured for its scales, compression and "modify routes, never close destinations"; its
  rejection of simulated ephemerides is superseded after the slice (R9); the Draft 0.1 three-day
  amendment stays withdrawn.
  **As built:** done with Draft 0.3 (the same amendment, `10-tacsim-foundations.md:794`).
- **Doc 14 §3.8 S7 / `tacsim/src/config.rs:76-82` / `mushd.toml.example:190-192` /
  `deploy/instance/mushd.toml.tmpl:218-220`.** The sky's epoch moves from server config to the pack's
  bodies (`BodyDef.epoch_ic`, D-18-48), as doc 10 T17 originally said; the knob is documented as "sky
  motion" and collapses to `constant | real` at P-A.
  **As built:** done — the code and config comments read "the constant sky" and the epoch is the
  bodies' (IC2, `484647bc`); doc 14 §3.8's paragraph is in at `2f268ee3` (IC14).
- **14b §3 / 14-spatial-implementation-plan.md:1321.** D-14-35 decided as two records with a hard
  boundary (Q-18-6, binding), with the wording that the drawn `spacecraft` connection is a **projection**
  of the ephemeris at the epoch that no verb traverses and no code reads as a travel path.
  **As built:** done — the rows under D-14-35 in `14-spatial-implementation-plan.md` §6 and 14b §3
  (`2f268ee3`), the projection wording completed in the 2026-10-18 docs pass.
- **Doc 15.** §3.5's lag table is already written in moving-sky ranges; a note that at P-A R5 reads
  `Sky` positions and a ship's frozen plan legs, and D-15-11 is honoured with the route's light distance
  taken from the plan.
  **As built:** done — the notes under §3.4 and §3.5 (`2f268ee3`).
- **Doc 11.** §2 (the 2320s row) and §5 (§1 never mentions Kirkwood): Kirkwood Depot's orbit is
  Q-18-36's — on Ceres's orbit a few degrees astern under the recommended default ("at the inner edge
  of the Belt" becomes "Ceres's outer harbour"), or an independent ~2.06 AU orbit that is on the lane
  once in decades — and the wording waits on the ruling; §7 the Dray's hold binds on 100 t and 120 m³
  and its drive is a tagged coast law whose season-dependence is Q-18-35 and whose laden law is
  doctrine (Q-18-41); §4.5 the Co-op buys crated yard fittings for its fleet and the Kirkwood tank farm;
  §4 Stickney Yards sells them.
  **As built:** still owed — the §2 and §5 wording waits on Q-18-36; the placeholder notes are in at
  `2f268ee3` (the 2320s row and §5 say the lane pack places Kirkwood on the Ceres–Mars chord at the
  epoch with a placeholder period), and §7, §4.5 and §4 are done under the doc 11 bullet below.
- **18a.** The hold, slate and Bridge readouts in tonnes and cubic metres; the departures board (C14)
  and the Dray lore line (§1.5) say "in a good season"; P08/P11, P15 and C08 carry the fittings; a sixth
  signatory (revised with this draft).
  **As built:** done (IC12; 18a revised with Draft 0.4 against the shipped packs).
- **Doc 06 §7.5.** A note that the boundary rule is built in IC4 as written (member rooms; staff not
  exempt), and that `InteriorSpaceRecord.rooms` has a writer.
  **As built:** done — the note under §7.5 (`2f268ee3`; the rule's code IC4, `3c9cded5`): built as
  written, staff not exempt; `InteriorSpaceRecord.rooms` has two writers, `Transition::Members` and
  boot's `drop_destroyed_member`.
- **10b §3.11.** The Draft 0.1 note on issue/retire is **withdrawn**; port listings draw from seeded,
  receipted pools (D-18-26). 10b:1753's declared-price lease buyout is built as `+coop/buyout` (IC13).
  **As built:** done — the issue/retire note withdrawn and the buyout row under §3.11 (`2f268ee3`);
  the buyout landed in IC13 (`94679739`).
- **08a.** Ledger rows for: Lot in `deliver()`'s short-circuit and the listing-TTL cap (IC0, landed);
  `with_party_cap_source`; `with_venue_gate` with `Presence::{Here, Ship}`; `Supply::Wanted`;
  `LotOp::Draw` and `LotOp::Deliver`; the `CapacityProvider` hook; `with_proceeds_policy`; the
  contract-keyed lot leg; the lapsed-hold release by the port tick as the holder (not the sweep;
  Q-18-31); `transfer_marked`; the `page_stale_contracts`
  lease exemption; and the options a keyed `create_system_obligation` variant and an optional
  `LeaseSpec.rent` (Q-18-27, Q-18-29).
  **As built:** done — the "As built (doc 18 IC0–IC14)" ledger under 08a's live seams and a Shipped
  row (`2f268ee3`; the Shipped row added in the 2026-10-18 docs pass), citing IC0
  `c87d78f7`, IC6 `3bf2bc90`, IC7 `d8427081`, IC8 `ee1d831f`, IC10 `7435f954`, IC13 `94679739`, IC14
  `2f268ee3`; the stale-page exemption as narrowed in IC7; the two options were not needed
  (`offer_lease` admits a zero rent; plain treasury tranches).
- **Doc 10 7A.** Nothing; finite capitalization stands (P-A's emergency bids draw on it too).
  **As built:** nothing, as written.
- **Doc 10 T1.** Applied in IC9 through the planner; its freeze is load-bearing again (the ticket
  freezes the whole passage plan); the "deferred, not denied" note of Draft 0.1 is closed.
  **As built:** done — applied in IC9 (`1137c373`) as written; the note under T1 at `2f268ee3`.
- **Doc 10 C2A–C4A / §18.1; doc 17 F3 row.** The note that `+ship/undock` bypasses the station-channel
  gate until P2 now cites R3 (an offline departure cannot wait on a live seat).
  **As built:** done — doc 10's C4 row and §18.1 note (`2f268ee3`); doc 17 §2's F3 row gains its
  note in the 2026-10-18 docs pass (code: IC9 `1137c373`; the title-or-charter gate IC13 `94679739`).
- **16c.** The hauler class filter and the custody-seam refusal test; the `@journey` "(staff)" help.
  **As built:** done — the in-code `@journey` help (IC12, `715151c4`: it names the true gate — staff, or
  the carrier's crew or controller — and that a lane route and a Co-op hull are flown only by
  `+ship/undock` or the sailing plan); the 16c ledger row is T-111 under §2.4 (IC14's paragraph at
  `2f268ee3`, made a row in the 2026-10-18 docs pass; code IC2 `484647bc`).
- **Doc 11.** §4.5 gains the First Captain programme and the Trust; §4.7's starter-charter line (Draft
  0.1) is not added; §4 gains a **Freeport Exchange** entry (the Port Council's commodity floor:
  consigns the Belt claims' metal concentrate and takes alloy intake as Freeport Works; its own factor
  as officer of record; the slice's largest capital), and §4.8 a note that Clearing's market-maker role
  is unused by the slice and that at the Freeport window it "opens accounts for captains and lends
  nothing to pilots" (never "lends to governments", which §4.8 does not say); §4.3 that Kasei's
  reaction-mass price is Republic-posted and its ice intake serves Phobos's volatiles demand, not the
  propellant line; §5 a note that
  Kirkwood is a seeded body whose orbit is Q-18-36's; §4.10 that Pallas has a body entry; §7 the
  Dray's scale (240 t dry, 100 t and 120 m³ hold) and the Class 0 ROM.
  **As built:** done — §4.5's programme and Trust, §4.8's note, §4.3, §5, §4.10 and §7 (`2f268ee3`);
  §4.7's line not added; the Freeport Exchange is §4.12, written in the 2026-10-18 docs pass (its org
  and capital IC3 `ea52634d`, its ask and the Works' bid IC8 `ee1d831f`).
- **Doc 14 §2.6.** Three starter packs and seventeen rooms; §3.3's mode filter true once `spacecraft`
  leaves `SELF_MODES`. **D-14-5 / 14a 5A** (carrier interiors get local frames) is not foreclosed by the
  interior template: the six rooms can take a local atlas frame later.
  **As built:** done — §2.6's and §3.3's notes (`2f268ee3`; the mode filter true since IC1 `07232d09`).
- **`mushd.toml.example` [onboarding]** and `mush-config/src/config.rs:394-401`: drop the WoD text.
  **As built:** done — `config.rs`'s onboarding comment lost its WoD job name at `38c47149` (16b cut
  R-b.3; it reads `pilot-chargen`); `mushd.toml.example`'s and `deploy/instance/mushd.toml.tmpl`'s
  `[onboarding]` text at `2f268ee3` (IC14). No code string changed.
- **In-code doc comments:** as Draft 0.1, plus `mush-market/tests/e7r2.rs:327-328` ("exactly
  extraction's": work-order delivery also mints), `mush-sim/src/custody.rs:212-220` (withdrawal leaves
  location at "the space object itself" is false under the member projection), `mush-sim/src/data.rs:832-835`
  ("reserved" no longer); and for R9 (IC2): `tacsim/src/commands.rs:435` ("the sky is frozen at the
  campaign epoch" → "constant sky at epoch <date> (T17)"), `mush-sim/src/route.rs:223-231` ("a frozen sky
  has no departure windows" → "a constant sky has no departure windows; the planner computes per
  departure"), `route.rs:347,559,777,842` ("frozen" → "constant"), `tacsim/src/ephemeris.rs:5` (the
  circular, coplanar model as the e = i = 0 case).
  **As built (IC12):** done — `mush-market/tests/ic0.rs` (the party-cap source as IC7/IC8 built it),
  `mush-market/tests/e7r2.rs` (the mint callers named: extraction, a work order's delivery, the port's
  stock clerk), `mush-sim/src/custody.rs` (withdrawal keeps the member-room projection),
  `tacsim/src/ephemeris.rs` (the circular, coplanar model as the e = i = 0 case); found already true at
  HEAD — `tacsim/src/commands.rs` (`@corridor/show` prints "constant sky at epoch <date> (T17)", IC2),
  `mush-sim/src/route.rs:223-231` (IC2's wording), `route.rs`'s remaining "frozen" (they mean T1's
  frozen route assumptions and a journey's frozen leg times, not the sky), `mush-sim/src/data.rs` (the
  member rooms are documented as written by IC4).
- **Help topics.** New: `ports.md`, `ships.md` (the hold's two limits; passage time by load and, later,
  season), `coop.md`, `orders.md`, `trade.md` (why you carry both; buy the bulky good first when cash
  is short). Amend as Draft 0.1
  listed (`getting-started`, `chargen`, `travel`, `atlas`, `spatial`, `cockpit`, `npc`, `intel`, `faction`,
  `chronicle`, `clocks`); in-code `@journey` "(staff)", `@agent` fields, `+market` price lines;
  `mush-market/tests/ic0.rs:19-21`, `mush-server/tests/e7r2_proving_loop.rs:35` and
  `18-cut-notes.md:109-110`, whose "cut IC5's `with_party_cap_source`" is Draft 0.2's IC7 (the IC0
  session owns the cut note; §5 carries the mapping until it is re-pointed).
  **As built (IC12):** done — the five new topics and every amendment listed (`getting-started`,
  `chargen`, `travel`, `atlas`, `spatial`, `cockpit`, `npc`, `intel` checked with nothing stale,
  `faction`, `chronicle`, `clocks`); in-code `@journey` (above), `@agent` (its field list and the
  one-sheet `/sheet` sentence; the code default `mortal` is dispositioned to the single-game cleanup, IC14's note), `+market` (`/accept`'s
  re-stage promise made true and `/depth`'s `<ruleset>:<code>` re-worded `<catalogue>:<code>`), and
  `SHIP_HELP`/`ORDER_USAGE` naming `+Nm`; `mush-market/tests/ic0.rs` re-pointed;
  `mush-server/tests/e7r2_proving_loop.rs` was already true; `18-cut-notes.md:109-110` is re-pointed by
  IC12's note at the end of its section (`18-cut-notes.md:5848-5857`, `715151c4`), IC0's lines left
  as written (the committer's file).
- **`docs/ops.md`.** The asker-authority bridge; the actor-swap executor posture (closed typed calls, one
  step per hull per fire, verify by pull) beside the typed-action posture at `:976-979`; the rate-24
  deploy line.
  **As built (IC12):** done — "The trade run (doc 18): the port tick, readiness and the play rate" (the
  executor posture, the asker-authority bridge, the status report and the gate, the moving-sky page,
  `@tacsim/port clear`, the rate-24 deploy line, the kept-world pack upgrade, the pre-deploy blocker on
  selling).

**Draft 0.1 review fold (2026-10-15, three lenses).** F-1..F-14, unchanged from Draft 0.1 and kept for
the record in its text at `8a32734a`; F-1, F-6 and F-8 are moot under the rulings.

**Draft 0.2 fold (2026-10-15; the owner's rulings R1–R6 and the second review).** By ruling: **R1**
replaced D-18-10 with D-18-21/25 and added IC13; **R2** replaced D-18-5/6/7 with D-18-21/24, deleted
`+credit/advance` and P1, and chose the Co-op through a programme Trust; **R3** replaced D-18-17 with
D-18-30, added the queue (D-18-31/32, IC11), the tick skeleton (D-18-33, IC5), the readiness gate and
the rate-24 deploy line, and re-justified D-18-23; **R4** replaced D-18-15 with D-18-26/27 and withdrew
the E7 pin amendment; **R5** replaced D-18-8 with D-18-22/29, kept D-18-18's verbs (D-18-34) and added
IC4; **R6** produced 18a and D-18-35 with the freight decks named as P-F. By review graft: the buy clamp
and `fill`, the Fact-gated payout, the receipt family, the Draw's source projection, the contract-keyed
lot leg, the lapsed-hold release on the port tick, the separate Trust, the `look` status line and
gauges, the watch-log framing, the NPC voices, the lane pack's fresh id (D-18-36). Not applied: the net
lay and 15-room stations (Q-18-13, Q-18-23); `Registered` hull endpoints.

**Draft 0.2 review fold (2026-10-15, three lenses: tree audit, rulings and fiction, tester and
buildability), applied in place by finding; the finding-by-finding record is Draft 0.2's text at
`1fc40d01` §10.** In short: *tree audit* A-1..A-8 (the passive-domain guard and IC3's skip; IC2 adds
the sky leg test rather than citing one; IC0 landed, cites re-pinned and the IC5→IC7 mapping; the
zero-rent check unverified; the 70 t return 17.75 h; three path and file cites corrected); *rulings and
fiction* F-1..F-13 (one fiction for the queue, the signed sailing plan; the price chain read as crude
fraction, volatiles demand and a Republic-posted propellant price; the Freeport Exchange defined with
its own factor; the commissary's signatory; `+ship/leave` in the airlock; the bumped-`issue` top-up
refused; IC time only in room text; margin not net; exit words rewritten to 18a's aliases; the airlock
forward; 8 of 8 and 0401 on a fresh world; a spun ring; `+credit` without "owed"); *tester and
buildability* B-1..B-7 (the lapsed-hold release moved to the port tick; IC5 listed under IC8/IC9; the
readiness gate growing per cut with `test_world` a composition flag; one crew-day rule; the lanes'
shared files named; IC11 sized L; `+coop/statement` readable anywhere). Not applied: re-pricing the
chain and editing `18-cut-notes.md`.

**Draft 0.3 fold (2026-10-15; the owner's second-round rulings R7–R9, two designs and their judges).**
By ruling: **R7** (volume beside mass) replaced D-18-28 with the capacity function over doc 06 D3's
measure map, re-authored the packed volumes so each leg has a whole-tonne vertex (D-18-39), added the
sixth good and the Stickney Yards vendor with a sixth signatory (D-18-40), rewrote `fill` and the clamp
over both limits (D-18-41), moved Kasei's metals bid to 280, resized D-18-27 and restated D-18-25's
timeline (day 15; Draft 0.2's own set reproduced as day 17), re-ran §2, §4.1, §4.2, §6 and 18a's hold
text, shrank P6 to the census index and struck "volume capacity" from the refusals; **R8** (every other
default accepted) made D-18-21..38 binding and closed Q-18-3..31 in one line each, with Q-18-6's
`spacecraft` removal promoted to a sky prerequisite in IC1; **R9** (the sky will move after the slice)
added §3-Q, §4.8 and D-18-42..51 — the `Sky` enum, `plan_passage` with the ticket freezing its whole
output (T1 load-bearing again), the coast rate as drive data under a tagged law, per-body epochs at a
real date, routes as a list, no passage bound as an invariant, belter-charts as inspiration only — the
T17 amendment and README item 8, the P-A brief in six sub-cuts and Q-18-35..40. By judge: the sky
design's derive-at-load of `base_s` and its 100 s filing quantum were scaled back to authored seconds
checked against the sky (the engine stops reading them; it deletes nothing); the dual-meaning
`distance_km` kept as the chord, relabelled; the burn prepare recomputes burn and crew-days from the
plan's seconds rather than trusting its input; a `PortPricing` trait dropped from the slice;
`sky_rev` as a `DefinitionDigest`; the 110 KB figure corrected to positions only (219 KB for state
vectors); the epoch supersession moved from doc 10 to doc 14 S7/config; the "no convenient date" risk
replaced by the 70-days-per-cycle constraint; "xtask" replaced by a committed script. The volume
design's alloys-bid edge (421, not 452), second cash-bound return (96 t = 45 + 51), the LP test's cut
(IC8, not IC2), the 181,000 ≥ 180,680 slip and "about 4,000 to spare" were corrected; goods rows became
an open measure map like the compartments; a Draw refused at prepare returns its escrow with no page;
the locker's two limits carry a consistency rule; `admits` is one enum. Not applied: a solver verb
(Q-18-34), the ice-bid alternative (Q-18-33), a Flip Dray in the slice (Q-18-35 is the owner's), and
editing `18-cut-notes.md`.

**Draft 0.3 review fold (2026-10-15, two lenses: tree and numbers; rulings and fiction), applied in
place by finding, each confirmed against the tree before it was folded.** *Blocking:* **Kirkwood's
orbit** — the "70 IC days per 3.18-yr cycle" was the Ceres–Mars two-body alignment; a via-Kirkwood
season is three-body and its offset walks 138° a cycle, so an independent 2.06 AU depot is on the lane
once in roughly 40–110 years: §3-Q (4) carries the arithmetic, the §4.8 regrets row and risk 18 the
consequence, Q-18-36 is re-presented with the lane-tied default (Kirkwood on Ceres's orbit a few
degrees astern, a ~9 % detour, the slice's leg split replaced at v2) and the honest costs, D-18-48's
orbit is provisional on it, IC2 authors Kirkwood at its epoch position with a placeholder period, and
the doc 11 cite is §2/§5 (§1 never names Kirkwood). *Should-fix:* **the frozen reference** — the one
slice decision P-A-4's price policy would have undone: `reference_major` becomes a `reference` map of
origin listings read live at settlement (§4.1, the port entry, D-18-24, D-18-49, §4.8, P-A-4, Q-18-39,
IC8), only the share permille and the buyout freeze at signing, the residual is `trade.md`'s;
**mean vs true longitude** — Ceres' equation of centre reaches 9° and its radius ±0.22 AU (±30 % on a
0.741 AU leg), so the lane pack authors true longitudes and heliocentric distances from the Horizons
script, P-A-1 keeps `orbit_au`/`phase_deg_at_epoch` exact at the epoch with `a_au` and the mean anomaly
as optional Kepler fields, P-A-6's golden test reads "identical at the epoch, drifting after it", and
lane v2 re-authors `base_s` (IC2's test retires with v1; D-18-48, §4.8 row 7, IC2); **the coast
law** — the laden multiplier and the flat 105‰ contradict each other under the rocket equation
(a fixed fraction is one Δv and one cruise; at 3,700 km/s exhaust a 115 km/s cruise burns ~6 %), so
`kms` is stated as the unladen cruise, the laden law and the 105‰ as doctrine, P-A-2's `exhaust_kms`
as reproducing 105‰ unladen only, and the cruise-vs-burn choice is **Q-18-41** (§4.4, the regrets row,
D-18-45, risk 18); **the Dray `note`** — 1,316 bytes against `MAX_NOTE_BYTES` 1,024, refused by
`UnitSpec::check` at load: cut to 991 bytes, the rationale in §4.2's prose, and IC2's pin decodes
`dray.json` through `UnitSpec::check`; **Stickney's geometry** — the Yard Concourse touched neither
the harbourmaster nor the observation gallery it opens onto (`geometry.portal_gap` is an error and the
applier refuses the exit, aborting a first-boot apply) and five exitless seams were unwalled (Ceres
C08|C10; Stickney P03|P05, P03|P08, P09|P11, P11|P12): 18a §0.4 re-boxes Stickney's middle and bottom
rows so P04 [125,130]–[185,155] shares an edge with P05, P08, P11 and P12, the wall lists in 18a §0.4
and §4.5 are complete (Ceres six, Stickney seven), and IC1's validate-clean test carries an adjacency
check over the authored boxes; **the hold's hard-coded 20/80** — 18a's Cargo Hold detail stored the
vertex D-18-39 forbids, so the load cells, the passage slate and the Bridge HOLD display are
`look_rendered` gauge lines from `interior.gauges` (now a pool → rooms map in §4.2's JSON) and no pack
text carries a load; **painted prices** — 18a's C08 chit board, P07 intake board, P08 boards and P11
fittings card carried 560/280/200/400 beside LookRendered items, stale the first time P-A-4 or a
re-price moves a line: the numerals are struck from details and two NPC lines, the boards carry the
figures, and C08 and P11 gain LookRendered items; **Q-18-13 presented as live** in the header, §4.1
and D-18-24 — closed under R8 in all three ("Rested on Q-18-13; binding"). *Nits:* 144 objects per
eight hulls (18 each: the applier opens `forward` and `back` as two exit objects), not ~115; the
gross-share example re-run on Draft 0.3's figures (250‰ of 21,600 is 5,400 against a 5,390 margin);
`Body.epoch_ic` and `phase_mdeg_at` advancing from the body's own epoch added to IC2 with a unit test,
since a wire-only `BodyDef.epoch_ic` would leave IC9's rate-1000 test unwritable; 18a's "proposal"
framing of names and numbers and doc 18's two residual lines made binding under R8 (18a §4 retitled
"Names (Q-18-12, accepted 2026-10-15)"); the one-house-one-signatory rule attributed to this slice
under the Warrant's principle (11 §2 says only that every automated act has a liable principal) and
P-A-4's writes given each org's officer of record as signatory; Q-18-40's keep-out made a detour cost
with a validator rule keeping a non-refusable route per destination, because the kept 1A rule forbids
closing a destination and `choose_route` picks only from the list. Skipped: nothing; every finding
reproduced. Open questions are now Q-18-32 to Q-18-41.
