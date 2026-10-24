# 15 — Communications: the relay backbone and the Malapert Exchange

**Draft 0.3, 2026-10-23.** The owner decisions in §9 are binding and all seven questions
in §10 are answered. **All mail is IC and must transit; there is no OOC mail mode.**
R1's registry and R2's routed orders, reports and returning ACKs are implemented
and deployed on dev1. Their live integration checks passed. The owner authorized
Gate 3 / R5 after the CI repair's dev1 rerun and is monitoring the long CI run.
R5, R6a and R6b are implemented and their dev1 checks passed. All mail and IC
channel traffic now transit. The real Discord bridge passed controlled-provider
tests; live Discord remains open because dev1 has no enabled bot or relay mapping.
[15b](15b-comms-relay-cut-notes.md) records the evidence and outstanding CI criterion.
[15a](15a-comms-relay-implementation-plan.md) defines the remaining sequence and a
mandatory integration gate after every milestone. R3 classes, declared congestion,
permissioned evidence and publication passed the fresh/upgrade dev1 gate. Work is
paused after R3, before R4; normal candidate CI remains owner-monitored.

**Parents.** Doc 10 §3 (tone, amended here), §12 C9A (deterministic delivery for consequential
information), §18.2 (IC traffic rides the propagation path; OOC never does), T15 (orders require
delivery; relays and store-and-forward extend the envelope later); doc 11 §5 (Kirkwood is already
"depot and relay"); `mush-propagation` (E7 cut 7: one physical context, two resolvers).

**Terminology.** A *pair* is two entangled halves manufactured together. A *half* is one end,
installed at a *node* (a relay station). A *link* is a live pair. The Exchange calls its stations
*gateways* in public; this document says node or half so that *gate* keeps meaning the travel gates
of doc 10 §3.

## 1. The problem and the ruling

Light-time makes the corridor unplayable if every message crawls: three to twenty-two minutes one
way from Earth to Mars, up to thirty-eight from Mars to Ceres. Universal instant communication is
worse: it deletes fog of war at the strategic scale, isolation, "the fleet is out of contact", and
every reason a relay could matter.

The ruling: **instant only between the halves of a pair, light speed everywhere else.** A message
travels at light speed from its source to the nearest node it can reach, instantly across the link,
and at light speed from the far node to its recipient. Nodes are significant deployments — a few in
the whole system, orbital or on airless bodies, owned, staffed, capacity-limited and cuttable.
Latency therefore becomes a function of **position relative to owned infrastructure**, not of raw
distance. That is what makes nodes targets, chokepoints and political instruments, and it is why the
rest of this document is about topology and ownership rather than physics.

Real entanglement cannot carry information (the no-communication theorem); this is a conceit either
way. Deriving every limit from the pairing story — halves made together, shipped apart, narrow,
fragile — makes the limits feel like physics rather than rules.

## 2. What already stands

| Standing item | Where | What it gives this document |
|---|---|---|
| C9A: deterministic delivery for consequential information | doc 10 §12 | The rule this document supplies numbers for |
| IC traffic rides a deterministic path; OOC is never constrained; owners see only telemetry that reached them | doc 10 §18.2 | The IC/OOC boundary — untouched here |
| T15: envelopes are queued, transmitted, delivered, acknowledged, expired or superseded; "relays, couriers, store-and-forward, interception, encryption and forged orders extend the envelope later" | doc 10 §13 | The envelope this document extends |
| `Medium::{Vacuum, Interior, Surface}` with `seconds_per_range`, `PropagationContext::time_of_flight`, `Envelope.path` of `HopRef` via `Envelope::relayed`, `MAX_ENVELOPE_HOPS`, store-and-forward for a ship in the in-transit room | `crates/mush-propagation` | The hop model a zero-cost link slots into; the constant-work discipline (the K lens) the routing step must keep |
| Strategic travel uses the moving sky and a passage plan frozen at departure; a journey is a record, not a ship coordinate | docs 18–19, `tacsim::sky`, `tacsim::port`, `mush-sim::journey` | Where stationary light-time and a ship's lag come from (§3.4); supersedes the original no-ephemerides baseline |
| The world timeline runs at a rate; scenes decouple from it | `mush-core::clock`, doc 10 6A | Minutes of IC lag compress between scenes and schedule inside them |
| Kirkwood Depot is "propellant depot and relay" | doc 11 §5 | One node already named |

