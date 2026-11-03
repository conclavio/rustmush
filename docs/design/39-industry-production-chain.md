# 39. Industry — the production chain and unit construction, surveyed

Draft 0.2, 2026-11-03, docs only. Surveyed against `origin/main` at `6f9dff29` (PR #344, the ten
tacsim shards) with [doc 38](38-after-the-ic-phase-next-systems.md) Draft 0.2 (PR #345, `0c7fd224`)
stacked under it: every anchor doc 38 §4 A cites was re-read at this tree, and the line numbers
below are this tree's where they moved (the matter pin is in `shard_9` now, not `shard_3`). This is
the document doc 38 §6 proposed as "Doc 39 — Industry: the production chain and unit construction",
economy-owned, and doc 38 §8's list of what it had to settle is this document's table of contents.
Draft 0.1, the same day, was the survey. **Draft 0.2 records the owner's rulings of 2026-11-03 on
every question: every default accepted as written** ("all q-39s accepted at their defaults"). No
question was amended, so each stands with its proposed text as the ruling, and §6's cut list is now
the implementation order.

The form is doc 38's: a survey that asks the owner numbered questions, each with a proposed default,
an alternative and a reason; this draft carries the rulings and the cut list is the plan.
"Provisional" marks a number to be tuned later under the 2026-10-24 rule (take a reasonable default,
tune later, never hold a build on it). **(expensive to reverse)** keeps doc 38 Q-38-10's meaning: an
answer that would take a significant refactor of the codebase to undo — not stored data and not a
live box, since every step may wipe and redeploy.

## 1. Why this document exists

Doc 38 §4 A surveyed the chain from ore to hulls for the first time and found it stops at the ore
mills: nothing refines, nothing makes a part, the workshop fits a new part from nothing, scrap mints
alloys unbounded by the hull's mass, and every hull in play was laid up by content or by staff. The
owner ruled (Q-38-1) that this chain leads the next round, because every repair, refit, salvage and
field-engineering conversation reaches for a part with a price and a store it came from, and
(Q-38-4, Q-38-5, Q-38-12, Q-38-13, Q-38-14, Q-38-15, Q-38-18) fixed the shape it must take. §2
lists those rulings; they are binding here and are not re-asked.

What is left to decide is what doc 38 §8 named: the catalogue rows, the stations, mill rows against
recipes, the facility cost model's inputs, the scrap ceiling's mechanism — and the pieces the brief
added beside them: the part lot's stores and bill, the mass budget as one rule, the yard's delivery,
and the cut list. Each gets a section in §4 with what exists, what is missing and the questions it
raises; §5 is the owner survey (Q-39-1..24, all ruled at the defaults); §6 the plan, cuts IN.0–IN.10.

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
what (doc 22 §7). The two outside reviewers' part-market items (doc 38 §3 R1-8, R2-9) are answered
by §4.5 and need no re-survey.

## 3. The tree at `6f9dff29`: the chain as built

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
family rule** — "a mill's ore and product are one family's goods" — is `check_ports`' (the goods
table's `family`: `metal_ore` and `metals` are both `metals`, `ice_ore` and `volatiles` both
`volatiles`). Space mining (SF.R5, D-35-13) lands the same four ores in `stow.ore`.

### 3.2 Refined goods (seeded, not made)

`structural_alloys` (goods :35–46, "Kasei's output, banded bar and plate", 400 cr/t, 300 L/t) is a
one-time seed of 7,000 t at Kasei (lane :2709) asked at 400 (:2711); Kasei bids `metals` at 280
(4,000 t intake, :2713) and `volatiles` at 200 (9,600 t, :2714) and converts nothing. The Exchange
bids alloys at 520 under the alias `"as": "Freeport Works"` (:2635, 7,000 t intake) — **"Freeport
Works" is a display name on one bid row, not an organization** (the Ceres orgs are :2595–2602).
Elsewhere Cabeus Works stocks 3,120 t at Luna (:2771) and bids concentrate at 270 (:2773); Syrtis
bids alloys at 460 (:2826), Selk at 760 (:2871). `industrial_parts` (goods :48–59, "yard fittings,
crated", 400, 2,300 L/t) is seeded at Stickney Yards, 5,400 t (:2717) asked at 400 (:2722), and the
Co-op bids 560 at Ceres (:2660). The only recipe is the test fixture `alloy_smelt`
(`fixture/tacsim_core.json:11–23`): 4 `metals` → 1 `structural_alloys`, `requires: ["smelt"]`, 3 work
units, at two fixture facilities, "Ceres Freeport Refinery" (`refine`, `smelt`, 2 slots,
`tacsim_corridor_v2.json:90–101`) and "Phobos Yard Forge" (`forge`, `smelt`, :134–144). The fixture
pack's commodity table says `industrial_parts` is `in_use: false` (:82–84). **At today's prices the
fixture's ratio loses money**: 4 t of concentrate at Kasei's own 280 bid is 1,120 cr for a tonne of
alloys Kasei asks 400 for. The deploy template loads no pack at all (`mushd.toml.tmpl:255`,
`pack_paths = []`), so no recipe is live outside the relay foundry's.

### 3.3 The engine (E7, built; one live user)

`FacilityRecord` (`mush-industry/src/facility.rs:75–82`): an object, an owner principal, a
capability set (at most 32, :54), `maximum_slots` (at most `MAX_FACILITY_SLOTS` 8, :51); availability
is derived from the reservation book, never stored. Three ways to make one: a pack `FacilityDef`
(`tacsim/src/pack.rs:1000–1017`: `label`, `at` a pack-local location, `capabilities`, `slots`),
staff's `@facility/open <object>=<slots> [capability...]` (`mush-industry/src/commands.rs:910–947`,
staff-only, the opener as owner, `@facility/owner` to hand it on :949ff), and **the port applier's
part j** (`tacsim/src/port/workshop.rs:1–50`), which opens the authored workshop room itself as a
facility of the operator org holding `bay_refit` with the authored berths as slots (`WorkshopDef`,
`port/def.rs:189–194`; Ceres' `c14_dockmaster`, Occator, 2 berths, ×1.2, lane :2678; Stickney's
`p11_service`, Stickney Yards, 4 berths, ×1.0, :2735). Recipes are pack entries `kind: "recipe"`
`{inputs, outputs, requires, work_units}` answered through `TacsimRecipeAuthority`
(`composition/industry_recipes.rs:25–112`) from the published pack's own digest; `PinnedRecipe`
refuses a recipe with no inputs (`workorder.rs:359`), so nothing seed-shaped can hide in a pack. Work
orders: `@wo/draft <pack>/<recipe>/<version>=<facility> <qty> <account> <source> <destination>
[beneficiary]` (`commands.rs:1135`, endpoints by number — a staff grammar), `@wo/submit`,
`@wo/deliver`, `@wo/cancel` (`@wo/set` refuses, :1488); one work unit an IC hour
(`PRODUCTION_CADENCE_SECS` 3,600, `workorder.rs:69`); at most 10 inputs (:44); E4 holds for the
money. Delivery consumes the inputs from the source (`provider.rs:3747`) and **mints each output
line as a lot at grade 0** (:3778 `Grade::new(0)`, :3792 `mint_origin` with the order as the
receipt): seeds are grade 50, strip's parts 9 or 3, so recipe outputs need a grade rule. The one
target is `DeliveryTarget::Endpoint` (`workorder.rs:446–454`). The one live recipe is the lunar relay
foundry's `private_pair` (`relay_foundry.rs:22`; `content/relays/manufacture-v1.json`: 1 `relay_blank`
+ 2 `instrument_boards` → 1 `relay_pair`, 120 work units, `requires: ["relay.entanglement"]`).

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
definitions, so `+bay/fit`'s "of the slot's definition" check (`bay_work/mod.rs:712`) refuses the
cross-fit while the market would list them as one good. The commodities have no row in
`goods.json`, so a part lot has no mass: `strip::hold_takes` (`bay_work/strip.rs:96–123`) sums bulk
lots only (:100, `lot.part.is_none()`), and a stripped cannon weighs nothing in the hold.
`Replace{lot: None}` fits an Intact part from nothing (`jobs.rs:467–491`, :480) for
`labor_cost_replace_component` 40,000 minor = 400 cr and 14,400 IC s (`workshop_tariff.json:11–12`),
scaled 0.5–2.0 by the operator (:15–16). `PartProvenance` (`bay_work/act.rs:85–110`) records the
parent hull, slot, definition, condition, claim, legality, stripper and time — every variant assumes
a parent hull.

