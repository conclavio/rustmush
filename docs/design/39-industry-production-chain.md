# 39. Industry — the production chain and unit construction, surveyed

Draft 0.3, 2026-11-03, docs only. Surveyed against `origin/main` at `6f9dff29` (PR #344, the ten
tacsim shards) with [doc 38](38-after-the-ic-phase-next-systems.md) Draft 0.2 (PR #345, merged)
and re-verified at `c83c2eff` (SF.R15 merged): every anchor doc 38 §4 A cites was re-read, and the
line numbers below are the current tree's where they moved (the matter pin is in `shard_9` now,
not `shard_3`). This is the document doc 38 §6 proposed as "Doc 39 — Industry: the production chain
and unit construction", economy-owned, and doc 38 §8's list of what it had to settle is this
document's table of contents. Draft 0.1, the same day, was the survey. **Draft 0.2 recorded the
owner's rulings of 2026-11-03 on every question: every default accepted as written** ("all q-39s
accepted at their defaults"). **Draft 0.3 triages two outside reviews of Draft 0.1 (§8) and revises
what they confirmed**: the accounting unit (§4.10), the component lifecycle (§4.11), the order's
escrow, cancellation and ordering authority (§4.12), the delivery and handover (§4.13), the supply
paths (§4.2), the cost coverage redone per site (§4.4) and a complete mass partition (§4.8). Twelve
new questions, Q-39-25..36, go to the owner; three of them (Q-39-33, Q-39-34, Q-39-35) revise
accepted defaults and say so. The cuts are reordered so identity, units and escrow precede their
consumers and the first player production story is IN.5.

The form is doc 38's: a survey that asks the owner numbered questions, each with a proposed default,
an alternative and a reason; this draft carries the rulings on Q-39-1..24 and the cut list is the
plan once Q-39-25..36 are ruled. "Provisional" marks a number to be tuned later under the
2026-10-24 rule (take a reasonable default, tune later, never hold a build on it). **(expensive to
reverse)** keeps doc 38 Q-38-10's meaning: an answer that would take a significant refactor of the
codebase to undo — not stored data and not a live box, since every step may wipe and redeploy.

## 1. Why this document exists

Doc 38 §4 A surveyed the chain from ore to hulls for the first time and found it stops at the ore
mills: nothing refines, nothing makes a part, the workshop fits a new part from nothing, scrap mints
alloys unbounded by the hull's mass, and every hull in play was laid up by content or by staff. The
owner ruled (Q-38-1) that this chain leads the next round, because every repair, refit, salvage and
field-engineering conversation reaches for a part with a price and a store it came from, and
(Q-38-4, Q-38-5, Q-38-12, Q-38-13, Q-38-14, Q-38-15, Q-38-18) fixed the shape it must take. §2
lists those rulings; they are binding here and are not re-asked.

What was left to decide was what doc 38 §8 named: the catalogue rows, the stations, mill rows
against recipes, the facility cost model's inputs, the scrap ceiling's mechanism — and the pieces
the brief added beside them: the part lot's stores and bill, the mass budget as one rule, the
yard's delivery, and the cut list. The two reviews (§8) added what Draft 0.1 had assumed the engine
did and it does not: whole-tonne ledger units, pooled inputs and batched outputs, a seven-day
reservation, a free cancel, a delivery that cannot make a hull, and a hull that nobody could sail.
Each area gets a section in §4 with what exists, what is missing and the questions it raises; §5 is
the owner survey (Q-39-1..24 ruled at the defaults; Q-39-25..36 open); §6 the plan, cuts
IN.0–IN.11.

## 2. What binds this survey, and what it leaves alone

**Binding rulings from doc 38 (2026-11-03).**