**Amendment to doc 10 §3 (D-15-1).** "No FTL before gates" becomes "no FTL *travel* before gates,
and FTL *information* only over paired quantum relays — rare, orbital, owned and cuttable". If gates
arrive later they are the same physics scaled up, and the setting gets its technology through-line
for free.

## 3. The physics of the backbone

### 3.1 Pairs, halves, links (D-15-2)

A pair is manufactured as a unit at a **foundry**; at world start the only foundry is the Concord's
at Luna (§7). The halves are separated physically: one stays at the hub, the other is *cargo* on an
ordinary journey — weeks to the Belt, months to Titan — and cargo can be intercepted (doc 10 1A,
`mush-sim` journeys). A link is live while both halves are installed, powered and staffed. A
destroyed or lost half kills the pair; its partner is an orphan, and the link is replaced only by
manufacturing and shipping a new pair (§6.2).

A node holds several halves. The Luna hub holds one for every spoke (§4). A spoke node holds one:
its link to Luna.

### 3.2 Capacity and classes (D-15-3)

Links are **narrow**. Unlimited capacity would make the slow path pointless and knowledge ungraded.
A link carries the short, consequential traffic — orders, reports, market ticks, channel text,
mail — and queues it by **class** (§5.2). Bulk traffic — sensor imagery, full telemetry, manifests
in the large — goes by conventional laser at light speed, or by courier. The corridor therefore
learns *that* something happened at once and *what it looked like* minutes to hours later, which is
the graded knowledge the epistemic engine wants (`docs/scenes_factions_knowledge.md`).

### 3.3 Coverage and the last mile (D-15-10)

A node **covers** a set of places (doc 06 / doc 14 places, atlas nodes): the body it orbits or sits
on and the domains around it. Within coverage the last mile is the conventional hop from the
sender's place to the node — ground to a stationary orbit is a tenth of a second, the Martian
surface to Phobos a fiftieth — and it rides the existing `Medium` time-of-flight. Cut 1 does **not**
model occlusion geometry for uplinks (a far-side outpost is "in coverage" or not, by content), and
depletion of entanglement stock with its resupply journeys is deferred. Both are later hooks, not
omissions.

### 3.4 Ships under way (D-15-11)

A ship in passage has no coordinate (1A), and today an envelope addressed to it **waits**
(`mush_propagation::context::place_of`). With the backbone the wait gets a number: the ship's
light-distance to the nearer end of its route, interpolated from journey progress, plus that end's
node hop. A torch cutter mid-lane between Phobos and Kirkwood is minutes out of the loop, and its
commander knows it — the default state of every ship in the corridor, and the row that keeps the
setting honest. Only a **mobile half** aboard changes it (§7.2), and none exists at world start.

**As built (doc 18 IC9):** a passage's legs are frozen in its ticket's `PassagePlan` at departure
(doc 18 §4.8, D-18-46; `crates/tacsim/src/port/voyage.rs`). At P-A, R5 reads the moving sky's `Sky`
positions and the ship's frozen plan legs, and D-15-11 is honoured with the route's light distance
taken from the plan, not re-derived from a sky that has moved since.

### 3.5 The lag table

One way, IC time. "In coverage" assumes both ends inside a node's coverage and an empty queue. The
Luna hub means Earth's own traffic first crosses to Luna at light speed.

| Leg | Fallback (no backbone) | In coverage, via Luna |
|---|---|---|
| Earth surface → Luna | 1.3 s | 1.3 s (Earth uplinks to Luna conventionally) |
| Earth surface → Mars surface | 3–22 min | ≈ 1.4 s |
| Luna → Mars surface | 3–22 min | ≈ 0.1 s |
| Phobos → Ceres (two hops, one queue) | 8–38 min | ≈ 0.1 s plus the Luna queue |
| Earth → Ceres | 14–33 min | ≈ 1.4 s |
| Luna → Mercury | 4–12 min | ≈ 0.1 s |
| Earth → a Jovian moon (no node at start) | 35–54 min | 35–54 min |
| Earth → Titan (no node at start) | 67–92 min | 67–92 min |
| Ship mid-lane, Phobos ↔ Kirkwood | ≈ 4–19 min to the nearer end | the same, then the node hop |

