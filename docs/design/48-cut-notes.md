# 48. Economy and production — cut notes

One section a unit of [doc 48](48-economy-and-production.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

## EX.0

Not built.

## PL.0

**Survey** (at `a5018131b`):
- **The goods cap.** `MAX_GOODS` was 48 (`tacsim/src/definitions/goods.rs:59`), read only at the
  load bound (`:282`). `goods_digest` hashes `goods.json` (`:624-629`), and
  `ic2_the_goods_table_is_pinned` (`tests/domain_arena/content.rs:3324`) pins that digest, so the
  bump moves no pin. No test names `MAX_GOODS`. The one goods count in a test,
  `mush-server/tests/split_boundary/content_studio.rs:194` (46), counts `goods.json`'s rows, not
  the cap. The market takes 512 catalogue rows a pack (`MAX_CATALOG_ENTRIES_PER_PACK`), so 256 fit.
- **The pool cap.** `MAX_POOLS_PER_ENDPOINT` was 64 (`mush-inventory/src/record.rs:31`). It is
  counted on create in `apply_counted` (`pool.rs:463-470`) and in `transfer_many` (`:674-705`), and
  never decremented. An emptied pool keeps its pair (balance and reservation book) and its lineage
  root. The recover's finish checks a hopper's census against the pools it can name
  (`surface/recovery.rs:1211-1238`).
- **The unload plan.** `UnloadFixed.goods_of_hold` listed every good the hold's ore compartment
  accepts (`surface/act/fixed.rs:426-432`): 27 on the Dray-P (23 bulk, 4 ore). `start_unload`
  refused a list over 64 (`providers.rs:2101-2107`) and read the hold's pool of each to weigh the
  room left (`:2184-2195`). A rig's hopper accepts `stow.ore` only (4 goods); the Dray-P's cargo
  accepts `stow.bulk` and `stow.ore`.
- **The parameter bound.** `MAX_ACTION_PARAMETER_BYTES` (64 KiB) bounds an offer's fixed
  parameters (`mush-core/src/offer.rs:733-736`). Each rig the surface offer fixes carries an unload
  plan. The old list was 616 bytes a rig today, and would be about 6.4 KB a rig at 256 goods (about
  25 KB for four rigs).

**Built:**
1. **`MAX_GOODS` 48 → 256** (Q-48-30). A `const` assertion beside it requires
   `MAX_GOODS <= mush_inventory::MAX_POOLS_PER_ENDPOINT`. A larger catalogue will not compile until
   the pool cap moves with it.
2. **`MAX_POOLS_PER_ENDPOINT` 64 → 256: the cap is sized to the catalogue, and an emptied pool
   still counts.** Why this and not freeing on empty:
   - An emptied pool is not an absent one. Its reservation book keeps terminal records, and its
     lineage root carries the slot's tranche generation.
   - Freeing it would need three new rules: a pair is deleted only with no live hold; a re-created
     slot gets a lineage; and the census delta is netted per bundle, since an `ActionTxn` writes a
     census key once (`transfer_many`'s rule). That is an E7c2 change to a hardened crate.
   - Freeing still caps the goods actually held at 64. A hold or warehouse holding 65 distinct goods
     of a 256-good catalogue would still refuse. Sizing to the catalogue removes the refusal for
     every catalogue good.
   - The commodities tacsim pools are goods rows (only tests seed other ids), so in play a count
     cannot pass the catalogue.
   - The cost is small: the census is one `u32`, and nothing preallocates by the cap.
3. **The unload plan names the hopper's goods only.**
   - `goods_of_hold` is gone from `UnloadFixed`.
   - The prepare derives the hold's list from the plan's own table and the one ore compartment it
     already resolves (`fixed.rs`, `hold_goods`). The same table chose the compartment, so the
     derived list is the one the offer would have carried.
   - The weigh is one function, `hold_admits`, which `start_unload` calls.
   - The provider's bound now covers the plan's lists: at most `MAX_BAY_ITEMS` siblings, and at
     most `MAX_GOODS` goods in a sibling hopper's list.
   - The plan no longer grows with the catalogue.

**Tests** (run on conclav.dev):
- **New:**
  - `mush-inventory`, `pool::pl0_tests::pl0_an_endpoint_pools_a_whole_catalogue`. One endpoint
    pools 256 distinct goods, one bundle each. An emptied pool still counts. The 257th refuses and
    writes nothing, and the emptied pool takes stock again.
  - `tacsim`, `definitions::goods::pl0_tests::pl0_a_catalogue_of_100_goods_loads`: the shipped
    table padded to 100 goods loads, and its 100 market catalogue rows derive.
  - `tacsim`, `definitions::goods::pl0_tests::pl0_the_catalogue_holds_256_goods_and_no_more`.
  - `tacsim`, `surface::act::fixed::pl0_tests::pl0_an_unload_weighs_a_hold_under_70_bulk_goods`.
    A test catalogue with 70 bulk goods gives a 74-good hold list. The prepare's weigh counts bulk
    goods no hopper carries, takes the siblings' reservations off the room, and treats an
    unreadable pool as an error.
  - `tacsim`, `surface::act::fixed::pl0_tests::pl0_the_unload_plan_names_only_the_hoppers_goods`.
    A hand variant with three sibling hoppers encodes in under 4 KiB (a sixteenth of
    `MAX_ACTION_PARAMETER_BYTES`) at any catalogue size. A plan that still carries `goods_of_hold`
    is refused.
- **Unchanged and green:**
  - `domain_economy` `unloading::` (24 stories through the real `+tv/unload`).
  - `domain_field` `sm3b_a_dump_the_spoil_will_not_take_is_refused_and_never_wedges`: its b2 part
    now seeds 255 filler pools, 9.5 s.
  - `domain_arena` `ic2_` and `ic8_`, including `ic2_the_goods_table_is_pinned`. No pin moved.
  - `mush-inventory` `e7c2` `vf_b8_` and `sm6` (4), which count the pool cap through the constant.
  - The surface act's `sm8_` and `sm3b_` unit tests (42 lib tests in all).
- **Gate:** clippy `-D warnings` on both crates with `--all-targets`, the M7 memory-size pass on
  `tacsim`, rustdoc `-D warnings --document-private-items`, and `mush-contract-tests`' feature
  census, panic budget and restriction lints (24).
- **Mutations** (each change's test fails without it):
  - `MAX_GOODS` back to 48: both goods tests and the unload weigh test fail (the padded
    catalogues refuse to load).
  - `MAX_POOLS_PER_ENDPOINT` back to 64: the inventory test fails at good 64 ("an endpoint pools
    at most 64 distinct commodities"), and `tacsim` does not compile (the `const` tie).
  - The old 64-good bound restored on the hold list, inside the weigh: the unload weigh test fails.
  - The weigh over the hopper's goods only (the brief's literal reading): the unload weigh test
    fails, since 40 t of ore are admitted beside 47 t of bulk.

**Divergences:**
- **The hold list is not the hopper's goods.** The brief said the unload plan should take the
  hopper's goods, as dump does with `hopper_goods_of`. But the hold list weighs what the hold
  already holds: 27 goods on the Dray-P against the hopper's 4. Narrowing it to the hopper's goods
  would ignore bulk aboard (ice or metals bought at a port), and the prepare's re-check would then
  over-admit. So the plan carries only the hopper's goods, and the hold's are derived in the
  prepare. The weigh still reads every good the compartment accepts.
  `pl0_an_unload_weighs_a_hold_under_70_bulk_goods` asserts the difference: 39 t fit, 40 t do not,
  where a weigh over the ores alone would take 86 t.
- **The 70-bulk-goods unload is tested below the verb.** The unload weighs with `plan_goods`, the
  compiled-in table chosen by digest. No seam sets a test table under the real `+tv/unload`
  (`surface/act/tests.rs:722-727` says so). A seam would touch `port/market.rs` and the feature
  census, outside this unit's files. So the test drives the prepare's own weigh and the plan's
  encoding over a 70-bulk test catalogue. The shipped verb is covered by `domain_economy`'s
  `unloading::` stories, run unchanged.
- An unload offer made by an older build carries `goods_of_hold` and is now refused
  (`deny_unknown_fields`). Pre-launch, nothing holds one.

**Not built:**
- Stale wording in files outside this unit, left for their owners:
  - `mush-inventory/tests/e7c2.rs`: `vf_b8_an_endpoint_refuses_a_sixty_fifth_commodity`, its
    "65th" and "existing 64" messages (the test reads the constant and passes at 256);
  - `tacsim/tests/domain_field/excursions.rs`: the b2 doc comment, "would make it 65";
  - docs 08a and 10b: §7's bound of 64; doc 47's cut note: "47 of `MAX_GOODS`' 48".
- `transfer_many` keeps the pool cap as its move bound, so its pre-check now passes up to 256 moves.
  A bundle that large may reach one bundle's 1,024 mutation operations (not measured here), which
  refuses the whole bundle and writes nothing. Today's callers each move one rig hopper's goods
  (the dump, the unload, the recover and the tow's pickup), a handful of ores.

## PL.10

Not built.

## EX.1a

Not built.

## EX.1b

Not built.

## EX.2

Not built.

## EX.2b

Not built.

## EX.3

Not built.

## EX.4

Not built.

## EX.5

Not built.

## EX.6

Not built.

## EX.7

Not built.

## EX.8

Not built.

## PL.1

**Survey** (at `0123f796c`, PL.0's tip):
- **The mills.** A mill is a vendor's `mill` row (`MillDef`, `port/def.rs`), at most four a
  vendor, ratio at most 100. `check_vendor_mills` required the ore to be one of the vendor's
  **bids** and the product one of its asks, and a vendor never asks and bids one good. A
  cross-family conversion is a `kind: refine` row, which needs the port's `refinery` block,
  operated by that vendor (`check_refinery`); only Stickney had one (Kasei's).
- **The arm.** The stock clerk's turn (`stock_step`) runs `mill::mill_step` after the intake drip:
  a restock first (the product's drawn ask raised by the units milled and not yet listed, with
  the vendor row's `mill_listed` counter in the same outbox), else a batch (`tacsim_port_mill`:
  ore debited and product credited at one storefront under the `tacsim.port_mill` ledger). One
  act a fire.
- **Occator at Ceres.** It stocks 9,600 t of water and 5,000 t of propellant and asks both
  (100 cr and 40 cr a tonne). It bids for ice ore (intake 240 t, 40 t a day) and mills it 5 : 2,
  20 t a batch and 40 t an IC day. Volatiles and propellant are different families.
- **Does the clerk already do this for a mill row? No.** Three things stood in the way:
  1. the ore had to be a bid, and Occator asks volatiles;
  2. a batch debits the storefront pool, and Occator's water backs its standing Draw listing,
     so milling it would leave the listing unbacked (an oversell, paged as drift);
  3. Ceres had no refinery.

  So `port/mill.rs` needed a change: an own-stock draw, in the shape of the works' house
  purchase (`port/supply.rs`: lower the seller's ask first with `mush_market::shrink_ask`, then
  move the goods).
- **Burns.** A Dray departure burns about 37–41 t (doc 18: the `coast_hauler` law, 105‰ of
  departure mass). Five propellant rows of 5,000 t are the only supply; nothing makes any.
  `mush_market::MAX_QUANTITY` (100,000) bounds a listing.
- **Goods.** Propellant and volatiles exist in `goods.json`. **PL.1 adds no goods row** (PL.7
  adds the next one).

**Built:**
1. **Occator's refine** (lane v35). A second `mill` row at Occator: `kind: refine`, `volatiles`
   into `propellant`, **5 : 4**, `batch_t` 20, `per_ic_day_t` 20, `floor_t` 5,000. **The figures
   are provisional:**
   - 16 t of propellant an IC day, 384 t a real day at rate 24: about nine Dray departures'
     burns.
   - At most 480 t of water a real day, against the ice mill's 384 t a real day at full drip.
   - The floor is the seed: the refine only replaces what is bought, so the ask never floods.
     It runs dry only when burns outrun it for long: at twice its rate (about 18 departures a
     real day) the 5,000 t last about 13 real days.
2. **The Ceres refinery.** A `refinery` block, operator Occator, room
   `ceres_freeport.room.c02_pressure` (the Pressure Lobby), capability `smelt`, cost 3,000,000 cr
   over 30 years, upkeep 20‰, six staff at 150 cr, margin 0 (provisional; Kasei's shape).
3. **`MillDef.floor_t`** (optional, skipped when absent, so the ore mills re-encode
   byte-identically): any mill, an ore mill too, starts a batch only while its product's
   storefront pool holds less. An unreadable product pool reads as at the floor. The Mill line
   names it ("while propellant is under 5000 t").
4. **The own-stock mill** (`MillDef::own_stock`): a row whose ore is one of the vendor's own
   asks. `check_vendor_mills` accepts the ore as a bid or an ask, the product as another ask,
   and requires an own-stock mill to name a `floor_t` (1..=`MAX_QUANTITY`).
5. **The draw** (`mill.rs`, `own_stock_step`, on the mill's batch pass):
   - **Drawn:** the ore pool less the other mills' unlisted units, the listing and its undrawn
     Draws. With a listing standing and a batch drawn, the batch mills it, whatever the floor
     reads now.
   - **The draw:** otherwise, with the product under its floor, a batch of room today and a live
     listing that lists the rest of a batch, the clerk lowers the ask by the rest, as its one act.
     A refusal passes the mill over for the IC day and warns once a boot (`MillWarn::Draw`).
   - **Waits:** no listing; a Draw quote unsettled on the pool (the restock's wait).
   - **The restart argument** is the mill's: everything is read off durable state each fire.
6. **The drift check and the Listings line** net the drawn batch out (`mill::drawn`, at most one
   batch, and only with a listing standing). The line names it ("20 t drawn to be milled") and
   reads Ready, so the desk stays open across the draw.
7. **The lane bump.** Lane v34 → v35 and relay v25 → v26 (re-signed, requiring lane v35, entries
   unchanged). `support::lane_chain::strip_refine` is the strip chain's new head (`strip_wrecker`
   calls it). Pins moved: `LANE_VERSION`, `lane.rs`' `LANE_DIGEST` and
   `ic2_the_lane_pack_is_pinned`, `cost_lane.rs`' `SHIPPED_LANE_DIGEST` and its version asserts,
   the version asserts in `mill.rs`, `drones.rs`, `sites.rs` and `stations.rs`,
   `relay_registry.rs`' relay digest and kept-world versions, and `boot_parity.rs`' relay version.
   Three story pins moved with the new rows: `mill::sm12a_strip_mill_rebuilds_the_published_v13`
   (the shipped mill list), `ports::ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`
   (68 commerce lines, the floor in the Mill line) and
   `ports::ic3_port_entries_refuse_what_the_applier_cannot_bind` (the v2 shape drops the refinery
   with the vendors).

**Tests** (run on conclav.dev):
- **New:**
  - `domain_economy` `mill::pl1_occator_refines_water_into_propellant_one_full_cycle` (4 s, the
    shipped lane at world rate 1). It runs:
    1. At the seed the refine is idle, and the Mill line names the floor.
    2. 20 t of ice ore is sold into Occator's bid and milled into 8 t of water (9,608 t listed).
    3. 30 t of propellant is bought.
    4. The water ask comes down by 20 t in one act, the pool still holding them. No drift pages,
       the Listings line reads "20 t drawn to be milled" and Ready, and `+coop/enrol` still
       works.
    5. After a restart, one batch (ordinal 1, 20 t into 16 t), and the ask is not lowered twice.
    6. The restock lists the 16 t (counter 16), and the day's room allows no second draw.
    7. A second buyer takes the 16 t.
    8. Two more IC days refill to 5,002 t, and the next day is idle.

    The last assertions: three batches, 60 t into 48 t; water at 9,548 t, pool and listing alike;
    propellant at 5,002 t.
  - `mill::pl1_a_floor_binds_an_ore_mill_too` (3 s). The Exchange's ore mill is floored at its
    seed by a lane variant: its sold ore waits unmilled, and once 10 t of concentrate is bought,
    it mills.
  - `tacsim` lib: `port::mill::tests::pl1_drawn_reads_one_batch_of_an_own_stock_ore` and
    `pl1_a_floored_mill_line_names_its_floor`. `port::def::tests::sm12a_each_mill_rule_refuses_with_its_sentence`
    is extended with the own-stock rules.
  - `lane::pl1_strip_refine_rebuilds_the_published_v34`: v34's digest `9e78ca9a…`, idempotent.
- **Unchanged and green:**
  - the `domain_economy`, `domain_world` and `cost_lane` binaries whole (874 tests); the only
    failures were the three pins above, since fixed;
  - every SM.12a mill story, the month of milling included;
  - mush-server `split_boundary`'s `relay_registry`, `boot_parity` and `content_studio` (91).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets`, the M7
  memory-size pass on `tacsim`, rustdoc `-D warnings --document-private-items`, and
  `mush-contract-tests`' feature census, panic budget and restriction lints (24).
- **Mutations** (each fails the stories or the unit tests):
  1. milling without the draw;
  2. no floor on the draw;
  3. no day's room on the draw;
  4. `drawn` always 0;
  5. `own_stock` always false;
  6. the drift check without the netting;
  7. the Listings line without the netting;
  8. an own-stock row without a floor accepted;
  9. the floor ignored on an ore mill.

**Divergences:**
- **No works slot is booked.** Q-48-41 asks for "inputs bought, slot booked, output collected and
  listed". The refine is a house mill row, as Kasei's refine is (`port/plant.rs`: the refinery
  holds "one slot never booked"). Its daily rate is its capacity. The cycle the story shows is:
  ice bought into Occator's bid, milled into water, water drawn off its ask, refined, the
  propellant listed and bought. Works recipes with public slots are PL.4's (Q-48-24).
- **The floor is new.** The brief asked only for a sized rate. A fixed rate either floods the ask
  with no traffic or drains it under heavy traffic, and an active refine would have changed the
  Ceres clerk's acts in every story from boot. The floor is the alloy buy's shape (a works buys
  below its floor) and keeps the refine idle until propellant is bought.
- **Files outside the brief's list**, all mill integration: `port/def.rs` (`MillDef.floor_t`, the
  own-stock rule), `port/readiness.rs` (the Listings line's netting) and `port/stock.rs`'s drift
  check. The starting session approved them before they were touched, with the rule general,
  not Occator's.
- **The refinery's room is a stand-in.** Ceres has no plant room; the Pressure Lobby is the free
  room nearest the berths. The freight deck's tank gallery (Ceres atlas, cut P-F) is the natural
  home, and moving it is a lane edit.

**Not built:**
- **The prices invert the refine.** Five tonnes of water at Occator's ask (500 cr) make four of
  propellant at its ask (160 cr). No money moves, since it is Occator's own stock, but a player's
  refine at PL.4 would lose money. EX.8's price rule, or a retune, is the remedy.
- **No floor on the water.** Under heavy burns the refine draws up to 480 t of water a real day,
  past the ice mill's 384 t. EX.9 balances consumption against supply.
- **The census.** The refine is a `tacsim.port_mill` ledger (`volatiles` into `propellant`).
  EX.0's matter census counts a mill only by `counted(&ledger.ore)`. Once it counts volatiles
  and propellant, `milled` takes the refine's 4 t lost per batch (noted for EX.0 in the session
  notes).
- **Other excess readers.** `supply.rs` and `pools.rs` compute a seller's excess without the
  drawn batch. No works buys Occator's water today. If one ever does, its purchase could take a
  drawn batch, and the refine would draw again.
- **Untested waits.** No story covers the draw's wait while a Draw quote on the water pool is
  unsettled, or a refused draw. The first copies the restock's wait, which
  `sm12a_an_undrawn_draw_holds_the_restock_and_never_loses_it` covers; neither was mutated.

## PL.2

Not built.

## PL.3

Not built.

## PL.4

Not built.

## PL.5

Not built.

## PL.6

Not built.

## PL.7

Not built.

## PL.8

Not built.

## PL.9

Not built.

## EX.9

Not built.

## EX.10

Not built.
