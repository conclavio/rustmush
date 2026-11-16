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

Not built.

## PL.8

Not built.

## PL.9

Not built.

## EX.9

Not built.

## EX.10

Not built.
