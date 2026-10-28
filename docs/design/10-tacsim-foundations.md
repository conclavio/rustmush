# 10 — TacSim Foundations

> **2026-10-28 continuation:** [doc 27](27-ic-field-operations-design-and-survey.md) records
> the owner's persistent damaged-unit, physical salvage, detachment and permission rulings,
> together with the accepted DT/H1/H2 survey. They refine 3A and T3: unit owner and landholder
> each have lawful recovery entitlement, and non-owner title uses the public claim/contest
> process. There is no player permadeath, superseding the former campaign-lethality exception
> in §18.6. Simulator convenience rules are not IC world policy.

**Draft 0.5 — 2026-09-09** (Draft 0.4 plus the addendum **T17**, §22, owner's ruling). The definitional record for the TacSim programme (build units E7+):
the owner's scope note, the foundational survey with the **locked decisions 1A–7A**, the
character-first survey with the **locked decisions C1A–C30A**, the accepted addenda **T1–T17**,
the Draft 0.2 review's **sharpenings R1–R7** (§21, accepted), the remaining qualifications, and
what all of that implies for the economy programme already in flight (docs 08–08e). This is not
a build plan; the E7 plan will cite it the way 08c cites 08 and 09.

Companion docs: 06 (object behaviors, spatial domains), 07 (NPC control and tactical AI), 08–08e
(economy, organizations, ownership; E3–E6 plans and owner answers), 09 (rules-engine contracts —
`ClockRef`, C4 time windows, C6 semantic events, C8 reservations, K-series).

---

## 1. Premise

A **solar-system industrial warfare sandbox** in which every battle has an economic cause and every
supply chain creates potential conflict. Three pillars:

- **BattleTech / MUX** — persistent machines, crews, loadouts, damage, salvage, territorial fights.
- **EVE** — corporations, extraction, logistics, manufacturing, contracts, local markets.
- **The Expanse** — distance, orbital geography, political fragmentation, vulnerable
  infrastructure, ships as inhabited places.

The governing rule: **never simulate the entire solar system at tactical resolution.**

## 2. Four scales, one world

| Scale | Gameplay |
|---|---|
| Interior | Bridges, hangars, boarding, social scenes, repairs |
| Tactical | Ground, atmosphere, orbital engagements |
| Operational | A moon, planetary orbit, convoy route, station vicinity |
| Strategic | Interplanetary travel, markets, production, fronts, politics |

Transitions connect the scales:

```text
Ceres mine → cargo loaded into hauler → interplanetary journey → interception near Phobos
→ local space engagement → surviving carrier docks → mechs unload into a Mars ground domain
```

Interplanetary travel is a **journey** with time, propellant, cargo, risk, and possible
interception — not a coordinate map ticking continuously. This matches the existing domains,
carrier interiors, custody, routes, and control stations (doc 06, `mush-sim`). Gates, later,
become new strategic-route edges into new regional graphs; nothing fundamental is replaced.

## 3. Setting and technology tone

Anchor **c. 2350**, "hard-ish, playable SF": no FTL *travel* before gates, and FTL *information*
only over paired quantum relays — rare, orbital, owned and cuttable (doc 15); no reactionless
drives; no artificial gravity beyond thrust and rotation; no general-purpose energy shields; mature
fusion power and propulsion; military torch drives fast but expensive and conspicuous; civilian
shipping slower and efficient; armor, thermal management, electronic warfare, point defense,
missiles, kinetics and lasers all matter; compute scarce against twenty-first-century expectations,
frontier-class minds few, licensed and located (the Compact); automation excellent but never in
authority — every automated act runs under a liable principal's warrant (the Warrant; the history
in doc 11 §2) — so people remain politically, economically and tactically decisive. Travel takes
IC days to months by vehicle, route and urgency. Mercury has sunward, night, polar and
moving-terminator operating regions (its long solar day makes thermal conditions campaign
geography), not a fixed dark side.

## 4. Geography and comparative advantage

Every region exports something and badly needs something, or every location is a reskinned
self-sufficient city.

- **Earth/Luna** — population, finance, biotech, high-end manufacturing, established naval power;
  imports bulk material.
- **Mars/Phobos** — heavy industry, shipbuilding, military manufacturing, large settlements;
  imports volatiles and some organics.
- **Ceres** — water, propellant production, logistics, Belt finance and politics.
- **Vesta/Pallas** — metals, industrial feedstock, isolated claims, extraction conflicts.
- **Venus** — aerostat settlements, atmospheric chemistry, carbon feedstock, extreme-environment
  research; surface operations are specialized expeditions.
- **Mercury** — solar power, metals, high-temperature industry, polar infrastructure.
- **Titan** — hydrocarbons, nitrogen chemistry, outer-system staging, distant communities.
- **Orbital structures** — shipyards, refineries, transfer stations, habitats, markets, fuel depots,
  military chokepoints.

Commodities stay deliberately broad (survey 4A fixes the list at ten families). Facility capacity,
skilled labor, power and shipyard time are **reservations**, not cargo (T4).

## 5. The economic loop

```text
survey → claim → extract → haul → refine → manufacture → deploy → fight
       → damage / capture / salvage → demand
```

Every arrow is a role for another player: surveyors, diplomats (licenses), miners, traders and
financiers, haulers, escorts, raiders, engineers, mercenaries, salvagers. NPC corporations provide
baseline liquidity and activity at mediocre rates (survey 7A: **funded**, never infinite). Markets
are **local**: orders belong to a station, settlement or facility; buying remotely does not
teleport goods; funds and goods are reserved at order time; delivery is a separate physical act or
contract; governments and station owners impose fees, access rules, embargoes and licensing; market
*information* may be widely visible while inventory stays local (§14, projection).

## 6. Organizations and politics

Six concepts, kept separate: **government factions** (sovereignty, law, diplomacy, fronts,
territory, strategic goals); **NPC corporations** (industrial actors with real accounts,
facilities, assets, agendas); **player corporations and mercenary units** (player-created, on the
same accountable machinery); **operation groups** (temporary parties of characters for one sortie
or contract — the E6 party subsystem); **formations** (persistent order-of-battle groupings of
units); and **task forces** (operation-scoped containers assembling formations, detached
subformations, individual units and support assets under one temporary command structure). A
faction may claim Mars, a corporation own the refinery, a rival hold de facto security control, a
union or intelligence service covert control — all at once; this is the existing de jure / de
facto / covert / economic control distinction (doc 08, `mush-faction`).

