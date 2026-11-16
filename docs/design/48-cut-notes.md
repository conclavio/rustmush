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

Not built.

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

Cores from the chain (Q-48-27, with doc 48b W-48-1c's correction, and Q-48-48). Built
2026-11-15 on `claude/pl7-cores-from-chain-f210f4`, stacked on PL.0 (#624), main at `72d5b7150`
merged in.

**Survey** (at `0123f796c` and `72d5b7150`):
- **The die.** `issue_core_die` is industry's origin provider (`core_production/provider.rs`). It
  checks the importer, its scope, the **depot** and the agent's stock vault live
  (`authority.rs` `check`), spends the month's quota, writes a `DieCertificate` and mints one
  `tacsim.electronics` lot into the agent's stock. Nothing is drawn from the depot: it is
  checked and never read. Every certificate retains the whole `OriginPolicy`.
- **Integration** takes exactly one input lot, the die (doc 48b W-48-1c). Nothing in it changes.
- **The prices.** `CoreClass::{finished,integration,die}_minor` were `const fn`s in
  `mush-economy` (`subject.rs:28-40`). The funding hold, the completion's two lines and the
  money check all read them (`plans.rs:51`, `:247-253`; `money.rs:417-420`). The hardware
  catalogue carried `core_minor` too, but `validate` required it to equal the constants and the
  line tank's 3/8/25 % (`hardware/content.rs:171-182`). So a price could not move without a code
  change.
- **The stock cores.** Eight yard designs carry one: the escort, five line tanks and two bipeds
  (Class II line core), the scout (Class I) and the heavy (Class III). The escort and the line
  tanks are not the only ones, as W-48-1c read. `DesignBill.core` was read and billed nowhere.
  `commission` creates the unit from its design, so the core comes with it: governed, untitled,
  no die.
- **The commission's consumption** (`works.rs` `consume`) consumes every lot in the escrow, the
  kit first. A part line billed like a part is consumed there with no new code.
- **A part needs a row.** A made part gets its provenance from `stock.rs` `part_row_of`, the
  catalogue's part index read backwards. The index holds no core, on purpose: a core in it
  would let a bay fit, a strip take, or a scrap yield one.

**Built:**
1. **A die consumes its chain goods at issue** (`mush-industry`).
   - `OriginPolicy.chain: [Vec<ChainLine>; 3]`, Class I, II, III.
   - Each class takes at most four lines, in strictly ascending commodity order, 1 to 1,000
     units each, never `tacsim.electronics`.
   - The field is written only when non-empty, so a policy without one keeps its bytes and
     digest (the industry `.snap` files did not move).
   - The issue's prepare debits each line from the depot (`InventoryFamilies::debit`), in the
     bundle that writes the certificate, the quota and the lot.
   - Short, the whole issue is refused and writes nothing: no certificate, no lot, no quota
     spent. Integration still takes one die lot; the kernel is not touched.
   - `DieCertificate::consumed()` and `::depot()` read the bill back from the retained policy.
2. **The shipped die chain** is content: `governed_hardware.json` `die_chain` (provisional):

   | Class | instrument boards | strategic ore |
   |---|---|---|
   | I | 1 t | 1 t |
   | II | 2 t | 1 t |
   | III | 4 t | 2 t |

   - The pack is re-signed (`404c9588…`).
   - The launch composition freezes it into the origin policy
     (`hardware/composition.rs`).
   - At 64/16/4 dies a month, a full month takes 112 t of boards and 88 t of ore.
3. **`CoreClass` prices are content** (`mush-economy` `CorePrices`).
   - `CorePaymentPolicy.prices` holds `{finished_minor: [i64; 3], integration_permille}`. It is
     bounded: each price 1 to 10¹⁰ minor, the share 1 to 999 ‰, both halves at least one minor
     unit.
   - The funding hold, the completion's integrator and supplier lines, and the money check read
     it.
   - It is written only when it differs from `CorePrices::V1`, so a policy at the shipped
     prices keeps its bytes and digest.
   - The hardware catalogue's `core_minor` and `integration_permille` are the source. Their
     validation is now the policy's bounds, not the v1 table.
   - The launch freezes them into the payment policy. The counter quotes them, and the mass
     budget's core reference reads them (`PublishedHardware::shipped_once`).
   - `CoreClass::finished_minor()` and its siblings stay, as the v1 table's names, for tests.
4. **The counter refuses an order the depot cannot cut a die for** (`hardware/counter.rs`).
   - `+core/buy` refuses with `CORE_NO_STOCK`, which names the bill, after the price refusals.
   - It does not ask when an integration of the class is already offered to the player: that
     order needs no new die, and a restart's request picks it up.
5. **A stock-cored design bills one core module** (`port/bill.rs`).
   - `DesignBill.core_module`: the core `tacsim.core.<name>` stocks as the part good
     `tacsim.part.core.<name>`. It is one of `part_lines()`, so the order bills it, the yard
     takes it as a lot, and the commission consumes it with the kit and the parts.
   - A design whose stock core has no module good in the goods table now has no bill: a stock
     core is never free.
   - The hull's core stays the governed core its design carries, outside the quota: no die is
     drawn and no title is born (the owner's call).
6. **The three module goods** (`goods.json`, v8): `tacsim.part.core.doctrine`, `.line` and
   `.command`.
   - Each is a part of its core's mass (150, 250 and 500 kg, crated at two cubic metres a tonne)
     and priced at its core (3,000cr, 8,000cr and 25,000cr).
   - Three works recipes make them (`recipes-v2.json`, re-signed): `module_doctrine`,
     `module_line` and `module_command` take the die's own chain bill (1+1, 2+1 and 4+2 t of
     boards and ore) in 24, 48 and 96 work units, one module a batch, at either works.
   - A made or stocked module's provenance names its core's definition (`stock.rs`
     `part_row_of`, through `bill::core_module_row`). The part index still holds no core, so no
     bay fits a module and no strip or scrap yields one.
7. **The census's build line** (`port/census.rs`, one line, outside this unit's files): the
   module is among the parts, so `inputs` no longer adds `core_kg` a second time.

**Tests** (run on conclav.dev):
- **New:**
  - `mush-industry` `core_production::tests::pl7_a_die_consumes_its_chain_goods_at_issue`
  - `mush-industry` `core_production::tests::pl7_the_chain_bill_is_bounded_and_written_by_presence`
  - `mush-industry` `core_production::integration_tests::pl7_an_integration_charges_its_policy_s_prices`
  - `tacsim` `hardware::tests::pl7_the_core_prices_and_the_die_chain_are_content`
  - `tacsim` `hardware::counter::tests::pl7_the_stock_refusal_holds_the_floor`
  - `tacsim` `port::bill::tests::pl7_a_stock_cored_design_bills_its_core_module`
  - `tacsim` `domain_field` `pl7_cores::pl7_a_core_takes_its_die_s_chain_goods_from_the_depot`
    (the combined M8 world, about 15 s). The depot's ore taken out, `+core/buy II` and
    `+core/buy II go` are both refused and name the bill. Nothing is queued or paid, and the stock
    agent's own `issue_core_die` is refused too. A tonne back, the order goes in and the die takes
    two boards and the tonne. After a restart the depot reads the same. With no ore left for a
    second die, asking again picks the order up and cuts none.
  - `tacsim` `domain_economy` `in7_part_recipes::pl7_the_works_makes_a_traced_line_core_module`.
    `module_line` is quoted, booked at Freeport Works, delivered and run for 48 hours. The arm
    commissions one module lot with a `Manufactured` row whose definition is `tacsim.core.line`,
    and `+works/collect` puts it in the hold.
- **Extended:**
  - `domain_economy` `yard_in9`:
    - The main story's escort is quoted with `1 tacsim.part.core.line`. Its five lots come from the
      hold, and the module is in the escrow and then consumed. The provenance names five parts,
      and her core is the governed Class II her design carries, with no core title in her.
    - IN.9b's line tank and scout, the laser tank and the ground leg's escort each lay and consume
      their module. The tank's core is checked the same way.
    - The review quote lists the module, and the seizure's escrow holds one.
  - `domain_economy` `in11_matter::in11_a_restart_between_consumption_and_title_finishes_the_hull_once`:
    the escort's escrow holds the module. `balance` and every build line's
    `inputs = hull + waste` hold.
  - `hardware::flow`:
    - The rig stocks the depot (`DEPOT_STOCK`, 200 t of each chain good, one transaction a good).
    - The shipped manufacture story checks that its Class I die took a board and a tonne.
    - The launch test checks that the composed policies carry the catalogue's chain and prices.
    - The named-launch test checks that an empty depot refuses the issue and a stocked one plans it.
  - The M8 rig stocks the depot the same way (`m8_world::DEPOT_STOCK`), and
    `driving_rig::seed_part` gives a seeded module its core's provenance.
- **Re-pinned** (content moved; nothing else):
  - the hardware pack's digest;
  - the goods table (v8, its digest, and the three rows in `ic2_the_goods_table_is_pinned`);
  - the market catalogue's count (`capacity::ic2_the_catalog_rows_derive_from_the_goods_table`,
    46 to 49);
  - the studio's goods count (`mush-server` `split_boundary` `content_studio`, 46 to 49).
  - The hardware catalogue's refusal test now refuses out-of-bounds prices, not prices that
    differ from the v1 table.