| Ruling | What it fixes here |
|---|---|
| Q-38-1 | the chain leads; docs 40–42 consume its lots and prices |
| Q-38-4 | price the part: `Replace{lot: None}` draws a `tacsim.part.*` lot of the slot's definition from the facility's own stores at a catalogue price; the stores are fed by strip today and by a part recipe later; until the recipe exists, seeded stock per workshop at a provisional reference (the component's share of the hull's reference price); an empty store refuses with the reason, as `NO_HULL` does; the labour fee stays |
| Q-38-5 | scrap is bounded by the hull's dry mass **and** worth at most about a fifth of the hull's buyout (a Dray's 90,000 cr scraps to ~18,000); the scrapping facility's fee consumes the rest; a workshop's scrap goes to the port's stock (the operator's), a site's remains scrap to the carrier's hold with the room check; whether that is a tuned yield or a mass-bounded mint with a fee is this document's |
| Q-38-12 | a hull is built by a work order at a facility with a `yard` capability; a registry design's bill = its slots + the manufacturing cost = an amortization of the facility's time over its capital cost and expected lifetime, before labour and margin (a 10 M cr yard over 30 years ≈ 1,000 cr a day; two weeks ≈ 14,000 cr); doc 39 derives every facility's **daily cost** from capital, lifetime and staffing, and that drives hull prices; the output is a hull delivered into the requester's title and the yard's berth, which needs a second `DeliveryTarget` **(expensive to reverse)** |
| Q-38-13 | three transformations: ore → concentrate (the mills, built) → `structural_alloys` and `industrial_parts` (a refinery at Kasei, a works at Stickney and Ceres, by recipe through `@wo` or a mill row) → parts and hulls (the yard); carbon ore feeds the works' `industrial_parts`; strategic ore feeds a part recipe for sensors, ECM and cores |
| Q-38-14 | conservation: every mint is a recipe's output or a seed with a receipt; the matter census pin covers `.mint_origins(`; Stickney's and Kasei's seeds become stock the chain refills, the intake drips retired as the recipes land |
| Q-38-15 | labour stays inside the facility fee; no technician market (E8 is unplanned) |
| Q-38-18 | an NPC salvage-retrieval organization at a main port owns a heavy crawler and whatever carries it; its fee comes from this document's facility cost model; its content and the retrieval contract are doc 41's |

**Older rulings the chain inherits.** Doc 10 §12 4A (l.255–262): ten commodity families, "chains ≈
three transformations deep", drydock and factory time as reserved capacities, "named major
components may be individually titled". Doc 10 §5 (l.92–106): the loop ends "refine → manufacture →
deploy"; NPC corporations are funded, never infinite. Doc 08 §10.2 (l.1225–1241): the nine steps of
a TacSim production order, ending in "transfers the output into faction title and physical custody".
Doc 18 P5 (l.2652–2655): "production restock with conservation (Kasei refinery work orders, Ceres
deposits, a construction sink) before a second cohort"; risk 9 (l.3610–3613): a recapitalization
path, attributed and receipted, before the alloy buyer drains. Doc 20 §7.3 (l.1318–1372): the four
ores, carbon "none in phase 1 — a stock for a later feedstock ask", strategic "the Exchange's prize
shelf", "Kasei bids no ore: the refinery's feed is concentrate"; O-3 (l.2306): milled ore leaves no
tailings. Doc 22 §7 (l.427–433): "A builder, its economy and who may field what are their own
design". Doc 31 D-31-13 (l.351–362): a like-for-like replacement comes "from stores (a stripped
record) or bought"; Q-31-2 (l.469, l.532): an IC hull may be built on a registry-only design, which
must be committed before a Redis reset. Doc 36 Q-36-31 (l.504): quotes from the tariff, parts drawn
from lots; Q-36-32 (l.512): the facility fee covers labour; Q-36-39 (l.576): scrap turns structural
mass into `structural_alloys` and salvageable parts into lots; doc 36a §9.1 and §9.5 (l.308–321).

**Left alone.** The crawler's unit, its carrier and the retrieval contract (doc 41); the kernel's
component effects (doc 40, Q-38-27); the core track (doc 17's dies, `hardware/content.rs`), which
keeps minting its monthly dies as a receipted seed until Q-39-22 says otherwise; the service market
(E8, doc 08 §18 l.1941); player-designed units and the review step (doc 31 §9 l.447); who may field
what (doc 22 §7); capital hulls, the relay station and the drone as buildable designs (Q-39-20,
Q-39-35). The two outside reviewers' part-market items (doc 38 §3 R1-8, R2-9) are answered by §4.5
and §4.11 and need no re-survey.

## 3. The tree at `c83c2eff`: the chain as built

What doc 38 §4 A said stands; this section adds what the re-read found and anchors every number the
survey uses. Prices are major credits (`price_major`, `reference_major`); the tariff's minor units are
converted where quoted.

### 3.1 Ore and concentrate (built)

Four ores in `crates/tacsim/content/market/goods.json` v4 (`metal_ore` :113–124 at 70, `ice_ore`
:126–137 at 35, `carbon_ore` :139–150 at 50, `strategic_ore` :152–163 at 600), each "drilled from a
Registry site" and bought only at Ceres. The Exchange bids metal, carbon and strategic ore
(`content/lane/ceres_phobos.json` v19 :2636–2638, drips 40/20/2 t an IC day to caps 240/120/12),
Occator bids ice (:2646, 40 to 240). Two mill rows (:2639, :2647): 5 t of ore to 2 of product, 20 t a
batch, 40 t an IC day, the Exchange's `metal_ore` → `metals` and Occator's `ice_ore` → `volatiles`.
`port/mill.rs` (module doc :1–80) runs them on the stock clerk's arm, debits the ore pool and
credits the product pool in one bundle under a ledger keyed by the ore, and `restock_ask` lists the
milled units. `MillDef` (`port/def.rs:1082–1096`) is checked by `check_vendor_mills` (:1847–1900):
at most `MAX_VENDOR_MILLS` 4 rows (:74), the ore one of the vendor's bids, the product one of its
asks, `1 ≤ product ≤ ore ≤ MAX_MILL_RATIO` 100 (:77), the batch a whole number of ratios; the **same
family rule** — "a mill's ore and product are one family's goods" — is `check_ports`' (:2450–2463,
the goods table's `family`: `metal_ore` and `metals` are both `metals`, `ice_ore` and `volatiles`
both `volatiles`). Space mining (SF.R5, D-35-13) lands the same four ores in `stow.ore`.

### 3.2 Refined goods (seeded, not made)

`structural_alloys` (goods :35–46, "Kasei's output, banded bar and plate", 400 cr/t, 300 L/t) is a
one-time seed of 7,000 t at Kasei (lane :2709) asked at 400 (:2711); Kasei bids `metals` at 280
(4,000 t intake, :2713) and `volatiles` at 200 (9,600 t, :2714) and converts nothing. The Exchange
bids alloys at 520 under the alias `"as": "Freeport Works"` (:2635, 7,000 t intake) — **"Freeport
Works" is a display name on one bid row, not an organization** (the Ceres orgs are :2595–2602).
Elsewhere Cabeus Works stocks 3,120 t at Luna (:2771) and bids concentrate at 270 (:2773); Syrtis
bids alloys at 460 (:2826), Selk at 760 (:2871). `industrial_parts` (goods :48–59, "yard fittings,
crated", 400, 2,300 L/t) is seeded at Stickney Yards, 5,400 t (:2717) asked at 400 (:2722), and the
Co-op bids 560 at Ceres (:2660). **Both refined goods are whole-tonne ledger goods**: `ledger_unit`
and `quote_unit` are `"t"` (goods :39–40, :52–53), and every balance, reservation, lot quantity and
recipe line is an integer of ledger units (`RecipeLine.units: u64`,
`mush-industry/src/workorder.rs:166–169`). The only recipe is the test fixture `alloy_smelt`
(`fixture/tacsim_core.json:11–23`): 4 `metals` → 1 `structural_alloys`, `requires: ["smelt"]`, 3 work
units, at two fixture facilities, "Ceres Freeport Refinery" (`refine`, `smelt`, 2 slots,
`tacsim_corridor_v2.json:90–101`) and "Phobos Yard Forge" (`forge`, `smelt`, :134–144). The fixture
pack's commodity table says `industrial_parts` is `in_use: false` (:82–84). **At today's prices the
fixture's ratio loses money**: 4 t of concentrate at Kasei's own 280 bid is 1,120 cr for a tonne of
alloys Kasei asks 400 for. The deploy template loads no pack at all (`mushd.toml.tmpl:255`,
`pack_paths = []`), so no recipe is live outside the relay foundry's. The Exchange asks `metals`
only (:2634): nothing sells carbon or strategic ore to anyone.

### 3.3 The engine (E7, built; one live user)

`FacilityRecord` (`mush-industry/src/facility.rs:75–82`): an object, an owner principal, a
capability set (at most 32, :54), `maximum_slots` (at most `MAX_FACILITY_SLOTS` 8, :51), a capacity
and a policy revision; availability is derived from the reservation book, never stored. Three ways
to make one: a pack `FacilityDef` (`tacsim/src/pack.rs:1000–1017`: `label`, `at` a pack-local
location, `capabilities`, `slots`), staff's `@facility/open <object>=<slots> [capability...]`
(`mush-industry/src/commands.rs:910–947`, staff-only, the opener as owner, `@facility/owner` to hand
it on :949ff), and **the port applier's part j** (`tacsim/src/port/workshop.rs:1–60`), which opens
the authored workshop room itself as a facility of the operator org holding `bay_refit` with the
authored berths as slots (`WorkshopDef`, `port/def.rs:189–194`; Ceres' `c14_dockmaster`, Occator, 2
berths, ×1.2, lane :2678; Stickney's `p11_service`, Stickney Yards, 4 berths, ×1.0, :2735) and
refuses a changed berth count or capability set on a kept world (:52–60). Recipes are pack entries
`kind: "recipe"` `{inputs, outputs, requires, work_units}` answered through `TacsimRecipeAuthority`
(`composition/industry_recipes.rs:25–112`) from the published pack's own digest; `PinnedRecipe`
refuses a recipe with no inputs **or no outputs** (`workorder.rs:349–359`), so nothing seed-shaped
can hide in a pack and nothing output-less can be drafted. Work orders: `@wo/draft
<pack>/<recipe>/<version>=<facility> <qty> <account> <source> <destination> [beneficiary]`
(`commands.rs:1135`, endpoints by number — a staff grammar) **for staff or a principal that
`acts_for` the facility's owner** (:1185–1189; a stranger reads `NO_SUCH_FACILITY`), `@wo/submit`,
`@wo/deliver`, `@wo/cancel` (`@wo/set` refuses, :1488); one work unit an IC hour
(`PRODUCTION_CADENCE_SECS` 3,600, `workorder.rs:69`); at most 10 inputs (:44). Submit **reserves
each input from a pool** (`provider.rs:3458–3461`; `pool.rs:1050–1066` answers `NoPool` for a lot)
with a deadline of `RESERVATION_TTL_IC_DAYS` 7 (`mush-inventory/src/record.rs:34–36`,
`reservation_deadline`, `pool.rs:198–204`), holds the slot, and places the E4 hold at `order_price`
= **one minor unit an input unit** (`quote_price`, `workorder.rs:1210–1214`; `money.rs:210–214`,
`:333–345`). Advance stops at `AwaitingDelivery` and restamps nothing (`provider.rs:3590–3618`).
Delivery is a prepare — "no `&World`, no `Ctx`, no `EconomyStore`" (:3637–3640) — that consumes the
reservations (:3728–3752; `checked_consume` requires `Held` only and never reads the deadline,
`mush-core/src/resource.rs:456–462`: expiry is a sweep's act, `pool.rs:1354`,
`mush-industry/src/money.rs:623`), releases the slot, and **mints one lot per output line at grade
0 with `units × quantity` in it** (:3778–3796, `wo_source` :4086–4093). Cancel runs from any
non-terminal state, `AwaitingDelivery` included (`holds_resources`, `workorder.rs:521–529`;
`provider.rs:3846–3900`), returning the materials and the slot, with the budget hold released
unspent by the sweep (`money.rs:366–392`). The one target is `DeliveryTarget::Endpoint`
(`workorder.rs:446–454`). The one live recipe is the lunar relay foundry's `private_pair`
(`relay_foundry.rs:22`; `content/relays/manufacture-v1.json`: 1 `relay_blank` + 2
`instrument_boards` → 1 `relay_pair`, 120 work units, `requires: ["relay.entanglement"]`) — and its
second half is the precedent §4.13 builds on: a tacsim action consumes the order's output lot by
its `workorder.output` lineage and materializes the machines with a `Ctx` the prepare never had
(`relay_foundry.rs:375–405`). Identified part custody for an order exists too: `BayPartCustody`
(`mush-inventory/src/bay_part.rs:1–10`, `:320–345`) holds a one-unit lot in a store for a bay order
so "no ordinary verb moves, splits or sells it", and refuses `quantity != 1` (:333).

### 3.4 Parts (minted by strip and scrap; consumed by fit; free from the workshop)

A part is a one-unit lot of `tacsim.part.<suffix>` where the suffix is the last segment of the
component's definition id (`hardware/bay.rs:46–55`; a core is no part :51). **The re-read's main
finding:** at bind a component's definition id is `{unit}.{slot}` (`definitions/unit.rs:1700`,
`:2045`) — `tacsim.unit.line_tank.cannon`, `tacsim.unit.heavy_mk1.cannon`,
`tacsim.unit.dray.power` — so the part commodity is **the slot's name, not the hardware the slot
holds**. The ten commodities in the shipped content are `cannon`, `machine_gun`, `autocannon`,
`laser`, `missiles`, `visual_sensor`, `thermal_sensor`, `ecm_pod`, `eccm_pod` and `power`, plus
`relay` on the Halcyon. A `tacsim.part.power` is one commodity from a drone to a Dray, and a
`tacsim.part.cannon` stripped from a line tank and one from a heavy are one commodity but two
definitions, so `+bay/fit`'s "of the slot's definition" check (`bay_work/mod.rs:712`, the hot and
fitted refusals :740–752) refuses the cross-fit while the market would list them as one good. The
commodities have no row in `goods.json`, so a part lot has no mass: `strip::hold_takes`
(`bay_work/strip.rs:96–123`) sums bulk lots only (:100, `lot.part.is_none()`), and a stripped cannon
weighs nothing in the hold. `Replace{lot: None}` fits an Intact part from nothing
(`jobs.rs:467–491`, :480) for `labor_cost_replace_component` 40,000 minor = 400 cr and 14,400 IC s
(`workshop_tariff.json:11–12`), scaled 0.5–2.0 by the operator (:15–16). `PartProvenance`
(`bay_work/act.rs:85–110`) records the parent hull, slot, definition, condition, claim, legality,
stripper and time — every variant assumes a parent hull, and the family `tacsim.bay_parts` is keyed
by lot id (`act.rs:719–726`), so a lot the market or a split made is untraced. The bay's `+bay/fit`
consumes the whole lot (`consume_product`, `pool.rs:1525–1539`, removes the record whatever its
quantity). `part_commodity` serves both `bay_work` (`strip.rs:198`, `:290`) and `bay_refit`; the
refit's own part diff is by slot id (`bay_refit/config.rs:235–252`) and its removed part is a lot
with an `industry/bay_refit` origin and the catalogue's grade, no `tacsim.bay_parts` row
(`bay_refit/provider.rs:535–552`), as doc 36 GC.8b recorded.

### 3.5 Scrap (unbounded by mass; priced above the hull)

`SCRAP_KG_PER_STRUCTURAL_POINT` 100 (`mush-tactics/src/field_ops/workshop.rs:28`) and
`FALLBACK_SECTION_SCRAP_KG` 1,000 (:31) make a hull's scrap 100 kg a surviving point; `strip::scrap`
(`strip.rs:255–300`) divides by `SCRAP_KG_PER_T` 1,000 (:49) into one `structural_alloys` lot at
`SCRAP_GRADE` 50 (:51) plus a lot per salvageable part (:282–299), everything to the carrier's hold
through `mint_origins` (`act.rs:785`, receipt kind `bay_scrap`); the shell stays in the room
(:322, :395). Remains on a site scrap the same way into the hold (`surface/tow.rs:1735–1800`,
`mint_origin` :1781; a yield under a tonne refuses as `scrap_changed`, :1772–1774). `scrap_hull`
sums **every section cell above its minimum** (`workshop.rs:745–752`), heat cells included, and the
heat-sink cell is born at its maximum (`definitions/unit.rs:1678–1683`), where the bay's quote
filters both heat cells out (`jobs.rs:239–246`); since SF.R15 the Dray and Dray-P carry `heat_max`
1000 and dissipation 30 (PR #339, the only content change it made), so a Dray's three sections scrap
90 phantom points today. The escort's 2,990 points (2,320 armour + 670 internals at `c83c2eff`) are
299 t out of an 8 t hull (`tv_escort.json:332`), and the Dray's 1,820 points (570 + 1,250:
`dray.json:15–45`, hull 120/80/80/60/60 + 600 internals, hold 60/60/50 + 400, drive 250) are 182 t
out of 240 (:83) — inside its mass, but worth 94,640 cr at Freeport Works' 520 bid against a 90,000
cr buyout. The repair side exceeds the hull too: `structural_points_per_industrial_parts_t` 100
(`workshop_tariff.json:6`) is 10 kg of fittings a point, rounded up to whole tonnes at the quote
(`jobs.rs:205–208`) and debited whole (`act.rs:712–716`), so rebuilding the escort's 2,990 points
draws 30 t of `industrial_parts` into an 8 t hull. Repair draws 10 kg where scrap returns 100.

### 3.6 Hulls (content pools and staff)

The Co-op First Captain programme (lane :2604–2629): eight Drays (`pool` 8, :2611), stake 25,000
(:2614, posted by the Trust at each enrolment, `coop.rs:1629–1663`, from its 250,000 of capital
:2597), `buyout_major` 90,000 (:2625), the `prospector` refit to the Dray-P (:2628). Occator's rig
lease (:2668–2675): eight `tv_prospector` at 1,400 + 5,000 deposit, two `tv_escort` at 2,500 +
15,000. The Occator Yard house hull (`port/yard.rs:1–20`, a Dray-P on Occator Rim's pad). The port
applier lays each up (`port/hull.rs:1–60`; `lay_up_one` :655 over a `HullPlan`, ships; rigs through
`vehicle::lay_up_rig` :643) **from a template that requires both `logistics` and `interior`**
(`hull::template_of`, `hull.rs:317–323`; `vehicle::template_of`, `vehicle.rs:181–186`): of the 19
unit definitions in `content/units/`, only the Dray, the Dray-P, the corvette, the Halcyon, the escort
and the prospector carry both; the thirteen simulator units (the five line tanks, the heavy, the
scout, the two bipeds, the drone, the carrier, the target, the relay station) carry neither. The
applier's preflight refuses a programme row past the pool (`hull.rs:465–471`, "hulls are never
retired by a port update"; `vehicle.rs:306–312` for rigs). `+coop/enrol` leases a Trust Dray only
while it is docked on the programme port's own domain (`hull_is_free`, `coop.rs:736–739`);
`+coop/buyout` turns the charter into title (`buyout.rs`), `+coop/refit` re-binds the unit for no
materials (`refit.rs`); the hold vault is opened at enrolment (`coop.rs:1669`), not at lay-up. Every
operate path reads a lease or a programme row: `helm_of` is the live charter, else the programme
map's lowest hull titled to `Object(pilot)` (`coop.rs:597–622`); `hull_view` searches programme and
yard rows (`ship.rs:318–329`); a rig is boarded from `tv_leases_of` (`boarding.rs:188–198`),
launched through a `TvLease`'s `lease_gate` (`surface/act/plans.rs:142–149`), sortied from
`tv_lease_on` (`detachment/sortie/surface.rs:89–97`); a unit in a bay with no lease lists as slot 0
(`vehicle.rs:1058`), which the buyout and the refit read as "still in the bay" (`buyout.rs:407–409`,
`refit.rs:180–182`); a carried unit without `logistics` fails `carried_rig` (`vehicle.rs:1189–1206`).
Staff `@unit/create` is the only other source of a hull. Dry masses: `tv_prospector` 6 t (`:97`),
`tv_escort` 8 t (`:332`, payload 220 kg :346, "so it rides a Dray-P bay beside one prospector rig"),
the Dray and Dray-P 240 t (`dray.json:83`, `dray_p.json:83`; holds 100 t and 86 t, :85; the Dray-P's
rig slot takes 20 t, :206). `UnitClass` (`definitions/unit.rs:749–772`) is Armour, Walker, Drone,
Hauler, Vehicle, Station or Capital; a unit with no authored class is Walker when it walks, else
Armour (`class_of`, :971–979), so the five line tanks, the heavy, the scout, the carrier and the
target are Armour and the two rigs are both Vehicle. A mount's magazine is born full at bind
(`:1718–1725`, `init: rounds`). The line tank's `tank_minor` 10,000,000 (100,000 cr,
`hardware/content.rs:160`) is the basis for core prices only (Class I/II/III at 3/8/25 %, :171:
3,000, 8,000 and 25,000 cr; the fitting fee `FITTING_MINOR` 10,000 = 100 cr,
`mush-economy/src/core_refit.rs:32`); no other hull has a reference price, and the escort's lease
deposit is a deposit. Doc 18 Q-18-70 (l.3528): a graduated hull leaves the Trust's pool for ever, so
the desk answers `NO_HULL` once all eight are chartered.

### 3.7 The price ladder as it stands

| Stage | Good | Ask (where) | Bid (where) | Note |
|---|---|---|---|---|
| ore | `metal_ore` | — | 70 Exchange | 5:2 → concentrate at 175/t cost |
| ore | `ice_ore` | — | 35 Occator | 5:2 → water at 87.5/t |
| ore | `carbon_ore` | — | 50 Exchange | no consumer, no seller |
| ore | `strategic_ore` | — | 600 Exchange | no consumer, no seller |
| concentrate | `metals` | 200 Exchange | 280 Kasei, 270 Cabeus | the trade run's outbound |
| concentrate | `volatiles` | 100 Occator | 200 Kasei, 195 Habitat, 190 Reserve | |
| refined | `structural_alloys` | 400 Kasei | 520 "Freeport Works", 460 Syrtis, 760 Selk | seed 7,000 t; no maker |
| refined | `industrial_parts` | 400 Stickney Yards | 560 Co-op | seed 5,400 t; no maker |
| electronics | `instrument_boards` | Crisium (Luna, 2,080 t), ref. 900 | 1,100 Syrtis, 1,500 Selk | the relay foundry's input |
| carbon | `carbon_feedstock` | Titan Concession (600 t), ref. 250 | 380 Habitat, 380 Reserve | Titan's billet, not the Belt's ore |
| parts | `tacsim.part.*` | none | none | no row, no mass, no price |
| hulls | — | none | — | 90,000 buyout; 100,000 tank basis |

### 3.8 The census pin

`sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter` (`tests/shard_9/mill.rs:2508`, moved
from `shard_3` by PR #344): over `crates/tacsim/src`, `.mint_origin(` occurs in `port/stock.rs`
(the seed, with its `StockReceipt`, :10–18, :420 — one lot of the row's `qty`, which the clerk's
next fire dissolves into the storefront pool, :639–670) and `surface/tow.rs` (remains scrap) only;
`.credit(` in the mill and the relay foundry; `.debit(` in the bay's draw, the burn and the mill.
`.mint_origins(` (`bay_work/act.rs:785`, the strip and scrap mints) matches no pattern, as doc 37
§5.1 (l.163–165) says. `mush-industry`'s own delivery mint is outside the pin's files and under its
own tests (`mush-industry/tests/e7r1.rs:2219`). The market makes a buyer's lot at every sale
(`Lineage::Tranche`, `pool.rs:1215–1224`) and pages when a storefront pool and its listing disagree
(`stock.rs:505–510`, `:728–737`); a lot listing (`Representation::Lot`,
`mush-market/src/record.rs:129`) is the market's shape for provenance-bearing stock.

## 4. The survey

### 4.1 The goods catalogue

**What exists.** §3.7. `goods.json` v4 has nineteen rows with mass and packed volume per unit, a
`family`, a `class` and a `reference_major`; the lane's `reference` map (:2616–2623) names which
vendor's ask is each good's reference for the Co-op share, and `reference_minor` (:2624) the ores'.

**What is missing.** Rows for the part commodities (mass, volume, a reference price, a ledger unit
of one) and an identity for them that a recipe can make and a slot can accept (§3.4, §4.11); a
reference price per hull design; a ladder in which each transformation covers its inputs. Under the
fixture's 4:1 and today's asks a refinery loses 720 cr a tonne; a 5:4 refine (concentrate is mostly
metal; 10 t of ore → 4 t of concentrate → 3.2 t of alloys) costs 350 cr of concentrate at Kasei's
280 bid for a tonne asked at 400, which the facility's ~60 cr/t (§4.4) turns into a loss. The works'
fittings have the same shape: 5 t of alloys and 1 t of carbon ore for 5 t of fittings asked at 400.
The ladder has to move at three rungs, and the movement is lane data. The proposed ladder
(provisional; the coverage arithmetic is §4.4's):

| Good | Today ask / bid | Proposed ask / bid | Why |
|---|---|---|---|
| `metals` | 200 / 280 | unchanged | the mill's 175 cost and the trade run's margin stand |
| `structural_alloys` | 400 / 520 | **450** Kasei / 520 Freeport Works | the 5:4 refine clears 100/t gross, ~40 net; the hauler keeps 70/t |
| `industrial_parts` | 400 / 560 | **520** Yards / **600** Co-op | 5 alloys at 450 + 1 carbon ore = 2,310 for 5 t at 520; the Ceres works needs 600 to cover (§4.4); the hauler keeps 80/t |
| `carbon_ore` | — / 50 | the Exchange **asks 60** from its ore pool; Freeport Works bids 50 at Ceres; Stickney Yards bids 60 | a seller at last (§4.2) |
| `strategic_ore` | — / 600 | the Exchange **asks 700**; Stickney Yards bids 700 | the sight recipes' input, hauled to Phobos |

The part rows, keyed by the hardware row a slot names, spelled `tacsim.part.<kind>.<row>` so a
plant, a sight and a mount can never collide (Q-39-35 amends Q-39-3's spelling), with masses in
kilograms, a ledger unit of one, `class: "part"`, and prices as shares of the line tank's 100,000 so
that the tank's parts and core sum near its basis (the 105 mm round's 150 cr is "a sixteenth of the
tank" by the goods note :204, which this ladder keeps):

| Row | Mass | Reference | Row | Mass | Reference |
|---|---|---|---|---|---|
| `tacsim.part.weapon.cannon_105` | 2,400 kg | 28,000 | `tacsim.part.sensor.visual_mk1` | 90 kg | 9,000 |
| `tacsim.part.weapon.autocannon_30` | 500 kg | 12,000 | `tacsim.part.sensor.thermal_mk1` | 130 kg | 18,000 |
| `tacsim.part.weapon.laser_mk1` | 1,200 kg | 36,000 | `tacsim.part.sensor.ecm_mk1` | 160 kg | 24,000 |
| `tacsim.part.weapon.missile_lr` | 800 kg | 18,000 | `tacsim.part.sensor.eccm_mk1` | 120 kg | 20,000 |
| `tacsim.part.weapon.mg_coax` | 40 kg | 2,500 | `tacsim.part.plant.light_mk1` (the rig, the scout, the drone, the target) | 300 kg | 5,000 |
| `tacsim.part.plant.vehicle_mk1` (the line tanks, the escort, the heavy, the carrier) | 1,800 kg | 22,000 | `tacsim.part.plant.walker_mk1` (the bipeds) | 2,500 kg | 26,000 |
| `tacsim.part.plant.hauler_mk1` (the Dray, the Dray-P) | 30,000 kg | 30,000 | | | |

Thirteen rows. The plant rows are new content (`content/plants/*.json`: a `plant` definition kind
with the mass and the states the power role carries today), named by a `plant` field on every
`power` component, refused at the catalogue build when missing (§4.8, Q-39-35); the corvette's, the
Halcyon's and the relay station's plants and the Halcyon's relay stay unpriced because capital hulls
and stations are not built here (Q-39-20). Volumes follow the mass at 1 m³/t for mounts and plants
and 2 m³/t for crated instruments. A hull's reference price is derived from its bill (§4.7), never
authored, except the programme's `buyout_major`, which stays lane data and is validated at most the
design's bill.

**Questions.** Q-39-1, Q-39-2, Q-39-3, Q-39-35.

### 4.2 The facilities and their supply

**What exists.** The port applier's part j (§3.3) is the precedent: a port row (`workshop`,
`medical`) names a room, an operator org, a capability set and a slot count, and the applier opens
the room as a `FacilityRecord` owned by the operator, refusing drift under hulls (`workshop.rs:52–60`
— which is why a capability added to `p11_service` on a kept world would refuse the apply). Pack
`FacilityDef`s exist for campaign seeds and need a pack path the deploy does not set. The lane's
orgs: Kasei Metals (capital 600,000, :2702), Stickney Yards (50,000, :2703), the Exchange
(4,000,000, :2598), Occator (50,000 + a 250,000 raise, :2599), the Co-op (3,100,000, :2596), the Trust
(250,000, :2597), and **Tycho Mutual Assurance & Salvage** (50,000, faction concord, :2600), an org
with no vendor rows and the salvage word already in its name. The Exchange buys carbon and
strategic ore and sells neither (§3.2).

**What is missing.** Three capabilities on three stations, each a port row the applier opens, with
the cost-model inputs beside the slots — and a supply path for every input a recipe names, between
organizations, with the money and the custody said. Proposed placement (doc 10 §4: Mars/Phobos is
"heavy industry, shipbuilding, military manufacturing"; Ceres "water, propellant production,
logistics"):

| Station | Row | Operator | Capability | Slots | Room |
|---|---|---|---|---|---|
| Stickney Yards | `refinery` | Kasei Metals | `smelt` (a refine row, §4.3) | — | a new `p12_refinery` room off the spine |
| Stickney Yards | `works` | Stickney Yards | `works` | 2 | a new `p13_works` room |
| Stickney Yards | `yard` | Stickney Yards | `yard` | 2 | **a new `p14_slipway` room** — not `p11_service`, whose record the preflight refuses to change |
| Ceres Freeport | `works` | **Freeport Works**, a new org taking the Exchange's alloy bid (the `as` alias becomes the org) | `works` | 2 | a new `c15_works` room |
| Ceres Freeport | the salvage organization | Tycho Mutual | `recovery` (doc 41 reads it) | 1 | its crawler's berth (doc 41) |

No yard at Ceres: the Freeport buys and sells, Phobos builds, and the trade run exists because they
differ. The refinery at Kasei and the Stickney works sit at the end of the concentrate run; the Ceres
works gives the Co-op's fittings bid a local maker fed by the alloys the trade run brings back
and the carbon ore mined on Ceres. Sights and pods (the strategic-ore recipes) are the Stickney
works' alone, on instrument boards hauled from Luna's Crisium Instruments — a third leg the moon
ports (doc 19 A.8) have waited for.

**The supply table** (Q-39-31). Every recipe input, where its maker buys it, from whom, and how the
matter and the money move:

| Input | At | Bought from | How |
|---|---|---|---|
| `metals` for the refine | Kasei | miners and traders, into Kasei's bid at 280, which gains `refresh { per_ic_day: 40, cap: 4000 }` so the refine's consumption re-posts it (the ore drips' shape, `stock.rs:906`; today a filled intake waits for the 90-day TTL) | the market's sale into the bid; the pool is the refine row's ore pool |
| `structural_alloys` for the Stickney works | Stickney | Kasei's ask at 450, next door | a house purchase on the port tick: a receipted in-kind transfer from Kasei's storefront pool to the works' pool at the ask, paid from Stickney Yards' account (the dispatcher's `transfer` precedent, `port/yard.rs`, with a payment post); a player's order sources from the player's own hold |
| `structural_alloys` for the Ceres works | Ceres | Freeport Works' own bid at 520 (the alias becomes the org and keeps the bid) | the market's sale into the bid; the pool is the works' |
| `carbon_ore` for the Stickney works | Stickney | the Exchange's new ask at 60 at Ceres, hauled by a trader (a second outbound cargo beside concentrate); or Stickney Yards' own bid at 60 | the trade run |
| `carbon_ore` for the Ceres works | Ceres | Freeport Works' own bid at 50 (the Exchange's carbon bid moves to the works with the alloy bid) | miners sell carbon to the works directly |
| `strategic_ore` for the sights | Stickney | the Exchange's new ask at 700 at Ceres (the prize shelf sells), hauled; or Stickney Yards' bid at 700 | the trade run |
| `instrument_boards` for the sights and the laser | Stickney | Crisium Instruments' ask at Luna (ref. 900), hauled | the Luna leg |
| `industrial_parts` for the part recipes | Stickney, Ceres | the works' own fittings output, pooled (§4.11) | in-house |
| the hull's bill | Stickney | the requester's hold or the Stickney vendors' asks, into the order's escrow (§4.12) | `+yard/deliver` or a purchase delivered to escrow |

**Questions.** Q-39-4, Q-39-5, Q-39-23, Q-39-31.

### 4.3 Recipes against mill rows, and conservation

**What exists.** Two engines that convert matter. The mill (§3.1) is a house process on the port
tick: one input, one output, a fixed ratio, a daily cap, no requester, no fee, its margin the ratio,
already inside the census pin, authorable as lane data, and barred from a second transformation
only by the same family rule. The work order (§3.3) is a requester's act with up to ten inputs, a
facility slot, a work-unit duration, E4 holds and a receipted delivery, driven today by `@wo` or by
R9a's foundry code — no NPC house runs one.

**What is missing.** A rule for which transformation uses which. Q-38-13 allows both. The refine at
Kasei is one input, one output and a house's own business; the works' fittings take two inputs; the
part recipes three; a hull a bill of ten lines and a delivery that is not a lot. The proposed rule:
**a house conversion with one input is a mill row with the family rule relaxed; everything with two
or more inputs, or a requester, is a recipe.** Kasei's row becomes `{kind: refine, ore: metals, into:
structural_alloys, ratio: {ore: 5, product: 4}, batch_t: 20, per_ic_day_t: 40}` (`MillDef` gains
`kind`, under which `check_ports` lets the product's family differ); its ledger, its restock and its
drift page come for free. Recipes live in a live pack, `content/industry/recipes-v1.json`, named by
the deploy template beside the relay pack. Each recipe says how its output is lotted (§4.11):
`output_lots: pooled` for a bulk good (the delivery's lot is dissolved into the destination's pool
by the facility's arm on its next fire, and `restock_ask` lists it when the destination is a
storefront), `output_lots: each` for a part (one one-unit lot a unit, grade 9, a provenance row
written by tacsim's commission hook from the `wo_applied` fact):

| Recipe | Inputs | Output | Lots | Requires | Work units | Loss |
|---|---|---|---|---|---|---|
| `fittings` | 5 `structural_alloys`, 1 `carbon_ore` | 5 `industrial_parts` | pooled | `works` | 6 | 1 t |
| `mount_105` | 3 `structural_alloys`, 1 `industrial_parts` | 1 `weapon.cannon_105` | each | `works` | 120 | 1.6 t |
| `mount_30` | 1 `structural_alloys`, 1 `industrial_parts` | 1 `weapon.autocannon_30` | each | `works` | 60 | 1.5 t |
| `mount_mg` | 1 `industrial_parts` | 10 `weapon.mg_coax` | each | `works` | 24 | 0.6 t |
| `mount_laser` | 2 `structural_alloys`, 2 `industrial_parts`, 1 `instrument_boards` | 1 `weapon.laser_mk1` | each | `works` | 160 | 3.8 t |
| `rack_lr` | 1 `structural_alloys`, 1 `industrial_parts` | 1 `weapon.missile_lr` | each | `works` | 72 | 1.2 t |
| `sight_visual` | 2 `strategic_ore`, 1 `instrument_boards`, 3 `industrial_parts` | 10 `sensor.visual_mk1` | each | `works` | 60 | 5.1 t |
| `sight_thermal` | 3 `strategic_ore`, 2 `instrument_boards`, 3 `industrial_parts` | 10 `sensor.thermal_mk1` | each | `works` | 120 | 6.7 t |
| `pod_ecm` | 3 `strategic_ore`, 2 `instrument_boards`, 2 `industrial_parts` | 10 `sensor.ecm_mk1` | each | `works` | 120 | 5.4 t |
| `pod_eccm` | 3 `strategic_ore`, 2 `instrument_boards`, 2 `industrial_parts` | 10 `sensor.eccm_mk1` | each | `works` | 120 | 5.8 t |
| `plant_light` | 1 `industrial_parts`, 1 `carbon_ore` | 5 `plant.light_mk1` | each | `works` | 60 | 0.5 t |
| `plant_vehicle` | 2 `structural_alloys`, 1 `industrial_parts`, 1 `carbon_ore` | 1 `plant.vehicle_mk1` | each | `works` | 96 | 2.2 t |
| `plant_walker` | 3 `structural_alloys`, 1 `industrial_parts`, 1 `carbon_ore` | 1 `plant.walker_mk1` | each | `works` | 120 | 2.5 t |
| `plant_hauler` | 25 `structural_alloys`, 8 `industrial_parts`, 2 `carbon_ore` | 1 `plant.hauler_mk1` | each | `works` | 480 | 5 t |
| hulls | the design's bill (§4.7) | a `tacsim.hull_kit.<design>` lot (§4.13) | each | `yard` | the design's days × 24 | per design |

Lines are whole tonnes because `RecipeLine.units` are ledger units; small parts come ten or five to
a batch and are delivered as that many one-unit lots. Every row's loss is the difference between
its inputs' mass and its outputs' catalogue mass — slag, off-gas, swarf — and O-3's "no tailings"
stays the mill's rule only. Carbon ore goes straight into the works, not through a feedstock pressing
step (Titan's `carbon_feedstock` is a different good for a different trade), keeping the chain three
deep (Q-39-23). Strategic ore feeds the sights and pods now; the core track's dies stay a receipted
seed (Q-39-22).

**Conservation (Q-38-14).** Every mint is one of: the stock clerk's receipted seed (`stock.rs:420`),
a recipe's delivery (`provider.rs:3792`, the order as receipt), a mill or refine batch's credit
(`mill.rs`, the ledger as receipt), a strip or scrap mint (`act.rs:785`, `bay_{why}/{hull}/{act}`
as receipt), a remains scrap (`tow.rs:1781`). The pin widens to count `.mint_origins(` with the
same file list, and a second pin asserts the receipt kinds: every call site names an `ExternalRef`
whose `kind` is one of a closed set. Above the pins, one **matter story** (IN.11) runs the chain end
to end and asserts Σ mined = Σ (pools + lots + hull bills + declared losses) at the end. The seeds
(Kasei's 7,000 t, the Yards' 5,400 t) stay as the opening stock; their `intake` caps — which today
bound a house's one-time buying — become the facility's consumption: Kasei bids concentrate because
its refine row consumes 40 t a day, and the bid's refresh is the row's `per_ic_day_t`, so a drained
house stops bidding and a fed one keeps buying. That is what "the intake drips retired" means here:
not the ore bids at Ceres (the ore economy's size knob stays), but the one-time intakes at the
refinery and the works.

**Questions.** Q-39-6, Q-39-7, Q-39-8, Q-39-22, Q-39-23.

### 4.4 The facility cost model (Q-38-12), and whether each site covers it

**What exists.** The tariff (`workshop_tariff.json:3–17`; its note :18 gives the basis): one IC hour
of bay labour at 100 cr (the core-fitting fee), a structural point 36 s and 1 cr, a component repair
two hours (200 cr), a replacement four hours (400 cr), a section rebuilt eight hours (800 cr) plus
its points, the operator's multiplier 0.5–2.0. Nothing derives it. `mush-economy`'s `FITTING_MINOR`
(100 cr) is the one other labour figure. The E4 hold on a work order is one minor unit an input unit
(§3.3).

**What is missing.** The model, and the proof that each site clears it. Draft 0.1 billed whole
slot-days, which charges a six-work-unit batch of fittings a full day; billing is **by work unit**,
which is the engine's own grain (`PRODUCTION_CADENCE_SECS`). Proposed:

```
daily_cost      = capital / lifetime_days + capital × upkeep_permille / 1000 / 365 + staff × wage
slot_day        = daily_cost / slots
slot_hour       = slot_day / 24                                   (one work unit an IC hour)
fee(order)      = work_units(order) × slot_hour × (1 + margin_permille / 1000)
manufacturing   = fee at margin 0                                   (Q-38-12's "before labour and margin"; labour is the staff term)
hold(order)     = fee(order)                                         (replaces quote_price's one minor an input unit, Q-39-28)
```

`FacilityCostDef { capital_major, lifetime_years, upkeep_permille, staff, wage_major,
margin_permille }` is authored on each facility row of the lane beside its slots; the applier
stores it on the record's policy half (a `policy_revision` bump, so an order drafted against the old
numbers re-validates — `facility.rs` already has the revision). Provisional inputs, with a wage of
150 cr an IC day a head (provisions are 10 cr a crew-day; a licence 15 cr a day; a captain's stake
25,000):

| Facility | Capital | Life | Staff × wage | Upkeep | Daily cost | Slots | Slot-hour |
|---|---|---|---|---|---|---|---|
| Stickney Yards yard | 10,000,000 | 30 y | 12 × 150 = 1,800 | 2 %/y = 548 | 913 + 548 + 1,800 ≈ **3,300** | 2 | **68.75** (1,650 a slot-day) |
| Kasei refinery (`smelt`, a refine row: no fee, a house cost) | 6,000,000 | 30 y | 10 × 150 = 1,500 | 2 % = 329 | 548 + 329 + 1,500 ≈ **2,400** | — | — |
| a works (Stickney, Ceres) | 3,000,000 | 25 y | 8 × 150 = 1,200 | 2 % = 164 | 329 + 164 + 1,200 ≈ **1,700** | 2 | **35.4** (850) |
| a workshop (Ceres 2 berths; Stickney 4) | 1,500,000 | 20 y | 6 × 150 = 900 | 3 % = 123 | 205 + 123 + 900 ≈ **1,250** | 2 / 4 | 26 / 13 (625 / 313) |
| Tycho Mutual's recovery (crawler + lift) | 4,000,000 | 15 y | 6 × 150 = 900 | 4 % = 438 | 730 + 438 + 900 ≈ **2,100** | 1 | 87.5 (2,100) |
| a mill (the Exchange's, Occator's): a crusher shed, outside the fee model | 300,000 | 20 y | 2 × 150 = 300 | 2 % = 16 | 41 + 16 + 300 ≈ **360** | — | — |

**Coverage, site by site** (review 2 #8; Q-39-32). At full utilization a fee facility covers its
daily cost by construction (the slot-hour is the daily cost over the slot-hours) and the margin is
its profit; what has to be shown is that the house's own production clears the cost at the ladder's
prices, and that a customer's order is worth placing:

| Site | Per day at the cap | Gross | Daily cost | Net | Reading |
|---|---|---|---|---|---|
| the Exchange's mill, 40 t of ore → 16 t of concentrate | 16 × (200 − 175) | 400 | 360 | +40 | thin; the ratio is the owner's knob (doc 20 O-1) |
| Occator's mill, 40 t of ice → 16 t of water | 16 × (100 − 87.5) | 200 | 360 | **−160** | the water ask covers the crusher only at 110; flagged for the tune, not moved here (the propellant business carries Occator) |
| Kasei's refine, 40 t of concentrate → 32 t of alloys | 32 × 450 − 40 × 280 | 3,200 | 2,400 | +800 | covers at 75 % of the cap |
| the Stickney works, 8 batches of fittings (48 work units over 2 slots) | 8 × (5 × 520 − 5 × 450 − 60) | 2,320 | 1,700 | +620 | covers at 73 %; alloys at Kasei's ask |
| the Ceres works, 8 batches of fittings | 8 × (5 × 600 − 5 × 520 − 50) | 2,800 | 1,700 | +1,100 | covers at 61 % with the Co-op's bid at 600; at 560 it loses 500 a day |
| the yard, 48 work units | 48 × 68.75 × 1.25 | 4,125 | 3,300 | +825 | the fee covers; an idle slot is the house's loss, as capital is |
| a workshop, 2 berths × 24 h at the tariff | 48 × 100 × 1.2 | 5,760 | 1,250 | +4,510 | the tariff is 3.8× the floor; left as it is (Q-39-9) |

The mills' rows were in Draft 0.1's fee table as if the model priced them; it does not — a mill is a
house process whose margin is its ratio — so the table above shows their coverage honestly and
withdraws them from the fee model. A customer's fittings order at the Stickney works is 6 × 35.4 ×
1.25 = 265 cr for 5 t of fittings on the customer's own alloys and carbon: 53 cr a tonne against a
520 ask, so a player who hauls alloys and carbon to the works and sells fittings to the Co-op at
600 clears about 127 cr a tonne after the fee — the first production story's margin (IN.5).

**The arithmetic for two hulls**, on §4.8's partition (parts and the core off the top, then
structure and fittings from the remainder, the bill's lines rounded up to whole tonnes, the
remainder declared as the build's loss) at the proposed references:

| Line | Escort (`tv_escort`, 8 t, class vehicle) | Dray (240 t, class hauler) |
|---|---|---|
| parts (one lot a slot) | cannon_105 28,000 + mg_coax 2,500 + visual_mk1 9,000 + plant.vehicle_mk1 22,000 = 61,500 (4,330 kg) | plant.hauler_mk1 30,000 (30,000 kg) |
| core | Class II line core 8,000 (250 kg) | none (Class 0) |
| assembly loss (3 %) | 240 kg | 7,200 kg |
| remainder for structure and fittings | 8,000 − 4,330 − 250 − 240 = 3,180 kg | 240,000 − 30,000 − 7,200 = 202,800 kg |
| alloys (0.75 of the remainder) | 2,385 kg → **3 t** × 450 = 1,350 | 152,100 kg → **153 t** × 450 = 68,850 |
| fittings (0.25) | 795 kg → **1 t** × 520 = 520 | 50,700 kg → **51 t** × 520 = 26,520 |
| build (1 day per 20 t, floor 2) | 2 IC days = 48 work units × 68.75 = 3,300 | 14 IC days = 336 × 68.75 = 23,100 |
| margin on the build | 25 % = 825 | 25 % = 5,775 |
| **bill** | **≈ 75,500** | **≈ 154,000** |

Against the tree: the escort's 15,000 lease deposit is a fifth of its bill; the Dray's 90,000 buyout
is 58 % of a new one, which is what a stake-financed used hull should cost; the line tank's 100,000
basis stays the core-price basis. The Dray's "two weeks" (Q-38-12) is 12 days by the 20 t rule and
14 by the owner's sentence — the table takes 14 and the rule needs the owner's eye (Q-39-14). The
workshop row prices a berth-hour at 26 cr (Ceres) against the tariff's 100: the model's figure is
the floor an operator's multiplier may not go under, and the tariff's rows are left as they are this
round (Q-39-9).

**Questions.** Q-39-9, Q-39-10, Q-39-24, Q-39-32.

### 4.5 The part lot: price, stores, bill, and what feeds the stores (Q-38-4)

**What exists.** §3.4. The bay's draw (`bay_work/act.rs`) takes an order's goods from the carrier's
hold when the work is done, all or nothing; `+bay/fit` takes a traced, clean lot from the hold; strip
and scrap put lots into the hold; `+market/list`, `+market/quote` and `+market/sell`
(`mush-market/src/interface.rs:44/58`, `trade.rs:1620`) trade at a port; a vendor's stock is a
storefront pool with receipted seeds the clerk dissolves (§3.8); `bay_work/mod.rs:34–37`: "the
workshop supplies a replaced part, the hold everything else".

**What is missing.** The store a `Replace{lot: None}` draws from, its price, the seed and the
provenance of a part that was never stripped — and a rule that keeps a part a part through all of
it, which is §4.11's. Proposed: the facility's stores are **the operator org's storefront vault at
that port** (Occator's at Ceres, Stickney Yards' at Stickney) — the same vault its stock rows mint
into — holding part lots as identified one-unit lots beside its pooled bulk. `Replace{lot: None}`
becomes `Replace{lot: Store}`: the quote names the catalogue price beside the labour, the order
takes custody of the lowest-grade clean lot of the slot's row in the store (`BayPartCustody`, §3.3)
so no sale or move can take it meanwhile, the draw consumes it and the fee pays the operator; an
empty store refuses at the quote — `NO_PART`: "The workshop has no <row> in its stores; +market/list
the yard, or bring one in the hold." — exactly as `NO_HULL` refuses the desk. Seeds until the
recipes land: part rows in the vendor's `stock` (`{good: "tacsim.part.weapon.cannon_105", qty: 6,
grade: 9, issue: 1}`) through the stock clerk's receipt, minted as six one-unit lots with `Seeded`
rows and never dissolved (§4.11), Occator at Ceres carrying four of each escort and rig part, the
Yards at Stickney six of every row. `PartProvenance` gains an origin: `Stripped {parent, slot,
claim…}` (today's), `Manufactured {order, facility}`, `Seeded {receipt}`; `+bay/fit` accepts all
three (a "traced" lot is one with a provenance row of any origin), and the hot-part rule reads
`Stripped` only while every origin keeps its condition. A part's bill of materials is its recipe
(§4.3); a strip costs labour only and returns the part at its condition's grade; a scrap returns the
parts as lots and the structure as alloys (§4.6). The hold's room check counts parts by catalogue
mass (§4.1), so a Dray-P's 20 t rig slot and 86 t hold weigh a cannon.

**Questions.** Q-39-11, Q-39-12, Q-39-13, Q-39-27.

### 4.6 The scrap ceiling (Q-38-5)

**What exists.** §3.5: 100 kg a point, every point, into the hold, no fee, no cap, phantom heat
points, a sub-tonne yield refused.

**What is missing.** The mechanism, and (review 2 #11) a ruling on what the ceiling bounds. The
ruling gives two bounds and the fact that a facility takes the rest. Two hulls show that no single
tuned constant satisfies both bounds: the escort's whole structure (§4.8's partition: 2,385 kg) at a
recovery fraction `r` 0.6 is 1,431 kg of alloys, ~640 cr at the 450 reference, far under a fifth of
its 75,500 bill (15,100) — the mass bound binds; the Dray's 152 t of structure at 0.6 is 91 t,
41,000 cr, far over a fifth of its 90,000 buyout (18,000) — the value bound binds. Proposed:

```
structure_kg     = structure_kg_per_point × surviving structural points        (§4.8; heat cells never counted)
mass_yield_kg    = structure_kg × r                                             (r 0.6, provisional)
ceiling_kg       = reference_price / 5 / alloy_reference × 1,000                (18,000 / 450 → 40,000 kg for a Dray)
yield_kg         = min(mass_yield_kg, ceiling_kg)
minted_t         = floor(yield_kg / 1,000)                                      (§4.10: whole tonnes in kind, the fraction a declared loss)
credit           = yield_kg × alloy_reference / 1,000 − scrap_fee               (the money on the exact kilograms)
```

At a workshop, **the whole mass yield goes into the operator's stock** in whole tonnes (Q-38-5's
"the port's stock"), minted at grade 50 under the `bay_scrap` receipt, and the title holder is
credited `yield_kg` at the reference less the scrap job's fee from §4.4 (a Dray is 288 work units of
yard time, 20,000 cr at the slot-hour; an escort 48, 3,300): the facility keeps the tonnes above the
ceiling in kind, which is the owner's "a navy pays its scrappers". For a Dray: 91 t into the Yards'
pool, 40 t × 450 = 18,000 credited, 20,000 of fee — **a whole Dray does not pay to scrap at the
yard's rate**, and should not: the point of the ceiling is that scrapping a hull is not a way to
make money, and a holder who wants the plant strips it first. For an escort: 1 t into the pool,
644 cr credited, 3,300 of fee. For remains on a site, where there is no facility, `minted_t` alone
is minted into the carrier's hold with the room check and no fee, and the rest is lost to the ground:
scrapping a Dray's remains on Urvara yields 40 t, a whole hold and a half of a Dray-P; an escort's
yields 1 t. The parts come off as lots at their own value in both cases — **and whether that is what
the ruling meant is Q-39-25's, raised to the owner and not decided here**: review 2 reads doc 38's
"the scrap of a whole hull is worth a fifth of its buyout at most" as bounding everything the hull
yields, parts included, and at the Ceres bid rather than the reference (40 t × 520 = 20,800).
`SCRAP_KG_PER_STRUCTURAL_POINT` and `FALLBACK_SECTION_SCRAP_KG` retire; the per-design figures come
from the catalogue build (§4.8). `reference_price` is the design's bill (§4.7), or the programme's
`buyout_major` for a programme hull.

**Questions.** Q-39-16, Q-39-17, Q-39-25.

### 4.7 Unit construction (Q-38-12)

**What exists.** §3.6 for where hulls come from and why none of it serves a built one; §3.3 for the
engine. The applier's `lay_up_one` (`port/hull.rs:655`) is the one path that makes a ship whole —
Thing, unit, rooms, store, airlock, bays, placement, title — and `vehicle::lay_up_rig` (:643) the
one for a rig; both run only from a port entry's programme, both need `logistics` and `interior`,
and the programme's `pool` is a ceiling the preflight enforces. `DeliveryTarget::Endpoint` is the
work order's one way to deliver, and the delivery is a prepare with no world. Q-31-2 lets a
registry-only design be built; only staff `@unit/create` does it; doc 31 §9 (l.447) left "what a
design costs to register and to build" to doc 08.

**What is missing.** The yard order, its escrow (§4.12), its delivery and handover (§4.13), and the
pools' funded way back to it.

- **The bill.** `design.bill = { parts: one row a slot (from the component list and its plant
  field), core: the design's core class or none, alloys_t: ceil(structure_kg / 1,000), fittings_t:
  ceil(fittings_kg / 1,000), days: max(2, ceil(dry_mass_t / 20)) }`, computed at the catalogue build
  from §4.8's partition and the goods table, with authored overrides on the unit def (`bill: {…}`)
  for a design the rule misjudges. The price is the bill at the references plus the yard's
  manufacturing cost and margin (§4.4's table). A buildable design needs `logistics` (its dry mass
  and stowage) and an `interior` (§4.13); the thirteen simulator units get class templates in IN.9
  (`armour_cab`, `walker_cab`: a cab room with a console, the stowage a locker) for the line-tank
  family, the heavy, the scout and the two bipeds, and the drone, the carrier, the target, the relay
  station and the capital hulls stay unbuildable (Q-39-20, Q-39-35). The check refuses a bill whose
  partition exceeds the dry mass.
- **The order.** `+yard` at a port whose `yard` row exists, the way `+bay` fronts the bay: `+yard`
  lists the designs the yard builds and the orders standing, `+yard/quote <design>` prints the bill,
  the days and the fee, `+yard/order <design>` books the order (§4.12: an escrow endpoint at the
  yard, the booking fee held), `+yard/deliver <order>=<good> <qty>` moves bill lines from a hold
  docked at the port or from a part lot the requester holds into the escrow, in as many trips as the
  bill takes (a Dray's 204 t over a 100 t hold is three), or buys them at the port's asks straight
  into escrow; the order submits itself when the bill is covered, the fee held and the slot reserved
  for the days; `+yard/cancel` under §4.12's charges; `+yard/collect <order>` is the handover
  (§4.13). `@wo` stays the staff and test grammar under it. `+works` is the same front at a works for
  the fittings and part recipes (Q-39-30).
- **The delivery.** §4.13: a `tacsim.hull_kit.<design>` lot into the escrow, then the commission.
- **The pools** (Q-39-34, revising Q-39-21). The content pools are houses with floors: the Trust
  keeps two free Drays, Occator two free rigs and one escort. A house orders at the yard **when its
  account covers the bill**, never on the floor alone: the Trust's 250,000 posts eight 25,000 stakes
  and is at 50,000 after the eighth enrolment, so a 154,000 Dray is funded by the buyouts that come
  back to it (90,000 each, `buyout.rs`) and by a lane raise of 250,000 to the Trust (`raises`,
  Occator's precedent, :2599) so the first reorder does not wait on the second graduation. House
  orders deliver at Stickney, where the yard is; **a house ferry** on the port tick files the
  passage to Ceres under the house's own helm principal (a DARK dispatcher Thing at the helm, the
  Occator Yard dispatcher's shape, `port/yard.rs`, sailing on `Standing::Fleet`'s precedent of a helm
  without a charter, `coop.rs:598–603`), and the hull joins the pool when it docks at Ceres; rigs
  ride the ferry Dray-P's bay. `NO_HULL` says which it is: "the Trust has no Dray free and cannot
  yet fund one" or "the Trust has a Dray on order at Stickney, due at Ceres in n IC days". Built
  hulls are **never programme rows** (§4.13's register), so the pool of eight stays the seed and the
  applier's preflight never sees a ninth. Q-18-70's shrink is answered by production, not reversed.

**Questions.** Q-39-14, Q-39-18, Q-39-19, Q-39-20, Q-39-21, Q-39-33, Q-39-34.

### 4.8 The mass budget: one rule across mining, milling, repair and scrap

**What exists.** Mining draws a deposit's tonnes into a hopper and a hold by measure (doc 20 §3, the
census once a bundle); the mill is 5:2 by its ratio; repair draws 10 kg of fittings a point; scrap
returns 100 kg of alloys a point and counts heat cells; a part weighs nothing. Three constants, none
from the hull. Draft 0.1 wrote σ and φ as fractions of the dry mass beside the parts, so an escort's
bill massed 10.7 t for an 8 t hull (review 2 #1).

**What is missing.** The hull's own figures from **one complete partition**:

```
dry_mass_kg      = parts_kg + core_kg + loss_kg + structure_kg + fittings_kg
parts_kg         = Σ the catalogue mass of one part a slot (§4.1; a plant by its plant row)
core_kg          = 150 / 250 / 500 kg for Class I / II / III, 0 for Class 0            (provisional)
loss_kg          = dry_mass_kg × loss_permille / 1000                                   (30 ‰, the assembly loss)
remainder_kg     = dry_mass_kg − parts_kg − core_kg − loss_kg                            (refused when ≤ 0)
structure_kg     = remainder_kg × structure_share(class)
fittings_kg      = remainder_kg − structure_kg
structure_kg_per_point = structure_kg / total structural points                           (armour + internals; never a heat cell)
fittings_kg_per_point  = fittings_kg / total structural points
```

`structure_share` by `UnitClass`: armour 0.75, vehicle 0.75, walker 0.70, hauler 0.75, drone 0.65
(provisional); capital and station are not partitioned (not built here). The escort: 3,180 kg of
remainder → 2,385 kg of structure and 795 of fittings, 0.80 kg of alloys and 0.27 kg of fittings a
point; the Dray: 202,800 → 152,100 and 50,700, 83.6 kg and 27.9 kg a point. A design with no
`dry_mass_t` (the simulator units) takes its class template's (§4.13), and an authored `bill`
override wins over the rule. Then, with §4.10's rounding:

| Act | Draws | Returns | Today |
|---|---|---|---|
| armour and rebuild | `points × structure_kg_per_point` of alloys **and** `points × fittings_kg_per_point` of fittings, from the hull's opened bale (§4.10), plus labour | — | 10 kg of fittings a point, no alloys |
| replace | one part lot (its catalogue mass) plus labour | the destroyed part as nothing | a part from nothing |
| strip | labour | the part at its grade | the same |
| scrap at a workshop | the scrap fee | §4.6's yield to the operator in whole tonnes, the credit to the holder on the kilograms, parts to the hold | 100 kg a point to the hold |
| remains scrap | — | §4.6's capped yield to the hold in whole tonnes | 100 kg a point |
| build | the bill, rounded up a line; the remainder the build's loss | the hull | — |

An escort's armour patch of 100 points draws 80 kg of alloys and 27 kg of fittings from the bale; a
Dray's draws 8.4 t and 2.8 t — 3,760 and 1,450 cr, which is what a 240 t hull's hundred points
should cost. The imbalance resolves because repair draws 1.07 kg a point and scrap returns at most
0.48 (0.80 × 0.6): repair-then-scrap loses mass and money. `structural_points_per_industrial_parts_t`
leaves the tariff with the two scrap constants; the tariff keeps labour and time only. Milling keeps
its ratio and O-3; mining is already conserved.

**Questions.** Q-39-15, Q-39-26, Q-39-35.

### 4.9 Acceptance

Q-38-29 (ruled): every new cut's story runs at 256 and 512 and the bot types the player's half over
SSH; the bot's four-digit addresses and vocabulary are the first cut of whichever document starts
first. The chain's stories are port-side, so "both sizes" means the mining half runs on Ahuna (256)
and Urvara (512) and the port half once; the bot gains `+yard`, `+works`, `+market/sell` of a part
lot and the `+bay/order replace` quote. Every cut in §6 names its story through the production
command paths (`@wo`, `+bay`, `+yard`, `+works`, `+market`, `+coop`), never a staff hand past the
seams doc 36a §8 allows — and, after review 2's "adversarial acceptance", the stories include the
ones that try to break the contract: sell → buy → fit, a disputed part in a build, a full 20-day
order, a late cancel, two orders competing for one slot and one bale, and a failure between the
inputs' consumption and the hull's title.

### 4.10 The accounting unit and the rounding policy

**What exists.** `structural_alloys` and `industrial_parts` are whole-tonne goods in the ledger and
on the market (§3.2); recipe lines, reservations, pools, lots and the bay's draw are integers of
that unit; the ores and the concentrates are tonnes too; a part has no unit at all. Every mass this
document computes — 2,385 kg of structure, 80 kg a point, 1,431 kg of yield — is sub-tonne, and
every rounding the engine would do today is either up (the quote's `div_ceil`, a tonne of fittings
for one point) or down to a refusal (`tow.rs:1772–1774`).

**What is missing.** A unit for computation, a unit for the ledger, and a rounding rule per act so
that no act creates matter and none hands it out free. Proposed (Q-39-26): **kilograms for every
computation, tonnes for the ledger and the market, and one rounding rule an act**:

- *Computation.* The catalogue build and every quote carry kilograms (`si.mass_kg` is already the
  measure every good declares, goods :4); the partition (§4.8), the per-point figures, the scrap
  yield and the bill's lines are kilograms until the moment they touch the ledger.
- *A bill* rounds each bulk line **up** to whole tonnes; the rounded remainder (615 kg of alloys and
  205 of fittings for an escort) is declared as part of the build's loss line, so the matter story
  sums. Parts are one lot a slot and round nothing.
- *A repair* draws from **the hull's opened bale**: the first repair on a hull debits one whole tonne
  of alloys and one of fittings from the source hold or store into the hull's bay record
  (`BayHull.bale: { alloys_kg, fittings_kg }`), and every repair after it consumes kilograms from
  the bale until a line runs short, when the next whole tonne is debited. The bale rides the hull
  (a carrier's bay or a workshop may continue it), is the title holder's, and is paid out in kind at
  scrap (its kilograms join the yield) — so a hull with an opened bale owes nothing and holds
  nothing it did not pay for. A one-point repair on a fresh hull therefore costs a tonne of each
  good up front and 1.07 kg of them for real, which is the tonne a yard would open anyway.
- *Scrap* mints **down** to whole tonnes into the receiving pool and declares the fraction a loss;
  the holder's credit is computed on the exact kilograms (§4.6), so the money never rounds against
  the holder and the matter never rounds for them. A yield under a tonne mints nothing and credits
  the kilograms — no refusal.
- *A recipe* is whole tonnes in, whole units out, by construction; a part lot is one unit of mass
  `si.mass_kg` and never splits (§4.11).
- *The market* keeps `quote_unit: "t"`; a part is quoted each.

The alternative — a kilogram ledger — makes every balance exact and moves the lane's every stock
row, the mills' `batch_t` and `per_ic_day_t`, `MAX_QUANTITY`, the hold capacity plan and every test
that asserts a tonne; it is the cleaner model and the larger migration, and nothing in play needs
the precision the bale does not give.

**Questions.** Q-39-26.

### 4.11 The component lifecycle: one identity from seed to scrap

**What exists.** A part is a one-unit lot with a `tacsim.bay_parts` row keyed by its lot id (§3.4).
Every other path the engine offers for a good treats it as pooled or batched: the stock clerk mints a
row's `qty` as one lot and dissolves it (§3.8); a work order reserves and consumes pools and delivers
one lot a line with `units × quantity` in it (§3.3); the market's sale makes a new tranche lot
(§3.8); `@inv/split` makes a child with `Lineage::Split` and no provenance row; `bay_refit` takes
custody of one-unit lots only and returns the removed part as a lot with an `industry` origin and no
tacsim row; `+bay/fit` consumes the whole lot. Draft 0.1's §4.5 and §4.7 assumed the seed, the sale,
the recipe and the build would all keep a part a part; none of them does.

**What is missing.** A lifecycle in which a component's identity, condition and provenance survive
every hand it passes through. Proposed (Q-39-27): **a part is an identified one-unit lot from the
moment it exists to the moment it is fitted or scrapped, and no path pools, batches or splits it.**

| Stage | Rule | Where |
|---|---|---|
| **born** (seed) | a part row in a vendor's `stock` mints `qty` one-unit lots, each with a `Seeded { receipt }` provenance row, in the same bundle as the receipt; the clerk never dissolves a `class: part` lot | `stock.rs` the stock provider; the dissolve step skips part rows |
| **born** (recipe) | a recipe with `output_lots: each` mints one one-unit lot a unit at grade 9 (`wo_deliver` reads the recipe's lotting, not a goods table); tacsim's commission hook, on the `wo_applied` fact, writes a `Manufactured { order, facility }` row for each | `provider.rs:3778–3796` gains the per-unit branch; a tacsim fact observer writes the rows |
| **born** (strip, scrap) | as today: `Stripped { parent, slot, claim, legality… }` | `strip.rs`, `act.rs` |
| **born** (refit) | `bay_refit`'s removed part gets a `Stripped` row written by the same tacsim observer from the refit's completion fact, so the two bay paths share one identity (review 1 §3.4) | `bay_refit/provider.rs:535–552` unchanged; the observer |
| **held** | a part is never pooled: `@inv/dissolve`, `@inv/split` and `@inv/merge` refuse a `class: part` commodity; the hold's capacity plan weighs it by its catalogue mass | `mush-inventory` verbs read the class from the goods table through a composition seam (as the recipe authority does, no tacsim edge) |
| **sold** | a part is listed as a lot (`Representation::Lot`), and a sale transfers **the lot itself** to the buyer's endpoint — the same id, the same row — never a tranche; `+market/sell <lot>` and the storefront's lot asks | `mush-market`'s lot path; the clerk lists part lots as lot asks, not `Supply::Stock` |
| **reserved** | an order (a replace, a fit, a build) takes custody of the exact lot through `BayPartCustody` (`bay_part.rs`), so no move, sale or split can take it before the draw | the existing custody, reused by `+bay/order replace`, `+bay/fit` and `+yard` |
| **fitted** | the lot is consumed and the row records `fitted_into`; the hull's own part census (`tacsim.hull_provenance`, §4.13) lists the lot ids it was built or refitted with, so a build keeps every input's ancestry — a `Stripped` part fitted into a new hull stays `Stripped` with its claim and condition, and a hot part is refused at the build as at the fit | `act.rs:719–740`; the commission |
| **scrapped** | the row records `scrapped`; the parts come off as lots (§4.6) | `strip.rs:282–299` |

The hot-part rule reads `Stripped` rows only (a seeded or manufactured part has no claim to be hot
under), and the condition is the row's whatever the origin. Grade stays the catalogue's 9 or 3
(`hardware/bay.rs:30–31`). Nothing here changes the bulk goods' pooling: fittings and alloys are
pools, and the works' fittings recipe delivers `pooled`.

**Questions.** Q-39-27.

### 4.12 The order contract: authority, escrow, cancellation, delivery

**What exists.** §3.3: a work order is drafted by staff or by the facility owner's own people,
reserves pools for seven IC days from a source endpoint, holds one minor unit an input unit, advances
to `AwaitingDelivery` and waits for a hand to deliver it, and can be cancelled from any state short
of terminal with everything returned. Doc 08 §10.2's nine steps describe the order this engine was
built to run; the engine has the steps and not yet the contract between a customer and a house.

**What is missing.** The public contract. Proposed (Q-39-28, Q-39-29, Q-39-30), as a state machine
`+yard` and `+works` front and the house arm drives:

| State | Entered by | What stands |
|---|---|---|
| **Quoted** | `+yard/quote`, `+works/quote` | nothing held; the bill, the days and the fee printed from the design and the `FacilityCostDef` |
| **Booked** | `+yard/order`, `+works/order` | an **escrow endpoint** for the order at the facility (a vault of the requester's, named by the order, titled to the requester); the **booking fee** held (one slot-day at the facility's slot-hour × 24, forfeit on cancel); a draft work order whose `source` is the escrow |
| **Supplied** | `+yard/deliver <order>=<good> <qty>` from a hold docked at the port or a part lot the requester holds, in as many deliveries as the bill takes; or a purchase at the port's asks delivered straight into the escrow | the bill's bulk lines in the escrow's pools, the part lots in `BayPartCustody` for the order; custody is the requester's throughout |
| **Running** | the arm's submit, on the fire after the bill is covered | the slot reserved for the days; the materials reserved with a deadline of **the order's duration plus `RESERVATION_TTL_SECS`** (the provider stamps `reservation_deadline(now + work_units × 3,600)`); the fee held at `fee(order)` (§4.4) in place of `quote_price`'s one minor an input unit (`order_price` reads the facility's policy half) |
| **AwaitingDelivery** | the kernel's schedule | the output ready; **the facility's arm delivers it on its next fire** (one `@wo/deliver` a fire, a house act, for every order at a house facility — player orders included), so nothing waits on a hand |
| **Delivered** | `wo_deliver` | the bulk output in the escrow's pool or the part lots in it; for a hull, the `tacsim.hull_kit.<design>` lot (§4.13); the fee captured to the operator's account |
| **Collected** / **Commissioned** | `+works/collect`, `+yard/collect` (or the arm, for a house order) | the goods moved to the requester's hold, or the hull commissioned (§4.13); the escrow closed |
| **Cancelled** | `+yard/cancel`, `+works/cancel`; or the facility, on an unfunded order | from Booked or Supplied: the escrow's contents returned to the requester's hold at the port, the booking fee forfeit; from Running: the elapsed work units' fee forfeit, the unconsumed materials returned (an order past half its work units cannot be cancelled by the requester — the cutoff); from AwaitingDelivery or later: refused, the work is done |

Authority: the facility's policy half gains `customers: Public | OwnerOnly` (the lane's rows are
Public); `@wo/draft`'s gate becomes "staff, the owner's `acts_for`, or any principal with an account
at a Public facility"; the requester is the payer and, unless an organization's officer names the
organization, the beneficiary; the order records all three and the cancel authority is the
requester's or the facility's. House orders are the same contract driven by the port tick's house
arm: the house is requester, payer and beneficiary, its storefront the source, and its capital the
limit (§4.7). The old `@wo` grammar stays for staff and tests and drafts against the same records.

Escrow custody and the collection: an order's escrow is the requester's vault at the port; a hull's
bill may arrive over several voyages (a Dray's 204 t over three 100 t holds); an uncollected delivery
sits in escrow and, after 28 IC days, the facility charges a slot-day a day of storage from the
requester's account until it is collected or the account runs dry, when the facility takes the goods
(the lease's dormancy shape, doc 18). The house arm delivers and collects its own.

**Questions.** Q-39-28, Q-39-29, Q-39-30.

### 4.13 Delivery and handover: from a kit lot to a hull somebody can sail

**What exists.** §3.6: nothing outside a programme row or a lease can be viewed, helmed, boarded,
launched, sortied, stowed, bought out around or carried; `wo_deliver` cannot make a hull; the lay-up
paths need two blocks thirteen designs lack; a new unit's magazines are born full. Draft 0.1's
`DeliveryTarget::Hull` (Q-39-18) assumed the delivery could lay the hull up in the prepare, and said
nothing about what the captain does next.

**What is missing.** Proposed (Q-39-33, revising Q-39-18; **expensive to reverse** for the register
and the provenance record, which every later claim reads):

- **The kit lot.** The hull recipe's output is one one-unit lot of `tacsim.hull_kit.<design>` (a
  `class: kit` good, no mass of its own — the mass is the bill's), minted into the order's escrow by
  `wo_deliver` as any output is, so `PinnedRecipe` has its output and the prepare stays a prepare.
  The kit is the stable order-to-hull identity: its lineage is `workorder.output` with the order
  id (`wo_source`), as the relay foundry's pair is.
- **The commission.** `+yard/collect <order>` (or the house arm) runs tacsim's **commission**: an
  action with a `Ctx` (the foundry's second half, `relay_foundry.rs:375–405`) that consumes the kit
  by its lineage and the part lots in the order's custody, and lays the hull up through a
  **`YardCommission` record** (`tacsim.commissions`, keyed by the order) whose steps are each
  idempotent and recorded — the Thing, the unit (`mush_tactics::module::create_unit` on the design),
  the rooms and the console from the class template, the store and **the hold vault opened at
  commission** (what `+coop/enrol` does at :1669), the airlock, the bays, the placement at the
  yard's slipway (a ship) or in the yard's workshop room (a vehicle), the title to the beneficiary
  under the registry issuer with a fresh mark, the magazines emptied (Q-39-36), the
  `tacsim.hull_provenance` row, the register row, the berth row. The port tick resumes an incomplete
  record from its last recorded step on every fire (the applier's re-apply shape, `hull.rs:1–60`:
  "A re-apply creates nothing"), so a failure between the kit's consumption and the title leaves a
  record that finishes, never a half-hull; the inputs are consumed first and the record is written
  in the same bundle, so nothing is lost between them.
- **The register.** `tacsim.owned_hulls`: one row a commissioned hull — the hull, the design, the
  mark, the home port, the provenance id — read by `hull_view`, `helm_of`, `berth_of`,
  `hull_is_free` (for a Trust-titled row docked at Ceres) and `vehicles_line` (for an Occator-titled
  rig in the shed) **after** the programme and yard rows, and never by `hull_line` or the applier's
  preflight: a built hull is not a programme row, so the pool stays the seed and nothing drifts.
  `helm_of` takes the register's hulls titled to the pilot, lowest id first, as it takes the
  programme's today; a pilot with two hulls chooses with `+ship/helm <hull>` (doc 18 P2's seat).
- **The provenance.** `tacsim.hull_provenance { order, facility, design digest, kit lot, the part
  lots fitted, at_ic }`, the record every later claim, resale and scrap reads (Q-38-12's reason).
- **The berth and the stow.** The commission writes a `BerthRow { workshop, carrier: None, bay_room:
  None, slot: None }` for a vehicle in the yard's workshop room, so `held` locks it to the floor and
  `+bay` quotes it; `+bay/stow <hull>[=<carrier>]` takes a named carrier docked at the port when the
  row names none, resolves a free slot of a bay the actor has the run of, and places the hull there
  (`stow`, `bay_work/mod.rs:1552–1586`, gains the argument; the carrier's own `hold_takes` room
  check applies with the part's catalogue mass).
- **The title view for rigs.** `TvLease::house_view` (`port/yard.rs`) is the precedent: a
  **`title_view`** for a titled unit riding a carrier the pilot has the run of, shaped like a lease
  with no term and no deposit, read by `rig_lease`, `live_on_hull` and `operating_hull`'s seams, so
  `+tv/<n> board`, `launch`, `tow` and `+sortie/launch` work on an owned rig, `rigs_in_bay` lists it
  with a slot, and the buyout's and the refit's "still in the bay" refusal exempts a titled rig (the
  captain's own cargo rides along). `repossess` never sees it (no lease to end).
- **Logistics and interior templates.** A buildable design must carry both blocks: the two rigs and
  the Drays do; IN.9 authors `armour_cab` and `walker_cab` class templates (a cab room with the
  console, a kit locker as the stowage, `dry_mass_t` from the design's bill) and applies them to the
  line-tank family, the heavy, the scout and the bipeds in the content, so `carried_rig` and the
  capacity plan read a mass for every unit a carrier may carry.
- **Ammunition.** A commissioned hull's magazines are empty; rounds are goods (`goods.json:165–219`)
  and the first load is a `+bay/order rearm` from the hold (Q-39-36). Beam charge is a cell and is
  full.

**Questions.** Q-39-33, Q-39-36.

## 5. Owner survey

**Ruled 2026-11-03 (Q-39-1..24).** The owner accepted every default as written ("all q-39s accepted
at their defaults"); nothing was amended, so no question carries a decision line of its own and the
proposed text of each is its ruling. Each keeps its proposed default, alternative and reason for the
record. **Open (Q-39-25..36)**, raised by Draft 0.3 after the two reviews (§8): each carries a
proposed default except Q-39-25, which the owner asked to have raised and not decided; three revise
an accepted default and say which (Q-39-33 revises Q-39-18, Q-39-34 revises Q-39-21, Q-39-35
revises Q-39-3's spelling, Q-39-14's count and Q-39-20's list). §6 is the plan built from all of
them.

### 5.1 The catalogue

**Q-39-1 — The price ladder.**
- **Proposed default:** §4.1's ladder: alloys asked 450 at Kasei, fittings 520 at the Yards, bids at
  Ceres unchanged (520, 560), Stickney bids for carbon ore (60) and strategic ore (700); the part
  rows at the shares in §4.1's table; lane v20 "the chain's numbers". Provisional, every one.
  *(Draft 0.3: the Co-op's fittings bid moves to 600 and the Exchange gains two ore asks under
  Q-39-31 and Q-39-32.)*
- **Alternative:** every price stays and the recipes' ratios absorb it (a 2:1 refine, the works at a
  loss carried by the house's capital).
- **Why:** two rungs cannot cover their inputs at today's asks (§4.1), and lane data is the cheapest
  thing in the tree to move; a house run at a loss is doc 18 risk 9 again.

**Q-39-2 — Part rows in the goods catalogue.**
- **Proposed default:** one `goods.json` row a part commodity (§4.1's thirteen), `class: "part"`,
  ledger and quote unit `each`, mass and volume, `reference_major` the share; the catalogue price is
  the authored reference and the cost model gives the floor a house never sells under; the margin
  between them is the house's recapitalization (doc 18 risk 9's path, attributed and receipted
  through the ledger).
- **Alternative:** cost-plus from the recipe (materials + the works' fee + margin), which prices a
  visual sight near 600 cr and an escort near 25,000.
- **Why:** the shares keep the tank near its 100,000 basis and the 105 mm round at a sixteenth of it;
  cost-plus would reprice every hull and core at once, and the owner's rule is to take a number and
  tune it.

**Q-39-3 — Part identity.** **(expensive to reverse)**
- **Proposed default:** a part is keyed by the hardware row its slot names — `tacsim.part.cannon_105`
  from the `tacsim.weapon.cannon_105` row, `tacsim.part.visual_mk1` from the sensor row, pods from
  their sensor rows, `tacsim.part.power.<class>` from the unit's class — so one works recipe makes a
  part that fits every slot naming that row, and `+bay/fit`'s check matches the slot's row, not the
  unit-qualified definition. `ShippedParts::commodity` (`hardware/bay.rs:46–55`), the fit check and
  `PartProvenance.definition` change together; the bind's component definition id stays. *(Draft
  0.3: the spelling becomes `tacsim.part.<kind>.<row>` and the plant is a row of its own, Q-39-35.)*
- **Alternative:** keep the slot name (`tacsim.part.cannon`), and let a line tank's cannon fit only a
  line tank.
- **Why:** a recipe cannot make "a cannon for a line tank" and "a cannon for a heavy" as two goods
  without a row per unit, and a `tacsim.part.power` that is one commodity from a drone to a Dray is
  a hole a player will find. Expensive because the commodity id is in every part lot, provenance
  record and listing from the first cut on.

### 5.2 The facilities

**Q-39-4 — Which station gets what.**
- **Proposed default:** §4.2's table: `smelt` at Kasei (Stickney), `works` at Stickney Yards and at a
  new Freeport Works org at Ceres (taking the Exchange's alloy bid), `yard` at Stickney Yards only;
  the strategic-ore recipes at the Stickney works only; Tycho Mutual at Ceres for Q-38-18. *(Draft
  0.3: the yard in its own room, `p14_slipway`, not `p11_service`.)*
- **Alternative:** a yard at Ceres too, or the Ceres works dropped.
- **Why:** doc 10 §4's comparative advantage is the reason the trade run exists; a Ceres works gives
  the Co-op's fittings bid a maker and the carbon ore a consumer where it is mined; one yard keeps
  hull construction a voyage away, which is the point of a Belt built on haulage.

**Q-39-5 — Facilities as port rows.**
- **Proposed default:** `refinery`, `works` and `yard` rows on `PortDef` beside `workshop`, each a
  room, an operator, capabilities, slots and the `FacilityCostDef`; the applier opens the room as the
  facility (the part j precedent) and refuses drift; new rooms in the lane's atlas.
- **Alternative:** pack `FacilityDef`s in a campaign pack the deploy template names.
- **Why:** the lane is the live content and the applier already owns facility opening, drift and
  titling; a campaign pack is a second authoring path for the same stations.

**Q-39-6 — Mill rows against recipes.**
- **Proposed default:** §4.3's rule: one-input house conversions are mill rows with `kind: refine`
  (the family rule relaxed for that kind); anything with two inputs or a requester is a recipe.
- **Alternative:** everything by recipe, with the house arm running work orders for the refine too.
- **Why:** the refine is the mill's shape in every respect but the family, its ledger and restock
  exist, and it is already inside the census pin; the recipes carry the cost model where there is a
  requester to pay it.

**Q-39-7 — The NPC house arm.**
- **Proposed default:** houses run work orders on the port tick (the Yards' fittings and parts, the
  Trust's Drays, Occator's rigs), each from the house's own capital and storefront, one draft, submit
  or deliver a fire, bounded by the capital clause; a player's order shares the slots. *(Draft 0.3:
  the arm also delivers and collects every order at a house facility, §4.12.)*
- **Alternative:** only players order; the houses are seeds that the chain refills through sales
  alone.
- **Why:** doc 10 §5's "NPC corporations provide baseline liquidity"; without house production the
  Yards' fittings ask drains to nothing before the first player works a recipe, and the pools never
  restock (Q-39-21).

**Q-39-8 — Conservation's pins and the matter story.**
- **Proposed default:** the pin counts `.mint_origins(` with the same file list; a second pin closes
  the set of `ExternalRef.kind` values a mint may carry; IN.11's matter story asserts the sum over
  the chain; recipe losses are declared by the recipe (inputs' mass − outputs' mass) and summed as a
  line of the story.
- **Alternative:** strict mass equality, with slag as a good.
- **Why:** a loss line is one number in a story; slag is a good nobody wants to haul.

### 5.3 The cost model

**Q-39-9 — The model's shape.**
- **Proposed default:** §4.4's formula on `FacilityCostDef`: daily cost from capital, lifetime,
  upkeep and staff × wage; a slot-day the daily cost over the slots; the fee slot-days × slot-day ×
  (1 + margin); manufacturing cost the fee at margin 0; margin 25 % provisional. The tariff's labour
  rows stay as they are this round, with the model's berth-hour printed as the floor an operator's
  multiplier may not go under; the tariff is re-derived from the model at the tune, not now. *(Draft
  0.3: the fee is by work unit, not by whole slot-day, Q-39-32.)*
- **Alternative:** re-derive the tariff now (bay labour from 100 to ~33 cr an hour).
- **Why:** the tariff's numbers are pinned by GC.7/GC.8's tests and stories, and moving them buys no
  play; the model is what the yard, the works and the crawler need, and it must exist before any of
  them prices a day.

**Q-39-10 — The provisional inputs.**
- **Proposed default:** §4.4's table (the yard 10 M over 30 years with twelve staff ≈ 3,300 a day,
  1,650 a slot-day; the refinery 2,400; a works 1,700; a workshop 1,250; the recovery 2,100; a mill
  650) and a wage of 150 cr an IC day a head. *(Draft 0.3: the mill's row is a 360-a-day crusher
  shed outside the fee model.)*
- **Alternative:** capital only, no staffing or upkeep terms (the yard at the owner's 1,000 a day).
- **Why:** the owner's sentence names staffing; staff is the term that makes a workshop's day cost
  something when its capital is small; every figure is lane data.

### 5.4 The part lot

**Q-39-11 — The facility's stores.**
- **Proposed default:** the operator org's storefront vault at that port, the same vault its stock
  rows and its bids fill; `Replace{lot: Store}` draws the lowest-grade clean lot of the slot's row;
  the quote names the part's price beside the labour; an empty store refuses at the quote with
  `NO_PART`.
- **Alternative:** a separate bay-stores vault per workshop room.
- **Why:** one store means a part the house makes, buys, scraps or seeds and a part a player sells it
  are the same stock, and `+market/list` at the port already shows it.

**Q-39-12 — The seeded part stock.**
- **Proposed default:** part rows in the vendors' `stock` through the stock clerk's receipt: Occator
  at Ceres four of each escort and rig part (`cannon_105`, `mg_coax`, `visual_mk1`, the vehicle and
  light plants); Stickney Yards six of every row; grade 9; retired as the recipes land (IN.7).
- **Alternative:** no seed; the workshop refuses every replace until IN.7.
- **Why:** Q-38-4 asks for the seed, and a refit story (GC.8's) that cannot replace a cannon for
  three cuts is a story nobody runs.

**Q-39-13 — Provenance for parts never stripped.**
- **Proposed default:** `PartProvenance.origin` = `Stripped {…}` | `Manufactured {order, facility}` |
  `Seeded {receipt}`; "traced" means any origin; hot-part rules apply to `Stripped` only.
- **Alternative:** a synthetic parent hull for made and seeded parts.
- **Why:** the record should say what happened; a fake parent would be read by the salvage loop.

### 5.5 The bill, the budget and the ceiling

**Q-39-14 — The design's bill.**
- **Proposed default:** derived at the catalogue build (§4.7): a part row a slot, σ and φ by class,
  the core class, days = max(2, ceil(dry_mass_t / 20)); authored `bill` overrides on a unit def; a
  simulator-only design takes its class's default dry mass; the price is the bill plus the yard's
  manufacturing cost and margin. *(Draft 0.3: nineteen shipped designs, not fourteen; the fractions
  are §4.8's partition, parts and core off the top.)*
- **Alternative:** a fully authored bill on every design.
- **Why:** every shipped and registry design needs a bill on the day the yard opens; the rule gives
  each one and the override fixes the ones it misjudges.

**Q-39-15 — The mass budget.**
- **Proposed default:** §4.8: per-design `structure_kg_per_point` and `fittings_kg_per_point` from σ,
  φ, the dry mass and the points; repair draws both goods; the three constants
  (`SCRAP_KG_PER_STRUCTURAL_POINT`, `FALLBACK_SECTION_SCRAP_KG`,
  `structural_points_per_industrial_parts_t`) retire; a part weighs its catalogue mass in the hold.
- **Alternative:** keep per-point constants and clamp scrap at the dry mass.
- **Why:** a clamp leaves an escort's armour patch drawing 1 t of fittings into an 8 t hull, and a
  Dray's drawing the same tonne; the hull's own figures make both right at once.

**Q-39-16 — The scrap mechanism.**
- **Proposed default:** §4.6: `yield = min(structure × r, reference_price / 5 / alloy_reference)`,
  `r` 0.6; at a workshop the whole mass yield into the operator's stock, the holder credited the
  capped yield at the reference less the scrap job's fee from the cost model; remains scrap mints
  the capped yield into the hold, no fee, the rest lost; the ceiling is on the alloys, parts come
  off at their own value. *(Draft 0.3: in kilograms, minted down to whole tonnes, credited on the
  kilograms, §4.10; whether the parts are exempt is Q-39-25's.)*
- **Alternative:** a tuned `r` alone (shown impossible: 0.24 fits the Dray and no value fits the
  escort), or the ceiling on everything a scrap yields, parts included.
- **Why:** the owner's two bounds bind on different hulls, so both must be rules; minting the whole
  yield into the facility's stock is the one choice that both conserves the matter and pays the
  scrapper; a ceiling on the parts would only make strip-then-scrap the routine, which it already is.

**Q-39-17 — The hull's reference price.**
- **Proposed default:** the design's bill-derived price (§4.4), never authored; a programme hull's
  reference is its programme's `buyout_major`, which the validator checks at most the bill.
- **Alternative:** an authored `reference_major` on every unit def.
- **Why:** Q-38-12 says the bill drives hull prices; an authored number beside a derived one is a
  drift the validator would have to police.

### 5.6 Construction

**Q-39-18 — The delivery.** **(expensive to reverse)** *Revised by Q-39-33.*
- **Proposed default (as ruled):** `DeliveryTarget::Hull { design, title, berth }` and the
  `tacsim.hull_provenance` record (§4.7); ships to the berth gallery as docked hulls, vehicles to the
  yard's workshop room as berthed hulls; parts with `Manufactured` provenance.
- **Alternative:** deliver a "hull kit" lot and a separate `+yard/launch` that lays the hull up from
  it.
- **Why:** doc 08 §10.2's step 7 ("transfers the output into faction title and physical custody") is
  one act, and a kit lot is a hull that can be lost in a hold. Expensive as Q-38-12 says: every later
  claim reads the provenance. *(Draft 0.3: the delivery cannot lay a hull up — it is a prepare with
  no world, and a recipe must have an output — so the alternative's shape is the one that can be
  built; Q-39-33 proposes it with the record that makes the kit never a hull lost in a hold.)*

**Q-39-19 — The player verb, and who may order.**
- **Proposed default:** `+yard`, `+yard/quote`, `+yard/order`, `+yard/cancel` at a port with a yard,
  fronting `@wo` as `+bay` fronts the bay; any principal with an account and the money may order any
  registered design the yard's class list takes, an organization through an officer with the
  capability; `@wo` stays staff. *(Draft 0.3: `+yard/deliver` and `+yard/collect` beside them, and
  `+works` at a works; the authority the engine needs for "any principal" is Q-39-30's.)*
- **Alternative:** expose `@wo` to players.
- **Why:** `@wo/draft` names endpoints by number and a recipe by pack and version; `+bay` is the shape
  players already have.

**Q-39-20 — What the yard takes.**
- **Proposed default:** the Stickney yard builds class hauler, vehicle, walker and drone; capital
  hulls (the corvette, the Halcyon) and the relay station are out of scope until a space-side yard
  exists (doc 35's track). *(Draft 0.3: armour too, which is every tank; the drone, the carrier and
  the target out; a design needs `logistics` and `interior`, Q-39-35.)*
- **Alternative:** every registered design.
- **Why:** a capital hull is delivered into the space track's berth and bubble, which this chain does
  not touch.

**Q-39-21 — The pools.** *Revised by Q-39-34.*
- **Proposed default (as ruled):** §4.7's floors (the Trust two Drays, Occator two rigs and one
  escort) with house orders from the house's capital; `NO_HULL` names the order and its due day.
- **Alternative:** Q-18-70's shrink stays and the pools are the seed.
- **Why:** a pool that only shrinks ends the programme in eight captains; production restock is what
  doc 18 P5 owed. *(Draft 0.3: the Trust cannot fund the order and the yard is a voyage away;
  Q-39-34 adds the funding and the ferry.)*

### 5.7 The consumers

**Q-39-22 — Strategic ore and the core track.**
- **Proposed default:** strategic ore feeds the sight and pod recipes now (§4.3); the core track's
  monthly dies stay a receipted seed, with a die recipe (strategic ore + instrument boards) as a
  later cut of doc 17's track.
- **Alternative:** the dies take strategic ore now.
- **Why:** Q-38-13 names "sensors, ECM and cores"; the die pipeline is its own audited track with its
  own tests and the sights give strategic ore a consumer this round.

**Q-39-23 — Carbon ore.**
- **Proposed default:** the works' fittings recipe takes carbon ore directly (5 alloys + 1 carbon ore
  → 5 fittings), hauled from Ceres to Stickney or consumed at the Ceres works; no pressing step.
- **Alternative:** carbon ore → `carbon_feedstock` at a mill row first, four transformations.
- **Why:** doc 10 4A's "≈ three transformations deep", and Titan's billet is another trade's good.

### 5.8 The salvage organization's fee

**Q-39-24 — Tycho Mutual's row.** (The acceptance rule is Q-38-29's and is not re-asked.)
- **Proposed default:** Tycho Mutual Assurance & Salvage (lane :2600) is Q-38-18's organization at
  Ceres; its recovery row in the cost model (§4.4: 4 M over 15 years, six staff, 2,100 a day, one
  slot); a retrieval fee = the job's days × 2,100 × 1.25 + the passage's propellant at the ask,
  which doc 41 quotes on the contract.
- **Alternative:** a new org at Stickney.
- **Why:** the org exists with the word in its name and 50,000 of capital to be raised like Occator's;
  Ceres is where the rigs and the claims are.

### 5.9 Raised by the reviews: the ceiling's scope (not decided here)

**Q-39-25 — What the scrap ceiling bounds.** *Raised at the owner's request; no default is taken.*
- **The question:** doc 38 Q-38-5 says "the scrap of a whole hull is worth on the order of a fifth of
  its buyout at most". Draft 0.1's mechanism (§4.6) bounds the **structural proceeds** — the alloys —
  and lets the parts come off as lots at their own value, so a Dray scrapped whole returns its
  30,000 cr plant beside the 18,000 cr ceiling, and the 40 t of alloys sell at the Ceres bid for
  20,800 rather than the 18,000 the reference prices them at. Three readings:
  1. **structural proceeds**: the ceiling bounds the alloys at the reference; parts are the holder's
     at their grade (§4.6 as written; a holder strips first and the ceiling never sees a part);
  2. **total recoverable value**: the ceiling bounds everything a whole-hull scrap yields, parts
     included, so a scrapped Dray's plant is the facility's past 18,000 (strip remains the way to
     keep a part, and scrap is only for shells);
  3. **net payment**: the ceiling is the money the holder receives after the facility's fee, parts
     and alloys together, and the valuation basis is the port's bid, not the reference.
- **What each costs:** (1) is built as §4.6; (2) adds a value sum over the part lots at the scrap
  and a "parts to the facility" branch; (3) prices scrap by the local bid, which moves with the
  market and makes the matter story's sum a market read. The reviewers lean to (2) with (1)'s
  exemption made explicit; the survey's author leans to (1).

### 5.10 Raised by the reviews: accounting, lifecycle, the order

**Q-39-26 — The accounting unit and the rounding policy.**
- **Proposed default:** §4.10: kilograms for every computation, tonnes for the ledger and the market;
  a bill rounds each bulk line up with the remainder declared as the build's loss; repairs draw from
  the hull's opened bale (a tonne of each good debited up front, kilograms consumed from it, the rest
  paid out at scrap); scrap mints down to whole tonnes in kind and credits the exact kilograms; a
  part never splits.
- **Alternative:** a kilogram ledger for every bulk good (exact balances; the lane's stock rows, the
  mills' batches, the capacity plan and every tonne-asserting test move).
- **Why:** the bale and the two rounding directions give every act a conserving integer rule without
  a migration, and the kilogram ledger can come at the tune if the bale proves awkward in play.

**Q-39-27 — The component lifecycle.**
- **Proposed default:** §4.11: a part is an identified one-unit lot from seed to scrap; `class:
  part` goods are refused by dissolve, split and merge; seeds mint one lot a unit with `Seeded`
  rows; recipes with `output_lots: each` mint one lot a unit and tacsim's observer writes
  `Manufactured` rows from the `wo_applied` fact; the market lists and transfers the lot itself;
  orders take custody through `BayPartCustody`; a fitted part's row records the hull and the hull's
  provenance lists the lot; `Stripped` parts keep their claim and condition into a build; the refit's
  removed part gets a `Stripped` row from the same observer.
- **Alternative:** synthesize a provenance row whenever a part is drawn from a pool or a tranche
  (the engine's pooling kept; identity invented at the draw).
- **Why:** provenance that is invented at the draw is not provenance; the custody and the lot
  listing exist and are the smaller change; a part that can be dissolved into a pool is a part that
  can be laundered.

**Q-39-28 — Escrow, the reservation deadline and auto-delivery.**
- **Proposed default:** §4.12: a per-order escrow endpoint at the facility, filled over any number
  of deliveries or purchases, the requester's custody throughout; the submit stamps the material
  deadline at the order's duration plus `RESERVATION_TTL_SECS`; the E4 hold is `fee(order)` from the
  `FacilityCostDef` in place of one minor an input unit; the facility's arm delivers every
  `AwaitingDelivery` order on its next fire; an uncollected delivery pays storage after 28 IC days.
- **Alternative:** the holder renews reservations by hand (`@wo/restamp`'s shape) and collects by
  hand.
- **Why:** a 20-day plant and a 14-day Dray cannot live on a 7-day stamp, and a 204 t bill cannot
  arrive in one hold; a delivery nobody triggers is an order that never finishes.

**Q-39-29 — Cancellation.**
- **Proposed default:** §4.12: a booking fee of one slot-day, forfeit on any cancel; a cancel while
  Running forfeits the elapsed work units' fee and returns the unconsumed materials; no requester
  cancel past half the work units; none from `AwaitingDelivery` or later.
- **Alternative:** no cancel after Running.
- **Why:** a free cancel from `AwaitingDelivery` lets a funded player hold both yard slots for
  nothing (review 2 #9); a cutoff at half protects a competing order without trapping a requester
  whose plans changed on day one.

**Q-39-30 — Public ordering authority and the works verb.**
- **Proposed default:** §4.12: `customers: Public | OwnerOnly` on the facility's policy half, the
  lane's rows Public; `@wo/draft` admits any principal with an account at a Public facility; the
  order records requester, payer and beneficiary; `+works/quote|order|deliver|collect|cancel` at a
  works beside `+yard`'s; `@wo` stays staff and tests.
- **Alternative:** every player order goes through a house officer's `acts_for` grant.
- **Why:** the engine's gate was written for a facility's own crew; a port's works is a shop, and a
  shop that only its owner may use has no customers.

### 5.11 Raised by the reviews: supply, coverage, delivery, pools

**Q-39-31 — Supply paths between organizations.**
- **Proposed default:** §4.2's table: the Exchange asks carbon ore at 60 and strategic ore at 700
  from the pools its bids fill; the carbon bid moves to Freeport Works with the alloy bid; Stickney
  Yards bids carbon at 60 and strategic at 700; Kasei's `metals` bid gains `refresh { per_ic_day: 40,
  cap: 4000 }`; the Stickney works buys Kasei's alloys by a receipted house purchase on the port
  tick; a player's order sources from the player's own hold.
- **Alternative:** every input a bid only (no asks), with the works' inputs hauled by players alone.
- **Why:** a recipe whose input nobody sells is a recipe nobody runs; the house purchase is the
  dispatcher's transfer with a payment post, and the asks give the trade run two more cargoes.

**Q-39-32 — Cost coverage and the billing grain.**
- **Proposed default:** §4.4: the fee is by work unit (`slot_hour`), never by whole slot-day; the
  mills are outside the fee model (a house process priced by its ratio, with a 360-a-day crusher
  shed shown for honesty); the Co-op's fittings bid moves to 600 so the Ceres works covers; Occator's
  water is flagged for the tune, not moved.
- **Alternative:** whole slot-day billing with the daily costs cut to fit it.
- **Why:** a six-work-unit batch billed a whole day is 170 cr a tonne on a 520 good; the engine's
  grain is the hour; the coverage table is what the owner's "derive the daily cost" asked for and
  Draft 0.1 did not show.

**Q-39-33 — Delivery and handover.** **(expensive to reverse)** *Revises Q-39-18.*
- **Proposed default:** §4.13: the hull recipe outputs a `tacsim.hull_kit.<design>` lot into the
  escrow; `+yard/collect` runs the commission — a tacsim action that consumes the kit by its
  lineage and the part lots in custody and lays the hull up through a resumable `YardCommission`
  record (Thing, unit, rooms, vault, airlock, bays, placement, title, empty magazines, provenance,
  register row, berth row); the `tacsim.owned_hulls` register read by the view, helm, berth and
  free-hull paths after the programme rows and never by the applier; `+bay/stow <hull>=<carrier>`;
  a `title_view` for owned rigs so board, launch, tow and sortie work and the buyout and refit
  refusals exempt them; `armour_cab` and `walker_cab` templates for the tanks and bipeds.
- **Alternative:** Q-39-18 as ruled (`DeliveryTarget::Hull`), with mush-industry given a world at
  delivery (a change to the kernel's prepare contract) and the simulator units given templates by
  hand.
- **Why:** the prepare has no world and a recipe must have an output, so the ruled shape cannot be
  built without changing the kernel's contract; the foundry already does the two-phase shape, and
  the register is the smallest change that makes a built hull viewable, sailable and stowable.

**Q-39-34 — The pools' funding and the ferry.** *Revises Q-39-21.*
- **Proposed default:** §4.7: a house orders when its account covers the bill; the Trust gets a lane
  raise of 250,000 (`raises`) and its buyouts fund the rest; house orders deliver at Stickney and a
  house ferry under a DARK house helm sails them to Ceres, rigs in the ferry Dray-P's bay; built
  hulls are register rows, never programme rows; `NO_HULL` distinguishes "cannot yet fund one" from
  "on order, due at Ceres in n days".
- **Alternative:** house orders deliver straight into the pool at Ceres (a Phobos yard teleporting
  hulls).
- **Why:** the Trust is at 50,000 after eight stakes; the yard is a voyage away by Q-39-4's own
  reason; a ninth programme row crashes the applier.

**Q-39-35 — Part spelling, the plant row and the class table.** *Revises Q-39-3's spelling,
Q-39-14's count and Q-39-20's list.*
- **Proposed default:** parts are spelled `tacsim.part.<kind>.<row>` (`weapon.cannon_105`,
  `sensor.visual_mk1`, `plant.vehicle_mk1`); a `plant` definition kind in `content/plants/` with
  four rows (light, vehicle, walker, hauler) and a `plant` field on every `power` component, refused
  at the catalogue build when missing; `structure_share` and the yard's class list include `armour`
  (every tank); the drone, the carrier, the target, the relay station and the capital hulls are not
  built; a buildable design must carry `logistics` and `interior`.
- **Alternative:** derive the plant tier from the dry mass alone (no content change).
- **Why:** `UnitClass` has no `Light` and puts the prospector and the escort in one class; a plant
  row is what mounts and sensors already have; a yard that cannot build a tank builds nothing
  anyone fights with.

**Q-39-36 — Ammunition at delivery.**
- **Proposed default:** a commissioned hull's magazines are empty and its first load is a
  `+bay/order rearm` from the hold at the munitions' prices; beam charge is full.
- **Alternative:** the bill includes a first load at the catalogue price.
- **Why:** a magazine born full (`init: rounds`) is 6,000 cr of 105 mm rounds minted from nothing
  on every escort; the rearm path exists and the matter story needs the rounds to come from a lot.

## 6. The plan: cuts IN.0–IN.11

The rulings of 2026-11-03 on Q-39-1..24 and the ones to come on Q-39-25..36 make this the
implementation order. Each cut lands as one PR over E7's engine, built from the rulings its row
names and tested through the production command paths, with its story at both ground sizes where
the ground is in it (Q-38-29); each cut's note goes to `39-cut-notes.md` as the GC and SF trains
did. The order follows review 2's rule: identity, units and escrow before their consumers, and the
first player production story as early as the chain allows (IN.5). **Dependencies:** IN.1 ∥ IN.2 ∥
IN.4 may start now; IN.3 after IN.2 and IN.4; IN.5 after IN.1 and IN.3; IN.6 after IN.1, IN.2 and
IN.4; IN.7 after IN.5 and IN.6; IN.8 after IN.1 and IN.4 (and Q-39-25's ruling); IN.9 after IN.3,
IN.6 and IN.8; IN.10 after IN.9; IN.11 last. Two cuts carry the expensive-to-reverse marker (IN.1's
part identity, Q-39-3/35; IN.9's register and provenance, Q-39-33) and are built exactly as ruled.
Doc 41 may start once IN.1, IN.2, IN.4 and IN.6 have landed (the lot shapes, the lifecycle, the
fees and the stores it needs).

| Cut | Rulings | What | Acceptance (the story) |
|---|---|---|---|
| **IN.0** | — | this document, its README row, Draft 0.2 with the rulings, Draft 0.3 with the triage (**done**, PR #347) | — |
| **IN.1** accounting and identity | Q-39-1, Q-39-2, Q-39-3, Q-39-8, Q-39-26, Q-39-35 | goods v5: the thirteen part rows spelled `tacsim.part.<kind>.<row>`, `class: part`, kilogram masses; the plant rows and the `plant` field; lane v20's ladder; `ShippedParts::commodity` and the fit check on the row; the kilogram computation with the tonne ledger, the bale on `BayHull`, the up/down rounding per act; `hold_takes` counts parts; the census pin on `.mint_origins(` and the receipt-kind pin | `+bay/strip` a cannon off an escort at Stickney: `+market/list` shows the lot with 2.4 t; the hold's load line moves; a heavy's `+bay/fit` takes it; a one-point armour patch opens a bale and draws a tonne of each good once, the second patch draws nothing; the pins pass |
| **IN.2** the component lifecycle | Q-39-11, Q-39-13, Q-39-27 | `class: part` refused by dissolve, split and merge; seeds as one-unit lots with `Seeded` rows, never dissolved, listed as lot asks; `PartProvenance.origin`; the lot-transfer sale; `BayPartCustody` reused by `+bay/order replace` and `+bay/fit`; the observer that writes `Manufactured` and the refit's `Stripped` rows from the industry facts | sell → buy → fit: a part stripped at Stickney is sold on the yard's board, bought by a second captain, fitted to their scout, and its row reads the same lot id with both hulls; `@inv/split` on it refuses; a hot part refuses at the fit |
| **IN.3** the order contract | Q-39-7, Q-39-28, Q-39-29, Q-39-30 | the escrow endpoint, `+works` and `+yard`'s quote/order/deliver/collect/cancel over `@wo`, `customers: Public`, the requester/payer/beneficiary triple, the deadline at duration + TTL, the hold at `fee(order)`, the booking fee and the cutoff, the house arm's submit/deliver/collect, storage after 28 days | a full 20-day order (a hauler plant) at the Stickney works runs to collection with the world clock stepped past day 7; a late cancel at day 11 is refused and one at day 3 returns the materials less the elapsed fee; two orders for one slot queue; a stranger with an account orders at Stickney and the owner's officer cannot cancel it |
| **IN.4** the cost model | Q-39-9, Q-39-10, Q-39-24, Q-39-32 | `FacilityCostDef` on the port rows, the slot-hour, `order_price` from the policy half, `+bay` quotes printing the floor, the multiplier clamped at it, the Tycho Mutual row; the mills withdrawn from the fee model | `+bay <hull>` at Ceres and Stickney prints the quote and the floor; an operator multiplier under the floor is refused by the validator; `+works/quote fittings` prints 265 cr |
| **IN.5** the first production story: the refine and the works | Q-39-4, Q-39-5, Q-39-6, Q-39-23, Q-39-31 | the `refinery` row and `MillDef.kind: refine` at Kasei (5:4, 20 t, 40 t a day, the seed as opening stock, the bid's `refresh`); the `works` rows at Stickney and Ceres (Freeport Works, the new org, with the alloy and carbon bids); `content/industry/recipes-v1.json` with `fittings` and `output_lots`, loaded by the deploy template; the Exchange's ore asks; the house purchase of Kasei's alloys | **the bot, over SSH, at 256 and 512:** a captain mines carbon ore on Kerwan (Ahuna / Urvara for the ground half), sells concentrate to the Exchange and buys its carbon ask, hauls both to Stickney, sells the concentrate to Kasei (refined on the next fires, Kasei's ask restocking by the milled units), orders fittings at the works with their own alloys and carbon through `+works`, collects five tonnes two fires later and sells them to the Co-op at Ceres for 600; the matter pin and the mill's drift check hold throughout |
| **IN.6** the part lot at the workshop | Q-39-12 | `Replace{lot: Store}` from the operator's storefront at the catalogue price, `NO_PART`, the seeded part rows | GC.8's refit run pays 480 of labour and 28,000 for the cannon; the seventh replace at Ceres refuses `NO_PART`; the lot's row reads `Seeded`; the store's count after the order is six less one |
| **IN.7** the part recipes | Q-39-22, Q-39-31 | the mount, sight, pod and plant recipes at the works with `output_lots: each`; the Stickney strategic-ore bid; instrument boards from Luna; the seeds retire | 2 t of strategic ore, 1 t of boards and 3 t of fittings become ten `sensor.visual_mk1` lots at Stickney with `Manufactured` rows; one is fitted to a scout by `+bay/fit`; a disputed part (a `Stripped` row under a live claim) is refused at a build's custody |
| **IN.8** the mass budget and scrap | Q-39-15, Q-39-16, Q-39-17, Q-39-25 | the partition at the catalogue build, per-design `structure_kg_per_point` and `fittings_kg_per_point` counting structural cells only; repair draws both from the bale; the three constants retire; scrap as §4.6 and §4.10 at a workshop (the operator's stock in tonnes, the credit on the kilograms, the fee) and on a site (the capped yield, the room check), parts under Q-39-25's ruling | a whole escort scrapped at Stickney puts 1 t in the Yards' pool and credits 644 cr less the fee with its four parts in the hold; a Dray's remains on Urvara (512) and an escort's on Ahuna (256) yield 40 t and 1 t into the hold, the full hold refusing; a Dray after SF.R15 counts no heat points |
| **IN.9** the yard and the handover | Q-39-14, Q-39-18, Q-39-19, Q-39-20, Q-39-33, Q-39-36 | the `yard` row at `p14_slipway`; the hull recipe and the kit lot; the `YardCommission` record and the commission action; `tacsim.owned_hulls` and `tacsim.hull_provenance`; the register read by view, helm, berth and free-hull paths; `+bay/stow <hull>=<carrier>`; the rig `title_view`; `armour_cab` and `walker_cab` templates on the tanks and bipeds; empty magazines; the bill from the design | a captain books an escort at Stickney, delivers its bill over two trips, waits two IC days with the clock stepped, collects it: it stands in the slipway's workshop room titled to them with a fresh mark, `+bay` quotes it clean with empty magazines, `+bay/order rearm` loads it from the hold, `+bay/stow <hull>=<their Dray-P>` puts it in the bay, `+tv/<n> board` and `launch` work on Ahuna, and `+coop/buyout` on the Dray-P is not refused for it; a commission failed by staff hand between the kit's consumption and the title resumes on the next fire and finishes; the bot types it |
| **IN.10** the pools | Q-39-7, Q-39-21, Q-39-34 | the Trust's raise; the floors and the funded house orders; the house ferry under a DARK house helm; `NO_HULL`'s two texts | eight charters leave no free Dray and the ninth captain hears "cannot yet fund one"; two buyouts later the Trust orders; fourteen IC days plus the passage later a ninth Dray docks at Ceres under the Trust's title as a register row, the desk enrols the captain, and the applier's preflight still passes |
| **IN.11** the matter story and the adversarial set | Q-39-8, Q-38-29 | one story at 256 and 512: mine → sell → mill → refine → works → part → fit, and ore → alloys → hull, with Σ mined = Σ (pools + lots + bills + losses) asserted; the adversarial stories not already in a cut (two orders for one bale, the failure between consumption and title under a restart, a competing reservation at the deadline); the bot's `+yard`, `+works`, `+market/sell` and `+bay/order replace` vocabulary | the story passes at both sizes with the bot typing the player's half; the sum holds to the kilogram with the losses declared |

## 7. What the first cuts need

Content and schema the cuts author as they land, listed once so no two cuts invent them twice:
lane v20's rows (the ladder, the part stock, the three facility rows and their `FacilityCostDef`s,
the Freeport Works org, the Trust's raise, the Exchange's two ore asks, the Stickney bids, Kasei's
`refresh`; IN.1, IN.4, IN.5, IN.10), the goods v5 rows and the plant rows (IN.1),
`content/industry/recipes-v1.json` with its `output_lots` field and its path in the deploy template
(IN.5, IN.7, IN.9), the new rooms in Stickney's and Ceres' atlas entries (`p12_refinery`,
`p13_works`, `p14_slipway`, `c15_works`; IN.5, IN.9), the `armour_cab` and `walker_cab` templates
(IN.9), the `tacsim.owned_hulls`, `tacsim.hull_provenance` and `tacsim.commissions` schemas (IN.9),
the `customers` and `FacilityCostDef` fields on the facility's policy half and the `plant` field on
the power role (IN.1, IN.3, IN.4), the test list the pins become (`shard_9/mill.rs:2508`'s successor
and the receipt-kind pin, IN.1); and, for doc 41, the lot shapes and the lifecycle (§4.5, §4.11), the
facility fees (§4.4) and Tycho Mutual's row (Q-39-24) as the inputs doc 38 §8 promised it, available
once IN.1, IN.2, IN.4 and IN.6 have landed.

## 8. The two outside reviews of Draft 0.1, triaged

Two outside reviews of Draft 0.1 (PR #347 at `4e1be226`) arrived on 2026-11-03. The owner's framing:
"they're not as familiar with the code base so some may just be misunderstandings", so every tree
claim below was re-verified against `origin/main` at `c83c2eff` (SF.R15 merged) before it changed
anything. The form is doc 36a §3's: CONFIRMED means the claim was traced in code or arithmetic;
PARTLY means the substance holds and a detail does not; REFUTED means the tree says otherwise;
MISREADING means the claim rests on a reading of Draft 0.1 or of the code that the cited line
does not support. The last column says what Draft 0.3 did with it.

### 8.1 Review 1

| # | Claim | Verdict | What decides it | Draft 0.3 |
|---|---|---|---|---|
| G1–G6 | the six "got right" items: chain first, the slot-name bug, refine rows for one-input conversions, the dual-bound ceiling, the ladder and the carbon/strategic consumers, the census pins | stand | G2's fit check is `bay_work/mod.rs:712` (the doc) and `:740–752` (the hot and fitted refusals), not `:747` alone; G3's family rule is `check_ports` at `port/def.rs:2450–2463` as claimed | kept |
| 2.1 | `structural_alloys` and `industrial_parts` are whole-tonne ledger goods, so 4.8 t bills, 160 kg repairs and 2.9 t scrap cannot be represented; a sub-tonne remains scrap refuses | **CONFIRMED** | `goods.json:39–40, 52–53` (`ledger_unit` and `quote_unit` `"t"`); `RecipeLine.units: u64` (`mush-industry/src/workorder.rs:166–169`); the bay debits `row.units` whole (`bay_work/act.rs:712–716`) and the quote rounds fittings up (`jobs.rs:205–208`, `div_ceil`); `tow.rs:1772–1774` refuses `alloys_t == 0` as `scrap_changed` | §4.10 the accounting unit and rounding policy; Q-39-26; IN.1 |
| 2.2.1 | work orders reserve and consume pools, never lots; a delivery mints a lot nothing can draw until it is dissolved | **CONFIRMED** | `pool.rs:1050–1066` and `:1099–1111` (`NoPool`); `provider.rs:3458–3461` reserve, `:3728–3752` consume; the output a lot at grade 0, `:3778–3796`; the stock clerk dissolves its own seed lots because asks draw pools, `stock.rs:639–670` | §4.11 the component lifecycle; the bulk recipes' outputs go to pools, the part recipes' to one-unit lots; Q-39-27; IN.2 |
| 2.2.2 | a ten-unit part output is one lot; fitting one consumes all ten; a split child is untraced; `bay_refit` refuses `quantity != 1` | **CONFIRMED** | one lot per output line with `units × quantity` (`provider.rs:3780–3796`); `consume_product` removes the whole lot (`pool.rs:1525–1539`); `bay_part.rs:333`; `split` writes `Lineage::Split { parent }` (`pool.rs:1454`) and `tacsim.bay_parts` is keyed by lot id (`act.rs:719–726`) | §4.11: parts are never batched, split or pooled; Q-39-27 |
| 2.2.3 | the 7-IC-day reservation TTL expires a 14- or 20-day order's materials before delivery | **PARTLY** | the deadline is `now + RESERVATION_TTL_SECS` at submit (`record.rs:34–36`, `pool.rs:198–204`, `provider.rs:3458`) and `wo_advance` restamps nothing (`:3590–3618`) — CONFIRMED; but `checked_consume` requires `Held` only (`mush-core/src/resource.rs:456–462`) and never compares `expires` with `now`: the order fails only once a sweep has expired the reservation (`pool.rs:1354`, `mush-industry/src/money.rs:623`), so the failure is real and its timing is the sweep's | §4.12 escrow: the deadline is the order's duration plus the TTL; Q-39-28; IN.3 |
| 2.2.4 | the E4 hold is one minor unit an input unit, and submission requires exactly that hold | **CONFIRMED** | `quote_price` sums input units (`workorder.rs:1210–1214`); `order_price` (`money.rs:210–214`); `place` holds `order_price` (`:333–345`) | §4.12: the hold is priced from `FacilityCostDef` through `order_price`; Q-39-28 |
| 2.3 | a seeded part row mints one lot of `qty` with no provenance, the clerk dissolves it, a sale makes a new tranche lot with no provenance, and a store debit outside the market pages drift | **CONFIRMED** | `stock.rs:420` (one lot of `qty`); `:648–670` (`InventoryVerb::Dissolve`); `pool.rs:1215–1224` (`Lineage::Tranche`, a new `buyer_lot`); `stock.rs:505–510`, `:728–737` (`drifts`) | §4.11: part rows are never pooled; the clerk mints one-unit lots with `Seeded` rows and lists them as lot asks (`Representation::Lot`, `mush-market/src/record.rs:129`); Q-39-27 |
| 2.4 (1) | 19 shipped units, not 14 | **CONFIRMED** | `content/units/` has 19 files; Q-39-14 miscounted | Q-39-14's text corrected in §4.7 |
| 2.4 (2) | `UnitClass` has no `Light`, every tank is `Armour`, and `tv_prospector` and `tv_escort` are both `Vehicle`, so `tacsim.part.power.<class>` cannot be derived from the class | **CONFIRMED** | `definitions/unit.rs:749–772` (Armour, Walker, Drone, Hauler, Vehicle, Station, Capital); `class_of` defaults to Armour or Walker by locomotion (`:971–979`); nine units author a class (`armour`, `capital` ×2, `hauler` ×2, `drone`, `station`, `vehicle` ×2) and ten default | §4.1 the plant row on the power component; §4.8 the class table with `armour`; Q-39-35 |
| 2.4 (3) | `power.walker` and `relay` have no recipe; `pod_eccm`'s loss is 5.8 t | **CONFIRMED** on the recipes; **PARTLY** on the loss (the row covered both pods: 5.4 t for ECM at 160 kg, 5.8 t for ECCM at 120 kg) | §4.3's table | `plant_walker` added, the pods' rows split; the relay stays out with the capital hulls (Q-39-20) |
| 2.4 (4) | after SF.R15 the Dray has 3,950 points and the escort 3,475 | **REFUTED** | PR #339 (`c83c2eff`) changed `dray.json`, `dray_p.json` and `corvette.json` by `heat_max` 1000 and `heat_dissipation_per_s` 30 only (and the corvette's drive); at `c83c2eff` the Dray sums 570 armour + 1,250 internals = 1,820 and the escort 2,320 + 670 = 2,990 | the figures stand |
| 2.4 (4b) | `scrap_hull` counts the heat cells that the bay quote filters out | **CONFIRMED** | `workshop.rs:745–752` sums every `CellAddr::Section` cell above its minimum; the heat-sink cell is born at its maximum (`definitions/unit.rs:1678–1683`, `init: dissipation`); `jobs.rs:239–246` filters `HEAT_CELL` and `HEAT_SINK_CELL`; after SF.R15 the Dray carries them (90 phantom points over three sections) | §4.8: the per-design figures count structural cells only; IN.8 retires the sum |
| 2.5 | `DeliveryTarget::Hull` cannot run inside `wo_deliver`: a recipe needs an output, the prepare has no world, and the lay-up paths need logistics and an interior the simulator units lack | **CONFIRMED** | `PinnedRecipe::new` refuses empty outputs (`workorder.rs:349–353`); `wo_deliver` "is a prepare: no `&World`, no `Ctx`, no `EconomyStore`" (`provider.rs:3637–3640`); `hull::template_of` (`hull.rs:317–323`) and `vehicle::template_of` (`vehicle.rs:181–186`) require both blocks; thirteen simulator units carry neither; the foundry precedent consumes the order's output lot by its `workorder.output` lineage in a tacsim action (`relay_foundry.rs:375–405`, `provider.rs:4086–4093`) | §4.13 the kit lot and the commissioning record; Q-39-33 revises Q-39-18; IN.9 |
| 2.6 | Stickney builds and Ceres enrols, so IN.9's ninth Dray teleports; a ninth programme row crashes the applier; a changed workshop capability set refuses the apply | **CONFIRMED** | `hull_is_free` needs the hull docked on the programme port's domain (`coop.rs:736–739`); `vehicles_line` counts the shed (`vehicle.rs:927–934`); `hull::preflight` refuses `row.n >= pool` (`hull.rs:465–471`), `vehicle::preflight` the same (`vehicle.rs:306–312`); `workshop::preflight` refuses drift in berths or capabilities (`workshop.rs:52–60`); the Trust's 250,000 (lane `:2597`) posts each 25,000 stake (`coop.rs:1629–1663`) | §4.7 and §4.13: built hulls live in an owned-hull register, never in a programme row; the house ferry; Q-39-34 revises Q-39-21; the yard gets its own room |
| 3.1 | a yard-built vehicle in `p11_service` cannot be stowed, is not held, and has no carrier to draw from | **CONFIRMED** | `stow` needs `record.berth` (`bay_work/mod.rs:1560`); `held` reads an open order, a berth or a scrap only (`:442–453`); `source_carrier` needs the berth row's carrier (`:611–629`) | §4.13: the commission writes a yard berth row; `+bay/stow <hull>=<carrier>`; Q-39-33 |
| 3.2 | a titled, non-leased unit cannot be bought out around, boarded, launched or sortied, bricks a carrier without logistics, and a built ship has no hold vault and no helm | **CONFIRMED**, **PARTLY** on the hulls line | `rigs_in_bay` gives a non-lease unit slot 0 (`vehicle.rs:1058`) and `bay_slot_word` turns it into the buyout's and refit's refusals (`buyout.rs:407–409`, `refit.rs:180–182`); boarding reads `tv_leases_of` (`boarding.rs:188–198`), the launch's `lease_gate` takes a `TvLease` (`plans.rs:142–149`), the sortie host needs `tv_lease_on` (`sortie/surface.rs:89–97`); `carried_rig` needs logistics (`vehicle.rs:1189–1206`); the hold vault opens at enrol (`coop.rs:1669`); `helm_of` is the charter, else the programme map's hull titled to `Object(pilot)`, `.min()` (`coop.rs:597–622`); `hull_view` searches programme and yard rows (`ship.rs:318–329`). `hull_line` reads a player-titled row as graduated, and only an org-titled non-Trust row as Missing (`hull.rs:1195–1205`) | §4.13 the owned-hull register and the title view for rigs; Q-39-33 |
| 3.3 | nothing auto-delivers `AwaitingDelivery`; the mill never refreshes a bid; Kasei's `metals` bid needs `refresh` | **CONFIRMED** | `wo_advance` stops at `AwaitingDelivery` (`provider.rs:3596–3600`); `mill_step` touches no bid (`mill.rs:935–1000`); the drip serves `refresh` rows only (`stock.rs:906`); Kasei's bid has none (lane `:2713`) | §4.12 the house arm delivers; §4.2 Kasei's bid refreshes from the refine row; Q-39-28, Q-39-31 |
| 3.4 | `ShippedParts::commodity` reaches only `bay_refit`; `bay_refit::moves` diffs slot names; its removed part is minted at grade 0 with no provenance | **PARTLY** | `part_commodity` is called by `bay_work` (`strip.rs:198`, `:290`), so the first clause is REFUTED; `moves` diffs `ComponentId`s (`bay_refit/config.rs:235–252`) CONFIRMED; the removed lot takes `c.parts.grade(removed)` (`bay_refit/provider.rs:538`; the trailing `0` is lineage depth), so "grade 0" is REFUTED and "no `tacsim.bay_parts` row" CONFIRMED (`:545–552`, an `industry/bay_refit` origin; doc 36 GC.8b already says so) | §4.11 states the one identity both paths share; IN.2 |

### 8.2 Review 2

| # | Claim | Verdict | What decides it | Draft 0.3 |
|---|---|---|---|---|
| 1 | the escort's bill (4.8 + 1.6 + 4.33 t of parts) is 10.73 t for an 8 t hull; the Dray's covers 210 of 240 t | **CONFIRMED** | arithmetic on §4.1's masses: 2,400 + 40 + 90 + 1,800 kg of parts beside σ 0.6 and φ 0.2 of the dry mass; 132 + 48 + 30 t | §4.8 a complete partition: parts and core off the top, structure and fittings from the remainder, an assembly loss; Q-39-26 |
| 2 | the ledger cannot represent the quantities | **CONFIRMED** | as 8.1 row 2.1 | §4.10; Q-39-26 |
| 3 | components need an identity-preserving lifecycle; stamping every input `Manufactured` loses damage and claims | **CONFIRMED** | as 8.1 rows 2.2.2 and 2.3; Draft 0.1 §4.7's last sentence said the parts "carry `Manufactured` provenance pointing at the order" | §4.11; Q-39-27 |
| 4 | long orders need durable escrow; the Dray's 210 t bill does not fit a 100 t hold | **CONFIRMED** | as 8.1 row 2.2.3; `dray.json:85` (100 t), `dray_p.json:85` (86 t) | §4.12 the order escrow and multi-delivery; Q-39-28 |
| 5 | delivery needs a recovery protocol and a usable handover | **CONFIRMED** | as 8.1 rows 2.5, 3.1, 3.2 | §4.13; Q-39-33 |
| 6 | a funded customer cannot draft at Stickney's facility; IN.4's "a player's `@wo`" contradicts Q-39-19 | **CONFIRMED** | `@wo/draft` admits staff or a principal that `acts_for` the facility's owner (`commands.rs:1185–1189`) | §4.12 the customer policy; `+works` beside `+yard`; Q-39-30 |
| 7 | the Exchange buys carbon and strategic ore and sells neither, so the Stickney works cannot buy its carbon and the Ceres works has no path to it | **CONFIRMED** | lane `:2634` (asks `metals` only), `:2637–2638` (the two ore bids), `:2635` (the alloy bid's alias) | §4.2 the supply table; Q-39-31 |
| 8 | the mills and the Ceres works do not cover their stated daily cost | **CONFIRMED** | 16 t × 25 = 400 against 650 (the Exchange); 16 × 12.5 = 200 against 650 (Occator); 8 batches × (2,800 − 2,600 − 50) = 1,200 against 1,700 (Ceres) | §4.4 redone per site, billing by work unit, the mill rows withdrawn from the fee model; Q-39-32 |
| 9 | a slot can be held and cancelled at no cost, from `AwaitingDelivery` included | **CONFIRMED** | `holds_resources` includes `AwaitingDelivery` (`workorder.rs:521–529`); `wo_cancel` releases materials and the slot from any non-terminal state (`provider.rs:3846–3900`); the budget hold is released unspent (`money.rs:366–392`) | §4.12 the booking charge and the cutoff; Q-39-29 |
| 10 | the Trust cannot fund a ninth Dray and the build is at Stickney | **CONFIRMED** | 250,000 − 7 × 25,000 = 75,000 (lane `:2597`, `:2614`; `coop.rs:1629–1663`); as 8.1 row 2.6 | §4.7 funding from buyouts and a raise, the ferry, the register; Q-39-34 |
| 11 | the scrap mechanism narrows the accepted ceiling's scope, and 40 t at the 520 bid is 20,800 | **CONFIRMED** as a scope question | §4.6 exempts parts; 40 × 520 = 20,800 > 18,000 | raised to the owner as Q-39-25, not decided here |
| om. 1 | a new unit's magazines are born full | **CONFIRMED** | `definitions/unit.rs:1718–1725` (`init: rounds`) | §4.13 and Q-39-36: delivered empty |
| om. 2 | `power.walker` has no recipe; "light" is not a class | **CONFIRMED** | as 8.1 row 2.4 | §4.1, §4.3; Q-39-35 |
| om. 3 | adversarial acceptance is missing | taken | — | §4.9 and §6's stories |
| om. 4 | identity, units and escrow should precede their consumers; the first production story is too late | taken | — | §6 reordered; the first story is IN.5 |

**What neither review saw.** The yard's room: Draft 0.1 put the `yard` capability on `p11_service`
beside the workshop, which `workshop::preflight` refuses on a kept world (8.1 row 2.6); the yard gets
its own room. `checked_consume` ignores the deadline (8.1 row 2.2.3), so an overdue reservation is a
sweep's business, which the escrow rule in §4.12 makes explicit. And `class: part` goods must be
refused by the pool verbs outright, or every lifecycle rule in §4.11 has a hole the size of
`@inv/dissolve`.
