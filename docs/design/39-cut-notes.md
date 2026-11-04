# 39. Cut notes — the industry train

One `## IN.n` heading per cut of [39-industry-production-chain.md](39-industry-production-chain.md) §6, in the
order they landed: what was built, the story that proves it, and where the PR departed from the row. Each note
records its cell's summed-minutes delta (doc 43 rule 2.3 #14).

## IN.1 — accounting and identity

Rulings: Q-39-1, Q-39-2, Q-39-3, Q-39-8, Q-39-26, Q-39-35 (expensive to reverse: built as ruled).

**Built.**

- **Goods v5** (`content/market/goods.json`): the thirteen part rows of §4.1, spelled `tacsim.part.<kind>.<row>`
  (`weapon.cannon_105` … `plant.hauler_mk1`), `class: "part"`, no family, counted and quoted `each`, the row's
  kilograms as `si.mass_kg`, packed volume 1 L/kg for mounts and plants and 2 L/kg for crated instruments, the
  §4.1 references. `GoodClass::Part` and `PART_PREFIX`; the loader refuses a part with a family, a unit other than
  `each`, a spelling other than `<kind>.<row>`, and a non-part carrying the prefix. Labels name the hardware
  (`105 mm cannon mount`); the ruled ids keep `weapon` as the kind word, which no player line prints.
