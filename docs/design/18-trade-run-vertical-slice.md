# 18 — The trade run: the first IC vertical slice

**Draft 0.2 — 2026-10-15**, revised in place at `main` `c87d78f7` (IC0 landed; its parent `8a32734a`
is Draft 0.1's commit) from Draft 0.1 (written at `f4b14f3b`; only this document and the README changed
between `f4b14f3b` and `8a32734a`, so every A–I file:line in §3 holds at `8a32734a`; IC0 then moved six
source files — `mush-market` `trade.rs` +28, `lot.rs` +78, `commands.rs` +132, `module.rs` +34,
`api.rs` +16 and `mush-sim` `commands.rs` +22 — and the cites into those files are re-pinned at
`c87d78f7` where this draft uses them, or marked "8a32734a" where it does not).
Draft 0.1's nine-beat map (§3 A–I), three designs and three-lens review stand; this draft adds a
seven-beat map of the new ground (§3 J–P: the six-room interior, the captain's queue, the backer, finite
stock, mass and volume, the two stations, and an audit of the owner's pasted proposal), three revised
designs and a second three-lens review (daily loop as play, fidelity, buildability), and folds the
owner's rulings of 2026-10-15 (R1–R6, §1) over Draft 0.1's defaults. Decisions are §7, D-18-1 to
D-18-20 re-stated with the replaced ones marked, plus D-18-21 to D-18-38; questions are §8, with the
answered ones struck and the new ones added. The companion content brief is
[18a-trade-run-stations-and-ship.md](18a-trade-run-stations-and-ship.md) (the two stations and the
Dray's interior, room by room). **IC0 has landed** as `c87d78f7`, HEAD of `main`; its record is
[18-cut-notes.md](18-cut-notes.md), written by the IC0 session, which never writes this document.

**What the owner ruled (2026-10-15).** Start on IC0 while the details settle. **R1** The pilot charters;
a required purchase is refused; owning a hull is an aspiration "a couple of weeks of good luck away", so
a buyout exists in the design and the start does not make it trivial. **R2** No one-time advance and no
revolving line: a corporation invests in new pilots up front on a profit-sharing basis because it wants
pilots who can manage runs for it (the default lay of §4.1 shares **trading margin** over posted
reference prices, not net profit — fuel, stores and port costs stay the pilot's — which departs from the
word "profit"; Q-18-13 asks whether the owner wants the net). **R3** A passage under a day and around a day; travel is primarily
offline; a pilot who logs in 30–60 minutes at about the same hour each day runs a productive career by
queuing orders for the time they are away (departure times, loading, the captain's minutiae) and logging
off. **R4** Seed an initial finite world stock; nothing from thin air. **R5** A five- or six-room ship —
an airlock on a central passage with the bridge, a crew area and the cargo hold off it — because the
pilot may be aboard for long stretches. **R6** Flesh out the two stations and the flavour of each. The
ports (Q-18-1) stand. Where a ruling overrides a Draft 0.1 default the text says so and §7 marks the
decision "replaced by R*n*".

Reading order for someone new: §0 (the run in one paragraph), §2 (two days, beat by beat), §4.1–§4.4
(the backer, the ship, the goods, the captain's day), §5 (the cuts), §8 (the questions).

**The slice in one paragraph.** A new account connects, makes a pilot, and walks out of Limbo through
the `Ceres Freeport <C>` door into Port Watch Arrivals. At the Registry & Co-op desk they sign the Belt
Carriers' Cooperative's First Captain programme: the Co-op charters them a Co-op Dray, the *Salt Moth*,
stakes them 25,000 cr of working cash, and takes a quarter of the trading margin on every sale until the
pilot buys the hull out at a price fixed the day they sign. At the Exchange Hall they buy 70 t of metal
concentrate and 30 t of water ice into the Moth's hold; at the chandlery, 42 t of reaction mass and 21
crew-days of provisions. They board through the Visiting Berth Gallery, walk the six rooms, check the
plan on the Bridge (payload, burn, ETA about twenty hours), queue the day's orders — depart now; at
Stickney sell everything, fill the tank and the locker, buy alloys with what is left, and leave again at
19:30 tomorrow — and log off in a bunk. The ship flies while they sleep: the burn settles, the Moth
casts off, passes Kirkwood, docks itself at Stickney Yards, and the queue sells to Kasei Metals, buys,
and waits for its hour. The next evening the pilot logs in docked at Stickney, reads what the queue did
and what the Co-op took, walks the Yard, re-files the Ceres orders and logs off again. Two weeks of good
runs buys the hull. Every act is the player's own verb, or the queue executing the pilot's own signed
orders; no staff command and no NPC stands on the path.

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
That flow is the corridor's reason to exist and the pair the T17 epoch was chosen for (10 §22). The
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

Verbs whose status is **new** do not exist today; their names are proposals (Q-18-12). Evidence is
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
| 5 | Check the ship | `+ship` | "*Salt Moth* (Dray): docked Ceres Freeport, Berth Gallery. Hold: cargo 0/100 t · tank 0/80 t · locker 0/120 crew-days. Programme: 0 sales; buyout 90,000.00cr." | Reads the enrolment, the live lease, `pose_of`/JourneyBook, the hold pools through `scan_action_state` | new | `mush-sim/src/window.rs:362`; `mush-core/src/module.rs:561,573` |
| 6 | Buy cargo | `arrivals` · `concourse` · `exchange` · `+market/list` · `+market/quote 4=70` · `+market/accept 11` · `+market/quote 5=30` · `+market/accept 12` | "Metal concentrate 200.00cr/t, 8,500 t · Water ice 100.00cr/t, 3,600 t · WANTED structural alloy 520.00cr/t"; "70 t of metal concentrate are loaded into the *Salt Moth*'s hold. Rosalind Qureshi-Mbeki countersigns." Hold: cargo 100/100 t | The lot path with the port seams: `market=` and `to=` default to this venue and the berthed hold (VenueGate `Presence::Here`); the ask is `Supply::Stock + Representation::Lot` whose matter leg is `LotOp::Draw`, a pool-to-pool `transfer` staged at activation, with the listing quantity as the synchronous availability counter (no reservation, no hold); the vendor org accepts through the content-resolved party-cap source; capacity pre-checked synchronously and again in the Draw prepare | partial | `mush-market/src/trade.rs:573` (`booked_loss` pre-check), `:701-716` (the lot leg staged at activation, `LotOp::Consume` at `:706`), at `c87d78f7`; `mush-inventory/src/pool.rs:413-528`; `mush-economy/src/contract_live.rs:5151-5175`; `tacsim/src/composition/mod.rs:361` |
| 7 | Fuel and stores | `chandlery` · `+market/quote 6=42` · `+market/accept 13` · `+market/quote 7=21` · `+market/accept 14` · `+ship/hold` | "42 t of reaction mass are bunkered." "21 crew-days into the stores locker." Hold: cargo 100/100 t · tank 42/80 t · locker 21/120. `+credit`: available 6,110.00cr | Same path; the goods table's stowage tag routes each good to its pool (`stow.bulk`, `stow.propellant`, `stow.provisions`); ice into the tank is refused | partial | `crates/tacsim/content/goods.json` (new, §4.3) |
| 8 | Board and walk the ship | `exchange` · `concourse` · `arrivals` · `lobby` · `gallery` · `+ship/board` · `in` · `up` · `look` · `down` · `crew` | "You cycle the lock into the *Salt Moth*." Airlock; Central Passage; Bridge (course, tank and hold readouts, the status line "Aboard the *Salt Moth*, docked at Ceres Freeport"); Crew Compartment | `+ship/board` in the berth gallery places the lease tenant `Interior{airlock space, berths}`; the member-room boundary rule lets an Interior actor take an exit whose destination is another member of its own space, with no placement write per step; projection keeps the member location across a restart | new | `mush-sim/src/window.rs:324-341`; `lib.rs:293-303`; `graph.rs:167`; `data.rs:832-835` |
| 9 | Plan | `+ship/plan stickney` | "Payload 142.2 t of 181 t. Burn 41 t (tank 42 t; 1 t left). Stores 21 crew-days. Passage 19.6 IC days ≈ 19 h 36 m real; arrives about 15:06 UTC tomorrow. Nothing was written." | Pure functions over the hold pools: `route_leg_durations` on the Dray's drive at this payload, `burn_t`, `crew_days`; the `@journey/plan` write-nothing precedent | new | `mush-sim/src/route.rs:794-836`; `mush-sim/src/commands.rs:2605-2614` (`c87d78f7`) |
| 10 | Queue the orders | `+ship/order depart stickney at=now` · `+ship/order sell metals all min=250` · `+ship/order sell volatiles all min=150` · `+ship/order buy propellant fill max=60` · `+ship/order buy provisions fill` · `+ship/order buy structural_alloys 100 max=400` · `+ship/order depart ceres at=daily 19:30` · `+ship/orders` | Seven numbered orders with states and due times in UTC, IC and real time; "Order 1 fires on the next tick." | The `tacsim:orders:<hull>` document, written in the command's batch; a closed vocabulary (buy, sell, depart, stop); fuel and stores are queued **before** cargo so the executor's clamp can reserve the next departure's needs (§4.4) | new | §4.4; `mush-propagation/src/commands.rs:1252-1279` (`+orders` is taken, hence `+ship/order`) |
| 11 | Log off aboard | `QUIT` from the Crew Compartment | (in every room, a moment later) "The *Salt Moth*'s lock seals. Berth clamps release; the drive lights." | The disconnected body stays placed aboard (only seat focus lapses, and there is no seat); the tacsim tick's order arm runs the depart step **as the pilot**: lease re-check, undock checks, then one authoritative `tacsim.port.burn` keyed `tacsim.port.burn/<hull>/1/1` (debits the propellant and provisions pools, writes the ticket with payload, `leg_s`, burn and crew-days); the next fire launches it through `pub mush_sim::depart` with the frozen `leg_s`; narration loops over `space.rooms` | new | `mush-sim/src/seat.rs:736-770`; `mush-npc/src/module.rs:1496-1502` (actor swap); `mush-sim/src/journey.rs:11-35,398-408` |

**Day 1/2 — offline, about twenty hours.** The `CarrierJourney` advances on the world IC clock; at the
Kirkwood boundary every member room hears "Kirkwood Depot slides past: the Co-op's tank farm, a Republic
picket's transponder asking for your manifest. Leg 2 of 2." and a notice is queued (`journey.rs:1088-1172`;
today's narration reaches no one aboard, `journey.rs:1396-1409`, partial). Nothing can cross the hull's
boundary: the carrier seal refuses placements, the boundary rule refuses every non-member destination
(the global `o` and `s` exits included), and `+ship/leave` refuses under way.

**Day 2 — the turnaround runs offline at about 15:06 UTC; the pilot logs in at 19:00.**

| # | Step | The player types | The player sees | Mechanism | Status | Evidence |
|---|---|---|---|---|---|---|
| 12 | Arrival and docking | (nothing) | (as a notice) "15:06 UTC *Salt Moth* docked at Stickney Yards, Visiting Berth Gallery." | `apply_arrival` places the hull Direct on Stickney's passive domain, presented into P01; docking is automatic (D-18-18 kept): every hull at a port shares the gallery and is listed there | partial | `mush-sim/src/commands.rs:2207-2256`; `journey.rs:1340-1420`; `window.rs:195` |
| 13 | The queue sells | (nothing) | (notices) "Sold 70 t metal concentrate to Kasei Metals at 300.00cr/t: 21,000.00cr; Co-op share 1,750.00cr; you received 19,250.00cr. Jonas Achterberg-Nkemelu countersigns." "Sold 30 t water ice at 200.00cr/t: 6,000.00cr; Co-op share 750.00cr; you received 5,250.00cr." | The order arm, as the pilot under VenueGate `Presence::Ship(hull)`: the `+market/sell` lot arm against Kasei's `Supply::Wanted` bid; the buyer org's consideration escrowed by `accept_contract_as`; payouts `Exact(share)` to the Trust and `Exact(rest)` to the pilot from the `ProceedsPolicy`, gated on the inventory `transferred` Fact so money never moves before the matter; `LotOp::Deliver` = `transfer(hold pool → Kasei pool)`; one step per fire, verified by pull before it is marked done | new | `mush-economy/src/contract.rs:1886-1909,1418-1458`; `mush-inventory/src/facts.rs:44-82`; `mush-market/src/trade.rs:1023-1036` (one payout today) |
| 14 | The queue buys | (nothing) | (notices) "Bought 40 t reaction mass at 60.00cr/t (2,400.00cr): tank 41/80 t." "Bought 19 crew-days (190.00cr)." "Bought 70 t structural alloy stock at 400.00cr/t (28,000.00cr); short of 100 t: funds." "Departure for Ceres Freeport set for 19:30 UTC." | `fill` tops the tank to the full-hold burn of the next depart's route (41 t) and the locker to that route's crew-days (the return at 70 t is 8 + 11 = 19 by the per-leg rule, §4.2); the cargo buy is clamped to min(qty, listing, capacity, funds) **after** the next departure's needs are met; `quote` then `accept_into` as the pilot, clamped before quoting because `quote` refuses a quantity above the listing | new | `mush-market/src/trade.rs:246` (`quantity > listing.quantity`), `:515` (`accept_into`), at `c87d78f7` |
| 15 | Log in | (connect) | "While you were off watch:" then the lines above, once; "Aboard the *Salt Moth*, docked at Stickney Yards; departs 19:30 UTC (in 31 m)." | tacsim reconnect notices on the housing pattern, rendered once on `Connected`; the look status line on the atlas `look_rendered` precedent | new | `mush-housing/src/notice.rs:1-20`; `mush-atlas/src/travel.rs:2673-2695` |
| 16 | Ashore at the Yard | `passage` · `airlock` · `+ship/leave` · `lobby` · `customs` · `concourse` · `+coop/statement` · `observation` | The gallery; Republic forms; the statement (readable anywhere, §4.1): 2 sales, margin 10,000.00, shares 2,500.00, cash 20.00 after the alloys, buyout progress; the Guard's pad under Mars | `+ship/leave` in the airlock, docked: `unplace(keep)` + `move_to(gallery)`; `tacsim.last_port` stamped for the IC-return rule; the statement reads the enrolment and share records | new | `mush-sim/src/window.rs:346-358`; `tacsim/src/match_/cockpit.rs:2401-2418` put_out shape |
| 17 | Re-file and log off | `concourse` · `customs` · `lobby` · `gallery` · `+ship/board` · `+ship/order sell structural_alloys all min=480` · `+ship/order buy propellant fill max=40` · `+ship/order buy provisions fill` · `+ship/order buy metals 70 max=200` · `+ship/order buy volatiles 30 max=100` · `+ship/order depart stickney at=now` · `QUIT` | The Ceres orders queued behind the pending 19:30 depart; at 19:30 the lock seals and the return leg (17 h 45 m at 70 t) lands about 13:15 UTC the next day | The same queue; `rev` bumps on every edit and the inflight step is untouchable | new | §4.4 |
| 18 | Day 3 and after | (nothing) · (connect at 19:00) | "Sold 70 t structural alloy stock to Freeport Works at 520.00cr/t: 36,400.00cr; Co-op share 2,100.00cr; you received 34,300.00cr." Then the outbound bought; cash 15,630.00cr; the cycle repeats one leg per day | Freeport Works' bid at Ceres is the return leg's buyer; from here a 30–60 minute session a day is the career | new | — |
| 19 | Week two: buyout | `registry` · `+coop/buyout` | "The Registry writes your name over the Co-op's mark. The *Salt Moth* is yours: 90,000.00cr paid; the charter ends; no further share." | Pilot-issued asset contract (`TitleHeld` + `TitleTransfer`, the market's shape); the Trust accepts by party cap; `end_lease` before settlement so invariant 12 holds; the observer marks the enrolment graduated | new (IC13) | `mush-market/src/trade.rs:829-848`; `mush-economy/src/contract_live.rs:4710`; `engine.rs:1121-1125` |

**Money check.** Stake 25,000 − 14,000 (70 t at 200) − 3,000 (30 t at 100) − 1,680 (42 t at 40) − 210
(21 crew-days at 10) = 6,110. Stickney: + 19,250 + 5,250 (27,000 gross less a 2,500 share on a 10,000
margin) − 2,400 (40 t at 60) − 190 (19 crew-days) − 28,000 (70 t at 400; the clamp's funds term is
28,020, so 70 t) = 20.00, with the clamp having reserved fuel and stores first. Ceres: + 34,300 (36,400
less a 2,100 share on an 8,400 margin) − 1,480 (37 t at 40: the return burned 37 of 41, so `fill` tops 4
to 41) − 210 − 14,000 − 3,000 = 15,630. A full outbound leg nets the pilot about 5,600 cr in cash and a
full return about 6,400;
the first return is funds-limited to 70 t. At one leg a day the 90,000 buyout plus a 20,000 float lands
on day 14–15 at full loads and about day 22 at 70 % loads (§4.1). Fuel is a decision: a Ceres top-up to
80 t costs burn (105 kg per tonne carried) and still cannot cover both legs with a full hold (out 45 t,
back 37 t), so Stickney's 60 cr/t reaction mass is bought every return.

---

## 3. What the tree has, beat by beat

Condensed from the verified map. Each beat lists what exists, what is partial, what is missing, and the
corrections the verifiers made to the first reading, so the owner can see where earlier docs or surveys
were wrong. "Code wins" throughout.

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

**Two engine facts fixed in IC6.** `page_stale_contracts` pages every Active contract with no deadline
and no condition progress after 7 real days, with no lease exemption (`sweep.rs:1597-1640`;
`STALE_CONTRACT_AGE_SECS`, `contract.rs:109`): every charter would raise a level-2 page mid-programme, and
recovery and housing leases share the defect. IC6 exempts lease-shaped contracts (Collateral by the
issuer plus an Obligation), S. And `offer_lease` sets one `Attestation{Staff}` condition
(`contract_live.rs:4504-4510`), which is how staff end a charter; the observer above handles it.

**The stake.** 25,000 cr, an investment: no obligation, no principal, no repayment. It is the Trust's
capital at risk and cannot be clawed back (an org has no debit authority on a pilot's unscoped account,
`api.rs:370-394`); alt-farming is bounded by one enrolment per gateway account ever and by finite
capital. No top-up in the slice (Q-18-14's alternative).

**The share (D-18-24): 250‰ of trading margin over the Co-op's posted reference prices, paid inside the
sale.** share = 250 × max(0, (price − reference[good]) × qty) / 1000, with the reference per good in the
programme block (metals 200, volatiles 100, structural_alloys 400: the Ceres and Stickney asks). It is
computed at contract build by a tacsim `ProceedsPolicy` as an extra `Exact` payout to the Trust's account
beside the pilot's `Exact(offer − share)`: the engine already accepts a non-party payee and exact splits
that must sum to the escrow (`contract.rs:1886-1909,1418-1458`) and settles them atomically
(`:2856-2873`); only the seam is new, because every sell path hard-codes one payout today
(`trade.rs:1023-1036,877-892`). The policy fires when the sale's **source endpoint** is an enrolled hold,
never from command arguments, so a queued sale (R3) pays exactly as a typed one. Why margin over
reference and not gross or net: gross bills a losing run (250‰ of a 27,000 gross is 6,750 against a
5,600 cash leg); net needs an M-size run book because lots carry no price and quotes no destination
(`mush-inventory/src/record.rs:222-252`; `mush-market/src/record.rs:771-815`); the reference margin is
deterministic, computable from the exchange board before the pilot logs off, never bills a sale below the
posted origin price, and leaves fuel and stores to the pilot. It can bill a run whose fuel ate the
margin, which is the honest cost — and it is a **margin share, not the "profit-sharing" of R2's
words**; §1 says so where the ruling is restated, and Q-18-13 offers net with a carried deficit once
the run book exists. Every sale line prints gross, share and net.

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

**Losses, dormancy and release.** A sale at or under reference pays the Trust nothing; the pilot's cash
bears operating costs. Dormancy: seven real days without a departure, judged by the tacsim tick; the
lease ends when the hull is docked at Ceres with an empty cargo pool, otherwise the queue is held and
staff paged (no confiscation of matter in the slice). Staff `@coop/revoke <pilot>=<why>` is attributed;
`+coop/release` at Ceres is voluntary and keeps the stake. All three land in IC13; before it a dormant
charter simply stays. Re-enrolment is refused for ever by default (Q-18-14's 30-day cooldown is the
alternative). Hull damage is out of scope; Tycho underwrites Trust hulls later (11 §4.7).

**What refuses when broke.** `+market/accept` with insufficient available funds answers the escrow
refusal; a queued buy clamps to funds and logs the shortfall; a queued depart short of fuel or stores
holds with nothing burned; `+coop/enrol` twice is refused; nothing is minted on demand.

**Numbers.** Per the §2 money check: a full outbound leg nets the pilot about 5,600 cr cash (27,000 −
17,000 cargo − 1,680 fuel − 210 stores − 2,500 share), a full return about 6,400 (52,000 − 40,000 − 2,400
− 190 − 3,000); the first return is funds-limited to 70 t. Simulated at one leg a day with a 20,000 float
kept at Ceres: full loads reach 90,000 on day 14–15 ("a couple of weeks of good luck"); 70 % loads on day
21–24; the Trust receives about 43,000 in shares plus the 90,000 sale against its 25,000 stake and the
hull. At 75,000 the buyout lands on day 12–13, close to trivial; at 60,000 on day 9. Programme capital
8 × 25,000 + a 50,000 floor = 250,000 cr.

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

**Compartments.** Occupancy is the mush-sim `berths` slot, `maximum_items 2`, enforced at placement; in
the slice only the lease tenant may board, so the second berth is reserved for a passenger (Q-18-17).
Matter is the one hold vault with every quantity in **pools**: cargo (Σ `stow.bulk` goods ≤ 100 t),
propellant (≤ 80 t), provisions (≤ 120 crew-days), from `logistics.stowage`, mass only (D-18-28). Every
delivery into a hull endpoint is a pool credit — `LotOp::Draw` is a `transfer` (`pool.rs:413-528`) — so
no loose-lot census is needed, the burn debits pools directly (`pool.rs:306`), and the in-transaction
load figure is three point lookups. The check runs twice: synchronously in the market lot leg's
pre-check through `Ctx::action_state` (`trade.rs:572`) and as a backstop inside the Draw prepare. The
venue gate refuses any non-port listing that would deliver into a hull endpoint, so no lot ever lands
there. What the pilot sees of the compartments is in each room's `look` (18a §3): the load cells in the
Cargo Hold, the tank gauges in Engineering Access, the locker tally in the Crew Compartment, all read
from the same pools, plus `+ship/hold`.

**Propellant and provisions as consumables (D-18-11 kept, extended).** Both are pools in the hold, bought
like cargo and spent by the burn. The burn is settled at undock, matter first, by ONE authoritative
`tacsim.port.burn` action submitted as the pilot (by `+ship/undock` live, or by the queue's depart step
offline): it recomputes payload from transaction quantities, refuses `Overloaded`, tank < burn and locker
< crew-days, debits the two pools, and writes the typed ticket `{hull, route, route_pack, definition
digest, manifest{cargo, tank, locker, occupants}, payload_kg, leg_s, burn_t, crew_days, order, attempt,
burned_at, launched: false}` in the same bundle. The launch is the tacsim tick's arm consuming the ticket
through `pub mush_sim::depart(ctx, hull, route, leg_s)`; never an `ActionStateChanged` persist
(`engine.rs:2939-2942`), never a GOD-tick submission (`tick.rs:28-37`); boot reconcile launches
stragglers once. A refused departure consumes nothing. **Crew-days burned = occupants × Σ over the
route's legs of ceil(laden leg IC days)** — the one rule, used by `+ship/plan`, the burn and the
queue's `fill`: the full outbound is 12 + 9 = 21 (not ceil(19.6) = 20), the 70 t return 8 + 11 = 19.

**The helm.** There is no station (D-18-23). `+ship/plan`, `+ship/undock` and the queue are usable from
anywhere aboard; the Bridge carries the readouts and the status line. P2's helm station, `request_seat
_activation` and the `movement` channel return when a second crew member can hold the helm; under R3 a
seat lease that lapses 300 s after a disconnect (`seat.rs:736-770`) could never be the departure gate.

**The Dray definition (D-18-29).** `crates/tacsim/content/units/dray.json`, in `fielded_sources` with a
digest pin. Everything below is valid against today's schema except `class: "hauler"`, `logistics` and
`interior`, added as optional fields with `default` + `skip_serializing_if` so no shipped `DefinitionRef`
moves (`unit.rs:822-847`). The pasted scale replaces Draft 0.1's 2,800 t hold.

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
            "cargo":      { "accepts": [ "stow.bulk" ],       "limit": 100, "unit": "t" },
            "propellant": { "accepts": [ "stow.propellant" ], "limit": 80,  "unit": "t" },
            "provisions": { "accepts": [ "stow.provisions" ], "limit": 120, "unit": "crew_day" }
          },
          "berths": 2,
          "occupant_allowance_kg": 100,
          "payload_ceiling_kg": 181000,
          "drive": { "class": "coast_hauler", "thrust_permille": 1000, "law": "coast", "burn_permille": 105 }
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
          "gauges": { "cargo": "hold", "propellant": "engineering", "provisions": "crew" }
        },
        "note": "Stickney-built hauler (doc 11 section 7): slow, efficient, unarmed, Class 0 (no core component, so the drive ROM of doc 17 section 9; nothing separately titled). The port verbs read logistics and interior only; the combat anatomy is authored for later and never fielded (class hauler filtered out of SIM UNIT; tracked max_speed 0 is the immobile stand-in). Room descs live in the pack beside the names (18a section 3); the airlock is FORWARD of the passage and the crew compartment AFT, and this JSON is the authority 18a section 3.1 matches alias for alias. burn_t = ceil((dry_mass_t + payload_t) * burn_permille / 1000). payload_ceiling_kg is both route_duration's Overloaded rail and its laden denominator."
      }
    }
  }
}
```

**Numbers.** Dry 240 t; cargo 100 t; tank 80 t; locker 120 crew-days at 4 kg each; two berths at 100 kg;
payload ceiling 181,000 kg, a rail equal to the stowage maxima plus occupants (180.68 t) that never refuses
on its own; `coast_hauler` at thrust 1000 with burn 105‰ of departure mass. Full outbound (100 t cargo,
42 t propellant, 21 crew-days, one person): payload 142,184 kg, burn 41 t, 19.6 h at rate 24. Return
at 70 t (41 t aboard, 19 crew-days): 111,184 kg, burn 37 t, 17.75 h (17 h 45 m: 950,000 × 292,184 /
181,000 = 1,533,560 IC s). Per hull: 7 Rooms, 5 exit pairs, 2 space records; eight hulls
make about 115 objects, far under `MAX_CARRIER_SUBTREE` 256 per hull.

**Schema additions.** (1) `UnitClass::Hauler`, excluded from arena class caps; `fieldable_ids` filters
it (`lobby.rs:1600-1619`) with a custody-seam refusal test. (2) `UnitSpec.logistics` and
`UnitSpec.interior` (optional), read by tacsim; `bind()` ignores both. (3) `tacsim.registry` title issuer.
(4) In mush-sim: `Transition::Members`, `member_of`, the gate and projection arms, `pub
declare_compartment` (the `@spatial/slot` write without the parser, `commands.rs:1029-1068`). Not added:
a thrust mobility mode, `ComponentRole` drive/hold/tank (Q-18-5's alternative), stations (P2), a
`BerthLink` (P-G).

### 4.3 Goods

**The resource set (D-18-13 amended).** Five sale items, four from the ten families and one aggregate.
Units are tonnes for bulk goods and propellant and whole crew-days for provisions (D-18-28): every listing
and quote is capped at `MAX_QUANTITY` 100,000 units (`mush-market/src/record.rs:90-91`), which is
100,000 t, so the cap is untouched and every pasted price is exact in minor units per tonne.

| Good (inventory id) | Family | Unit | Packed | Stowage | Role | Ceres | Stickney |
|---|---|---|---|---|---|---|---|
| `metals` metal concentrate | metals | t | 1,000 kg; 0.5 m³/t (display) | `stow.bulk` | Ceres export (doc 10 §11), Kasei's refinery input | **Freeport Exchange** ask 200.00cr/t | **Kasei** bid 300.00cr/t |
| `volatiles` water ice | volatiles | t | 1,000 kg; 1 m³/t (display) | `stow.bulk` | Occator water to Phobos (11 §3.2) | **Occator** ask 100.00cr/t | **Kasei** bid 200.00cr/t |
| `structural_alloys` structural alloy stock | structural_alloys | t | 1,000 kg; 0.5 m³/t (display) | `stow.bulk` | Kasei's output (11 §4.3), the return cargo | **Freeport Works** bid 520.00cr/t | **Kasei** ask 400.00cr/t |
| `propellant` reaction mass | propellant | t | 1,000 kg | `stow.propellant` (tank only) | burned at departure | **Occator** ask 40.00cr/t | **Kasei** ask 60.00cr/t |
| `provisions` voyage provisions | none (class `ship_supply`) | crew_day | 4 kg; 10 L | `stow.provisions` | `occupants × ceil(days)` burned per leg | **Co-op chandlery** ask 10.00cr | **Yard Commissary** ask 10.00cr |

Provisions stay an opaque inventory commodity with no instrument; `FAMILIES` stays ten
(`tacsim/src/definitions/resources.rs:34-45,94-104`); Q-18-3 stands. **The price chain, read
honestly.** At Ceres reaction mass (40) is cheaper than water ice (100): Occator's reaction mass is
the bulk, crude fraction of the same extraction and the refined water is the product Phobos pays for;
18a's lore line says so, never that reaction mass "costs more than the water". At Stickney Kasei bids
200 for ice and asks 60 for reaction mass, which no refining chain explains (electrolysis conserves
mass): the ice bid is **Phobos's demand for volatiles** — the refinery's process water, the habitat and
the Republic's reserve (doc 11 §3.2 wants the volatiles; "Phobos needs volatiles", §1) — and Kasei's
reaction-mass ask is a **Republic-posted price** (the "Republic prices" of 11 §4.3's refinery, a
strategic asset the Republic keeps fuelled for the Guard and the Yard), dearer than Occator's crude
and unrelated to the ice bid. Kasei's capital (600,000) carries the difference; the economist says
the Stickney prices, not the chain. The numbers 40/60/100/200 are unchanged, so §2's money check and
§4.1's buyout timeline stand (the alternative, re-pricing so Occator's reaction mass sits above its
water and Kasei's ask above its bid, re-runs both and is not taken).

**The goods table.** All five live in one tacsim-owned compiled-in document,
`crates/tacsim/content/market/goods.json` (`deny_unknown_fields`, version 1, a **new** digest pin —
`resources.json` is only decode-tested, `tests/content.rs:313-322`): id, label, family or null, class,
ledger unit, quote unit, `mass_kg_per_unit`, `volume_l_per_unit` (display only in the slice), stowage tag,
consumable role, a reference baseline. It is the one source for the market `CatalogEntry` rows (derived,
representation lot, loaded by the port applier through `api::load_pack`) and for the capacity check.
Physics never goes into `CommodityDef` (label/in_use, in the test-only `tacsim.core` fixture, dropped by
`Catalog::build`) or into the market catalog wire, which silently drops unknown fields
(`record.rs:328-348`).

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

**Vendors and buyers.** One vendor per org (`store.rs:334-336`); a vendor may hold asks and bids.
Ceres (venue `ceres`): **Freeport Exchange** (`freeport_exchange`: metals ask; Freeport Works' alloy bid
is carried by the same org under the name Freeport Works), **Occator Water & Propellant** (`occator`:
volatiles and propellant asks), **Belt Carriers' Cooperative** (`coop`: provisions). Stickney (venue
`stickney`): **Kasei Metals** (`kasei`: metals and volatiles bids at P07; alloys and propellant asks at
P08), **Yard Commissary** (`yard_commissary`: provisions). **The Freeport Exchange is not in doc 11
§4** and is added by this slice (§10's doc 11 amendment): the Port Council's commodity floor, a
Freeport institution (11 §3.3's Council seats registered claim-holders), consigning the Belt claims'
metal concentrate (Occator's extractor drones and the smaller claim-holders, 11 §4.1; doc 10 §11's
Ceres export) under the Exchange's mark and taking the corridor's alloy intake under the Freeport Works
name; its capital is the slice's largest because it is the return leg's buyer. It has **its own officer
of record**, the Exchange's factor (18a: Margit Søndergaard-Achebe, `npc.sondergaard`, C07); the
Occator factor (Qureshi) signs **only Occator's lines**, because under the Warrant (11 §2) an officer
of one org cannot be the liable principal of another's offers. Storefront vaults are
`CustodyRef::Container` of a manifold or cage item placed in each station's staff-locked Maintenance
Spine (C15/P15), so no player can `get` it and no `ItemDef` lock is needed; buyer warehouses are the
same vaults. Each listing is a standing offer signed by the org's officer of record (the `signatory` in
the `vendors` block, **required** by the `port` schema: an entry with an unsigned listing is refused
before any mutation), the Warrant principal for every acceptance under it (11 §2; Q-18-10); the accept
line names them. Five signatories: the Exchange's factor, Occator's factor, the Co-op chandler (C08),
Kasei's receiving clerk (P07) and the Yard Commissary's storekeeper (P09; 18a: Hanne Okonkwo-Steyn,
`npc.okonkwo`).

**Stock and capital (D-18-27).** Sized for one cohort of eight to ten pilots for about three weeks (the
pasted seed is about ten round trips in all, beats L and M). Ceres: metals 8,500 t, volatiles 3,600 t,
propellant 5,000 t, provisions 2,600 crew-days; Freeport Works' alloy intake 12,000 t. Stickney: alloys
12,000 t, propellant 5,000 t, provisions 2,600; Kasei's intake metals 8,500 t and volatiles 3,600 t.
Capital by one-shot `create_system_obligation` tranches of at most 800,000 cr, each tranche's receipt row
written in the same batch as its obligation (the economy does not dedup, `api.rs:1683-1818`), under a
per-instrument mint-cap override of 1,000,000 in the template: Kasei 600,000 (self-replenishing: its
alloy and propellant sales exceed its intake); Freeport Exchange/Works 4,000,000 (a season budget: it
pays about 52,000 per full return and receives about 14,000 per outbound); the Trust 250,000; the
rest 50,000 each. Exhaustion is honest: a good with no listing quantity renders "SOLD OUT: seeded 8,500 t,
0 left" from the port map and receipts (a listing cannot carry 0, `record.rs:634-636`); an unfunded or
exhausted bid renders "not buying". Restock is P5, by production only, and must land before a second
cohort. `fund_from_faucet` is never used (a 60 s drip, `api.rs:556-590`); capitalization settles on the
next sweep, so the readiness report lists unfunded buyers until it does.

**Presence (D-18-12 kept, extended).** `MarketModule::with_venue_gate(Arc<dyn VenueGate>)`, consulted at
quote, accept and sell with (actor, venue, endpoint) and a presence mode: `Presence::Here` (the actor in
the port's rooms and the hold's hull Direct on this port's domain and not under way) for typed verbs;
`Presence::Ship(hull)` (the hull's locality and the author's live lease; the actor's room ignored) for
queued orders. The locality chain is backing → space → anchor host → hull Direct on the domain. With no
argument `market=` defaults to the port the actor stands in and `to=`/`from=` to the berthed hold.
Locality on **every** inventory path (`@inv/move` between player vaults) is deferred (D-18-37): the pilot
holds exactly one vault and only staff create vaults (`mush-economy/src/commands.rs:3973-3976`).

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

**Passage length and rate (D-18-30; replaces D-18-17).** The lane `ceres_stickney` / `stickney_ceres` via
Kirkwood Depot, legs 0.74 AU and 0.56 AU, unladen `base_s` **540,800 + 409,200 = 950,000 IC s** (11.0 IC
days). **No test, constant or per-leg distance for this exists today**: `coast_km_s` appears nowhere in
the workspace, the only T17 tests are the ephemeris pins, `LegDefWire` is `{to, base_s}` with no
distance (`route.rs:76-80`) and `RouteDefWire.distance_km` is one `Option<u64>` for the whole route
(`:255-273`), which `route_duration_at` scales as a whole (`:849-863`). **IC2 therefore adds** a T17 test
that takes each leg's distance from the sky — `distance_km_at(ceres, kirkwood, T17)` and `(kirkwood,
mars, T17)`, with the new `kirkwood` body's orbit (2.06 AU) authored so they come to 0.74 and 0.56 AU —
pins each leg's `base_s` as `distance_km / 205` (the `coast_hauler` coast rate, a constant the test
owns) and the route's single `distance_km` as the sum, and refuses if the lane pack disagrees.
**Payload changes flight time now**, because doc 10
T1 is binding (`10-tacsim-foundations.md:286-291`) and Draft 0.1's deferral was a live breach: a new pure
`route_leg_durations(route, drive, payload) -> Vec<u32>` using `route_duration`'s exact per-leg arithmetic
(`route_duration` becomes Σ of it, so its pins hold); `CarrierJourney.leg_s: Option<Vec<u32>>`, a
serde-defaulted wire field on the `applied_revision` precedent (`journey.rs:204-232,309-318`), validated
by a fallible `with_leg_times`, read by `leg_due_at` so `due_cursor`, `arrival_at`, interdiction and
`@journey/show` inherit it; `pub mush_sim::depart` takes `leg_s`; the burn ticket freezes it. The drive is
the hull's (`DriveProfile::new(class, 1000, 181,000).with_law(coast)`, `route.rs:637-660`), never
`nominal_drive`. With the laden law leg_s = base_s × (181,000 + payload)/181,000: near-empty (32 t fuel)
12.9 h, a 70 t return 17.75 h (17 h 45 m), a full outbound 19.6 h, a brimmed 180.7 t hull 22.0 h at
rate 24 — all under a day with at least two hours of slack before a same-time login (R3), and a light
load lands earlier.
The alternative (Q-18-15) is Draft 0.2-thin's fixed 21 IC days with the evaluator deferred. The 181 t
ceiling stays a non-binding rail: it is also the laden denominator (`route.rs:806-831`), so a binding
ceiling would slow every passage. **Rate 24** (doc 10 1A's one OOC hour per IC day) is a deploy step and
a readiness line: dev1 seeds `IC_RATIO=4` (`deploy/dev1/instance.env:81`), where a 20 h passage is five
real days; `@tacsim/time rate=24` on the next owner-approved redeploy (Q-18-16), and `+coop/enrol`
refuses at any other rate outside test worlds. Tests run at 3600 (a full leg in about 470 s). The
real-time ETA is (arrival_ic − now_ic)/rate, re-derived after a rate change; journeys relabel across
generations (`journey.rs:1230-1257`).

**The fuel function.** `burn_t = ceil((dry_mass_t + payload_t) × burn_permille / 1000)`, `burn_permille`
105 on the Dray's drive; a tacsim pure function beside `nominal_drive` (`store.rs:838-850`). `RouteDef.
propellant` is authored 0 and **not** reinterpreted (the `store.rs:823-835` ruling and `e7c1.rs:1447`
treat it as a payload-unit quantity). Fuel aboard is departure mass, so a reserve costs 105 kg per tonne
carried; `+ship/plan` says so and shows the remainder; no minimum reserve rule (Q-18-26). The tank cannot
fuel both legs with a full hold (out 45 t at a brimmed tank, back 37 t), so Stickney's price matters.

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
             "text": "Sold 70 t metal concentrate to Kasei Metals at 300.00cr/t: 21,000.00cr; Co-op share 1,750.00cr; you received 19,250.00cr." } ] }
```

Bounds: 16 orders; 32 log lines of at most 240 bytes, FIFO; `kind` ∈ buy | sell | depart | stop;
`on_short`/`on_fail` ∈ partial | skip | hold; order `state` ∈ pending | inflight | done | skipped | held;
document `state` ∈ running | paused | held. The venue is implicit (the port the hull is docked at when the
step runs). `fill` is a quantity for propellant (top the tank to the full-hold burn of the next depart's
route, the fixed point ceil((240 + 100 + fuel + 0.68) × 0.105) = 41 t) and for provisions (that depart's
crew-days). A depart's `not_before` is a UTC instant, a daily UTC time or `+Nh` (a real-world habit, immune
to `@tacsim/time`), with an optional `not_after` window after which the order holds rather than leave at
an odd hour. **The fiction of the queue, one story in both documents (Q-18-24).** The orders document
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

**Offline execution (D-18-32).** The order arm of the tacsim normal-lane tick (IC5's skeleton; period
5 s) rotates over hulls with orders (visit ≤ 64, work ≤ 16), takes at most ONE step per hull and ONE
authoritative submission per pilot per fire, and for a due step: (1) re-checks that the author still
holds the live lease; (2) evaluates the condition from World state (placement and JourneyBook,
MarketStore listings and bids, hold pools through `Ctx::action_state`, available funds); (3) swaps
`ctx.actor` to the pilot (`mush-npc/src/module.rs:1496-1502`; `tacsim/src/seed.rs:384-398`), calls ONE
closed typed function, and restores it — **buy** = `trade::quote` then `accept_into` against the port
counter, destination the hold vault, the quantity clamped first to min(qty, listing remaining, capacity,
available funds **less the next depart order's burn and crew-days at this port's prices**) because
`quote` refuses a quantity above the listing (`trade.rs:246` at `c87d78f7`); **sell** = the Wanted lot arm;
**depart** = the undock checks, then `tacsim.port.burn` staged as the pilot under key
`tacsim.port.burn/<hull>/<order>/<attempt>`, the same tick launching the ticket on a later fire; (4)
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

**What the packs declare.** Each station is a surface region on its body plus one neighborhood (the tested
path; no orbital region), one site, one structure with floors rising in index and z
(`validate.rs:1679-1715`): Freight Deck 0 at z 0, Public Deck 1 at z 6 (Ceres) / 7 (Stickney), Ceres' Office
Deck 2 at z 10.5. Cross-deck exits carry `portal.at` + `dz_m` (`mode: stair` for the public stairs,
`hatch` for the staff hatch, `ladder` for the Registry ladder; no `lift` mode exists). Every room is
`kind indoor` or `circulation`. Paired exits lock only the way in (`apply.rs:1500-1519`); the one
restricted room per station below the office deck uses `lock: staff` + `portal.visibility: hidden`
(the staff-lobby precedent). **Partitions between adjacent public rooms with no exit are `wall`
features** (`blocks.movement`), because only structure rings and Wall features are walls on the scene
map (`raster.rs:412-450`): Ceres C02|C10, C01|C10, C01|C08, C05|C06, C06|C09; Stickney P05|P08, P08|P11,
P02|P10, P01|P10, P09|P12. Hydration `{enabled:false, max_player_sites:0}` on both neighborhoods. Themes:
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
lacks `port.dock`. Names below doc 11's principals are proposals (Q-18-12).

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
        "reference_major": { "metals": 200, "volatiles": 100, "structural_alloys": 400 },
        "buyout_major": 90000, "dormancy_real_days": 7, "per_account": 1
      },
      "vendors": [
        { "org": "freeport_exchange", "storefront": "neighborhood.ceres_freeport.item.exchange_manifold",
          "agent": "freeport_exchange.clerk", "signatory": "neighborhood.ceres_freeport.npc.sondergaard",
          "stock": [ { "good": "metals", "qty": 8500, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "metals", "price_major": 200 } ],
          "bids":  [ { "good": "structural_alloys", "price_major": 520, "intake": 12000, "as": "Freeport Works" } ] },
        { "org": "occator", "storefront": "neighborhood.ceres_freeport.item.occator_manifold",
          "agent": "occator.clerk", "signatory": "neighborhood.ceres_freeport.npc.qureshi",
          "stock": [ { "good": "volatiles", "qty": 3600, "grade": 50, "issue": 1 },
                     { "good": "propellant", "qty": 5000, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "volatiles", "price_major": 100 }, { "good": "propellant", "price_major": 40 } ],
          "bids":  [] },
        { "org": "coop", "storefront": "neighborhood.ceres_freeport.item.chandlery_cage",
          "agent": "coop.clerk", "signatory": "neighborhood.ceres_freeport.npc.akintola",
          "stock": [ { "good": "provisions", "qty": 2600, "grade": 50, "issue": 1 } ],
          "asks":  [ { "good": "provisions", "price_major": 10 } ],
          "bids":  [] }
      ],
      "routes": { "stickney": "ceres_stickney" }
    }
  }
}
```

Ask quantities are never authored: an ask lists the live pool. `hull_names` are the Trust's **laid-up**
names, replaced by the pilot's choice at `+coop/enrol` (the registry mark stays); no hull is pre-named
for a transcript. Stickney's entry has no `programme`, orgs `kasei` (capital 600,000) and
`yard_commissary` (50,000), `stickney_yards` (50,000, lore), vendors Kasei (stock alloys 12,000 t and
propellant 5,000 t; asks alloys 400 and propellant 60; bids metals 300 with intake 8,500 t and volatiles
200 with intake 3,600 t; signatory `neighborhood.stickney_yards.npc.achterberg`, the receiving clerk)
and the commissary (provisions 2,600 at 10; signatory `neighborhood.stickney_yards.npc.okonkwo`, the
storekeeper), `rooms` for the gallery, customs, harbourmaster, exchange, receiving, commissary and
spine, and `routes: { "ceres": "stickney_ceres" }`.

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

**Idempotency.** As Draft 0.1, plus: stock issues are deduped by the `tacsim.port_stock` receipt (the
derived LotId refuses reuse while the lot lives, `pool.rs:1280-1292`; the receipt outlives the dissolve);
the stake is keyed by the lease contract; the burn by hull/order/attempt; the lot leg by contract.

**Config flips on dev1.** `[economy]` per-instrument mint-cap override for `tacsim.credit` = 1,000,000
(tranches of ≤ 800,000). `[tacsim.economy].market` stays false. `[onboarding].start_room` stays 0.
`[tacsim].pack_paths/seed_paths` stay `[]`. World rate to 24 by `@tacsim/time rate=24` **after the owner
approves a redeploy; never redeploy without the owner's ask** (memory rule).

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
| IC1 | Two stations, the IC door and the IC-return rule | The four atlas packs (13 + 12 rooms, three decks at Ceres; wall features on every exitless partition; themes; thirteen NPCs with fallbacks; principals `enabled:false`; lore pools); `RoomDef.markers` + the `port.dock` topology seed (`validate.rs:1958-2000`); unlocked `exit.limbo_arrivals`; the `tacsim.last_port` stamp, the door hook and `+ic`; boot list + shelf; validate-clean, digest-pin and kept-world boot tests | mush-atlas, tacsim | M | — | fresh and kept-world boots apply; `c` lands in C03; every public room walkable; Stickney validates with no foot route; exitless partitions walled on `+map`; `o`→`x`→`c` from a Stickney position refused with the `+ic` hint; starter digests unmoved; T-45 |
| IC2 | The Dray, the goods table and the lane | `dray.json` (class hauler, `logistics`, `interior`) in `fielded_sources` + pin; `UnitClass::Hauler`; the lobby filter + custody refusal test; `goods.json` (tonnes/crew-days, stowage, mass) with its own pin, deriving the catalog; `tacsim.lane.ceres_phobos` v1 (fresh id; bodies incl. `kirkwood` at 2.06 AU, `pallas`; three locations; two routes at 540,800 + 409,200 s, `distance_km` the sum, `propellant 0`) via the game-packs list; **a new T17 leg test** (`lane_legs_pin_t17` in `tacsim/tests/ephemeris.rs`: each leg's `base_s` = `distance_km_at(a, b, T17) / 205`, the route's `distance_km` their sum; none exists today, §4.4) | tacsim | M | IC1 | `lane_legs_pin_t17` green: `@corridor/show ceres_stickney` prints two legs of 540,800 and 409,200 s from sky distances of 0.74 and 0.56 AU; SIM UNIT never offers `dray`; `@tacsim/pack list` shows `tacsim.lane.ceres_phobos@1` beside the untouched `tacsim.corridor` fixtures; the goods validator refuses a family-less trade row; shipped DefinitionRefs unmoved |
| IC3 | Ports, orgs, capital and readiness | `PackEntry::Port` + `PortApplier` a–c (two passive domains; treasury; the seven orgs with capital tranches, each tranche's map row in its batch; the Trust's scoped programme account; fail-closed port map; `signatory` required on every listing); mush-sim: `open_passive_domain` skips the `MAX_WINDOWS` guard for passive opens (`window.rs:205-206`, S); the composition's `test_world` flag; the template mint-cap override 1,000,000; `+credit` (posted, held, available); `@tacsim/port status` | tacsim, mush-sim, mush-server | M | IC1, IC2 | after boot and one sweep `@org/list` shows the orgs and capital; a double boot posts nothing; a corrupt map refuses re-creation; `@domain` shows both passive domains presenting into the galleries; **a kept world with eight windows open at restart still applies the lane pack**; the readiness report lists every binding and flags rate ≠ 24 |
| IC4 | Walking aboard: the boundary rule and the hull template | mush-sim: `Transition::Members`, `member_of` + verify, the destination-aware gate (exit-only, member destination), projection returns the member location, the destroy-gate arm, `backing_valid` false for multi-room spaces, `egress` refused in them, narration over `space.rooms`, `pub declare_compartment`. tacsim: `PortApplier` part f instantiates 8 Drays (Thing + unit + 7 Rooms + 5 exit pairs + 2 spaces + Members + `berths`), titled to the Trust, named from `hull_names`, laid up on the Ceres domain | mush-sim, tacsim, mush-economy | M | IC2, IC3 | staff `place_interior` into an airlock, then `in`/`up`/`down` walk all six rooms with no placement write per step; `o`, `s`, `home` and `@teleport` by the occupant refused; a reboot keeps the walker in the Bridge; e7c6 and placement suites green; eight Drays listed in C01; `@asset/show` title Trust |
| IC5 | Tacsim's authoritative plumbing and the port tick | Tacsim's first action provider registration and typed families (`tacsim.port_stock` receipts, `tacsim.ticket`); tacsim joins `with_quantity_delegates` and gets the inventory handle cell (`composition/mod.rs:404`, pinned edge test); the `tacsim.port` normal-lane Coarse tick skeleton (5 s): rotation cursor, visit/work budgets, per-agent actor swap, at most one submission per actor per fire, the reconnect-notice store on `Connected` | tacsim, mush-server | M | IC0 | a test action submitted by an agent from the tick lands; two agents in one fire do not void each other; GOD never submits; tacsim reads inventory pools via `action_state`; the composition-edge pin updated |
| IC6 | The Co-op programme and boarding | `+coop`, `+coop/enrol` (account open + `offer_lease` + in-verb accept + keyed stake post + hold vault + enrolment doc; gated on the IC3/IC4 readiness lines and the rate only — pools and listings join the gate in IC8, §4.7), `+coop/statement` (readable anywhere); the lease-reading operate gate; the `page_stale_contracts` lease exemption (`sweep.rs:1597-1640`); the observer folding a staff-ended lease; `+ship`, `+ship/board`, `+ship/leave`, `+ship/hold`; the `look` status line (`look_rendered`) and per-room gauges | tacsim, mush-economy | M | IC4 | enrol once (25,000.00cr posted, lease Active, vault opened, hull renamed); a second enrolment from the same account refused; board and leave only by the tenant, only docked; after eight simulated days no stale-contract page; staff `@contract/cancel` revokes; `look` names the hull and the port |
| IC7 | Market seams for content ports | `EconomyModule::with_party_cap_source` (after the frozen table, `contract_live.rs:5163-5169`); `Supply::Stock` + `Representation::Lot` with `LotOp::Draw` (seller-side source projection; synchronous quantity decrement); `with_venue_gate` with `Presence::{Here, Ship(hull)}` + `venue_of_room` + default `market=`/`to=`; the `CapacityProvider` hook on the lot pre-check and the Draw prepare; refuse non-port listings into hull endpoints; the contract-keyed lot leg (`market.lot.<op>.<contract>`, e7r2 sweep). **Not here:** the lapsed-hold release — a hold's release is the `market_lot` action's release op under `listing_holder`, an authoritative submission the GOD lapse sweep may not stage (`tick.rs:28-37`; the IC0 cut note); it is IC8's tick arm | mush-economy, mush-market, mush-inventory | M | IC0 | production composition in miniature: a seeded pool, a Draw-backed ask; accept moves the pool quantity and the money once; the listing decrements; two buyers racing the last tonnes, the second refused before any contract; a gate refusal escrows nothing; capacity refusal before any contract; a re-staged lot leg replays as a HIT; no `SettlementRefused` page |
| IC8 | Finite stock and port commerce | `tacsim_port_stock` (mint + receipt) on IC5's provider; the stock arm (mint → `inv.dissolve` → list, one step per clerk per fire); `PortApplier` d–e (storefront items in C15/P15, vaults, vendors, clerks, grants, stock rows, Wanted docs); the tacsim `VenueGate`, party-cap source and `CapacityProvider` from `logistics`; SOLD OUT and "not buying" rendering; the listing-vs-pool check and the pools/listings lines in the readiness gate; **the lapsed-hold release arm** on the `tacsim.port` tick: one lapsed player listing per fire, the tick acting as the listing holder by IC5's actor swap, the `market_lot` release op keyed by the derived reservation id, verified by pull (D-18-26; Q-18-31) | tacsim, mush-inventory, mush-market | L | IC3, IC5, IC6, IC7 (the two lanes join here) | after boot the ticks seed every pool exactly once (a kill and reboot mid-seed re-mints nothing; conservation from lineage and receipts); at C07 a no-argument list/quote/accept loads the hold pool; 101 t refused; from Limbo refused; into a hull docked at Stickney refused; the board decrements to SOLD OUT; a lapsed player listing's hold is released by the tick, never by the sweep; the applier submitted nothing |
| **IC9** | **Undock and fly — the first player-runnable flight** | `route_leg_durations` + `CarrierJourney.leg_s` + `pub mush_sim::depart(leg_s)`; `tacsim.port.burn` (pool debits, manifest, the ticket with `leg_s`, burn, crew-days, order/attempt); the launch arm on IC5's tick + boot reconcile; `+ship/plan`, `+ship/undock <port>` (at=now); automatic docking at arrival; narration to member rooms at the Kirkwood boundary and at `apply_arrival`; `+ship` ETA in IC and real time; `+travel/status` aboard arm; apply-time refusal of exits into anchored interiors | mush-sim, tacsim, mush-atlas | L | IC5, IC6, IC8 | at rate 3600: undock burns 41 t / 21 crew-days, launches within a tick, the pilot in the Crew Compartment hears Kirkwood (~270 s) and the arrival (~470 s), the hull docks at Stickney and `+ship/leave` lands in P01; the journey's legs equal `route_leg_durations(payload)`; refusals: tank 40 t, overloaded, not aboard, under way twice; a server-half restart between burn and launch yields exactly one journey; one hand-played smoke at rate 24 on local `mushd`, logged in the cut note |
| IC10 | Selling and the Co-op share — the loop closes interactively | `Supply::Wanted`; inventory `transfer_marked` + the Fact-gated payout; `LotOp::Deliver`; the `+market/sell <listing>=<qty> [from=]` lot arm (gate, synchronous checks, buyer-org escrow by party cap, intake decrement/restore); `MarketModule::with_proceeds_policy` + the tacsim margin-over-reference policy; bids by the applier; the enrolment folds sales and shares; sale lines print gross/share/net | mush-market, mush-inventory, tacsim | L | IC9 | at Stickney, sell 70 t metals: Kasei −21,000 = pilot +19,250 + Trust +1,750 from journals; hold −70 t = Kasei pool +70 t from lineage; money moves only after the marker; a sale at reference pays share 0; over-selling, an unfunded bid and a cross-port sale refused before any contract. From here a tester plays one leg per login by hand |
| **IC11** | **Captain's orders — the FIRST CUT WITH THE FULL DAILY LOOP** | `tacsim:orders:<hull>` (fail-closed); the order arm on IC5's tick (rotation, one step per hull, actor swap, closed typed calls, the buy clamp reserving the next depart's needs, `fill`, inflight in the same Outbox, pull verification, relabel on a rate generation); `+ship/orders`, `+ship/order …`, `+ship/plan` over the queue; the UTC time parse/format helper; the watch-log reconnect notices; the pilot-aboard and passenger rules | tacsim, mush-core | **L** (risks 2, 3, 7, 8 live here and D-18-38 makes it the acceptance; if it must be split, IC11a = the document, the nine verbs, the time parser and `+ship/plan` over the queue, IC11b = the executor arm with its restart and clustering tests, M each) | IC10 | at rate 3600 with the pilot disconnected after queueing: depart → arrive → sell ×2 → buy ×3 → depart executes unattended, the log and notices matching; a reboot mid-step re-runs nothing (one sale, one burn); a sold-out buy goes partial, an unfunded bid skips, a depart short of fuel holds and burns nothing; the Stickney queue buys fuel and stores before cargo; two pilots with the same due time both complete |
| IC12 | The story test, help and readiness | `ic_slice_loop.rs`: two simulated days at rate 3600 on the production composition (§6); `docs/help` ports, ships, coop, orders, trade; amendments to getting-started, travel, spatial, cockpit, npc; `ops.md`; the full readiness report and the gate over all of it on `+coop/enrol` (§4.7); every §2 and §6 path walked against the packs' exit aliases; one hand-played two-session smoke at rate 24 on local `mushd`, logged | mush-server, docs, tacsim | M | IC11 | the story test green; a fresh tester completes two legs from help alone; T-45 green; the smoke log shows a sub-day full leg and a docked hull at the next session |
| IC13 | Buyout, graduation and dormancy | `+coop/buyout` (pilot-issued asset contract, Trust accepts by party cap, `end_lease` before settlement with refuse-on-Retrying, observer graduation and re-lease on refund); dormancy recall on the tick; `+coop/release`; `@coop/revoke` | tacsim, mush-economy | M | IC10 | a pilot with 110,000 cr buys out: title to the pilot once, lease ended, the share silent on the next sale; a refunded buyout re-leases; seven idle real days recall a docked empty hull; a buyout while under way refused |

Post-loop cuts, named and ordered: **P-G** real gangway exits (outer door relinked at dock, a station
exit named by berth and hull, `BerthLink`, the Moved observer, atlas `on_moved` ordering tests, boot
reconcile; M); **P-F** berth slots, berth-holder markers, the six freight-deck rooms, `ExitDef`
fail/succ, the `+move` restricted fix (M); **P-H** locality and the under-way seal on every inventory
path at offer projection, for the day a player holds a second endpoint (S–M); **P2** stations in the unit
spec, `+ship/helm`, `request_seat_activation` and the `movement` channel for a second crew member (M);
**P3** provider identity fix, `ledger`/`ship`/`port` providers (M); **P4** Kirkwood as a checkpoint, a
hold of a sim hull, the Customs Assertion front (M); **P5** production restock with conservation (Kasei
refinery work orders, Ceres deposits, a construction sink) before a second cohort (L); **P6** volume
capacity (`si.volume_ml`, ice at 1.5 m³/t) and the `LotCensus` quantity index (M); **P7** Pallas as a
port (data); **P8** delete the legacy seed latch (M); **P9** the net share with a run book and carried
deficit (M).

**IC0.** As recorded in [18-cut-notes.md](18-cut-notes.md). Still owed under Q-18-6: `spacecraft` out of
`SELF_MODES` (`travel.rs:90`) and `options_from_corridor_room`; folded into IC1.

**IC1.** Author the four packs to 18a §1–§2 and §4.5. Add `markers: Vec<String>` to `RoomDef` with the
registry the validator consults; seed `port.dock` rooms in `validate.rs:1958-2000`. Write
`exit.limbo_arrivals` unlocked. Add the `tacsim.last_port` stamp (on `+ship/leave` in IC6 and on the
Limbo-bound OOC exit now), the door hook and `+ic`. Copy the starter digest-pin and validate-clean tests;
add the kept-world boot test.

**IC2.** Author `dray.json`; add `Hauler`, `logistics` and `interior` with `skip_serializing_if`; filter
`hauler` in `fieldable_ids`. Author `goods.json` with its validator and pin; derive the catalog rows.
Author `tacsim.lane.ceres_phobos` with §4.4's legs; the T17 test.

**IC3.** `PackEntry::Port(PortDef)` and the misplacement row. `PortApplier` a–c with the fail-closed map
and capital tranches. The passive-open guard skip in mush-sim. The `test_world` composition flag.
`+credit` with the major-unit formatter. `@tacsim/port status`.

**IC4.** The five mush-sim edits of §4.2 and their tests; `PortApplier` f from the interior template.

**IC5.** Register the provider, the two typed families, the delegate and the handle cell; the tick
skeleton with its budget and the notice store; the composition-edge pin.

**IC6.** `+coop/enrol` and its siblings; the lease gate; the stale-page exemption; the observer; the
`+ship` verbs; the status line.

**IC7.** The six market/economy seams of §4.3 and §4.4, on IC0's `trade.rs`, `lot.rs`, `api.rs` as
landed.

**IC8.** `tacsim_port_stock`; the stock arm; `PortApplier` d–e; the tacsim gate, caps and capacity
provider; the lapsed-hold release arm; the rendering.

**IC9.** The time half (`route_leg_durations`, `leg_s`, `depart`); the burn; the launch and dock arms;
the plan and undock verbs; the narration. Play it once by hand at rate 24.

**IC10.** The sell arm, `transfer_marked`, the Fact gate, `ProceedsPolicy`; `ic_slice_loop.rs` part 1.

**IC11.** The orders document, the executor arm, the verbs, the time parser, the notices;
`ic_slice_loop.rs` part 2 with the pilot disconnected.

**IC12.** Help, the readiness gate, the smoke.

**IC13.** Buyout, release, revoke, dormancy.

---

## 6. Acceptance

**The story test**, `crates/mush-server/tests/ic_slice_loop.rs`, runs on the production composition
(`build_engine`) at `@tacsim/time rate=3600` (the only staff act) with no staff act after boot, in two
parts. In this listing `;` separates lines typed one at a time (there is no command separator; a
leading `;` is the semipose prefix, `mush-core/src/cmd.rs:1300-1304`). Timings are for the full outbound
load at 3600 (a 19.6 h passage is about 471 s; Kirkwood at about 268 s).

```
--- part 1 (from IC10): interactive, the pilot online ---
create Vex pw                       -> Limbo; exits O, S, C
c                                   -> Port Watch Arrivals, Ceres Freeport
registry                            -> (C05 is north of Arrivals; the concourse has no exit to it)
+coop                               -> stake 25,000.00cr; share 250‰ of margin; buyout 90,000.00cr; 8 of 8 Drays free
+coop/enrol Salt Moth               -> "... account #41 ... charters you FPC-D-0401, laid up as the Slack Water, now the Salt Moth ... 25,000.00cr staked ..."
+ship                               -> docked Ceres Freeport, Berth Gallery; cargo 0/100 t; tank 0/80 t; locker 0/120
arrivals ; concourse ; exchange
+market/list                        -> metal concentrate 200.00cr/t 8,500 t · water ice 100.00cr/t 3,600 t · WANTED alloy 520.00cr/t
+market/quote 4=70 ; +market/accept 11     -> "70 t of metal concentrate are loaded into the Salt Moth's hold. Rosalind Qureshi-Mbeki countersigns."
+market/quote 5=30 ; +market/accept 12     -> 30 t water ice, 3,000.00cr
chandlery
+market/quote 6=42 ; +market/accept 13     -> 42 t reaction mass, 1,680.00cr
+market/quote 7=21 ; +market/accept 14     -> 21 crew-days, 210.00cr
+ship/hold                          -> cargo 100/100 t · tank 42/80 t · locker 21/120 ;  +credit -> available 6,110.00cr
exchange ; concourse ; arrivals ; lobby ; gallery ; +ship/board     -> "You cycle the lock into the Salt Moth."  (look: Airlock; status line)
in ; up ; look                      -> Bridge: course, tank and hold readouts
+ship/plan stickney                 -> payload 142.2 t; burn 41 t (1 t left); 21 crew-days; 19.6 IC days ≈ 19 h 36 m real
+ship/undock stickney               -> "Clearance granted. 41 t reaction mass and 21 crew-days committed ... arrival in 19.6 IC days."
down ; crew
   (~268 s)                         -> "Kirkwood Depot slides past ... Leg 2 of 2."     (heard in the Crew Compartment)
   (~471 s)                         -> "Salt Moth docks at Stickney Yards."            (heard in the Crew Compartment)
passage ; airlock ; +ship/leave     -> Visiting Berth Gallery, Stickney Yards
lobby ; customs ; concourse ; exchange ; down
+market/list                        -> Kasei Metals WANTS metal concentrate 300.00cr/t (8,500 t) · water ice 200.00cr/t
+market/sell 21=70                  -> "Sold 70 t metal concentrate to Kasei Metals at 300.00cr/t: 21,000.00cr; Co-op share 1,750.00cr; you received 19,250.00cr. Jonas Achterberg-Nkemelu countersigns."
+credit                             -> available 25,360.00cr

--- part 2 (from IC11): the pilot queues and disconnects before departure ---
(at Ceres, after loading, aboard)
+ship/order depart stickney at=now ; +ship/order sell metals all min=250 ; +ship/order sell volatiles all min=150
+ship/order buy propellant fill max=60 ; +ship/order buy provisions fill ; +ship/order buy structural_alloys 100 max=400
+ship/order depart ceres at=+600s ; +ship/orders ; QUIT
   (tick)                           -> burn 41 t / 21 crew-days; launch; (~471 s) dock at Stickney
   (fires)                          -> sell 70 t metals (+19,250 net); sell 30 t ice (+5,250); buy 40 t fuel; buy 19 crew-days; buy 70 t alloys (short of 100: funds); cash 20.00
   (+600 s)                         -> depart Ceres; (~426 s = 1,533,560 IC s at 3600, the 17.75-day return) dock at Ceres
connect                             -> "While you were off watch:" every line above, once; "Aboard the Salt Moth, docked at Ceres Freeport"
+coop/statement                     -> 2 sales; margin 10,000.00; shares 2,500.00; buyout 90,000.00cr
```

**Assertions.** Conservation from journals and lineage: Σ money over pilot, Trust and the orgs is
unchanged except the boot capitalizations; pilot + Trust = Kasei's payment on every sale; every tonne in
the hold traces to a seeded Origin receipt; seeded − sold − burned = remaining on every listing and pool;
Kasei's pool = delivered intake. No `SettlementRefused` and no stale-contract page.

**Negative checks**, each refused with an in-fiction line and nothing moved: a second `+coop/enrol` on the
same gateway account; another player's `+ship/board` on the Moth; a third person placed into the berths
slot; `+market/accept` from Limbo, or into a hold docked at the other port; 101 t of cargo; ice into the
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
digest including `goods.json`; the T17 leg test; the custody-seam test refuses a hauler; the
composition-edge pin covers tacsim's delegation; T-45 holds the help. The load-sensitive suites rule of
the 16b loop applies to the rate-3600 tests.

---

## 7. Decisions taken here (under the defaults of §8)

Draft 0.1's twenty decisions, re-stated. "Stands" keeps the number and the text; "replaced by R*n*"
records the owner's ruling of 2026-10-15 and points at the decision that takes its place.

- **D-18-1 Ports.** Stands, confirmed by the owner (Q-18-1 closed): Ceres Freeport → Kirkwood Depot →
  Stickney Yards; Pallas a body, a lore thread and the named next port. Tycho is an insurer's agency at
  both ports, no longer the ship desk.
- **D-18-2 Blockers first.** Stands; IC0 landed unchanged as `c87d78f7`.
- **D-18-3 The IC door.** Amended: an **unlocked** Limbo exit (the `OnboardingGate` already bars
  unapproved characters, so no approval marker and no P9) plus an **IC-return rule** (`tacsim.last_port`,
  a door hook, `+ic`) because Limbo exits are global.
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
  `leg_s`, burn, crew-days and the order key; a queued depart stages the same action as the pilot.
- **D-18-12 Presence and defaults.** Stands, extended with `Presence::Ship(hull)` for queued orders.
- **D-18-13 The resource subset.** Amended: five goods (metals, volatiles, structural alloys,
  propellant, provisions); provisions opaque and instrument-less; `FAMILIES` ten; units tonnes and crew-days.
- **D-18-14 Vendor funding.** Stands in shape (finite one-shot capitalization, applier-map dedup);
  numbers resized → D-18-27; tranches ≤ 800,000 under a 1,000,000 cap.
- **D-18-15 Goods origin by issue/retire.** **Replaced by R4** → D-18-26; the E7 pin amendment withdrawn.
- **D-18-16 The travel record.** Stands (`CarrierJourney`; `spacecraft` leaves `SELF_MODES`, Q-18-6).
- **D-18-17 Flight time by authored `base_s` only.** **Replaced** → D-18-30 (binding T1 applied).
- **D-18-18 Docking.** Stands: one passive domain per port; boarding and leaving are placement verbs;
  no gangway exits in the slice (P-G); docking automatic at arrival; no berth slots (P-F).
- **D-18-19 `RoomDef.markers` / `port.dock`.** Stands.
- **D-18-20 The `port` kind.** Stands; its pack is `tacsim.lane.ceres_phobos` v1 → D-18-36.

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
- **D-18-24 The share base.** 250‰ of trading margin over posted reference prices, computed by a
  `ProceedsPolicy` from the source endpoint and paid as an `Exact` payout inside the sale; a sale at or
  under reference pays nothing. The default shares trading margin, not net profit (R2's word);
  Q-18-13 asks whether the owner wants the net. (Rests on Q-18-13.)
- **D-18-25 Programme terms.** Stake 25,000 cr; buyout 90,000 cr frozen at signing; one enrolment per
  gateway account ever; a pool of 8 Trust Drays; programme capital 250,000 with a 50,000 floor; dormancy
  seven real days; release and revoke in IC13. (Rests on Q-18-14.)
- **D-18-26 Goods origin.** Finite seeded stock: minted once per issue by `tacsim_port_stock` with a
  `tacsim.port_stock` receipt as the guard, pooled by `inv.dissolve`, driven by a per-port clerk actor on
  the tacsim tick; asks are Stock+Lot with `LotOp::Draw` (pool to pool, the listing quantity as the
  availability counter, no reservation); bids are `Supply::Wanted` with `LotOp::Deliver` into the buyer's
  pool and a payout gated on the `inventory.transferred` Fact. The applier never submits, and refuses a
  bumped `issue` on a receipted row (paged as drift). A lapsed **player** listing's hold is released by
  the `tacsim.port` tick acting as the listing holder (IC5's actor swap), one per fire, keyed by the
  derived reservation id, never by the GOD lapse sweep. (Rests on Q-18-19, Q-18-31.)
- **D-18-27 Stock and capital sizing.** For one cohort of 8–10 pilots for about three weeks (Ceres
  metals 8,500 t, volatiles 3,600 t; Stickney alloys 12,000 t; propellant 5,000 t and provisions 2,600
  per port; Kasei intake 8,500/3,600 t; Freeport Works intake 12,000 t); capital Kasei 600,000, Freeport
  4,000,000, the Trust 250,000; no restock in the slice (P5); no top-ups by default. (Rests on Q-18-19.)
- **D-18-28 Units and capacity.** Tonnes for bulk goods and propellant, crew-days for provisions, in a
  tacsim-owned `goods.json` with its own digest pin deriving the market catalog; capacity by mass only
  (cargo 100 t, tank 80 t, locker 120 crew-days) checked synchronously in the lot pre-check and in the
  Draw prepare; volume deferred (P6). (Rests on Q-18-21.)
- **D-18-29 The Dray.** 240 t dry, 100 t cargo, 80 t tank, 120 crew-days, two berths at 100 kg, a
  181,000 kg payload ceiling that is a non-binding rail, `coast_hauler` at thrust 1000 with
  `burn_permille` 105; `UnitClass::Hauler`, `logistics` and `interior` as optional fields; no core.
- **D-18-30 Passage.** The laden evaluator applied to live flight now (T1): `route_leg_durations` →
  `CarrierJourney.leg_s` → `pub depart(leg_s)`; the lane's unladen `base_s` is 540,800 + 409,200 s so a
  brimmed hull lands in 22 h, a full load in 19.6 h and a 70 t return in 17.75 h at rate 24; crew-days
  are occupants × Σ ceil(laden leg days); the rate is 24 for play and 3600 for tests;
  `RouteDef.propellant` is 0 and not reinterpreted; IC2's new T17 test pins the legs from the sky.
  (Rests on Q-18-15, Q-18-16.)
- **D-18-31 The queue.** A durable legacy `tacsim:orders:<hull>` document with a closed vocabulary (buy,
  sell, depart, stop; `fill`; UTC departure times), executed by the tacsim pull tick acting as the pilot
  through closed typed calls; `@wait` and `schedule_source` refused as carriers; the verb family is
  `+ship/order`. In fiction it is the **sailing plan** the pilot signs at the port counter, executed by
  the port's clerks and berth under the pilot's signature; the Class 0 ROM only flies the cleared route
  (doc 17 §9; the Warrant). (Rests on Q-18-22, Q-18-24, Q-18-28.)
- **D-18-32 Executor rules.** One step per hull and one submission per pilot per fire; the lease
  re-checked per step; inflight written in the step's batch; completion verified by pull; after a reboot
  verified, never re-run; the buy clamp reserves the next depart's burn and crew-days; deterministic keys
  (burn by hull/order/attempt; lot leg by contract); holds consume nothing and notify.
- **D-18-33 The tick skeleton** (tacsim's first action provider, the two typed families, the quantity
  delegation and inventory handle, the `tacsim.port` normal-lane tick with its budget and the notice
  store) lands as its own cut, IC5, shared by the stock, launch, dock and order arms.
- **D-18-34 Boarding by verb.** `+ship/board` in the gallery and `+ship/leave` in the airlock (and
  nowhere else, in both documents), gated on the lease and the hull's placement; real gangway exits are
  P-G. (Rests on Q-18-18.)
- **D-18-35 The stations.** Ceres 13 rooms on three decks, Stickney 12 on two, as 18a; the six freight
  rooms pruned to P-F; one staff-locked spine per station holding the storefront items; principals placed
  `enabled:false`; wall features on every exitless partition; hydration off. (Rests on Q-18-23.)
- **D-18-36 The lane pack id** is `tacsim.lane.ceres_phobos` v1; the `tacsim.corridor` fixtures are
  untouched.
- **D-18-37 Locality on every inventory path and the under-way quantity seal** are deferred to P-H,
  because the pilot holds one vault and only staff create vaults; the venue gate covers the market; when
  built, the filter sits in offer projection (`world_task.rs:3187-3210`).
- **D-18-38 The daily rhythm is the acceptance.** The slice is done when IC11's queue runs a two-day
  loop unattended and IC12's hand smoke at rate 24 shows a docked hull at the next same-time session.

**Locked decisions this plan touches.** *Leaves:* README superseded 1–7; D-16-* (the arena untouched;
the hauler filter keeps it so); doc 17 D-UM-* (the Dray has no core: Class 0, the drive ROM, nothing
separately titled; the queue is the pilot's instrument, not a mind); T2 (no economic transaction in a
pulse); T4; doc 10 4A (ten families); doc 10 7A (finite capitalization); D-E5-4 (no lock, no exit
rewrite: boarding is a verb and gangways, when they come, use a hook). *Honours:* **T1** (passage time by
route, drive and payload, now applied); T17 (the pinned sky: distances by test); doc 11 §2's Warrant
(every listing has a signatory of its own org; every queued order is the pilot's signed sailing plan,
executed by the port under that signature, and the Class 0 ROM contracts for nothing); doc 17 D-UM-32
and D-UM-11/52 (the ROM flies, never trades); D-14-34 (the
Asker audience); doc 06 §7.5 (the boundary rule, built as written: staff not exempt). *Amends, on the
owner's answer:* C2A–C4A / doc 10 §18.1 / F3's station-channel gate for the hull's departure (D-18-23,
until P2); DL-11 / D-E6-15's frozen party-cap table gains a content-resolved source (Q-18-10); D-14-35's
wording (Q-18-6); doc 11 §4.5 gains the First Captain programme and §4.7 loses the starter-charter line
Draft 0.1 proposed; doc 10 1A is honoured as written (one OOC hour per IC day; a Belt passage of
"weeks" IC in about a day real), so the Draft 0.1 three-day amendment is withdrawn. *Withdrawn:* the E7
"market never mints" pin amendment (R4 makes it unnecessary).

---

## 8. Questions for the owner

Each open question has a recommended default; "as recommended" builds §5 as written.

**Answered on 2026-10-15** (the ruling recorded in one line; the question is closed):

- **Q-18-1 Ports.** Ceres Freeport → Kirkwood → Stickney Yards, confirmed; the owner's own material uses
  the pair. Tycho is an agency at both ports, not the ship desk.
- **Q-18-2 World rate.** "Real-world travel times less than a day, around a day": the lane is laden
  13–22 h at rate 24, so 24 is the play rate and the next owner-approved redeploy sets it (Q-18-16 asks
  only for the go-ahead).
- **Q-18-2b Lane length.** About a day real, primarily offline, with a productive 30–60 minute daily
  session: 950,000 IC s unladen, 19.6 h at a full load; the three-day alternative is withdrawn.
- **Q-18-4 Starter ship.** A charter, never a required purchase; owning a hull is "a couple of weeks of
  good luck away" and a buyout exists (IC13). The charter is the lease now, from the Co-op's programme.
- **Q-18-8 Ports as economic boundaries.** No: "seed an initial finite world stock versus just creating
  things out of thin air". Issue/retire is gone; stock is minted once with receipts.
- **Q-18-9 Vendor funding.** Finite; one-shot capitalization; no drip.

**Still open from Draft 0.1** (defaults unchanged unless noted):

- **Q-18-3 Food.** Default: `provisions`, opaque, crew-days, no instrument, `occupants × ceil(days)` per
  leg. Alternatives: an 11th family; no food. Why: zero code, doc 10 §9's granularity.
- **Q-18-5 Dray logistics shape.** Default: optional `UnitSpec.logistics` (now with `stowage` and
  `drive`) and `UnitSpec.interior`, read only by the port verbs. Alternative: `ComponentRole`
  drive/hold/tank/life_support plus a `drive` rules kind. Why: digests stable, `bind` untouched.
- **Q-18-6 D-14-35.** Default: decide it as two records with a hard boundary and remove `spacecraft`
  from `SELF_MODES` (folded into IC1). Alternatives: fold (1,786 lines); leave open.
- **Q-18-7 When the burn settles.** Default: at undock, matter first, the launch by the tick consuming
  the ticket (now carrying the manifest and `leg_s`). Alternatives unchanged.
- **Q-18-10 Party caps by content reference.** Default: yes, conferring no money authority. Needed by
  every port ask and bid and by the buyout.
- **Q-18-11 May packs declare money?** Default: yes, as numbers executed by the applier and verbs under
  the 1,000,000 cap in ≤ 800,000 tranches.
- **Q-18-12 Names.** Proposed and replaceable (18a §4): the eight laid-up hull names (*Slack Water*
  first; *Salt Moth* is the transcript pilot's chosen name); the thirteen NPCs of 18a; the Freeport
  Exchange; the Co-op First Captain Trust; `tacsim.registry`; `coast_hauler`; the verbs
  `+coop`, `+ship`, `+ship/order`, `+ic`; the registry prefix FPC-D-04. "Names ok", or replacements.

**New in Draft 0.2:**

- **Q-18-13 Share base.** Default: 250‰ of trading margin over posted reference prices (metals 200,
  volatiles 100, alloys 400), paid inside each sale; computable from the board before logging off; no run
  book. Alternatives: net of average-cost basis, burned consumables and fees with a carried deficit (adds
  the M run book, P9; fairer to a run whose fuel ate the margin; the fidelity lens's pick); gross permille
  (simplest; bills losing runs). Why: deterministic, no new record, never bills a sale below the origin price.
- **Q-18-14 Programme terms.** Default: stake 25,000; buyout 90,000 fixed at signing (day 14–15 at full
  loads, 21–24 at 70 %); one enrolment per gateway account ever; 8 hulls; no top-up. Alternatives: buyout
  75,000 (day 12–13; near trivial); 60,000 (day 9); a list price less half the shares paid; re-enrolment
  after a 30-day cooldown; one 10,000 top-up when cash cannot fund a load. Why: R1's "couple of weeks of
  good luck", not trivial.
- **Q-18-15 Passage law.** Default: apply `route_duration`'s laden law now (T1 is binding) with the
  unladen lane at 950,000 IC s, so every load lands in 13–22 h at rate 24; the 181 t ceiling stays a
  non-binding rail. Alternatives: a fixed 21 IC days with the evaluator deferred (the thin design; one
  fewer S–M in IC9; keeps T1 breached); a reference-mass term in `DriveProfile` to damp the 2× curve
  (revises the T1 pins); a binding 165 t ceiling (slows every passage, since capacity is the laden
  denominator). Why: a light load lands earlier and the worst case still leaves two hours of slack.
- **Q-18-16 Rate 24 at the next redeploy.** Default: `@tacsim/time rate=24` is a line in the next
  owner-approved dev1 runbook, and `+coop/enrol` refuses at any other rate outside test worlds. On dev1's
  `IC_RATIO=4` a 20 h passage is five days and the daily loop silently fails. Never redeploy without asking.
- **Q-18-17 Must the pilot be aboard for a queued departure, and what of a passenger ashore?** Default:
  yes, Interior in any member room, connection irrelevant; a depart with the pilot ashore holds with
  nothing burned and notifies; a listed passenger ashore is left behind with a notice to both.
  Alternatives: the hull sails on the Class 0 ROM with whoever is aboard; refuse while any listed crew is
  ashore. Why: the Warrant's liable principal travels with the hull, and the manifest stays honest.
- **Q-18-18 Gangway: verbs or real exits.** Default: `+ship/board`/`+ship/leave` in the slice; real
  exits (outer door relinked at dock, a station exit named by berth and hull, a housing-shaped access
  hook, no lock rewrites) as cut P-G right after the loop. Alternative: the exits now (an M cut before the
  first flight). Why: the same seal for forty lines; walking ashore each session is texture the loop can
  add next.
- **Q-18-19 Stock sizing, restock and top-ups.** Default: seed for one cohort of 8–10 pilots for about
  three weeks (D-18-27); restock only by P5 production before a second cohort; **no** attributed top-ups.
  Alternatives: the pasted seed (about ten round trips in all; unplayable for a cohort); permit a bumped
  `issue` on the port entry as a receipted staff top-up until P5. Why: R4 forbids thin air; a receipted
  top-up is a staff act the owner may or may not count as thin air.
- **Q-18-20 Berths.** Default: none in the slice; every hull shares the gallery and docking never holds
  at approach. Alternative: berth slots equal to the pool at both ports (8/8) with berth-holder markers
  (P-F). Why: a berth count below the pool stalls offline turnarounds when departures cluster at one hour.
- **Q-18-21 Volume capacity.** Default: mass only. Alternative: `si.volume_ml` in `goods.json` with
  water ice at 1.5 m³/t so volume binds on ice-heavy loads (P6). Why: at the pasted 0.5/1/0.5 m³/t a
  120 m³ hold can never bind and is dead UI.
- **Q-18-22 The queue's verb and limits.** Default: `+ship/orders` and `+ship/order …` (`+orders` is
  mush-propagation's); 16 orders, 32 log lines; no repeating plans in the slice (the daily login re-files
  two or three lines). Alternatives: `+plan` as a bare verb if it is unregistered in the production
  composition (it appears only in a housing test); `+voyage`; a capped `/repeat <n>` later (unattended
  repeats also drain finite stock).
- **Q-18-23 Station scope.** Default: 13 + 12 rooms; the six freight rooms behind berth-marker locks
  pruned to P-F; restricted rooms stay hidden on `+atlas` even to berth holders. Alternatives: all fifteen
  rooms per station now (adds marker locks, marker sync, the `+move` fix and `ExitDef` fail text to IC1);
  a third access level visible to whoever the lock admits.
- **Q-18-24 The Warrant and standing orders.** Default: the queue is the pilot's signed **sailing
  plan**, filed at the port counter; the port's clerks and berth execute its sales, loading and
  departure under the pilot's signature, the Class 0 drive ROM only flies the cleared route (doc 17
  §9, D-UM-32; the Warrant forbids it to contract or sign, 11 §2), and every log line names the pilot
  as liable principal. One fiction in both documents (18a's Bridge pane and port rooms already tell
  it). Alternative: the queue limited to departures, sales interactive only.
- **Q-18-25 The six rooms.** Default: Airlock, Central Passage, Bridge, Crew Compartment, Cargo Hold,
  Engineering Access, plus the unwalkable Hold Stowage; plain names with the hull named by the `look`
  status line. Alternatives: five rooms (drop Engineering Access); hull-prefixed room names (a rename
  must rewrite them); a separate galley.
- **Q-18-26 Fuel reserve.** Default: no minimum; the remainder is shown on `+ship/plan`. Alternative:
  refuse a departure under 10 % of the burn in reserve.
- **Q-18-27 The charter's rent.** Default: a 1-minor rent on `EconomicClock::Manual` (never due) plus
  the stale-page exemption in IC6. Alternative: an S economy change making `LeaseSpec.rent` optional with
  a lease marker.
- **Q-18-28 Where the queue may be edited.** Default: aboard the hull, or anywhere in the port area where
  it is docked; readable from anywhere. Alternative: anywhere, through the relay (doc 15).
- **Q-18-29 Kasei's capital as a Ceres Clearing facility (11 §4.3).** Default: plain treasury
  capitalization now. Alternative: an applier-keyed Clearing→Kasei post plus a Manual-clock debt record;
  but `create_obligation` admits only an Object actor on the debit account (`api.rs:1984,2012-2017`), so
  the debt record needs a keyed `create_system_obligation` variant (08a) first.
- **Q-18-30 The Trust as a separate org.** Default: yes (`coop_trust`, `ChapterOf` the Co-op in the
  faction cut), so eight leases do not consume the Co-op's 16 issuer slots. Alternative: charters issued
  by the Co-op itself, with the pool capped and the Co-op's own hauling contracts counted against it.
- **Q-18-31 Who releases a lapsed player listing's hold.** Default: the `tacsim.port` tick, acting as
  the listing holder through IC5's actor swap (the executor pattern), one release per fire keyed by the
  derived reservation id, verified by pull (IC8) — because the release is the custodian's authoritative
  submission and the GOD lapse sweep may not stage it (the IC0 cut note; `tick.rs:28-37`).
  Alternatives: the seller withdraws by hand and the sweep only pages (IC0's state today); a
  mush-market-owned release on the economy's carve-out lane (an engine change, M). Why: the swap
  already exists for the stock and order arms and a port ask never holds, so only player listings
  are touched.

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
12. IC0 landed (`c87d78f7`) moving `mush-market` `trade.rs`, `lot.rs`, `commands.rs`, `module.rs`,
    `api.rs` and `mush-sim` `commands.rs`; this draft's cites into them are at `c87d78f7` where marked
    and at `8a32734a` otherwise, and IC0's test comments and cut note call Draft 0.2's IC7 "IC5"
    (Draft 0.1's numbering; §5 carries the mapping).
13. The stale-contract page: until IC6's exemption lands, any test that keeps a lease Active for seven
    real days pages; recovery and housing leases have the same latent defect today.
14. Doc drift a builder will hit, as Draft 0.1's risk 11, plus `e7r2.rs:327-328` ("exactly
    extraction's"), `custody.rs:212-220`, and 10b §3.11's issue/retire note, which is withdrawn.

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
- Payload-independent passage time (T1 is applied); volume capacity, a minimum fuel reserve, passengers
  and a manifest beyond the tenant (reserved berth); port fees, Kirkwood customs or holds (P4).
- Capacity or locality checked only on the market leg as the final shape (P-H); rewriting exit locks per
  crew change (D-E5-4).
- Reusing the `tacsim.corridor` id or editing the corridor fixtures or the starter packs; a Luna port, a
  Pallas station, Earth/Luna on the map.
- An 11th family, a thrust mobility mode, a `drive` rules kind, a `lift` portal mode.
- Redeploying dev1 without the owner's ask.

---

## 10. Supersessions and amendments owed

- **README.** The doc 18 row becomes:
  `| [18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) | **The trade run, the first IC vertical slice (Draft 0.2, 2026-10-15, at c87d78f7; D-18-1..38 under stated defaults, Q-18-13..30 open, six closed by the owner's rulings R1–R6)**: a pilot walks from Limbo into Ceres Freeport, signs the Belt Carriers' Cooperative's First Captain programme (a chartered Co-op Dray under \`Encumbrance::Lease\`, a 25,000 cr stake, a quarter of the trading margin to the Co-op inside every sale, a 90,000 cr buyout two weeks of good runs away), loads metals, ice, reaction mass and provisions into a six-room Dray's hold from finite receipted stock, queues the captain's orders and logs off; the ship burns, flies the pinned Ceres–Kirkwood–Stickney lane in about twenty hours at rate 24, docks itself, and the queue sells to Kasei, buys and departs again at the pilot's hour; the verified map A–P, the design (backer, interior, goods, the captain's day, stations, pipeline), the Dray definition, fourteen cuts (IC0 landed at c87d78f7; IC9 first flight; IC10 closes the loop; IC11 the first daily-loop cut), the two-day story test |`
  and a new row follows it:
  `| [18a-trade-run-stations-and-ship.md](18a-trade-run-stations-and-ship.md) | **The trade run's places (content brief, Draft 0.1, 2026-10-15)**: Ceres Freeport (13 rooms, three decks) and Stickney Yards (12 rooms, two decks) room by room with themes, details, exits, items, the NPC cast with voices and fallbacks, lore pools and ambience; the Dray's six-room interior and its stowage in the same voice; how the brief becomes atlas packs (decks as floors, locks, wall features, the \`port.dock\` seed, the port entry's room references, the validator tests); the names proposed for Q-18-12 |`
- **README "Superseded decisions".** No new row yet; on Q-18-6 a row for D-14-35 as decided.
- **14b §3 / 14-spatial-implementation-plan.md:1321.** D-14-35 status and wording (Q-18-6).
- **Doc 06 §7.5.** A note that the boundary rule is built in IC4 as written (member rooms; staff not
  exempt), and that `InteriorSpaceRecord.rooms` has a writer.
- **10b §3.11.** The Draft 0.1 note on issue/retire is **withdrawn**; port listings draw from seeded,
  receipted pools (D-18-26). 10b:1753's declared-price lease buyout is built as `+coop/buyout` (IC13).
- **08a.** Ledger rows for: Lot in `deliver()`'s short-circuit and the listing-TTL cap (IC0, landed);
  `with_party_cap_source`; `with_venue_gate` with `Presence::{Here, Ship}`; `Supply::Wanted`;
  `LotOp::Draw` and `LotOp::Deliver`; the `CapacityProvider` hook; `with_proceeds_policy`; the
  contract-keyed lot leg; the lapsed-hold release by the port tick as the holder (not the sweep;
  Q-18-31); `transfer_marked`; the `page_stale_contracts`
  lease exemption; and the options a keyed `create_system_obligation` variant and an optional
  `LeaseSpec.rent` (Q-18-27, Q-18-29).
- **Doc 10 1A.** Honoured as written; the Draft 0.1 three-day amendment is withdrawn.
- **Doc 10 7A.** Nothing; finite capitalization stands.
- **Doc 10 T1.** Applied in IC9; the "deferred, not denied" note of Draft 0.1 is closed.
- **Doc 10 C2A–C4A / §18.1; doc 17 F3 row.** The note that `+ship/undock` bypasses the station-channel
  gate until P2 now cites R3 (an offline departure cannot wait on a live seat).
- **16c.** The hauler class filter and the custody-seam refusal test; the `@journey` "(staff)" help.
- **Doc 11.** §4.5 gains the First Captain programme and the Trust; §4.7's starter-charter line (Draft
  0.1) is not added; §4 gains a **Freeport Exchange** entry (the Port Council's commodity floor:
  consigns the Belt claims' metal concentrate and takes alloy intake as Freeport Works; its own factor
  as officer of record; the slice's largest capital), and §4.8 a note that Clearing's market-maker role
  is unused by the slice and that at the Freeport window it "opens accounts for captains and lends
  nothing to pilots" (never "lends to governments", which §4.8 does not say); §4.3 that Kasei's
  reaction-mass price is Republic-posted and its ice intake serves Phobos's volatiles demand, not the
  propellant line; §5 a note that
  Kirkwood is a seeded body at 2.06 AU; §4.10 that Pallas has a body entry; §7 the Dray's scale (240 t
  dry, 100 t hold) and the Class 0 ROM.
- **Doc 14 §2.6.** Three starter packs and seventeen rooms; §3.3's mode filter true once `spacecraft`
  leaves `SELF_MODES`. **D-14-5 / 14a 5A** (carrier interiors get local frames) is not foreclosed by the
  interior template: the six rooms can take a local atlas frame later.
- **`mushd.toml.example` [onboarding]** and `mush-config/src/config.rs:394-401`: drop the WoD text.
- **In-code doc comments:** as Draft 0.1, plus `mush-market/tests/e7r2.rs:327-328` ("exactly
  extraction's": work-order delivery also mints), `mush-sim/src/custody.rs:212-220` (withdrawal leaves
  location at "the space object itself" is false under the member projection), `mush-sim/src/data.rs:832-835`
  ("reserved" no longer).
- **Help topics.** New: `ports.md`, `ships.md`, `coop.md`, `orders.md`, `trade.md`. Amend as Draft 0.1
  listed (`getting-started`, `chargen`, `travel`, `atlas`, `spatial`, `cockpit`, `npc`, `intel`, `faction`,
  `chronicle`, `clocks`); in-code `@journey` "(staff)", `@agent` fields, `+market` price lines;
  `mush-market/tests/ic0.rs:19-21`, `mush-server/tests/e7r2_proving_loop.rs:35` and
  `18-cut-notes.md:109-110`, whose "cut IC5's `with_party_cap_source`" is Draft 0.2's IC7 (the IC0
  session owns the cut note; §5 carries the mapping until it is re-pointed).
- **`docs/ops.md`.** The asker-authority bridge; the actor-swap executor posture (closed typed calls, one
  step per hull per fire, verify by pull) beside the typed-action posture at `:976-979`; the rate-24
  deploy line.

**Draft 0.1 review fold (2026-10-15, three lenses).** Unchanged from Draft 0.1 and kept for the record:
F-1 (lane duration to the owner), F-2 (finite funding), F-3 (keyless obligation), F-4 (route scaling),
F-5 (Occator's prices), F-6 (per-round-trip economics), F-7 (no core), F-8 (the helm bypass), F-9
(arrival narration), F-10 (play rate), F-11 (`assignment`), F-12 (mint cap), F-13 (IC7 size), F-14 (the
Warrant signatory). Several are now moot under the rulings (F-1 by Q-18-2b's answer; F-6 by R2; F-8 by
D-18-23) and are listed so a reader of Draft 0.1 can see what moved.

**Draft 0.2 fold (2026-10-15; the owner's rulings R1–R6 and the second review).** By ruling: **R1**
(charter, buyout as an aspiration) replaced D-18-10 with D-18-21/25 and added IC13; **R2** (a
corporation invests on a profit-sharing basis) replaced D-18-5/6/7 with D-18-21/24, deleted
`+credit/advance` and P1, and chose the Co-op through a programme Trust; **R3** (under a day, offline,
a daily 30–60 minute session) replaced D-18-17 with D-18-30, added the queue (D-18-31/32, IC11), the
tick skeleton (D-18-33, IC5), the readiness gate and the rate-24 deploy line (Q-18-16), and re-justified
D-18-23; **R4** (finite seeded stock) replaced D-18-15 with D-18-26/27, withdrew the E7 pin amendment,
and sized the seed to a cohort; **R5** (a five- or six-room ship) replaced D-18-8 with D-18-22 and
D-18-29 (the pasted 100 t scale over 2,800 t), kept D-18-18's verbs (D-18-34), and added IC4; **R6**
(the stations and their flavour) produced 18a and D-18-35, with the owner's fifteen-room grids pruned
to 13 + 12 and the freight decks named as P-F. By review graft: the executor's buy clamp and `fill`; the
Fact-gated bid payout; the receipt family as the mint guard; the Draw's seller-side source projection;
the contract-keyed lot leg; the lapsed-hold release (moved to the port tick as the holder, Q-18-31);
the separate Trust org; the `look` status line, per-room gauges and named hulls; the watch-log framing
and the passenger rule; the NPC voices; the lane pack's fresh id. Corrections to Draft 0.1 found by the
J–P map are listed per beat in §3; the one tree claim Draft 0.1 got wrong that the rulings did not
touch — `tacsim.corridor` v1 as a new pack id — is D-18-36. Not applied: the fiction lens's net lay and
15-room stations (Q-18-13, Q-18-23 carry them); the one-lane design's `Registered` hull endpoints (the
store space keeps the custody locality chain).

**Draft 0.2 review fold (2026-10-15, three lenses: tree audit, rulings and fiction, tester and
buildability), applied in place, by finding.** *Tree audit:* (A-1) the passive-domain narrowing was
half true — `open_passive_domain` refuses while eight windows are open (`window.rs:205-206`) though a
passive domain takes no slot; §3-P, §4.4 and risk 1 re-cited, IC3 gains the guard skip and the
eight-windows kept-world test. (A-2) The §4.4 "content test pins `base_s = distance_km / coast_km_s`"
did not exist (`coast_km_s` nowhere; `LegDefWire` has no distance; `distance_km` is per route); §4.4
and IC2 now say IC2 **adds** `lane_legs_pin_t17` from sky distances. (A-3) IC0 had landed as
`c87d78f7`, HEAD, not "in flight": header, §2, §5, IC0 row, D-18-2, risk 12 and the README row
restated; shifted cites re-pinned (`trade.rs:573/246/515/701-716`, `commands.rs:2605-2614/3428`,
`commands.rs:524-539`, `lot.rs` +78 marked); the Draft 0.1 IC5 → Draft 0.2 IC7 mapping added to §5
(the cut note is the IC0 session's and is not edited here). (A-4) `contract.rs:1361` was
`apply_split`'s doc comment, not a zero-rent check; §4.1 now cites `api.rs:1731/1978`, says the
activation path is unverified and keeps the 1-minor rent as belt-and-braces. (A-5) The 70 t return is
17.75 h by the doc's own law, not 17.0; §2, §4.2, §4.4, §6 and D-18-30 corrected, arrival 13:15 UTC.
(A-6) `items.rs:46-52` is `mush-core/src/commands/items.rs` (no atlas `items.rs`); §3-O and 18a §0.2
re-cited with `apply.rs:1205-1262`. (A-7) 18a §0.1's "twenty-five rooms" summed to thirty-two.
(A-8) `mushd.toml.tmpl` is `deploy/instance/mushd.toml.tmpl` (three cites). *Rulings and fiction:*
(F-1) three fictions for the queue's executor (doc 18's ROM contracting; 18a's ROM "contracts for
nothing"; 18a's berth and dockmaster) became one — the signed sailing plan executed by the port under
the pilot's signature, the ROM only flying (§4.4, Q-18-24, D-18-31, §7 Honours). (F-2) Ceres reaction
mass (40) below water (100) contradicted 18a's "costs more than the water", and Kasei's 200 ice bid
against its 60 propellant ask had no chain; the numbers stand, the lore now reads crude fraction vs
refined water and Kasei's ice bid as Phobos's volatiles demand with a Republic-posted propellant
price (§4.3; 18a §1.5; §10 doc 11). (F-3) The Freeport Exchange was not in doc 11 §4 and its Warrant
signatory was Occator's factor; it is now defined (the Council's commodity floor, the claims'
concentrate), has its own factor (18a: Søndergaard-Achebe), Qureshi signs Occator only, the §4.1
"lends to governments" misquote is corrected and §10 adds the doc 11 entry. (F-4) The Yard Commissary's
ask had no signatory and P09 no NPC: the storekeeper (18a: Okonkwo-Steyn) added, `signatory` made
required, thirteen NPCs. (F-5) 18a put `+ship/leave` in the gallery (C01, the harbourmaster's clerk) and
in the airlock; the airlock wins, as doc 18 had it. (F-6) The applier's bumped-`issue` top-up built
Q-18-19's alternative; under the default it now refuses and pages (§4.3, §4.7, D-18-26). (F-7) 18a's
room details and NPC speech exposed the OOC rate ("a day of the captain's time", "the rate", "the
real one"); stripped to IC time, with the real-time ETA kept on the OOC-facing renders (§4.4). (F-8)
§1's R2 restatement and D-18-24 now say the default shares margin, not net profit. (F-9) §2 and §6
typed exit words 18a never authored (`concourse ; registry`, `pressure`, `arrivals` from C08,
`airlock` from the crew compartment, `gallery` from P12); rewritten to 18a's aliases, with IC12
walking every path. (F-10) The Dray JSON put the airlock aft and 18a forward; the JSON now says
forward and is named the authority. (F-11) "7 of 8 free" and "FPC-D-0403" were wrong on a fresh world
(`next_loaner`'s `min_by_key` charters 0401 and all eight are laid up), and `hull_names[0]` was
pre-named Salt Moth; transcripts read 8 of 8 and 0401, `hull_names` are laid-up names (*Slack Water*
first) replaced at enrolment, the bench remembers another hull. (F-12) Ceres read as one g against
doc 10 §3; 18a §1.1 now says the Hub is a spun ring. (F-13) `+credit`'s "owed" column was the loan
era's; posted, held, available only. *Tester and buildability:* (B-1) IC7's lapse-sweep release arm
was a GOD-sweep authoritative submission, which the tick-cadence rule and the IC0 cut note forbid;
moved to IC8 as a port-tick arm acting as the holder (D-18-26, Q-18-31, §10 08a). (B-2) IC8 depended
on IC5's provider and tick without listing it; IC5 added to IC8 and IC9, and §5 names IC8 as the
lanes' join. (B-3) `+coop/enrol`'s readiness gate at IC6 needed IC8's pools (a cycle) and "test
world" was undefined; the gate now grows per cut and `test_world` is a composition flag (§4.7, IC3,
IC6, IC12). (B-4) The `fill` top-up at Ceres was 40 t/1,600 against a 4 t remainder (37 t/1,480), the
Stickney `fill` bought 21 crew-days against a 19-day return, and the crew-day rule was stated two ways;
one rule (occupants × Σ ceil(leg days)), cash 20.00 at Stickney and 15,630 on day 3. (B-5) The lanes
were not file-disjoint (tacsim's `composition/mod.rs`, `lib.rs`/`commands.rs`, mush-economy); §5 now
says so and sequences IC5's registration first in its own file. (B-6) IC11 at M was the estimate the
schedule would miss; sized L with the a/b split named. (B-7) `+coop/statement` was read at Stickney
and aboard while 18a bound it to C05; §4.1 and 18a C05 now say `+coop` and `+coop/statement` read
anywhere. Not applied: re-pricing the chain (F-2's option a; it re-runs every money figure for no
mechanism gain) and editing `18-cut-notes.md` (the IC0 session's file).
