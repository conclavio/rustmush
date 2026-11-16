# 48. Economy and production — cut notes

One section a unit of [doc 48](48-economy-and-production.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

## EX.0

Row: §7.1 EX.0; Q-48-17, Q-48-31, Q-48-36's census note, doc 48b W-48-6.

**Survey.**

- **Money.** Every instrument is zero-sum, and only System accounts may go negative (§3.4), so
  a System account's fall over a day is that day's mint, and its rise is the fees and levies it
  took in. No System account is marked as a faucet or a sink: every one is opened with
  `CreditPolicy::unbounded()`, so the census tells them apart by the sign of the day's change.
- **The journal cannot be bucketed by IC day after the fact.** An `EconomicTransaction` carries
  only `posted_at`, in wall-clock seconds. The world timeline is re-anchored after every outage,
  and only the last re-anchor of each generation is kept, so a past post's IC instant cannot be
  rebuilt. The census reads balances instead: two readings bound a day.
- **Organisations carry no kind.** Lane institutions are the port maps' `orgs` rows, and Houses
  are the launch deployment's three `NamedHouses`. Both are tacsim's knowledge, so the money
  census takes them through a hook.
- **The matter census and its sinks.** Today it counts the chain's goods only
  (`census.rs:29-31`). Five acts destroy matter outside the chain. Only the first leaves a lasting
  amount:
  - the departure burn: each `tacsim.ticket` keeps `burn_t` and `crew_days`, written in the
    debit's own bundle and never deleted;
  - the in-flight burn (`+helm/burn`, the autopilot's passage): `SpaceBurnReceipts` kept the
    landed sequences only;
  - the bay's rearm: its draw debits the hold, and the order keeps no amount once it closes;
  - the field tender's rearm: likewise;
  - the wreck's vent (`tow.rs`) and the customs ruling's seizure (`space/customs.rs`).
- **The goods.** Munitions goods are the rows with a `munitions` entry. Propellant and provisions
  carry a `consumable` mark. Occator's ice mill makes volatiles from `ice_ore`, which is also
  uncounted.

**Built.**

- **`@econ/census [<days>]`** (`mush-economy/src/census.rs`; staff, SIM_ADMIN or Royalty):
  - The sweep's census step (`census::sample`, from `sweep.rs`) reads every currency account
    once when the IC day turns. A day that has not turned costs one comparison.
  - A row is the difference between two readings: each System account that moved (a source when
    it fell, a sink when it rose), and the holdings by category at the close (`lane`, `house`,
    `org`, `player`, `other`).
  - `drift` is the net mint less the holdings' change. It is 0 unless the scan stopped at its
    bound.
  - The verb prints the last 1 to 28 rows (default 7), the day still open, and the window's
    totals with each faucet's net mint.
  - Bounds: 262,144 accounts a reading, 168 rows an instrument (a real week at 24:1), and four
    currency instruments. Lines are fitted to 120 columns.
  - The book is economy module state and one `census` document, written once a row and read at
    boot (`module.rs`; `boot.rs` knows the key).
  - Categories: `MoneyCensusHook` names an organisation's category. tacsim installs
    `port::census::money_census_hook()` (lane institutions from the port maps, the three
    Houses). Any other organisation is `org`, a player object is `player`, and anything else is
    `other`.
- **The matter census** (`tacsim/src/port/census.rs`, `@industry/census`):
  - It counts `volatiles`, `ice_ore`, propellant and provisions (by their `consumable` mark), and
    every munitions good (by its row).
  - New sink terms: `burned`, `loaded`, `seized` and `consumed`. `consumed` reads 0 until EX.9's
    consumption arm writes it.
  - The balance subtracts all four.
  - A new source term, `rerolled` (PL.10's, by the merge-order rule): the tonnes each
    `tacsim.pockets` re-roll ledger credited back into its deposit's pool (`rerolled_t`), weighed
    by the deposit's good. It covers surface sites and space clusters alike, since both re-roll
    through `surface::opening::reroll_step`.
  - A new term is one field, one line in `balance` and one in `line`.
  - A mill whose product is uncounted no longer subtracts that product.
- **The writers the census reads** (approved outside the row):
  - `BayHull.loaded_kg` (`bay_work/record.rs`): the munitions kilograms of each draw, added in
    `bay_work::settle` as the draw lands (`census::munitions_kg`).
  - `SpaceBurnReceipts.{burned_t, crew_days, seized_t}` (`port/burn.rs`): running totals, added
    in the same prepare as the debit.
  - `SiteMatter.loaded` (`surface/site_record.rs`, `surface/tow.rs`): a tender's rearm, units by
    good, in the tend's own bundle.
  - One line in tacsim's `register()` that installs the hook.
- Help: the two census verbs in `help admin`, the `@econ` switch line, and `@industry/census`'s
  own help.

**Tests.**

- **The money census** (`mush-economy/src/census/tests.rs`, 15 tests):
  - the ledger's arithmetic on hand-built readings: open, close, a missed day as one row of two
    days, the clock moving back, the row bound, drift on a partial scan, the System accounts past
    the bound;
  - the categories, the 120-column view (a side capped at eight lines, flags kept on a carried
    line), the window's per-faucet totals, and the document's round trip;
  - `@econ/census` over a `TestWorld` through the real sweep: a faucet capitalises an org the
    test's hook names `lane` and pays two purses, a player pays a levy. Each day's mint, sink,
    net, holdings and source and sink lines are checked, and so are the open day, the window,
    `@econ/census 1`, a bad count and a mortal;
  - a reboot from the economy's own documents over a book wiped in RAM, and the next day's row
    opening from it.
- **The matter census** (`tacsim/src/port/census.rs`, 3 tests): what is counted, the four sinks and
  `rerolled` in the balance and the line, and the new fields encoding as before until they count.
- **Stories** (conclav.dev, `CARGO_INCREMENTAL=0`, on main `c85a8744f`; 22 of 22 green):
  - `domain_field::ex0_matter_sinks::ex0_the_census_holds_across_a_burn_and_a_rearm`, new: GC.8b's
    refit run. `burned` moves by the ticket's tonnes and crew-days, `loaded` by the rearm's
    munitions kilograms, and the balance holds. `@econ/census` files the lane's capital as `lane`.
  - `domain_world::voyage::sfr4_free_flight…`: the balance holds across the departure, timed and
    autopilot burns, and `burned` grows past the ticket.
  - `domain_field::field_rs1::rs1_*`: the tender's rearm writes its units to the site.
  - `domain_economy::deposits::pl10_a_worked_out_site_deposit_rerolls…`: across the re-roll and
    its restart, `rerolled` grows by the deposit's reserve, and the balance holds.
  - These re-ran green: `bay_gc8::*`, `in11_matter::*`, `house_resale_rs6c2::*` and
    `house_sweep_rs6c::*`.
- **Suites**: the whole of `mush-economy` (921 tests, on main `c85a8744f`), and tacsim's
  `port::census`, `port::burn`, `port::bay_work`, `surface::site_record`, `surface::tow`,
  `surface::tender` and `surface::cursor` unit tests (30).
- **Guards**: every `mush-contract-tests` `ci_policy` gate (80) and the engine contracts'
  `help_commands` and `state_slot_owners` (10); clippy and the M7 memory-size pass on both crates;
  rustdoc.
- **Mutations** (conclav.dev; each one is a single substitution, run against the tests named to
  kill it), on main `c85a8744f` with this unit applied. 34 mutants in all; 28 were killed. **E1**
  (an account that did not move kept as a flow) survived the first round, until the first ledger
  test gained an unmoved account. The six that survive:
  - **E5** (`partial` never set): no test reaches 262,144 accounts.
  - **E11** (the boot's `census` key arm removed): the key then falls to the "unknown key" warning
    and is still read by the module itself. This is equivalent: only the log line changes.
  - **E15** (every System account named, the bound never applied): no test opens 257 System
    accounts.
  - **T9** (the census's read of a tender's loads): no story reads the census across a tender's
    rearm. RS.1's story checks the site's units, but its window also opens a rig's bale, which the
    census does not hold (below).
  - **T15** (`munitions_kg` weighs every row of a draw, not only munitions): EX.0's rearm draws
    munitions only. GC.8's mixed order reads no census.
  - **T16** (a mill's product counted whatever it is): no lane mill makes an uncounted good.
  - The rest were killed by the tests named above: `@econ/census`'s ledger, categories, view,
    document, boot and refusals (E1–E4, E6–E10, E12–E14, E16); `counts` and the balance (T1–T5,
    R2); the departure tickets, the bay's `loaded_kg`, the hook line and the lane classes (T6, T8,
    T10, T13, T14, by EX.0's story); the receipt totals and their read (T7, T11, by SF.R4's); the
    tender's units (T12, by RS.1's); and `rerolled`'s read (R1, by PL.10's).

**Divergences.**

- **The four writers outside the row's files**, approved by the lead on 2026-11-16: `bay_work/`
  (the record and the landing), `port/burn.rs`, `surface/site_record.rs` with `surface/tow.rs`,
  and `tacsim/src/lib.rs` (one line).
- **The bay's `loaded_kg` is written where the draw lands** (`settle`), not beside `repaired_kg`
  in `complete`. The draw has already debited the hold when `complete` runs its checks, and a
  cancelled order gives nothing back, so counting there keeps the census whole for a cancelled
  rearm too.
- **The tender's rearm is kept as units by good, not kilograms.** The tow's prepare holds no
  goods table, so the census weighs the units. `SiteMatter` is no longer `Copy`; nothing copied
  it.
- **A fourth sink, `seized`**: the unlicensed strategic ore an in-flight burn's picket takes. That
  ore was already a counted good and left with no term.
- **`ice_ore` is counted** beside the four goods Q-48-31 names, so Occator's ice mill balances.
- **A fifth holdings category, `other`**: NPC purses and objects, beside lane institutions,
  players, Houses and other organisations.
- **`rerolled` is EX.0's**, because PL.10 (#625) merged first (the lead's rule). PL.7's
  build-line fix in this file had not landed when this PR opened. If it lands first, the merge
  keeps its line.
- **IN.11's port-half story now allows for one staff seed**
  (`in11_the_port_half_ore_to_parts_to_a_prospector_conserves_matter`). in5's `sail` helper seeds
  60 t of propellant and 40 crew-days by staff hand inside the story's window. That seed is a mint
  the census cannot see, and it was invisible while propellant went uncounted. The balance now
  moves by exactly that seed's 60,160 kg.

**Not built.**

- **The wreck's vent** (`tow.rs` `vent`) and **the customs ruling's seizure**
  (`space/customs.rs`) destroy matter and record no amount. Both are outside the approved files.
  The census cannot see a vent now that it counts propellant, and never saw a ruling's seizure.
- **The bale's alloys and parts in a cancelled bay order** are debited and not consumed. This gap
  predates EX.0.
- **A rig's bale is not a census holding.** `+tv/<n> stores open` debits a tonne each of alloys and
  fittings into `RigRecord.bale`, which the census does not count, so the balance rises by the two
  tonnes when the bale opens. The tender's repairs then subtract what they consume, as
  `repaired`. This predates EX.0.
- **Not run:** the bot census stories (mush-bot `mining_cohort::in11_*` and `rs9_story_e`,
  `#[ignore]`d dispatch runs). They read the balance across a window that now counts propellant,
  so a staff seed of a tank inside their window would show the same way as in IN.11's port half.

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

Not built.

## PL.8

Not built.

## PL.9

Not built.

## EX.9

Not built.

## EX.10

Not built.
