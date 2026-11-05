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

- **Lane v24, relay v15** (after CR.2's v23/v14, which merged first). The strip chain's head is
  `lane_chain::strip_industry`, called first by CR.2's `strip_release_authority`;
  `shard_4/lane.rs::in5_strip_industry_rebuilds_the_published_v23` pins it against CR.2's digest, and the chain
  under it against IN.6's.
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
    Its vendor row is the last at Ceres. It stocks 240 t of carbon ore and asks it at 60, and its storefront
    is the works room, so its stock is the works' stores.
  - The programme's carbon reference becomes that listing, since a fixed reference is only for a good no port
    asks.
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
  loses a paid lot. Before the move, the seller's ask is lowered by the lot with `mush_market::shrink_ask`
  (`restock_ask`'s mirror, never while a Draw is unsettled), so the ask stands throughout. The pool's excess over
  the listing and its undrawn quotes is the durable mark that the shrink is done. The drift check and the
  readiness report discount the lot in flight (`supply::moving`) until the works ledger shows it landed.
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
  2. A First Captain buys 20 t of concentrate at the Exchange's ask and 1 t of carbon at Freeport Works', and
     sails to Stickney.
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
  every answer with its own reader: the Exchange's and Freeport Works' asks, the bunkers, the passage, Kasei's bid, the restock
  watched on `+market/list`, the alloys, `+works/quote|order|deliver|collect`, the passage home and the Co-op's
  bid (gross 3,000.00 cr). Both `#[ignore]`d, run by name on conclav.dev (by-hand tier, doc 43 TS.2):
  `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored in5_ --nocapture --test-threads 1`.
  The way out is by Luna and Deimos: Ceres→Aitken Highport→Voltaire Anchorage→Stickney (`ceres_aitken`,
  `aitken_voltaire`, `voltaire_stickney`, about 27 IC days), bunkering at each port. The way home is
  `stickney_ceres`, whose legs carry no hazard. The detour adds about 11 real minutes a site at the 3,600 rate.
  Seed: God mints the pilot a 50,000.00cr float from a system source of the story's own (`@account/system … =tacsim.credit`, the
  pilot's `@account/list`, `@econ/mint`), staff lines outside the bot's vocabulary.
  The closing check reads `@tacsim/port status`: no line pending, missing or flagged but the two the story's rate
  brings, the rate line itself and Occator's house rig stopped under a claim after the IC month the lift runs.
  by-hand: `cdd30329` (main `a193de9c` with the busy-sale retry), both side by side on conclav.dev: Ahuna 1,895 s,
  Urvara 1,897 s, each about 570 s of mining, 1,325 s of production and passage, and the sale. Tier: by hand
  (`test-tiers.toml`).

**Departures from the row.**

- **No new atlas rooms.** §7 names `p12_refinery`, `p13_works` and `c15_works`. A room in
  `neighborhood.ceres_freeport` re-publishes it at v5, and ten packs pin v4 exactly (the cast, crew, passenger,
  quest and relay packs). That cascade crosses the parallel cuts' files, so the facilities open in standing
  rooms: Kasei's receiving office for the refinery, and the two stations' service spines for the works.
  `+works` needs only the port. The station re-publish is IN.9's, which owes `p14_slipway` anyway; the rooms
  move then.
- **The Exchange keeps its bids; the carbon ask is Freeport Works'.** §4.2 moves the Exchange's alloy and carbon
  bids to Freeport Works and gives the Exchange a carbon ask. The port applier never retires a bid on an update
  (a kept world would refuse the apply), and a vendor may not both ask and bid one good. So the bids stay where
  they are (the alloy bid keeps its "Freeport Works" alias), and Freeport Works stocks and asks the carbon. Its
  240 t is finite (provisional): nothing refills it. Flagged for the tune, and for IN.10's house orders, which
  draw on those same stores.
- **No strategic-ore ask**, for the same rule: the Exchange keeps its strategic bid. IN.7, which owns the
  strategic recipes, rules where that ask lives.
- **"Mines carbon ore on Kerwan"** is read as the bot's mining loop (metal ore, sold into the Exchange's bid) at
  the story's two ground sizes, Ahuna and Urvara. The story's carbon is bought at the Exchange's ask, as the row's
  next clause says. The bot mines metals and volatiles only; a carbon family for the lane is a later bot cut.
- **The house purchase has no consumer yet**: house orders at the works are IN.10's. It stops at the floor.
- **The bot sails out by Luna and Deimos, not the corridor.** On main every Ceres→Stickney passage is interdicted
  at Kirkwood: `ceres_stickney`'s leg 0 has hazard 4 against an interdiction threshold of 3, and the Kirkwood
  location carries the `kirkwood_picket`. The SF track's wall-clock customs bubble wants heave-to and
  `+helm/submit`, then the raider fight (doc 35 SF.R6). The owner has said these cuts raise no space exposure,
  and the lead ruled the story should not fly the picket. Stripping it in the bot rig would need a production
  hook, because the rig boots the real server with the built-in lane, and an operator seed may not claim the
  lane's id. So the story takes the lane's own picket-free way out, on production paths with nothing stripped,
  and the matter pin and the drift check are untouched.
- **The bot's money does not cover the long way round** (an economy finding, provisional numbers): unseeded, at
  both sites the pilot reached Stickney, sold, refined, ordered and collected the fittings, then could not pay the
  bunkers home ("Your account does not have enough available to cover that."). The mining loop's earnings plus the
  trade's margin do not buy propellant at 60 cr a tonne for three legs out and one home. The story seeds a float
  (above); tuning the margin or the propellant asks is the economy's, not this cut's.
- **Stickney Yards' capital is thin** (an economy finding, provisional numbers): its 50,000 now also pays the works'
  house purchases of Kasei's alloys (two lots, 4,500), so IN.8's `in8_a_whole_escort_scrapped_at_stickney…`,
  whose escort costs the Yards about 45,000, can no longer buy the heavy's bale back. The story seeds the Yards
  50,000 from Ceres' treasury before that step (`fund_the_yards`); the capital is the economy's to tune.
- **For the SF thread:** the bot has no picket words (brake under 10 m/s, `+helm/submit`, `+tac/pdc auto`, a
  jink), so any bot haul through Kirkwood stalls in the bubble.

**Other tests moved.**

- The lane pins: `LANE_VERSION` 24, `LANE_DIGEST`/`SHIPPED_LANE_DIGEST` `0d07927c…`, relay v15 `2fff2038…`
  (`relay_registry.rs` and its kept-world test, now `sm12b_a_kept_v13_star_takes_lane_v24_and_relay_v15`;
  `boot_parity.rs`; `shard_10/stations.rs`), and the version asserts in `shard_3/drones.rs`,
  `shard_5/sites.rs`, `shard_9/mill.rs`, `cost_lane.rs` and `shard_4/lane.rs`.
- `shard_4/lane.rs`: the Syndicate is still appended last before Freeport Works.
- `shard_9/mill.rs`:
  - the shipped lane's mill rows count Kasei's refine, and the vendors' mill flags count Freeport Works;
  - Stickney's clerk is now offered the mill action, so another port's clerk is refused at the resolve;
  - the one-act check leaves out Stickney's own writes (Kasei's drip and the house purchase), by its listings
    before and after the fire.