Earth is a little worse off than Luna by design: the network is Luna's (§4).

**As built (doc 18 IC9):** the last row's interpolation reads the ship's frozen plan legs (doc 18
D-18-46): at P-A, R5 takes the route's light distance from the plan and the end bodies' `Sky`
positions.

**As built (doc 19 cut A.5, 2026-10-18):** the sky side of this table is live — `Sky::light_lag_s(a,
ta, b, tb)` (cut A.1; one-way, whole seconds up, over the Kepler positions) and `+chart`'s lag column
(cut A.3: every lane body's light lag from the port you stand at, e.g. Pallas from Ceres), so the
"fallback" column's ranges are now read off the moving sky rather than authored. R1 now records
installed nodes, pairs and coverage. R2 now routes stationary traffic through those
nodes with bounded FIFO capacity and timed fallback. R5 reads a ship's lag from the
sky at the end bodies and the ticket's frozen legs, as D-15-11 says.

## 4. Topology at world start: the Luna star (D-15-6)

Every world-start link terminates at **Malapert**, the Luna hub next door to the Concord Executive
at Shackleton. No spoke-to-spoke pair exists. The star is the Concord's deliberate instrument of
control, in the same spirit as the Registry: a Republic order to Ceres, a Freeport tip to Kirkwood,
a Co-op invoice to Phobos — all of it transits Luna's queue and Luna's eyes.

```text
                 Mercury (Solar Authority)
                          |
   Phobos ———————— MALAPERT (Luna hub) ———————— Ceres
                          |
                      Kirkwood
```

There are **five nodes, four pairs and eight halves**: four halves at Malapert and
one at each of its four spokes. Earlier references to "five halves" counted nodes.
The halves are assets inside doc 11 §5's regions, not regions of their own. Control kinds as in
doc 11 §5: **J** de jure · **F** de facto · **C** covert · **E** economic.

| Node (installed halves) | Hosted at | J | F | C | E | Contest at start |
|---|---|---|---|---|---|---|
| **Malapert hub** (four halves) | Luna, Malapert | Concord charter | the Exchange | the Concord | Concord treasury (the original build); the Exchange (upkeep) | none; under Concord Fleet's guns |
| **Phobos half** | Phobos, the Exchange compound | the 2312 Settlement's extraterritoriality clause, inherited by the Republic | the Exchange (own staff) | OTS wants a tap; whether it has one is a seed-pack secret | the Exchange | the Republic wants to nationalize it — the Concession's twin |
| **Ceres half** | Ceres station | Freeport licence, mirrored from the Settlement | the Exchange | the Squadron's intelligence section; the Tether has physical access | the Exchange; Tycho (the compound) | none declared |
| **Kirkwood half** | Kirkwood Depot | Freeport licence | the Exchange, hosted by the Co-op | OTS, via the pickets | Tycho (depot financing) | Republic pickets; the likeliest node to be cut in the Concession crisis |
| **Mercury half** (stub) | Mercury Solar Authority | Concord charter | the Exchange | — | the Concord | strategic map only (§10 q2) |

Consequences that fall out of the star:

- **No private channel between the Republic and the Freeport.** Two instant hops, one queue, one
  operator. Encryption (T15, later) hides content but not volume and timing, and metadata is
  intelligence.
- **The Luna queue is everyone's queue.** Congestion at one node — real or declared (§5.2) — delays
  the whole corridor.
- **Cutting Malapert partitions everyone from everyone** (§6.2). It is the one node the Concord's
  expensive navy unambiguously exists to guard.
- **Future pairs may bypass Luna.** A Phobos–Ceres pair, whoever makes it, is the strategic
  transformation of §7.1. The Concord treats that prospect the way it treats Registry divergence.

**As built ([doc 19](19-astronomy-engine.md) cut A.8, 2026-10-19; coverage promises below remain
lore until routing is built):** three trade-run ports stand on the moons, and the star covers two of them.
**Aitken Highport**, the Concord's drum in low polar orbit over the South Pole–Aitken basin, lies
under **Malapert's** coverage — instant to Phobos, Ceres and Kirkwood through the hub, 67–92
light-minutes to Titan (§3.5). **Voltaire Anchorage** on Deimos lies inside the **Phobos half's**
coverage: the Phobos compound's last mile reaches the Roads. **Ligeia Terminal** at Titan has **no
half** (§10 q2, option A): its board posts Luna's prices an hour and more old, and the Exchange quotes
the Titan Concession Company for a half every decade and the Company declines every decade (doc 11
§4.21). No half, link or coverage set is added; the table above and the lag table's Titan row stand.

## 5. The Malapert Exchange (D-15-7)

Doc 11 §4.11 holds the organization's seed entry (charter, principal, linkage). This section holds
its **powers** as mechanics.

### 5.1 Charter and posture

A Concord-chartered concern operating every half in the system with its own staff, under the 2312
Settlement's extraterritoriality clause — Concord-era paper the Republic inherited, like the
Concession, and resents for the same reason. Its stated posture is neutrality: it serves every
account. Its real interests are revenue, the monopoly (no private pairs), and never being *seen* to
choose a side. The Concord's leverage over it is informal (§10 q3): a request from Shackleton, not
an order, deniable on both ends.

### 5.2 Service classes and the congestion board

Every envelope carries a class; the class decides its queue position at each node.

| Class | Who | Waits by | Price |
|---|---|---|---|
| **Sovereign** | governments, by treaty | actual load | flat, by treaty |
| **Priority** | anyone who pays | actual load | a multiple of Standard (seed number) |
| **Standard** | anyone with a current account | **declared** load | per message-unit (seed number) |
| **Deferred** | anyone | off-peak only | free; IC hours |

The Exchange publishes a **congestion board** per link: the load it *declares*. Standard traffic
waits by the board; Priority and Sovereign wait by the truth. The board overstates the truth: the
gap is the upsell. The board is computed by Malapert's licensed scheduling mind (doc 11 §2, the
Compact); in whose interest it overstates is a fair question for a scene. Two facts therefore
exist for every link — `declared_congestion` (public,
operator-set) and the actual load (private, derived from real traffic) — and the gap between them is
exactly the kind of secret the epistemic engine is built to hold: discoverable by comparing one's own
timestamps across classes, by an insider, or by an OTS tap, and worth a scandal front when published
(§10 q5).

