# 18 — The trade run: the first IC vertical slice

**Draft 0.1 — 2026-10-15**, written against `main` at `f4b14f3b` from a nine-beat verified map of the
tree (A–I, §3), three competing designs and a three-lens review (playability, fidelity, buildability).
The decisions this document takes under stated defaults are collected in §7 as D-18-1 to D-18-20; the
questions put to the owner are §8, Q-18-1 to Q-18-12 plus Q-18-2b (the lane's IC duration, added by the
review fold, §10), each with a recommended default. Nothing here is binding until the owner answers §8;
the defaults are what gets built if the answer is "as recommended".

Reading order for someone new: §0 (the run in one paragraph), §2 (the run beat by beat), §4.1–§4.4 (money,
ship, goods, travel), §5 (the cuts), §8 (the questions).

**The slice in one paragraph.** A new account connects, makes a pilot, and walks out of Limbo through the
`Ceres Freeport <C>` door into Arrivals on the Dock Deck of the Freeport. On the Concourse they open a
credit account at Ceres Clearing and take the Clearing's 20,000 cr trading advance. At the Tycho Mutual
agency they charter a Dray-class bulk hauler from Tycho's loaner pool, the *Salt Moth*, berthed on the
Dock Ring. At the Occator exchange they buy 1,400 t of volatiles into the Moth's hold and 800 t of
propellant (both legs: Stickney's is dear); at the Co-op chandlery they buy ten crew-days of provisions.
They board, take the helm, and undock for Stickney Yards. The burn commits 400 t and three crew-days
before the Moth casts off; some IC days later (how many is Q-18-2b: doc 10 1A's "weeks" is the default,
about 24 IC days and one real day at the recommended world rate; three IC days and three real hours is
the playtest alternative) she passes Kirkwood Depot and arrives Direct on the Stickney dock. They
disembark onto the Yard Gantry, walk to Kasei Metals' receiving floor, and sell the volatiles into
Kasei's standing bid for 21,000 cr, ending the run at 22,950 cr with 400 t of propellant in the tank for
the trip home and 20,000 cr plus the charter note owed. Every act is the player's own verb or
boot-applied content; no staff command and no NPC stands on the path.

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
(11 §10). All three designs and all three judges converged on it.

**Disposition of the two names (Q-18-1).** Tycho is honoured as the ship desk: a Tycho Mutual agency
stands in both station maps, and the starter hull comes from Tycho's loaner pool. Pallas is honoured as
a thread: a `pallas` body entry (orbit 2.773 AU) so distances exist, a Syndicate agent at the Ceres
exchange speaking the outer-claim lore 11 §5 already seeds, and "Ceres–Pallas" named as the first
off-corridor extension. Swapping port B to Pallas later costs one location, one station pack and one
route pair, all data.

---

## 2. The run, beat by beat

Verbs whose status is **new** do not exist today; their names are proposals. Evidence is file:line at
`f4b14f3b`. "cr" is `tacsim.credit`, scale 2; every money line below is rendered in major units
(today's verbs parse and print minor units, `crates/mush-economy/src/commands.rs:2376-2380`).

| # | Step | The player types | The player sees | Mechanism | Status | Evidence |
|---|---|---|---|---|---|---|
| 1 | Log in, make a pilot | `ssh play@host` · `create Vex <pw>` · optionally `+chargen; +stat …; +chargen/submit` (staff `+job/approve`) | Limbo #0 with `OOC Area <O>`, `Staff <S>` and `Ceres Freeport <C>` | Gateway creates account+character; world task creates the Player in Limbo, home Limbo; chargen gate inert while `chargen_required=false` (dev1) | exists | `mush-gateway/src/accounts.rs:135-148`; `mush-server/src/world_task.rs:9228`; `tacsim/src/sheet/onboarding.rs:107,205-208`; `deploy/dev1/instance.env` |
| 2 | The IC door | `c` | Arrivals, Dock Deck, Ceres Freeport: a 2350 desc, exits to the Dock Ring and the lift | `exit.limbo_arrivals` from `limbo` in the new `neighborhood.ceres_freeport` pack (the OOC/Staff packs' own pattern); no `requires` on any starter pack, so no starter digest moves; with chargen on, lock `flag^tacsim:approved` on an **unregistered** marker set at approval | new | `mush-atlas/src/def.rs:23` LIMBO_REF; `mush-core/src/lock.rs:266-274`; registered markers are self-settable `mush-core/src/commands/building.rs:459-464` |
| 3 | Walk to Ceres Clearing | `ring; lift; clearing` | The Clearing hall; a teller NPC; a board naming the advance | Atlas rooms on structure floors; stair portal with `portal.at`+`dz_m`; rooms `kind indoor` so no weather | new (content) | `mush-atlas/content/neighborhood.staff_lobby.json` exit.stairs precedent; `mush-atlas/src/apply.rs:1088-1090` |
| 4 | Open a wallet | `+credit/open` | "Ceres Clearing opens account #41 in your name, in cr. Balance 0.00 cr." | Presence-gated to the port entry's clearing room (resolved by pack entry through the latch); `open_account(Object(actor), tacsim.credit, CreditPolicy::forbid())`, create-only per (owner, instrument); durable ack | new | `mush-economy/src/api.rs:1036`; `tacsim/src/match_/centre.rs:122-128` latched_object precedent; `mush-economy/src/commands.rs:1306` ack precedent |
| 5 | The credit line | `+credit/advance` | "Ceres Clearing advances 20,000.00 cr on the Freeport's new-operator line. You owe 20,000.00 cr; terms not yet scheduled." (No Tycho cover yet: the charter is step 7.) | Deterministic eligibility (open account, no live advance); disbursal is `post` with actor `Organization(ceres_clearing)` on Clearing's own scoped account (synchronous, the recovery stipend precedent); the debt is `create_obligation` with the pilot as actor, creditor Clearing, cause `Ruleset{tacsim}`, `EconomicClock::Manual` (charges nothing until terms are ruled) | new | `mush-economy/src/api.rs:370-375` org acts on own compartment; `mush-campaign/src/recoverycmd.rs:621-661`; `api.rs:1968-2035` create_obligation; `clock.rs:64-74` Manual |
| 6 | See the wallet | `+credit` | "Account #41: posted 20,000.00 cr, available 20,000.00 cr. Owed: 20,000.00 cr to Ceres Clearing (terms not yet scheduled)." | Plain read of `EconomyStore` (`accounts_of`, `obligations()` filtered by debtor) in major units | new | `mush-economy/src/snapshot.rs:112-117,246`; `record.rs:198-205` |
| 7 | Charter the starter ship | `concourse; tycho; +ship/charter Salt Moth` | "Tycho Mutual charters you the Dray-class hauler *Salt Moth* (registry FPC-D-0417), cradled at the Dock Ring. Hold vault #212 opened in your name. Charter: 1,500.00 cr per IC week to Tycho Mutual, terms not yet scheduled." | Presence at the Tycho room; one charter per pilot; the lowest-id free hull of Tycho's seeded pool (six Drays, titled to the Tycho Mutual org by `tacsim.registry`); the charter is a tacsim record `{hull, pilot, name, since}`; `create_vault(SpatialInterior(bay), owner Object(pilot))`; a Manual-clock charter-fee obligation. No hull is minted (11 §4.7 "never a mint") | new | `mush-campaign/src/recovery.rs:935-948` lowest-id loaner precedent; `mush-economy/src/api.rs:1051,2498`; `mush-sim/src/custody.rs:43-50` |
| 8 | Check the ship | `+ship` | "*Salt Moth* (Dray): docked Ceres Freeport, Dock Ring. Hold #212: cargo 0/2,800 t · propellant 0/1,200 t · provisions 0/480 crew-days. Drive coast_hauler." | Reads the charter record, `pose_of`/JourneyBook, and the hold's pools through `scan_action_state` as the vault owner | new | `mush-sim/src/window.rs:362`; `journey.rs:511`; `mush-inventory/src/commands.rs:88-107` |
| 9 | Buy cargo and fuel | `concourse; occator; +market/list; +market/quote 7=1400; +market/accept 31; +market/quote 8=800; +market/accept 32` | "Volatiles, bulk 7.00 cr/t · Propellant 10.00 cr/t"; quote 1,400 t = 9,800.00 cr; "1,400 t of volatiles are loaded into the *Salt Moth*'s hold. Occator's factor countersigns." 800 t propellant 8,000.00 cr | Existing lot path with four fixes: venue defaults to this port and destination to the pilot's berthed hold; vendor accepts through a content-resolved party cap (the listing is a standing offer signed by the org's officer of record, §4.3); `Supply::Stock`+`Representation::Lot` allowed, with a new `LotOp::Issue` minting into the buyer's endpoint at activation (`Lineage::Origin{contract}`), listing quantity decremented; a `VenueGate` refuses unless the actor stands in Ceres rooms and the hold's hull is docked here; `deliver()` short-circuits Lot | partial | `mush-market/src/trade.rs:576-593,673-688`; `api.rs:718-735` Stock+Lot refused today; `module.rs:386-412`; `tacsim/src/composition/mod.rs:361` empty party caps; `mush-inventory/src/record.rs:228-233` Origin |
| 10 | Buy food | `chandlery; +market/quote 9=10; +market/accept 33; +ship/hold` | 10 crew-days provisions 250.00 cr; "Hold #212: cargo 1,400/2,800 t · propellant 800/1,200 t · provisions 10/480." `+credit` shows 1,950.00 cr | Same path; `provisions` is an opaque inventory commodity (no instrument); capacity enforced at accept through the hull's logistics | partial | `mush-inventory/src/provider.rs:273-290` opaque ids; `mush-market/src/commands.rs:960-963` commodity=code |
| 11 | Board | `concourse; ring; +ship/embark` | "You cycle the ring lock into the *Salt Moth*." `look` shows the Bridge | Actor must stand in the presentation room of the domain the hull is Direct on and hold the charter; `mush_sim::place_interior(actor, bridge)`; an Interior-placed occupant rides the subtree and cannot walk any exit, so the ship is sealed in flight with no door logic | new | `mush-sim/src/window.rs:324`; `lib.rs:263-276`; `tacsim/src/match_/cockpit.rs:1943-2076` arena BOARD precedent |
| 12 | Undock | `+ship/undock stickney` | "Clearance granted. 400 t propellant and 3 crew-days committed to the burn. *Salt Moth* leaves Ceres Freeport for Stickney Yards by Kirkwood Depot; arrival in 24 IC days (about 24 h at the world rate)." (the duration is Q-18-2b's; 3 IC days / 3 h under the alternative) | Synchronous checks (aboard, charter, docked, authored route from this port, drive class, payload ≤ 2,800 t, propellant ≥ route.propellant, provisions ≥ crew×ceil(days)); ONE authoritative `tacsim.port.burn` action debits the hold pools and writes a typed departure ticket; the tacsim tick (normal lane) launches each burned-not-launched ticket once through a new `pub mush_sim::depart` (authored-route arm only), marks it launched, and boot reconcile re-launches stragglers | new | `mush-inventory/src/pool.rs:306,1092`; `tacsim/src/composition/mod.rs:404` quantity delegates; `mush-sim/src/commands.rs:2654` journey_depart (private), `:2268-2320` authored arm; NOT via `ActionStateChanged` (observers may not persist, `mush-core/src/engine.rs:2939-2942`) and NOT by `submit_authoritative` from a GOD tick (`mush-campaign/src/tick.rs:31-35`) |
| 13 | Transit | `+ship` · `+travel/status` | "Leg 1 of 2, cruising for Kirkwood Depot; ETA 13 d 16 h IC (13 h 40 m real); log off and the Moth keeps flying." | CarrierJourney steps on the world IC clock (`leg_due_at` = departed + Σ `base_s`); subtree sealed; the atlas `travel.status` ability already answers aboard a hull; `+travel/status` gains the `aboard_carrier` arm; `+ship` prints the ETA as a countdown in IC and real time and says the passage continues while the pilot is disconnected (the Bridge has no exits and no one aboard, so the flight is a log-off-and-return experience at any play rate) | exists / partial | `mush-sim/src/journey.rs:1021-1087,398-412`; `mush-atlas/src/interface.rs:229-242`; `travel_cmd.rs:285-312` |
| 14 | Customs at Kirkwood | (nothing) | "Kirkwood Depot slides past: the Co-op's tank farm, a Republic picket's transponder asking your manifest. Leg 2 of 2." | Leg-boundary narration to onboard occupants (new, S). No hold: the leg's interdiction band stays 0, no domain stands at Kirkwood, no atlas checkpoint exists; pickets are a follow-on (§4.4) | new (narration only) | `mush-sim/src/journey.rs:1088-1172`; `tacsim/src/content.rs:320-340` lossy atlas copy |
| 15 | Dock | (nothing) | "*Salt Moth* arrives, and the passage ends." The hull appears on Stickney's Yard Gantry; the pilot in the Bridge reads the same line | `apply_arrival` places the root Direct on the Active domain hosted by the `stickney_yards` location room, presented into the atlas Yard Gantry room. Today's arrival line is `to_room_from(to, …)` on the destination room (`journey.rs:1396-1409`): a Room audience resolves to `contents_of(room)` (`world_task.rs:9344-9362`) and an Interior-placed pilot's location is the Bridge (`graph.rs:141-167`), so nothing reaches the Bridge; IC7 adds an Interior-occupant audience at BOTH the leg boundary and `apply_arrival` | partial | `mush-sim/src/commands.rs:2207-2256`; `journey.rs:1340-1420`; `window.rs:195` open_passive_domain; `tacsim/src/content.rs:547-572` applier-opens-domain precedent |
| 16 | Disembark | `+ship/debark` | "You step through the lock onto the Yard Gantry." The station is walkable | Refused while under way; else `mush_sim::unplace` then `move_to(presentation room)`; avoids the windowless-domain strand that `egress` would cause | new | `mush-sim/src/window.rs:346`; `mush-atlas/src/window.rs:1944` unplace-on-leave precedent; `lib.rs:263-276` |
| 17 | Sell | `spine; kasei; +market/list; +market/sell 14=1400` | "Kasei Metals buys 1,400 t of volatiles at 15.00 cr/t: 21,000.00 cr. Kasei's receiving clerk countersigns." `+credit` shows 22,950.00 cr; the volatiles lot is gone | A `Supply::Wanted` listing (vendor pays per unit, bounded quantity, funded account); `+market/sell <listing>=<qty> [from=<vault>]` (new lot arm): VenueGate, synchronous source-quantity check, contract with the vendor's consideration, vendor accepts by party cap (escrow under Kasei's account), seller accepts, staged `LotOp::Retire` (dissolve loose lots, debit), bid quantity decremented, settlement pays out | new | `mush-market/src/trade.rs:931-1085` object-only sell today; `lot.rs:160-195` LotOp set; `api.rs:556` fund_from_faucet |
| 18 | Repayment | `+credit` | "Owed: 20,000.00 cr to Ceres Clearing; 1,500.00 cr/IC week to Tycho Mutual (terms not yet scheduled)." | Both obligations sit on `EconomicClock::Manual` in this slice; the first post-loop cut adds `Obligation.max_periods` and a declared `tacsim.week` cadence and the debts start charging | deferred | `mush-economy/src/obligation.rs:660-667`; `config.rs:124-141` DeclaredCadence; no cadence declared anywhere (`deploy/instance/mushd.toml.tmpl` has no `[economy.cadences]`) |

Money check: 20,000 − 9,800 − 8,000 − 250 + 21,000 = 22,950. The economics are **per round trip**, not
per leg: only Stickney has a buyer (Ceres' vendors carry `bids: []`), so the return leg earns nothing and
its fuel and food are bought at Ceres before the outbound burn. At these prices a round trip of q tonnes
nets 8q − 8,250 (margin 15 − 7 per tonne; 800 t of propellant at 10 plus ten crew-days at 25):
break-even 1,032 t, +2,950 cr at the transcript's 1,400 t, and at most +5,174 cr at the 1,678 t the
advance can fund (the hold's 2,800 t is never the bound). Fuelling both legs at Ceres saves
(24 − 10) × 400 = 5,600 cr, so fuel is a bigger decision than cargo (§4.3). The first draft's "+2,700 a
leg" counted a leg that cannot earn and its 1,000 t run was a 650 cr loss per round trip.

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

### B. Wallet and the initial credit line

- **Exists.** `tacsim.credit` create-only at boot (`tacsim/src/economy.rs:53-79`); per-account `CreditPolicy`
  enforced on posted and available balances (`engine.rs:630-642,668-690`); invariant 26: a hold needs
  non-negative available funds (`engine.rs:691-697`); market purchases escrow a hold
  (`contract_live.rs:1934-1947`); `open_account` public, no validation but zero balance (`api.rs:1036-1047`);
  `create_system_obligation` is the in-code faucet, no actor check, capped **per obligation** (the amount
  ceiling, `spec.amount > cap`, re-asked at creation and at each settlement; not an aggregate per period)
  (`api.rs:1725-1818`, `:1766-1773`, cap `config.rs:299-324`, 50,000 default `mushd.toml.tmpl:162`); `create_obligation`
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
- **Missing.** An NPC buyer for a lot; a presence gate (`remote` only changes TTL, `commands.rs:431-446`); a
  stock seed path; restock; a food model; a departure debit.
- **Corrections.** (1) Lot reservations are **never expired** in production while listings default to 14 real
  days against a 7-IC-day hold, and `is_current` never checks the hold: a late accept takes the money and
  delivers nothing (`mush-inventory/src/record.rs:33-35`, `mush-market/src/commands.rs:1001-1008`,
  `trade.rs:90-131`; `lot.rs` has no compensation). (2) Endpoint choices are capped at 64 per actor and staff
  may take every vault, so staff seeding breaks past ~64 vaults (`mush-inventory/src/provider.rs:63,187-206`).
  (3) Grade is seller-declared and `dissolve` drops it (`pool.rs:1092,1216-1245`). (4) A container placed in a
  sim interior is sim-bound and cannot back a container vault (`mush-sim/src/data.rs:1241-1248`).

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
  (`mushd.toml.tmpl:197-199`).
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

---

## 4. The design

The spine is the thin design (first playable earliest, one new pack kind, no staff act), with the
fiction-first design's institutions, defaults and pricing and the engine-integrity design's blockers-first
cut and three cheap guards grafted on. Where the judges disagreed the text says which way it goes and why.

### 4.1 Money

**The wallet.** `+credit/open` (new, tacsim) opens the pilot's `tacsim.credit` account with
`CreditPolicy::forbid()`, create-only per (owner, instrument) because the market's `principal_account`
takes the first active match (`mush-market/src/api.rs:107-123`), presence-gated to the port's clearing
room. `+credit` renders posted, held, available (`available − floor` when a floor exists) and every
obligation where the pilot is debtor, in major units. It is a port of the retired `+nuyen` (`56e877c4`),
not a design. No approval hook: dev1 runs chargen off, and the desk works either way (D-18-6).

**Institutions, not faucets.** The fidelity judge was right that a System-principal "Ceres Clearing" is
content the polity cuts cannot attach to. The port entry seeds real orgs: **Ceres Clearing**, **Tycho
Mutual**, **Occator Water & Propellant**, **Belt Carriers' Cooperative** (the chandlery), **Kasei Metals**.
Each is `mush_org::submit(CreateOrg)` plus a root scope and a scoped `tacsim.credit` account, the shape
`mush-faction/src/econ.rs:391-447,851-900` already runs in production. Each carries an optional `faction`
symbol the faction cut resolves later. One System faucet, `system:tacsim.treasury`, is opened create-only
at port apply with `open_account(System owner, CreditPolicy::unbounded())`, the SR `ensure_system_account`
shape. Its fiction is **the engine's world-start capitalization** (7A's first funding source), not the
Concord's money: doc 11 §3.1 keeps the Concord treasury "large and unavailable", and Clearing "owes
Luna" (11 §4.8) is a debt, not a grant. Capitalization is one one-shot `create_system_obligation` per
org (actor-free, settles on the next 30 s sweep, `clock.rs:39`). **The economy does not dedup it:**
`ObligationSpec` carries no key (`mush-economy/src/api.rs:1683-1698`) and the call allocates a fresh id
and `put_obligation`s on every call (`:1723-1818`); `AlreadyPosted` is a `post` outcome and never
applies here. The applier's own fail-closed `TacsimStore` map (§4.7) is the sole dedup for obligation
creation, so the map row is written in the same normal-lane batch as the obligation; a keyed
`create_system_obligation` variant is listed as an IC3 engine option in §10's 08a rows. Clearing 1,000,000
cr, Tycho 200,000, Occator 100,000, the Co-op 100,000, Kasei 500,000 — **finite, 7A's list as written**
("world-start capitalization"; "Rejected: infinite system market makers with caps"). Kasei's 500,000 is
23 sales at the transcript's 21,000 cr; in fiction it is the Ceres Clearing money Kasei was built with
(11 §4.3), which is the lever the setting gives the Freeport's bank over Phobos. When a playtest drains
it, recapitalization is 7A's "attributed staff/system action": staff `@econ/mint` from the treasury
(`mush-economy/src/commands.rs:118,2603`), or a `capital_major` bump on the `port` entry re-applied
(the applier's update path never re-capitalizes, so the bump is a new keyed entry revision, §4.7). That
is D-18-14; the first draft's perpetual drip is the alternative in Q-18-9, and if the owner takes it the
drip is a **bounded Ceres Clearing facility to Kasei**: a standing obligation created through
`create_system_obligation` directly on `EconomicClock::WorldIc` (`clock.rs:62-76`) with period 86,400 IC
s and a total cap (`Obligation.max_periods`, P1's field), never through `fund_from_faucet`, which
hard-codes `RealUtc` and `period_secs.max(60)` (`mush-market/src/api.rs:570-581`) and so cannot follow
`@tacsim/time` (at rate 3600 an IC day is 24 real seconds, under the floor: 50,000 cr a minute). The
per-instrument mint cap override for `tacsim.credit` goes to 2,000,000 major in the template
(`deploy/instance/mushd.toml.tmpl:162`; `mush-economy/src/config.rs:299-324`), an attributable config
act; the cap is a per-obligation amount ceiling compared strictly at creation and at each settlement
(`api.rs:1766-1773`; a refusal pages `SettlementRefused`, `sweep.rs:466`), so no capitalization sits on
the boundary.

**The credit line's shape (D-18-5).** A loan disbursed into a positive balance. An overdraft floor is
refused: invariant 26 refuses a hold that leaves available funds below zero
(`engine.rs:691-697`) and every `+market/accept` escrows the price as a hold
(`contract_live.rs:1934-1947`), so an overdraft can never buy cargo. A faucet grant is refused: doc 10 7A
and doc 11 §4.8 make Clearing a funded lender. `+credit/advance` (new) runs deterministic eligibility (an
open account, no live advance, no Delinquent obligation), then two writes in one command: the disbursal,
`post` with actor `Organization(ceres_clearing)` on Clearing's own scoped account (WorldAuth admits an org
acting on its own compartment, `api.rs:370-375`; recovery's `post_stipend` is the precedent), and the debt,
`create_obligation` with the pilot as actor (lock 4: the debtor takes it on, `api.rs:1940-1985`), debtor the
pilot's account, creditor Clearing's account, cause `Ruleset{tacsim}`, clock `EconomicClock::Manual`. A
refused obligation reverses the disbursal before replying. Synchronous, so `+credit` shows the money at
once (the thin design's faucet one-shot would have shown 0.00 cr for up to 30 s).

**Repayment (D-18-7).** Deferred. The advance and the charter fee sit on the Manual clock and `+credit`
says "terms not yet scheduled". The first post-loop cut (P1, §5) adds `Obligation.max_periods` checked at
the branch that already flips one-shots to Satisfied (`obligation.rs:660-667`), a declared `tacsim.week`
cadence contributed by `config_ext` (`world_ic`, period 604,800; no cadence exists anywhere today), and
re-points both debts: advance 20,000 over 20 periods of 1,100 cr (10% flat, since doc 08 §20 defers
interest), charter 1,500 cr per week — 2,600 cr per IC week in all. **Sized against the hold-and-advance
ceiling** (§2 money check): a round trip at the transcript's 1,400 t nets 2,950 cr and the most the
advance can fund nets 5,174, so the first draft's 2,200 + 1,500 = 3,700 a week put every pilot into
default by construction. The terms are also conditional on Q-18-2b: at 1A's "weeks" a round trip is
some seven IC weeks, so P1 must express the instalment per round trip (a `tacsim.run` cadence advanced
by the arrival) or per IC month, not per week; the weekly figures above are the three-day alternative's.
Default consequence then: Clearing refuses a second advance and
Tycho refuses a renewal while any obligation is Delinquent (a tacsim `EconomyObserver` mirroring
Grace/Delinquent, the housing precedent `mush-housing/src/econ.rs:158`). No seizure.

**What refuses when broke.** `+market/accept` with insufficient available funds answers the market's
own "You have no account this market can be paid from" or the escrow refusal; `+credit/advance` twice is
refused; `+ship/undock` with an unfunded burn is refused before anything moves. Nothing minted on demand.

**Numbers.** Advance 20,000.00 cr; charter 1,500.00 cr/IC week. Run: −9,800 cargo (1,400 t at 7),
−8,000 propellant (800 t at 10, both legs), −250 provisions, +21,000 sale (1,400 t at 15) = 22,950.00 cr;
20,000 + the charter owed. Per **round trip** (the only unit that earns, since Ceres has no bid):
8q − 8,250 cr for q tonnes; +2,950 at 1,400 t; break-even 1,032 t; ceiling +5,174 at the 1,678 t the
advance funds. The return leg carries nothing sellable in this slice; a Ceres-side bid (Occator or the
Co-op buying Kasei's structural alloys) is the P5 way to make both legs earn.

### 4.2 The ship

**Object model (D-18-8).** One hull Thing is the carrier root (`sim.participant`, placed Direct on the
port domain). Its MUSH owner is the content owner, never the pilot, so `perm::controls` cannot bypass the
port verbs (`mush-sim/src/commands.rs:2606-2610`, `mush-core/src/perm.rs:53-65`). It carries `tactics.unit`
plus `unit:<obj>` bound from `tacsim.unit.dray`; the trade slice reads nothing from the unit document except
`logistics`, so the TacticalUnit is authored for later and costs nothing now. Two Rooms carry
`sim.interior`, each anchored `SpaceAnchor::MovingObject{host: hull}` through `Transition::Space`
(`transition.rs:593-608`): the **Bridge**, where crew are placed Interior and ride the subtree sealed
(an Interior-placed actor cannot take any exit, `lib.rs` Interior arm, and multi-room interiors are
reserved, `data.rs:832-835`, so one boardable room), and the **Cargo Bay**, unboardable, which backs
exactly one economy Vault (`CustodyRef::SpatialInterior(bay)`) owned by the charterer. That vault is the
inventory endpoint for cargo, propellant and provisions; its pools and lots move with the hull because
they name only the vault (`mush-inventory/src/record.rs:84-96`). The bay is separate from the bridge so
`interior_contents` (`mush-sim/src/custody.rs:68-104`) never counts the crew as custody (the fidelity
judge's point against the fiction-first design). Title is an `EconomicAsset` keyed by the hull, issued at
pack apply to the **Tycho Mutual org** under a new `tacsim.registry` issuer (`composition/mod.rs:123-146,322`).
There are no stations or billets in this slice (D-18-9); the operate gate is the charter record. Why the
charter record and not `EconomicAsset.assignment` (which the economy already reads as standing beside
title, `mush-economy/src/commands.rs:549`): an assignment on a Tycho-titled hull is written by the title
owner's own act, so a pilot verb would have to post it as the Tycho org (an org actor on an asset it did
not just accept a contract for, a new authority), it carries no name, fee or registry mark, and it would
be read as standing by every economy verb, not only by `+ship`; the charter record is read by the four
`+ship` verbs and nothing else, which is the placeholder's whole point (P2 replaces it with the lease).
Why no station: C2A–C4A and doc 10 §18.1 ("ownership alone never satisfies reach or control") make
control a station's channel bundle reached from a console, and F3 made that concrete (stations grant
channels, departure requires `movement`, `commands.rs:2652`). This slice **bypasses that gate for the
hull's departure until P2** (listed under "amends" in §7): a new `pub mush_sim::depart` reached only
from `+ship/undock`, whose checks are Interior-in-Bridge plus the charter. The stand-in is acceptable
because the hull has exactly one boardable room, one charterer and no other crew, so "in the Bridge and
chartered" and "seated at the one helm station with `movement`" admit the same person; the cheaper P2
subset (declare one helm station at spawn, grant `movement` on `+ship/embark` through
`request_seat_activation`) removes the amendment and the bypass and is the alternative in Q-18-4.

**Acquisition (D-18-10).** Tycho's loaner pool: six Drays seeded by the port entry's `charter` block, titled
to Tycho, placed on the Ceres dock domain. `+ship/charter <name>` (new) at the Tycho room takes the
lowest-id uncharted hull (the recovery provider's `next_loaner` rule, `recovery.rs:935-948`), writes a
tacsim charter record `{hull, pilot, name, registry_mark, since}`, opens the hold vault, and records the
weekly fee as a Manual-clock obligation to Tycho's account. No hull is minted (11 §4.7: Tycho's pool is
"never a mint"); an empty pool queues. Title stays with Tycho ("no sale", 11 §4.7). The charter record is
an explicit placeholder for `Encumbrance::Lease` through the fixed recovery provider (P2), at which point an
`OperatorAuthority` seam in mush-sim reads the lease and the `+ship` verbs stop reading tacsim state. This
is the buildability judges' default; the fidelity judge's lease-now path is the alternative in Q-18-4.

**The hold as an endpoint.** `create_vault(SpatialInterior(bay), owner Object(pilot))`; the vault id is
printed by `+ship` and is the default `to=`/`from=` of every market verb at a port (D-18-12). Capacity:
cargo ≤ `logistics.hold_t`, propellant ≤ `tank_t`, provisions ≤ `provisions_crew_days`, enforced at
`+market/accept` by a tacsim `CapacityProvider` consulted by the market's lot leg (pools and vaults are
unbounded today, `record.rs:837-852`). The hold is **sealed while under way**: `InteriorCustodyProvider`
answers `sealed()` for a SpatialInterior backing whose hull is under way, and `@inv/move`, `@vault/*` and
the market refuse it; scoped to that backing only (the engine-integrity judge's graft, narrowed after the
buildability judge's warning about a global predicate).

**Propellant and provisions as consumables (D-18-11).** Both are inventory pools in the hold, bought like
cargo and spent by the burn at undock. Not a `ScalarCell` fuel cell beside a pool (one tonne would have two
records, PRD-4 again) and not an off-lane unit cell (E7k). Burn: `route.propellant()` tonnes (400 each
way) plus `crew × ceil(IC days)` crew-days (3). The burn is settled **at undock, matter first** (the
buildability judge's default; T2 forbids economic transactions inside a combat pulse and a departure is
in no pulse), through one authoritative `tacsim.port.burn` action the pilot's own command submits
(tacsim joins `with_quantity_delegates`, `composition/mod.rs:404`; `InventoryFamilies::dissolve` then
`debit`, `pool.rs:1092,306`), which also writes a typed departure ticket `{hull, route, seq, burned_at,
launched: false}`. The launch is **not** done in an `ActionStateChanged` observer (that seam may not
persist, `engine.rs:2939-2942`) and **not** by `submit_authoritative` from a GOD tick (one submission per
actor per drain, `mush-campaign/src/tick.rs:31-35`): the tacsim `reg.tick` (normal lane, module identity
tacsim) scans its own tickets, launches each burned-not-launched one through a new `pub
mush_sim::depart(ctx, hull, route_id)` extracted from `journey_depart`'s authored arm, and marks it
launched in `TacsimStore` in the same normal-lane write; boot reconcile launches any burned-not-launched
ticket. Latency is one tick; a hull never flies on fuel it did not burn. The arrival-fact alternative is
Q-18-7.

**The helm.** In this slice `+ship/undock` requires the pilot to be Interior in the Bridge and to hold the
charter; there is no station, which is the C2A–C4A/F3 bypass named above and in §7. P2 adds `stations`
to the unit spec, `declare_station` at spawn, and `request_seat_activation` from a `+ship/helm` verb,
granting both `movement` (the journey gate, `commands.rs:2652`) and `helm` (the arena vocabulary,
`arena/state.rs:3187`), after which `+ship/undock` goes through `journey_depart`'s own channel check
and the `pub depart` bypass is deleted.

**The Dray definition.** `crates/tacsim/content/units/dray.json`, shipped in `fielded_sources` with a digest
pin in `tests/content.rs`. Everything below is valid against today's schema except `class: "hauler"` and
`logistics`, both added as optional fields with `default` + `skip_serializing_if` so no shipped
`DefinitionRef` moves (`unit.rs:822-847`). Numbers are placeholders for a tuning pass.

```json
{
  "manifest": {
    "id": "tacsim.pack.dray",
    "version": 1,
    "kind": "rules_definition",
    "digest": "<computed by crates/tacsim/tests/content.rs; pinned there>",
    "requires": []
  },
  "entries": {
    "tacsim.unit.dray": {
      "kind": "unit",
      "def": {
        "label": "Dray-class bulk hauler",
        "class": "hauler",
        "sections": [
          { "id": "hull", "facets": [
              { "id": "front", "armor": 120 }, { "id": "left", "armor": 80 }, { "id": "right", "armor": 80 },
              { "id": "rear", "armor": 60 }, { "id": "top", "armor": 60 } ],
            "internals": 600, "consequences": [ { "kind": "destroy_unit" } ] },
          { "id": "hold", "facets": [
              { "id": "left", "armor": 60 }, { "id": "right", "armor": 60 }, { "id": "top", "armor": 50 } ],
            "internals": 400, "consequences": [ { "kind": "slow", "percent": 75 } ] },
          { "id": "tankage", "facets": [], "struck_through": { "section": "hull", "facet": "left" },
            "internals": 150, "consequences": [ { "kind": "slow", "percent": 50 } ] },
          { "id": "drive", "facets": [], "struck_through": { "section": "hull", "facet": "rear" },
            "internals": 250, "consequences": [ { "kind": "disable_motion" } ] },
          { "id": "bridge", "facets": [], "struck_through": { "section": "hull", "facet": "front" },
            "internals": 120, "consequences": [ { "kind": "destroy_core" } ] }
        ],
        "components": [
          { "id": "power", "section": "hull",
            "role": { "kind": "power", "capability": "tacsim.power",
                      "states": [ "off", "starting", "online" ], "initial": "off" } }
        ],
        "mobility": {
          "tracked": { "max_speed_cms": 0, "accel_cms2": 100, "brake_cms2": 200,
                       "turn_rate_mdeg_s": 1000, "reverse_factor_pct": 0 }
        },
        "facets": [
          { "band": "front", "sections": [ { "section": "hull", "weight": 70 }, { "section": "bridge", "weight": 30 } ] },
          { "band": "left",  "sections": [ { "section": "hull", "weight": 50 }, { "section": "hold", "weight": 40 }, { "section": "tankage", "weight": 10 } ] },
          { "band": "right", "sections": [ { "section": "hull", "weight": 50 }, { "section": "hold", "weight": 50 } ] },
          { "band": "rear",  "sections": [ { "section": "hull", "weight": 50 }, { "section": "drive", "weight": 50 } ] },
          { "band": "top",   "sections": [ { "section": "hull", "weight": 40 }, { "section": "hold", "weight": 60 } ] }
        ],
        "silhouettes": [
          { "band": "front", "width_cm": 2800,  "height_cm": 2200 },
          { "band": "left",  "width_cm": 11000, "height_cm": 2200 },
          { "band": "right", "width_cm": 11000, "height_cm": 2200 },
          { "band": "rear",  "width_cm": 2800,  "height_cm": 2200 },
          { "band": "top",   "width_cm": 11000, "height_cm": 2800 }
        ],
        "sensor_height_cm": 2000,
        "logistics": {
          "hold_t": 2800,
          "tank_t": 1200,
          "provisions_crew_days": 480,
          "crew": 1,
          "crew_max": 4,
          "drive_class": "coast_hauler",
          "thrust_permille": 1000,
          "law": "coast"
        },
        "note": "Stickney-built bulk hauler (doc 11 section 7): slow, efficient, unarmed. The port verbs read logistics (hold, tank, provisions, drive class) and nothing else; the combat anatomy is authored for later and never fielded. class hauler is filtered out of SIM UNIT. The tracked mobility is the immobile stand-in until a thrust mode exists. No core component: a hull with no core is Class 0, the drive ROM (doc 17 section 9), nothing separately titled and no licence question. thrust_permille is 1000 because route_duration scales every leg by 1000/thrust (mush-sim route.rs:794-836); the authored base_s IS the Dray's time."
      }
    }
  }
}
```

**Schema additions.** (1) `UnitClass::Hauler` (`unit.rs:726-733`), excluded from arena class caps.
(2) `UnitSpec.logistics` (optional), read by tacsim from `TacsimStore.definitions()`; `bind()` ignores it.
(3) `fieldable_ids` filters `hauler` (`lobby.rs:1600-1619`); a custody-seam refusal test guards the arena
(any reader of the shared catalog must honour the filter, the buildability judge's note). (4) `tacsim.registry`
title issuer. Not added now: a thrust mobility mode (needs a rules schema id, a medium per mode, and the
arena's `CostRow` defaulting unknown rows to Tracked, `arena/state.rs:115-121`), `ComponentRole` drive/hold/
tank/life_support (doc 10 §8's shape; the alternative in Q-18-5), `stations` and `interiors` fields (P2).

**Title and the lease/lien.** Title to Tycho by `issue_title` (idempotent per generation, `api.rs:2488-2490`).
No lien (staff-only to place, org creditor cannot release, `api.rs:422-440`). No `Encumbrance::Lease` until
P2. Invariant 12 is moot while Tycho holds title.

### 4.3 Goods

**The resource subset (D-18-13).** Three commodities, two from the ten families and one aggregate:

| Commodity | Identity | Role | Ceres | Stickney |
|---|---|---|---|---|
| volatiles | family `volatiles` (`tacsim.volatiles` instrument registered; inventory id `volatiles`; catalog `tacsim:volatiles`) | the trade good, Occator water to Kasei's refinery | **Occator** ask 7.00 cr/t, 200,000 t on the listing | **bid** 15.00 cr/t, 50,000 t, Kasei's finite account |
| propellant | family `propellant` | fuel: 400 t burned per leg at undock; also cargo | **Occator** ask 10.00 cr/t (refined from its own volatiles by electrolysis, 11 §4.1, so priced above them, 4A) | ask 24.00 cr/t (Kasei refines imported volatiles bought at 15; Republic prices, 2.4× Ceres) |
| provisions | **opaque inventory commodity**, catalog `tacsim:provisions`, unit = crew-day, **no instrument** | the food answer at doc 10 §9's endurance scale; `crew × ceil(IC days)` burned per leg | **Co-op chandlery** 25.00 cr/crew-day | 30.00 cr/crew-day |

Occator is "Water & Propellant" and the port's baseline supplier of both (11 §4.1); the Co-op is the
hauling line that runs Kirkwood (11 §4.5), so its Ceres chandlery sells stores, not Occator's reaction
mass. The first draft gave propellant to the Co-op at 8 under volatiles at 9, a refined family cheaper
than its raw input at the port that refines it.

Catalog codes equal family ids, so `@market/vendor stock` needs no `commodity=` (`commands.rs:960-963`).

**The three representations reconciled.** The economy instrument is the unit of account and stays
orphaned by the lot path; matter is lots and pools (Q13A: quantities are neither containers nor balances);
the catalog entry is the price reference. For `provisions` only two of the three exist (no instrument), an
accepted deepening of PRD-4 for one aggregate consumable until the owner rules on an 11th family
(Q-18-3). `FAMILIES` stays `[&str; 10]` (`tacsim/src/definitions/resources.rs:34`).

**The chandlery and the exchange as content.** Each port's `vendors` block (§4.7) declares an org, a
storefront container item in the atlas pack (the vendor vault's `Container` backing; a container in a sim
interior cannot back a vault, `data.rs:1241-1248`, so storefronts are plain room items), a venue
(`ceres`, `stickney`), asks and bids. The catalog (`crates/tacsim/content/market/goods.json`, a compiled-in
`Vec<CatalogEntry>`, baseline in centicredits) is loaded once by the port applier through
`mush_market::api::load_pack` (`api.rs:253-257`; no `CatalogSource` is needed for an applier, only for
`@market/catalog load`).

**Where goods come from and go to (D-18-15, Q-18-8).** The judges split. Judges 1 and 3 default to the
thin design's **listing-bounded issue and retire**: an ask is `Supply::Stock + Representation::Lot` with a
new `LotOp::Issue` that mints the bought quantity into the buyer's endpoint at activation with
`Lineage::Origin{source: the contract}` (the arm `record.rs:228-233` already documents as "a delivery")
and decrements the listing; a bid is `Supply::Wanted` with a `LotOp::Retire` that dissolves and debits
the seller's endpoint. Judge 2 defaults to a one-time content endowment (`Lineage::Origin{seeded fixture}`)
plus industry restock. This document goes with issue/retire for the slice: it needs no boot-time
`ActionTxn` mint, no restock sweep, never touches the 64-endpoint Choice cap or the lot-hold TTL on a
shelf, and conservation returns when industry closes the corridor (P5). The cost is an amendment to the
E7 "market never mints" pins (`mush-market/tests/e7r2.rs:315-332`), surfaced as Q-18-8 in its own words:
ports as bounded economic boundaries.

**Per-port prices and maker budgets.** Sell side: each listing carries its own price under its venue
(`record.rs:608-610`), so Ceres cheap and Stickney dear is two listings by two orgs (one vendor per org,
`mush-market/src/store.rs:334-336` `vendor_of_org`). Buy side: `Supply::Wanted` carries price and quantity
per venue, paid from the vendor's funded account; maker budgets (`MarketSource`) are not used in the slice.

**The Warrant principal.** Doc 11 §2's Warrant says no automated system may contract or sign and every
automated act has a liable principal; doc 11 §4 has NPC principals speak for the orgs. A port listing is
therefore a **standing offer signed by the org's officer of record** — the Occator factor, the Kasei
receiving clerk, named in the `vendors` block (`signatory`) — who is the Warrant principal for every
acceptance under it; the engine's party-cap acceptance executes that standing offer on the org's own
debit authority and confers no new one (Q-18-10). The accept line prints the name ("Kasei's receiving
clerk countersigns").

**Presence (D-18-12).** `MarketModule::with_venue_gate(Arc<dyn VenueGate>)`, consulted at quote, accept and
sell with (actor, venue, endpoint): tacsim answers from the port map (actor in the port neighborhood's
rooms; the endpoint's backing → bay → `MovingObject` host → hull Direct on that port's domain).
`+market/depth` stays readable anywhere (the public board). With no argument, `market=` defaults to the
port the actor stands in and `to=`/`from=` to the pilot's berthed hold (the fiction-first graft).

**Food ruling.** Provisions as above; no 11th family, no engine change, matches doc 10 §9 ("endurance,
not meals"). The 11th-family alternative and "no food" are in Q-18-3.

### 4.4 Travel

**The record (D-18-16).** The ship rides mush-sim's `CarrierJourney` over the compiled-in corridor
pack's `RouteDef`, resolved through `TacsimRouteCatalog` (`composition/sim_routes.rs:28-38`,
`store.rs:492-525`). It is the only record that moves a hull with its subtree (bridge occupant, bay, hold
vault backing) and arrives as a placement on a port domain. The atlas `JourneyRec` is untouched for
characters and the tour bus. **D-14-35 disposition:** this document removes `spacecraft` from the atlas
`SELF_MODES` in cut IC0 (`mush-atlas/src/travel.rs:89-90`; it is a cargo-free way across the corridor and
doc 14 §3.3 already says modes are filtered by what the character has) and proposes in Q-18-6 that
D-14-35 be decided as "two records, hard boundary; the route source is the tacsim pack and the atlas
connection is a derived projection" (its "one route source, the atlas" wording is false in code). The fold
(1,786 lines, 142 references, `14-spatial-implementation-plan.md:1228-1238`) is not taken.

**Verbs.** `+ship/embark` (place_interior into the Bridge), `+ship/undock <port>` (checks, burn, ticket;
authored routes only), `+ship` (status, hold, ETA in IC and real time), `+ship/debark` (unplace + move),
`+travel/status` (gains the aboard arm), `@journey/show` (staff/crew), `@corridor/show <route>`. The
non-staff manual place-token arm of `@journey/depart` becomes staff-only in IC0.

**The pinned distance and the real duration.** Ceres (2.766 AU, 140°), Kirkwood (new body, 2.06 AU,
135°: the 4:1 gap, "the inner edge of the Belt", 11 §2), Mars (1.524 AU, 130°): legs 0.74 AU and 0.56 AU,
summing to the 1.29 AU direct distance (checked). **T17 pins the distance, not the duration**: the
ephemeris fixes each leg's `distance_km`, and `route_duration_at` applies the distance law only when
asked at a distance other than the pin (`route.rs:555-560,583-588`), returning the authored time
unchanged at the pin (`:849-872`), so the first draft's "pin each leg against `route_duration_at`" was
a tautology (and, at the 600-permille profile it named, false by construction: `route_duration` scales
every leg by `1000/thrust`, `:794-836`, and `DriveProfile`'s fields are private, `:605-612`). The
content test instead pins each leg's `distance_km` to the ephemeris at the T17 epoch and `base_s` to
`distance_km / coast_km_s`, with `coast_km_s` a named constant for `coast_hauler` beside the pin, and
asserts `route_duration(route, nominal_drive, 0) == Σ base_s` at 1,000 permille (`store.rs:842-848`).
**How long the lane takes is the owner's (Q-18-2b)**, because the first draft's three IC days was an
un-cited amendment of doc 10 1A ("inner system: days to weeks; Belt: weeks"; doc 10 §3 "IC days to
months") and of doc 11 §7's Dray fiction (a hauler that "cannot outrun a Vigil", whose evasion is a
propellant decision): 1.29 AU in three days is ≈745 km/s average, faster than any torch, copied from
the test-only fixture's `fusion_torch` route (`tacsim_corridor_v2.json:67-87`, 86,400 + 172,800 s) onto
a coaster. **Default, 1A's scale:** 24 IC days, `base_s` 1,180,800 + 892,800 = 2,073,600 IC s split in
proportion to distance (≈93 km/s; a Vigil torching the same lane in a few days keeps doc 11 §7 true).
**Alternative, the playtest figure:** three IC days, 147,600 + 111,600 = 259,200 s. Live legs sum `base_s`
(`journey.rs:398-408`); payload does not vary flight time until P6. Real time = IC / world rate, one global
knob (`[game].ic_ratio` at first boot, then `@tacsim/time rate=<n>`, 1..3600): at the default, 24 days at
1, 6 days at dev1's 4, **24 h at the recommended 24** (1A's ≈ one OOC hour per IC day), 4 h at 144, 576 s
(≈10 min, leg 1 at ≈328 s) at 3600 for the scripted test; at the alternative, 72 h / 18 h / 3 h / 30 min /
72 s (leg 1 at ≈41 s) (D-18-17). A faster play rate, not a shorter lane, is the lever if the owner wants a
three-hour run. The reverse route `stickney_ceres` is authored with identical legs; sim routes are one-way.

**Docking as a mechanism (D-18-18).** Each port entry opens ONE passive domain whose **host is the
corridor location room** and whose presentation is the atlas dock room (`open_passive_domain(location,
dock)`, `window.rs:195`; the applier precedent `content.rs:547-572`). Route endpoint = origin anchor =
domain host, as departure requires (`commands.rs:2190-2203,2268-2320`), one domain per host. The hull is
placed Direct on it and projects into the dock room (`graph.rs:152-166`); arrival lands it on the
destination's domain. Boarding and leaving are placement verbs, so there are no gangway exits, no
open/close-on-dock doors, and no need for a destination seal on static exits; pack apply additionally
refuses any exit whose end is an anchored interior (the integrity graft, S). Berth slots are deferred:
every hull at a port shares the dock room and is listed there.

**What customs gets for free.** Nothing today, and nothing in this slice beyond narration at the Kirkwood
leg boundary. Atlas stake-outs need a `CheckpointRec` the `RouteApplier` does not emit
(`content.rs:320-326`), `watchable()` ignores corridor rooms (`travel_cmd.rs:449-468`), `+intercept` cannot
target a sim hull, and the sim's interdiction raise needs an Active domain at Kirkwood and an intermediate
leg with hazard ≥ interdiction (`journey.rs:1084-1145`). P4 makes the atlas copy faithful (per-leg bands,
a Kirkwood checkpoint) and adds a hold of a sim hull at a watched checkpoint.

### 4.5 The stations

**Layout template** (both ports; every room `kind indoor` or `circulation`, never outdoor, or the applier
forces OUTDOORS and weather follows, `apply.rs:1088-1090`; exit aliases avoid kernel words `l i inv ex go
move take p`, `apply.rs:1366-1372`): an arrival room on the ground surface (the Limbo door lands here at
Ceres); a dock deck with the dock room (marker `port.dock`; the domain presentation; `+ship/embark` works
here) and a customs post; a stair or lift portal (`portal.at` segment + `dz_m`) to a concourse deck with a
circulation spine and shared-edge doors to the desks (clearing, Tycho agency, exchange, chandlery, Registry),
a social room for extras and lore, and a restricted office deck (ladder, `lock: staff`) that never sits on
the public topology path.

**Port A — Ceres Freeport**, 13 rooms. `region.ceres_freeport` (surface region, body `ceres`: the tested
path; no pack or test uses an orbital region) + `neighborhood.ceres_freeport` (requires the region only).
Structure "Freeport Hub": Dock Deck 0 (6 m), Concourse Deck 1 (4.5 m), Office Deck 2 (3.5 m).

| Room | Deck | Purpose |
|---|---|---|
| `room.arrivals` | ground | the Limbo door lands here; the Luna packet's lock |
| `room.dock_ring` | 0 | marker `port.dock`; the domain presentation; Tycho's cradles; `item.arrivals_board` |
| `room.port_watch` | 0 | customs counter feature; the Berth Officer |
| `room.concourse` | 1 | circulation spine |
| `room.clearing` | 1 | Ceres Clearing: `+credit/open`, `+credit/advance` |
| `room.tycho_agency` | 1 | Tycho Mutual: `+ship/charter` |
| `room.occator_exchange` | 1 | Occator's board: volatiles and propellant; `item.occator_manifold` (storefront container); Syndicate agent |
| `room.coop_chandlery` | 1 | provisions (stores); `item.chandlery_cage` |
| `room.registry_annex` | 1 | flavour; the Registry clerk; title-vs-charter lore |
| `room.slack_line` | 1 | bar; barkeep; guarded Tether lore |
| `room.coop_hall` | 1 | Co-op hiring hall; dispatcher (extra now, principal later) |
| `room.port_authority` | 2 | restricted, `lock: staff`; principals' rooms |
| `room.council_gallery` | 2 | restricted; Wen, Varga, Ndiaye as `enabled:false` principals |

`room_budget` target 13, maximum 16.

**Port B — Stickney Yards, Phobos**, 9 rooms. `region.stickney_yards` (surface region, body `phobos` in
the atlas; the corridor location uses body `mars` for the ephemeris) + `neighborhood.stickney_yards`. No
Limbo door: reachable only by flight. Structure "Yard Spine": Gantry 0, Gallery 1.

| Room | Deck | Purpose |
|---|---|---|
| `room.yard_gantry` | 0 | marker `port.dock`; domain presentation; Vigil cutters in the desc |
| `room.guard_customs` | 0 | Orbital Guard picket post; Customs Assertion lore |
| `room.transit_spine` | 0/1 | circulation; stair portal |
| `room.kasei_floor` | 1 | Kasei Metals receiving: the volatiles bid; `item.intake_hopper` |
| `room.yard_chandlery` | 1 | propellant and provisions at Republic prices; `item.yard_store` |
| `room.tycho_desk` | 1 | Tycho's Phobos agent (returns and renewals later) |
| `room.refinery_gallery` | 1 | view over the volatiles-to-propellant line; flavour |
| `room.crew_mess` | 1 | extras; Settlement lore |
| `room.yard_office` | 1 | restricted; Castellanos, Tanaka, Bask as `enabled:false` principals |

**Sites, connection, region.** The corridor is NOT an atlas connection: the two station regions are
separate surface regions (an atlas connection is single-region and coordinates cap at 1,000 km,
`geom.rs:13`). The tacsim corridor pack's `location` entries make a `SiteRec` each in the `sol` system
region and the `route` entries a `spacecraft` `ConnectionRec` between the two location rooms
(`content.rs:160-356`); the station neighborhood and the corridor meet only through the port domain
(host = location room, presentation = dock room). **Schema addition (D-18-19):** `RoomDef.markers`, a closed
vocabulary registered by tacsim (`port.dock` now; `port.clearing`, `port.charter`, `port.exchange`,
`port.chandlery` reserved), unregistered as a player-settable flag; the validator treats a `port.dock`
room as a topology seed ("reached by carrier"), which is how Stickney passes `topology.unreachable`
(`validate.rs:1958-2000`), and the port applier refuses a dock room that lacks it. The fiction-first
design's `NeighborhoodDef.arrivals` is the alternative; markers answer the "rooms cannot declare a
function" gap too.

**Theme.** One `Theme` per region (`theme.rs:83-101`): "TacSim 2350: the Freeport" / "the Yards";
vocabulary deck, berth, lock, spinward, manifest, tare, reaction mass, volatiles, chit, Port Watch,
Registry, cr; banned warp, hyperspace, FTL, teleport, alien, android; `desc_length` medium; present
tense, no "you see", no named NPCs in room descs. Theme has no `deny_unknown_fields`, so a typo is
silently dropped; the validate test checks the fields it expects.

**Decks, berths, airlocks.** Decks are structure floors; inter-deck exits carry `portal.at` and `dz_m`.
Berths are the dock room (one per port in this slice); hulls appear there by projection. There are no
airlocks: `+ship/embark`/`+ship/debark` are the lock.

**Authoring and review.** Hand-authored JSON compiled in with `include_str!`, digest-pinned by a copy of
`crates/mush-atlas/tests/starter.rs:31-60` (which prints moved digests: there is no CLI), validated clean by
a copy of `the_starter_packs_validate_clean` (`tests/validate.rs:138-152`), because boot packs bypass the
validator and a refused first apply aborts the boot of dev1's kept world (`module.rs:113-139`). Appended to
`content_module_with`'s boot list and the atlas shelf (`composition/mod.rs:690-707,762-787`). The building
agent is not used for drafting (staff-only; SCHEMA lacks neighborhood/region, `plan.rs:39-60`);
`@atlas/propose --from` and `@gen` may dress descriptions once geometry is fixed. Names below doc 11's
principals are proposals (Q-18-12).

### 4.6 NPCs

All NPCs ship inside the pack that makes their rooms (`mush-npc/src/content.rs:214-219`). Interface NPCs
front only providers that exist; no new provider ships in the slice (P3 adds `ledger`, `ship`, `port`
and the provider-identity fix). No NPC fronts `request` (its persist is dropped under the `npc` identity,
§3-I). No NPC performs a transaction: every money and matter act is the player's own verb; the desks read,
explain and hint (doc 14 §4.6 rules 1–4).

| Port | NPC | Tier | Interfaces | Fallback / lore |
|---|---|---|---|---|
| Ceres | Berth Officer (Port Watch) | minor | atlas, travel | "Your hull's lock is on the ring: +ship/embark, then +ship/undock." |
| Ceres | Ceres Clearing teller | minor | help | "An advance is +credit/advance; your balance is +credit." Public lore: clearing through Luna |
| Ceres | Tycho Mutual agent (Corin Mbatha, proposed) | minor | help | "Starter charters are +ship/charter <name>, one to a pilot." Lore: Re-float is for the ruined; Assessor Adeyemi is at Kirkwood (11 §4.7 places him there) |
| Ceres | Occator factor (the Warrant signatory of Occator's listings) | minor | market (venue pinned from the port map; replies Asker) | "The board is +market/list: water and reaction mass. Fuel both legs here: Stickney charges Republic prices." |
| Ceres | Co-op chandler | minor | market | "Stores and crew-days. Reaction mass is Occator's board next door." |
| Ceres | Registry clerk | minor | help | title vs charter vs paper |
| Ceres | Syndicate agent; barkeep; dispatcher | extra | none | Pallas outer-claim lore; guarded/secret Tether lore |
| Ceres | Wen, Varga, Ndiaye | major, `enabled:false` | none | placed now so the faction cut binds existing objects |
| Stickney | Orbital Guard petty officer | minor | atlas, travel | Customs Assertion lore |
| Stickney | Kasei receiving clerk (the Warrant signatory of Kasei's bid) | minor | market | "We buy volatiles: +market/list, then +market/sell." |
| Stickney | Yard chandler; Tycho desk clerk; mess cook | minor / extra | market / help / none | — |
| Stickney | Castellanos, Tanaka, Bask | major, `enabled:false` | none | principals for the faction cut |

Lore pools by tag: `corridor_trade`, `port_ceres`, `port_stickney`, `tycho`, `tether`. Facts quote doc 11.
Daily token caps per NPC; the LLM budget ledger is RAM-only (survey NPC-7).

### 4.7 The content pipeline

**Which vocabulary carries what.** (1) **Rules**, tacsim `rules_definition`, compiled in:
`tacsim.pack.dray`; the goods catalog is a compiled-in `Vec<CatalogEntry>` JSON, not a pack kind.
(2) **Places**, mush-content atlas packs: `region.ceres_freeport`, `neighborhood.ceres_freeport`,
`region.stickney_yards`, `neighborhood.stickney_yards` (existing kinds plus `RoomDef.markers`; npc and lore
inside). (3) **The corridor and the ports**, one tacsim `campaign_seed` pack `tacsim.corridor` v1: a foreign
`sol` region; bodies `ceres`, `kirkwood`, `mars`, `pallas`; locations `ceres_freeport`, `kirkwood_depot`,
`stickney_yards`; routes `ceres_stickney` and `stickney_ceres` (legs via Kirkwood, `drive_class
coast_hauler`, `propellant 400`); and the ONE new kind, **`port`** (D-18-20), seed-only under the
misplacement rule (`pack.rs:1129-1155`), added to `OWNED_KINDS`. This pack must be in `TacsimStore.loaded`
for `route_in_world`, so it joins `loaded` through a narrow **game-packs list** in `verify_sources`
(`lib.rs:197-243`), separate from the 28 fielded unit packs (so the digest gate, `lib.rs:871-884`, arms only
for it), and it carries no `requires`: the port applier resolves atlas rooms by `<pack>.<entry>` through the
latch and refuses before mutation if any is absent (the arena precedent, `composition/mod.rs:737-745`).

**The `port` entry.**

```json
{
  "ceres_port": {
    "kind": "port",
    "def": {
      "location": "ceres_freeport",
      "venue": "ceres",
      "area": "neighborhood.ceres_freeport",
      "rooms": {
        "dock": "neighborhood.ceres_freeport.room.dock_ring",
        "clearing": "neighborhood.ceres_freeport.room.clearing",
        "charter": "neighborhood.ceres_freeport.room.tycho_agency"
      },
      "treasury": "tacsim.treasury",
      "orgs": [
        { "id": "ceres_clearing", "name": "Ceres Clearing", "capital_major": 1000000, "faction": "freeport" },
        { "id": "tycho_mutual", "name": "Tycho Mutual Assurance & Salvage", "capital_major": 200000, "faction": "concord" },
        { "id": "occator", "name": "Occator Water & Propellant", "capital_major": 100000, "faction": "freeport" },
        { "id": "coop", "name": "Belt Carriers' Cooperative", "capital_major": 100000, "faction": "freeport" }
      ],
      "clearing": { "org": "ceres_clearing", "advance_major": 20000 },
      "charter": { "org": "tycho_mutual", "unit": "tacsim.unit.dray", "issuer": "tacsim.registry",
                   "pool": 6, "registry_prefix": "FPC-D-04", "fee_major_per_week": 1500 },
      "vendors": [
        { "org": "occator", "storefront": "neighborhood.ceres_freeport.item.occator_manifold",
          "signatory": "neighborhood.ceres_freeport.npc.occator_factor",
          "asks": [ { "code": "volatiles", "price_major": 7, "qty": 200000 },
                    { "code": "propellant", "price_major": 10, "qty": 200000 } ], "bids": [] },
        { "org": "coop", "storefront": "neighborhood.ceres_freeport.item.chandlery_cage",
          "signatory": "neighborhood.ceres_freeport.npc.coop_chandler",
          "asks": [ { "code": "provisions", "price_major": 25, "qty": 20000 } ], "bids": [] }
      ],
      "routes": { "stickney": "ceres_stickney" }
    }
  }
}
```

Stickney's entry has no `clearing` or `charter` block, orgs `kasei` (capital 500,000, finite, D-18-14)
and `yard_chandlery`, Kasei's `bids: [{ "code": "volatiles", "price_major": 15, "qty": 50000 }]` with
`signatory` the receiving clerk, and asks propellant 24 / provisions 30, and `routes: { "ceres":
"stickney_ceres" }`. Prices are in major units and converted at apply. Under Q-18-9's alternative the
`kasei` org carries `facility: { "per_ic_day_major": 50000, "total_major": 1500000 }`: a standing
`create_system_obligation` from the treasury (the only faucet principal the call admits) on an IC-day
cadence (`EconomicClock::WorldIc`, period 86,400) with a total cap, fictioned as the Ceres Clearing
facility of 11 §4.3; never a `drip_major_per_day` on the real clock (`fund_from_faucet` floors at 60 real
seconds, §4.1).

**The `PortApplier`** (tacsim/src/content.rs, phase 11, on the `ArenaApplier` pattern). Create, in order:
(a) `open_passive_domain(location room, dock room)` after checking the dock room's `RoomGeo` and
`port.dock` marker; (b) the treasury System account if absent; (c) per org: `mush_org::submit(CreateOrg)`
+ root scope, `open_account` (forbid), one-shot capitalization obligation (keyless in the economy: the
applier map row `content.capital/<pack>/<entry>/<org>` → obligation id is the only dedup and is written
in the same batch), the Q-18-9 facility if declared; (d) the goods catalog, once; (e) per vendor:
`create_vault(Container(storefront item))`, `register_vendor(venue)`, ask and bid listings; (f) the charter
pool: `pool` hulls, each a Thing (owner = content owner), `create_unit`, Bridge and Bay rooms with
`sim.interior` anchored `MovingObject`, placed Direct on the port domain, `issue_title` to the Tycho org
under `tacsim.registry`. It persists an applier-owned entry → `{domain, orgs, accounts, vault, vendor,
listings, hulls}` map in `TacsimStore` that fails closed when corrupt (the `tacsim:seed` key rule,
`store.rs:5-8`), because `AppliedEntry` holds only `Option<ObjId>`. Vaults are addressed by id, never
through `live_endpoints` (the 64-choice cap). **Update** rewrites prices, quantities and names in place and
never re-capitalizes, re-funds or re-mints (it overrides the default retire+recreate,
`apply.rs:233-245`). **retire_check** refuses while a vendor has live contracts or a hull is chartered.
**Retire** withdraws listings and retires vendors; the domain, orgs, accounts and hulls stand.

**Boot-once acts.** Everything above runs inside the content module's boot pass, after the sheet module
has registered `tacsim.credit` and the ten instruments (`sheet/mod.rs:352`), in pack order: starter (3),
proving ground (2), the four station packs, `tacsim.corridor`. A never-latched id applies at ANY boot,
including dev1's kept world; an anomaly aborts the boot. Money at apply is limited to account opening and
obligation records (normal writes); the only posting is the capitalization obligation, which settles on the
next sweep, so the `RetryShortly` cold-key risk (`api.rs:1334-1339`) never lands in the boot pass. Nothing
per-player runs at boot: account, advance, charter, hold vault and title-free charter record are all
player verbs.

**Idempotency.** The latch skips unchanged entry digests (`apply.rs:183-190`); a partial apply is latched
honestly and the next apply continues from the map: an org whose map row names an obligation id is
skipped, one without a row is capitalized (there is no `AlreadyPosted` for obligation creation, §4.1, so
a lost row would be a second 1,000,000 cr one-shot, which is why the row and the obligation share a
batch and the map fails closed); player verbs are create-only per (pilot, port entry) in `TacsimStore`;
`issue_title` is idempotent per generation. Re-apply of an identical pack is a HIT.

**Config flips on dev1.** `[tacsim].enabled` already true. `[economy]` per-instrument mint cap override for
`tacsim.credit` = 2,000,000 (template; headroom over Clearing's 1,000,000, §4.1). `[tacsim.economy].market` stays false (the market composes anyway;
the flag gates only the market title issuer). `[onboarding].start_room` stays 0 (the door is from Limbo).
`[tacsim].pack_paths/seed_paths` stay `[]` (compiled in). World rate to 24 by `@tacsim/time rate=24`
after the owner approves a redeploy; **never redeploy without the owner's ask** (memory rule).

---

## 5. The cut plan

Sizes: S ≤ a day, M a few days, L a week-plus, honest after the buildability review. One commit and push
per cut, with the Fable trailer, as the 16b/16c loop does. Platform cuts (IC0, IC5) have no content
dependency and can run in parallel with content cuts.

| Id | Name | Scope | Crates | Size | Depends on | Verify lens |
|---|---|---|---|---|---|---|
| IC0 | Engine blockers first | Lot short-circuit in `deliver()`/`cannot_deliver`; listing TTL ≤ hold TTL and `is_current` refuses a lapsed hold; non-staff manual-route arm closed; `spacecraft` out of `SELF_MODES`; major-unit prices market-wide; economist replies Asker; `lot_tour` driver id fix | mush-market, mush-sim, mush-atlas | M | — | `e7r2_proving_loop` promoted to a gate: stock → quote → accept → settle with no `SettlementRefused` page; non-staff `@journey/depart x=r #12:1` refused; `+travel` offers no spacecraft |
| IC1 | Two station maps and the IC door | Four atlas packs (13 + 9 rooms, items, features, themes, NPCs incl. `enabled:false` principals, lore pools); `RoomDef.markers` + `port.dock` topology seed; `exit.limbo_arrivals`; boot list + shelf; validate-clean and digest-pin tests | mush-atlas, tacsim | M | — | fresh and kept-world boot both apply; `c` lands in Arrivals; every Ceres room walkable; no OUTDOORS room; Stickney unreachable on foot and not flagged; starter digests unmoved; T-45 |
| IC2 | The Dray, the catalog and the sky | `tacsim.pack.dray` (class hauler, logistics) in `fielded_sources` + pin; `UnitClass::Hauler`; lobby filter + custody refusal test; `goods.json`; `tacsim.corridor` v1 (sol, bodies incl. kirkwood/pallas, three locations, two routes) compiled in and joined to `loaded` via the game-packs list; T17 leg test | tacsim | M | IC1 | `@corridor/show ceres_stickney` prints two legs and propellant 400; SIM UNIT never offers dray; a staff-placed Thing flies the catalog arm; `@tacsim/pack list` shows `tacsim.corridor@1`; shipped DefinitionRefs unmoved |
| IC3 | Ports, institutions and the Clearing advance | `port` kind + `PortApplier` parts a–c (domain, treasury, orgs with capital, port map, fail-closed); template mint-cap override; `+credit/open`, `+credit/advance`, `+credit` | tacsim, mush-server | L | IC1, IC2 | after boot + one sweep `@org/list` shows the orgs with capital; double boot posts nothing; corrupt map refuses re-creation; at Clearing `+credit/advance` shows 20,000.00 cr at once, twice refused, elsewhere refused; `@domain` shows both domains Active presenting into the dock rooms |
| IC4 | The loaner pool, the charter and boarding | `PortApplier` part f (six hulls, interiors, title to Tycho under `tacsim.registry`); `+ship/charter`, `+ship`, `+ship/embark`, `+ship/debark`; charter record; hold vault at charter | tacsim, mush-economy, mush-sim | M | IC3 | six Drays listed in the Dock Ring, `@asset/show` title Tycho; `+ship/charter Moth` once, second refused; pilot's raw `@journey/depart` refused; embark → Bridge, exits vetoed; debark → ring; another player's embark refused |
| IC5 | Market and economy seams for a content port | `with_party_cap_source`; `Supply::Stock`+`Lot` with `LotOp::Issue` (Origin{contract}) and quantity decrement; `with_venue_gate` + `venue_of_room`; default `market=`/`to=`/`from=`; `CapacityProvider` hook on the lot leg; amend the "market never mints" pins | mush-economy, mush-market | M | IC0 | mush-server test on the production composition with a test vendor, party-cap source and venue gate: accept lands an Origin lot, listing decrements, no page, price reads "7.00 cr"; gate refusal escrows nothing; capacity refusal before any contract |
| IC6 | Port commerce | `PortApplier` parts d–e (catalog, storefront vaults, vendors with signatories, asks, Kasei bid; the Q-18-9 facility only if the owner takes it); tacsim `VenueGate`, party-cap source, `CapacityProvider` from logistics; hold seal while under way (scoped); `+ship/hold`; the accept line names the signatory | tacsim | L | IC3, IC5 | at Ceres list/quote/accept with no arguments loads the hold; 2,801 t refused; from Limbo refused; into a hold docked elsewhere refused; `+market/depth` readable anywhere; kept-world reboot duplicates no org; seed path never touches the 64-choice cap |
| **IC7** | **Undock and fly — first player-runnable cut** (buy, board, fly, dock) | `pub mush_sim::depart` (authored arm); `tacsim.port.burn` authoritative action + departure ticket (its own action registration and delegate test: tacsim joins `with_quantity_delegates` for write authority only, `composition/mod.rs:400-404`); a **new tacsim normal-lane `reg.tick`** (today's only tacsim tick is the 1 s board tick, `match_/sim.rs:79,626-628`) for the launch + boot reconcile; `+ship/undock` checks; narration to Interior occupants at the Kirkwood boundary AND at `apply_arrival`; `+ship` ETA countdown; apply-time refusal of exits into anchored interiors; reverse route. **IC7b** (split out): `+travel/status` aboard arm; the server-half-restart test | mush-sim, tacsim, mush-inventory, mush-atlas | M–L | IC4, IC6 | at rate 3600: undock burns 400 t / 3 crew-days, ticket launched within one tick, **the pilot in the Bridge sees** the Kirkwood line at the boundary and the arrival line (≈328 s / ≈576 s at the Q-18-2b default; ≈41 s / 72 s at the alternative), debark walkable; refusals: from the ring, 399 t propellant, no route, overloaded, under way twice; hold moves refused in flight; **one hand-played smoke at the play rate** on local `mushd` (the owner or the builder: undock, log off, return, arrive), logged in the cut note. IC7b: a server-half restart between ticket and launch yields exactly one journey |
| IC8 | Selling: Wanted listings and the lot sell arm — **the loop closes** | `Supply::Wanted` listings; `+market/sell <listing>=<qty> [from=]`: gate, synchronous source check, vendor consideration, party-cap accept, seller accept, staged `LotOp::Retire`, quantity decrement; port bids listed by the applier; `ic_slice_loop.rs` end to end | mush-market, mush-inventory, tacsim, mush-server | L | IC7 | the §6 transcript at 3600 on the production composition; conservation from journals and lineage (pilot +21,000 = Kasei −21,000; hold −1,400 t = retired 1,400 t); selling more than held refused before any contract; unfunded vendor refuses; Ceres sale against the Stickney bid refused |
| IC9 | Help and the honest surfaces | `docs/help` ports, ships, economy, trade; amend getting-started, chargen, travel, atlas, spatial, cockpit, npc; in-code help for `@journey` and `@agent`; `ops.md` asker-authority note | docs, mush-sim, mush-npc | S | IC8 | T-45 green; a fresh tester completes the loop from help alone |

Post-loop cuts, named and ordered, not sized here: **P1** `Obligation.max_periods` + `tacsim.week`
cadence + repayment + default consequence (S–M); **P2** `OperatorAuthority` seam in mush-sim, stations in
the unit spec, `+ship/helm`, the recovery provider fixed (pool by content ref, tier → class, cadence
upkeep) and the charter record replaced by `Encumbrance::Lease` (M–L); **P3** provider identity fix,
`InterfaceAbility.kind`, `ledger`/`ship`/`port` providers, `market.depth` (M); **P4** Kirkwood as a
checkpoint, faithful atlas route copy, a hold of a sim hull, the Customs Assertion front (M); **P5**
content endowment + Deposit/Facility appliers + industry restock, conservation closed (L); **P6** drive-
and payload-dependent journey time, berth slots (M); **P7** Pallas as a port (data); **P8** delete the
legacy seed latch, porting `e7c1`, `e7r1`, `e7r2`, `e7r2_proving_loop` (M); **P9** IC1b onboarding by pack
entry and the approval marker (S).

**IC0.** Add `Representation::Lot` to the short-circuits at `mush-market/src/module.rs:386,443`. Cap
`@market/vendor stock` days at the inventory hold TTL (`commands.rs:1001-1008` against `record.rs:33-35`)
and make `trade.rs` `is_current` read the hold's deadline. In `mush-sim/src/commands.rs` build_route,
refuse place tokens for non-admin. Remove `spacecraft` from `SELF_MODES` (`travel.rs:90`) and from
`options_from_corridor_room`. Route every price line in mush-market through one formatter that reads the
instrument scale and symbol. Default `market.list`/`market.quote` replies to `ReplyAudience::Asker`
(`interface.rs:89,113`). Fix `region.limbo.json:179`. Promote `e7r2_proving_loop` from fixture to gate.

**IC1.** Author the four packs to §4.5. Add `markers: Vec<String>` to `RoomDef` (`def.rs:366-390`) with a
registry the validator consults; seed `port.dock` rooms in `validate.rs:1958-2000`; store markers on the
room object's `markers` set unregistered. Write `exit.limbo_arrivals` with `portal.at`. Append the packs
to `definitions::content::boot_packs()` and the atlas shelf. Copy the starter digest-pin and
validate-clean tests. Add a kept-world boot test from a pre-IC1 snapshot.

**IC2.** Author `dray.json`; add `Hauler` to `UnitClass` and `logistics` to `UnitSpec` with
`skip_serializing_if`; filter `hauler` in `fieldable_ids` and add a custody-seam test that refuses it.
Author `goods.json`. Author `tacsim.corridor` with the sky of §4.4; add `pallas` and `kirkwood` bodies.
Add a `game_packs()` list merged into `loaded` in `verify_sources`, excluded from `MAX_CONFIGURED_PACKS`.
Write the T17 leg test against `route_duration_at`.

**IC3.** Add `PackEntry::Port(PortDef)` and the misplacement rule row. Implement `PortApplier` a–c with
the fail-closed map. Open the treasury with `open_account`. Create orgs with the mush-faction shape;
capitalize with `create_system_obligation` one-shots. Add the template cap override. Implement
`+credit/open`, `+credit/advance` (post as org actor + `create_obligation` as pilot, with reversal) and
`+credit` with a major-unit formatter shared with mush-market.

**IC4.** Implement `PortApplier` part f. Register `tacsim.registry` in `with_title_issuers`. Implement the
charter record and `+ship/charter`, `+ship`, `+ship/embark` (`place_interior`), `+ship/debark` (`unplace` +
`move_to`). Open the hold vault at charter.

**IC5.** Add `EconomyModule::with_party_cap_source(Fn(&World, &PrincipalId, PartyCap) -> bool)` consulted
after the frozen table (`contract_live.rs:5163-5169`). Allow `Supply::Stock` + `Representation::Lot` in
`api.rs:718-735`; add `LotOp::Issue` staged at activation in place of `Consume` (`trade.rs:673-688`,
`lot.rs:160-195`) calling `mint_origin` with `Lineage::Origin{contract}`. Add `with_venue_gate` and
`venue_of_room`; default arguments in `commands.rs:424-499`. Add a `CapacityProvider` hook on the lot leg.
Amend `tests/e7r2.rs:315-332`.

**IC6.** Implement `PortApplier` d–e; the tacsim `VenueGate` (actor in the port area; endpoint backing →
bay → host → Direct on this port's domain), the party-cap source (port-map vendor orgs) and the
`CapacityProvider` from `logistics`. Add `sealed()` to `InteriorCustodyProvider` for backings whose hull
is under way and consult it in custody and inventory movement. Add `+ship/hold`.

**IC7.** Extract `pub fn depart` from `journey_depart`. Register the `tacsim.port.burn` action
(dissolve, debit, ticket) with its own delegate test and add `tacsim` to `with_quantity_delegates`.
Register a tacsim normal-lane `reg.tick` (there is none today) with the launch arm, and the boot
reconcile. Implement `+ship/undock` and the `+ship` ETA countdown. Add narration to Interior occupants
of the carrier subtree (an audience built from the subtree's Interior occupants, or `Audience::Actor`
per occupant) at the leg boundary in `journey.rs:1088-1114` AND in `apply_arrival` (`journey.rs:1396-1409`,
whose `to_room_from(to, …)` never reaches the Bridge). Refuse exits into anchored interiors at
`apply.rs:1494-1498`. Play it once by hand at the play rate on local `mushd` and log the run. **IC7b:**
add the `aboard_carrier` arm to `travel_cmd.rs:285-312`; write the restart-mid-flight test.

**IC8.** Add `Supply::Wanted{commodity, grade, price, qty}` and the applier's bid listing. Implement the
lot arm of `+market/sell` beside the object arm (`trade.rs:931-1085`): gate, source check, contract with
the vendor's consideration, `accept_contract_as` for the vendor, seller accept, staged `LotOp::Retire`,
decrement. Write `ic_slice_loop.rs` to §6.

**IC9.** Write and amend the help topics listed in §10; correct `@journey`'s "(staff)" and `@agent`'s field
list and "default mortal"; note the asker-authority bridge in `ops.md`.

---

## 6. Acceptance

**The story test.** On a fresh world (local `mushd`, and once on the split) at `@tacsim/time rate=3600`
(the only staff act), a non-staff tester types the transcript below and sees the quoted lines; the same
transcript is the body of `crates/mush-server/tests/ic_slice_loop.rs` on the production composition.
In this listing `;` separates lines typed one at a time: the server has no command separator (a
leading `;` is the semipose prefix, `mush-core/src/cmd.rs:1300-1304`, and nothing splits on it), so
`ring ; lift ; clearing` typed literally is one line that matches nothing. The timings are for the
Q-18-2b default (24 IC days) with the three-day alternative in brackets.

```
create Vex pw                       -> Limbo; exits O, S, C
c                                   -> Arrivals, Dock Deck, Ceres Freeport
ring ; lift ; clearing
+credit/open                        -> "Ceres Clearing opens account #41 ... Balance 0.00 cr."
+credit/advance                     -> "... advances 20,000.00 cr on the Freeport's new-operator line ... terms not yet scheduled."
+credit                             -> posted 20,000.00  available 20,000.00  owed 20,000.00
concourse ; tycho
+ship/charter Salt Moth             -> "... charters you ... Salt Moth (FPC-D-0417) ... Hold vault #212 ..."
+ship                               -> docked Ceres Freeport; cargo 0/2,800 t; propellant 0/1,200 t; provisions 0/480
concourse ; occator
+market/list                        -> Volatiles, bulk 7.00 cr/t · Propellant 10.00 cr/t (market=ceres defaulted)
+market/quote 7=1400 ; +market/accept 31   -> "1,400 t of volatiles are loaded into the Salt Moth's hold. Occator's factor countersigns."
+market/quote 8=800 ; +market/accept 32    -> 800 t propellant, 8,000.00 cr
chandlery
+market/quote 9=10 ; +market/accept 33     -> 10 crew-days provisions, 250.00 cr
+ship/hold                          -> cargo 1,400/2,800 · propellant 800/1,200 · provisions 10/480
+credit                             -> available 1,950.00 cr
concourse ; ring ; +ship/embark     -> "You cycle the ring lock into the Salt Moth."  (look: Bridge)
+ship/undock stickney               -> "Clearance granted. 400 t propellant and 3 crew-days committed ... arrival in 24 IC days"
+ship                               -> leg 1 of 2, cruising for Kirkwood Depot; ETA 13 d 16 h IC
   (~328 s [~41 s])                 -> "Kirkwood Depot slides past ... Leg 2 of 2."   (seen in the Bridge)
   (~576 s [~72 s] total)           -> "Salt Moth arrives, and the passage ends."     (seen in the Bridge)
+ship/debark                        -> Yard Gantry, Stickney Yards
spine ; kasei
+market/list                        -> Kasei Metals WANTS volatiles 15.00 cr/t (market=stickney defaulted)
+market/sell 14=1400                -> "Kasei Metals buys 1,400 t of volatiles at 15.00 cr/t: 21,000.00 cr. Kasei's receiving clerk countersigns."
+credit                             -> available 22,950.00 cr; owed 20,000.00 cr + 1,500.00 cr/week
+ship/hold                          -> cargo 0 · propellant 400 · provisions 7
```

**The 3600 story test does not stand in for the play-rate experience.** At the play rate the pilot
spends the flight Interior-placed in one room with no exits (`mush-sim/src/lib.rs:263-276`) and one
read verb; the rate is a global knob that moves contracts and cooldowns (`mush-core/src/clock.rs:74,83`;
dev1 seeds `IC_RATIO=4`, `deploy/dev1/instance.env:81`), so flipping dev1 to 3600 for a session is not an
option. IC7 therefore carries one hand-played smoke at the play rate on local `mushd`, and `+ship` prints
the ETA as a countdown and says the passage continues while the pilot is disconnected.

Negative checks in the same run, each refused with an in-fiction line and nothing moved: `+credit/advance`
twice; another player's `+ship/embark Salt Moth`; `+market/accept` from Limbo; `+ship/undock` from the
ring (not aboard), with 399 t propellant, to `pallas` (no route), or with 2,801 t in the hold; `@inv/move`
out of the hold while under way; `+market/sell` at Ceres against the Stickney bid; the pilot's raw
`@journey/depart`. A server-half restart between the burn ticket and the launch yields exactly one journey
and one arrival; a restart mid-flight changes nothing. After the run, every balance reconciles from the
economy journal and every quantity from inventory lineage (pilot +21,000.00 = Kasei −21,000.00; hold
−1,400 t = 1,400 t retired with the contract as sink).

**The harness/CI lens.** `ic_slice_loop.rs` runs on the production composition (`build_engine`) like the
16b A1 story test; `e7r2_proving_loop` is a gate from IC0; the four station packs and `tacsim.corridor`
pass `validate()` clean and a kept-world boot test; `tests/content.rs` pins every digest; the T17 leg
test pins the sky; the custody-seam test refuses a hauler; T-45 holds the help. The load-sensitive suites
rule of the 16b loop applies to the rate-3600 test.

---

## 7. Decisions taken here (under the defaults of §8)

- **D-18-1 Ports.** Ceres Freeport → Kirkwood Depot → Stickney Yards. Tycho Mutual is the ship desk at
  both; Pallas is a body, a lore thread and the named next port. (Rests on Q-18-1.)
- **D-18-2 Blockers first.** IC0 lands the six engine guards before any content.
- **D-18-3 The IC door** is an unlocked exit from Limbo in the Ceres pack with no `requires` on a starter
  pack; locked on an unregistered approval marker when chargen is on (P9).
- **D-18-4 No staff act and no NPC act on the player path.** Every setup act is boot-applied content; every
  money and matter act is the player's own verb.
- **D-18-5 The credit line** is a loan into a positive balance from the Ceres Clearing org's own account,
  with the debt a pilot-taken Manual-clock obligation; never an overdraft; never a faucet grant.
- **D-18-6 The pilot account** opens at the Clearing desk by verb, not at chargen approval.
- **D-18-7 Repayment is deferred** to P1 (`max_periods` + `tacsim.week`); default consequence is refusal
  of new credit and charter renewal, no seizure.
- **D-18-8 The hull** is a content-owned carrier Thing with a Bridge (boardable, Interior placement) and a
  Cargo Bay (unboardable, the hold vault's backing); one boardable room; title to the Tycho org under
  `tacsim.registry`.
- **D-18-9 No stations or billets** in the slice; the charter record is the operate gate, read only by the
  `+ship` verbs; the hull's MUSH owner is never the pilot. This bypasses C2A–C4A/F3's station-channel
  gate for the hull's departure until P2 (one boardable room, one charterer, no other crew make the two
  gates admit the same person); the alternative that removes the bypass is in Q-18-4.
- **D-18-10 Acquisition** is a charter from Tycho's seeded pool of six; no hull is minted; title stays with
  Tycho; the charter record is a placeholder for `Encumbrance::Lease` (P2). (Rests on Q-18-4.)
- **D-18-11 Consumables** are inventory pools in the hold; the burn settles at undock, matter first, by one
  authoritative action; the launch is the tacsim tick consuming a typed ticket; never an
  `ActionStateChanged` persist, never a GOD-tick authoritative submission. (Rests on Q-18-7.)
- **D-18-12 Presence and defaults.** A `VenueGate` seam, implemented by tacsim from the port map; `market=`
  defaults to here and `to=`/`from=` to the berthed hold; `+market/depth` is remote-readable.
- **D-18-13 The resource subset** is volatiles, propellant and `provisions` (opaque, crew-days, no
  instrument); `FAMILIES` stays ten. (Rests on Q-18-3.)
- **D-18-14 Vendor funding:** finite world-start capitalization by one-shot faucet obligations (7A's
  list as written; Kasei 500,000 cr = 23 transcript sales), deduped only by the applier map; recapitalization
  is an attributed staff/system act (`@econ/mint` or a re-applied `capital_major`), never a perpetual
  drip. (Rests on Q-18-9.)
- **D-18-15 Goods origin:** listing-bounded issue at a purchase and retire at a sale, `Lineage::Origin{contract}`.
  (Rests on Q-18-8.)
- **D-18-16 The travel record** is `CarrierJourney`; `spacecraft` leaves `SELF_MODES`; D-14-35's fold is not
  taken. (Rests on Q-18-6.)
- **D-18-17 Flight time** is authored `base_s`; T17 pins each leg's `distance_km`, and the content test
  pins `base_s` to `distance_km / coast_km_s` for `coast_hauler` at 1,000 permille (never
  `route_duration_at` at the pin, which is a tautology); the rate is 24 for play and 3600 for tests; the
  lane's IC duration is Q-18-2b's (default 24 IC days, 1A's scale), and the play rate, not the lane, is
  the lever for a shorter real run.
- **D-18-18 Docking** is one passive domain per port, host = the location room, presentation = the dock
  room; boarding and leaving are placement verbs; no gangway exits; exits into anchored interiors are
  refused at apply; the hold is sealed under way.
- **D-18-19 `RoomDef.markers`**, a closed vocabulary, with `port.dock` as a validator topology seed.
- **D-18-20 One new pack kind, `port`**, in the compiled-in `tacsim.corridor` seed pack, joined to
  `TacsimStore.loaded` through a narrow game-packs list; party caps resolved by content reference.
  (Rests on Q-18-10, Q-18-11.)

**Locked decisions this plan touches.** *Leaves:* README superseded 1–7; D-16-* (the arena is untouched;
the hauler filter keeps it so); doc 17 D-UM-* — **because the Dray carries no core component**: a hull
with no core is Class 0, the drive ROM (17 §9), so nothing is separately titled (D-UM-33), no licence or
warrant principal is needed, and `+ship/undock` is not a sealed sortie dispatch (D-UM-45); authoring a
Class I governed core on a Tycho loaner would have been a lore and hardware decision this slice does
not take (the first draft installed `tacsim.core.doctrine`, which is exactly that, `cores.json:10-16`);
T1 (travel time by drive and payload is deferred, not denied: P6); T4; doc 10 4A (ten families kept);
doc 10 7A (finite capitalization; the drip is now only Q-18-9's alternative, and taking it amends 7A).
*Honours:* T17 (the pinned sky: distances by test, durations by Q-18-2b); T2 (no economic transaction in
a pulse; the burn is in no pulse); doc 11 §2's Warrant (every listing has a named signatory). *Amends, on
the owner's answer:* **doc 10 1A's travel scale** if Q-18-2b takes the three-day alternative (Belt
"weeks" → days; cite 1A as amended); **C2A–C4A and doc 10 §18.1 / F3's station-channel gate** for the
hull's departure until P2 (D-18-9: `pub mush_sim::depart` from `+ship/undock` checks Interior-in-Bridge
plus the charter instead of a station's `movement` channel; Q-18-4's alternative removes this); the E7
"extraction is the only mint / the market never mints" pins (Q-18-8); DL-11 / D-E6-15's frozen party-cap
table gains a content-resolved source (Q-18-10); D-14-35's wording (Q-18-6); doc 11 §4.7 gains one line on
starter charters from the loaner pool (Q-18-4); D-14-34 is applied (Asker audience) rather than amended.

---

## 8. Questions for the owner

Each has a recommended default; "as recommended" builds §5 as written.

- **Q-18-1 Ports.** Default: Ceres Freeport → Kirkwood → Stickney Yards; Tycho as the ship desk; Pallas a
  body plus a Syndicate agent and the next port. Alternatives: (a) literal Tycho (a Luna port at Earth's
  ephemeris point, amending 11 §5) to Pallas (new body, no institutions, months of coast); (b) Ceres to
  Pallas with the Syndicate as buyer (invents most of Pallas, loses Kasei's fiction). Why: only the
  default reuses doc 11 §§3–7 wholesale and yields content the faction, front and campaign cuts consume.
- **Q-18-2 World rate for play.** Default 24 (doc 10 1A's ≈ one OOC hour per IC day; 24 real hours
  Ceres → Stickney at Q-18-2b's default, 3 h at its alternative). Alternatives: dev1's 4 (6 days / 18 h);
  144 (4 h / 30 min). Why: the rate is one global knob (contracts, cooldowns move with it) and it is the
  lever for a shorter real run; the lane's length is Q-18-2b's, and T17 pins distances, not durations.
- **Q-18-2b How long is Ceres → Stickney IC for a Dray?** Default: **1A's scale, 24 IC days** (`base_s`
  1,180,800 + 892,800; ≈93 km/s on a coast law; a Vigil torches it in days, so 11 §7's "cannot outrun a
  Vigil" holds). Alternative: **the playtest figure, three IC days** (147,600 + 111,600; ≈745 km/s, faster
  than any torch, copied from the test-only fixture's `fusion_torch` route; amends 1A's "Belt: weeks" and
  doc 10 §3's "IC days to months", to be cited as amended). Real-time consequence at rate 24: one day
  against three hours; at 3600 the story test runs ≈10 min against 72 s (the engine's rate ceiling is
  3600, `clock.rs:83`, so the test's length is the default's price). Why: the setting's whole scale of
  travel — convoys, blockades, the propellant decision — is set by this number and the first draft took it
  silently; if the owner wants a three-hour run, Q-18-2's 144 at the default gives four hours without
  touching the fiction.
- **Q-18-3 Food.** Default: `provisions` as an opaque inventory commodity in crew-days, no instrument,
  burned `crew × ceil(days)` per leg. Alternatives: an 11th family `life_support` amending 4A (S code:
  `FAMILIES`, `resources.json`, the "ten" pins, a new instrument); no food, propellant only. Why: zero
  code, doc 10 §9's granularity, and the family can follow without changing play.
- **Q-18-4 Starter ship.** Default: a Tycho charter from a seeded pool of six, title with Tycho, a tacsim
  charter record now and `Encumbrance::Lease` via the fixed recovery provider in P2; doc 11 §4.7 gains
  "or a starter charter to a new owner-operator". Alternatives: the lease now (tenant `@contract/accept`,
  pool by content ref, tier → class, cadence upkeep, a lease-activation observer granting the helm: M–L
  more before the first flight); a Stickney purchase on a note with title to the pilot (contradicts
  "no sale" and needs Asset delivery); join Lantern Haulage (needs ECO-3 membership verbs); **the
  station subset in IC4** (declare one helm station at spawn, grant `movement` on `+ship/embark` through
  `request_seat_activation`, S–M more), which removes D-18-9's C2A–C4A/F3 bypass and the `pub depart`
  entry point. Not `EconomicAsset.assignment` on the Tycho-titled hull (§4.2: it is read as standing by
  every economy verb and would be written as the Tycho org). Why: the smallest consented path with no
  mint and no staff act.
- **Q-18-5 Dray logistics shape.** Default: one optional `UnitSpec.logistics` object read only by the port
  verbs. Alternative: `ComponentRole` drive/hold/tank/life_support (doc 10 §8's components, strikable, with
  condition cells), plus a `drive` rules kind replacing the test-only `nominal_drive`. Why: one field keeps
  digests stable and bind untouched; the component shape is right once the hull must also fight (P6).
- **Q-18-6 D-14-35.** Default: decide it as "two records with a hard boundary: `CarrierJourney` for hulls
  across the corridor, `JourneyRec` within a region; the route source is the tacsim pack and the atlas
  connection is a derived projection"; remove `spacecraft` now. Alternatives: fold (1,786 lines); leave
  open and keep the self-mode. Why: it closes the one ship-less way across the lane and makes the decision
  text match code.
- **Q-18-7 When the burn settles.** Default: at undock, matter first, with the launch by the tacsim tick
  consuming a typed ticket (one-tick latency, boot reconcile). Alternatives: checked at undock and settled
  on a `sim.journey.arrived` fact with the hold sealed in flight (needs the fact provider and an abort
  rule); a core "committed-action continuation" seam (M–L core work that would also give market lot sales
  compensation). Why: no new core lane machinery, and "never flies unfuelled" is the simpler invariant.
- **Q-18-8 Ports as bounded economic boundaries.** May a port listing issue matter at a purchase and
  retire it at a sale (`Lineage::Origin{contract}`), amending the E7 "market never mints" pins for port
  listings only? Default: yes. Alternatives: a one-time content endowment (`Origin{seeded fixture}`, a
  boot `ActionTxn` mint, restock, the hold-TTL and 64-endpoint fixes) then industry; industry-only from day
  one (needs volatiles/propellant deposits and recipes, PRD-3). Why: four fewer mechanisms, and
  conservation returns in P5.
- **Q-18-9 Vendor funding.** Default: **finite world-start capitalization** (7A's list as written: Kasei
  500,000 cr, 23 transcript sales), recapitalized when a playtest drains it by 7A's attributed
  staff/system action (`@econ/mint` from the treasury, or a bumped `capital_major` re-applied). Alternatives:
  a **bounded** facility into Kasei (50,000 cr per IC day on `EconomicClock::WorldIc` via
  `create_system_obligation`, capped in total by `Obligation.max_periods`, fictioned as the Ceres Clearing
  money of 11 §4.3 — this amends 7A's "Rejected: infinite system market makers with caps" and is to be
  recorded as such; it is never `fund_from_faucet`, which is `RealUtc` with a 60 s floor and at rate 3600
  would mint 50,000 cr a minute); revenue-only. Why: the first draft's perpetual drip was exactly 7A's
  rejected shape and inverted doc 11 (Kasei is indebted to Clearing; the Concord treasury is "large and
  unavailable"); finite money is one `@econ/mint` away from any number of playtests.
- **Q-18-10 Party caps by content reference.** May composition answer `PartyCap::Accept` for orgs created
  by a latched `port` entry, through a code-declared source, in place of a frozen `OrgId` list? Default:
  yes, conferring no money authority. Alternatives: a config list filled after first boot (two-step deploy
  per world); a staffer in every deal. Why: content-created orgs have runtime ids the frozen table can
  never name.
- **Q-18-11 May packs declare money?** Default: yes, as numbers (capital, advance, fees, and the Q-18-9
  facility if taken) executed by the tacsim applier and verbs through `create_system_obligation`, `post`
  and `create_obligation`, deduped by the applier's fail-closed map (the economy's obligation creation
  carries no key, §4.1), under a raised per-instrument mint cap. Alternatives: a `[tacsim.economy.institutions]`
  config block; staff `@econ/mint` per world start. Why: the economy keeps faucet calls in game code
  (`api.rs:1700-1710`) while every number is data.
- **Q-18-12 Names.** Proposed and replaceable: *Salt Moth* (the starter hull, `FPC-D-0417`; a salvaged
  name from Tycho's pool of paid-out wrecks, 11 §4.7 — not "Lantern", which is Lantern Haulage & Salvage,
  the seeded player corporation of 11 §4.9 whose own Dray the faction cut will bind), the
  Ceres Clearing teller, Tycho agent **Corin Mbatha** (Adeyemi stays at Kirkwood), Occator factor, Co-op
  chandler, Registry clerk, Berth Officer (Port Watch), the Slack Line (bar), Kasei receiving clerk, Orbital
  Guard petty officer, the Yard chandler, `system:tacsim.treasury`, `tacsim.registry`, the verbs `+credit`,
  `+ship`, and the drive class `coast_hauler`. "Names ok", or replacements.

---

## 9. Risks, and what this slice refuses

**Risks.**

1. dev1 keeps its world: every new boot pack id applies at the next restart and any refusal or anomaly
   aborts the boot (`mush-content/src/module.rs:113-139`). IC1–IC3 carry validate-clean, digest and
   kept-world boot tests; no redeploy without the owner's ask.
2. The burn-then-launch ordering is new: a ticket with no launch (tick stalled) leaves a burned hull at
   the berth until reconcile; the test in IC7 asserts exactly one launch across a restart.
3. The market's paid-for-nothing class persists on both legs: `Issue` and `Retire` are staged after the
   contract is in force with no compensation in `lot.rs:719-750`. `Issue` cannot run short; `Retire` is
   pre-checked synchronously and only the owner can move a sealed hold's pools.
4. Issue/retire amends E7 doctrine; if Q-18-8 is refused, IC5 and IC8 grow into a boot mint, restock,
   the TTL and the 64-endpoint fixes.
5. `PortApplier` runs `mush_org::submit` (terminal on success, `org api.rs:255-257`) and several opens per
   apply; partial apply relies on honest latching and the fail-closed map.
6. The party-cap source loosens a wiring-frozen table (DL-11); it must confer no money authority, so the
   escrow still posts under the org's own debit authority.
7. Kasei's capital is finite: after ~23 transcript sales the bid is unfunded and `+market/sell` refuses
   with "unfunded vendor" until staff `@econ/mint` or a re-applied `capital_major`; the cut note for IC6
   says so. (Under Q-18-9's alternative the facility mints under a total cap instead.)
8. All hulls at a port share one dock room; twenty charters list twenty hulls. The pool is six.
9. The tracked stand-in makes a Dray read as ground armour to any arena reader; the hauler filter and the
   custody refusal test are the guards.
10. `with_venue_gate` and the hold seal touch the market and custody paths; both are scoped (one venue
    answer, one backing kind) and tested for non-interference with the OOC lot's vaults.
11. Doc drift a builder will hit: `content module.rs:15-17` "first boot only"; `tacsim economy.rs:28-31`
    "`market` gates composition"; `recovery.rs:39-41` and 11 §4.7 "campaign cadence"; `@journey` help
    "(staff)"; `api.rs:371-375` "accept_contract_as is the only org actor". Code wins; IC9 corrects the help
    and §10 lists the docs.
12. The economist's venue pin needs the port map; until IC6 the factor lists every venue.

**Refused, deliberately.**

- An overdraft as the credit line (invariant 26); interest, liens, foreclosure, a loan record.
- Minting a hull per charter; a market Asset sale of hulls; Re-float's recovery provider as-is; Lantern
  Haulage membership.
- `@chown` of the hull to the pilot, or MUSH ownership as an operate truth.
- A fuel or life-support `ScalarCell` beside a tradeable pool; propellant in two places.
- The atlas `spacecraft` self-mode; the non-staff manual place-token route; a market that trades
  regardless of place.
- Gangway or airlock exits, berth slots, multi-room hulls, stations and billets (P2, P6).
- Seeding real stock matter, industry-sourced goods, restock sweeps, and fixing the lot-hold TTL on a
  shelf (avoided by construction; the TTL cap in IC0 is the guard that remains).
- New interface providers and the NPC provider-identity fix (P3); any NPC that moves money.
- An 11th family, a thrust mobility mode, a `drive` kind, drive-dependent flight time (P5, P6).
- An IC/OOC room marker, a start room by pack reference, opening accounts at approval (P9).
- Customs holds at Kirkwood, interception, hazard bands on the atlas copy (P4).
- A Luna port, a Pallas station, Earth/Luna on the map.
- Editing the shipped starter packs or the test-only corridor fixtures.

---

## 10. Supersessions and amendments owed

- **README.** New row:
  `| [18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) | **The trade run, the first IC vertical slice (Draft 0.1, 2026-10-15, at f4b14f3b; decisions D-18-1..20 under stated defaults, questions Q-18-1..12 open)**: a pilot walks from Limbo into Ceres Freeport, opens an account and takes Ceres Clearing's advance, charters a Dray from Tycho Mutual's pool, buys volatiles, propellant and provisions into the hold, undocks, flies the pinned Ceres–Kirkwood–Stickney lane on a CarrierJourney, docks and sells to Kasei Metals; the verified map A–I with the verifiers' corrections, the money/ship/goods/travel/stations/NPC/pipeline design, the Dray definition, the one new pack kind `port`, ten cuts (IC0 blockers first; IC7 first player-runnable; IC8 closes the loop), the story test |`
- **README "Superseded decisions".** No new row yet; on Q-18-6 a row for D-14-35 as decided (two records,
  route source = the tacsim pack).
- **14b §3 / 14-spatial-implementation-plan.md:1321.** D-14-35 status and wording (Q-18-6); the
  "one route source (the atlas)" claim is false in code (`store.rs` materialize_route).
- **10b §3.11.** "No listing, no vendor record" was superseded by E7r2's lot listings; on Q-18-8, a note
  that port listings issue and retire matter under `Lineage::Origin{contract}`, bounded by listing quantity.
- **08a.** Ledger rows for: Lot in `deliver()`'s short-circuit, the listing-TTL cap, `with_party_cap_source`,
  `with_venue_gate`, `Supply::Wanted`, `LotOp::Issue/Retire`, the `CapacityProvider` hook; an IC3 option,
  a keyed `create_system_obligation` variant (`ObligationSpec.key`, refusing a second creation under the
  same key), since the call dedups nothing today; and the follow-on `Obligation.max_periods` and the
  `tacsim.week` cadence (P1).
- **Doc 10 1A.** On Q-18-2b's alternative only: "Belt: weeks" amended to days for the corridor, cited as
  amended; on the default, nothing.
- **Doc 10 7A.** On Q-18-9's alternative only: the bounded facility recorded as an amendment of
  "Rejected: infinite system market makers with caps"; on the default, nothing.
- **Doc 10 C2A–C4A / §18.1; doc 17 F3 row.** A note that the trade slice's `+ship/undock` bypasses the
  station-channel gate until P2 (D-18-9).
- **16c.** The hauler class filter in `fieldable_ids` and the custody-seam refusal test, as an entry beside
  the lobby's unit list; the "(staff)" on `@journey` help.
- **Doc 11.** §4.7 gains the starter-charter line (Q-18-4); §4.8 gains "the advance"; §5 a note that
  Kirkwood is a seeded body at 2.06 AU; §4.10 that Pallas has a body entry.
- **Doc 10.** §9 unchanged (provisions fit it); on Q-18-3's alternative, 4A to eleven. T1's deferral noted.
- **Doc 14 §2.6.** Three starter packs and seventeen rooms, not two and fourteen; §3.3's mode filter is
  now true once `spacecraft` leaves `SELF_MODES`.
- **`mushd.toml.example` [onboarding]** (`:413-419`, the WoD wizard and `wod-chargen`) and
  `mush-config/src/config.rs:394-401` ("guided wizard"; it already says `pilot-chargen`): drop the WoD text.
- **In-code doc comments:** `content module.rs:15-17`, `tacsim economy.rs:28-31,64-67`,
  `recovery.rs:39-41`, `mush-economy api.rs:371-375`, `tacsim lib.rs:36`, `fixture.rs:96`,
  `mush-content pack.rs:16-20` ("nothing else" is hashed; the brief is).
- **Help topics.** New: `ports.md`, `ships.md`, `economy.md`, `trade.md`. Amend: `getting-started.md:10-16,
  61-69` (the IC door; lot framing), `chargen.md:12-13,55-57` (money is `+credit`; start room defaults to
  Limbo), `travel.md:71-76` (player `+ship/undock`), `atlas.md:18,87-91` (the shelf holds the game's boot
  packs), `spatial.md:9-11,44-46` (stale "later cut"), `cockpit.md:3-5`, `npc.md:28-48` (cross-reference
  interfaces), `intel.md:36-41`, `faction.md:3-23`, `chronicle.md:42-43`, `clocks.md:10` (WoD residue).
  In-code: `@journey` "(staff)" (`mush-sim/src/commands.rs:3439`), `@agent` fields and "default mortal"
  (`mush-npc/src/module.rs:207-218`), `+market` price lines.
- **`docs/ops.md`.** The asker-authority bridge (use_ability → provider as the asker, Asker audience by
  default) beside the typed-action posture at `:976-979`.

**Draft 0.1 review fold (2026-10-15, three lenses: citation fidelity, fiction consistency, tester
walk-through).** Every finding was opened against the tree at `f4b14f3b` and the docs before it was
applied; none was found wrong. By finding: **F-1** (blocking) the lane's IC duration is now the owner's,
Q-18-2b, with 1A's 24 IC days as the default and three days as the playtest alternative; §0, §2 steps
12–13, §4.4, §6 timings, D-18-17 and Q-18-2 reworded; T17 is claimed only for the pinned km and the
tautological `route_duration_at` test replaced by a `base_s = distance_km / coast_km_s` pin. **F-2** (7A)
Kasei's funding is finite world-start capitalization by default (D-18-14, Q-18-9 flipped), the treasury's
fiction is the engine's capitalization, 7A is named under §7 "leaves"/"amends", risk 7 rewritten; the
drip survives only as Q-18-9's bounded alternative on `WorldIc` via `create_system_obligation`, never
`fund_from_faucet` (`RealUtc`, 60 s floor) — folds the two faucet-cadence findings and retires
`drip_major_per_day`. **F-3** (keyless obligation) §4.1 and §4.7 now say the economy does not dedup
`create_system_obligation` and the applier map, written in the same batch, is the sole guard; a keyed
variant is an 08a row. **F-4** (route scaling) the Dray's `thrust_permille` is 1000 and §4.4 explains
`route_duration`'s `1000/thrust` scaling and the private `DriveProfile` fields. **F-5** (Occator)
Occator sells volatiles 7 and propellant 10 at Ceres, the Co-op sells provisions; §2, §4.3, §4.5, §4.6
and the `vendors` block re-priced. **F-6** (economics) the money check is per round trip (8q − 8,250),
the transcript runs 1,400 t for +2,950, §4.1 Numbers restated, P1's terms resized to 1,100 + 1,500 a
week and made conditional on Q-18-2b. **F-7** (core) the Class I core and its `requires` are dropped from
`dray.json` (Class 0 ROM) and §7 says why D-UM-* is untouched. **F-8** (helm) D-18-9 and §7 "amends" name
the C2A–C4A/F3 bypass, §4.2 says why it is acceptable, Q-18-4 carries the station-subset alternative.
**F-9** (arrival narration) step 15, IC7 and its verify lens now reach the Bridge at both the leg
boundary and `apply_arrival`. **F-10** (play rate) IC7 carries a hand-played play-rate smoke, `+ship` is a
countdown with log-off-and-return stated, and §6 says the 3600 test is not the play-rate experience.
**F-11** (`assignment`) §3-C corrected; §4.2 and Q-18-4 weigh it. **F-12** (mint cap) "per obligation" in
§3-B; template override 2,000,000. **F-13** (IC7 size) M–L, the tacsim tick registration and the action's
own delegate test named, IC7b split out. **F-14** (Warrant) every listing has a named signatory who
countersigns, in §4.3, §4.6, the `vendors` block and the accept lines. Nits: the hull is the *Salt Moth*
(not "Lantern"); the refinery citation is 11 §4.3; `tacsim/src/definitions/resources.rs` and
`mush-market/src/store.rs` cited in full; `mushd.toml.example` vs `config.rs` drift attributed correctly;
"on the Freeport's new-operator line" and "the ring lock" replace the Tycho cover and Berth C; `;` is
declared listing notation; the Kirkwood timing is computed from `base_s` (≈41 s at the alternative, not
36). Not applied: nothing.
