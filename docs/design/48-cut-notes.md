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

**Survey.**
- **Surface.** A lane site opens one `OreDeposit` per (block, family) with ore
  (`surface/opening.rs` `opening_plan`, industry's `DepositOpen` through the composed
  `SiteOpener`). Its reserve is the keyed geology's sum of the block's pockets. A block with an
  authored worked share moves that share to the spoil by `tacsim_site_work`.
- **Space.** A cluster's rock opens one deposit per spectral family at its authored `gross_t`
  (120 t on the lane). Laser and drone draws land through the same action (`space/mining.rs`).
- **Industry's audit.** Every 60 s it holds `initial_reserve − pool == Σ drawn` for every
  `external` deposit, Σ drawn being the composed `SurfaceDraws`' answer from the pocket cursor. A
  mismatch closes the whole block (`mush-industry/src/store.rs` `external_draws`).
- **Nothing in tacsim can raise a reserve.** Only industry writes `industry.deposit`. A place
  holds at most eight deposits, so re-opening under a new receipt each time would run out.
- **Claim renewal** reads the block's Σ drawn as a counter that never falls ("worked this term",
  `port/claims.rs` `drawn_now`). The matter census counts Σ `initial_reserve` as `opened` and the
  deposits' pools as `in_ground` (`port/census.rs`).
- **`surface/mine.rs`** deletes a pocket cursor once its rows are empty.
- **There is no world seed** beyond the geology key. The ground is a pure HMAC of it, and the key's
  fingerprint is pinned in `tacsim:geology` (doc 20 §3.2).

**Built.** A worked-out deposit re-rolls 28 IC days after its clerk records it worked out.
"Worked out" means the pool is empty. The deposit is a site's (block, family) or a cluster's
(rock, family). Two clerk acts on `tacsim_site_work` do it, each one bundle:
- **The mark** (`{deposit receipt}/out/{n}`) stamps the IC instant on the cursor.
- **The re-roll** (`{deposit receipt}/reroll/{n}`) runs once the delay has passed. It credits the
  pool with the deposit's opened reserve and clears the rows; a locked row stays, with nothing
  drawn, so a rig mid-run keeps its pocket.