- **Unchanged and green:**
  - CO.6's two stories (`co6_cored_ship`), CO.13's (`c4_hardware::co13_a_stored_core_is_sold_with_its_title`),
    CO.4's, `c4_story_4` and `m8_end_user`, with their files untouched;
  - all of `mush-economy` and `mush-industry` (1,169 tests), their `ct0` snapshots included:
    no `.snap` moved, because a policy at the v1 prices and with no chain writes the same bytes;
  - `tacsim`'s `hardware::`, `port::bill`, `port::census`, `port::stock` and `definitions::`
    lib tests (62);
  - `mush-contract-tests` (200).
- **On base**, every new test fails: the new ones do not compile (no `CorePrices`, no
  `OriginPolicy.chain`, no `DesignBill.core_module`, no `CORE_NO_STOCK`). The extended yard
  stories fail their new assertions: the quote names no module, and the module lot is left in
  the escrow. The mutations below show each change's test failing without the change.
- **Gate:**
  - clippy `-D warnings` on `mush-economy`, `mush-industry`, `tacsim` and `mush-server`, all
    targets;
  - the CI lint job's M7 memory-size pass on the first three and `tacsim`;
  - rustdoc `-D warnings --document-private-items`;
  - `mush-contract-tests` (200).
  - The whole-workspace sweep (`gate.yml`, `kind=sweep` on conclav-dev, run 43803540797, at
    `757c359f5`) ran 12,718 tests. 12,715 passed, plus the bot's 791. Three failed, all from
    the content added, and all are fixed since: the studio's goods count; PL.0's unload test,
    which counted bulk goods exactly (now relative to the shipped table, with the same 47 test
    goods and the same weigh); and this unit's works story, which stood in the works' own room
    rather than a port room. They pass on conclav.dev.