Player-created organizations are **real** here (unlike the conservative WoD default): a charter fee
and sensible caps rather than staff creating every corporation. Templates: corporation, mercenary
command, cooperative, shipping concern, mining syndicate, political association. A corporation
receives scoped divisions (operations, logistics, production, payroll, intelligence); roles decide
who may spend, requisition, approve construction, assign units, or publish contracts (`mush-org`
scopes and capabilities).

## 7. Contracts create group play

Player corporations publish hauling contracts, escort missions, mining rights, survey commissions,
salvage rights, equipment procurement, repair and refit orders, convoy operations, bounties,
facility defense, territorial assaults. The employer escrows money and possibly cargo; an ephemeral
operation group forms, brings or receives assets, performs, and settles against **recorded
outcomes** (T2). NPC strategy and LLM-authored campaigns use the same legal templates, budgets and
pressure limits — no separate mission system (E6 answers: LLMs draft, authorities commit).

## 8. Units, combined arms, space combat

> **Scoped 2026-09-16 (owner, [16 §2](16-combat-arena.md), W2 and W10).** On the tactical map the sky
> is a laser kill zone: no VTOLs, aircraft or drone swarms take part in ground fights, and elevation is
> terrain, never flight. Strategic and orbital aerospace below are untouched; "drones scout" reads
> "drones scout until a laser sees them".

**Unit design** is BattleTech in texture on the existing compositional model (`TacticalUnit` —
sections, facets, scalar cells, installed components, separately titled major equipment;
`mush-tactics/src/unit.rs`). A chassis supplies mass, volume, structural sections and exposed
facets, power, cooling, crew stations, control bandwidth, hardpoints and internal space, mobility
modes, environmental envelope. Components consume or alter those: engines and reactors, armor,
weapons and ammunition, sensors and EW, heat rejection, life support, cargo and troop capacity,
bays and launch systems, atmospheric or vacuum equipment. Balancing constraints: mass, volume,
power, heat rejection, crew, control bandwidth, manufacturing complexity, maintenance burden,
environmental rating — a design cannot maximize everything. Field swaps, yard refits and full
rebuilds are distinct operations; designs are versioned blueprints and eventually licensable
corporate assets (E6 catalog packs + entitlements). Work orders reserve materials, money, capacity,
destination custody and title, then build and deliver through a ruleset driver (doc 08 §work
orders).

**Combined arms**: mechs excel in broken terrain, low-infrastructure colonies, rapid redeployment,
mixed environments; tanks are cheaper on suitable ground; infantry captures facilities, boards,
sabotages, holds; drones scout, screen, relay, absorb risk; artillery shapes movement and supply;
VTOLs give atmospheric mobility; aerospace craft link atmosphere and orbit; carriers create range
and group roles. Territory is captured through objectives and control, not annihilation.