### 5.3 Service state: Served, Throttled, Suspended

Per principal — a government, an organization or a person — the Exchange sets a service state:

- **Served** — normal.
- **Throttled** — demoted one class: Priority becomes Standard, Standard becomes Deferred.
- **Suspended** — no backbone at all. The principal's traffic falls to the **fallback**:
  conventional laser and radio at light speed, or store-and-forward. A suspended Freeport is not
  mute; it is twenty minutes behind, and the corridor knows it.

Reasons, from common to rare: **arrears** (the most ordinary blackout in the setting — your
corporation's account lapses, your market data is twenty minutes stale, you lose the contract);
**incident** (an Exchange compound touched, its staff detained); **displeasure** (the Exchange's, or
its friends' — Luna's request). Every change of state is a ledger event with a stated reason; the
neutrality theater *requires* the audit line, which is what makes a political suspension expensive
for the Exchange too. The Exchange controls the backbone, never the physics: the fallback is always
there, slow.

### 5.4 What players buy

Relay time is the first natural product of doc 08's service market (E8, unbuilt): per-message class
upgrades, subscriptions, and for a government the Sovereign treaty rate. A corporation's standing
with the Exchange is a real asset: current, in arrears, throttled, suspended.

## 6. Blackouts and the nuclear option

### 6.1 The six blackouts

All six fall out of §3–§5 without new mechanisms.

| Blackout | Cause | Scope |
|---|---|---|
| **Coverage loss** | the uplink cannot reach a node: dish down, jammed, out of coverage | one place; common |
| **Node loss** | a node disabled | everything it covered drops to fallback |
| **Partition** | a node destroyed in a paired graph | the far side keeps talking to itself and loses the hub |
| **Denial** | the Exchange throttles or suspends (§5.3) | one principal; political, audited |
| **Interception** | the operator reads what transits its node | metadata always; content until encryption (T15) |
| **Transit isolation** | a ship under way (§3.4) | every ship in passage, by default |

### 6.2 The nuclear option (D-15-8)

Destroying a system's node before an attack is the corridor's nuclear option: legal nowhere,
condemned everywhere, and effective. The consequences the engine must carry:

1. **Pair death.** The orphaned half at Luna is scrap. Replacement is a new pair from the Luna
   foundry and a shipment — weeks to Phobos, months beyond — so the attacker buys a window of
   isolation measured in **months**, not hours, during which the system runs on fallback. That
   window is why it is nuclear: too large a consequence for anyone with a stake in the system.
2. **Universal condemnation.** A canon event every faction reacts to: the three governments unite
   rhetorically, the Exchange declares the perpetrator **anathema** (suspended everywhere, forever),
   Tycho voids their cover, and every front that measures legitimacy moves against them.
3. **The attention economy.** A terrorist actor does not care and may enjoy it. The faction
   simulation must therefore be able to hold an actor whose pools include **`attention`** — for whom
   condemnation is income. None is seeded at world start; the Tether's hard edge, a Pallas Syndicate
   cutout, or an unnamed Belt cell are the natural candidates when the story wants one.
4. **Malapert itself.** Cutting the hub partitions the whole system. It is the ultimate act, it sits
   under Concord Fleet, and the *threat* of it is the best argument the Concord has for keeping a
   navy it cannot afford to use.

Nodes are therefore attackable objects — station-class units in the `TacticalUnit` family
(`mush-tactics`: sections, facets, the damage pipeline) — and their destruction is a tactical fact
the canon ledger consumes (§8 R7).

## 7. End-game: private pairs and the first half afloat

### 7.1 Private pairs (D-15-9)

Manufacturing a pair is a faction capability: a foundry, scarce inputs, and know-how the Concord
guards. A faction that acquires it can make **low-bandwidth** pairs — a fraction of an Exchange
link — and place a private link between two of its own places: instant, undeniable, unreadable,
Luna-less. For the Republic and the Freeport a direct Phobos–Ceres pair is the end of the leash; for
the Concord, proliferation is existential and answered with a front. This is end-game content by
design: the industry chain (doc 08 production, `mush-industry`) is long, the inputs are
corridor-scarce, and the political cost begins the day the foundry is discovered. A faction that can
make pairs can also put one on a ship (§7.2), and pair manufacturing is the setting's precursor to
the travel gates of doc 10 §3. The foundry is lunar because the lunar pocket refined the pair
during the Interval (doc 11 §2), and the Compact's chartered fabs are the second leash: private
fabs sit beside private pairs as end-game content.

### 7.2 The first half afloat (D-15-4)

A capital ship may carry a half — a mobile node whose coverage moves with the hull. At world start
this is **theoretical**: the Concord has the foundry and the hulls, and a refit programme is a front
(doc 11 §6, "A Half Afloat") whose completion is a canon event with a recap. The first launch is a
big IC day, and whoever hosts the hull decides where instant comms go. A ship carrying a half loses
transit isolation for itself and everything in its coverage, becomes the most valuable target in any
engagement it joins, and dies with its link.

## 8. Engine backlog (R1, R2 and R5 implemented)

The R identifiers remain the scope map. [15a](15a-comms-relay-implementation-plan.md)
is the implementation sequence: R1 → R2 → R5 → R6a → R6b → R3 → R4 → R7 →
R8a → R8b → R9a → R9b → R10. Every cut has a mandatory integration stop.
R2 pulls forward stationary light-time from R5; R10 content is introduced alongside
its first consumers, beginning with a real Luna–Ceres pair in R1.

**R1 as built (2026-10-23):** `mush_sim::relay` owns the versioned, bounded
`relay:registry` document. `tacsim.relay.luna_ceres@1` installs the Exchange,
Malapert's endpoint in Aitken's service spine and Ceres's endpoint in its maintenance
room, with two physical halves and explicit room coverage. `@relay` provides staff
inspection and availability controls. Startup reconciles records against the real
objects and refuses contradictory evidence. The full Malapert compound and star
remain R10. See [15b](15b-comms-relay-cut-notes.md) for implementation, test results
and CI status.

**R2 as built (2026-10-23):** the existing propagation delivery pass now persists
stationary itineraries, frozen Sky light-time, shared FIFO pair capacity and
bounded forwarding. Orders, contact reports and returning ACKs use that path;
typed dispatch kinds keep messages out of standing-order supersession. Staff
inspect routes with `@prop/route`. The dev1 live gate passed; full CI status and
limits are recorded in 15b. The owner continued to R5 after the corrective dev1
rerun. Mail/channels, service classes and billing remain later cuts.

**R5 as built (2026-10-23):** the metric adapter reads the real typed passage ticket
and journey cursor, interpolates the frozen leg distance to its nearer endpoint,
and freezes each physical radio flight. Hull membership covers passengers and
nested rooms; disembarkation uses bounded forwarding. Missing tickets wait.
An aboard-only `+ship` estimate states one-way physical lag. A real Dray completed
the Ceres–Kirkwood–Stickney passage on dev1 with delayed traffic and ACKs, restart,
rate-change, ticket-loss and docking checks. See 15b for evidence and CI status.

| # | Item | Home | Shape |
|---|---|---|---|
| **R1** | **Link registry** | `mush-sim` (strategic topology); atlas nodes host relay facilities | Pairs; halves as placed objects; link state `Live / Degraded / Dead`; capacity; operator; class policy; coverage as a set of places. A monolithic record in the `unit:<obj>` style with boot reconciliation between binding and document. |
| **R2** | **Routing step in the delivery pass** | `mush_propagation::pass::delivery_pass` | Nearest reachable node by the existing context and `Medium` time-of-flight; a **bounded** walk over links (a `MAX_LINK_HOPS` beside `MAX_ENVELOPE_HOPS`); the far node; the recipient. Each hop is an ordinary `HopRef` via `Envelope::relayed`. Constant work: a miss and a hit walk the same branches, so a route lookup never leaks whether a node exists. |
| **R3** | **Class queue and the congestion board** | `mush_propagation::policy` plus an operator-set posture record | Class on the envelope; Sovereign and Priority wait by actual load, Standard by `declared_congestion`, Deferred off-peak. Queue and delivery stamps recorded on the envelope so the gap is discoverable. |
| **R4** | **Service state per principal** | interim: a staff/operator table; final: the ownership and permissions layer (doc 08) | `Served / Throttled / Suspended` with a reason. Suspended routes to the fallback (the existing conventional time-of-flight or store-and-forward). Every change is a ledger event. |
| **R5** | **Ship-under-way lag** | `mush-atlas` travel; `mush_sim::journey::CarrierJourney` | Progress times the route's light-distance to the nearer end, plus the node hop; replaces "waits" with "waits N" for a ship in the in-transit room. The ship-progress extension of the strategic metric adapter introduced for stationary routes in R2. |
| **R6** | **IC channels and all mail over envelopes** | `mush-comsys` (the `ic` flag); `mush-mail` | All mail transits, including quick mail, replies and forwards; no OOC mail path. OOC channels stay unconstrained. Arrival gates inbox/history/unread/notifications as well as live text. Discord consumes the authorized per-observer delivered stream. Split into R6a mail and R6b channels/bridges by 15a. |
| **R7** | **Nodes as structural units; pair death** | `mush-tactics` (the `TacticalUnit` family); `mush-canon`; `mush-faction` | A node is a station-class unit; destruction sets the link `Dead` and orphans the partner; a canon event; the condemnation consequence and the Exchange's anathema; an `attention` pool kind for actors who value notoriety. |
| **R8** | **The Exchange as an economic principal** | `mush-org`; `mush-economy`; doc 08 E8 | Tariffs by class, subscriptions, the Sovereign treaty rate; arrears → Throttled → Suspended on a cadence clock. The service market's first product. |
| **R9** | **Pair manufacturing and the mobile half** | a gated `mush-industry` recipe; a unit capability in `tacsim` | Foundry plus inputs plus know-how yields a low-bandwidth pair; a mobile half as an opaque capability on a hull; the Concord's anti-proliferation front. |
| **R10** | **Seed pack** | doc 11 §10, cut 11 | The Exchange organization, five nodes / four pairs / eight halves, two fronts, one principal; seeded incrementally with earlier cuts. Doc 11 §10's counts were updated in this change. |

Constants to keep straight: `Medium::Vacuum` at four seconds per abstract range unit and
`ADJACENT_RANGE = 2` are the tactical scale's placeholders. Under 15a, R2 brings
forward stationary strategic light-time; R5 adds ships from their frozen passage plans.

## 9. Decisions (owner, 2026-09-12 and 2026-10-23, binding)

| # | Decision |
|---|---|
| D-15-1 | Doc 10 §3 amended: no FTL *travel* before gates; FTL *information* only over paired quantum relays — rare, orbital, owned and cuttable. |
| D-15-2 | Paired links, not a mesh: a link is one manufactured pair, its halves shipped apart as cargo. |
| D-15-3 | Links are narrow; traffic is queued by class, and the paid tier is genuinely faster. |
| D-15-4 | A capital ship may carry a half. None exists at world start; the first launch is a canon event driven by a front. |
| D-15-5 | IC comsys channels obey lag when a speaker is out of coverage. OOC is never constrained (doc 10 §18.2 stands). |
| D-15-6 | Every world-start link terminates at Luna (the Malapert hub). No spoke-to-spoke pair exists at start; future pairs may bypass Luna. |
| D-15-7 | A ComStar-like operator, the Malapert Exchange, runs every half: it sells expedited delivery, publishes a congestion board that overstates load, and may throttle or suspend any principal at will — for arrears, incident or displeasure, its own or its friends'. |
| D-15-8 | Destroying a node before an attack is the nuclear option: condemned by all, anathema to the Exchange, months to replace. Terrorist actors may value the attention, and the faction simulation must be able to represent that. |
| D-15-9 | Building private, low-bandwidth pairs is end-game faction content. |
| D-15-10 | Cut 1 exclusions: no occlusion geometry for uplinks; no entanglement depletion or resupply; coverage is a set of places. |
| D-15-11 | A ship under way takes its lag from journey progress along its route, not from a coordinate. |
| D-15-12 | **2026-10-23:** all mail is IC and must transit. No OOC mail mode. OOC channels remain unconstrained. |
| D-15-13 | **2026-10-23:** all other recommended answers in §10 are accepted; prices remain seed data. |
| D-15-14 | **2026-10-23:** pause and integration-test after every milestone; 15a defines the gates and the stop before the next milestone. |

## 10. Owner answers (2026-10-23; all seven questions closed)

The owner accepted every recommended default except Q4, which received a new ruling.
These are design decisions. R1 now implements topology only; later cuts implement
the transport, commercial and world-content consequences.

| Question | Binding answer |
|---|---|
| Q1 — Names | Malapert Exchange; Postmaster Solveig Adebayo-Krishnan; Sovereign, Priority, Standard and Deferred. |
| Q2 — Mercury at world start | A: Mercury has a half; Venus, Titan, Pallas and Vesta do not. Aitken Highport uses Malapert's coverage and Voltaire Anchorage uses Phobos's. |
| Q3 — The Concord's lever | A: informal requests from Shackleton; the Exchange decides and owns the reasoned audit line. |
| Q4 — Mail | **All mail is IC and must transit the communications network. There is no OOC mail mode, including ordinary, urgent or staff-authored mail.** This supersedes both original alternatives; OOC channels remain unconstrained. |
| Q5 — The congestion board | A: mildly dishonest and undiscovered at world start. Publishing evidence can open the scandal front. |
| Q6 — Prices | Keep Standard per message-unit, Priority a multiple, Sovereign flat and Deferred free with an IC-hours wait. The numbers are versioned seed-pack data. |
| Q7 — First half afloat | A: a Concord Fleet hull refitted at Luna and sent to the corridor as the Belt Squadron's relief. No mobile half is active at world start. |

**Implementation cadence.** The owner also requires a pause and integration test after
each milestone. [15a](15a-comms-relay-implementation-plan.md) defines thirteen
integration gates, splitting R6, R8 and R9 and bringing real seed content forward.
Each gate includes its production composition, durable/restart path and live-server
evidence. Stop and report after the gate; continue on the owner's instruction.
Integration is never deferred to R10.