### 3.5 Scrap (unbounded by mass; priced above the hull)

`SCRAP_KG_PER_STRUCTURAL_POINT` 100 (`mush-tactics/src/field_ops/workshop.rs:28`) and
`FALLBACK_SECTION_SCRAP_KG` 1,000 (:31) make a hull's scrap 100 kg a surviving point; `strip::scrap`
(`strip.rs:255–300`) divides by `SCRAP_KG_PER_T` 1,000 (:49) into one `structural_alloys` lot at
`SCRAP_GRADE` 50 (:51) plus a lot per salvageable part (:282–299), everything to the carrier's hold
through `mint_origins` (`act.rs:785`, receipt kind `bay_scrap`); the shell stays in the room
(:322, :395). Remains on a site scrap the same way into the hold (`surface/tow.rs:1735–1800`,
`mint_origin` :1781). The escort's 2,990 points are 299 t out of an 8 t hull (`tv_escort.json:332`),
and the Dray's 1,820 points (`dray.json:15–45`: hull 120/80/80/60/60 + 600 internals, hold
60/60/50 + 400, drive 250) are 182 t out of 240 (:83) — inside its mass, but worth 94,640 cr at
Freeport Works' 520 bid against a 90,000 cr buyout. The repair side exceeds the hull too:
`structural_points_per_industrial_parts_t` 100 (`workshop_tariff.json:6`) is 10 kg of fittings a
point, so rebuilding the escort's 2,990 points draws 29.9 t of `industrial_parts` into an 8 t hull.
Repair draws 10 kg where scrap returns 100.

### 3.6 Hulls (content pools and staff)