**Space combat** is sensor- and decision-driven: detection and classification, emissions control,
intercept geometry, missile salvos, point defense, EW, thermal limits, maneuver reserves, damage
control, surrender / capture / boarding. Strategic travel sets encounter geometry; combat runs in a
bounded local-space domain. Large ships are multicrew (helm, tactical, EW, engineering, flight ops,
command): continuous controls are latest-order-wins; firing, launching, docking and damage control
are bounded discrete intents (doc 06's continuous-control / discrete-action split; doc 09 K6).

The player identity remains a **character**, not an asset portfolio. A character has one physical
location and one active station. Ownership, station billets, active control and command authority
are separate facts: a character or organization may own many units and roster a character on many
stations, but the character directly operates only the bounded channels of one active station.
Commanders affect larger forces through delivered mission orders and doctrine, not by manually
driving every subordinate unit. NPCs and automation make solo capital-ship operation viable;
additional player crew contribute parallel judgment, local competence and resilience rather than
an unexplained ship-wide attendance bonus (§§17–19).

## 9. Economic granularity

Track what creates decisions: ammunition at magazine or sortie scale; propellant per journey or
deployment; damage and replacement components; maintenance aggregated by sortie or operational
period; life support for endurance and stranded expeditions, not meals; payroll periodic and
automatic (obligations on cadence clocks); industrial inputs and cargo **exact**; routine background
consumption aggregated.

## 10. Persistence and risk

Meaningful loss matters; 24/7 offline destruction is poison. Deployed assets are genuinely at risk;
docked assets inherit their facility's and vault's security; attacking major offline infrastructure
requires a declared operation or campaign window; territorial change happens through visible fronts
and clocks; linkdead units get configured fallback control, never immunity; insurance and salvage
soften loss without erasing it; every player can recover to a basic employable state. Losing a
beloved mech must follow a legible deployment decision, never an invisible timer.

## 11. First playable corridor

All major bodies on the strategic map; deep implementation of **Mars–Phobos–Ceres** only: a Mars
ground theater, a Phobos shipyard and orbital domain, a Ceres mining/freeport region, one
interplanetary route, one refinery, one open market, one player corporation, one government faction
and one rival, mining drones, a hauler, a patrol craft, a dropship, infantry, a tank, a mech.

Proving story: extract water and metals near Ceres → hire players to haul to Phobos → fight or
evade an interception → manufacture or refit a ground unit → load it aboard a carrier → deploy to
Mars → complete a ground operation that changes control or access → salvage, repair, pay, repeat.
If that loop is fun, the rest of the system is content and variation.

**As built (doc 18; [doc 19](19-astronomy-engine.md) cut A.8, 2026-10-19):** the trade run's deep
implementation now spans **five ports** on lane v8: Ceres Freeport and Stickney Yards on the
corridor, and the first ports on the moons — Aitken Highport (Luna), Voltaire Anchorage (Deimos) and
Ligeia Terminal (Titan) — with six direct routes from or to them (doc 19 §3.8, D-19-19; doc 11
§4.13–§4.23; 18a §5–§7). The Mars ground theater, the refinery recipe, the open market and the units
stay as this section scopes them; every other body is still the strategic map.

---

## 12. Foundational survey — decisions (owner, 2026-09-03, binding)

| # | Question | Decision |
|---|---|---|
| 1 | Propulsion and travel time | **1A** abstracted fusion routes |
| 2 | Combat cadence | **2A** continuous pulse combat with persistent orders |
| 3 | Asset loss and recovery | **3A** risk-bound full loss with a recovery floor |
| 4 | Resource and production granularity | **4A** ten broad commodities, short production chain |
| 5 | PvP declarations and vulnerability | **5A** risk zones plus declared infrastructure operations |
| 6 | Scene time during travel | **6A** journey clocks never pause; arrival is a scene time boundary |
| 7 | NPC corporation funding | **7A** funded corporations with bounded capitalization |

**1A.** A strategic node-and-route graph. Routes use distance, broad alignment conditions, drive
performance, payload and propellant; no continuous ephemerides. Local orbit and nearby moons: IC
hours; inner system: days to weeks; Belt: weeks; Titan and the outer system: weeks to months.
Default campaign compression ≈ one OOC hour per IC day. Journeys run in the background while
characters play aboard interiors; transfer conditions modify routes rather than closing
destinations. Rejected: simulated ephemerides (depth at the cost of long non-opportunity and
tooling), fast node travel (kills logistics, regional identity, convoys, blockade).

**2A.** Deterministic logical pulses: ground/atmosphere ≈ 1–2 s, space ≈ 5–10 s. Heading, thrust,
speed and sensor posture persist until changed; fire, launch, jump, dock, eject and damage control
are bounded discrete actions. Players command rather than spam movement verbs; crew stations divide
control; NPC and link-loss controllers use the same legal actions and information; rules resolve
from recorded intents and RNG, allowing replay. Turn-driven mode remains for tutorials and
facilitated scenes. Rejected: WEGO (multicrew becomes waiting rooms), sequential initiative (scales
poorly to drones, combined arms, fleets).

**3A.** Deployed assets can be disabled, captured, salvaged or destroyed permanently; protection
follows custody and decisions, not universal insurance. Most defeated units become disabled or
abandoned before unrecoverable; recovery needs physical access, capacity and transport; captured
units keep title and provenance until legally transferred, abandoned or written off (T3); docked
assets get facility, vault and 5A protection; insurance pays a configurable fraction after delay
with exclusions; blueprints, skills and membership survive loss; characters face injury, ejection,
rescue, imprisonment or ransom, with **no player permadeath** (doc 27 Q-H12); the recovery floor is employment,
crew positions, leased equipment or loaners (T7). Rejected: repair-only loss (weakens capture,
salvage, insurance, replacement production), unrestricted full loot (rewards timezone coverage).

**4A.** Ten exact commodity families — raw: metals, volatiles, carbon feedstock, strategic
elements; refined: structural alloys, composites, electronics, propellant; operational: industrial
parts, munitions. Energy, skilled labor, refinery throughput, drydock and factory time are reserved
facility capacities (T4). Deposits vary by yield, quality, accessibility, location; quality and
provenance modify commodities (lots, T5); chains ≈ three transformations deep; cargo and industrial
inputs exact; ammunition economic at magazine/rack/lot scale while weapons track shots internally;
propellant settles per journey or sortie; maintenance and spares aggregate per operational period;
named major components may be individually titled. Rejected: deep EVE chains (needs a much larger
population), four abstract points (kills shipping, geography, piracy, specialization).

**5A.** Mobile assets accept the risk of the space they deploy into; persistent holdings get
scheduled, legible vulnerability. Core jurisdictions: hostile action needs a war, sanctioned
operation, mutually accepted scene, or explicit crime with severe automated response. Frontier and
contested space: deployed ships, extractors, convoys and field units may be attacked without
notice. War zones: open combat between recognized hostiles. Docked or properly vaulted mobile assets
cannot be surprise-destroyed. Habitats, stations, factories, shipyards and permanent extractors
require a **declared operation** (T6): objective, attacker, eligible defenders, stakes, start
window, duration, consequences; owners choose bounded recurring vulnerability windows; ≈ 24–72 h
notice for major operations; attackers commit a bond, supplies or force; defenders may pre-stage NPC
forces and standing orders; failing to log in does not make a structure invulnerable but its
declared defenses participate; success advances fronts and control records, it does not transfer
every asset in a region; corporation-wide war declarations broaden legal hostility after notice.
Rejected: declared-only PvP (piracy, scouting, escorts, emissions control become theatrical),
EVE-style universal exposure.

**6A.** A scene cannot hold or delay a journey. At arrival: (1) the journey commits at its scheduled
campaign time; (2) the carrier changes operational location; (3) active onboard scenes receive a
typed arrival event; (4) their local time is advanced to **at least** the arrival timestamp before
further actions; (5) the scene continues as a new chapter, keeping participants and transcript.
This needs a **temporal checkpoint / time floor on the scene**, not a prose notice: today a scene's
IC time is `ic_started_at + (round − 1) × round_seconds` (`mush-scenes/src/data.rs`
`scene_ic_time`), so a journey arriving days later would otherwise leave later scene events stamped
before arrival. Rejected: scenes hold arrival (lets an open scene delay contracts, interceptions,
markets and strategic events).

**7A.** NPC corporations have real accounts, inventories, obligations and budgets; their strategic
controllers spend only those. Funding sources are explicit: world-start capitalization, sales and
completed contracts, government subsidies, shareholder or parent investment, loans, bounded
market-liquidity facilities, emergency recapitalization as an attributed staff/system action. NPC
market makers get explicit cash, inventory, spread, volume and loss budgets; "mediocre prices" is a
trading policy, not a substitute for funding. Rejected: infinite system market makers with caps
(corporations cannot fail, run dry, or respond to war). This is E6's "never from an uncontrolled
faucet" rule applied to NPC corporations.

## 13. Addendum — accepted decisions T1–T7 (owner, 2026-09-03, binding)

**T1. Strategic route cadence.** Route cost and travel duration are deterministic functions of the
route definition, drive and payload, declared departure, propellant plan, and the body-pair
**alignment band on a campaign cadence**. No wall-clock randomness changes a contract after
publication; a hauling offer freezes its route assumptions, expected arrival, deadline and
late-delivery terms.

**As built (doc 18 IC9):** applied through the passage planner (doc 18 §4.8, D-18-43): a departure
plans its whole passage over the sky at its departure instant, and the burned ticket freezes that
plan — legs, seconds, burn and crew-days — so a later rate or sky change never alters a passage in
flight (`crates/tacsim/src/port/passage.rs`, `voyage.rs`; `CarrierJourney.leg_s`). Doc 18 Draft
0.1's "deferred, not denied" note on this decision is closed.

**T2. Tactical/economic boundary.** **No economic transaction occurs inside a combat pulse.**
Tactical domains mutate combat state (ammunition, heat, damage, position, control) and emit typed
facts at boundaries: sortie started/ended; journey arrived/failed; unit disabled, abandoned,
captured, destroyed; cargo delivered; docking completed; salvage recovered. Contracts, maintenance,
insurance and logistics consume those facts afterward. Combat RNG belongs in the mechanical
action/domain journal, not the economic journal; the economy records only the resulting authorized
settlement and its causal fact.

**T3. Capture, abandonment, salvage.** Capture changes custody or possession, **not title**. Valid
title transitions: voluntary owner transfer; owner write-off or abandonment; adjudicated
forfeiture; public salvage claim after a contest window; insurer acquisition under frozen policy
terms. Owner abandonment is a **direct attributed asset action** (it need not masquerade as a
bilateral contract); forfeiture and salvage claims use contracts because they need deadlines,
notice, possible contest, and an adjudicator.

**T4. Heterogeneous reservations.** Adopt the **C8 resource-generic reservation coordinator**; do
not turn the economic `Hold` into a universal abstraction. Money keeps economy holds; factory and
drydock capacity use industry-owned reservations; cargo, components and commodities use
inventory/custody reservations; a work order references all required reservations and consumes
them atomically. A refinery slot is not a bank balance, though both need reservation identity,
expiry, ownership and idempotent consumption.

**T5. Commodity lots.** Use lots whenever grade, source, legal status or provenance matters.
Splitting creates child lineage; merging creates a new blended lot referencing all parents. The
active representation may summarize or coarsen provenance, but **the immutable lineage is never
discarded**. Fully homogeneous commodities may use fungible accounts where provenance is
irrelevant; the ruleset or recipe decides which representation applies.

**T6. Declared operations.** A structure operation is **contract-shaped but not necessarily
world-visible**. Visibility classes: public; eligible defenders; participating organizations; covert
existence with a visible vulnerability warning; staff/audit only. "Declared" means every party
entitled to defend receives sufficient notice and knows the window — not that the attacker's plan or
identity is published. The bond, window, eligible defenders, legal targets, objectives and
consequences **freeze at activation**; tactical facts settle the result into front and control
changes.

**T7. Recovery floor.** A lease encumbrance is part of the floor, not the feature. A recovery
provider needs a funded NPC or government program, an inventory or production source for loaner
units, eligibility rules, a lease contract, operating restrictions, and return/loss/upgrade terms —
so a bankrupt player keeps participating without conjuring an asset or erasing loss.

## 14. Remaining qualifications (owner)

- **The knowledge dependency is split**, not one prerequisite: (a) a deterministic **contact and
  observer-projection layer** is required before meaningful space combat; (b) public market
  information needs an explicit **safe projection**, not the full belief engine; (c) rich NPC
  beliefs, inference, misinformation and relationship-sensitive pricing follow later.
- **Insurance resolves ordinary cases deterministically**: a policy inspects recorded facts
  (coverage active, unit deployed in a covered zone, destruction fact valid, exclusions absent) and
  settles without staff; a named adjudicator is needed only for ambiguity or dispute.
- **Crate boundary**: `mush-economy` (accounts, money holds, contracts, title, settlement) · a
  generic **C8 layer** (heterogeneous reservation coordination) · **`mush-industry`** (deposits,
  extraction, facilities, recipes, capacity, production) · **`mush-market`** (listings, order
  books, quotes, public projections — the neutral crate E6 already names) · `mush-sim` /
  `mush-tactics` (journeys, domains, tactical facts) · **`ruleset-tacsim`** (propulsion, resources,
  recipes, combat formulas, insurance and warfare policy) · a **campaign/warfare layer** (declared
  operations, fronts, territorial consequences).

The central reading stands: TacSim is mostly a rich composition of existing foundations; the
genuinely new work is **industry, market projection, campaign warfare, and tactical perception**,
not another economy engine.

---

## 15. Implications for the economy programme (E4–E6)

Recorded here so the in-flight plans pick them up; each is small.

- **E4b2 (contracts live).** Keep `EconomicVisibility` extensible toward T6's audience classes — at
  minimum an *audience list* variant so an operation contract can be visible to eligible defenders
  without being public; the existence-hiding renderer treats a contract id like an account id for
  non-audience viewers. T3's owner abandonment is a unilateral title effect (title → none, custody
  unchanged) — an `AssetEffect` variant, cheap in E4b2 or first thing in E7; the salvage claim that
  turns possession into title is then an ordinary contract with a `Before` contest deadline and a
  named adjudicator. Insurance is a contract template (fact condition + `After` delay + exclusions
  as further conditions), dispute-only adjudication — no new machinery.
