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

## IN.3 — the order contract (Q-39-7, Q-39-28, Q-39-29, Q-39-30)

**Built.**

- **`customers: Public | OwnerOnly`** (Q-39-30) on `FacilityRecord`'s policy half (`mush_industry::Customers`,
  skipped when `OwnerOnly`, so an unopened facility's bytes are unchanged); `with_customers` bumps the policy
  revision as `with_cost` does, and `install_authored_facility` re-terms in place. **`may_order`** (staff, the
  owner's `acts_for`, or any principal with an `Active` account at a Public facility) is the one gate
  `@wo/draft`, `@wo/submit` and the offer's facility book ask. `@facility/terms <obj>=public|owner
  [<capital>/<years>/<upkeep>/<staff>/<wage>/<margin>]` is the staff hand until a lane row authors both.
- **The deadline at duration + TTL** (Q-39-28): `wo_submit` reserves materials and the slot to
  `now + work_units × 3,600 + RESERVATION_TTL_SECS` (`mush_inventory::reserve_for` /
  `reservation_deadline_after`, `IndustryFamilies::reserve_slot_for`); `@wo/restamp` renews the materials to
  the remaining work plus the TTL (`renew_for`). `reserve`, `renew` and `reserve_slot` keep their signatures.
- **The hold at `fee(order)`**: IN.4's `order_price`, unchanged.
- **The cutoff** (Q-39-29): `provider::cancel_cutoff` refuses a cancel from `AwaitingDelivery`
  (`CANCEL_DONE`) and from a holding order past half its work units (`CANCEL_PAST_HALF`), in the provider, so
  `@wo/cancel` is bound too. An order on the server-authored `unfunded` list (now projected for `WoCancel`
  as for `WoDeliver`) is past no cutoff: its exit stays open.
- **The forfeit**: the sweep settles a cancelled order's budget by `capture_part` of
  `money::cancel_forfeit_minor` — the facility's fee for the work units it ran, capped at the hold — and
  releases the rest through the economy's partial-capture seam; an order that never ran is released whole.
  Nothing is consumed before delivery, so the bill returns whole (round 2's 39-4).
- **The booking fee**: `money::place_booking` / `settle_booking` hold one slot-day
  (`FacilityRecord::booking_fee_minor`, the slot-hour × 24 at margin 0, rounded up to the credit) under the
  new engine-owned namespace **`industry.booking`** (`mush_economy::ENGINE_OWNED_RESERVATIONS`), forfeit to the
  facility on a cancel or an unfunded order, released at delivery. `money::charge_storage` posts one storage
  day under `industry.storage/{contract}/{day}`.
- **`@wo/draft`'s body is `commands::draft`** over a parsed `commands::Draft`, and `commands::submit` is
  public: the front books and the arm submits, delivers and cancels through `@wo`'s own records and gates.
- **The contract** (`tacsim::port::works`): `WorksContract` (`tacsim:works:{id}`, the id its escrow vault's)
  records the requester, the payer (the requester) and the beneficiary (the requester, or the organization an
  officer names with `=<org>`), the bill, the work units, the booking, the bound work order, the delivery
  instant and the storage days. The **escrow** is a vault of the requester's on a crate object of its own in
  the facility (one vault a backing). The one goods move a step owes is the authoritative act
  **`tacsim_works`** under the contract's ledger ordinal (`tacsim.works_ledger`): pools moved, lots relocated
  (a bulk lot dissolved where it lands, a part kept a lot), the destination hold's room checked by the plan
  the offer carried — the bay act's shape.
- **`+works` and `+yard`** (Authoritative): the bare verb lists the facility's recipes, its free slots and
  your orders; `/quote <recipe> [qty]`, `/order <recipe> [qty][=<org>]`, `/deliver <order>=<good> <qty>` (from
  the docked hold, admitted by the market's own gate), `/collect <order>`, `/cancel <order>` (the
  requester's alone). A front finds its facility as the one carrying `tacsim.works` / `tacsim.yard` in a room
  of the actor's port.
- **The arm** (`works::tick`, on the port tick after the bay's): binds a landed draft (a bounded walk of
  the work-order family for the requester's order sourced from the escrow); submits a covered order in
  booking order while the facility has a slot free (**two orders for one slot queue**), placing its budget
  hold first, and cancels it as the facility when the payer cannot fund it; delivers an `AwaitingDelivery`
  order; on delivery releases the booking; on a cancel forfeits the booking and returns the escrow's
  contents to the requester's docked hold (else they wait for `/collect`); collects a **house** order (the
  beneficiary the facility's owner: no booking, sourced from and collected to the facility's stores);
  and after **28 IC days** uncollected charges one slot-day a day, the facility's stores taking the goods
  when the account runs dry.

**The story.** `shard_2/in3_order_contract.rs::in3_a_twenty_day_plant_order_at_stickney_runs_to_collection`:
a captain and a stranger sail Dray-Ps to Stickney Yards; at the Service Counter's works (2 slots, §4.4's
works row, Public, owned by the Yards) the captain quotes and books two 20-day hauler-plant orders, delivers
both bills from the hold, and both run; the stranger books and supplies a fittings batch, which queues; the
Yards' officer cannot cancel it; at day 3 the captain cancels the second plant — the bill returns to the hold
whole and exactly the booking fee plus the work units run are forfeit to the Yards — and the freed slot runs
the stranger's order; past day 7 the first plant's materials are still held (deadline = the work + the TTL);
at day 11 its cancel is refused; at day 20 the arm delivers it, `+works/collect` puts the plant in the hold,
the fee is paid and nothing stays held; 28 IC days after the stranger's delivery, two storage days are
charged. Seeds (doc 43 rule #4): the works facility (`@facility/open`, `/owner`, `/terms`), a test recipe
pack (the plant and fittings recipes are IN.5's and IN.7's content), the holds' goods, the officer's grant.

**Departures from the row.**

- **No lane rows.** The works rows and their rooms are IN.5's, the yard row IN.9's, and the lane is held by
  RS.1 for v21, so the story seeds the works and `+yard` answers "There is no yard at this port." until IN.9.
- **No 256/512 pair.** The contract is port-side; §6 runs a story at both sizes "where the ground is in it"
  (IN.2's and IN.4's precedent).
- **The payer is always the requester.** `check_wo_accounts` refuses a budget account that is not the
  requester's, so an officer's order for an organization pays from the officer's account and only the
  beneficiary is the organization; an organization-paid order needs an officer's hold on the org's account,
  a later cut.
- **A purchase at the port's asks straight into the escrow** is not built; `/deliver` takes the hold's goods.
- **The part commission still has no producer**: a part recipe's output is collected as the industry lot
  (`output_lots: each`), untraced; the arm's commission is IN.7's, with its row's restart story.
- **The storage day is the booking fee** (a slot-day at margin 0), and a seizure moves the goods to the
  facility's stores vault (opened on the facility object on first need).

**Cell delta (rule #14).** tacsim-integration-2: _filled from the PR's JUnit_.

## IN.6 — the part lot at the workshop (Q-39-11, Q-39-12)

**Built.**

- **The workshop's stores** (Q-39-11): the operator org's storefront at the facility's port (`bay_work::stores_of`,
  `def.workshop.operator`'s vendor row), the same vault its stock rows mint into. `Replace` with no lot is now
  `Replace{lot: Store}`: `quote_here` prices it at the part row's catalogue reference (`QuotedJob.part_minor`,
  beside the labour in `cost_minor`); `+bay` prints the part's price on its own line "from the workshop's
  stores", or `NO_PART` ("The workshop has no <part> in its stores; +market/list the yard, or bring one in the
  hold.") when the stores hold none; the "all" line and the fee are `BayQuote::fee_minor` (labour and parts).
- **The take**: `+bay/order` picks the lowest-grade clean lot of the slot's row in the stores
  (`port::stock::store_lots`: the vendor's seeded lots of that good at the storefront, unfitted, held by nothing
  but their own lot ask and claimed by no open order), writes it into the job (`lot: Some`), records the
  storefront on the order (`BayOrder.part_vault`) and owes IN.2's **`Hold`** act at the storefront, so the lot
  is the order's until the draw; the draw consumes it there (`OwedKind::Draw.fit_at`), and a cancel owes the
  `Release` at the storefront. An empty store refuses with `NO_PART`; nothing is escrowed. The store pick runs
  after the take's dry run (a seeded part comes in intact, as the dry run fits one), so a doomed order keeps the
  engine's refusal.
- **The seeds** (Q-39-12; lane v22, relay v13, after RS.1's v21/v12): part `stock`
  rows only (grade 9, issue 1). Occator at Ceres holds four each of `cannon_105`, `mg_coax`, `visual_mk1`,
  `plant.vehicle_mk1` and `plant.light_mk1`; Stickney Yards six of all thirteen part rows. `MAX_VENDOR_ROWS`
  16 → 24 (the Yards carry 18 stock rows). The strip chain's head is `lane_chain::strip_part_seeds`, called first
  by `strip_tender` (RS.1's v21 strip), so every chain rebuilding an older lane still hits its published digest.
- **The stores cost a boot no fire**: the clerk's mint of a vendor's bulk row lays every unminted part row of the
  same vendor in the same bundle (`mush-inventory` `mint_origins_from`, a lineage source a row, so each lot is
  rooted in its own stock issue; at most ~170 writes at the Yards against the bundle's 1,024), so a port boots in
  exactly the fires it took before. Readiness counts a part row seeded once its receipt exists (its lots are never
  dissolved, so IN.2's rule read it as never seeded). A part row added to a kept world (a pack apply) is minted on
  its own, after the bulk rows, asks and bids (`seed_step(parts: true)`).
- **For a lane that authors part asks** (none does yet): `part_ask_step` keeps one lot ask a part good standing
  at a time, and a vendor at `MAX_LISTINGS_PER_VENDOR` lists no more. A store lot an order takes from its own ask
  is withdrawn by the stock clerk, who listed it (`stock::claimed_step`; the market's release leg is the
  seller's), and the order's `Hold` waits for that without spending a try; the clerk never lists a claimed lot
  again (`bay_work::claimed_store_lots`).

**The story.** `shard_2/in6_part_lot.rs::in6_a_refit_at_ceres_takes_the_cannon_from_occators_stores`: a First
Captain leases Occator's escort at Ceres. By the time the port reads ready the clerk has seeded four cannon lots at
Occator's storefront, each with a `Seeded` row. The escort's cannon is destroyed and it is berthed at C14
(Occator's workshop, 1.2x). `+bay` quotes 480 of labour and 28,000 for the part from the stores;
`+bay/order …=replace` escrows 28,480 and takes the first seeded lot. At the draw the cannon is whole, the lot is
consumed, its row reads `Seeded` with `fitted_into` the escort, and the stores count three (four less one). Three
more refits empty the stores; the fifth replace refuses `NO_PART` at the quote and at the order, and nothing is
taken. Seeds: the destroyed cannon (staff damage through the unit's plan path, GC.8's fixture) and the captain's
money (`@econ/mint`).

**Departures from the row.**

- **At Ceres the stores hold four, not six** (Q-39-12 as ruled: Occator four of each escort and rig part, the
  Yards six of every row). The row's numbers (480 of labour, "six less one", "the seventh replace at Ceres")
  mix the two ports: 480 is Ceres' 1.2x on the 400 labour, six is the Yards' count. The story runs at Ceres
  with Q-39-12's four, so the store reads four less one and the **fifth** replace refuses `NO_PART`.
- **The stores are not listed on the market** (Q-39-11's "+market/list at the port already shows it"): the lane
  authors no part asks. With them, Occator's lots stood on the Exchange board, and their listings and mints were
  background acts in every rig on the shipped lane (PR #399's first CI run: five cells red; a second, which
seeded the stores within readiness one row a fire, lengthened every boot and moved four clock-bound stories). The lot-ask path is
  kept for a cut that authors them, and that cut needs the market's reserve grace (below).
- **One part from the stores an order** (`ONE_STORE_PART`): an order owes one act at a time (and with lot asks a
  command can submit only one authoritative act), so an order that would replace several parts from the stores is
  refused with "order the other replaces after it". A turret's three parts are three orders. A multi-lot hold is
  a follow-up for IN.8/IN.9 if the owner wants it.
- **Only the seeds are read** as store lots: the stores are found by the seeds' deterministic lot ids
  (`seed_lot_id`), because the inventory has no scan of an endpoint's lots. A part sold, made or scrapped into a
  storefront is not drawn by `Replace{lot: Store}` until IN.7/IN.9 give the stores an index.
- **The part's fee is the order's one fee**: the price rides the fee contract to the operator, not the
  market's lot sale; the lot is consumed by the draw, not handed over (doc 39 §4.5: "the draw consumes it and
  the fee pays the operator").
- **No 256/512 pair**: the cut is port-side (IN.2's and IN.4's precedent).
- **A slot with no part row** refuses `NO_PART` (naming the slot) instead of being supplied free.

**Found, and split out: the market's reserve race.** The first build listed the seeded lots, and the story then
lost its first cannon. The market sweep's lot-hold reconcile withdrew a lot ask whose reserve was staged but had
not landed (both ran in one fire); the hold then landed under a listing that no longer existed, stranding the lot.
It applies to any exact-lot listing (IN.2's `@market/vendor stock … lot=`). The fix (`RESERVE_GRACE_SWEEPS`, a RAM
`reserve_staged` map the reconcile skips for two sweeps) is its own PR, not this one.

**Other tests moved.** `shard_10/commerce.rs`: the receipt counts 16 → 34 and Occator's readiness line
(7 of 7 rows); part receipts are skipped where a test checks pools. `shard_3/drones.rs` `sm12b_a_delivery…`'s
licence-fire census leaves out a journey's `atlas` tick (a guard for the boot's length).
`shard_9/mill.rs`'s matter pin counts the clerk under `.mint_origins_from(`.
`shard_1/bay_36a.rs` `b36a_2_a…` orders the cannon alone (one store part an order) and lets the order's hold land
before its cancel. `shard_9/mill.rs`'s pack-apply story waits 64 fires after the apply (it adds the part rows). The
lane pins: `LANE_VERSION` 22, `LANE_DIGEST`/`SHIPPED_LANE_DIGEST` `af3f150a…`, relay v13 `c55ce471…`
(`relay_registry.rs`, `boot_parity.rs`, `shard_10/stations.rs`), and the version asserts in `shard_3/drones.rs`,
`shard_5/sites.rs`, `shard_9/mill.rs`, `cost_lane.rs` and `shard_4/lane.rs`.

**Cell delta (rule #14).** `tacsim-integration-2`: **+0.35 min** for the new story (21.2 s on PR run 41805020497's
JUnit, the merged head 1fbd149e). The cell's sum reads 25.10 → 37.44 min, main's run at the merge base (41791768168,
`2b1eca4c`) against the PR's, 413 → 414 tests: runner speed outweighs the story. Every test in the cell ran about 1.4×
slower, unrelated ones too (`twin::m2_gate_a_on_a_mixed_pool` 28.1 → 52.5 s, `moon_ports::a8_titan_throttle…` 42.7 →
64.0 s), and no test ran faster. The other changed tests run no longer for the cut: the stores are minted inside the
bulk mint, so a port boots in exactly main's fires.

## IN.5 — the first production story: the refine and the works (Q-39-4, Q-39-5, Q-39-6, Q-39-23, Q-39-31)

**Built.**

- **Lane v23, relay v14** (after IN.6's v22/v13). The strip chain's head is `lane_chain::strip_industry`, called
  first by `strip_part_seeds`; `shard_4/lane.rs::in5_strip_industry_rebuilds_the_published_v22` pins it against
  IN.6's digest.
- **The refine** (Q-39-6). `MillDef.kind` is `mill` (the default, skipped on encode, so the ore mills re-encode
  byte-identically) or `refine`. The family rule in `check_ports` binds a `mill` row only. Kasei's row:
  `{kind: refine, ore: metals, into: structural_alloys, ratio 5:4, batch_t 20, per_ic_day_t 40}`. Its
  concentrate bid gains `refresh {per_ic_day: 40, cap: 4000}`. The 7,000 t seed stays as opening stock. The
  refine is the mill engine unchanged: its ledger, its restock of Kasei's alloy ask, its drift check and its
  readiness line.
- **The facility rows** (Q-39-5): `PortDef.refinery` (room, operator, `smelt`, cost; one slot, never booked) and
  `PortDef.works` (room, operator, the works capability, slots, cost, `customers`, `buys`). The applier opens them
  as parts l and m (`port/plant.rs`, part j's precedent: the room is the facility, drift refused, a changed cost or
  customer policy re-termed in place). The checks: the room is in the area and belongs to no other block; the
  operator is an org of the entry; the capability is held. A `refine` row needs the port's refinery, and the
  refinery's operator is the vendor that refines.
- **The rows**:
  - Stickney's refinery at `p07_receiving` (Kasei, §4.4's row, margin 0).
  - Stickney's works at `p15_maintenance` (Stickney Yards, two slots, Public, §4.4's works row; `+works/quote
    fittings` reads 265.00 cr).
  - The Ceres works at `c15_maintenance`, owned by **Freeport Works**, a new Ceres org (capital 1,000,000).
    Its vendor row takes the Exchange's alloy bid (520, intake 1,000) and carbon bid (50, intake 120, the
    refresh unchanged). Its storefront is the works room, so the bids fill the works' stores.
  - The Exchange gains a carbon stock row (240 t) and ask (60). The programme's carbon reference becomes that
    listing, since a fixed reference is only for a good no port asks.
- **The recipe pack** (§4.3): `content/industry/recipes-v1.json`, `tacsim.industry.recipes` v1, with `fittings`
  (5 `structural_alloys` + 1 `carbon_ore` → 5 `industrial_parts`, `works`, 6 work units, `output_lots: pooled`).
  It ships as a game seed source beside the relay's manufacture pack (`game_seed_sources`), so every deploy with
  the game on publishes the works' menu.
- **The house purchase** (Q-39-31; `port/supply.rs`): while the Stickney works' stores hold under 10 t of alloys,
  the stock clerk buys a 5 t lot an IC day from Kasei at its ask, paid from Stickney Yards' account. The step
  runs in two phases:
  1. one economy post keyed `tacsim.works_supply/{stores}/{act}/{good}/{lot}`;
  2. one `tacsim_works` move, the order contract's own act, under the works ledger keyed by the stores vault.

  A purchase is in flight exactly when the next ordinal's key has resolved, so a restart neither pays twice nor
  loses a paid lot. The seller's listing is withdrawn before the move (no Draw in flight) and re-listed by the
  list step (doc 20 SM.12a's healing path), so the drift check never sees units leave from under a listing. While
  the ask is down, its readiness line reads Ready ("a house purchase is moving part of it"), never Pending.
- **A port update retires a bid it no longer authors** (`vendor.rs`): the standing listing is withdrawn and the
  map row forgotten (the retire's rule). It is refused while a sale into the bid is unsettled. This is what lets a
  kept world take the Exchange's bids moving to Freeport Works.
- **`+works` at a room facility**: `here()` and the arm's `port_of` find a facility that is the port's authored
  works room (the spine is not one of `port_rooms`).
- **The pooled output** (§4.3): the arm dissolves a delivery's bulk lot in the escrow on the fire after the
  delivery (`Then::Pooled`, an escrow-to-escrow act). The collection then moves pools, and a collection asked
  for first answers "That order is still settling." IN.3's prepare relocated and dissolved in one act, which can
  never land: a bundle mutates a pool's tally key once. IN.3's story never collected a pooled output. Escrow
  contents now name the order's bulk outputs beside the bill.
- **The bot** (`mush-bot/src/mine/works.rs`, mounted as `lane::works`): the closed vocabulary (`+works` and its
  switches, `+ship/plan|undock ceres|stickney`) and the reader (`booked`, `delivered`, `supplied`, `collected`,
  `quote_fee`, `row_quantity`, `ask_row`, `bid_row`). The contract test holds it to `tacsim::port::works`'s new
  `pub` sentence constants (`BOOKED_AT`, `DELIVERED_AT`, `INTO_ESCROW`, `INTO_HOLD`, `FEE_CLAUSE`, `SETTLING`).

**The stories.**

- `shard_2/in5_production.rs::in5_concentrate_refined_at_kasei_becomes_fittings_sold_at_ceres` (shard_2, the
  shard with the most room), through the production commands:
  1. The lane's facilities and Freeport Works' bids are read.
  2. A First Captain buys 20 t of concentrate and 1 t of carbon at the Exchange's asks and sails to Stickney.
  3. They sell the concentrate into Kasei's bid. The refine ledger reads 20 → 16 and Kasei's ask restocks by 16.
  4. They buy 5 t of alloys, quote and book `fittings`, and deliver the bill. The arm runs, delivers and pools
     the order, and `+works/collect` puts 5 t of fittings in the hold.
  5. Home, the Co-op's bid pays 3,000.00 cr less the Trust's 100.00 cr share.
  6. On the way, the works' stores reach their 10 t floor in two purchases (the works ledger reads 2, no third
     key paid). The drift check never pages, and no write is refused.

  Seeds: the captain's money (`@econ/mint`) and the passage's bunkers (`load`).
- `mush-bot/tests/mining_cohort.rs::in5_the_bot_mines_refines_and_sells_fittings_from_ahuna` (256) and
  `…_from_urvara` (512), the bot over SSH. The mining lane plays its loop to the hold sold, under the canary's rate
  lift. God then lifts the rate to 3,600, and the bot types the production half in its own vocabulary, reading
  every answer with its own reader: the Exchange's asks, the bunkers, the passage, Kasei's bid, the restock
  watched on `+market/list`, the alloys, `+works/quote|order|deliver|collect`, the passage home and the Co-op's
  bid (gross 3,000.00 cr). Both `#[ignore]`d, run by name on conclav.dev (by-hand tier, doc 43 TS.2):
  `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored in5_ --nocapture --test-threads 1`.
  by-hand: _the revision and walls of the conclav.dev run_.

**Departures from the row.**

- **No new atlas rooms.** §7 names `p12_refinery`, `p13_works` and `c15_works`. A room in
  `neighborhood.ceres_freeport` re-publishes it at v5, and ten packs pin v4 exactly (the cast, crew, passenger,
  quest and relay packs). That cascade crosses the parallel cuts' files, so the facilities open in standing
  rooms: Kasei's receiving office for the refinery, and the two stations' service spines for the works.
  `+works` needs only the port. The station re-publish is IN.9's, which owes `p14_slipway` anyway; the rooms
  move then.
- **No strategic-ore ask.** "The Exchange's ore asks" has only carbon: a vendor may not both ask and bid one good,
  and the Exchange keeps its strategic bid (the ore economy's size knob). IN.7, which owns the strategic recipes,
  rules where that ask lives.
- **The Exchange's carbon is a finite 240 t** (provisional). Its intake moved to Freeport Works with the bid
  (§4.2's table), so nothing refills the Exchange's ask but the opening row. Flagged for the tune.
- **"Mines carbon ore on Kerwan"** is read as the bot's mining loop (metal ore, sold into the Exchange's bid) at
  the story's two ground sizes, Ahuna and Urvara. The story's carbon is bought at the Exchange's ask, as the row's
  next clause says. The bot mines metals and volatiles only; a carbon family for the lane is a later bot cut.
- **The house purchase has no consumer yet**: house orders at the works are IN.10's. It stops at the floor.

**Other tests moved.**

- The lane pins: `LANE_VERSION` 23, `LANE_DIGEST`/`SHIPPED_LANE_DIGEST` `fe502fe1…`, relay v14 `476bf7d8…`
  (`relay_registry.rs` and its kept-world test, now `sm12b_a_kept_v13_star_takes_lane_v23_and_relay_v14`;
  `boot_parity.rs`; `shard_10/stations.rs`), and the version asserts in `shard_3/drones.rs`,
  `shard_5/sites.rs`, `shard_9/mill.rs`, `cost_lane.rs` and `shard_4/lane.rs`.
- `shard_4/lane.rs`: the Syndicate is still appended last before Freeport Works.
- `shard_9/mill.rs`:
  - the shipped lane's mill rows count Kasei's refine, and the vendors' mill flags count Freeport Works;
  - Stickney's clerk is now offered the mill action, so another port's clerk is refused at the resolve;
  - the one-act check leaves out Stickney's own writes (Kasei's drip and the house purchase), by its listings
    before and after the fire.
- `shard_2/in3_order_contract.rs` books at the lane's Stickney works instead of seeding one.

**Cell delta (rule #14).** _To be read off the PR run's JUnit._