The Co-op First Captain programme (lane :2604–2629): eight Drays (`pool` 8, :2611), stake 25,000
(:2614), `buyout_major` 90,000 (:2625), the `prospector` refit to the Dray-P (:2628). Occator's rig
lease (:2668–2675): eight `tv_prospector` at 1,400 + 5,000 deposit, two `tv_escort` at 2,500 +
15,000. The Occator Yard house hull (`port/yard.rs:1–20`, a Dray-P on Occator Rim's pad). The port
applier lays each up (`port/hull.rs:1–60`); `+coop/enrol` leases (`coop.rs`), `+coop/buyout` turns
the charter into title (`buyout.rs`), `+coop/refit` re-binds the unit for no materials (`refit.rs`).
Staff `@unit/create` is the only other source. Dry masses: `tv_prospector` 6 t (`:97`), `tv_escort`
8 t (`:332`, payload 220 kg :346, "so it rides a Dray-P bay beside one prospector rig"), the Dray and
Dray-P 240 t (`dray.json:83`, `dray_p.json:83`; the Dray-P's rig slot takes 20 t, :206). The line
tank's `tank_minor` 10,000,000 (100,000 cr, `hardware/content.rs:160`) is the basis for core prices
only (Class I/II/III at 3/8/25 %, :171: 3,000, 8,000 and 25,000 cr; the fitting fee `FITTING_MINOR`
10,000 = 100 cr, `mush-economy/src/core_refit.rs:32`); no other hull has a reference price, and the
escort's lease deposit is a deposit. Doc 18 Q-18-70 (l.3528): a graduated hull leaves the Trust's
pool for ever, so the desk answers `NO_HULL` once all eight are chartered.

### 3.7 The price ladder as it stands

| Stage | Good | Ask (where) | Bid (where) | Note |
|---|---|---|---|---|
| ore | `metal_ore` | — | 70 Exchange | 5:2 → concentrate at 175/t cost |
| ore | `ice_ore` | — | 35 Occator | 5:2 → water at 87.5/t |
| ore | `carbon_ore` | — | 50 Exchange | no consumer |
| ore | `strategic_ore` | — | 600 Exchange | no consumer |
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
(the seed, with its `StockReceipt`, :10–18, :420) and `surface/tow.rs` (remains scrap) only;
`.credit(` in the mill and the relay foundry; `.debit(` in the bay's draw, the burn and the mill.
`.mint_origins(` (`bay_work/act.rs:785`, the strip and scrap mints) matches no pattern, as doc 37
§5.1 (l.163–165) says. `mush-industry`'s own delivery mint is outside the pin's files and under its
own tests (`mush-industry/tests/e7r1.rs:2219`).

## 4. The survey

### 4.1 The goods catalogue

**What exists.** §3.7. `goods.json` v4 has nineteen rows with mass and packed volume per unit, a
`family`, a `class` and a `reference_major`; the lane's `reference` map (:2616–2623) names which
vendor's ask is each good's reference for the Co-op share, and `reference_minor` (:2624) the ores'.

**What is missing.** Rows for the part commodities (mass, volume, a reference price, a ledger unit
of one) and an identity for them that a recipe can make and a slot can accept (§3.4); a reference
price per hull design; a ladder in which each transformation covers its inputs. Under the fixture's
4:1 and today's asks a refinery loses 720 cr a tonne; a 5:4 refine (concentrate is mostly metal; 10 t
of ore → 4 t of concentrate → 3.2 t of alloys) costs 350 cr of concentrate at Kasei's 280 bid for a
tonne asked at 400, which the facility's ~60 cr/t (§4.4) turns into a loss. The works' parts have
the same shape: 5 t of alloys and 1 t of carbon ore for 5 t of fittings asked at 400. The ladder has
to move a little at two rungs, and the movement is lane data. The proposed ladder (provisional):

| Good | Today ask / bid | Proposed ask / bid | Why |
|---|---|---|---|
| `metals` | 200 / 280 | unchanged | the mill's 175 cost and the trade run's margin stand |
| `structural_alloys` | 400 / 520 | **450** Kasei / 520 Works | the 5:4 refine clears 100/t gross, ~40 net; the hauler keeps 70/t |
| `industrial_parts` | 400 / 560 | **520** Yards / 560 Co-op | 5 alloys at 450 + 1 carbon ore at 50 = 2,300 for 5 t at 520; the hauler keeps 40/t |
| `carbon_ore` | — / 50 | — / 50, and a Stickney bid at 60 | the works at Stickney needs it hauled; a second outbound cargo |
| `strategic_ore` | — / 600 | — / 600, and a Stickney bid at 700 | the sensor recipe's input |

The part rows, keyed by the hardware row (Q-39-3) and priced as shares of the line tank's 100,000 so
that the tank's parts and core sum near its basis (the 105 mm round's 150 cr is "a sixteenth of the
tank" by the goods note :204, which this ladder keeps):

| Row | Mass | Reference | Row | Mass | Reference |
|---|---|---|---|---|---|
| `tacsim.part.cannon_105` | 2,400 kg | 28,000 | `tacsim.part.visual_mk1` | 90 kg | 9,000 |
| `tacsim.part.autocannon_30` | 500 kg | 12,000 | `tacsim.part.thermal_mk1` | 130 kg | 18,000 |
| `tacsim.part.laser_mk1` | 1,200 kg | 36,000 | `tacsim.part.ecm_mk1` | 160 kg | 24,000 |
| `tacsim.part.missile_lr` | 800 kg | 18,000 | `tacsim.part.eccm_mk1` | 120 kg | 20,000 |
| `tacsim.part.mg_coax` | 40 kg | 2,500 | `tacsim.part.power.vehicle` | 1,800 kg | 22,000 |
| `tacsim.part.power.light` (rig, scout, drone) | 300 kg | 5,000 | `tacsim.part.power.walker` | 2,500 kg | 26,000 |
| `tacsim.part.power.hauler` (the Dray's plant) | 30,000 kg | 30,000 | `tacsim.part.relay` | 4,000 kg | 40,000 |

Volumes follow the mass at 1 m³/t for mounts and 2 m³/t for crated instruments. A hull's reference
price is derived from its bill (§4.7), never authored, except the programme's `buyout_major`, which
stays lane data and is validated at most the design's bill.

**Questions.** Q-39-1, Q-39-2, Q-39-3.

### 4.2 The facilities

**What exists.** The port applier's part j (§3.3) is the precedent: a port row (`workshop`,
`medical`) names a room, an operator org, a capability set and a slot count, and the applier opens
the room as a `FacilityRecord` owned by the operator, refusing drift under hulls (`workshop.rs:60ff`).
Pack `FacilityDef`s exist for campaign seeds and need a pack path the deploy does not set. The lane's
orgs: Kasei Metals (capital 600,000, :2702), Stickney Yards (50,000, :2703), the Exchange
(4,000,000, :2598), Occator (50,000 + a 250,000 raise, :2599), the Co-op (3,100,000, :2596), the Trust
(250,000, :2597), and **Tycho Mutual Assurance & Salvage** (50,000, faction concord, :2600), an org
with no vendor rows and the salvage word already in its name. The Stickney port's rooms key `yard`
names `p11_service`, the same room the workshop occupies (:2735).

**What is missing.** Three capabilities on three stations, each a port row the applier opens, with
the cost-model inputs beside the slots. Proposed placement (doc 10 §4: Mars/Phobos is "heavy
industry, shipbuilding, military manufacturing"; Ceres "water, propellant production, logistics"):

| Station | Row | Operator | Capability | Slots | Room |
|---|---|---|---|---|---|
| Stickney Yards | `refinery` | Kasei Metals | `smelt` | 2 | a new `p12_refinery` room off the spine |
| Stickney Yards | `works` | Stickney Yards | `works` | 2 | a new `p13_works` room |
| Stickney Yards | `yard` | Stickney Yards | `yard` | 2 | `p11_service` gains the capability (the workshop and the yard are one shop with two slot books), or a new `p14_slipway` |
| Ceres Freeport | `works` | **Freeport Works**, a new org taking the Exchange's alloy bid (the `as` alias becomes the org) | `works` | 2 | a new `c15_works` room |
| Ceres Freeport | the salvage organization | Tycho Mutual | `recovery` (doc 41 reads it) | 1 | its crawler's berth (doc 41) |

No yard at Ceres: the Freeport buys and sells, Phobos builds, and the trade run exists because they
differ. The refinery at Kasei and the Stickney works sit at the end of the concentrate run; the Ceres
works gives the Co-op's 560 fittings bid a local maker fed by the alloys the trade run brings back
and the carbon ore mined on Ceres. Sights and pods (the strategic-ore recipes) are the Stickney
works' alone, on instrument boards hauled from Luna's Crisium Instruments — a third leg the moon
ports (doc 19 A.8) have waited for.

**Questions.** Q-39-4, Q-39-5, Q-39-23.

### 4.3 Recipes against mill rows, and conservation

**What exists.** Two engines that convert matter. The mill (§3.1) is a house process on the port
tick: one input, one output, a fixed ratio, a daily cap, no requester, no fee, its margin the ratio
(the Exchange's 40 t a day of ore at 175 cost against the 200 ask is ~1,000 cr a day), already inside
the census pin, authorable as lane data, and barred from a second transformation only by the same
family rule. The work order (§3.3) is a requester's act with up to ten inputs, a facility slot, a
work-unit duration, E4 holds and a receipted delivery, driven today by `@wo` or by R9a's foundry
code — no NPC house runs one.

**What is missing.** A rule for which transformation uses which. Q-38-13 allows both. The refine at
Kasei is one input, one output and a house's own business; the works' fittings take two inputs; the
part recipes three; a hull a bill of ten lines and a delivery that is not a lot. The proposed rule:
**a house conversion with one input is a mill row with the family rule relaxed; everything with two
or more inputs, or a requester, is a recipe.** Kasei's row becomes `{ore: metals, into:
structural_alloys, ratio: {ore: 5, product: 4}, batch_t: 20, per_ic_day_t: 40}` (`MillDef` gains
`kind: refine`, under which `check_ports` lets the product's family differ); its ledger, its restock
and its drift page come for free. Recipes live in a live pack, `content/industry/recipes-v1.json`,
named by the deploy template beside the relay pack:

| Recipe | Inputs | Output | Requires | Work units | Loss |
|---|---|---|---|---|---|
| `fittings` | 5 `structural_alloys`, 1 `carbon_ore` | 5 `industrial_parts` | `works` | 6 | 1 t |
| `mount_105` | 3 `structural_alloys`, 1 `industrial_parts` | 1 `tacsim.part.cannon_105` | `works` | 120 | 1.6 t |
| `mount_30` | 1 `structural_alloys`, 1 `industrial_parts` | 1 `tacsim.part.autocannon_30` | `works` | 60 | 1.5 t |
| `mount_mg` | 1 `industrial_parts` | 10 `tacsim.part.mg_coax` | `works` | 24 | 0.6 t |
| `mount_laser` | 2 `structural_alloys`, 2 `industrial_parts`, 1 `instrument_boards` | 1 `tacsim.part.laser_mk1` | `works` | 160 | 3.8 t |
| `rack_lr` | 1 `structural_alloys`, 1 `industrial_parts` | 1 `tacsim.part.missile_lr` | `works` | 72 | 1.2 t |
| `sight_visual` | 2 `strategic_ore`, 1 `instrument_boards`, 3 `industrial_parts` | 10 `tacsim.part.visual_mk1` | `works` | 60 | 5.1 t |
| `sight_thermal` | 3 `strategic_ore`, 2 `instrument_boards`, 3 `industrial_parts` | 10 `tacsim.part.thermal_mk1` | `works` | 120 | 6.7 t |
| `pod_ecm` / `pod_eccm` | 3 `strategic_ore`, 2 `instrument_boards`, 2 `industrial_parts` | 10 pods | `works` | 120 | 5.4 t |
| `plant_vehicle` | 2 `structural_alloys`, 1 `industrial_parts`, 1 `carbon_ore` | 1 `tacsim.part.power.vehicle` | `works` | 96 | 2.2 t |
| `plant_light` | 1 `industrial_parts`, 1 `carbon_ore` | 5 `tacsim.part.power.light` | `works` | 60 | 0.5 t |
| `plant_hauler` | 25 `structural_alloys`, 8 `industrial_parts`, 2 `carbon_ore` | 1 `tacsim.part.power.hauler` | `works` | 480 | 5 t |
| hulls | the design's bill (§4.7) | the hull | `yard` | the design's days × 24 | per design |

Lines are whole tonnes because `RecipeLine.units` are ledger units; small parts come ten to a batch.
Every row's loss is the difference between its inputs' mass and its outputs' catalogue mass — slag,
off-gas, swarf — and O-3's "no tailings" stays the mill's rule only. Carbon ore goes straight into
the works, not through a feedstock pressing step (Titan's `carbon_feedstock` is a different good for
a different trade), keeping the chain three deep (Q-39-23). Strategic ore feeds the sights and pods
now; the core track's dies stay a receipted seed (Q-39-22).

**Conservation (Q-38-14).** Every mint is one of: the stock clerk's receipted seed (`stock.rs:420`),
a recipe's delivery (`provider.rs:3792`, the order as receipt), a mill or refine batch's credit
(`mill.rs`, the ledger as receipt), a strip or scrap mint (`act.rs:785`, `bay_{why}/{hull}/{act}`
as receipt), a remains scrap (`tow.rs:1781`). The pin widens to count `.mint_origins(` with the
same file list, and a second pin asserts the receipt kinds: every call site names an `ExternalRef`
whose `kind` is one of a closed set. Above the pins, one **matter story** (IN.10) runs the chain end
to end and asserts Σ mined = Σ (pools + lots + hull bills + declared losses) at the end. The seeds
(Kasei's 7,000 t, the Yards' 5,400 t) stay as the opening stock; their `intake` caps — which today
bound a house's one-time buying — become the facility's consumption: Kasei bids concentrate because
its refine row consumes 40 t a day, and the bid's refresh is the row's `per_ic_day_t` (the ore
drips' shape, `stock.rs:28–41`), so a drained house stops bidding and a fed one keeps buying. That is
what "the intake drips retired" means here: not the ore bids at Ceres (the ore economy's size knob
stays), but the one-time intakes at the refinery and the works.

**Questions.** Q-39-6, Q-39-7, Q-39-8, Q-39-22, Q-39-23.

### 4.4 The facility cost model (Q-38-12)

**What exists.** The tariff (`workshop_tariff.json:3–17`; its note :18 gives the basis): one IC hour
of bay labour at 100 cr (the core-fitting fee), a structural point 36 s and 1 cr, a component repair
two hours (200 cr), a replacement four hours (400 cr), a section rebuilt eight hours (800 cr) plus
its points, the operator's multiplier 0.5–2.0. Nothing derives it. `mush-economy`'s `FITTING_MINOR`
(100 cr) is the one other labour figure.

**What is missing.** The model. Proposed:

```
daily_cost      = capital / lifetime_days + capital × upkeep_permille / 1000 / 365 + staff × wage
slot_day        = daily_cost / slots
fee(order)      = slot_days(order) × slot_day × (1 + margin_permille / 1000)
slot_days       = work_units / 24                       (one work unit an IC hour; a slot is held whole days, rounded up)
manufacturing   = fee at margin 0                        (Q-38-12's "before labour and margin"; labour is the staff term)
```

`FacilityCostDef { capital_major, lifetime_years, upkeep_permille, staff, wage_major,
margin_permille }` is authored on each facility row of the lane beside its slots; the applier
stores it on the record's policy half (a `policy_revision` bump, so an order drafted against the old
numbers re-validates — `facility.rs` already has the revision). Provisional inputs, with a wage of
150 cr an IC day a head (provisions are 10 cr a crew-day; a licence 15 cr a day; a captain's stake
25,000):

| Facility | Capital | Life | Staff × wage | Upkeep | Daily cost | Slots | Slot-day |
|---|---|---|---|---|---|---|---|
| Stickney Yards yard | 10,000,000 | 30 y | 12 × 150 = 1,800 | 2 %/y = 548 | 913 + 548 + 1,800 ≈ **3,300** | 2 | **1,650** |
| Kasei refinery (`smelt`) | 6,000,000 | 30 y | 10 × 150 = 1,500 | 2 % = 329 | 548 + 329 + 1,500 ≈ **2,400** | 2 | 1,200 |
| a works (Stickney, Ceres) | 3,000,000 | 25 y | 8 × 150 = 1,200 | 2 % = 164 | 329 + 164 + 1,200 ≈ **1,700** | 2 | 850 |
| a workshop (Ceres 2 berths; Stickney 4) | 1,500,000 | 20 y | 6 × 150 = 900 | 3 % = 123 | 205 + 123 + 900 ≈ **1,250** | 2 / 4 | 625 / 313 |
| Tycho Mutual's recovery (crawler + lift) | 4,000,000 | 15 y | 6 × 150 = 900 | 4 % = 438 | 730 + 438 + 900 ≈ **2,100** | 1 | 2,100 |
| a mill (Exchange, Occator) | 1,000,000 | 20 y | 3 × 150 = 450 | 2 % = 55 | 137 + 55 + 450 ≈ **650** | 1 | 650, recovered in the ratio |

The yard's 913 cr a day of capital is the owner's 1,000; staffing and upkeep bring the whole shop to
3,300 and a slot to 1,650. **The arithmetic for two hulls** (materials at the proposed references;
the mass fractions σ for alloys and φ for fittings are §4.8's):

| Line | Escort (`tv_escort`, 8 t, class vehicle) | Dray (240 t, class hauler) |
|---|---|---|
| parts (one lot a slot) | cannon_105 28,000 + mg_coax 2,500 + visual_mk1 9,000 + power.vehicle 22,000 = 61,500 | power.hauler 30,000 |
| core | Class II line core 8,000 | none (Class 0) |
| alloys | σ 0.6 × 8 t = 4.8 t × 450 = 2,160 | σ 0.55 × 240 t = 132 t × 450 = 59,400 |
| fittings | φ 0.2 × 8 t = 1.6 t × 520 = 832 | φ 0.2 × 240 t = 48 t × 520 = 24,960 |
| build | 2 IC days (the floor) × 1,650 = 3,300 | 14 IC days × 1,650 = 23,100 |
| margin on the build | 25 % = 825 | 25 % = 5,775 |
| **bill** | **≈ 76,600** | **≈ 143,000** |

Against the tree: the escort's 15,000 lease deposit is a fifth of its bill; the Dray's 90,000 buyout
is 63 % of a new one, which is what a stake-financed used hull should cost; the line tank's 100,000
basis stays the core-price basis. Build days are 1 a day per 20 t of dry mass, rounded up, floor 2,
so the Dray's "two weeks" (Q-38-12) is 12 days by the rule and 14 by the owner's sentence — the
table takes 14; the rule needs the owner's eye (Q-39-14). The workshop row prices a berth-hour at
26 cr (Ceres) against the tariff's 100: the model's figure is the floor an operator's multiplier may
not go under, and the tariff's rows are left as they are this round (Q-39-9).

**Questions.** Q-39-9, Q-39-10, Q-39-24.

### 4.5 The part lot: price, stores, bill, and what feeds the stores (Q-38-4)

**What exists.** §3.4. The bay's draw (`bay_work/act.rs`) takes an order's goods from the carrier's
hold when the work is done, all or nothing; `+bay/fit` takes a traced, clean lot from the hold; strip
and scrap put lots into the hold; `+market/list`, `+market/quote` and `+market/sell`
(`mush-market/src/interface.rs:44/58`, `trade.rs:1620`) trade lots at a port; a vendor's stock is a
storefront pool with receipted seeds; `bay_work/mod.rs:34–37`: "the workshop supplies a replaced
part, the hold everything else".

**What is missing.** The store a `Replace{lot: None}` draws from, its price, the seed and the
provenance of a part that was never stripped. Proposed: the facility's stores are **the operator
org's storefront vault at that port** (Occator's at Ceres, Stickney Yards' at Stickney) — the same
vault its stock rows mint into, so a part the house makes (IN.6), buys, or takes in scrap (§4.6) and
a part a player sells to its bid are one store. `Replace{lot: None}` becomes `Replace{lot:
Store}`: the quote names the catalogue price beside the labour, the order holds both, the draw takes
the lowest-grade clean lot of the slot's row from the store and the fee pays the operator; an empty
store refuses at the quote — `NO_PART`: "The workshop has no <row> in its stores; +market/list the
yard, or bring one in the hold." — exactly as `NO_HULL` refuses the desk. Seeds until the recipes
land: part rows in the vendor's `stock` (`{good: "tacsim.part.cannon_105", qty: 6, grade: 9, issue: 1}`)
through the stock clerk's receipt, Occator at Ceres carrying four of each escort and rig part, the
Yards at Stickney six of every row. `PartProvenance` gains an origin: `Stripped {parent, slot,
claim…}` (today's), `Manufactured {order, facility}`, `Seeded {receipt}`; `+bay/fit` accepts all
three (a "traced" lot is one with provenance of any origin). A part's bill of materials is its recipe
(§4.3); a strip costs labour only and returns the part at its condition's grade; a scrap returns the
parts as lots and the structure as alloys (§4.6). The hold's room check counts parts by catalogue
mass (§4.1), so a Dray-P's 20 t rig slot and 86 t hold weigh a cannon.

**Questions.** Q-39-11, Q-39-12, Q-39-13.

### 4.6 The scrap ceiling (Q-38-5)

**What exists.** §3.5: 100 kg a point, every point, into the hold, no fee, no cap.

**What is missing.** The mechanism. The ruling gives two bounds and the fact that a facility takes
the rest. Two hulls show that no single tuned constant satisfies both bounds: the escort's whole
structure (σ 0.6 × 8 t = 4.8 t) at a recovery fraction `r` 0.6 is 2.9 t of alloys, ~1,300 cr at the
450 reference, far under a fifth of its 76,600 bill (15,300) — the mass bound binds; the Dray's 132 t
of structure at 0.6 is 79 t, 35,600 cr, far over a fifth of its 90,000 buyout (18,000) — the value
bound binds. Proposed:

```
structure_t      = σ × dry_mass_t × surviving_points / total_points
mass_yield_t     = structure_t × r                                   (r 0.6, provisional)
ceiling_t        = reference_price / 5 / alloy_reference             (18,000 / 450 = 40 t for a Dray)
yield_t          = min(mass_yield_t, ceiling_t)
```

At a workshop, **the whole mass yield goes into the operator's stock** (Q-38-5's "the port's
stock"), minted at grade 50 under the `bay_scrap` receipt, and the title holder is credited `yield_t
× alloy_reference` less the scrap job's fee from §4.4 (a Dray is 12 slot-days of yard time, 7,500 cr;
an escort one day, 313 cr): the facility keeps the tonnes above the ceiling in kind, which is the
owner's "a navy pays its scrappers". For a Dray: 79 t into the Yards' pool, 40 t × 450 = 18,000
credited, 7,500 fee, 10,500 to the holder — and 79 t of alloys that refill Kasei's and the Yards'
stock from a wreck, which is the conservation the chain wants. For remains on a site, where there is
no facility, the capped `yield_t` alone is minted into the carrier's hold with the room check and
no fee, and the rest is lost to the ground: scrapping a Dray's remains on Urvara yields 40 t, a
whole hold and a half of a Dray-P. The parts come off as lots at their own value in both cases
(the ceiling is on the alloys, not on a stripped cannon: a player strips first, as the salvage loop
intends). `SCRAP_KG_PER_STRUCTURAL_POINT` and `FALLBACK_SECTION_SCRAP_KG` retire; the per-design
figures come from the catalogue build (§4.8). `reference_price` is the design's bill (§4.7), or the
programme's `buyout_major` for a programme hull.

**Questions.** Q-39-16, Q-39-17.

### 4.7 Unit construction (Q-38-12)

**What exists.** §3.6 for where hulls come from; §3.3 for the engine. The applier's `lay_up_one`
(`port/hull.rs:1–60`) is the one path that makes a hull whole — Thing, unit, rooms, store, airlock,
bays, placement, title — and it runs only from a port entry's programme. `DeliveryTarget::Endpoint`
is the work order's one way to deliver. Q-31-2 lets a registry-only design be built; only staff
`@unit/create` does it; doc 31 §9 (l.447) left "what a design costs to register and to build" to
doc 08.

**What is missing.** The yard order, its delivery and the pools' way back to it.

- **The bill.** `design.bill = { parts: one row a slot (from the component list), alloys_t: σ ×
  dry_mass_t, fittings_t: φ × dry_mass_t, core: the design's core class or none, days: max(2,
  ceil(dry_mass_t / 20)) }`, computed at the catalogue build from the design and the goods table,
  with authored overrides on the unit def (`bill: {…}`) for a design the rule misjudges. The price is
  the bill at the references plus the yard's manufacturing cost and margin (§4.4's table). A unit
  def with no `dry_mass_t` (the simulator's tanks, bipeds and drones carry none; `logistics` is the
  port units') gets one from its class's default and the check refuses a bill whose mass exceeds the
  dry mass.
- **The order.** `+yard` at a port whose `yard` row exists, the way `+bay` fronts the bay: `+yard`
  lists the designs the yard builds (every registered design whose class the yard takes), `+yard/quote
  <design>` prints the bill, the days and the fee, `+yard/order <design>` drafts and submits the work
  order with the requester's hold or the port's storefront as the source (materials bought at the
  asks land in the hold first; the order checks the hold carries the bill when taken, the GC.8b
  rule), the E4 hold for the fee, the yard slot for the days; `+yard/cancel` releases. `@wo` stays the
  staff and test grammar under it.
- **The delivery.** `DeliveryTarget::Hull { design, title: PrincipalId, berth: ObjId }`
  **(expensive to reverse)**: at completion the provider consumes the lots and, instead of minting an
  output, lays the hull up by the applier's own steps at the yard's berth — a ship (class hauler,
  capital) into the berth gallery as a docked hull, a vehicle (class vehicle, drone, walker) into the
  yard's workshop room as a berthed hull a docked carrier may `+bay/stow` — titled to `title` under
  the registry issuer with a fresh mark, and writes `tacsim.hull_provenance { order, facility,
  design digest, input lots, at_ic }`, the record every later claim, resale and scrap reads. The
  hull's parts carry `Manufactured` provenance pointing at the order.
- **The pools.** The content pools are houses with floors: the Trust keeps two free Drays, Occator
  two free rigs and one escort. When a pool is under its floor the house's arm on the port tick (the
  dispatcher's precedent, `port/yard.rs`) drafts and submits a yard order for the design from its
  own capital, sourcing the bill from the Stickney vendors at their asks by a receipted transfer, and
  the delivered hull joins the pool; `NO_HULL` becomes "the Trust has a Dray on order at Stickney,
  due in n IC days". A house whose capital cannot cover a bill stops ordering, as the ore drip's
  capital clause stops a bid. Q-18-70's shrink is answered by production, not reversed.

**Questions.** Q-39-14, Q-39-18, Q-39-19, Q-39-20, Q-39-21.

### 4.8 The mass budget: one rule across mining, milling, repair and scrap

**What exists.** Mining draws a deposit's tonnes into a hopper and a hold by measure (doc 20 §3, the
census once a bundle); the mill is 5:2 by its ratio; repair draws 10 kg of fittings a point; scrap
returns 100 kg of alloys a point; a part weighs nothing. Three constants, none from the hull.

**What is missing.** The hull's own figures. Proposed: every design's catalogue build derives
`structure_kg_per_point = σ × dry_mass_kg / total_points` and `fittings_kg_per_point = φ ×
dry_mass_kg / total_points` (escort: 4,800 kg / 2,990 = 1.6 kg of alloys and 1,600 / 2,990 = 0.54 kg
of fittings a point; Dray: 132,000 / 1,820 = 72.5 kg and 48,000 / 1,820 = 26 kg); σ and φ are per
class (vehicle 0.6/0.2, walker 0.55/0.25, hauler 0.55/0.2, drone 0.5/0.3, capital 0.5/0.25,
provisional) with the same authored override as the bill. Then:

| Act | Draws | Returns | Today |
|---|---|---|---|
| armour and rebuild | `points × structure_kg_per_point` of alloys **and** `points × fittings_kg_per_point` of fittings from the hold or the store, plus labour | — | 10 kg of fittings a point, no alloys |
| replace | one part lot (its catalogue mass) plus labour | the destroyed part as nothing | a part from nothing |
| strip | labour | the part at its grade | the same |
| scrap at a workshop | the scrap fee | §4.6's yield to the operator, the credit to the holder, parts to the hold | 100 kg a point to the hold |
| remains scrap | — | §4.6's capped yield to the hold | 100 kg a point |
| build | the bill | the hull | — |

An escort's armour patch of 100 points draws 160 kg of alloys and 54 kg of fittings; a Dray's draws
7.25 t and 2.6 t — 3,260 and 1,350 cr, which is what a 240 t hull's hundred points should cost. The
imbalance resolves because repair draws 2.1 kg a point and scrap returns at most 0.97 (1.6 × 0.6):
repair-then-scrap loses mass and money. `structural_points_per_industrial_parts_t` leaves the tariff
with the two scrap constants; the tariff keeps labour and time only. Milling keeps its ratio and O-3;
mining is already conserved.

**Questions.** Q-39-15.

### 4.9 Acceptance

Q-38-29 (ruled): every new cut's story runs at 256 and 512 and the bot types the player's half over
SSH; the bot's four-digit addresses and vocabulary are the first cut of whichever document starts
first. The chain's stories are port-side, so "both sizes" means the mining half runs on Ahuna (256)
and Urvara (512) and the port half once; the bot gains `+yard`, `+market/sell` of a part lot and the
`+bay/order replace` quote. Every cut in §6 names its story through the production command paths
(`@wo`, `+bay`, `+yard`, `+market`, `+coop`), never a staff hand past the seams doc 36a §8 allows.

## 5. Owner survey

**Ruled 2026-11-03.** The owner accepted every default as written ("all q-39s accepted at their
defaults"); nothing was amended, so no question carries a decision line of its own and the proposed
text of each is its ruling. Each question keeps its proposed default, alternative and reason for the
record; §6 is the plan built from them.

### 5.1 The catalogue

**Q-39-1 — The price ladder.**
- **Proposed default:** §4.1's ladder: alloys asked 450 at Kasei, fittings 520 at the Yards, bids at
  Ceres unchanged (520, 560), Stickney bids for carbon ore (60) and strategic ore (700); the part
  rows at the shares in §4.1's table; lane v20 "the chain's numbers". Provisional, every one.
- **Alternative:** every price stays and the recipes' ratios absorb it (a 2:1 refine, the works at a
  loss carried by the house's capital).
- **Why:** two rungs cannot cover their inputs at today's asks (§4.1), and lane data is the cheapest
  thing in the tree to move; a house run at a loss is doc 18 risk 9 again.

**Q-39-2 — Part rows in the goods catalogue.**
- **Proposed default:** one `goods.json` row a part commodity (§4.1's fourteen), `class: "part"`,
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
  `PartProvenance.definition` change together; the bind's component definition id stays.
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
  the strategic-ore recipes at the Stickney works only; Tycho Mutual at Ceres for Q-38-18.
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
  or deliver a fire, bounded by the capital clause; a player's order shares the slots.
- **Alternative:** only players order; the houses are seeds that the chain refills through sales
  alone.
- **Why:** doc 10 §5's "NPC corporations provide baseline liquidity"; without house production the
  Yards' fittings ask drains to nothing before the first player works a recipe, and the pools never
  restock (Q-39-21).

**Q-39-8 — Conservation's pins and the matter story.**
- **Proposed default:** the pin counts `.mint_origins(` with the same file list; a second pin closes
  the set of `ExternalRef.kind` values a mint may carry; IN.10's matter story asserts the sum over
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
  multiplier may not go under; the tariff is re-derived from the model at the tune, not now.
- **Alternative:** re-derive the tariff now (bay labour from 100 to ~33 cr an hour).
- **Why:** the tariff's numbers are pinned by GC.7/GC.8's tests and stories, and moving them buys no
  play; the model is what the yard, the works and the crawler need, and it must exist before any of
  them prices a day.

**Q-39-10 — The provisional inputs.**
- **Proposed default:** §4.4's table (the yard 10 M over 30 years with twelve staff ≈ 3,300 a day,
  1,650 a slot-day; the refinery 2,400; a works 1,700; a workshop 1,250; the recovery 2,100; a mill
  650) and a wage of 150 cr an IC day a head.
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
  at Ceres four of each escort and rig part (`cannon_105`, `mg_coax`, `visual_mk1`, `power.vehicle`,
  `power.light`); Stickney Yards six of every row; grade 9; retired as the recipes land (IN.6).
- **Alternative:** no seed; the workshop refuses every replace until IN.6.
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
  manufacturing cost and margin.
- **Alternative:** a fully authored bill on every design.
- **Why:** fourteen shipped designs and every registry design need a bill on the day the yard opens;
  the rule gives each one and the override fixes the ones it misjudges.

**Q-39-15 — The mass budget.**
- **Proposed default:** §4.8: per-design `structure_kg_per_point` and `fittings_kg_per_point` from σ,
  φ, the dry mass and the points; repair draws both goods; the three constants
  (`SCRAP_KG_PER_STRUCTURAL_POINT`, `FALLBACK_SECTION_SCRAP_KG`,
  `structural_points_per_industrial_parts_t`) retire; a part weighs its catalogue mass in the hold.
- **Alternative:** keep per-point constants and clamp scrap at the dry mass.
- **Why:** a clamp leaves an escort's armour patch drawing 1 t of fittings into an 8 t hull, and a
  Dray's drawing the same tonne; the hull's own figures make both right at once.

**Q-39-16 — The scrap mechanism.**
- **Proposed default:** §4.6: `yield_t = min(structure_t × r, reference_price / 5 /
  alloy_reference)`, `r` 0.6; at a workshop the whole mass yield into the operator's stock, the
  holder credited the capped yield at the reference less the scrap job's fee from the cost model;
  remains scrap mints the capped yield into the hold, no fee, the rest lost; the ceiling is on the
  alloys, parts come off at their own value.
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

**Q-39-18 — The delivery.** **(expensive to reverse)**
- **Proposed default:** `DeliveryTarget::Hull { design, title, berth }` and the
  `tacsim.hull_provenance` record (§4.7); ships to the berth gallery as docked hulls, vehicles to the
  yard's workshop room as berthed hulls; parts with `Manufactured` provenance.
- **Alternative:** deliver a "hull kit" lot and a separate `+yard/launch` that lays the hull up from
  it.
- **Why:** doc 08 §10.2's step 7 ("transfers the output into faction title and physical custody") is
  one act, and a kit lot is a hull that can be lost in a hold. Expensive as Q-38-12 says: every later
  claim reads the provenance.

**Q-39-19 — The player verb, and who may order.**
- **Proposed default:** `+yard`, `+yard/quote`, `+yard/order`, `+yard/cancel` at a port with a yard,
  fronting `@wo` as `+bay` fronts the bay; any principal with an account and the money may order any
  registered design the yard's class list takes, an organization through an officer with the
  capability; `@wo` stays staff.
- **Alternative:** expose `@wo` to players.
- **Why:** `@wo/draft` names endpoints by number and a recipe by pack and version; `+bay` is the shape
  players already have.

**Q-39-20 — What the yard takes.**
- **Proposed default:** the Stickney yard builds class hauler, vehicle, walker and drone; capital
  hulls (the corvette, the Halcyon) and the relay station are out of scope until a space-side yard
  exists (doc 35's track).
- **Alternative:** every registered design.
- **Why:** a capital hull is delivered into the space track's berth and bubble, which this chain does
  not touch.

**Q-39-21 — The pools.**
- **Proposed default:** §4.7's floors (the Trust two Drays, Occator two rigs and one escort) with
  house orders from the house's capital; `NO_HULL` names the order and its due day.
- **Alternative:** Q-18-70's shrink stays and the pools are the seed.
- **Why:** a pool that only shrinks ends the programme in eight captains; production restock is what
  doc 18 P5 owed.

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

## 6. The plan: cuts IN.0–IN.10

The rulings of 2026-11-03 make this the implementation order. Each cut lands as one PR over E7's
engine, built from the rulings its row names and tested through the production command paths, with
its story at both ground sizes where the ground is in it (Q-38-29); each cut's note goes to
`39-cut-notes.md` as the GC and SF trains did. **Dependencies:** IN.1 ∥ IN.2 ∥ IN.3 may start now;
IN.4 after IN.3; IN.5 after IN.1 and IN.2; IN.7 after IN.1 and IN.2; IN.6 after IN.4 and IN.5; IN.8
after IN.2 and IN.5; IN.9 after IN.8; IN.10 last. Two cuts carry the expensive-to-reverse marker
(IN.1's part identity, Q-39-3; IN.8's delivery target, Q-39-18) and are built exactly as ruled. Doc
41 may start once IN.1, IN.2 and IN.5 have landed (the lot shapes, the fees and the stores it needs).

| Cut | Rulings | What | Acceptance (the story) |
|---|---|---|---|
| **IN.0** | — | this document, its README row, Draft 0.2 with the rulings (**done**, PR #347) | — |
| **IN.1** the catalogue | Q-39-1, Q-39-2, Q-39-3, Q-39-8 | goods v5: the part rows keyed by hardware row, masses, references; lane v20's ladder; `ShippedParts::commodity` and the fit check on the row; `hold_takes` counts parts; the census pin on `.mint_origins(` and the receipt-kind pin | `+bay/strip` a cannon off an escort at Stickney: `+market/list` shows the lot with 2.4 t; the hold's load line moves; a heavy's `+bay/fit` takes it; the pins pass |
| **IN.2** the cost model | Q-39-9, Q-39-10, Q-39-24 | `FacilityCostDef` on the port rows, the daily and slot-day rates on the facility's policy half, `+bay` quotes printing the floor, the multiplier clamped at it, the Tycho Mutual row | `+bay <hull>` at Ceres and Stickney prints the quote and the floor; an operator multiplier under the floor is refused by the validator |
| **IN.3** the refine | Q-39-4, Q-39-5, Q-39-6, Q-39-8 | the `refinery` row at Kasei; `MillDef.kind: refine`, the family rule relaxed for it, Kasei's row (5:4, 20 t, 40 t a day), the seed as opening stock, the bid's refresh from the row's consumption | concentrate sold into Kasei's bid at Stickney is refined on the next fires and Kasei's alloy ask restocks by the milled units; the matter pin and the mill's drift check hold |
| **IN.4** the works | Q-39-4, Q-39-5, Q-39-6, Q-39-7, Q-39-23 | `works` rows at Stickney (the Yards) and Ceres (Freeport Works, the new org); `content/industry/recipes-v1.json` loaded by the deploy template; the `fittings` recipe; the NPC house arm running house orders on the port tick under the capital clause; the Stickney carbon-ore bid | carbon ore mined on Kerwan and sold at the Exchange is hauled to Stickney and sold; the Yards' fittings ask restocks from a delivered house order; a player's `@wo` at the Ceres works does the same from the hold |
| **IN.5** the part lot | Q-39-11, Q-39-12, Q-39-13 | `Replace{lot: Store}` from the operator's storefront at the catalogue price, `NO_PART`, the seeded part rows with receipts, `PartProvenance.origin`, `+bay/fit` on every origin | GC.8's refit run pays 480 of labour and 28,000 for the cannon; the seventh replace at Ceres refuses `NO_PART`; the lot's provenance reads `Seeded` |
| **IN.6** the part recipes | Q-39-4, Q-39-22 | the mount, sight, pod and plant recipes at the works; outputs at grade 9 with `Manufactured` provenance; the Stickney strategic-ore bid; instrument boards from Luna | 2 t of strategic ore, 1 t of boards and 3 t of fittings become ten `visual_mk1` lots at Stickney; one is fitted to a scout by `+bay/fit`; the seeds retire |
| **IN.7** the mass budget | Q-39-15, Q-39-16, Q-39-17 | per-design `structure_kg_per_point` and `fittings_kg_per_point`; repair draws both; the three constants retire; scrap as §4.6 at a workshop (the operator's stock, the credit, the fee) and on a site (the capped yield, the room check) | a whole escort scrapped at Stickney credits ~1,300 cr and 2.9 t to the Yards' pool with its four parts in the hold; a Dray credits 10,500 after the fee and 79 t to the pool; a Dray's remains on Urvara (512) and an escort's on Ahuna (256) yield 40 t and 2.9 t into the hold, the full hold refusing |
| **IN.8** the yard | Q-39-14, Q-39-17, Q-39-18, Q-39-19, Q-39-20 | the `yard` row at Stickney; `DeliveryTarget::Hull`; `tacsim.hull_provenance`; the bill from the design (σ, φ, days, parts, core); `+yard`, `/quote`, `/order`, `/cancel`; the hull laid up and titled; capital hulls refused | a captain with the money and a hold of alloys, fittings and four part lots orders an escort at Stickney; two IC days later it stands in `p11_service` titled to them with a fresh mark; `+bay` quotes it clean; `+bay/stow` puts it in the Dray-P's bay; the bot types it |
| **IN.9** the pools | Q-39-7, Q-39-21 | the Trust's and Occator's floors and house orders; `NO_HULL` names the order | eight charters leave no free Dray; the ninth captain hears the due day; fourteen IC days later a ninth Dray stands in Ceres' gallery under the Trust's title and the desk enrols them |
| **IN.10** the matter story and the bot | Q-39-8, Q-38-29 | one story at 256 and 512: mine → sell → mill → refine → works → part → fit, and ore → alloys → hull, with Σ mined = Σ (pools + lots + bills + losses) asserted; the bot's `+yard`, `+market/sell` and `+bay/order replace` vocabulary | the story passes at both sizes with the bot typing the player's half; the sum holds to the tonne |

## 7. What the first cuts need

Content and schema the cuts author as they land, listed once so no two cuts invent them twice:
lane v20's rows (the ladder, the part stock, the three facility rows and their `FacilityCostDef`s,
the Freeport Works org, the Stickney bids; IN.1–IN.4), the goods v5 rows (IN.1),
`content/industry/recipes-v1.json` and its path in the deploy template (IN.4), the new rooms in
Stickney's and Ceres' atlas entries (IN.3, IN.4, IN.8), the `tacsim.hull_provenance` schema (IN.8),
the test list the pins become (`shard_9/mill.rs:2508`'s successor and the receipt-kind pin, IN.1);
and, for doc 41, the lot shapes (§4.5), the facility fees (§4.4) and Tycho Mutual's row (Q-39-24)
as the inputs doc 38 §8 promised it, available once IN.1, IN.2 and IN.5 have landed.

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
| 2.2.2 | a ten-unit part output is one lot; fitting one consumes all ten; a split child is untraced; `bay_refit` refuses `quantity != 1` | **CONFIRMED** | one lot per output line with `units × quantity` (`provider.rs:3780–3796`); `consume_product` removes the whole lot (`pool.rs:1525–1539`); `bay_part.rs:333`; `split` writes `Lineage::Split { parent }` (`pool.rs:1454`) and `tacsim.bay_parts` is keyed by lot id (`act.rs:722–726`) | §4.11: parts are never batched, split or pooled; Q-39-27 |
| 2.2.3 | the 7-IC-day reservation TTL expires a 14- or 20-day order's materials before delivery | **PARTLY** | the deadline is `now + RESERVATION_TTL_SECS` at submit (`record.rs:34–36`, `pool.rs:198–204`, `provider.rs:3458`) and `wo_advance` restamps nothing (`:3590–3618`) — CONFIRMED; but `checked_consume` requires `Held` only (`mush-core/src/resource.rs:456–462`) and never compares `expires` with `now`: the order fails only once a sweep has expired the reservation (`pool.rs:1354`, `mush-industry/src/money.rs:623`), so the failure is real and its timing is the sweep's | §4.12 escrow: the deadline is the order's duration plus the TTL; Q-39-28; IN.3 |
| 2.2.4 | the E4 hold is one minor unit an input unit, and submission requires exactly that hold | **CONFIRMED** | `quote_price` sums input units (`workorder.rs:1210–1214`); `order_price` (`money.rs:210–214`); `place` holds `order_price` (`:333–345`) | §4.12: the hold is priced from `FacilityCostDef` through `order_price`; Q-39-28 |
| 2.3 | a seeded part row mints one lot of `qty` with no provenance, the clerk dissolves it, a sale makes a new tranche lot with no provenance, and a store debit outside the market pages drift | **CONFIRMED** | `stock.rs:420` (one lot of `qty`); `:648–670` (`InventoryVerb::Dissolve`); `pool.rs:1215–1224` (`Lineage::Tranche`, a new `buyer_lot`); `stock.rs:505–510`, `:728–737` (`drifts`) | §4.11: part rows are never pooled; the clerk mints one-unit lots with `Seeded` rows and lists them as lot asks (`Representation::Lot`, `mush-market/src/record.rs:129`); Q-39-27 |
| 2.4 (1) | 19 shipped units, not 14 | **CONFIRMED** | `content/units/` has 19 files; Q-39-14 miscounted | Q-39-14's text corrected in §4.7 |
| 2.4 (2) | `UnitClass` has no `Light`, every tank is `Armour`, and `tv_prospector` and `tv_escort` are both `Vehicle`, so `tacsim.part.power.<class>` cannot be derived from the class | **CONFIRMED** | `definitions/unit.rs:749–772` (Armour, Walker, Drone, Hauler, Vehicle, Station, Capital); `class_of` defaults to Armour or Walker by locomotion (`:971–979`); the nine authored classes are three `armour`/`capital`/`hauler`/`drone`/`station`/`vehicle` rows and ten defaults | §4.1 the plant row on the power component; §4.8 the class table with `armour`; Q-39-35 |
| 2.4 (3) | `power.walker` and `relay` have no recipe; `pod_eccm`'s loss is 5.8 t | **CONFIRMED** on the recipes; **PARTLY** on the loss (the row covered both pods: 5.4 t for ECM at 160 kg, 5.8 t for ECCM at 120 kg) | §4.3's table | the two recipes added, the pods' rows split |
| 2.4 (4) | after SF.R15 the Dray has 3,950 points and the escort 3,475 | **REFUTED** | PR #339 (`c83c2eff`) changed `dray.json`, `dray_p.json` and `corvette.json` by `heat_max` 1000 and `heat_dissipation_per_s` 30 only (and the corvette's drive); at `c83c2eff` the Dray sums 570 armour + 1,250 internals = 1,820 and the escort 2,320 + 670 = 2,990 | the figures stand |
| 2.4 (4b) | `scrap_hull` counts the heat cells that the bay quote filters out | **CONFIRMED** | `workshop.rs:745–752` sums every `CellAddr::Section` cell above its minimum; the heat-sink cell is born at its maximum (`definitions/unit.rs:1678–1683`, `init: dissipation`); `jobs.rs:239–246` filters `HEAT_CELL` and `HEAT_SINK_CELL`; after SF.R15 the Dray carries them (90 phantom points over three sections) | §4.8: the per-design figures count structural cells only; IN.8 retires the sum |
| 2.5 | `DeliveryTarget::Hull` cannot run inside `wo_deliver`: a recipe needs an output, the prepare has no world, and the lay-up paths need logistics and an interior the simulator units lack | **CONFIRMED** | `PinnedRecipe::new` refuses empty outputs (`workorder.rs:349–353`); `wo_deliver` "is a prepare: no `&World`, no `Ctx`, no `EconomyStore`" (`provider.rs:3637–3640`); `hull::template_of` (`hull.rs:317–323`) and `vehicle::template_of` (`vehicle.rs:181–186`) require both blocks; thirteen simulator units carry neither; the foundry precedent consumes the order's output lot by its `workorder.output` lineage in a tacsim action (`relay_foundry.rs:375–405`, `provider.rs:4086–4093`) | §4.13 the kit lot and the commissioning record; Q-39-33 revises Q-39-18; IN.9 |
| 2.6 | Stickney builds and Ceres enrols, so IN.9's ninth Dray teleports; a ninth programme row crashes the applier; a changed workshop capability set refuses the apply | **CONFIRMED** | `hull_is_free` needs the hull docked on the programme port's domain (`coop.rs:736–739`); `vehicles_line` counts the shed (`vehicle.rs:927–934`); `hull::preflight` refuses `row.n >= pool` (`hull.rs:465–471`), `vehicle::preflight` the same (`vehicle.rs:306–312`); `workshop::preflight` refuses drift in berths or capabilities (`workshop.rs:52–60`); the Trust's 250,000 (lane `:2597`) posts each 25,000 stake (`coop.rs:1629–1663`) | §4.7 and §4.13: built hulls live in an owned-hull register, never in a programme row; the house ferry; Q-39-34 revises Q-39-21; the yard gets its own room |
| 3.1 | a yard-built vehicle in `p11_service` cannot be stowed, is not held, and has no carrier to draw from | **CONFIRMED** | `stow` needs `record.berth` (`bay_work/mod.rs:1560`); `held` reads an open order, a berth or a scrap only (`:442–453`); `source_carrier` needs the berth row's carrier (`:611–629`) | §4.13: the commission writes a yard berth row; `+bay/stow <hull>=<carrier>`; Q-39-33 |
| 3.2 | a titled, non-leased unit cannot be bought out around, boarded, launched or sortied, bricks a carrier without logistics, and a built ship has no hold vault and no helm | **CONFIRMED**, **PARTLY** on the hulls line | `rigs_in_bay` gives a non-lease unit slot 0 (`vehicle.rs:1058`) and `bay_slot_word` turns it into the buyout's and refit's refusals (`buyout.rs:407–409`, `refit.rs:180–182`); boarding reads `tv_leases_of` (`boarding.rs:188–198`), the launch's `lease_gate` takes a `TvLease` (`plans.rs:142–149`), the sortie host needs `tv_lease_on` (`sortie/surface.rs:89–97`); `carried_rig` needs logistics (`vehicle.rs:1189–1206`); the hold vault opens at enrol (`coop.rs:1669`); `helm_of` is the charter, else the programme map's hull titled to `Object(pilot)`, `.min()` (`coop.rs:597–622`); `hull_view` searches programme and yard rows (`ship.rs:318–329`). `hull_line` reads a player-titled row as graduated, and only an org-titled non-Trust row as Missing (`hull.rs:1195–1205`) | §4.13 the owned-hull register and the title view for rigs; Q-39-33 |
| 3.3 | nothing auto-delivers `AwaitingDelivery`; the mill never refreshes a bid; Kasei's `metals` bid needs `refresh` | **CONFIRMED** | `wo_advance` stops at `AwaitingDelivery` (`provider.rs:3596–3600`); `mill_step` touches no bid (`mill.rs:935–1000`); the drip serves `refresh` rows only (`stock.rs:906`); Kasei's bid has none (lane `:2713`) | §4.12 the house arm delivers; §4.3 Kasei's bid refreshes from the refine row; Q-39-28, Q-39-31 |
| 3.4 | `ShippedParts::commodity` reaches only `bay_refit`; `bay_refit::moves` diffs slot names; its removed part is minted at grade 0 with no provenance | **PARTLY** | `part_commodity` is called by `bay_work` (`strip.rs:48`, `:289`), so the first clause is REFUTED; `moves` diffs `ComponentId`s (`bay_refit/config.rs:235–252`) CONFIRMED; the removed lot takes `c.parts.grade(removed)` (`bay_refit/provider.rs:538`; the trailing `0` is lineage depth), so "grade 0" is REFUTED and "no `tacsim.bay_parts` row" CONFIRMED (`:545–552`, an `industry/bay_refit` origin; doc 36 GC.8b already says so) | §4.11 states the one identity both paths share; IN.2 |

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
| 10 | the Trust cannot fund a ninth Dray and the build is at Stickney | **CONFIRMED** | 250,000 − 7 × 25,000 = 75,000 (lane `:2597`, `:2614`; `coop.rs:1629–1663`); as 8.1 row 2.6 | §4.7 funding from buyouts, the ferry, the register; Q-39-34 |
| 11 | the scrap mechanism narrows the accepted ceiling's scope, and 40 t at the 520 bid is 20,800 | **CONFIRMED** as a scope question | §4.6 exempts parts; 40 × 520 = 20,800 > 18,000 | raised to the owner as Q-39-25, not decided here |
| om. 1 | a new unit's magazines are born full | **CONFIRMED** | `definitions/unit.rs:1718–1725` (`init: rounds`) | §4.13 and Q-39-36: delivered empty |
| om. 2 | `power.walker` has no recipe; "light" is not a class | **CONFIRMED** | as 8.1 row 2.4 | §4.1, §4.3; Q-39-35 |
| om. 3 | adversarial acceptance is missing | taken | — | §6's stories |
| om. 4 | identity, units and escrow should precede their consumers; the first production story is too late | taken | — | §6 reordered; the first story is IN.5 |

**What neither review saw.** The yard's room: Draft 0.1 put the `yard` capability on `p11_service`
beside the workshop, which `workshop::preflight` refuses on a kept world (8.1 row 2.6); the yard gets
its own room. `checked_consume` ignores the deadline (8.1 row 2.2.3), so an overdue reservation is a
sweep's business, which the escrow rule in §4.12 makes explicit. And `class: "part"` goods must be
refused by the pool verbs outright, or every lifecycle rule in §4.11 has a hole the size of
`@inv/dissolve`.