- **E4c (facts, events).** The `FactProvider` seam must be shaped for T2's boundary facts from
  `mush-sim`/`mush-tactics` (sortie, journey, unit state, delivery, docking, salvage), not only for
  jobs; `ExternalRef { provider, kind, id, revision }` needs a stable id space per domain journal.
  The economy's causal reference points *at* the mechanical journal (T2), never duplicates it.
- **Cadence clocks.** T1's alignment bands and §9's per-sortie maintenance are cadence clocks whose
  source is the sim: `CadenceSource::Manual` advanced by `mush-sim`/`mush-tactics` at journey and
  sortie boundaries (`advance_manual_cadence` exists; the hook from the sim does not).
- **Holds stay money-only (T4).** E4's `Hold` is doc 09 C8's *money* specialization and must not
  grow a resource field. Industry reservations wait for the C8 coordinator, or `mush-industry` ships
  an interim record **shape-conformant to C8** (the 08c §12 conformance strategy) so no second
  semantics diverge.
- **E6 (Shadowrun economy; `mush-market`).** T5 lots refine E6's "stack" representation: the
  catalog policy chooses lot vs fungible per commodity; lot split/merge lineage is a `mush-market`/
  inventory concern the E6 plan (08f) must define once. §14's **safe market projection** replaces
  "the knowledge model" as E6c's gate — a real scheduling gain. 7A confirms E6's faucet rule for
  NPC principals: system principals with budgets and obligations, never unlimited accounts.
- **E5/E6 recovery.** T7's recovery provider generalizes E6's loaner idea; leases (`Encumbrance::
  Lease`) are the settlement primitive.
- **`mush-scenes` (6A).** A scene gains a monotonic **IC time floor** that `scene_ic_time` respects;
  journey arrival raises it and emits a typed arrival event. Doc 09's `SceneTurn` clock is the
  natural carrier; this is E7 scope but the field is worth adding when scenes are next touched.
- **Consent shapes from the character-first decisions (R6, as amended).** A T10 proxy-seat
  invitation is a **consent grant** — a campaign/control record sharing E4b2's consent-hash helper
  (station, channels, mission order, rules of engagement, window, revocation frozen at acceptance;
  the account reference audit-only) — not an economy contract, because an OOC proxy player is not
  an in-world principal (10a Q10/Q19); an E4 contract is linked only where a fictional principal has
  economic terms. C15A's readiness obligation is an E4b1 obligation on the operation cadence with success
  notifications off. C18A's passage contract is a deliverable-free contract granting bounded
  onboard rights. None needs new engine work.
- **Participation facts (R6, T13).** "An absent character earns nothing" means settlement splits
  index on *participation*, which E4b2's outcome-indexed terms already support once `mush-sim`
  emits a served-station fact (`sim.station.served { actor, station, from, to }`) — register it
  as an E4c fact kind next to the T2 boundary facts. Kill, salvage, and delivery credit are the
  same shape.