Both acts are durable, so a restart between them still re-rolls on time. For a rock, the space
draw that empties it records the mark in its own bundle, at the instant its row was offered
(`port/site.rs`'s space branch, `stamped_worked_out`), so a cluster clerk files a mark only for a
rock emptied some other way. A site's draws are the surface providers' (`surface/act/`, PL.0's),
so a site's clerk files the mark.
- **The cursor's re-roll ledger** (`surface/cursor.rs` `RerollLedger`): `rerolls` (the counter),
  `carried_t` (the cleared rows' tonnes), `rerolled_t` (the tonnes credited) and `worked_out_ic`.
  It is `Option` with skip-if-empty fields, so a cursor that never re-rolled encodes exactly as
  before. `drawn_total()` adds `carried_t` and never falls, so claim renewal needs no change.
  `audit_drawn()` subtracts `rerolled_t`, and the audit seam reads it
  (`composition/industry_seams.rs`, one line). `initial_reserve − pool == audit_drawn` holds before
  and after every re-roll.
- **The rule and the rewrites** (`surface/opening.rs`): `reroll_next` (pure), `marked_cursor`,
  `rerolled_cursor`, the receipts, the offered row (`next_reroll_row`) and the prepare
  (`prepare_reroll`). The prepare re-checks every figure against the cursor and the pool in its
  own transaction. It refuses a pool that is not empty, a number that is not the ledger's next, a
  re-roll before its due instant, or a credit that is not the cursor's audit total.
- **The clerks.** A lane site's clerk files the acts once its claim steps are idle
  (`port/site.rs` `site_step`). A cluster clerk files them when it has no draw to land
  (`step_clusters_on_tick`). Each looks over its deposits once an IC hour (150 real seconds at
  24:1), or at the earliest marked deposit's due instant if sooner. The throttle is RAM in
  `SiteOpenings`, so a restart looks at once.
- **What a re-roll yields** is the opened reserve: the keyed geology's tonnes for a surface block,
  the authored `gross_t` for a rock. Nothing new is drawn, so the geology key, its fingerprint and
  every reading, chart and survey stay true. The counter numbers the receipts and keys.
- **The config key** is `[tacsim.surface].ore_reroll_ic_days`: default 28 (provisional), 0 to
  365, and `0` keeps ore finite (Q-48-61's alternative). It is in `mush-config`, the host's
  mapping (`mush-server/src/config_ext.rs`), `tacsim::config::SurfaceConfig` (validated) and
  `mushd.toml.example`.

**Tests.**
- **The two done-when stories**, each failing under the mutation that turns re-rolls off
  (`reroll_period_ic_s` answering 0):
  - `domain_economy/deposits.rs`
    `pl10_a_worked_out_site_deposit_rerolls_after_28_ic_days_across_a_restart`. An Ahuna deposit
    is worked out by hand (its pool moved to the spoil, its cursor drawn to every pocket's keyed
    tonnes), and industry's sweep passes it. The clerk marks it; the world reboots; two IC days
    short of the delay nothing has re-rolled; past it one re-roll lands, pool back to the
    reserve, rows cleared, ledger 1 / carried / credited, total unchanged. The sweep still passes,
    a second reboot reads it back, and an unworked deposit is never marked.
  - `domain_world/voyage.rs`
    `pl10_a_worked_out_cluster_rock_rerolls_after_28_ic_days_across_a_restart`. A Dray-P flies to
    Pallas; Block C's metals are worked out by hand; the cluster clerk marks the rock; the same
    reboot and two waits follow; the rock holds its authored 120 t again, and the sweep passes.
  - Both jump the clock to the due instant (`jump_to_ic`, `to_ic`), never striding idle fires.
- **Unit tests:**
  - `surface/opening.rs`: the rule, mark then due, with off, ore-in-pool, no cursor and audit
    mismatch all idle; the re-rolled cursor's rows, ledger, totals and refusals over two cycles;
    the rig's cursor edits (`mine.rs`) keeping a ledger with no rows; receipt round-trip and
    refusals.
  - `surface/cursor.rs`: a ledgerless cursor encodes byte-identically, and the ledger round-trips
    and is checked.
  - `mush-config` and `mush-server` `config_ext`: the key's default, band and refusal line.
- **`sfr23_mining_and_sensing_remainders`** (`voyage.rs`) pins that the cluster clerk files nothing
  once Block A is exhausted. It failed on the first cut, where the clerk filed a mark there. The
  pin was kept, not re-blessed: the emptying draw now stamps the mark itself, and one assertion was
  added that Block A carries it.
- **Existing suites, run unchanged on conclav.dev at the patched tree, all green:**
  - tacsim lib `surface::`, `space::`, `config::` and `port::site` (341);
  - `domain_economy` `deposits::`, `mining::`, `unloading::`, `claims::`, `drones::`, `quarry::`,
    `plans::`, `sites::`, `mill::` and `in11_matter::` (221);
  - `domain_world` `voyage::` and `sky::` (82);
  - `domain_field` `driving::` and `space_ap9::` (26);
  - the SM.8 story (`mush-server` `commerce_composition` `sm_story`, 2).

**Divergences.**
- **Files outside the row**, each needed because only the cursor can carry the re-roll's
  accounting:
  - `surface/cursor.rs` (the ledger), `composition/industry_seams.rs` (the audit reads
    `audit_drawn`) and `port/site.rs` (the clerk's step, the offer's third source, the prepare's
    branch, and a space draw keeping the ledger and stamping the mark). The starting session
    approved these three.
  - `surface/mine.rs`: `cursor_after` and `cursor_released` keep a cursor with a ledger and no
    rows instead of deleting it, and its two struct literals gained the new field. Otherwise a
    lock released with nothing drawn would delete the ledger.
  - The config key's files, as listed above.
  - Tests in `tacsim/tests/domain_economy/deposits.rs` and `domain_world/voyage.rs`, which reuse
    those files' rigs: one new story in each, and one added assertion in `sfr23`. Also one struct
    literal in `domain_field/driving.rs`.
- **The unit is the deposit, not the whole site or cluster.** A site holds up to 64 deposits, and
  one held or unreachable block would keep a whole site from ever re-rolling.
- **The 28 days run from the mark.** For a site, the clerk's mark lands within an IC hour of the
  pool emptying, not at the last draw; for a rock, the mark is made at the emptying draw.
- **A re-roll restores the opened reserve; it is not a fresh random draw.** A surface draw would
  need a second quantiser beside the keyed geology, which doc 20's one-quantiser rule and its
  honest readings forbid. "Deterministic from the world seed and a re-roll counter" holds
  trivially: the amount depends on the key and the content, and the counter numbers the
  re-rolls.
- **No pin moved.** A cursor without a ledger encodes byte-identically, and nothing marks a
  deposit that still has ore. The one story that pinned the clerk's silence at an exhausted rock
  (`sfr23`) is met by the draw's own mark, as above.
- **The census term is not added here.** EX.0 had not merged when this PR opened, so EX.0 adds the
  `rerolled` source (Σ `cursor.rerolled_t`) in `port/census.rs`. Until then a census window that
  spans a re-roll shows its tonnes as unexplained `in_ground`.

**Not built.**
- The census's `rerolled` source term (EX.0's, per the ruling above).
- No player-facing sentence, and nothing in `docs/help/mining.md` says ore comes back.
- No staff verb that reads or forces a deposit's re-roll state.
- A re-rolled block does not re-apply its authored worked share: a re-rolled commons block (400 ‰)
  is whole.
- The clerk's offer carries one row, as every site row does. If another deposit falls due between
  the step and its drain, that submission is refused and the next fire files it.

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
   with the vendors). `def::tests::sm12b_each_yard_rule_refuses_with_its_sentence`'s mill-less
   case now drops the ice mill alone. mush-server's `commerce_composition` suite bunkers at Ceres,
   so its refine runs there:
   - `ic_slice_loop` and `ic_buyout_dormancy` land Ceres's clerk with the house's acts in
     `drain_players`, as IN.5 did for Stickney's;
   - `ic_slice_loop`'s "seeded − sold = remaining" checks count the refine's water and
     propellant off its ledger and counter (`occator_refined`, beside `kasei_refined`).

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
  - the whole `tacsim` lib (2,365);
  - the `domain_economy`, `domain_world` and `cost_lane` binaries whole (874 tests); the only
    failures were the three pins above, since fixed;
  - `domain_field`, `domain_arena` and `harness` (1,471). `field_rs1`'s two tender-tow stories
    (Ahuna, Urvara) failed once at load 55, beside another session's sweep, and passed alone on
    this tree (62 s and 64 s);
  - every SM.12a mill story, the month of milling included;
  - mush-server `split_boundary`'s `relay_registry`, `boot_parity` and `content_studio` (91),
    and `commerce_composition` whole (91), after the two fixes above;
  - mush-bot (791).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets` (on the
  final tree), the M7
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
  below its floor) and keeps the refine idle until propellant is bought. **Its 5,000 t, the seed,
  is provisional**, like the rate.
- **Three edits beyond the brief's files**, approved by the starting session before they were
  made, each general rather than Occator's:
  1. **`port/def.rs`:** `MillDef.floor_t` (optional, skipped when absent, so the existing rows
     encode as before) and `check_vendor_mills` admitting an ore that is one of the vendor's own
     asks, for any vendor. Only `MillDef` and its check changed; EX.3's acceptor rows come later.
  2. **`port/readiness.rs`:** the gated Listings line nets out the batch drawn and not yet milled.
  3. **The lane's Ceres `refinery` block** (operator `occator`, room `c02_pressure`). Ceres had no
     plant room, so the Pressure Lobby, the free room nearest the berths, is a stand-in. The
     freight deck's tank gallery (Ceres atlas, cut P-F) is the natural home, and moving it is a
     lane edit.

  `port/stock.rs`'s drift check nets the same batch; that file is the stock clerk's mill arm,
  which the brief already allowed. PL.2's farm, fab and feedstock mill can use the floor and the
  own-stock ore on any vendor as they stand.

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

**Survey** (at `ba203074e`, PL.1's tip with main `c85a8744f` merged):
- **The seeds with no producer.**
  - Provisions: five rows of 2,600 crew-days.
  - Instrument boards: 2,080 t at Crisium Instruments, Luna.
  - Carbon feedstock: 600 t at Titan Concession, beside 1,800 t of water.
  - Relay blanks: 8, minted once.
- **The buyers already there.**
  - The Lunar Habitat Board bids for water (4,100 t at 195) and feedstock (300 t at 380), with
    no stock and no asks.
  - Crisium stocks and asks boards and bids for nothing.
  - Titan Concession stocks and asks feedstock, water and propellant.
  - Neither Luna nor Titan authors a facility block, and their atlases have no plant room.
- **Why a PL.1 mill row could not make these goods:**
  1. A row takes one input. The farm takes water and feedstock, the fab strategic ore and metals.
  2. Its ratio is tonnes to tonnes with `product <= ore`. A crew-day is 4 kg, so a tonne of
     inputs makes about 250 of them.
  3. A cross-family conversion is a `refine`, and `check_refinery` allows one refiner a port,
     the refinery's operator. Luna would have two: the farm and the fab.
- **The works.**
  - A house order exists: the beneficiary is the facility's owner, the bill comes from its
    stores, and the arm collects into the stores (`works.rs` `house_collect`).
  - Nothing books one for goods: `pools.rs` books yard orders for the Trust only.
  - A works' stores are a vendor's storefront only where the works room is that storefront
    (Freeport Works).
  - Recipes are capability-gated: all 14 require `tacsim.works`, so a new row is offered at the
    Ceres and Stickney works.
- **The relay foundry** (`relay_foundry.rs`):
  - Blanks are no good: they are a pool of 8 credited once by `+relaypair/stock` (:358-373).
  - Boot fails when a blank pool stands with no stock receipt (:508-515).
  - `MAX_PAIRS` (8) bounds the receipt book, but the real ceiling is the relay registry: 32
    nodes and 64 pairs (`mush-sim/src/relay.rs:20-21`). Each private pair takes two nodes and
    the public star five, so at most about 13 pairs can ever be installed.

**Built:**
1. **`kind: process`** (`port/def.rs`, `MillKind::Process`), a house process:
   - one or two goods into a good of any family and any unit;
   - bound by mass, not by family: `check_ports` refuses a product that outweighs its inputs,
     by the goods' `si.mass_kg`;
   - its ratio counts in the goods' own units: ore 1..=100, product 1..=10,000
     (`MAX_PROCESS_PRODUCT`);
   - it needs no refinery: `check_refinery` now ties only `refine` rows.
2. **`MillDef.with`** (optional, skipped when absent): a process's second input, `per_ratio`
   units a ratio.
   - It must be a good the vendor bids or asks, neither the ore nor the product.
   - Only a process names one.
   - An input the vendor asks makes the row own-stock, so it names a floor (PL.1's rule, now over
     either input). `MillDef::inputs` lists a batch's goods and their units.
3. **The mill takes two inputs** (`port/mill.rs`):
   - **The batch:** it debits the second input in the same bundle as the ore. The ledger keeps
     the input and its total (`MillLedger.with`, `with_t`, both skipped when absent), checked to
     conserve under the ratio. A changed second input is refused like a changed product (D9).
   - **The draw:** each input is read on its own. A bought input must hold its batch's worth.
     An asked input is drawn off its own ask, one act a fire, and the first short one is drawn
     next. A draw starts under the floor; one under way finishes whatever the floor reads.
   - **The drift check and the Listings line** net each own-stock input's drawn batch (`drawn`
     sums the allowances).
   - **The Mill line** names the second input. It gives a process's product in its own unit
     ("1000 provisions") and drops the unit after the floor.
4. **Lane v36** (provisional figures):
   - **The Lunar Habitat Board's farm.** A process of water into provisions with feedstock,
     5 : 1,000 with 1 t of feedstock a ratio, 5 t of water a batch and an IC day, floor 1,000.
     Its inputs are its existing bids. It gains a provisions ask at 10 and a stock row of 1,000.
   - **Crisium Instruments' fab.** A process of strategic ore into boards with metals, 1 : 2 with
     2 t of metals a ratio, 2 t of ore a batch and an IC day, floor 2,080 (the seed). It gains
     two bids: strategic ore at 650 (intake 12, 2 a day) and metals at 260 (intake 40, 8 a day).
   - **Titan Concession's carbon mill.** A mill of carbon ore into feedstock, 5 : 4, 20 t a
     batch and an IC day, floor 600 (the seed), with a carbon-ore bid at 55 (intake 120, 20 a
     day). This closes the second review's finding that the farm would eat a finite seed.
   - **Titan Concession's farm.** The Luna farm's figures, drawn off its own water and
     feedstock asks, with a provisions ask at 20 and a stock row of 1,000.
5. **The carbon press** (recipe pack v3, `INDUSTRY_RECIPES_VERSION` 3): `carbon_feedstock`, 5 t
   of carbon ore into 4 t of feedstock, 4 work units, pooled. It is a public slot at both works.
6. **The lane bump.** Lane v35 → v36, relay v26 → v27, and
   `support::lane_chain::strip_producers` as the strip chain's new head (`strip_refine` calls
   it). Pins moved:
   - `LANE_VERSION`, `lane.rs`' `LANE_DIGEST` and `ic2_the_lane_pack_is_pinned`;
   - `cost_lane.rs`' `SHIPPED_LANE_DIGEST` and its version asserts;
   - the version asserts in `mill.rs`, `drones.rs`, `sites.rs` and `stations.rs`;
   - `relay_registry.rs`' relay digest and kept-world versions, `boot_parity.rs`' relay
     version;
   - `mill::sm12a_strip_mill_rebuilds_the_published_v13` (the shipped mill list),
     `mill::sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter` (the mill's second
     `.debit(`), and `ports::ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`
     (77 commerce lines).

**Rates** (provisional; at 24:1 an IC day is a real hour):
- Each farm makes at most 1,000 crew-days an IC day, 24,000 a real day. A passage burns its
  occupants times its days, about 20 to 40 crew-days. So the floor, not the rate, binds: a farm
  only replaces what is bought.
  - Titan's draws are at most 120 t of water and 24 t of feedstock a real day.
  - The Habitat Board's bids are one-time intakes (no refresh), so the Luna farm stops buying
    once they fill.
- The fab makes at most 4 t of boards an IC day, 96 a real day, and its bids drip 2 t of ore
  and 8 t of metals an IC day.
- Titan's carbon mill makes at most 16 t of feedstock an IC day (384 a real day), fed by its
  bid's 20 t a day.
- Value at the asks: a farm batch's inputs are worth 1,355 cr at Luna's bids and
  10,000 cr as provisions. The fab's are worth 2,340 cr and its boards 3,600 cr.

**Tests** (run on conclav.dev):
- **New:**
  - `domain_economy` `pl2_producers::pl2_carbon_pressed_at_ceres_feeds_the_luna_farm_and_the_fab_one_full_cycle`
    (13 s) runs the full cycle at Luna:
    1. At Ceres the captain quotes, orders, delivers and collects the carbon press: 5 t of ore
       becomes 4 t of feedstock, through `+works`.
    2. At Luna, water and strategic ore are sold into the bids, and the farm and the fab stay
       idle at their floors.
    3. A crew-day and a tonne are bought under the floors. Still no batch is queued without the
       second inputs.
    4. Feedstock and metals are sold. One farm batch (5 t and 1 t into 1,000 crew-days) and one
       fab batch (2 t and 4 t into 4 t) follow, the asks are restocked to 1,999 and 2,083, and
       nothing pages drift.
    5. A crew-day and a tonne are bought off the restocked asks.
  - `mill::pl2_titan_farm_draws_its_own_stock_and_the_carbon_mill_refills_it` (3 s) runs
    Titan on its lane variant:
    1. The farm draws 5 t of water and 1 t of feedstock off its own asks, batches once and
       lists 1,990 crew-days. The Mill line names the second input and the floor.
    2. 20 t of carbon ore is milled into 16 t of feedstock and listed at 615.
    3. Nothing moves the next IC day.
  - `lane::pl2_strip_producers_rebuilds_the_published_v35`: v35's digest `4db678a2…`,
    idempotent.
  - `tacsim` lib:
    - `port::def::tests::pl2_each_process_rule_refuses_with_its_sentence`;
    - `pl2_a_process_never_outweighs_its_inputs` (1,500 crew-days of 6 t accepted, 1,501
      refused, the shipped lane accepted with no Luna refinery);
    - `port::mill::tests::pl2_drawn_reads_each_input_of_an_own_stock_process`;
    - `pl2_a_process_ledger_conserves_its_second_input`;
    - `pl2_a_process_line_names_its_second_input`.
- **Pins moved, besides the lane bump's:**
  - `plumbing::sm5_only_ceres_bids_ore` (Crisium's and Titan's ore bids);
  - `commerce::a8_the_moon_ports_stock_list_and_bid` (12 moon stock rows; the Habitat Board
    no longer bid-only);
  - the web client's `web/src/lib/fixtures/chart/sky_chart.json`, re-blessed with
    `TACSIM_BLESS=1`: the chart lists Crisium's two new bids.
- **Green on the final tree:**
  - the whole `tacsim` lib (2,389);
  - `domain_economy`, `domain_world` and `cost_lane` (882; the five failures were the pins
    above, since fixed);
  - `domain_field`, `domain_arena` and `harness` (1,471);
  - mush-server `split_boundary` and `commerce_composition` (318);
  - mush-bot (791).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets`, the M7
  memory-size pass, rustdoc `-D warnings --document-private-items`, and the contract guards'
  feature census, panic budget and restriction lints (24).
- **Mutations:** seven killed and one survived.
  - Killed:
    1. the second input not debited;
    2. a bought second input not awaited;
    3. `drawn` counting only the ore;
    4. the draw reading only the ore;
    5. no mass rule;
    6. a second input on any kind;
    7. the refinery tying a process.
  - Survived: the ledger's second-input total not accumulating (`with_t`). It fails closed: the
    ledger's own check refuses the second batch, and every story runs one batch a process.

**Divergences:**
- **Relay blanks are not built here.** The starting session approved the split on 2026-11-16, and
  PL.2b waits for the owner's call on how many relay pairs the world gets (Not built, below).
- **No works slot is booked, and the farms and the fab have no facility.**
  - They are house processes on the stock clerk's turn, as PL.1's refine is.
  - Luna and Titan have no plant rooms in their atlases.
  - A facility for each is PL.9's (facilities as titled assets) or later.
- **New starting stock:** 1,000 crew-days of provisions each at the Habitat Board and Titan
  Concession. A farm's ask needs a stock row, and a seed at the floor keeps the farm idle until
  provisions are bought, so no existing story moves from boot.
- **The Titan story runs on a lane variant** with Titan's provisions seed at 990, ten under the
  floor, and the carbon ore delivered in kind. No story flies to Titan: the leg fits the tank
  only throttled (`moon_ports.rs`' `a8_titan_fits_the_tank_only_throttled`). The Luna story
  flies, sells and buys through the player's commands.
- **The Mill line's wording:** the unit after the floor is dropped for every mill ("while
  propellant is under 5000"), since a farm's floor is in crew-days. PL.1's story and unit test
  moved with it.

**Not built:**
- **Relay blanks: PL.2b, waiting for the owner's call.** The ceiling is the relay registry, not
  `MAX_PAIRS` (8, `relay_foundry.rs:26`). The registry allows 32 nodes and 64 pairs
  (`mush-sim/src/relay.rs:20-21`); each private pair takes two nodes and the public star five,
  so about 13 pairs can ever be installed. How many pairs the world gets is a design call. PL.2b
  fixes two findings once that is ruled:
  - **Boot fails on a blank pool with no stock receipt** (`relay_foundry.rs:508-515`). A
    produced blank delivered into the foundry vault before `+relaypair/stock` would stop the
    boot.
  - **`ic_slice_loop`'s r9a and r9b stories pin the blank counts**
    (`mush-server/tests/commerce_composition/ic_slice_loop.rs`,
    `r9a_paid_manufacture_survives_restart_and_installs_once` and
    `r9b_fleet_front_real_refit_passage_and_carrier_loss`), with 8 blanks then 7.
- **The census.** A process ledger consumes two goods. Once EX.0 counts the chain's goods,
  `milled` for a process ledger is `ore_t` of the ore plus `with_t` of the second input, less
  `product_t` of the product, in kilograms. Its mill scan reads `counted(&ledger.ore)` only.
- **Refreshes and prices:** the Habitat Board's bids do not refresh, and no price follows the
  new supply. Both are EX.8's and EX.9's.
- **A process's ledger tail** (`recent`) keeps the ore and the product per batch, not the second
  input. The second input's per-batch figure follows from the ratio.

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