- **The plant kind** (Q-39-35): `content/plants/plants.json` (`tacsim.pack.plants`), the tenth definition kind
  and the eighth a pack carries (`PlantDef`: label, `mass_kg`, `states`), minted before the units like the cores.
  `ComponentRole::Power` gains a required `plant`; the catalogue build refuses a missing plant and a power role
  whose states are not its plant's. Every chassis names one: light (the prospector, scout, drone, target),
  vehicle (the five line tanks, the escort, the heavy, the carrier), walker (the bipeds), hauler (the Dray, the
  Dray-P). The bound power capability is the role's states as before, but the unit's rules carry its
  authored components, so every unit definition moves by content (the line tank's pin `6bfa9bdb…` →
  `c85d03ac…`; pre-launch, nothing persisted to migrate). An attempt to keep `plant` out of the bound rules
  broke every check that rebuilds a unit's rules from its spec (the Conduct registry's evidence, the core
  replacement family) and was dropped. The unit packs were re-signed, not re-versioned (SF.R15's precedent).
- **Part identity** (Q-39-3): `Catalog::parts` indexes each component definition `{unit}.{slot}` to its
  `PartRow { commodity, row }` — the hardware row the slot names and its part good, only where goods v5 has the
  row. The standing-order verbs (`+ship/order buy|sell`) do not trade parts: a part is a lot, never a
  pool quantity, and the unknown-good list names bulk goods only. `ShippedParts` reads that index (the bay refit's catalogue); `hardware::bay::part_of` serves strip and
  scrap. `PartProvenance.definition` is now the hardware row's reference, and `+bay/fit`'s check matches the
  slot's row, so a line tank's cannon fits a heavy. The bind's component definition id is unchanged.
- **The tonne ledger with kilogram computation** (Q-39-26, §4.10): `BayHull.bale: Option<Bale { alloys_kg,
  fittings_kg }>`; `Bale::draw` opens an unopened bale with a tonne of each good and tops a line up by whole
  tonnes only when it runs short; `BayQuote::bale_need` gives the kilograms (fittings at the tariff's 100 points a
  tonne, 10 kg a point; alloys 0 until IN.8's per-design figures). `order_goods` returns the hold's tonnes and
  the bale after; the order carries `bale_after`, which lands with the order's completion. The quote prints the
  bale it leaves. Rounding per act: a repair rounds **up** for the hold and is exact for the hull (the bale);
  workshop scrap mints **down** (unchanged); remains scrap under a tonne is no longer refused: it breaks up with
  nothing minted, a declared loss (`SCRAP_LOSS_PREVIEW`, `SCRAPPED_LOSS`). Bills round up when IN.9 makes them.
- **`hold_takes` counts parts**: every lot, a part by its catalogue mass, in `strip::hold_takes` and the bay act's
  in-bundle mirror. A part never pools, so the parts already aboard are read from a new **`inventory.lot_tally`**
  family (units held in lots per endpoint and commodity, keyed by the pool key, written by every lot verb in
  `mush-inventory` in the bundle that moves the lot, absent at zero). `port_tick::read_pool`/`read_pool_in` add it
  for a part, so `+ship/hold`'s load cells list a part aboard (to the tenth of a tonne) and the capacity check
  weighs it; zero part rows are not listed. The strip's line names the part and its mass (`1 105 mm cannon
  mount, 2.4 t (lot)`).
- **The census pins** (Q-39-8): `shard_9/mill.rs::sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter`
  counts `.mint_origins(` (once, `port/bay_work/act.rs`); `in1_the_receipt_kinds_a_mint_may_carry_are_closed`
  closes the receipt kinds to `port.stock`, `remains_scrap`, `bay_strip`, `bay_scrap`.
- **Lane v20** (the ladder, Q-39-1/32): Kasei's alloy ask 450, Stickney Yards' fittings ask 520, the Co-op's
  fittings bid 600; relay seed v11 re-signed to require lane v20. `strip_ladder` puts the three prices back in
  both strip chains (`tests/support/sites.rs` through `strip_workshops`, `relay_registry.rs` through
  `strip_urvara`); every lane and relay pin moved. IN.4's facility rows join v20 without a second bump (the
  hand-over note `in1-lane-v20.md` in the wave's notes folder).

**The story.** `shard_2/in1_accounting.rs::in1_a_cannon_stripped_at_stickney_fits_a_heavy_and_a_patch_opens_a_bale_once`
(shard_2, the shard with the most room, doc 43 §10.2): a captain's Dray-P sails to Stickney Yards with a titled
spare escort and a titled heavy in its bay; at the Service Counter `+bay/strip` takes the escort's cannon off as
one lot of `tacsim.part.weapon.cannon_105` whose provenance is the 105 mm row; `+ship/hold` lists the mount at
2.4 t; the heavy's wrecked cannon is replaced from that lot by `+bay/fit`; a one-point `+bay/order …=armor`
opens the heavy's bale (a tonne each of alloys and fittings from the hold, 1,000 kg and 990 kg left) and a
second one-point patch draws nothing (980 kg left). `in1_the_bale_opens_once_and_tops_up_by_whole_tonnes` pins
the rule; `hardware::bay::tests::in1_a_part_is_its_slots_hardware_row_and_a_core_is_none` the identity.
Seeds (doc 43 rule #4): the two hulls' title and bay placement, the heavy made by `@unit/create`, the damage,
and the hold's two tonnes of each good.

**Departures from the row.**

- *`+market/list` shows the lot with 2.4 t*: a hold lot is not a listing until IN.2's lot-transfer sale and lot
  asks, so the 2.4 t is shown where the lot is — the strip's line and `+ship/hold`'s load cells. IN.2 takes the
  listing half.
- *Two more plant rows*: the corvette, the Halcyon and the relay station carry power roles, and the ruling
  refuses a missing `plant`, so `plants.json` carries `capital_mk1` and `station_mk1` beside the four ruled rows,
  unpriced: goods v5 has no row for them, so they are no part and come off no hull (Q-39-20's list).
  Their masses are provisional and unread. The Halcyon's relay socket stays no part.
- *Alloys per point*: IN.1 opens the bale with both goods as §4.10 rules, but the per-point alloy figure is IN.8's
  partition; until then a repair consumes fittings only (10 kg a point) and the alloy tonne waits in the bale.
- *The market's own capacity reads* (a purchase into a hold, `mush-market`'s `trade.rs`/`lot.rs`) still read the
  pool alone; no part reaches a hold through the market before IN.2, which adds the tally there with the lot
  sale. The bay refit's and the core input's custody terminals do not move the tally (their write sets are
  kernel-declared and their lots live in facility stores, never a hold).
- *The four-digit bot parser* is FR.0's (the session brief), not carried here.

**Cell delta.** shard_2: _filled from the PR's JUnit_.

## IN.4 — the cost model (Q-39-9, Q-39-10, Q-39-24, Q-39-32)

**Built.**

- `mush_industry::cost::FacilityCostDef { capital_major, lifetime_years, upkeep_permille, staff,
  wage_major, margin_permille }` (§4.4) with its bounds (`check`), the daily cost, the slot-hour (exact as a
  fraction; callers multiply before they divide), `fee_minor` (work units × slot-hour × (1 + margin)),
  `floor_minor_for_secs` (margin 0) and `floor_multiplier_permille`. All integer, minor units: each capital
  term rounds up to the minor unit; a fee or a floor rounds up to the **whole credit**, the grain a quote
  prints.
- `FacilityRecord` carries `cost: Option<FacilityCostDef>` on its policy half (`skip_serializing_if` none, so
  an unpriced facility's stored bytes are unchanged). `with_authored_cost` at open; `with_cost` re-prices
  and bumps `policy_revision` as `with_owner` does, so a draft against the old numbers is refused at
  submit. `same_identity` (object, owner, capabilities, slots) is what `install_authored_facility` and the
  workshop's preflight now refuse drift on: a changed cost on a kept world re-prices in place instead of
  stopping the applier.
- `money::order_price(world, order)` is **`fee(order)`** — the order's work units (`quote_duration`) at the
  facility's slot-hour and margin — when the order's facility carries a cost model, and `quote_price`'s one
  minor an input unit otherwise (the relay foundry and the medical rooms are unpriced, so their holds do not
  move). `place` holds it; the relay foundry's `paid` check reads it with the world. `@wo/quote` prints
  `fee N cr` for a priced facility.
- `WorkshopDef.cost` (lane): both workshops author §4.4's workshop row (1.5 M over 20 years, 3 % upkeep, six
  at 150, 25 % margin = 1,228.77 cr a day). The validator refuses a `multiplier_permille` under
  `floor_multiplier_permille(berths, tariff.labour_hour_minor())` — the berth-hour over the tariff's own
  labour hour (100 cr, `WorkshopFacilityTariff::labour_hour_minor`): 256 ‰ at Ceres (two berths), 128 ‰ at
  Stickney (four). Both sit under the tariff's 500 ‰ minimum, so the shipped multipliers (1,200 and 1,000)
  pass; the clamp bites only on a costlier row. The tariff's rows are left as they are (Q-39-9).
- `+bay <hull>` prints a `floor` line under `all` whenever the port's workshop is priced: the work's
  workshop-time hours at a berth's slot-hour, before margin. A carrier bay docked at the port prints the
  same floor (the labour is the workshop's crew either way; the bay's ×2 time factor is not charged to it).
- `PortDef.recovery: RecoveryDef { operator, slots, cost }` — **Tycho Mutual's row** at Ceres (4 M over 15
  years, 4 % upkeep, six at 150, one slot = 2,068.96 cr a day) with `retrieval_fee_minor(days)` = the days'
  work units at the margin (one day 2,587 cr). A cost row only: no room is opened; doc 41 adds the passage's
  propellant and quotes it on the contract.
- **The mills are withdrawn from the fee model** by construction: a `MillDef` carries no cost and
  `deny_unknown_fields` refuses one; §4.4's 360-a-day crusher shed stays a coverage figure in the doc.
- **Lane v20** (IN.1's bump, PR #370; this cut stacks on it and adds its rows to the same version): the lane
  re-signed with both cuts' rows; relay v11 unmoved (it requires the lane by version). `strip_urvara`
  (`relay_registry.rs`) and `support/sites.rs::strip_workshops` remove the `recovery` row; the `cost` blocks
  go with the workshops.

**Tests.**

- `mush-industry` `cost::tests` (4): §4.4's daily costs exactly (yard 3,261.20, works 1,693.16, workshop
  1,228.77, recovery 2,068.96); a six-unit fittings order 265 cr; the floor multipliers 256 / 128 ‰; bounds.
- `mush-industry/tests/e7r2.rs` `in4_order_price_is_the_fee_from_the_facilitys_cost_model`: facility #7
  re-priced with §4.4's works row; a draft against the unpriced record is refused at submit; a six-work-unit
  order's `@wo/quote` prints **`fee 265 cr`** and the submitted hold is 26,500 minor.
- `tacsim` lib `in4_an_operator_multiplier_under_the_floor_is_refused` (a payroll of thirty lifts the floor to
  1,006 ‰: 1,000 is refused with its sentence, 1,006 accepted) and
  `in4_the_recovery_row_is_bounded_and_prices_a_retrieval`.
- `shard_1/bay_gc8.rs`: `gc8_who_may_have_a_hull_worked_on` asserts the **Ceres** quote's floor (the unjam's
  two hours of a Ceres berth = 51.20, billed 52 cr); `gc8_the_refit_run_at_stickney_…` asserts the
  **Stickney** workshop quote's floor against the cost model over the quote's hours, under the tariff's
  labour at 1×. Both through `+bay` on the production command path.

**Divergences from §6's row.**

- **`+works/quote fittings` does not exist** — `+works` is IN.3's verb. The 265 cr figure is asserted through
  `@wo/quote` and the submitted hold, i.e. `order_price`, at a facility carrying §4.4's works row (the works
  rows themselves, with their rooms, are IN.5's). 264.56 cr exactly; 265 because a fee rounds up to the
  whole credit.
- **No 256/512 story**: the cut touches no site; the acceptance is at the two ports.
- **Lane v20 is IN.1's bump** (agreed with `in1-accounting-identity`): this PR stacks on #370, so #370 merges
  first; no second version bump.
- The "multiplier clamped at the floor" is a refusal by the validator at pack load (the house's "refused by
  the validator"), not a silent raise.

**Cell delta (rule #14).** No new story: two assertions in the GC.8 stories and one `@wo` test in `e7r2`
(0.08 s for the binary locally), three tacsim lib unit tests (≈ 0 s). Local `shard_1` `gc8_` pair: 16.7 s,
unchanged by the cut. The PR run's JUnit delta is to be read off the draft PR's CI.

## IN.2 — the component lifecycle (Q-39-11, Q-39-13, Q-39-27)

**Built.**

- **A part is indivisible** (Q-39-27 "held"): `mush_inventory::IndivisibleGoods`, a composition seam in the
  `PoolCapacity` shape (`InventoryModule::with_indivisible_goods`), answered by tacsim's `port_tick::PartGoods`
  (`class: part`, through `is_part`). `split`, `merge` and `dissolve` refuse a part with
  `InventoryError::Indivisible` ("that is a part: one identified lot, never split, merged or dissolved"), and
  `@inv/split`, `@inv/merge` and `@inv/dissolve` refuse it at once, before staging anything.
- **`PartProvenance.origin`** (Q-39-13): `PartOrigin::Stripped(Stripped { parent_hull, parent_schema, slot,
  claim, stripped_by, stripped_at_ic, scrapped })` (today's fields, moved), `Manufactured { order, facility }`
  and `Seeded { receipt }` (the stock issue's key text). The row keeps `lot`, `definition`, `condition`,
  `legality` and `fitted_into` at the top for every origin; `+bay/fit` accepts all three ("traced" is a row of
  any origin); `check` refuses a hot part that is not `Stripped`, and the `Hot` act flags only a part stripped
  off that hull. Pre-launch, no stored row migrates.
- **The lot hold** (Q-39-27 "reserved", "sold"): `inventory.lot_holds`, one exact lot held for one holder text
  (`mush_inventory::lot_hold`): `hold_lot`, `release_lot`, `relocate_held` (the lot itself, the same id, grade
  and lineage, to another endpoint) and `consume_held`. Every ordinary lot verb (split, merge, dissolve,
  relocate, `consume_product`, the commission) refuses a held lot.
- **The bay's custody**: an order that fits a part from a lot (`+bay/fit`, and `+bay/order replace` when a lot
  is named) owes a **`Hold`** act at its take (`bay_holder(hull, order)`); `finish` waits for it, the draw
  consumes the lot through `consume_held`, a cancelled order owes a **`Release`**, and a hold refused every try
  cancels the order ("the part could not be held for the order …"). `+bay/fit` refuses a lot a sale or another
  order holds ("That part is held for a sale or another order.").
- **The lot-transfer sale** (Q-39-27 "sold"): `Supply::Lot { …, lot: Some(id) }` (skipped when `None`, so a
  pooled listing's bytes are unchanged), stocked with `@market/vendor stock … endpoint=<vault> lot=<lot>` (the
  lot's own commodity, grade and quantity). Its `reserve` leg holds the lot under `listing/<id>`; `consume`
  hands the lot itself to the buyer through `relocate_held` (`PaidLot.lot`, matched against the paid entry);
  `release`, the lapsed-hold release and the sweep's reconcile read the lot hold. The governed-endpoint rule
  (lead 2e) admits an exact-lot sale into a hold, behind the capacity pre-check, which now reads a part's lot
  tally (`lot::held_balance`), so a part sold into a Dray weighs its catalogue mass there.
- **The seeds** (`port/stock.rs`): a part row in a vendor's `stock` mints `qty` one-unit lots
  (`seed_lot_id(issue, n)`, the first the issue's own lot) with their `Seeded` rows in the receipt's bundle
  (the offer row carries the part's hardware row, `part_row_of`); the clerk never dissolves them, the pooled
  ask loop skips part asks, and `part_ask_step` lists each seeded lot still at the storefront, unheld and
  unlisted, as its own lot ask (`Supply::Lot` naming it, quantity 1). The clerk's one mint is now
  `mint_origins` (one row of `qty` for bulk, as before).
- **`output_lots`**: `mush_industry::OutputLots { Pooled (default), Each }` on `PinnedRecipe`
  (`with_output_lots`) and on tacsim's pack `RecipeDef`, skipped when pooled, so no published pack digest and
  no pinned order moves.
- **The part commission** (§4.11, round 2's 39-3): `OwedKind::Commission { source, lineage, lots }` under the
  bay ledger's ordinal replay; `CommissionSource::{WorkOrder { order }, BayRefit { order }}` names the industry
  lineage it consumes (`workorder.output` `wo/<order>`, or `bay_refit` `<order>`) and the row it gives
  (`Manufactured` naming the order, or `Stripped`). The prepare consumes the industry lot and lays it back as
  one-unit lots (`InventoryFamilies::commission_lots`: `Lineage::Split` of the source, the same quantity,
  nothing minted, the census staged once) and inserts each row in the same bundle. `bay_work::owe_commission`
  is the producer's call.

**The story.** `shard_2/in2_lifecycle.rs::in2_a_sight_stripped_at_stickney_is_sold_bought_and_fitted_to_a_scout`
(shard_2, after IN.1's bay story): two captains sail Dray-Ps to Stickney Yards, the seller with a titled spare
escort, the buyer with a titled scout. `+bay/strip` takes the escort's sight off as
`tacsim.part.sensor.visual_mk1` with a `Stripped` row; `@inv/split` on it refuses at once; the seller, a broker
of the yard, lists that lot on the yard's board (`@market/vendor stock … lot=`), and while it is listed
`+bay/fit` refuses it; the buyer's `+market/quote` and `+market/accept` hand **the same lot** to the buyer's
berthed hold with its row unchanged; the scout's wrecked sight is replaced by `+bay/fit`, the order holding the
lot until its draw; the row then reads the lot with both hulls (`parent_hull` the escort, `fitted_into` the
scout). The seller's stripped coaxial mount, its claim vacated, refuses at the fit as hot. Unit and crate
tests: `mush-inventory` `lot_hold::tests::in2_a_part_is_one_lot_held_and_handed_over_whole` (split, merge and
dissolve refused; one holder; the hand-over; the fit's consume; a commission of three);
`act::tests::in2_every_origin_is_traced_and_only_a_stripped_part_is_hot`,
`act::tests::in2_a_commission_names_its_lineage_and_mints_one_unit_parts`,
`stock::tests::in2_a_part_seed_mints_a_distinct_derived_lot_a_unit`. Seeds (doc 43 rule #4): the two titles and
bay placements, the seller's `@org/grant … economy.account.post` over Stickney Yards, the scout's damage and the
vacated claim (GC.8's `vacate_claim`).

**Departures from the row.**

- **The custody is the inventory's lot hold, not `BayPartCustody`.** `inventory_bay_parts` is one of the twelve
  families the kernel's bay-refit protocol names (`mush-core` `terminal_capacity/bay_refit.rs`): any write to it
  must be one of industry's three refit actions joined to exactly one industry bay order, so tacsim's bay act
  cannot write it. The lot hold gives the same guarantee (no move, sale, split or consume before the draw) and
  serves the market's lot-transfer sale too; `+yard` (IN.9) takes it the same way.
- **The commission has no producer yet.** The act, its replay and `owe_commission` are built and pinned by unit
  tests; the work order's commission is owed by the works' arm or `+works/collect` (IN.3/IN.7; IN.7's row
  carries the commission's end-to-end story and its restart), and the refit's removed part needs a read of
  industry's refit stores that tacsim does not have, so it is owed with IN.7.
- **The seeds have no content rows.** The mechanism is built; the part rows in the vendors' `stock` and the part
  asks are IN.6's (Q-39-12), so the lane does not move here and the seeded path's end-to-end proof is IN.6's
  story ("the lot's row reads `Seeded`").
- **The listing verb is the vendor's.** A part is listed through `@market/vendor stock … lot=`, which a player
  reaches as an officer of the vendor's org (the story's broker grant); a player's own `+market/sell <lot>` is
  not built.
- **No 256/512 pair**: the cut is port-side (IN.4's precedent).
- **The fit takes one more fire**: the hold lands before the draw, so the GC.8 and IN.1 stories' helpers advance
  one more fire.
- **The matter pin moved**: `shard_9/mill.rs::sm12a_…` counts the clerk under `.mint_origins(`; `.mint_origin(`
  is the remains scrap's alone. The receipt kinds are unchanged (`port.stock`).

**Cell delta (rule #14).** `tacsim-integration-2`: **+0.34 min** for the new story (20.6 s on PR run 41764030040's
JUnit; 12.6 s locally). The cell's sum reads 34.06 → 33.25 min, main's run at the merge base (41763500559, `ea602323`)
against the PR's, 411 → 412 tests: runner noise outweighs the story. The other changed tests run no longer: IN.1's
story 20.3 → 18.1 s with the fit's extra fire. On the same PR run `tacsim-integration-10` failed once on
`field_carrier::gc1b_a_refused_middle_party_never_lends_its_letter_to_the_next_arrival` (53 s against its 20 s
wall-clock `until_refusing` deadline; it passed on main's run at the merge base and locally in 20 s on this branch), a
timing flake outside the cut.