- **Account-distinctness is a predicate, never a principal (R3).** C23/C24 need "are these
  characters account-distinct" at roster lock and side assignment; C1A forbids an account
  principal. Core (or `mush-org`) exposes a boolean oracle over character ids; contracts and the
  party subsystem call it and store only the verdict. The private account id never enters a
  journal, contract, or canon record.
- **Character switching aboard (R5, C22).** A character left aboard after a switch is present and
  vulnerable, so E5's attention budget and non-response semantics (08d) govern prompts to them;
  the two rulesets share one fallback rule rather than inventing two.

## 16. E7 definition and implementation backlog

Definition decisions still open before an E7 plan can be drafted:

1. **Extraction and yield model** — deposit record, survey resolution, depletion, claim tenure,
   licensing as entitlements; production as scheduled, journaled lot creation (doc 09 C8 already
   lists production and upkeep among idempotent scheduled actions).
2. **Lot representation** — lineage storage, coarsening rules, legal-status flags (stolen,
   embargoed, licensed), and how fungible accounts and lots interconvert.
3. **C8 interim** — whether `mush-industry` waits for the kernel coordinator or ships a
   shape-conformant interim; what "consume atomically" means before K3 durable atomicity.
4. **Contact and observer-projection layer** — the minimum deterministic sensor/contact model that
   space combat and emissions control need, separate from the belief engine.
   **R4:** merged with item 14 into one *propagation layer* — sensing and consequential
   communications are both "what has reached whom" and must share the substrate.
5. **Campaign/warfare layer ownership** — whether declared operations live in `mush-faction`, a new
   crate, or the ruleset; freeze-at-activation semantics; the bond as an escrow hold; consequence
   events into fronts and control.
6. **`ruleset-tacsim` configuration** — propulsion tables, route graph, commodity list, recipes,
   combat formulas, insurance and warfare policy; the corridor content list (§11) as data.
7. **Recovery provider** — program funding, eligibility, restrictions, upgrade terms (T7).
8. **Time compression** — the campaign clock's OOC↔IC ratio, its interaction with obligations
   tuned for WoD/Shadowrun cadences, and `TimeJump` forgiveness thresholds.

Accepted requirements which still need build-plan decomposition:

9. **Carrier journeys** — replace the present `Journey` requirement that its traveler be wholly
   sim-unbound with a carrier-root invariant: the root is not directly placed in a live domain
   while traveling, but its station infrastructure, anchored interiors, occupants, mounts, cargo
   and carried craft remain coherent; define promotion into and return from an interception domain.
10. **Active-station focus** — add an actor-side active focus and lease grant invariant above the
    existing host-scoped station assignments and per-channel control leases; define local and
    remote reach, traversal, safe-boundary handoff and republished command-subject links.
11. **Formation and task-force state** — persistent administrative trees, operation-scoped command
    overlays, unique operational parentage, ruleset-authored formation kinds and `mush-org`
    authorization without importing military vocabulary into the organization core.
    **R1/R2:** a task force is the force half of its operation record (T6), or linked one-to-one
    with it; a unit is in at most one operation at a time; every formation node binds to exactly
    one owning org scope so `force.*` capabilities authorize through the existing machinery.
12. **Human-directed NPC control** — distinguish the in-world actor from the decision driver;
    invitation, acceptance, scope, guardrails, dual attribution, revocation and deterministic
    fallback for operation-scoped OOC proxy seats.
13. **Account integrity identity** — an immutable private account id used for one foreground focus,
    account-distinct quorum/reward rules and one-side-per-conflict enforcement, while accounts
    remain neither economic nor fictional principals.
    **R3/R7:** exposed only as a boolean distinctness oracle; stored opaque, audit-only; never in a
    journaled, contract, or canon record.
14. **Delivered command state** — versioned order envelopes, communications reachability,
    queued/transmitted/delivered/acknowledged state, supersession, expiry, relays and the rule that a
    unit obeys the newest valid order it has actually received.
    **R4:** delivery rides the same propagation layer as sensing (item 4).
15. **Crew, survival and participation** — named decision NPCs plus aggregate complements,
    readiness obligations, station-local competence, evacuation/capture/rescue facts, scheduled
    musters, open crew calls and contribution-aware settlement.

---

## 17. Character-first identity — governing model

TacSim is character-first even though characters and organizations may own and operate many
assets. The player is normally the character who occupies a place in the world; neither an account
nor an ownership record is a disembodied tactical actor.

| Concept | Binding meaning |
|---|---|
| Account | Authentication and competitive-integrity identity; never an in-world principal |
| Character / named NPC | Occupies space, perceives information, incurs risk and performs fictional actions |
| Ownership | Economic title; does not itself grant present operational control |
| Billet | Durable authorization or roster assignment to a station; a character may hold several |
| Active station | The character's one current operational focus and its bounded control channels |
| Order | Delegated intent that becomes authoritative for a recipient only when delivered |
| Decision driver | Human, deterministic AI or LLM-assisted planner supplying choices for an actor |
| Automation | Non-person controller providing continuity, routine execution and safe fallback |

This requires two explicit corrections above the current implementation:

1. `mush-sim` station assignments are host-scoped today. An actor may be assigned on several units,
   and its projected command-subject links include every placed unit it crews; seats on one unit
   union their channels. Preserve those records as **billets**, but add an actor-side active focus
   so only one station supplies live human control.
2. A durable `Journey` currently moves a plain object and rejects any traveler that is itself a
   domain, interior, mount or station host. TacSim needs a carrier journey: the carrier root travels
   while its anchored interior topology and everyone aboard remain playable and move with it.

An account may own multiple characters, and a character or organization may own multiple units.
Neither fact expands the character's body, perception or active control surface.

## 18. Character-first survey — decisions (owner, 2026-09-03, binding)

All recommended answers were accepted, with C6, C7/C29, C8 and C14 sharpened by T8–T16 (§19).