- `shard_2/in3_order_contract.rs` books at the lane's Stickney works instead of seeding one.
- Ceres' fourth vendor and ninth org: `shard_10/commerce.rs` (receipts 34 → 35, vendor counts, the Exchange Hall
  board's carbon line), `shard_10/ports.rs` (orgs 12, treasury 15,951,000.00 cr, 63 commerce lines),
  `shard_10/plumbing.rs` (Freeport Works' two tranches; it is the one non-Ceres-bid vendor stocking ore),
  `shard_5/sites.rs` (the org order), and `boot_parity.rs` (25 institutions).
- Kasei's bid refreshes and the refine consumes what Kasei buys:
  - `shard_9/sale.rs` runs IC10's suite on the lane without IN.5's rows (its subject is the plain bid);
  - `shard_3/orders.rs` counts the pilot's own submissions and reads one sale by its contract;
  - `ic_slice_loop.rs` reads the bid through the port map, counts Kasei's concentrate with its refine
    ledger, and counts the alloy ask's sales net of the refine's restock and the house purchases. Its
    `drain_players` treats Stickney's clerk as a house actor.
- `cost_lane.rs` settles the Stickney works' two purchases before the C2/C3 cohort, so its quiet points find
  Stickney's clerk idle.
- `shard_5/sites.rs`'s "nothing is burned" checks burns, not every tacsim submission.
- `shard_1/driving.rs`'s site-map fixture is re-blessed: 18 IC seconds of extra stocking fires.
- mush-market `e7r2`: `shrink_ask` joins `restock_ask` as a taker of `RestockSpec`.

**A flake seen on this PR (doc 43 rule #7).** `shard_8/escort_lease.rs::cf6c_a_leased_escort_flies_out_fights_withdraws_and_goes_back`
failed twice on the PR's CI (`tacsim-integration-8`) and once in three runs on conclav.dev on this tree. It passed
2 of 2 there on main (`4390f04c`) and always on Windows. Each time the escort is destroyed during the fire
exchange, before it drives to the ramp ("Your tank is destroyed and takes no orders"). The fight reads no industry
row, so the likely cause is timing: the story's `until` loops fire on wall-clock steps, and Stickney's new port
activity (the refine, the drip, the house purchases) adds work to every fire. Owed: a CF-train look at the story's
fight budget (the exchange should stop at the first hit both ways, not let the intruder keep firing).

**Cell delta (rule #14).** `tacsim-integration-2`: **+0.22 min**. The cell's sum is 21.51 → 21.73 min, main's run
at the merge base (41827163605, `4390f04c`) against the PR's (41853059138), 414 → 415 tests. The new story takes
20.8 s. The common tests moved −0.12 min in all; `cost_lane`'s C2/C3 cohort adds about 10 s for the house
purchases it settles first (`sm12c_a_restart…` 69.1 → 78.8 s).

## IN.7 — the part recipes (Q-39-22, Q-39-31)

**Built.**

- **The part recipes** (§4.3): `tacsim.industry.recipes` v2 (`content/industry/recipes-v2.json`, which replaces v1
  in `game_seed_sources`) carries `fittings` unchanged and the table's thirteen part rows. They are the five
  mounts and racks (`mount_105`, `mount_30`, `mount_mg`, `mount_laser`, `rack_lr`), the two sights, the two pods
  and the four plants, each at the works with `output_lots: each`, with the table's inputs, outputs and work
  units. The works' menu shows the latest published version of each recipe, so a kept world that holds v1 beside
  v2 offers `fittings` once.
- **The arm's part commission** (§4.11; IN.2's act):
  - On the fire after a part recipe's delivery, the works arm finds the escrow's untraced part lot (a part good
    with no provenance row) and owes IN.2's `Commission` under the facility's own bay ledger
    (`bay_work::owe_commission`, ledger = the facility object, submitted as the requester).
  - The act consumes the lot by its `workorder.output` lineage. It lays one lot a unit at grade 9, each with its
    `Manufactured {order, facility}` row, in one bundle. The bay tick submits it on the next fire, and its ordinal
    replay lands it once across a restart.
  - Until the act lands, `+works/collect` answers "That order is still settling.", and a house order is not
    collected. The ten-unit industry lot carries no row, so `+bay/fit` refuses it.
  - A hull kit is not a part, so IN.9's yard commission does not meet this one.
  - An order of a part recipe is capped at 32 parts, one commission's rows (`+works/quote`/`order` refuse
    "The works lays at most 32 parts an order; order fewer.").
- **Lot moves in one bundle** (`mush_inventory::InventoryFamilies::relocate_lots`): a collection of ten part lots
  failed "a lot could not be moved", because ten single `relocate`s write one endpoint's census and tally keys ten
  times and a bundle writes a key once. The works' move relocates its lots together.
- **The stores index** (IN.6 left it to IN.7/IN.9): `stock::store_lots` finds a storefront's part lots by a bounded
  walk of the lot family (16 pages of 256), not by the seeds' derived ids. A part made, scrapped or sold into a
  storefront is now drawn by `Replace{lot: Store}` like a seeded one: traced, unfitted, clean, unheld but for its
  own ask, and unclaimed.
- **The disputed part** (the row's last clause): `bay_work::disputed` is a stripped part that its parent's record
  lists under a claim (`claim_lots`) that is not vacated and whose filed contract has not ended. An unreadable
  contract counts as live: fail closed. `+bay/fit` refuses it: "That part is disputed: it came off #… under claim
  …, which is still open; nothing takes custody of it until the claim is settled." `disputed_refusal` is the
  sentence for a build's custody too. `strip::list_under_claim` is staff's seam, `vacate_claim`'s pair, since no
  ruled path strips under a claim (Q-36-33).
- **The lane** (v26, relay v17; after CR.2's v23, IN.5's v24 and IN.9's v25):
  - Stickney Yards bids strategic ore at 700: intake 12, refresh 2 a day, boards exchange and yard. This is
    Q-39-31's Stickney bid; the Exchange's 600 at Ceres stays, so a hauler clears 100 a tonne.
  - **The seeds retire** (Q-39-12): Occator's five part stock rows and the Yards' thirteen are gone, so the stores
    hold what is made, scrapped or sold into them.
  - The strip chain's head is `lane_chain::strip_part_recipes`, which `strip_industry` calls first. It puts the
    rows back from `SEEDED_PARTS` and removes the bid. `shard_4/lane.rs::in7_strip_part_recipes_rebuilds_the_published_v24`
    pins it against IN.5's v24 digest.
- **Instrument boards from Luna**: no new row. Crisium Instruments' ask at Aitken Highport (900) is the source,
  and the lane's own routes are the leg (Ceres → Luna → Voltaire → Stickney: Luna has no route back to Ceres, and
  this leg passes no Kirkwood picket).

**The story.** `shard_2/in7_part_recipes.rs::in7_strategic_ore_boards_and_fittings_become_ten_sights_at_stickney`
(shard_2, the shard with the most room):
1. A captain at Ceres with three tonnes of strategic ore and a titled scout in their Dray-P's bay sails to Luna.
   They buy a tonne of boards at Crisium's ask and sail by Voltaire to Stickney.
2. The scout is berthed and its sight wrecked. `+bay` quotes `NO_PART`: the seeds retired.
3. A tonne of ore goes into the Yards' bid at 700, and three tonnes of fittings come off the Yards' ask.
4. `+works/quote sight_visual` prints the bill and sixty work units, and a quantity of four is refused at 32
   parts. The captain books the order and delivers the three lines, and the arm runs it.
5. Delivered, the escrow holds one ten-unit industry lot with no row. `+bay/fit` refuses it, `+works/collect` is
   settling, and the works' bay ledger owes the commission.
6. The commission is submitted and the world restarts before it lands (the submission dropped, `reboot`). After
   the restart it lands once: the ledger reads 1, the industry lot is gone, and there are ten one-unit lots at
   grade 9 with `Manufactured` rows naming the order and the works.
7. `+works/collect` puts the ten in the hold, and `+bay/fit` fits one to the scout. Its row reads manufactured,
   fitted into the scout, and the sight is intact.
8. The sight is stripped again and staff list it under an open claim. `+bay/fit` refuses it as disputed and
   stages nothing.

Seeds: the money, the ore in the hold, the bunkers, the scout's unit, title and placement, the wrecked sight, the
open claim.

**Departures from the row.**

- **No 256/512 pair**: the cut is port-side (IN.2's, IN.4's and IN.6's precedent). The ore the story seeds is the
  bot's to mine, and the bot mines metals and volatiles only (IN.5's note).
- **"Refused at a build's custody"** is proved at `+bay/fit`'s custody. On this tree the yard's `+yard/deliver`
  of part lots is IN.9's (#416, not merged). `disputed_refusal` is the one sentence for IN.9 to call there, with
  `relocate_lots` for its multi-lot delivery.
- **The strategic ask at Ceres is not authored**: the row names only the Stickney bid. The Exchange cannot ask
  the good it bids (IN.5's rule), so strategic ore reaches the works from miners and haulers, who sell into the
  700 bid or deliver into an order from their own hold.
- **The seeds retire with no house restock**: with the lane's part stock gone, a workshop's stores are empty
  until parts are made, scrapped or sold into them. A refit that wants a part from the stores now finds `NO_PART`
  unless one is there, and the captain orders the part at the works and `+bay/fit`s it. A house order that keeps
  the stores stocked is IN.10's arm (house orders) or a later cut. **Flagged for the owner.**

**Other tests moved.**

- `shard_2/in6_part_lot.rs` and `shard_1/bay_36a.rs::b36a_2_…` stock the stores they draw from by staff
  (`driving_rig::seed_store_parts`: one-unit grade-9 lots with `Seeded` rows, as IN.6's stock row laid them),
  since the lane seeds none. Their claims are unchanged.
- The lane pins: `LANE_VERSION` 26 and the lane digest (`shard_4/lane.rs`, `cost_lane.rs`), relay v17 and its
  digest (`relay_registry.rs`), and the version asserts (`drones`, `mill`, `sites`, `stations`, `boot_parity`).

**Cell delta (rule #14).** `tacsim-integration-2`: _filled from the PR's JUnit_.
## IN.8 — the mass budget and scrap (Q-39-15, Q-39-16, Q-39-17, Q-39-25, Q-39-26)

**Built.**

- **The partition at the catalogue build** (§4.8, Q-39-15): `definitions::mass::partition` gives every design the
  yard partitions a `MassBudget` (`Catalog::budgets`, `budget_of`): the dry mass (`logistics.dry_mass_t`, else the
  field-ops measure), one part a slot at goods v5's masses and references, the core (150 / 250 / 500 kg, and its
  3 / 8 / 25 % reference), and the remainder split by class (armour, vehicle and hauler 0.75, walker 0.70, drone
  0.65; capital and station not partitioned). The figures spread over the design's **structural points** — facet
  armour and section internals, never a heat cell. The escort reads 2,565 kg of structure and 855 of fittings
  over 2,990 points (0.86 and 0.29 kg a point); the Dray 157,500 and 52,500 over 1,820 (86.5 and 28.8). A design
  whose parts and core leave no remainder has no budget (it draws nothing and yields nothing). The bill (§4.7:
  the loss on top, each bulk line rounded up, a day per 20 t, the build at §4.4's 68.75 cr a work unit and 25 %
  margin) is computed from the budget at the live references; IN.8 reads it only as the reference a scrap's fifth
  bounds (the escort's is 75,495 cr at 450 and 520), and a programme hull's reference is its buyout (the Dray's
  and the Dray-P's 90,000, Q-39-17).
- **Repair draws both goods** (§4.8): `BayQuote::bale_need` takes the hull's budget — `points ×
  structure_kg_per_point` of alloys and `points × fittings_kg_per_point` of fittings, rounded up — through
  IN.1's bale; the tender's stabilize (RS.1) draws the same figures for its target (`TendWork::need(budget)`).
  An escort's hundred-point patch draws 86 kg and 29 kg; a Dray's 8,654 and 2,885.
- **The three constants retire**: `SCRAP_KG_PER_STRUCTURAL_POINT` and `FALLBACK_SECTION_SCRAP_KG` (mush-tactics;
  `ScrapYieldReport` now reports the hull's surviving structural points, the host weighing them), and the
  tariff's `structural_points_per_industrial_parts_t` (with `WorkshopCostQuote::industrial_parts_t`): the tariff
  keeps labour and time only. RS.1's `fittings_kg_per_point` constant is gone with it.
- **Scrap at a workshop** (§4.6, Q-39-16, Q-39-25): everything goes to the operator. The yield (the design's
  structure over its surviving structural points at r 0.6) and the hull's bale go into the workshop's **shop
  bale** (`tacsim:shopbale:{port}`, §4.10), whose whole tonnes mint into the operator's storefront (its stock) and
  whose remainder stays; each salvageable part goes to the same storefront as a lot with its scrapped provenance
  (its stores). The title holder is credited `min(yield at the alloys' reference, (reference − parts new) / 5)`
  for the structure, each part's used value (0.75 intact, 0.40 worn) less its removal (the tariff's replacement
  labour at the operator's multiplier), and the bale's kilograms at the references; the operator posts it, from
  its account to the holder's, when the mint lands (idempotent on the hull and the act). A workshop that cannot
  pay refuses the scrap. The shop bale is held for the owed act (`pending`), so a second scrap or return at the
  same workshop waits for it; a given-up mint lets it go as it stood and pays nothing.
- **`+bay/return <hull>`** (§4.10, Q-39-26): the title holder sells the hull's opened bale back to the workshop at
  the references, no labour, no fifth, into the shop bale (which mints a tonne each time it passes 1,000 kg); the
  hull's bale is emptied when it lands. A new receipt kind, `bay_return` (the closed-kinds pin in `shard_9/mill.rs`
  now reads five).
- **Scrap on a site** (§4.6, the fix thread's room check): `+tv/<n> scrap` mints `claim::remains_alloys_t` — the
  design's whole structure at r, capped at the fifth of its reference at the alloys' reference, down to whole
  tonnes (the rest a declared loss) — no longer the remains' whole dry mass; remains of no known design yield
  nothing. The command refuses a hold that will not take the tonnes (`SCRAP_NO_ROOM`, the hold's own line), and
  the prepare checks the same plan against the pool its bundle reads.

**The story.** `shard_2/in8_mass_scrap.rs` (shard_2, the shard with the most room):
`in8_a_whole_escort_scrapped_at_stickney_pays_for_its_parts_and_a_tonne` — the escort's partition is the document's;
a captain's Dray-P sails to Stickney Yards with a titled spare escort and a titled heavy; at P11 `+bay/scrap` breaks
the whole intact escort up: 1,539 kg of yield, a 1 t lot in the Yards' stock and 539 kg in its shop bale, its four parts
(the cannon, the coax, the sight, the plant) as lots in the Yards' stores, nothing in the Dray's hold, and about
45,200 cr paid of its 75,500; a Dray counts no heat points and carries 94 t of yield; a ten-point patch on the
heavy draws its own figures from the bale it opens, and `+bay/return` sells the rest back, the shop bale minting a
tonne as it passes 1,000 kg, the alloys conserved. `in8_an_escorts_remains_on_ahuna_yield_a_tonne_and_a_full_hold_refuses`
(256) and `in8_a_drays_remains_on_urvara_yield_forty_tonnes_and_a_full_hold_refuses` (512): remains dropped beside the
captain's rig scrap into the hold at 1 t and 40 t, and a full hold refuses the next at the command. The arithmetic
is pinned by `definitions::mass::tests` (the escort's and the Dray's lines) and `port::bay_work::scrap::tests` (the
escort's 45,217.55, the Dray's 34,100, the wrecked Dray's 18,000, the 914 kg return at 411.30 and its second return
minting a tonne). Seeds: the hulls' title and bay placement, the heavy made by `@unit/create`, the dents, the
remains (GC.6's staff drop) and the holds' goods.

**Departures from the row.**

- *The scrapped parts sit in the stores but are not yet drawn by a `Replace`*: IN.6 finds store lots by the seeds'
  ids; the stores' index is IN.7/IN.9's.
- *The bill's build line* is §4.4's yard row as constants (68.75 cr a work unit, 25 %) until IN.9 prices the build
  from the yard facility's own cost row; the build days follow the rule (the Dray's 12, not the table's 14, which
  Q-39-14 left to the owner's eye).
- *"1 t in the Yards' pool"*: the tonnes are minted as a lot at the operator's storefront, as every bay mint lays its
  lots; nothing dissolves a bay lot into the pool yet (`@inv/dissolve` does, and the stock clerk dissolves only its
  own seeds). The matter is the Yards' either way.
- *The return's 914 kg* is the tender's two-patch figure (`surface::tender` tests); the story's heavy draws its own.
- *No bot vocabulary*: `+bay` stays outside the bot's (the IN.1/IN.6 stories' precedent).

**Other tests moved.** `shard_1/bay_gc8.rs` `gc8b_scrap…`: the alloys and parts are in Occator's storefront, the
alloy lot a whole escort's tonne, the parts at rows 2…; `shard_1/bay_36a.rs` `b36a_5_a`/`5_b` exercise the hold's
room through a strip (a workshop scrap lays nothing in the hold now); `shard_1/field_gc6.rs`'s remains of a rig scrap
at their design's yield; `shard_2/in1_accounting.rs`'s heavy patch draws the heavy's figures; `shard_10/voyage.rs`
drops `industrial_parts_t`; mush-tactics' scrap test reads 60 points; the tender's unit test the escort's figures.

**Cell delta (rule #14).** `tacsim-integration-2`: **+1.37 min** for the three new stories (11.5 s Stickney, 35.6 s
Ahuna, 35.1 s Urvara on PR run 41838007042's JUnit). The cell's sum reads 21.51 → 30.83 min, main's run at the merge
base (41827163605, `4390f04c`) against the PR's, 414 → 417 tests: runner speed outweighs the stories, as IN.6's note
found; the moved tests do no more work than before.

## IN.9 — the yard and the handover (Q-39-14, Q-39-18, Q-39-19, Q-39-20, Q-39-33, Q-39-36)

**Built.** The expensive-to-reverse marker cut (Q-39-33): the register and the provenance, built as ruled.

- **The register** — `tacsim:owned_hull:{hull}` (`port::owned::OwnedHull`): the hull, the design, the fresh mark
  (`SYB-01`, `SYB-02`, …, one a commission, never re-issued), the home port, the provenance id and the lay-up row
  (`OwnedBody::Ship(HullRow)` or `Rig(RigRow)`). It is never a programme row: the applier's preflight and `hull_line`
  do not read it, so the pool stays the seed. It is read **after** the programme and yard rows by `hull_view`,
  `hull_of_room`, `helm_of` (a hull titled to the pilot or to an org the pilot `acts_for`, lowest id first),
  `world_hulls` (so `tow.rs::carriers_at` and the second-carrier recovery see a yard-built carrier; `tow.rs` is not
  edited), `bay_carriers` (RS.2's seam behind `carrier_of_bay` and `facility_of`), `carrier_right` and `right_of`
  (an org's officer works its hull as title holder), `hopper_of`, the rig tools and battery reads and `world_rigs`.
- **The provenance** — `tacsim:hull_provenance:{order}`: the order, the work order, the hull, the facility, the
  design's pack digest, the kit lot, the part lots fitted and the instant.
- **The commission** — `tacsim:commission:{order}` (`port::commission::YardCommission`). `+yard/collect` on a
  delivered hull order writes the record first, then owes one `tacsim_works` move (`Then::Commissioned`) that
  **consumes** the kit lot (its `workorder.output` lineage checked, the relay foundry's shape) and the part lots in
  the escrow, through the new `InventoryFamilies::consume_products` (the census and tallies summed per key: the
  inventory stages an endpoint's lot census once a transaction, so two `consume_product` calls at one escrow are
  refused). When the works ledger shows it landed the record steps on, and the port tick (`commission::tick`, run
  before `works::tick` so a consumption settled in a fire is laid up on the next) runs Laying → Placed → Titled →
  Vault → Emptied → Provenance → Registered → Berthed, each idempotent and recorded. The lay-up is the applier's own
  `hull::lay_up_one` / `vehicle::lay_up_rig`, now writing through a `HullRowSink` / `RigRowSink` (the port map's
  `MapRows` / `MapRigRows`, or the commission's record). A ship is placed docked on the yard port's domain with a
  hold vault the beneficiary's; a rig stands on the port's workshop floor (a location, no simulation placement, as a
  berthed hull does) with a yard berth row naming no carrier. Magazines are emptied by one plan (Q-39-36).
- **The bill from the design** (`port::bill`, Q-39-14): §4.8's partition and §4.7's bill. The escort is 3 t of
  alloys, 1 t of fittings, four parts and the line core, two days; the Dray and the Dray-P 163 t, 55 t and a hauler
  plant, twelve days (doc 39's 2,565/855 kg and 157,500/52,500 kg). The hull recipe pack `tacsim.industry.hulls` v1
  (`content/industry/hulls-v1.json`, in `game_seed_sources`) carries one recipe a buildable design (the bulk lines,
  `work_units` = days × 24, one kit `output_lots: each`, `requires: tacsim.yard`), pinned row for row to
  `design_bill` by `port::bill`'s test. A yard order's bill adds the design's part lines; a part is delivered as its
  lots.
- **Kits** — goods v6: `class: kit`, five `tacsim.hull_kit.<design>` rows (escort, prospector, tender, Dray,
  Dray-P), no mass of their own and stowage `stow.kit`, which no hold accepts (the gauges never list them);
  `MAX_GOODS` 32 → 48; never orderable on the market.
- **`+bay/stow <hull>[=<carrier>]`** (§4.13, §8.4: this document's verb): the named carrier docked here, the actor's
  run of her, a lessee or salvor only to the row's own carrier, a free bay slot whose mass limit takes her catalogue
  mass; the custody refusal ("The <name> is in <custodian>'s custody; only they may move her.") before the right.
  `BerthRow.carrier` and `bay_room` are optional (old rows read `Some`); `+bay` reads a yard berth as "Built on this
  floor". The first rearm draws from a carrier the hull's title holder holds docked at the port.
- **The title view for rigs** (`owned::title_view`): a commissioned rig in the bay of a carrier the pilot has the run
  of, or out on the ground by her `tacsim:tv:{tv}` projection's hull, read as `TvLease::house_view`'s shape with the
  carrier's hold, by `rig_lease`, `live_on_hull`, boarding and the surface act's rig readers. `repossess` skips a
  titled rig, and `bay_slot_word` (the buyout's and refit's "still in the bay") never counts one.

**The story.** `shard_6/yard_in9.rs`, three tests through the production commands:

- `in9_the_yard_builds_a_dray_p_and_an_escort_and_hands_them_over` (port-side): a captain who enrolled and handed
  the charter straight back orders a Dray-P at Stickney; twelve days later `+yard/collect` hands her over docked,
  titled to them, `SYB-01`, a register row and a provenance, and `helm_of` gives them her helm. `+yard/quote
  tv_escort` prints the design's bill and two days; the bill comes from the Dray-P's hold in two deliveries (the bulk,
  then the four part lots); two days on, `+yard/collect` writes the commission, **staff stop it after the lay-up and
  the placement** (no title, no register row), and the next fire finishes it. The escort stands on the workshop
  floor titled to them, magazines empty, the provenance naming the four lots; `+bay` quotes her clean but for the
  rearm ("0/2000 rounds", "Built on this floor"); `+bay/order …=rearm` loads her from the Dray-P's hold;
  `+bay/stow escort=<Dray-P>` puts her in the Dray-P's bay, and `facility_of` names the Dray-P. Granted office in the
  Co-op, the captain orders a second escort `=org:<coop>`, collects it titled to the Co-op, stows it as its officer and
  reaches it by its title view; a crewmate without office reaches neither.
- `in9_ahuna_…` and `in9_urvara_…` (256 and 512): a second captain's chartered Dray-P stands on the pad with a leased
  tug. The yard builds the first captain a Dray-P and an escort; she is stowed, the captain boards the Dray-P at
  Stickney's gallery (`+ship/board`, the register's helm) and sails by Ceres to the site, walks to the bay
  (`+tv/bay in`), `+tv/1 board` and `+tv/1 launch go` by the title view. Disabled and indexed (GC.6's staff fixture),
  the escort is towed by the second captain's tug and loaded into **their** programme Dray-P; that Dray-P sails to
  Ceres, returns its own leased tug, and `+coop/buyout` writes the title transfer with the titled escort still in her
  bay.

Seeds: the Dray-P's bill and, on the ground, the escort's in their escrows; the escort's bill and the munitions in the Dray-P's hold; the
money; the officer's grant; the captain's move to Stickney; the escort's disablement and its index row.

**Departures from the row.**

- **The lane's `yard` row is at P12 Observation, not a new `room.p14_slipway`** (lane v27, relay v18, over
  IN.7's v26, IN.5's v24 and CR.2's v23): a `PortDef.yard` block of IN.5's `WorksDef` shape under `tacsim.yard`, opened
  by the applier's part m (`plant::rows`), found by `+yard` through `plant::block_room`. A new atlas room would
  bump Stickney's atlas pack v4 and the Phobos exchange pack that requires it; the gallery over the drydocks is
  the slipway's counter. v27 also carries **Stickney Yards' raise of 50,000** (IN.7's, folded in on the owner's
  ruling: it funds the Yards' strategic bid, so IN.8's whole-escort scrap can buy back its bale; the tranche count
  in `plumbing.rs` goes 5 → 6 and the treasury in `ports.rs` to 16,001,000.00 cr). `lane_chain::strip_yard` heads
  the strip chain and takes out both (`in9_strip_yard_rebuilds_the_published_v26`).
- **`armour_cab` and `walker_cab` were not in this cut** (built since in IN.9b, below). Giving the line-tank family, the heavy, the scout and the
  bipeds an authored `logistics` changes every fielded tank's dry mass from the field-ops measure to whole tonnes,
  which moves the tow masses and IN.8's per-point figures for those designs; it is a follow-up with its own review.
  The five designs that already carry both blocks are the buildable ones, and only they have kits and recipes.
- **`+yard/deliver` buys no parts at the port's asks**: the lane authors no part asks (IN.6). A part line comes from
  a lot the requester holds in the docked hold.
- **"The bot types it"**: the stories type every player line through the rig's command path (`run`), the field
  stories' precedent; no `mush-bot` vocabulary is added (IN.11's).
- **A pilot with two hulls**: `helm_of` takes the charter, then the programme rows, then the register; `+ship/helm
  <hull>` (doc 18 P2's seat) is not built. The story's captain hands the charter back so the yard-built Dray-P is
  their one helm.

**Fixed after the reviews (docs 38a/38b, 2026-11-05).**

- `+yard/deliver` takes only clean, whole part lots: the provenance reads, never fitted, not hot, intact,
  unheld (38b F-07). A hot, held or damaged lot stays in the hold.
- `+yard/order` and `+yard/quote` refuse a quantity other than one (`YARD_ONE_HULL`, 38a #3). `book` refuses it too.
  Before this, the order billed N hulls and the commission built one from one kit.
- `escrow_contents` lists a part only as its lots, never as a bulk line (38a #4). Cancel, collect and fail-back
  return a delivered part lot.
- The storage seizure moves the kit lot (38b F-17). Before this it refused and retried every fire.
- `next_mark` knows a house seed by `work_order == 0` and steps past its indices (38a #10). Before this, marks
  repeated from the 65th commission on.
- `+yard/quote` quotes the part lines (38b F-18(a)).
- The programme's hull readers (`live_charter`, `helm_holder`, `check_hull_name`, `row_of`) read the
  owned-hull register too, through `coop::programme_rows` (38b F-06, F-22c).

**Owed / flagged.**

- **The core is never billed (38b F-18(b)).** `DesignBill.core` names the design's core class, but there is no core
  good yet, so no bill line takes it and a built hull's core is free. The cost model's core good and its bill line
  are owed.
- **A used part's condition** (38a #6's larger half). Deliver now refuses a damaged lot. But a lot that reaches the
  escrow by another path (a staff seed, a later part-ask flow) is still built in, and the hull's slot comes up
  intact. Carrying the lot's `condition` onto the commissioned slot is owed (Astra's
  `review_72h_yard_keeps_a_used_parts_condition` probe).

**Cell delta (rule #14).** `tacsim-integration-6`: **+2.76 min** for the three new tests (the port half 48.0 s,
Ahuna 64.5 s, Urvara 53.3 s on PR run 41850018642's JUnit, head `e6b0f82c`). The cell's sum reads 46.35 → 50.16 min
against main's run at the merge base (41827163605, `4390f04c`), 80 → 83 tests; the other +1.05 min is spread over
the unchanged tests (runner speed), none of which this cut touches.

## IN.10 — the pools (Q-39-7, Q-39-21, Q-39-34)

**Built.** `port::pools`, one record a house (`tacsim:house_pool:{org}`, `HousePool`, written whole, an unreadable one
orders nothing) and one arm on the port tick after the works arm, one step a house a fire, every step re-derived
from durable state.

- **The floor** (`pools::FLOORS`): the Trust's two free Drays at Ceres. `coop::free_hulls` now counts the Trust's
  built Drays too — register rows of the programme's unit under the same `hull_is_free` rule — so the floor and the
  desk read one list. A house whose free hulls and orders standing are under its floor orders **when its account
  covers the order** (`pools::price_minor`: the bill at Stickney's asks and the parts' catalogue price, the yard's fee
  and booking fee, and the ferry's stores at an allowance of 120 t of propellant and five crew-days). On the shipped
  lane a Dray is 165,290 cr.
- **The helm**: a `DARK` Thing `"coop_trust ferry helm"` (the Occator Yard dispatcher's shape, `site::dark_thing`)
  with **its own credit account**. The industry's budget rule makes a work order's account its requester's own
  (`check_wo_accounts`), so the house funds the helm with the order's price in one post
  (`tacsim.house_order/{org}/{n}/fund`) and the helm books and pays as itself. The order's beneficiary is the house,
  so the hull is the Trust's from the commission on. Once the last order is home the helm's unspent purse goes back.
- **The order**: `works::book` (factored out of `+works/order`/`+yard/order`) at the Stickney yard, as the helm, for
  the Trust; the bill bought line by line, paid under `tacsim.house_order/{escrow}/{good}/{held}` (the count the
  escrow holds names the purchase, so a restart finds the payment and never pays twice), the seller's standing ask
  lowered first (IN.5's `shrink_ask` rule), then moved through `works::supply_from` under the order's own ledger.
  A part is one lot from the yard operator's stores (the Yards'), at its catalogue price: IN.7 retired the seeded
  part stock, so the stores hold what was made, scrapped or sold into them, and the order waits while they hold
  none. The works arm runs the order; the helm collects it into IN.9's
  commission (`works::commission_as`, the route IN.9's note asked for).
- **The ferry**: the passage's burn with a quarter over and its crew-days are dry-run (`voyage::passage_needs`) and
  bought into her hold the same way, under the works ledger keyed by the hold (`pools::owed_moves` in the works
  offer). Then the helm files `+ship/undock ceres` as itself (`voyage::undock_as`). `coop::helm_of` reads
  `pools::ferry_hull` on `Fleet`'s precedent of a helm without a charter, `voyage::aboard` holds the helm aboard
  wherever it stands (the dispatcher's precedent), and `prepare_departure` counts the house's hands as one soul.
  She docks at Ceres a free hull of the pool, a register row and never a programme row.
- **`NO_HULL`'s two texts**: "The Co-op has no Dray free to charter." then "The Trust has no Dray free and cannot yet
  fund one." (`NO_HULL_UNFUNDED`) or "The Trust has a Dray on order at Stickney Yards, due at Ceres in n IC days."
  (`NO_HULL_ON_ORDER`, from `pools::next_due`: the build's days and a four-day passage allowance until she sails,
  then her filed arrival).
- **The raise**: lane v29 / relay v20 (on RS.6a's v28) — `coop_trust` gains `raises: [250000]` (Q-39-34), so the Trust boots at
  500,000. `lane_chain::strip_trust_raise` heads the strip chain (`strip_tycho_house` calls it first), and
  `in10_strip_trust_raise_rebuilds_the_published_v28` holds it to RS.6a's v28 digest. The pins: `LANE_DIGEST`
  `7c1b822f…`, the relay `2afc58ea…`, the treasury's 16,701,000.00cr, and the version asserts.
- **`works::port_of` reads the yard room** (IN.9's row is a room that is the facility itself), so an order at the
  yard finds its port; the house collection needed it.

**The story.** `shard_2/in10_pools.rs` (port-side; no site, so no 256/512 pair):
`in10_the_trust_restocks_its_pool_from_the_yard_and_ferries_her_home`. Eight captains enrol; the Trust is at 50,000
and orders nothing; the ninth hears `NO_HULL` and the unfunded text. Two buyouts bring 180,000 home, and the Trust
orders on the next fire: the DARK helm books the Dray at Stickney for the Trust and buys the bill into the escrow;
the ninth hears "on order at Stickney Yards, due at Ceres in 16 IC days". The yard hands her over 13 IC days after
the order as `SYB-01`, titled to the Trust; her stores are bought, the helm files the passage, and she docks at
Ceres a free register hull while the programme map still holds eight; the helm's purse is empty. The desk then
enrols the ninth captain. Over those weeks the dormancy recall (seven real days on the rig's clock) may hand a
programme Dray back first, and the desk lends the lowest free id, so the story asserts the ninth captain is
chartered and the Trust's Dray is theirs or still free. The seeds: one hauler plant in the Yards' stores, the
captains' buyout money, and the Trust's raise held back in the treasury by staff from boot to the ninth enrolment
(the world the row reads is the one before v29; with the raise in from boot the Trust would order at the seventh
charter, which is what the raise is for). 22 s locally.

**Cell delta (rule #14).** `tacsim-integration-2`: **+1.79 min**, 17.25 → 19.04 summed minutes against main's run at
`458304cb` (41936816433), 419 → 420 tests; the new story is 0.40 min (23.9 s on PR run 41937075526's JUnit), the
rest runner speed on unchanged tests.

**Owed / flagged.**
- Occator's floors (two free rigs and one escort) ride a ferry Dray-P's bay, and no house keeps one at the yard, so
  `FLOORS` carries the Trust's row alone.
- **A house restock of the stores** (IN.7's flag): nothing refills the Yards' part stores, so with the seeds
  retired a house Dray waits on a hauler plant someone made or scrapped there. Ordering the plant at the works
  (`plant_hauler`, 20 days, carbon from Ceres) inside the house order would break the row's fourteen days; left for
  the owner.

## IN.11 — the matter story and the adversarial set (Q-39-8, Q-38-29)

**The census's shape** (the owner's ruling, relayed 2026-11-04: option (a)). A world's seeded stock carries no lineage once
its lots are pooled, so §4.10's rule is held **across a window**: `@industry/census` (staff, read-only;
`tacsim::port::census`) prints one `tacsim.matter_census` line of absolute kilograms and a story holds the difference of
two. RS.9 reads the same line.

- **Sources**: the opened deposits' reserves (`opened`; mining is a transfer out of a deposit's vault, so `mined` =
  `opened` − `in_ground`) and the stock clerk's mints (`stock`, the `tacsim.port_stock` receipts).
- **Holdings**: pools and lots of the chain's goods outside the ground (the ores, the refined goods and the parts;
  volatiles, propellant, provisions and munitions are outside the chain), the hulls' bales and the shop bales.
- **Sinks**: completed work orders' inputs less outputs (`worked`; at a yard the hull's bulk, a kit being massless),
  the mills' ore less product (`milled`), and the parts commissions consumed (`built`).
- `balance` = sources − holdings − sinks. One `tacsim.matter_build order= inputs= hull= waste=` line a commission
  holds §4.9's "inputs = the hull + the waste" (inputs: the bill's bulk, its parts and its core).

**Measured.** On a settled shipped world the balance is **0** — every stock mint is a receipt and every pool is
counted. Across the IN.10 Trust order (purchases, the 12-day build, the commission) it holds at −30,000 kg on both
sides: the staff-seeded hauler plant, a mint the census cannot see, which is the point of a window. Occator's
yard drones mine through the window (mined 525 t → 878 t) and the census conserves it.

**The census's terms, as built** add `fitted` (the bay's fitted parts: a fit consumes the lot and keeps its provenance;
a plant's good by Q-39-35's spelling, the slot index not holding plant rows) and count only the chain's mills (Occator's
ice mill is outside it). `@industry/census <good>` adds one `tacsim.matter_lot` line a lot of that good (its vault, the
vault's name and owner): a staff diagnostic.

**The lane** (the owner's choices of 2026-11-05: a strategic-ore ask and part bids). **Lane v32 / relay v23** (claimed as v30; the player-response floor took v31 first, so re-signed on it):
- Freeport Works stocks 200 t of strategic ore and asks 800.00cr at Ceres (above the 600 and 700 bids), so a sight is made
  from bought matter. The programme's strategic-ore reference moves from its fixed 300.00cr to that ask.
- Stickney Yards bids for 4 light plants at 4,000.00cr.
- `strip_strategic_ask` heads the strip chain (`strip_licence_grace` calls it first) and rebuilds the published v31. Pins:
  `LANE_DIGEST` `aa5d90fb…`, the relay `727c74f4…`, eighteen stock receipts, 66 readiness lines, the Exchange board.

**A part lot sold into a bid** (the engine had none):
- mush-inventory: `relocate_marked`, one lot moved under an applied `inventory.transferred` marker.
- mush-market: an indivisible good is held as its free lots (`sale_lots`), a sale into a bid freezes one lot on the
  quote (`ONE_LOT_A_SALE` refuses more than one), and the Deliver prepare hands that lot over (`prepare_deliver_lot`),
  refusing a held lot in its own transaction. Which goods sell as lots is the inventory's indivisible table **or the
  presence gate's** (`VenueGate::indivisible`, tacsim's answers `is_part`): on the live server the market's copy of
  the inventory families read no part as indivisible (the bot story's finding), and the gate is the seam that is known
  to be composed there. An unreadable hold or lot family is never read as no lot.
- The burn's prepare counts a part's lots as the planner does (`burn::pool_of`), so a hold with a part aboard burns: it
  refused "the hold changed since the passage was planned".
- `+bay/fit <hull>=<slot> <part>` takes a part's good in place of a lot id (its first free lot in the carrier's hold):
  a player sees the part, not its lot id.

**Built** (`domain_economy/in11_matter.rs`, in-process):
- `in11_probe_the_census_holds_still_on_a_settled_world`.
- `in11_the_port_half_ore_to_parts_to_a_prospector_conserves_matter` — §4.9's port half, once: concentrate, carbon and
  strategic ore at Ceres; Kasei refines at Stickney; the works make fittings, five light plants and ten sights; the yard
  builds a prospector from them; a plant sells into the Yards' bid; the prospector's plant destroyed (staff), she is
  stowed in the Dray-P, berthed from it and a plant fitted from the hold. The census holds at −32,890 kg either side
  (the seeded boards and the boot builds of Tycho's house), every build reads inputs = hull + waste.
- `in11_a_light_plant_lot_sells_into_the_yards_bid` — one plant sails aboard, a second is seeded at Stickney, one sold,
  the lot in the Yards' stores, `=2` refused, the census unchanged.
- `in11_a_restart_between_consumption_and_title_finishes_the_hull_once`; `in11_two_orders_for_one_bale_draw_it_once`.

**The bot's story** (`mush-bot/tests/mining_cohort/in11_matter.rs`; `in11_the_bot_mines_and_builds_a_prospector_from_`
`ahuna` and `_urvara`, `#[ignore]`d, by-hand tier): the mining loop to the hold sold at the Exchange, then the whole chain
above typed over SSH — boards bought at Aitken on the Luna–Deimos way out (IN.5's route round Kirkwood), the walk to the
Service Counter, God's `@unit/damage` for the plant, the bay's quote, stow, berth and fit by the bot. God types
`@industry/census` when the houses' boot builds have settled and at the end; the balance does not move. The bot's
lines: `+yard`, `+market/sell <n>=1`, `customs`/`service`, `+bay <mark|#n>`, `+bay/quote|berth #n`, `+bay/stow #n=<hull>`,
`+bay/fit #n=<slot> <part>`; readers `yard_booked`, `commissioned`, `bay_ref`. About 29 minutes a site locally.

**Owed / flagged.**
- **A competing reservation at the deadline**: as built, every order's materials are its own escrow's, so two
  reservations never compete for one pool, and an overdue reservation is the sweep's; the row's story needs the owner's
  reading of what competes.
- **Repairs and remains scrap in the census**: a repair consumes bale kilograms into the hull and nothing durable sums
  them, and remains scrap's sub-tonne is undeclared, so a window with either does not balance — the case for **the
  durable matter ledger (option (b))**, named as a future cut.
- The market's inventory cell on the live server reads no part as indivisible; the gate covers the sale, and why the
  cell differs (a second registration of the `OnceLock` at boot, likely) is a follow-up.

## IN.9b — the class templates (Q-39-14, Q-39-20, Q-39-35)

**Built.** IN.9's deferred row item: `armour_cab` and `walker_cab` (§4.13).

- **The templates** — `content/industry/cab_templates.json` (a table, version 1), loaded and checked by
  `definitions::templates`. Each is a cab room with the console and a kit locker as the stowage (120 kg, 300 L of
  provisions), one berth and no drive, and names the designs it applies to: `armour_cab` the line-tank family
  (`line_tank`, `_ecm`, `_eccm`, `_laser`, `_thermal`), the heavy and the scout; `walker_cab` the two bipeds. The
  drone, the carrier, the target, the relay station and the capital hulls take none (Q-39-20, Q-39-35). A design
  that authors either block and is named by a template is refused, as is a template whose class is not its
  design's, and a design named twice.
- **Applied at the catalogue build, to the stored spec only** — after the unit's binding and IN.8's mass budget
  are made, so neither moves: no unit pack changes, no binding or arena digest moves, and every budget, bill and
  per-point figure is what it was.
- **No mass moves.** A templated block carries `LogisticsDef::derived_mass` (never on the wire): its `dry_mass_t`
  is the design's field-ops mass rounded up to whole tonnes for the readouts that print tonnes, and
  `field_ops::dry_mass_kg_for_spec` keeps returning the structural-point measure for it. A line tank tows,
  loads and partitions at 29,900 kg as before; only the readouts round it to 30 t.
- **A templated tank is still a combat hull** unless the yard built it: `vehicle::combat_hull` and `carried_rig`
  read a template's logistics as no logistics for a unit with no register row, so a fielded scout or tank in a
  bay is inert cargo at its field-ops mass, as before (IN.7's story carries one to Luna); a yard-built one is a
  rig with its locker.
- **Buildable** — the hull recipe pack `tacsim.industry.hulls` v2 (`content/industry/hulls-v2.json`, which
  replaces v1 in `game_seed_sources`) adds one recipe a templated design beside v1's, each pinned to
  `port::bill::design_bill` by its test: the line tanks 20 t of alloys and 7 of fittings (the laser 21), two days;
  the heavy 34/12, three; the scout 12/4, two; the bipeds 34/15 and 39/17, three. Goods v7 adds the nine
  `tacsim.hull_kit.<design>` rows (45 goods). The commission lays a tank up as a rig on the workshop floor, by its
  template's cab and locker.

**The story.** `shard_6/yard_in9.rs::in9b_the_yard_builds_a_line_tank_and_a_scout_by_their_templates`: a free
captain has the Stickney yard build a Dray-P, a line tank and a scout car (their bills laid in the escrow, the days
stepped, `+yard/collect`). Both tanks stand on the workshop floor titled to them with the template's cab and
locker, and keep their field-ops mass (29,900 and 15,900 kg). `+bay/stow <line tank>=<Dray-P>` is refused, "she
masses 30 t and the slot takes 20 t"; the scout stows into the Dray-P's bay and `facility_of` names the Dray-P.

**No lane rows**: the templates, the recipes and the kits are content tables and packs of their own; the lane
stays at main's version.

**Pins moved.** `shard_6/vehicles.rs`'s dark-view and no-interior fixtures take the target (`target_mk1`, which
no template names) where they took the line tank; `shard_2/content.rs`'s goods table (v7, the nine rows, the digest), `shard_3/capacity.rs` and
`content_studio`'s goods counts (36 → 45, tacsim and mush-server).

**Cell delta (rule #14).** tacsim-integration-6: _filled from the PR's JUnit_.