- **Mutations**, each applied alone in a throwaway clone on conclav.dev, its tests run (the loop
  is `pl7-muts.py` in the group's notes folder):

  | # | Mutation | Caught by |
  |---|---|---|
  | M1 | the issue debits nothing | `pl7_a_die_consumes_its_chain_goods_at_issue` |
  | M2 | the issue debits the agent's stock, not the depot | the same |
  | M3 | the chain's order check removed | `pl7_the_chain_bill_is_bounded_and_written_by_presence` |
  | M4 | the die-commodity check removed | the same |
  | M5 | the requester pays the v1 price | `pl7_an_integration_charges_its_policy_s_prices` |
  | M6 | the integrator's line at the v1 fee | the same |
  | M7 | the money check at the v1 die price | the same |
  | M8 | the policy's prices unvalidated | `pl7_the_policy_s_prices_are_bounded_and_written_by_absence` (economy; added after the first loop had no economy test for it) |
  | M9 | the catalogue's prices unbounded | `hardware_catalogue_refuses_a_price_display_that_differs_from_money` |
  | M10 | the catalogue's chain order unchecked | `pl7_the_core_prices_and_the_die_chain_are_content` |
  | M11 | the launch composes no die chain | three `hardware::flow` tests (the shipped manufacture's depot, the named launch's empty depot, the composed policy) |
  | M12 | the launch composes the v1 prices | `pl7_the_launch_composes_the_catalogue_s_prices_and_chain`. It **survived** the first loop, since the shipped prices are the v1 table; the test composes a re-signed catalogue at other prices through a new seam (`GovernedHardware::prepare_with`, `PublishedHardware::from_source`) |
  | M13 | the counter asks no stock | `pl7_a_core_takes_its_die_s_chain_goods_from_the_depot` |
  | M14 | the counter asks stock of an order under way | the same (the restart's request) |
  | M15 | the bill names no core module | `pl7_a_stock_cored_design_bills_its_core_module` |
  | M16 | the part lines leave the module out | the same |
  | M17 | a core with no module good billed free | the same |
  | M18 | a made core module has no row (`part_row_of`) | `pl7_the_works_makes_a_traced_line_core_module` |
  | M19 | the census counts the core twice | `in11_a_restart_between_consumption_and_title_finishes_the_hull_once` |

  None survives.

**Divergences:**
- **One core-module good a class, not one good.** W-48-1c named the escort and the line tanks.
  But the scout (Class I), the bipeds (Class II) and the heavy (Class III) also build at the yard
  with stock cores. So each core class has its own module, at its core's mass (150, 250 and
  500 kg, §4.8) and price. A single good would have priced a Class III core like a Class II, and
  broken the bill's matter balance.
- **"Its core stays Class II"** is read as "its core stays the class its design carries, outside
  the quota". The escort, the line tanks and the bipeds keep a Class II core; the scout keeps
  Class I and the heavy Class III. **For the owner:** a yard-built heavy carries a governed
  Class III core outside the quota and outside MW.23's end-user registration, for a 25,000cr
  module. Before PL.7 it carried that core for nothing. Cheap to reverse (content: drop the
  heavy's row, or give it a Class II core).
- **The chain goods leave the depot, not the agent's stock vault.** The depot is the importer's
  vault, and the origin checks it live (owner, scope, the agent's right to take from it) but
  never read it. The die lot is minted into the agent's stock, as before.
- **The die chain is a content table beside the prices**, not a recipe: integration still takes
  one die lot, and the die is still a quota issue. The core-module recipes take the same goods,
  so a stock core consumes what a die does.
- **Three files outside the unit's list:**
  - `port/census.rs`: one line (the build line's `core_kg`). EX.0's terms are in the group's notes
    folder (`pl7-census-terms-for-ex0.md`).
  - `port/stock.rs` `part_row_of`: a core module's row is its core's definition. Without it a
    made module has no provenance, and the yard refuses it.
  - `definitions/mass.rs` `core_minor`: the core reference reads the catalogue.
  - Test support also changed: `excursions.rs` (`seed_pool` and `keep` made `pub(crate)`),
    `m8_world.rs` and `driving_rig.rs`.
- **`commission.rs` is unchanged.** The commission's consumption already consumes every lot in
  the escrow, so the module needs no new step.
- **Not stacked on PL.1.** PL.1's branch touches neither `goods.json` nor `recipes-v2.json` (its
  refine is a lane mill row). The two branches share only `port/stock.rs`, on other lines.
- **`CoreClass::finished_minor()` and its siblings stay**, as the v1 table's names, so the
  stories that read a price (CO.6's, CO.13's, `m8_end_user`) are unchanged. No production path
  reads them.

**Not built:**
- **The bring-up's depot stock** (`deploy/instance/launch-bringup.py` makes the Basin die depot
  and stocks nothing). Until an operator stocks it, `+core/buy` refuses `CORE_NO_STOCK` on the
  launch. This is a `deploy/` change, left to the owner, as CO.6 left the shop's placement.
  Cheap to reverse.
- **No NPC hauls boards or ore to the depot.** Its supply is the lane's or EX.9's, as for every
  finite seed (§3.7).
- **The census's die sink** (`cored`) is EX.0's to add; the term is named in the notes folder.
- **CO.13's buy-back baseline** (`hardware/sale.rs`, the market's core catalogue entries and the
  quote) still reads the v1 table. That is EX.2b's closed `core_sale` family. At the shipped
  prices the two agree.
- **Heirloom and grey dies** (staff issues) consume no chain goods. They are not made from the
  chain.
- **No vendor stocks core modules.** A module is made at a works from boards and ore.

## PL.8

Not built.

## PL.9

Not built.

## EX.9

Not built.

## EX.10

Not built.