| # | Question | Decision |
|---|---|---|
| C1 | Account as an in-world principal | **C1A** private integrity identity only |
| C2 | Station assignment | **C2A** many billets, one active station |
| C3 | Station capability | **C3A** ruleset-bounded channel bundle |
| C4 | Station reach | **C4A** physical console or valid remote link |
| | **As built (doc 18 IC9):** C2A–C4A stand; the Dray's `+ship/undock` (and the sailing plan's queued depart) bypasses the station-channel gate until P2, by doc 18 R3: an offline departure cannot wait on a live seat. | |
| C5 | Station switching | **C5A** traversal plus safe-boundary handoff |
| C6 | Fleet and task-force representation | **C6A** operation-scoped task-force container over formations/assets |
| C7 | Command of subordinate units | **C7A** structured mission orders |
| C8 | Changing orders | **C8A** versioned replacement at decision boundaries |
| C9 | Communications latency | **C9A** deterministic delivery for consequential information |
| C10 | Absent-owner visibility and action | **C10A** administration plus legitimately delivered telemetry |
| C11 | Large-crew representation | **C11A** named decision-makers plus aggregate complement |
| C12 | NPC performance | **C12A** lower decision bandwidth and adaptability, not a type penalty |
| C13 | Solo capital-ship operation | **C13A** one player plus funded crew and suitable automation |
| C14 | Human multicrew advantage | **C14A** parallel discretion, actor competence and resilience |
| C15 | Crew cost | **C15A** aggregate readiness obligation; individual terms when salient |
| C16 | Journey topology | **C16A** carrier root moves its contained topology |
| C17 | Relationships aboard | **C17A** manifest, billet, station, access, custody and title are separate |
| C18 | Passage contract | **C18A** itinerary and bounded onboard rights |
| C19 | Routine travel activity | **C19A** background journey with optional opportunity events |
| C20 | Interception | **C20A** promote the carrier into a live tactical domain |
| C21 | Simultaneous account play | **C21A** one foreground character or control focus per world |
| C22 | Character switching | **C22A** allowed with location, exposure and fallback preserved |
| C23 | Multiplicative competitive benefits | **C23A** account-distinct eligibility plus policy-defined tenure/stake |
| C24 | Opposing participation | **C24A** one side per defined conflict episode |
| C25 | Hostile-organization alts | **C25A** narrative membership allowed; incompatible privileged roles restricted |
| C26 | Sibling-character transfers | **C26A** ordinary, attributed and journaled |
| C27 | Character defeat | **C27A** staged injury, evacuation, rescue, capture and campaign-rated death |
| C28 | Personal progression | **C28A** bounded competence, issuer trust, contacts and reputation |
| C29 | Short-session participation | **C29A** published crew calls and explicit relief handoffs |
| C30 | Unfilled stations | **C30A** declared NPC/automation fallback with reduced discretion |

### 18.1 Presence, billets and control

A billet records that a character may serve at a station; active focus records what the character
is operating now. An integrated small-craft cockpit may grant helm, sensors and weapons, while a
capital command station remains narrower. Remote drone or vehicle operation still consumes the
single active focus and requires an authorized terminal, compatible equipment and a live
communications path. A drone-control station issues bounded group commands rather than becoming a
loophole for manually piloting an armada.

**As built (doc 18 IC9):** the trade-run slice has no stations or billets yet (P2): the Dray's
`+ship/undock` and a queued depart fly without an active station seat, by doc 18 R3 (an offline
departure cannot wait on a live seat); the station-channel gate applies again from P2.

Station changes relinquish the former lease, require physical traversal when consoles occupy
different places, and complete at a safe pulse boundary. NPC or automation fallback maintains the
old station through the handoff. Ownership alone never satisfies reach or control.

### 18.2 Command and information

A mission order contains objective, priority, area or route, formation, rules of engagement,
withdrawal/surrender/abort conditions, fallback behavior, duration and review time. Subordinate
controllers translate it into the same legal intents available to human station holders. An
operation has one operational command parent per assigned unit even when its administrative
formation belongs elsewhere.

Consequential commands, sensor reports, market intelligence and operational messages traverse a
deterministic communications path. Owners may conduct legitimate administration remotely, but see
only telemetry that has actually reached them. Ordinary OOC communication is not constrained by
IC propagation. **Doc 15 D-15-12 (2026-10-23) classifies all mail as IC:** it must
transit, with no OOC mail mode; OOC channels remain unconstrained. The server never
supplies a universal live owner map.

### 18.3 Crew and multicrew

Named NPC objects occupy dramatically relevant positions: captain, helm, tactical, engineering,
marine command and similar roles. Routine watchstanders, technicians, loaders, stewards and
medical staff may be an aggregate complement carrying readiness, capacity and morale. A periodic
readiness obligation covers ordinary payroll and support; individual employment contracts are used
when a named NPC's personal terms matter.

NPCs use their real equipment and traits. Their usual disadvantage is reduced reconsideration
frequency, parallel attention, initiative under ambiguity and ability to recover from cascading
exceptions—not a universal `NPC` penalty. One player, suitable automation and a funded competent
crew can transit, fight and withdraw a capital ship. More player crew provide parallel observation,
station-local skill, creative judgment, quicker handoffs and casualty resilience, never an
unexplained ship-wide attendance bonus.

### 18.4 Voyages as inhabited play

The carrier root owns one journey. Anchored rooms, occupants, cargo, mounted equipment and carried
craft inherit its strategic movement without duplicate journey records. Passenger status, crew
billet, active station, compartment/berth access, cargo custody, prisoner status and title remain
independent relationships. A passenger buying passage from an NPC captain receives itinerary and
bounded onboard rights, not control of the ship.

Journeys advance without required watch chores. Departure, cruise, maintenance, contact, distress,
approach and arrival create optional prompts or actionable events. A real mechanical interception,
not a prose scene, may suspend or divert a leg: it promotes the carrier into a live domain while
preserving interiors, resolves through tactical facts, then resumes, diverts, captures or ends the
journey. Survey 6A still governs scenes: a scene cannot hold an otherwise due arrival.

### 18.5 Accounts, alts and integrity

Add an immutable private account id for durable integrity policy; do not turn it into a fictional or
economic principal. An account has one foreground character or OOC control focus per world, though
several client connections may attach to that same focus. Switching characters is allowed even on
a long voyage: the former character remains physically present and vulnerable, and any active
station hands off without granting a fresh link-loss exploit. The character left behind falls
under E5's attention budget and non-response semantics (R5).

Quorums, sides, referral and first-time rewards, command succession and similar multiplicative
benefits require distinct eligible accounts; sensitive policy may also require membership tenure or
committed stake. An account may materially assist only one side of a defined operation, battle,
claim contest or similar competitive episode. Narrative alts may belong to different factions, but
one account cannot concurrently hold command, intelligence, treasury, market-making, adjudication
or operation-planning authority for active opposing belligerents in the same campaign.

Sibling characters remain separate in location, money, reputation, obligations and knowledge.
Their transfers use ordinary contracts, custody and communications and remain attributed; a special
ban would only encourage laundering through another principal. Account rules prevent ordinary alt
multiplication, not determined multi-account evasion, which remains an audit and community-policy
problem absent intrusive real-person verification.

