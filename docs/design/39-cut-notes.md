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