### 18.6 Character stakes, progression and access to play

Vehicle defeat resolves compartment danger, injury, evacuation or ejection, suit/pod endurance,
beacon detection, rescue, capture and ransom as separate facts. **Owner amendment 2026-10-28
(doc 27 Q-H12): there is no player permadeath.** In the worst case a third-party NPC organization
retrieves the player to a known port such as Phobos to recover. The disabled unit and its
equipment remain in the field for a later recovery expedition; crew rescue does not repair or
retrieve the hull. Doc 27 Q-F4 leaves the service timing/destination and capture boundary open,
not the guarantee. This supersedes the previous campaign-lethality permission for player death.

Progression expands options and trust rather than creating an inexorable stat treadmill. Bounded
competence improves reliability and information; certifications express an issuer's trust; contacts
create opportunities; reputation changes access and terms. Published crew calls state destination,
station, qualifications, risk, time window and compensation. Relief handoffs and fallback control
let characters join for a short session without making uninterrupted attendance a prerequisite.

## 19. Addendum — accepted decisions T8–T16 (owner, 2026-09-03, binding)

**T8. Formations and task forces.** Maintain two layers. A persistent order-of-battle tree records
fleets, subfleets, wings, squadrons, flights, battalions, companies, platoons, lances, stars and
other formations. An operation-scoped task force references whole formations, detached
subformations, individual units and support assets under a temporary command tree. Administrative
parentage is unchanged by attachment; a unit has exactly one operational command parent in an
operation. The engine stores generic formation kind, branch/domain, echelon, parent/children,
members, command roles and exact/minimum/maximum/preferred composition. `ruleset-tacsim` supplies
names and doctrine—for example a four-unit lance and five-unit star may share an echelon but remain
distinct kinds; an air flight may be four aircraft, a squadron three flights/twelve aircraft, and a
wing four squadrons/forty-eight aircraft; ground platoons, companies and battalions use their
ruleset ranges. `mush-org` supplies scoped authority to create, attach, assign and command, but does
not hard-code military vocabulary or misuse the organization scope tree as force composition.

**T9. Actor and decision driver.** Separate the in-world actor occupying a station from the source
of its decisions. A player character driven by its authenticated player, a named NPC driven by
deterministic/LLM control, and the same NPC temporarily directed by a human all retain the actor's
location, traits, perception, injuries and fictional identity. Automation need not invent a person.
Every accepted intent records actor, decision driver, authority and control-lease epoch.

**T10. Human-directed NPC invitations.** A commander may offer an operation-scoped NPC station to
an online player: organization, faction, allies, allowlist or public eligibility; operation, side,
unit, station, time window, required certification/standing, granted channels, mission order, rules
of engagement, permitted area, revocation and fallback all freeze in the accepted invitation. The
player consents; they cannot be forcibly attached. Public seats suit open/non-secret operations;
covert operations restrict the audience because the human inevitably learns what their station
perceives.

**T11. Proxy-control guardrails.** Human-directed NPC control grants tactical discretion, not title,
administration or an unrestricted possession verb. It does not by default permit asset transfer or
abandonment, scuttling, fallback changes, loadout/crew mutation, organization spending, surrender
of unrelated assets, departure from the authorized operation, or assumption of another station.
Ordinary tactical loss cannot be made impossible without making the seat fictitious; legal intents,
ROE/safety interlocks, immediate revocation and dual actor/account attribution bound deliberate
misuse. Revocation or disconnect returns the lease to its declared NPC/automation fallback at a
safe boundary and invalidates stale intents.

**T12. One account control focus.** OOC proxy control consumes the account's one foreground focus.
The player's own character remains at their durable location and cannot act concurrently. Entering
proxy mode hands off that character's station, clearly identifies the represented NPC and exposes
only that NPC station's observer projection. Leaving restores AI control; it never moves or
protects the player's character.

**T13. Proxy canon and reward.** A proxy operator supplies tactical choices for the NPC; canon,
injury, knowledge, reputation, kill/salvage credit and economic consequences attach to the NPC,
unit, commanding organization and operation. The absent player character receives no automatic
pay, advancement, reputation, knowledge or property. The account may receive non-fungible OOC
participation history or matchmaking standing, never a currency convertible into character power.
Matchmaking standing is a *policy input* to crew-call eligibility and nothing else (R6).
IC reward requires the player's character to participate through a real physical/remote billet or
another actual mission role.

**T14. Scheduled musters and open seats.** An operation may publish its start and expected duration,
committed formations, character billets, proxy-eligible NPC stations, certifications/faction
restrictions, command succession, secrecy and compensation. At muster, a slot resolves to a player
character, human-directed NPC, AI-controlled named NPC, automation or vacancy. Missing players
reduce discretion and resilience but do not cancel a valid operation. Late arrival and early
departure use the same explicit handoff.

**T15. Orders require delivery.** Issuing revision N does not make it the recipient's active order.
An attributed envelope progresses through queued, transmitted, delivered and acknowledged states,
or expires, is cancelled or is superseded. The recipient obeys the newest valid revision it has
actually received; under ECM it continues its last order and fallback doctrine while the commander
sees that the replacement is unacknowledged. On restored reachability, deliver the newest valid
superseding order rather than an obsolete execution backlog. Relays, couriers, store-and-forward,
interception, encryption and forged orders extend the envelope later. Ordinary jamming blocks or
delays; content corruption/deception requires an explicit electronic-warfare result.

**T16. Station-local competence.** The actor at a station supplies the relevant traits: reactions
and spatial judgment at helm; perception, analysis and EW at sensors; leadership and tactics at
command; technical skill in engineering; officer decisions plus aggregate readiness in damage
control. A physically present player character uses their sheet, a named NPC uses its sheet, and a
human proxy directing an NPC still uses the NPC's sheet—the human adds judgment, not an absent
character's stats. Any wider effect needs a causal action or changed ship state, never a free global
bonus for seat occupancy.

## 20. Cross-ruleset environmental boundary

TacSim supplies physical and civil context, not ruleset metaphysics. The generic seam exposes facts
such as campaign time, body/region, authoritative local or shipboard civil phase, illumination and
direct environmental exposure, atmosphere, gravity and radiation. A ruleset decides what those
facts mean to its characters.

This prevents cross-ruleset composition from multiplying clocks or embedding supernatural doctrine
in orbital mechanics. A WoD ruleset may treat Luna as the one supernatural lunar authority for a
werewolf wherever they travel; Jupiter's satellites are geography unless story policy promotes a
specific local influence. It may treat vampire day sleep as following the vessel or habitat's
stable civil cycle while direct solar exposure remains a separate physical hazard. Ships and
habitats therefore register a durable authoritative schedule that cannot be toggled moment to
moment for mechanical advantage. Other rulesets may consume the same facts differently without
changing `mush-sim`.

## 21. Draft 0.2 review — accepted sharpenings R1–R7 (2026-09-03)

The Draft 0.2 review verified §17's two code corrections against the tree (`StationAssignment` is
keyed by host; units carry a monotonic control epoch; a `Journey` is "a scheduled route a plain
object is following", keyed by traveler) and folded these sharpenings, all accepted:

- **R1. One operation, one force record.** A declared operation (T6) and a task force (T8) are one
  record, or linked one-to-one: the task force is the operation's roster and command overlay. A
  unit has at most one operation at a time, not merely one parent within an operation.
- **R2. Formation authority binds to an org scope.** Each formation node references exactly one
  owning org scope; `force.*` capabilities resolve through the existing capability machinery, and a
  task force borrows authority from its operation's consent surface. The org scope tree still never
  mirrors the order of battle.
- **R3. Account-distinctness as a predicate.** A boolean oracle over character ids, called at
  roster lock and side assignment, storing only the verdict; the account id is never a principal
  and never appears in a journaled, contract, or canon record.
- **R4. One propagation layer.** C9A's deterministic delivery and §14's contact/observer projection
  share one "what has reached whom" substrate; backlog items 4 and 14 are one layer.
- **R5. Switching meets E5's fallback rules.** A character left aboard after a switch is governed by
  E5's attention budget and non-response semantics; no second fallback model.
- **R6. New decisions are consent shapes.** **T10 invitations are consent grants that share the
  contract's consent-hash discipline; an E4 contract exists only where a fictional principal has
  economic terms** (amended 2026-09-03 from "audience contracts": an OOC proxy player is not an
  in-world principal — 10a Q10). C15A readiness, C18A passage, and T13's participation-indexed
  settlement remain templates and fact kinds over E4b1/E4b2/E4c (§15). Matchmaking standing is a
  policy input only.
- **R7. Storage rule for the account id.** Opaque, audit-only, staff-visible through an audit verb;
  never in a journaled or canon record.

## 22. Addendum — T17, the frozen sky (owner, 2026-09-09, binding; amended 2026-10-15: the constant sky of the first slice, superseded after it by the astronomy engine)

**T17. Distances are derived from real geometry at one chosen epoch and then frozen.** Refines
1A and T1; supersedes nothing. The solar system is **not a coordinate map** — no 2²⁴ × 2²⁴ grid,
no ephemeris ticking under the players — it is a fixed constellation of places (doc 14's
orbital, system and interstellar regions, their sites and connections) placed fixedly relative
to one another, exactly as a city's neighborhoods are. The distance between two places is
computed once from real orbital geometry at the campaign's chosen **epoch** (circular, coplanar
orbits: each body an orbit radius and a phase at the epoch) and pinned into the connection.
**No body moves. No transfer window opens or closes. Nobody waits for an alignment**, and Titan
is the same distance from Saturn on the day a character moves there as it is a year later.
Travel time is the campaign's compression (1A's rates, `[tacsim.time]`) of the IC duration a
drive needs for that frozen distance, by the drive class's distance law (a torch drive's time
grows with the square root of distance; a coasting transfer's with the distance).

The mechanism is a **campaign ephemeris** in the TacSim pack: `body` entries
`{ orbit_au, period_days, phase_deg_at_epoch }` and a campaign `{ epoch, rate }`. **`rate = 0` is
the ruling and the default.** The knob exists so a campaign that wants a moving sky can have one
coherently — every pair sharing a body would change together — but this campaign does not. The
epoch is chosen for the corridor: favourable for Mars–Ceres, the proving loop's load-bearing
pair; every other pair falls where the real sky puts it, which is what keeps them mutually
consistent (no configuration has every pair at closest approach). T1's freeze-at-publication
rule is unchanged and, with a frozen sky, moot. `RouteDef.alignment_band` — declared by E7 cut 6
and never applied by `route_duration` — is replaced by the body-pair reference the distance came
from. Build: the spatial plan's cut S7 (`14-spatial-implementation-plan.md` §3.8).

**Amendment (owner, 2026-10-15; doc 18 R9).** T17 now governs only the constant sky of the first IC
vertical slice ([doc 18](18-trade-run-vertical-slice.md)). After that slice it is superseded by the
astronomy engine (doc 18 cut P-A): real ephemerides in three dimensions, with eccentricity and
inclination (Pallas far from the ecliptic); bodies moving on the IC calendar, so a route's distance,
duration and fuel depend on when it departs; the best route between two ports changing over months; a
port's position moving its prices. Until P-A lands `rate = 0` stays configured and every slice figure is
the constant sky's output at its epoch, never an invariant: no code, test or help text may treat a
distance, a duration or a price as constant beyond a frozen commitment. The slice reaches the sky only
through the seams the engine will reuse — one sky query (the distance between two bodies at two
instants) and one passage planner over the sky at departure, the route's legs as body pairs, the drive's
law and the departure mass — and derives durations rather than relying on authored seconds.

**What stands:** 1A's scales and compression (local orbit in hours, the inner system in days to weeks,
the Belt in weeks, the outer system in weeks to months, about one OOC hour per IC day); 1A's rule that
transfer conditions modify routes rather than closing destinations (no launch windows; every port
reachable at a cost); T1's freeze, no longer moot — a passage's per-leg distances, durations, burn and
crew-days are computed once at commitment and frozen into the ticket and the journey, and nothing after
the burn re-reads the sky; T17's coherence (every pair sharing a body changes together), kept by the one
query. **Superseded, after the slice:** "No body moves. No transfer window opens or closes. Nobody waits
for an alignment" (bodies move and distances vary; still no window closes a destination); the circular,
coplanar model as the final model (it remains the e = i = 0 case of the engine's elements); 1A's
rejection of simulated ephemerides ("long non-opportunity" is doc 18 Q-18-35 by another name, re-opened
knowingly). Travel time becomes the planner's output: the sky at departure, the drive's law (coasting at
a cruise speed, or a constant-acceleration flip) and the departure mass. The epoch moves from server
config to the pack's bodies (doc 18 D-18-48), which is what this addendum always said.

**Superseded (2026-10-18; [doc 19](19-astronomy-engine.md), cut A.1).** The astronomy engine is
built: every lane body moves on an element table from JPL Horizons behind `Sky::Kepler`, in three
dimensions, within 0.07 ‰ of Horizons over 2340–2400; the circular model stays the e = i = 0 case
every fixture pack still is. T17 now describes the fixtures' sky and the slice's history, nothing
the lane relies on.
