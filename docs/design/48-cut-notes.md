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

**Survey.**
- **Who sold.** Only a vendor row: an org registered by staff. `list_goods` wrote the seller
  as `Organization(vendor.org)`, and every quote and accept path looked the vendor row up.
- **Who accepted.** The market accepted for a seller through `accept_contract_as`, which asks
  `party_cap(principal, PartyCap::Accept)`. That cap covers a whole principal: it comes from
  a table frozen at composition or from tacsim's content source, and nothing in it can name a
  listing. So mush-market alone could not accept for an offline player. The starting session
  approved a seam in `mush-economy` and one line in `mush-composition` (Divergences).
- **Bids.** `post_wanted` escrowed nothing. On a sale into a bid, the seller names the
  bidder's account as the consideration, and `offer_contract`'s `consideration_for` refused
  an account the issuer cannot view unless its owner held the party cap. So a seller could
  not even name a player bidder's account.
- **The hold clock.** An exact lot's hold has no clock of its own: `lot::hold_standing` reads
  the lot hold and the listing's own deadline. Even so, `list_goods` cut every lot listing to
  `hold_ttl_real_secs`, 7 IC days, which is 7 real hours at 24:1.
- **Vendor ids** are allocated from 0, so 0 cannot mark "no vendor row".

**Built.**

*The seller model* **(expensive to reverse).**
- **A personal listing** names no vendor row and carries its seller's own account:
  - `Listing.vendor` is `NO_VENDOR` (`VendorId(u64::MAX)`, which `alloc_vendor` never mints);
  - `Listing.account` is the seller's own live account in the venue's currency.
  - The wire takes exactly one of the two, a vendor row or an account (`record::check_personal`).
- **The seller** is any principal the actor may sell as: themself, or an organization they
  act for that may trade. The player verbs sell as the player.
- **No vendor row is made.** The listings are indexed by seller (`MarketStore::by_personal`)
  and capped at `MAX_LISTINGS_PER_SELLER` (32, provisional), asks and bids together. The cap
  is checked on post and again on load, as the per-vendor cap is.
- **A personal ask** is one exact lot only (Q-48-49; `Supply::Lot { lot: Some(_) }`).
  - The market's lot action holds the lot for the listing, as IN.2's vendor lot listings are.
  - Pooled personal asks are refused on the wire and at posting.
  - The catalogue entry is found from the lot's own commodity.
- **A personal bid** is a `Supply::Wanted` listing whose `Listing.escrow` names a reservation
  hold on the bidder's own account:
  - purpose `Reservation { kind: "market.bid", id: <listing> }`;
  - amount `price × quantity`, placed at posting under the bidder's own debit authority
    (`seller::post_bid`);
  - it expires at the listing's own deadline, so the economy's sweep releases it at expiry.
  - Withdrawal releases it (`withdraw_listing` → `seller::release_bid_escrow`).
  - Bids are players' only in this cut.
- **A private offer** sets `Listing.private_to` (a person) and is `Hidden`.
  - Only its seller, its counterparty and staff see it (`may_view_listing`).
  - Anyone else is refused, in the offer's own words where they can see it:
    - at the quote (`trade::quote`);
    - at the accept (`trade::accept_at`, before every other check on the quote);
    - by the standing acceptance;
    - for a bid, at `+market/sell` (`trade::bid_buyer`).

*The party-cap shape* **(expensive to reverse).**
- **mush-economy.**
  - The trait is `StandingAcceptance::stands(w, contract, party, against) -> bool`. It is wired
    once by `EconomyModule::with_standing_acceptance` and has one implementor,
    `mush_market::ListingAcceptance`, wired in `mush-composition` beside `SaleDisputeGuard`.
  - **The accept** is `accept_contract_standing(ctx, id, party, against) -> StandingAccepted`.
    - It refuses when the seam is unwired or says no, when `party` is not a party or is the
      issuer, and at §9's per-party cap.
    - It runs `accept_core` with the party as the plan's actor, so the escrow posts under the
      party's own debit authority.
  - **With `against`**, the one escrow plan does three things (`accept_core_backed`,
    `backed_ops`), so the money is reserved throughout and once:
    - releases the bid's hold;
    - places the sale's escrow;
    - places the remainder again, with the same purpose, beneficiary and deadline.
  - **The backing hold** must be the party's own: `Held`, placed by the party, a
    `Reservation`, in no engine-owned namespace, on the consideration's own account, and
    covering it (`standing_backing`).
  - **At the offer**, `consideration_for` admits an account the issuer cannot view, when its
    owner is a party, only provisionally. `offer_contract` then asks the seam about the
    finished draft, under the id it is about to take, with `against: None`. That is how a
    seller names a player bid's account.
- **mush-market's `ListingAcceptance`** answers yes only for a market quote's contract whose
  listing still stands:
  - The listing must be present (a withdrawal deletes it, which revokes the acceptance),
    personal, its seller trading, not lapsed, at the quote's revision and price, and carrying
    at least the quote's quantity. `party` must be its seller.
  - **An ask:**
    - the buyer is the issuer;
    - every consideration is the buyer's, summing to price × quantity;
    - every outcome pays the seller's own account exactly that;
    - no backing hold.
  - **A bid:**
    - the party is the quote's buyer, and a private bid's counterparty is the seller;
    - its one consideration is (the bid's account, price × quantity);
    - the backing is none on the draft at the offer, and exactly the bid's escrow at the
      acceptance.
- **The trade paths.**
  - `accept_at` accepts a personal ask by standing (`against: None`).
  - `sell_into_bid` writes a personal bid's sale quote before the offer, under the peeked
    contract id, so the offer-time question can find it. It then accepts by standing against
    `listing.escrow` and writes the remainder hold back as the bid's escrow.
  - A personal bid drawn to nothing is deleted, not kept as an exhausted marker.
  - A refunded sale does not restore a personal bid's intake: the refund returns the sale's
    escrow to the bidder's available balance, not to the bid's hold.

*Hold renewal* (Q-48-54's first point).
- An exact-lot listing, a vendor's or a personal one, is no longer cut to the reservation's
  clock.
- A personal listing stands 14 real days by default (`days=` up to 90).

*The verbs.*
- `+market/post <lot> price=<n> [to=<player>] [market=<venue>] [days=<n>]`.
- `+market/bid <ruleset>:<code> qty=<n> price=<n> into=<vault> [to=<player>] [market=<venue>] [days=<n>]`.
- `+market/withdraw <listing>`.
- `+market/mine`.
- **The venue** is `market=`, or the venue of the room the actor stands in. Its currency is
  its first vendor's (the market's one-instrument-a-venue rule).
- **Help:** the `+market` command help, and a new topic, `help exchange`
  (`docs/help/exchange.md`).
- **Sentences:** every new player sentence is a `pub const` in `mush_market::seller`. The bot
  table has a new `exchange` family in `crates/mush-bot/src/mine/minds.rs`, with in-step,
  census (over `seller.rs`), distinct and 120-column tests.

**Tests.**
- **The gates:** `crates/mush-market/tests/ex1a.rs` (18 tests), plus unit tests in `record.rs`
  (wire rules), `store.rs` (the per-seller trim on load) and `contract_live.rs`
  (`backed_ops`).
- **The done-when, test by test:**
  - A player lists an exact lot, and another buys it while the seller types nothing:
    `ex1a_a_player_lists_an_exact_lot_and_another_buys_it_while_the_seller_is_away`.
  - A bid escrows at posting and fills, converting in one plan with no gap and no double hold:
    `ex1a_a_bid_escrows_at_posting_and_fills_in_one_plan`.
  - A private offer is refused to a third party at quote and at accept, the accept case
    through a hand-edited quote across a reboot:
    `ex1a_a_private_offer_is_refused_to_a_third_party_at_quote` and `…_at_accept`.
  - A listing outlives 7 real hours (personal and vendor):
    `ex1a_a_listing_outlives_seven_real_hours`.
  - The caps: `ex1a_a_seller_holds_at_most_32_listings`,
    `ex1a_the_store_ceiling_holds_under_32_sellers_load`,
    `ex1a_the_party_cap_binds_a_player_seller` and
    `ex1a_the_issuer_cap_binds_a_player_selling_into_bids`.
- **The standing acceptance's guards:**
  - `ex1a_the_standing_acceptance_holds_a_bid_to_its_terms`: a stale revision, a withdrawn
    listing, a third party, the other party, no backing, a foreign hold, a price or
    quantity mismatch, and a consideration mismatch.
  - `…_an_ask_to_its_terms`.
  - `ex1a_a_listing_whose_terms_moved_under_its_quote_is_refused_at_accept`.
  - `ex1a_a_backing_hold_that_is_not_the_bid_is_refused`.
  - `ex1a_an_unlanded_sale_into_a_bid_refunds_and_the_bid_is_not_restored`.
- **The load:** 32 sellers (more than the 30-hull harness cohort) fill the 1,024 ceiling. On
  conclav.dev, in a debug build:
  - the fill took 1.34 s;
  - `+market/list` at the ceiling took 122 µs;
  - one sweep fire took 533 µs.

  The 1,025th listing is refused, and a withdrawal frees a place that is then taken.
- **Existing tests, on conclav.dev:**
  - every mush-market and mush-economy test passes (1,166);
  - `mush-server --test commerce_composition` passes (91, including `ic_slice_loop`, the IC
    trade run);
  - `tacsim --test domain_economy` passes (589);
  - the M7 market stories (`player_market_story`, `player_market_typed`) fail, and fail
    identically on base main `c85a8744f` without this change. In both, `+minds/resell <lic>=<player>`
    queues and is refused at its prepare ("That did not go through. Nothing changed.").
    That failure is pre-existing and is reported as a follow-up; EX.1a leaves it unchanged.
- **The gates:** `cargo fmt --check`, workspace clippy with `-D warnings`, the M7 memory-size
  clippy, rustdoc with `-D warnings` on the touched crates, `mush-contract-tests` (200: the
  census, the panic budget, the restriction lints and the help-page lint), and the bot's
  `minds::` tests (32).
- **The mutations:** 50 run, each alone, against the EX.1a tests. 37 are killed. The 13 that
  survive are defence in depth, each masked by a check that runs before it:
  - **S1:** `stands`' listing seller equals the party (the quote's own party checks).
  - **S2:** `stands`' seller still trading (`quote` and `accept_at` check first).
  - **S3:** `stands`' listing lapse (`is_current`, and `sell_into_bid`'s step 3).
  - **S9, S16:** `stands`' private-offer checks (the quote and accept checks, and a
    stranger cannot see a private bid).
  - **S10:** `stands`' one-consideration rule for a bid (the market builds one).
  - **T5:** `sell_into_bid`'s `bid_funds` pre-check (`standing_backing` refuses the same).
  - **E4:** the backing hold's placer (no test path makes a Reservation hold on a player's
    account by someone else).
  - **E6:** the party cap at the standing accept (`offer_contract` counts it first).
  - **E7:** the issuer refusal (the market never asks standing of the issuer).
  - **E8, E9:** the offer-time question, and the seam-wired flag on the consideration
    (`consideration_for` admits only a party's own account, and the market builds true
    drafts).

  The loop is `ex1a-mut.py` and `ex1a-mut.sh` (scratch, run on conclav.dev).

**Divergences.**
- **Files outside mush-market, approved by the starting session:**
  - `mush-economy/src/contract.rs`, `contract_live.rs`, `module.rs` and `lib.rs`, each
    addition in its own block;
  - `mush-composition/src/lib.rs` (one line).
- **Files outside mush-market that the brief's done-when asks for:**
  - `mush-bot/src/mine/minds.rs` (the sentence table);
  - `docs/help/exchange.md` (the help topic).
- **The seam also answers at the offer**, of the draft contract with `against: None`, when a
  seller names a player bid's account. The trait's signature is the one approved.
- **tacsim's own composition root** (`tacsim/src/composition/mod.rs`) is not wired, so a
  personal listing refuses there, failing closed. EX.1b wires it when it builds the venue
  side.
- **A personal bid's intake is not restored** on a refunded sale.
- **The API admits an org seller for asks**; the verbs do not yet.

**Not built.**
- **EX.1b's venue side:** the endpoint by presence, locker vaults, the non-port gate, the
  Co-op share on every sale, offline buys, and the seam in tacsim's root.
- **An offline seller's lapsed exact lot stays held until they withdraw it.** The market's
  sweep cannot stage a holder's release. tacsim's `release_lapsed_hold` makes it for port
  houses. With the 14-day default the window is long.
- **Renewal for pooled lot holds**, the port counters' (no inventory renew op exists).
- **`org=` on the verbs**, and org bidders.
- **Fees (EX.4), one asset sale (EX.2) and services (EX.5).** EX.4's sale cut and levy must
  widen `ListingAcceptance`'s ask rule, which today requires every outcome to pay the
  seller's own account the whole price.

## EX.1b

Row: §7.1 EX.1b; Q-48-49, Q-48-56; doc 48b G-6, G-7, G-8. Stacked on EX.1a (#632).

**Survey.**
- **The endpoint.** `PortVenueGate::endpoint_for` took the actor's helm hull: `helm_of`'s
  `.min()` over the programme rows and the titled hulls (`coop.rs:602-638`). `hull_admits`
  refused any other hull. So a captain with two hulls could only ever trade into the first,
  wherever it lay.
- **Off-hold storage was free.** `PortCapacity::plan` answers only for a hull's hold
  (`governed_hold`), so every other vault had no plan and no limit (`market.rs:477`). A player
  without a hull had no endpoint at all.
- **The gate failed open.** `PortVenueGate::admit` returned `Ok` for a venue no port authors
  (`market.rs:258-263`). The only such venue today is the launch counter, and its own gate
  (`launch/venue.rs`) calls `PortVenueGate.admit` first, then checks its room.
- **The Co-op share.** mush-market asks `ProceedsPolicy` only on a sale into a bid
  (`trade.rs:2429`). EX.1a's `ListingAcceptance` requires an ask's outcomes to pay the seller
  the whole price. Posting asks tacsim only `gate.admit(actor, venue, Some(lot endpoint))`,
  the same call a purchase into that hold makes.
- **The offline buy.** `ask_at` reads the authored vendors' storefront asks only
  (`executor.rs:614-645`), so a personal ask was out of reach. Q-48-56 keeps it that way unless
  the order names a ceiling.

**Built.**
- **The endpoint by presence** (`coop.rs` `trade_hulls`, `market.rs` `choose_hull`):
  - every hull whose hold the actor may trade (the fleet hull, the ferry, the live charter, the
    titled programme rows, every commissioned hull), each held to `may_trade_hold`;
  - a trade at a port delivers into the first of those docked there; failing that, the actor's
    locker at the venue; failing that, the helm hull, so the gate's refusal still names it;
  - `hull_admits` admits any of them.
- **Venue lockers** (`port/locker.rs`, `+locker`):
  - one vault per player and venue, backed by a container: a locker object in LIMBO owned by
    the player, so the economy's built-in custody provider serves it. A vault is a locker when a
    locker record names it, and it is never a hull's hold;
  - opened by `+locker/open` where the player stands; `+locker` shows what it holds and owes;
  - capacity-bound: the capacity plan of a locker vault is one compartment that takes any
    stowage, held to 20 t and 40 m³ (provisional);
  - what it owes: 1 cr an IC day per started tonne (provisional, the berth's rate), accrued in
    whole IC days onto the record's `owed_minor` when `+locker` shows it. Nothing collects it.
  - The record is `tacsim:locker:{player}:{venue}` in the store's own block; an unreadable one
    is skipped loudly, and its vault stands.
  - A locker loads only at its own venue and only for its owner (`LOCKER_ELSEWHERE`).
- **The location gate** (`market.rs` `placed_admit`, `venue_room`): a venue no port authors
  trades only with an asker standing in its room, in person. Today that room is the launch
  deployment's counter. A venue the game places nowhere is refused (`VENUE_UNPLACED`).
- **No personal ask from a chartered hold** (`proceeds.rs` `CoopProceeds::may_list`): the
  posting is refused with `CHARTERED_ASK` while the hold is Co-op-enrolled; any other source
  lists as before. mush-market's `ProceedsPolicy` gains `may_list`, defaulting to `Ok`, called
  once in `seller::post_ask` after the gate.
- **The offline buy** (`executor.rs` `ask_for_order`): the houses' lowest ask, and, only when the
  order names `max=`, the lowest personal ask at or under it. A personal ask is one exact lot
  sold whole, so it is a candidate only when the order wants all of it and the hold's room and
  the pilot's funds cover it. At an equal price the house wins. The `+ship/orders` preview makes
  the same choice.
- **tacsim's own root** wires EX.1a's `with_standing_acceptance(ListingAcceptance)` beside the
  market's other economy seams, so a personal listing stands in tacsim's compositions too.
- Help: `help exchange` gains where goods go, lockers, counters and chartered cargo; `+locker`
  has its own help. The bot's sentence table has a `venue` family.

**Tests.** (conclav.dev, `CARGO_INCREMENTAL=0`, on EX.1a's head `c9e37c57c` with this unit.)
- **Unit:** `port::market` (`ex1b_the_endpoint_is_the_hull_docked_where_the_actor_is`,
  `ex1b_an_unplaced_venue_refuses`) and `port::locker` (the accrual by started tonne and whole IC
  day, the key, the capacity plan refusing 21 t). With the existing `port::{market, locker,
  executor, proceeds, coop, orders}` tests, 43 pass.
- **Stories** (`tacsim/tests/domain_economy/ex1b_venue.rs`):
  - `ex1b_a_hull_less_player_buys_into_their_locker`: `+locker` before and after
    `+locker/open`; the record across a restart; a purchase with no hull lands in the locker;
    18 t more is refused at 20 t; two IC days later `+locker` shows 6 cr owed.
  - `ex1b_a_chartered_hold_cannot_post_a_personal_ask`: a lot in a Co-op-chartered hold is
    refused with `CHARTERED_ASK`; the same captain's locker lot lists.
  - `ex1b_an_offline_buy_takes_a_players_ask_only_under_a_ceiling`: with a player's two-tonne
    lot listed under the house's price, `buy metals 2` takes the house's ask; `buy metals 1
    max=<player's price>` takes nothing (the lot is sold whole and the house is over the
    ceiling); `buy metals 2 max=<player's price>` takes the lot, whole, into the hold.
- **Existing, green:** all of mush-market (258); `mush-server --test commerce_composition` (91,
  the mind commerce through the launch counter's gate); tacsim's `orders::*`,
  `in5_production::*`, `bay_gc8::*` and `in11_matter::*` stories; the bot's `minds::` tests (35,
  with the new `venue` family in step, distinct and under 120 columns).
- **The M7 market stories** (`player_market_story`, `player_market_typed`) were **not
  verifiable**: they are red on main from #623's regression, and its fix had not landed when
  this PR opened. The counter they trade at is the one venue `placed_admit` places; the 91
  composition tests that trade there pass.
- **Gates:** `cargo fmt --check`; clippy `--all-targets -D warnings` on tacsim, mush-market and
  mush-bot; the M7 memory-size clippy; rustdoc `-D warnings`; `mush-contract-tests`' `ci_policy`
  (80) and `help_commands` / `state_slot_owners` (10).
- **Mutations:** 18, each alone, against the tests named to kill it; 12 killed. The six that
  survive:
  - **M2** (the docked-hull choice skipped): every story's actor has one hull, so the helm
    fallback lands in the same hold.
  - **M3** (`hull_admits` admits any hull): `endpoint_for` offers only the actor's own holds,
    and the market's `may_take` refuses another's vault first.
  - **M4** (the counter's location check): the launch counter's own gate repeats it.
  - **M9** (another's or another venue's locker): `may_take` refuses another's vault first; the
    other-venue case has no story.
  - **M13** (a personal ask winning a tie with the house): no story prices them equal.
  - **M14** (a personal ask over the ceiling): the story's ceiling equals the player's price.

**Divergences.**
- **Files outside the row's five, approved by the lead:**
  - `tacsim/src/composition/mod.rs`: the standing-acceptance line;
  - `tacsim/src/port/locker.rs`, new, and its `mod` line in `port/mod.rs`;
  - `tacsim/src/store.rs`: the locker map, its boot load and its put, in their own block;
  - `tacsim/src/commands.rs`: `+locker`'s registration, one statement;
  - `mush-market/src/module.rs` (`ProceedsPolicy::may_list`) and `mush-market/src/seller.rs`
    (its one call in `post_ask`);
  - `docs/help/exchange.md` and `mush-bot/src/mine/minds.rs` (the sentences);
  - `mush-contract-tests`' `state-slot-owners.toml` (the locker's vault id from the economy's
    allocator, `coop.rs`'s precedent) and `legacy-handlers.toml` (`+locker`'s row).
- **A locker opens with `+locker/open`**, not silently on first use: the gate that would see the
  first use is read-only and cannot create a vault.
- **What a locker owes accrues when it is shown**, at the tonnes held then, in whole IC days. A
  daily pass would measure each day's own contents; EX.4's player-owed obligation replaces it.
- **The endpoint falls back to the helm hull** after the locker, so a captain whose only hull
  is elsewhere still reads the gate's refusal naming it.

**Not built.**
- **The Co-op share on a personal ask** (G-7). It is refused at posting instead. The hook is
  EX.4's: `accept_at` (`mush-market/src/trade.rs:2429`'s sale-into-bid path is the only one that
  asks `ProceedsPolicy`) must ask the policy for a personal ask from a governed hold, and
  `ListingAcceptance`'s ask rule must admit the policy's share row.
- **Collecting the locker's charge**: EX.4.
- **Station and site venues**: none exists; `venue_room` is where each gets its row.

## EX.2

**Survey.**
- **Three copies of one contract.** The market's purchase of an `asset` listing
  (`trade::purchase_spec`'s asset arm), the Co-op buyout (`port::buyout::buyout_spec`) and a
  recovery house's resale (`port::house_sale::sale_spec`, RS.6c-2, written as a copy of the
  buyout's) each wrote the same contract by hand: the buyer issues and pays, the seller is the
  one party, the price is escrowed and paid whole to the seller, the title passes at
  settlement while the seller still holds it and the window is open, and a lapse refunds.
- **Nobody listed an object.** Only a vendor row could carry an `asset` listing, and no
  vendor in the content has one. EX.1a's personal seller lists exact lots only.
- **The hold.** A hull's hold is the vault backed by her stowage (`port::ship::hold_vault`).
  `+coop/enrol` re-owns it to the pilot, a recall re-homes it to the Trust
  (`mush_economy::rehome_vault`), and a buyout leaves it the pilot's. Its contents are typed
  action state: `port_tick::read_pool` reads them from a `Ctx`, and a `FactProvider`, which is
  handed `&World`, cannot.
- **The bay** stripped, scrapped and fitted a hull with no regard to a sale.
- **The house-sale guard** keys on the contract's structure (`house_sale::is_sale_of`, doc 48
  §4.3), not its name. EX.2 keeps it as it is.
- **What Tycho resells.** A recovery house sweeps wrecks off lane sites: rigs. A rig's hopper
  vault belongs to the vehicles principal (doc 20 §4.2, `port::vehicle`), never to the house
  or a buyer.

**Built.**

*One asset sale* **(expensive to reverse).**
- `mush_market::asset_sale_spec(&AssetSaleTerms) -> ContractSpec` is the one builder. Its
  terms are the name, the outcome's name, the object, the seller and their account, the buyer
  and their account, the price, the window's deadline, and a list of facts.
- The contract: the conditions `TitleHeld { object, by: seller }`, `Before { deadline }`, then
  one `Fact` each; one outcome on every condition, paying the seller's account
  `Share::Exact(price)` with the title as its asset consequence; the remainder to the buyer;
  the buyer as author and payer; deterministic; `Expiry::Refund`; `Parties` visibility; and
  `Deliverable::TitleTransfer { object, from: seller, to: buyer }`.
- **On it:** the market's asset purchase (`purchase_spec`, now an early return), the Co-op
  buyout (`buyout_spec`) and a recovery house's resale (`house_sale::sale_spec`). Each passes
  its own name, outcome and window. The buyout and house-sale shape tests
  (`ic13_the_buyout_contract_is_the_market_asset_shape`,
  `rs6c2_the_sale_contract_is_the_buyout_shape`) pass unchanged, as do their stories.
- `sells_title_of(c, object)` and `live_title_sale(w, object)` read a live sale of an object's
  title from the contract's shape.

*The asset-sale seam* **(expensive to reverse).**
- `mush_market::AssetSale`, frozen at composition by `MarketModule::with_asset_sale`, with
  one implementor, `tacsim::port::hull_sale::HullSale`, wired in both roots by
  `with_port_seams_if`. It answers two questions and builds no plan:
  - `admit(ctx, object, seller) -> Option<Result<CatalogRef, String>>`: may `seller` list
    `object` now, and as which catalogue entry. `None` for an object it does not know, `Err`
    for its own refusal, rendered verbatim at posting.
  - `state_digest(w, object) -> Option<u64>`: the digest a sale is priced against.
- With no seam, no person lists an object, and every vendor asset listing is as before.

*No bait-and-strip.*
- **The pin.** A quote stamps `ObservedRevisions.state` with the seam's digest of the
  listing's object (serde default `None`, not written when `None`). `is_current` refuses a
  quote whose object's digest has moved, so `+market/accept` on it is `STALE_QUOTE`.
- **The fact.** The purchase contract carries `Fact market.asset.state "{object}/{digest}"`,
  answered by `MarketFactProvider` from the seam: satisfied while the digest holds, false once
  it drifts or the seam stops knowing the object. A drifted sale never settles and refunds at
  its window, as a Draw does. `accept_at` refuses an accept that would pin a fact no `market`
  provider answers.
- **Re-admitted.** A person's object listing is asked of the seam again at every quote and
  every accept. A refusal there is `STALE_QUOTE` (one family, D-E6-13), so a hull laden after
  listing is neither quoted nor bought.
- **The bay's lock.** `bay_work::under_sale` is `mush_market::asset_on_sale`: a listing names
  the hull, or a live contract sells her title. `+bay/strip` and `+bay/scrap`
  (`strip::ready`) and `+bay/fit` (`take`) refuse `UNDER_SALE` before any other check that
  could move her.

*Listing a hull.*
- `+market/post #<hull> price=<n> [to=<player>] [market=<venue>] [days=<n>]`
  (`commands::post_object`, `seller::post_asset`). It refuses, before any write:
  - a seller the actor may not sell as, or an object whose live, unfrozen title is not the
    seller's (`NO_SUCH_ASSET`, one answer);
  - a governed core itself (`MANAGED_CORE_SALE`, its own family);
  - no seam, an object the seam does not know, or no `asset` entry for it (`NOT_LISTABLE`);
  - the seam's own refusal;
  - an object already listed or being sold (`ALREADY_FOR_SALE`);
  - then EX.1a's checks: price, private offer, account, venue gate, caps.
- The listing: `vendor: NO_VENDOR`, `source: Stock { vault: NO_VAULT }`, `object`, quantity 1,
  the seller's own account. `record::check_personal` admits that one new shape. There is no
  reserve leg: the seam and the pin hold the object. EX.1a's `ListingAcceptance` accepts the
  purchase for the seller, so it settles with the seller away.

*TacSim's hull seam* (`port/hull_sale.rs`).
- **Admit:** a hull titled to the seller. First, no governed core fitted (`HULL_CORED`, for
  now: "A hull with a governed core fitted cannot be listed yet: the core sells only through
  its own sale, which is not open."). This is asked before her design, because a fitted core
  rebinds her to a cored unit that no yard budget partitions. Then a ship hull (her design's
  budget and a hull view; a rig is not one), with, in order:
  - not held in a workshop's bay (`HULL_IN_BAY`);
  - nothing in flight (the buyout's and recall's `in_flight_on` lines);
  - no leased rig in her bay (`HULL_RIG_ABOARD`);
  - an empty hold: every goods-table row that is no consumable reads zero
    (`HULL_HOLD_LADEN`; unreadable, `HULL_HOLD_DARK`). Her fuel and stores go with her.

  Admitted, she lists as `tacsim:hull.<design>`.
- **The digest:** FNV (`declared_hash`) of her schema, sections and installed components
  (each part, its condition and its titled asset, the core among them), never her volatile
  cells.
- **The catalogue:** a `tacsim.hulls` pack beside the goods and the cores
  (`port/vendor.rs`), one `asset` entry per design the yard partitions, not stackable, with
  no baseline.
- **The hold goes with her:** `HullSaleObserver`, in tacsim's root and in
  `mush-composition`'s `econ_observers()`. On a settled contract, for each `TitleTransfer`
  whose hull's title is now the buyer's and whose hold the old holder owns, it re-homes the
  hold to the buyer (`rehome_vault`, an organization at its root scope), in the settlement's
  own batch. A replay finds the hold moved and does nothing.

*Wiring.* tacsim's economy root gains `with_standing_acceptance(ListingAcceptance)` (EX.1b
adds the same line; the second to merge keeps one copy).

*Help and sentences.* `+market`'s help and `help exchange` gain "Listing a hull". The new
sentences are `pub const`s: four in `mush_market::seller` (the bot's `exchange` family) and
five in `tacsim::port::hull_sale` (a new `hull_sale` family in `mush-bot/src/mine/minds.rs`,
with in-step, census and distinct tests). `UNDER_SALE` now says the hull is up for sale.

**Tests.**
- **The done-when, test by test** (`tacsim/tests/domain_economy/hull_resale_ex2.rs`, three
  stories on the composed lane, and one in `domain_field/co6_cored_ship.rs`):
  - **A player sells a hull to a player while the seller is offline, the hold moves, and a
    strip after listing refuses:**
    `ex2_a_captain_sells_her_hull_while_away_and_her_hold_goes_with_her`.
    - A first captain buys her Dray out (`+coop/buyout`) and lists her at the exchange.
    - The listed hull refuses `+bay/strip`, `+bay/scrap` and `+bay/fit` (`UNDER_SALE`), and
      a second listing (`ALREADY_FOR_SALE`).
    - From then on the seller types nothing. A second captain quotes and buys her, and the
      sale contract records the seller's standing acceptance.
    - The next sweep pays the seller the price, passes the title and re-homes the hold to
      the buyer. The listing is gone, the bay lock is off, the seller no longer has her helm,
      and her own `+market/post` hears `NO_SUCH_ASSET`.
  - **A laden hold:** `ex2_a_laden_hull_is_neither_listed_nor_quoted`. She is laden after her
    quote, so the accept is stale and a new quote is refused. Withdrawn and posted again,
    she hears `HULL_HOLD_LADEN`.
  - **No bait-and-strip:** `ex2_a_hull_changed_after_the_quote_is_not_sold`. Changed after
    her quote, the accept is stale. Changed after her purchase, the sale does not settle.
    Past the window it is `Expired`, the buyer is refunded, and the title and hold stay the
    seller's.
  - **A cored hull is refused with its reason:**
    `ex2_a_hull_with_a_governed_core_fitted_is_not_listed_yet`. After CO.6's own route to a
    cored ship, the hull hears `HULL_CORED` and the core itself `MANAGED_CORE_SALE`. Nothing
    is listed.
  - **Buyout and Tycho's resale use the one builder, unchanged:** `coop`, `in10_pools`,
    `house_resale_rs6c2` and the two shape tests pass as they were.
- **Unit tests:**
  - `trade.rs`: the builder's shape with and without facts; the state pin's wire.
  - `facts.rs`: the fact against a test seam (pinned, drifted, an unknown object, malformed
    ids, a named revision, no seam).
  - `record.rs`: a person's object listing on the wire.
  - `hull_sale.rs`: the catalogue code, and the refusals under 120 columns.
- **On conclav.dev:**
  - every mush-market and mush-economy test passes (1,169);
  - `tacsim --test domain_economy` passes in full (593);
  - `domain_field`'s `co6_cored_ship` passes (3);
  - `mush-server --test commerce_composition` passes (91);
  - the whole of `mush-bot` passes (798, the new rows' family included);
  - `tacsim`'s lib tests over the buyout, the house sale, the bay, the vendor and the hull
    sale pass (66).
- **The M7 market stories** (`player_market_story`, `player_market_typed`): both pass on the
  tree merged with main `b4254e826`, which has #636's fix to the kernel's resale offer cap.
  Before that merge they failed exactly as on base main `c85a8744f`: `+minds/resell` was
  refused at its prepare.
- **After the merge with main** (`b4254e826`: EX.1b, #636, PL.2, PL.7, CC.4, CC.7, CC.9), on
  conclav.dev, these pass:
  - the format check, the bot's `minds::` rows (38), `mush-contract-tests` and workspace clippy;
  - mush-market and mush-economy (1,171);
  - `tacsim --test domain_economy` in full (603, EX.1b's venue stories included);
  - `co6_cored_ship` (3);
  - `commerce_composition` (91);
  - the M7 stories (2).
- **The gates:** on the final tree, `cargo fmt --check`; workspace clippy with
  `-D warnings`; the M7 memory-size clippy; rustdoc with `-D warnings` on mush-market, tacsim,
  mush-composition and mush-bot, with and without private items; `mush-contract-tests` (the
  census, the panic budget, the restriction lints, the help-page lint, the shard names and the
  state-slot baseline); the bot's `minds::` rows (35); mush-market and mush-economy (1,170);
  and the stories again (51 in `domain_economy`: the hull resale, the house resale, the Co-op
  buyout and the sale stories; 3 in `co6_cored_ship`).
- **The mutations:** 32 run, each alone, against the tests that can see them. 23 are killed.
  The 9 that survive:
  - **Masked by an earlier check:**
    - **MK2:** the quote's refusal of a person's object listing whose state the seam cannot
      read (the seam reads every hull it admits).
    - **MK5:** the accept's refusal when no `market` fact provider is composed (both roots
      compose one).
    - **MK9:** the re-admit's check that the seam still names the listing's own entry (a
      hull's design does not change while she is listed).
    - **TH8:** the observer's check that the old holder owns the hold (in the buyout the
      hold is already the pilot's, and a rig has no hull view).
    - **TH9:** the observer's check that the title is now the buyer's (a settled transfer
      has passed it).
    - **TH10:** `admit`'s own title check (`post_asset` checks the title first).
  - **Not reached by any story:**
    - **TH2:** a hull in a workshop's bay;
    - **TH3:** something in flight on her;
    - **TH4:** a leased rig in her bay.

    Each reuses the rule the buyout and the recall already apply (`bay_work::held`,
    `in_flight_on`, `bay_slot_word`).

  The loop is `ex2-mut.py` and `ex2-mut.sh` (scratch, run on conclav.dev).

**Divergences.**
- **Files beyond the approved list:**
  - `mush-market/src/trade.rs` (the builder, the pin, the re-admit) and `lib.rs` (exports);
  - `tacsim/src/port/buyout.rs` and `house_sale.rs` (moved onto the builder);
  - `tacsim/src/port/bay_work/mod.rs` and `strip.rs` (the lock);
  - `tacsim/src/port/mod.rs` (the module line);
  - `tacsim/tests/domain_economy.rs` (the module line);
  - `tacsim/tests/domain_field/co6_cored_ship.rs` (one test appended: a cored hull needs the
    M8 world's core-fitting route);
  - `mush-bot/src/mine/minds.rs` and `docs/help/exchange.md` (the sentence rows and help the
    brief asks for);
  - `mush-contract-tests/state-slot-owners.toml` (the facts test's seed, named in the existing
    reason).
- **The story file is `hull_resale_ex2.rs`**, not `hull_resale.rs`: the shard guard refuses a
  `domain_economy` module whose name ends with another's (`sale`).
- **The seam answers two questions, not three.** The hold's re-home is `HullSaleObserver`'s,
  keyed on the settled contract's `TitleTransfer` and on who owns the hold, so it covers every
  sale of a hull's title (the market's, a house's) without a third seam method.
- **Tycho's resale re-homes no hold.** It sells rigs, and a rig's hopper is the vehicles
  principal's, not the house's. The observer runs on that settlement and finds no hold to
  move. So there is no test that Tycho's resale re-homes a hold; the re-home is tested on the
  player sale, and `rs6c2` passes unchanged with the observer wired.
- **The pin does not cover the hold.** A fact cannot read typed pool state, so the empty hold
  is the seam's rule at posting, quote and accept instead. Between the accept and the
  settlement (one sweep) a seller can still put goods into the hold, and they go to the buyer
  with it. Fuel and stores ride with her unpinned.
- **A personal object listing reuses `Supply::Stock`** with `NO_VAULT`, not a new supply arm.
- **A lapsed hull sale leaves her unlisted**: the purchase took the listing, as it does every
  exhausted personal ask. The seller lists her again.
- **The pin applies to any listing with an object the seam knows**, a vendor's too (no
  vendor lists a hull today).

**Not built.**
- **Cored hulls.** Refused, for now: the core's own sale (doc 47's CO.13, the managed-title
  wall) is not open.
- **Rig resale between players.** The seam knows ship hulls only.
- **Org sellers on the verb** (`org=`), as in EX.1a.
- **Pinning the hold's contents or her consumables.**

## EX.2b

Row: §7.1 EX.2b; Q-48-47, Q-48-3's correction, Q-48-52's core leg; doc 48b W-48-1a, G-1(2).
Built 2026-11-16/17 on `claude/ex2b-core-resale-32f0ba`, cut from main `73d05678b`.

**Survey.**

- **Why CO.13 could not serve.** Its kernel image admits only a Thing or Room destination
  (`mush-core/src/core_sale.rs`, the `SaleObjects` rule); its economy plan has one
  composition-time org buyer, two lines and no holds (`registry_filing/core_sale.rs`). A player
  buyer needed a second closed plan, not a widened first one.
- **Why a cored hull cannot ride the exchange's E4 sale.** EX.2's hull sale settles in the
  economy sweep's generic contract settlement, and every generic path meets
  `require_unmanaged_mutations` on the core's managed title. A settle path with a managed leg is
  what doc 46 ruled out, and E4 settlement and a coordinator union cannot be made one atomic
  step. So "both or neither" is honest only if the hull's title rides the core's closed plan.
- **Where the offer lives.** The kernel policy reads a `&World` only and cannot see typed
  state, so the offer (an action-state record) is the producer's: read, checked and removed in
  the acceptance's own transaction. The policy re-reads the world facts and an opaque pin.
- **The fitted core** sits in the hull (`location == hull`, the unit's core component's asset).
  Selling the hull does not move it; only titles move.
- **The economy participant writes only listed key families** (`accepts_write`): `vault:` was not
  one of them, so the hold's re-home needed a closed-list entry of its own.

**Built** — the family's shape, **(expensive to reverse)**.

- **Two acts, one union.**
  - *The offer* (`+core/offer <core or ship>=<player> price=<n>`, the seller's own act): tacsim
    action `offer_core_resale` v1 writes one record in tacsim's `core_resale_offers` family,
    keyed by the core: seller, the one buyer, price, the core's title revision and, for a ship,
    hers and the digest of her parts, made and lapse IC seconds. It stands **28 IC days** (the
    owner's floor), until withdrawn (`=withdraw`), replaced or taken. Nothing moves; nothing is
    held.
  - *The acceptance* (`+core/accept <core or ship>=go`, the buyer's act): tacsim action
    `accept_core_resale` v1. Its provider reads the offer (it must name the actor and not have
    lapsed) and the seller's holding on the pinned state, then binds in one coordinator union:
    the economy's closed resale plan, the kernel's closed resale image, the kernel's record, and
    removes the offer.
- **"Escrowed at acceptance"** is read as: the buyer's money leaves their account at acceptance
  and only in the union that delivers title and object, so no buyer is paid-up and undelivered
  and no hold is left to release. There is no hold op; the union is the escrow.
- **Kernel** (`mush-core/src/core_sale.rs` and new `core_sale/resale.rs`):
  - CO.13's image gains a private `Destination` rule: `Shop` (Thing or Room, the buy-back's,
    unchanged) or `Buyer` (a Player, only the resale's fixed action selects it).
  - `CoreResales` (fixed action `tacsim/accept_core_resale` v1), `CoreResalePolicy`,
    `ResaleRequest { core, seller, hull, pin }`, `ResaleFacts`, `ResoldCore`; the tacsim-owned
    `core_resales` record family (operation, core, seller, buyer, hull) with `audit`.
  - Two image shapes: a **stored** core leaves the seller's hands for the buyer's (the buy-back's
    image with the `Buyer` rule; the buyer's carry hooks, as a delivery's); a **fitted** core sold
    with its hull is a fence only (the core's and the hull's exact pre-images and the policy's
    read, kept through custody), writing no object.
  - One variant on the closed `BuiltinObjects` / `BoundBuiltinObjects` list (`Resale`), and
    `PrepareCtx::resell_core` beside `sell_stored_core`.
- **Economy** (`registry_filing/core_sale/resale{.rs,/install.rs,/receipt.rs,/tests.rs}`):
  - `CoreResaleConfig { action, venue, levy, venue_permille, levy_permille }` (the venue is no
    person; the levy is a system sink; the cuts leave the seller something).
  - `RegistryCoreResale { core, title_revision, hull: Option<ResaleHull { hull, title_revision,
    hold }>, seller, seller_account, buyer_account, price }`; the buyer is the actor.
  - The plan: the buyer's own account pays the price; the seller's account is paid the net, the
    venue its cut and the sink the levy (a zero cut has no line); `TransferTitle` of the core to
    the buyer, then of the hull when she goes with it, both in no scope, both revision-fenced.
  - `ClosedCoreResalePlan`: exact plan, the exact before-titles, the exact core birth, exactly
    those lines and transfers, no hold or custody op, and the hull no managed title.
  - The native install (the buy-back's, with up to four lines, up to two titles and the hull's
    hold vault re-homed to the buyer through `put_vault`, its image in the same union).
  - `CoreResaleReceiptAudit`: receipts by commit, folded from the journal, finished at boot,
    checked against the managed-title index (every core a managed birth, no hull one), with
    checkpoint coverage as the buy-back's.
  - One variant each on `EconomyRequest`, `EconomyOutput`, `EconomyPatch`, `BoundEconomy` and
    `PlanGuard` (both arms); `EconomyModule::with_core_resale`; the participant's config and
    `register_with_house_updates`; `load_boot_with_coverage`'s flag;
    `EconomyStore::{core_resale_receipt, core_resale_receipts}`; and `accepts_write` admits
    `vault:<id>` only where the resale composes.
  - `require_unmanaged_mutations` is not touched.
- **TacSim** (new `hardware/sale/resale.rs`): both providers, the trusted policy (`holding`: a
  governed core in the seller's own hands, theirs alone, not impounded, not destroyed; or a ship
  of theirs with one fitted, through `port/hull_sale.rs`'s `core_leg`), the offers' family, the
  `ResaleHost` slot, `+core/offer`, `+core/offers`, `+core/accept`, and the boot pass
  (`boot_audit`, run from the buy-back's) pairing kernel records and economy receipts by
  operation, core, seller, buyer and hull.
- **The cored-hull leg** (`port/hull_sale.rs`, in its own functions for PL.9 and EX.4):
  - `core_leg(w, hull, seller)`: the listed hull's rules read for a cored one (not in a bay,
    nothing in flight, no leased rig aboard, a readable hold), her fitted core, her hold's vault
    when the seller owns it, and her digest;
  - `core_leg_hold_empty(ctx, hull)`: the empty hold, asked at offer and at acceptance;
  - `HULL_CORED` now routes the seller: "sells only with her governed core, by private offer:
    +core/offer #<n>=<player> price=<price>"; `admit`'s branch renders it; `state_digest`'s body
    is `unit_digest`, shared with `core_leg`.
- **Composition.** The resale composes where the fitting shop's buy-back does
  (`HardwareDeployment::buy_back_permille`): `HardwareDeployment::resale` and
  `SaleTerms::resale`, set by the launch from its own venue (the cut) and Registry (the levy).
  No serialized deployment field is added, so no `launch_deployment` digest moves. The roster
  policy and `configure_economy` add `with_core_resale`; `port_tick` registers both actions and
  outcomes; `+core` gains `/offer`, `/offers` and `/accept`.
- **Sentences**: 30 `pub const`s in `resale.rs`, a `core_resale` family in
  `mush-bot/src/mine/minds.rs` (in-step, census, distinct), and `HULL_CORED`'s needle updated.
  `legacy-handlers.toml`'s `+core` note covers the new switches.

**Defaults taken (provisional).**

1. **The rates are Q-48-8's, as constants**, because EX.4's fees have not merged: the venue's
   cut 30‰ (`VENUE_PERMILLE`), the levy 10‰ (`LEVY_PERMILLE`). The venue's cut goes to the
   launch's venue recipient and the levy to its Registry. **Cheap to reverse** (constants; EX.4
   points them at its fees).
2. **An offer stands 28 IC days**, the owner's player-response floor. **Cheap to reverse.**
3. **The resale composes with the buy-back.** A world with no fitting shop has no resale.
   **Cheap to reverse** (one composition line).
4. **Prices are typed in centicredits** (`price=<n>`), as every market verb takes them.

**Persisted shapes** **(expensive to reverse)**: the economy fold gains `core_resale_receipts`
(skipped while uninitialized; the boot closes it on every world, so new checkpoints carry
`{"receipts":{},"complete":true,...}`; `e2b`'s format-3 pin strips it, as the buy-back's); the
journal namespace `tacsim.core_resale.v1` and its correlation string; tacsim's action-state
families `core_resales` and `core_resale_offers`; the two action descriptors. No `.snap` moved.

**Tests.**

- `mush-economy` `registry_filing::core_sale::resale::tests` (engine level, six):
  `ex2b_the_split_pays_seller_venue_and_levy`; `ex2b_the_closed_plan_refuses_every_mismatch`
  (ten plan changes, stored and with a hull; moved images; a managed hull; wrong births);
  `ex2b_no_generic_transfer_touches_a_managed_title`;
  `ex2b_a_resale_posts_replays_and_a_forgery_is_refused` (money, both titles, the receipt, the
  replay, four forgeries, a repeated commit); `ex2b_a_cored_hull_sells_with_her_core_or_neither_moves`
  (another's hull, a claimed hull: refused whole, the core and the money unmoved);
  `ex2b_the_prepare_read_refuses_what_may_not_be_resold`.
- `mush-core` `core_sale::resale::tests`: the record's shape; only the resale admits a player.
- `tacsim` `hardware::sale::resale::tests`: the lines under 120 columns; the offer's checks; the pin.
- **The stories**, `tacsim` `domain_field` `ex2b_core_resale` (the launch world):
  - `ex2b_a_stored_core_is_resold_to_the_player_its_seller_names`: an offer moves and holds
    nothing; a third party is refused by the verb and by the action typed straight to the
    provider; the named buyer's acceptance pays 500,000 split 480,000 / 15,000 / 5,000, the
    title and the core are the buyer's, one receipt, one record, the offer gone, the buy-back's
    family untouched; a restart replays and pairs; the seller cannot offer it again, the buyer can.
  - `ex2b_a_cored_ship_sells_with_her_core_both_or_neither` (after CO.6's route): the exchange
    routes her; the core alone is refused as fitted; with a lien on her the acceptance is
    refused whole (no title, no money); released and offered again, the acceptance passes her
    title, her core's title and her hold to the buyer in one union, the core still in her; a
    restart reads the same.
- **CO.13's buy-back is unchanged**: `co13_a_stored_core_is_sold_with_its_title`, CO.4's, CO.6's
  three, story 4 and EX.2's cored-hull test (its line updated) pass beside them; all 930
  `mush-economy` tests pass, `e2b` included.
- **The gates**, on conclav.dev on the final tree: `cargo fmt --all --check`; workspace clippy
  with `-D warnings`; the M7 memory-size clippy (it first caught the resale image's and output's
  sizes, now boxed); rustdoc with `-D warnings`, the workspace with private items and the four
  touched crates without; `mush-contract-tests` (census, panic budget, restriction lints, help,
  shard names, state slots); the whole of `mush-bot` (the new `core_resale` rows' in-step,
  census and distinct tests); all of `mush-core`, `mush-industry`, `mush-market` and `mush-roll`
  with their CT.0 snapshots (**no `.snap` moved**); tacsim's ct0, launch and hardware lib tests;
  and EX.2's `domain_economy` stories (`ex2_`, `rs6c2`, `ic13_`).

**Mutations** (`ex2b-muts.py`, in this group's notes folder), each run alone on conclav.dev
against the tests that can see it:

| # | Mutation | Verdict |
|---|---|---|
| P1 | the closed check drops plan equality | caught: `ex2b_the_closed_plan_refuses_every_mismatch` |
| P2 | the resale's `PlanGuard` arm removed (falls to `require_unmanaged_mutations`) | caught: `ex2b_a_resale_posts_replays_and_a_forgery_is_refused` |
| P3 | the auth admits another holder's title | caught: `ex2b_a_cored_hull_sells_with_her_core_or_neither_moves` |
| P4 | a managed hull admitted as a hull leg | caught: the mismatch test |
| P5 | the economy's read lets a seller buy their own | **survives**, masked: the buyer's account must be the actor's, and tacsim refuses first |
| L1 | the levy has no line | caught: `ex2b_the_split_pays_seller_venue_and_levy` |
| L2 | the hull's title does not pass | caught: the mismatch test |
| L3 | the hold is not re-homed | caught: the cored-ship story |
| A1 | the fold ignores the family | caught: the cored-ship story (the restart's pairing) |
| A2 | the receipt admits a line beyond the split | caught: the forgery test (a fifth line) |
| A3 | the boot never closes the resale audit | caught: the stories |
| A4 | tacsim's boot pairing ignores the buyer | **survives**, unreached: no story forges a record whose buyer differs from its receipt's |
| K1 | the resale's rule admits a Thing | caught: `ex2b_only_the_resale_admits_a_player_destination` |
| K2 | the buy-back's rule admits a player | caught: the same |
| K3 | the resale image takes the shop's rule | caught: the stored-core story |
| K4 | the kernel drops its buyer check | **survives**, masked: the policy names the acting buyer |
| R1 | the acceptance admits any buyer | caught: the stored-core story (the third party's action) |
| R2 | the acceptance drops its pin check | **survives**, masked: the kernel policy re-reads the pin |
| R3 | the policy drops its pin check | **survives**, masked: the acceptance reads the pin first |
| R4 | a destroyed core is offered | **survives**, unreached: no story destroys a core it then offers (CO.13's story covers the same `stored_core_destroyed` read for the buy-back) |
| R5 | a withdrawal takes anyone's offer | **survives**, masked: an offer of a core the actor holds alone is theirs or stale |

21 run, 14 caught, 7 survive. R2 and R3 each guard the other.

**Divergences.**

- **The cored hull is not an exchange listing.** Q-48-52 said a cored hull "is listed only with
  its core, the core leg settling in Q-48-47's plan", and the brief "lists the hull with its core
  leg instead of refusing". `+market/post` of a cored hull answers with the route to the private
  offer, and `+core/offer #<ship>` *is* her listing, with both legs in one closed plan. Making
  the E4 sale and the closed leg settle together would need a managed leg in the generic settle
  path. A board notice for a cored hull (quotes answered "ask the seller") needs a small
  mush-market seam change, left for after EX.4 holds the crate no longer.
- **No hold.** "Escrowed at acceptance" is the union itself (above).
- **The offer is the producer's state**, not the kernel's: the kernel policy cannot read typed
  state, so the producer reads and removes the offer in the same transaction and hands the
  kernel an opaque pin the policy re-reads.
- **`accepts_write` admits `vault:`** where the resale composes: the one closed leg that writes a
  vault image.
- **Files beyond the owned list** (closed lists and composition): `mush-core` `participant.rs`,
  `resolution.rs`, `core_output/image.rs`; `mush-economy` `registry_filing.rs`, `prepare.rs`,
  `engine.rs`, `fold.rs`, `boot.rs`, `module.rs`, `tests/e2b.rs`; `tacsim` `hardware/counter.rs`
  (the dispatch), `hardware/composition.rs`, `hardware/flow.rs`, `commands.rs`, `port_tick.rs`,
  `launch.rs`, `launch/deployment.rs`, `tests/domain_field.rs`, `co6_cored_ship.rs` (EX.2's line);
  `mush-bot/src/mine/minds.rs`; `mush-contract-tests/legacy-handlers.toml`. No mush-market change.
- **Between acceptance and custody** the hold's emptiness is not re-read (typed state the world
  read cannot reach), as EX.2's hull sale.

**Not built.**

- A board notice for a cored hull or a stored core (see Divergences).
- An org seller or buyer (`org=`): persons only.
- A resale of a core fitted in a rig or a line tank: ship hulls only (`core_leg` uses the ship's
  hull view).
- A notice to an offline buyer that an offer stands for them (they read `+core/offers`).

## EX.3

Row: §7.1 EX.3 (Q-48-9; doc 48a A-3). Built in two halves, each its own PR: EX.3a (#644, after
EX.1b's #637), then EX.3b on main after it, which closes EX.3.

### EX.3a (2026-11-16): the gate's agents, V6 on the board, the acceptors as data

**Survey.**
- **V6, the works' house purchase** (`port/supply.rs`):
  - Stickney's clerk paid Kasei for a lot of structural alloys by a direct post, from the
    operator's account to Kasei's.
  - It then moved the lot from Kasei's storefront into the works' stores with `@inv/move`
    submissions under a works act, retried by `supply_tries`.
  - Payment and goods were two acts, so a move that never landed left Kasei paid. Nothing
    reached the tape.
- **The gate** (`PortVenueGate::admit`) admits `Presence::Here` only to an asker standing in one
  of the venue's rooms. A port's clerk stands in the staff spine, which is in no venue.
- **The acceptors** (`PortPartyCaps`) came from a rule in code: an org was an acceptor when a
  port map recorded it as a vendor, as the programme's org, as the licence's issuer, or as the
  workshop's or the recovery's operator.
- **The market's buyer** is always `PrincipalId::Object(ctx.actor)`, holding an unscoped account
  of its own. Before it escrows, `accept_at` checks that the actor may take from the endpoint it
  buys into (`may_take`).
- **The works' stores vault** was org-owned with no scope, so no NPC could take from it, and a
  purchase into it was refused as "No such quote."

**Built.**
- **The gate's agents** (`market.rs` `agent_admitted`). Under `Presence::Here` the gate admits,
  besides anyone standing at the venue:
  - a port map's clerk, at its own port's venue only;
  - a registered house helm, at the venue its floor buys at only (`pools.rs` `Floor.buys_at`,
    `helm_buys_at`; the Trust's is Stickney's).

  Players are unchanged. The endpoint and capacity checks still apply to an agent.
- **V6 on the board** (`supply.rs`). On the stock clerk's turn, a purchase starts for a works
  row that is under its floor, not yet bought today, and whose seller's ask lists at least a lot
  with no draw in flight:
  - The operator tops up the clerk's purse by what it lacks for the lot at the ask. That is at
    most one post an IC day a row, keyed `tacsim.works_supply/{stores}/{day}/{good}/{lot}`.
  - The clerk, as itself, quotes the ask and accepts it into the works' stores
    (`Presence::Here`).
  - The market escrows the price, draws the lot and settles. A leg that cannot be staged refunds
    the escrow to the purse at the contract's deadline.

  Three rules govern retries:
  - While a purchase is on the board (`on_board`: a quote whose contract is Draft, Offered,
    Active, ReadyToSettle or Disputed), no other starts, and the stock arm goes on to its list
    step: the market keeps the ask's draw in hand until the contract settles.
  - With the engine's queue full, the step defers before any money moves. This is IC14 row 2's
    rule for the sell gate, since a full queue would drop the draw after the escrow.
  - The IC day counts as spent only once the market answers.
- **The clerk's purse** (`pools.rs` `purse_of`): the buyer's own unscoped credit account, opened
  on first use under a name. The helm's purse uses the same opener.
- **The works' stores** (`works.rs` `stores`) open in the operator org's root scope; the scope
  was `None` before. The port's clerk already holds a `vault.withdraw` grant there for the
  operator's storefront (lead ruling L25), so it may buy into the stores. A player who is not the
  operator's agent still may not take from them.
- **The acceptors are lane data** (`def.rs` `OrgRole`; `PortOrgDef.roles`, serde default, left
  out when empty). `PortPartyCaps` accepts for an org whose port def row carries `"acceptor"`.
  The lane gives the role to the twenty orgs the old rule covered.
- **Lane v38 / relay v29**, over PL.3's v37: the role rows, re-signed.
  - The strip chain's new head, `strip_roles`, rebuilds v37
    (`ex3_strip_roles_rebuilds_the_published_v37`).
  - `keep_roles` puts the shipped roles back on a stripped lane, for a suite that boots an older
    lane's shape with today's code.

**Tests.** (conclav.dev, `CARGO_INCREMENTAL=0`, on main `517dce487` with this half.)
- **Stories** (`tacsim/tests/domain_economy/ex3_board.rs`):
  - `ex3_the_works_buys_on_the_board_and_a_missed_leg_refunds`:
    - the works' first lot is a settled quote on the tape after the ports stock up;
    - on the next IC day, with the engine's queue held full for twelve fires, nothing is bought;
    - then the purchase goes on the board, and Kasei's pool moves away before the draw lands;
    - an IC day on, the purchase is closed, no trade, the stores unchanged, and the escrow back
      in the purse;
    - with the pool back, a later IC day's lot settles on the board, paid from the refunded purse
      with no fresh top-up.
  - `ex3_a_clerk_trades_at_its_own_port_only`: Stickney's and Ceres's clerks are admitted at
    their own venues and refused at each other's, in the gate's words.
  - `ex3_the_stores_admit_the_clerk_and_no_player`: Stickney's clerk may take from the works'
    stores; a player may not.
  - `in10_pools`: the Trust's helm is admitted at Stickney's venue and refused at Luna's. Ceres
    admits it by presence, because it stands in Ceres's gallery.
- **Lane:** `ex3_strip_roles_rebuilds_the_published_v37`, and every digest and version pin moved
  to v38 / relay v29.
- **Broad run:** tacsim's lib (2,416), `domain_economy` (613), `domain_world` (275),
  `domain_field` (318), `domain_arena` (1,133) and `cost_lane` (12); mush-server's
  `commerce_composition` (91) and `split_boundary` (227). All green after the test adaptations
  under Divergences.
- **Gates:**
  - `cargo fmt --check`;
  - clippy `--all-targets -D warnings` on tacsim and mush-server, and the M7 memory-size pass;
  - rustdoc `-D warnings`;
  - `mush-contract-tests`' `ci_policy` (80) and `help_commands` / `state_slot_owners` (10).
- **Mutations:** 11, each alone, against the tests named to kill it; 9 killed. Two survive:
  - **M7** (a purchase starts while one is on the board). The IC-day rule already stops a
    second purchase the same day. The check matters only for a purchase still pending across an
    IC-day boundary, which no story builds.
  - **M8** (the in-flight check on the seller's ask dropped). No story has another buyer's draw
    in flight on Kasei's ask when the clerk's turn comes.

**Divergences.**
- **Files outside the row's, approved by the lead:**
  - `port/market.rs`: the gate's agent rule and `PortPartyCaps`;
  - `port/works.rs`: the stores' scope, one field (it was `None`);
  - `port/apply.rs`: one test literal;
  - the lane bump's pins.
- **`supply_tries` is gone** from `PortTickState`, since nothing moves goods by submission any
  more. `owed_moves` and `moving` stay for the works ledger's readers, but are no longer used.
- **A purchase on the board never holds up the stock clerk.** IN.5's step returned `Idle` while
  its move was in flight, which held the clerk's list step for one fire. A board purchase stays
  on the board until its contract settles, so holding the list step that long kept the ports
  from stocking within the rigs' 40 fires. The step now hands the turn on.
- **The purse top-up is keyed by IC day**, not by works act, because a purchase is no longer a
  works act.
- **in5's V6 assertion** now pins that no lot moved under the works ledger. It used to count two
  works acts and their payment keys. The story's 23 days roll past the tape's window, so
  `ex3_board` reads the trades on the tape instead.
- **Tests that boot a stripped lane** and trade before an apply call `keep_roles` after the
  strip: `sale.rs`' suite lane and mill's kept-world seed.
- **mill's one-act check** allows the market's allocator in a fire where Stickney's clerk
  quoted.
- **Other suites, adapted to the works buying on the board** (`supply::buyers` and
  `supply::purchase_on_board` are the readers they share):
  - `commerce`'s lapsed-listing stories count releases apart from the works' buyers' lot
    actions;
  - `coop`'s retirement control accepts Kasei's deal in progress as the one other refusal,
    since its rig never lands the market's draw;
  - `knowing`'s survey commission takes the next contract id: the works' first purchase now
    holds id 0, so that story no longer covers a zero id on the survey path;
  - the composition rigs (`ic_slice_loop`, `ic_buyout_dormancy`, `sm_story`) let the works'
    purchase settle before a jump of a Draw's whole window (a jump that size lapses a drawn
    purchase unpaid, which the market escalates); `ic14` counts contracts apart from the
    works'; `sm8`'s closed sum takes in the buyers' purses; `sm_story` lands the clerks'
    submissions with the house's;
  - `web/src/lib/fixtures/site/site_map.json` re-blessed: four timestamps 6 IC s earlier, the
    stock-up one fire shorter.
- **A house helm stands in its home port's gallery**, so the gate's presence rule admits it at
  its home venue, as it would anyone standing there. The agent rule adds only the venue its
  floor buys at.

**Not built in EX.3a.** V7 and V10, and their failed-leg stories: EX.3b.

### EX.3b (2026-11-17): V7 and V10 on the board

**Survey.**
- **V7, a house order's bill** (`port/pools.rs` `buy`). The helm paid the seller by a direct
  post, keyed by the escrow's count, after lowering the seller's ask (`shrink_ask`). It then moved
  the goods into the yard order's escrow with a works act. A part was one store lot at its
  catalogue price, moved the same way. Nothing reached the tape. The Yards' carbon, stocked and
  bid for but never asked (the owner's IN.11 ruling, lane v33), was sold at its bid.
- **V10, the scrap and the bale's buy-back** (`bay_work/scrap.rs`, `strip.rs`). Whole tonnes and
  part lots minted into the operator's storefront. When the mint landed, the credit posted from the
  operator to the holder; a post that did not land paged staff to pay by hand.
- **The market's sale into a bid** (`sell_into_bid`) sells bulk from a pool only, refuses a
  seller with a submission queued, and takes at most four standing sales from one seller to one
  bidder (`MAX_OPEN_SELLS_PER_VENDOR`). A personal bid drawn to nothing is deleted; a failed sale
  refunds the escrow to the bidder's available balance and does not restore the bid.

**Built.**
- **A private vendor listing** (`mush-market` `ListingSpec.private_to`, new market API surface).
  `list_goods` takes an optional one buyer, checked by `post_ask`'s rule (a live person who is not
  the seller). The listing is `Hidden`; its buyer, its seller and staff see it, and anyone else
  is refused at quote and at accept with `PRIVATE_TO_OTHER`, the sentence the bot table already
  holds. It trades on the tape like any other; only who may see and take it narrows. Every other
  constructor passes `None`.
- **V7 on the board** (`pools.rs` `board`). A bill line in a yard or works order's escrow is bought
  by the helm as itself, a quote and an acceptance into the escrow from its purse:
  - **a bulk good** from the seller's standing ask at the yard's port, at most what it lists, and
    nothing while a draw on that pool is unsettled;
  - **a good stocked and bid for with no ask** (the Yards' carbon): the port's clerk lists the helm
    a private draw ask of what the line wants, at the bid, and the helm takes it on a later fire;
  - **a part**: a lot of the yard operator's stores already standing in its own lot ask is quoted
    as it stands; otherwise the clerk lists the lowest-grade lot as a private exact-lot ask at its
    catalogue price, and the helm takes it on a later fire.

  One purchase stands at a time (`helm_on_board`); none starts while the engine's queue is full or
  the helm has a submission queued. A stores lot standing in a lot ask now counts as held
  (`stores_hold`), so the part is bought rather than ordered at the works. The seller and the
  price are the listing's, so the seam serves any buyer (PL.5). The ferry's stores into her hold
  keep the direct post (outside the row's V7).
- **V10 on the board** (`bay_work/scrap.rs`, `strip.rs`; the lead's rulings):
  - **The holder's scrap vault** (`scrap_vault`, `open_scrap_vault`): one a holder a venue,
    reused by every scrap and return there, the holder's own, uncapped (no capacity plan reads it),
    backed by a container in LIMBO the holder owns, named `scrap at {venue}`.
  - **The mint** of a scrap's or a return's whole tonnes and part lots goes into it; what is under
    a tonne stays in the operator's shop bale as before.
  - **The sale rows** (`sale_rows`): a part at its used value less its removal; a bulk good at its
    value over its whole tonnes, rounded up, at least 1 minor a tonne. A part worth nothing after
    its removal is no part sale: it is not minted as a part, and its mass joins the yield. A good
    with no whole tonne is no row.
  - **The sale** (`sell_step`), on the record (`BayHull.sale`) until every row is sold: one row a
    step, starting in the step the mint lands. The port's buy-back desk posts a private personal
    bid for the row into the operator's storefront, funded from its purse (the operator tops it up
    by what it lacks, keyed by the hull, the act and the row), and the holder sells into it from
    the scrap vault as themself. A bulk row's lot is first dissolved into the vault's pool, as the holder,
    since the market sells bulk from a pool. A refused sale withdraws its bid at once and spends a
    try; at eight the row is given up (`SALE_GIVEN_UP`) and its matter stays the holder's. A row
    waits, spending nothing, while the queue is full, either party has a submission queued, or four
    of the holder's sales to the desk are unsettled.
  - **The buy-back desk** (`scrap::desk`; the lead's ruling (A)): a `DARK` Thing per port, owned
    by the port's clerk, standing in the port's presentation room, with its own purse and a
    `vault.withdraw` grant on the workshop operator's root scope (lead ruling L25's, as the
    clerk's) so it may bid into the storefront. Laid up on the first sale at the port and recorded
    on the port map (`PortMap.desk`, a `DeskRow`; skipped when absent, the schema unchanged): a
    restart finds it and never makes a second, nor a second purse. The money census counts its
    purse as an NPC's (`Other`). The bidder is not the clerk itself because of the open defect
    below.
  - **The gate** admits a scrap vault's owner at that vault's venue under any presence, offline
    included (`market.rs` `admit`).
  - **Nothing is paid off the board.** The hand-pay page is gone. A scrap or a return with nothing
    whole pays nothing, and the holder hears `NOTHING_WHOLE`. The help page says scrap is paid by
    the whole tonne.
- **The bot's sentence table** (`mush-bot` `mine::minds::scrap`): `NOTHING_WHOLE` and
  `SALE_GIVEN_UP`, each under 120 columns.

**Tests.** (conclav.dev, `CARGO_INCREMENTAL=0`, on main `45b50f303`, then rebased on main
`c847b5dd2`.)
- **Stories:**
  - `ex3_board::ex3_the_house_buys_its_bill_on_the_board_and_a_missed_leg_refunds`: eight
    charters breach the Trust's floor; its helm puts the first bill line on the board; Kasei's
    alloys move away before the draw lands; past the deadline the purchase is closed, no trade,
    the escrow empty, the purse refunded; with the stock back a purchase settles into the escrow
    and is a trade on the tape.
  - `in8_mass_scrap::ex3_scrap_sells_on_the_board_past_twenty_tonnes_and_a_missed_leg_refunds`
    (Ceres, Occator's workshop): a whole Dray's 94 t mint into the captain's uncapped scrap vault
    and sell into Occator's stock on the board, paid as they settle; then an escort's tonnes are
    moved off before their sale's leg lands, the captain gone from the port: the bid's escrow comes
    back to the desk's purse, Occator's stock takes nothing, the tonnes stay the captain's, and
    the captain is paid exactly the parts' rows. The desk is the clerk's, laid up once (the same
    Thing and purse at the second scrap), and the money census's total is unchanged by the sale.
  - `house_sweep_rs6c`'s two Tycho stories: Tycho's dispatcher, a GOD-owned NPC holder, sells a
    swept wreck's matter into the desk's bids (its trades on the tape), and the credit reaches
    Tycho through the float.
  - A player still cannot sell into their own listing: `mush-market`'s
    `ex1a_the_personal_verbs_refuse_before_any_write`, unchanged.
  - `ic8::ex3_a_private_vendor_listing_is_its_one_buyers`: a private vendor listing is `Hidden`,
    its one buyer quotes it, a stranger finds no such listing, and naming the vendor's own
    organization lists nothing.
  - `in10_pools` (the Trust's Dray, carbon by the private ask, the plant by its works order),
    `in8_mass_scrap`'s escort and return (now sold through the scrap vault: five trades, 1 t into
    the Yards' stock, the parts' lots in its stores), and `bay_gc8`'s scrap.
  - `scrap::tests::ex3_the_sale_rows_are_the_whole_tonnes_and_the_parts`.
- **Broad run** on `45b50f303` (before the desk): tacsim's lib (2,424), `domain_economy` (615),
  `domain_field` (321), `domain_world` (275), `cost_lane` (12); mush-server's
  `commerce_composition` and `split_boundary` (318); `mush-market` and `mush-bot` (1,067). With
  the desk, again: `domain_economy` (615) and `domain_field` (321).
- **Rebased on `c847b5dd2`** (EX.2b, PL.9, PL.2b and CC.2 merged meanwhile; the one conflict was
  mush-bot's table, both families kept): tacsim's lib (2,431), `domain_economy` (617),
  `domain_field` (323), `mush-market` and `mush-bot`, and every gate below.
- **Gates:** `cargo fmt --check` (tacsim, mush-market, mush-bot); clippy `--all-targets
  -D warnings` (tacsim, mush-market, mush-bot, mush-server) and the M7 memory-size pass; rustdoc
  `-D warnings`; `mush-contract-tests` (200), `state-slot-owners.toml` taking a row for the scrap
  vault's id.
- **Mutations:** 11, each alone, against the tests named to kill it; 8 killed:
  - **killed:** the bulk lot's dissolve skipped (M1); the gate's scrap-vault owner dropped (M3, the
    captain offline); the purse's top-up skipped (M4); `private_to` not written (M5), the listing
    not `Hidden` (M6), the private-offer check dropped (M11); the no-whole-tonne row kept (M8,
    a division by zero); the bulk price rounded down (M10).
  - **M2 survives** (the wait while four sales to the desk are unsettled dropped): one row a step
    and a step's dissolve between the bulk rows keep the escort's five rows under the market's
    cap of four before the first settles; no story builds a holder with five rows ready at once.
  - **M7 survives** (the helm's one-purchase-at-a-time check dropped): no story has the helm
    reach a second bill line while its first purchase is unsettled, so a second purchase is never
    attempted.
  - **M9 survives** (a part worth nothing minted as a part): no story's hull carries a part whose
    removal takes its used value.

**Divergences.**
- **Touches EX.4's crate.** `mush-market/src/api.rs` (`ListingSpec.private_to`, its check and
  visibility in `list_goods`), `seller.rs` (`check_private` made crate-public), and the field on
  every constructor (`commands.rs`, tests `ic7.rs` and `ic8.rs`). EX.4 also works in mush-market:
  whichever merges second resolves.
- **Files outside the row's:** `port/map.rs` (`PortMap.desk`), `port/market.rs` (the gate's
  scrap-vault owner), `port/stock.rs`
  (two constructors), `store.rs` (`bay_selling_hulls`), `bay_work/mod.rs` (the tick's sale loop and
  the landing line, scrap paths only), `bay_work/record.rs` (`BayHull.sale`), `bay_work/strip.rs`,
  `bay_work/command.rs` (the scrap and return lines, the help), `mush-bot`'s table.
- **A bulk row sells a step after its mint lands**, not in the same step: the market sells bulk
  from a pool, and the scrap mints its tonnes as a lot, which the holder dissolves first.
- **The sale is one row a step**, since a seller with a submission queued is refused.
- **The bulk price is rounded up** (at most a minor a tonne over the value), and a tonne carrying
  only the shop bale's earlier remainder sells at 1 minor.
- **Stories that settle a sale step six seconds a fire** past a Draw's window; `days_on`'s jumps
  lapse a drawn sale unpaid.

**Open defect: GOD-owned NPCs act for each other.** `mush_economy::acts_for` holds for an object
over any object it `controls` (`mush_core::perm::controls`), and two objects with one owner control
each other. Every NPC Thing is GOD's, so any two NPCs act for each other, and the market refuses a
personal listing between them as self-trade ("That is your own listing"): Tycho's dispatcher could
not sell its scrap into the GOD-owned clerk's bid. EX.3b routes the bids through the clerk's own
Thing and leaves the ownership model alone. **EX.8 and EX.9**, which add NPC flows, must route
NPC-to-NPC personal listings through a distinctly owned Thing the same way, or raise the defect.

**Owed to the owner (flagged by the lead).** A return or scrap with nothing whole pays nothing
(provisional default).

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
   PL.2 is stacked on PL.7 (#629), whose three core-module recipes joined the pack at v2. v3
   carries them, re-signed over the four new rows together.
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

## PL.2b

**The ruling** (the owner, 2026-11-16: "pl.2b relay pair recommendation accepted"): relay blanks
are made by recipe only up to what the relay registry can install, and the foundry refuses past
that with a sentence. The registry and its 128 KiB budget stay as they are.

**Survey** (at `73d05678b`, main with PL.1 to PL.4 and EX.0 merged):
- **The foundry** (`relay_foundry.rs`) makes one private pair a work order:
  - `tacsim.relay.manufacture/private_pair/1` takes one relay blank and two tonnes of boards
    over 120 work units;
  - `+relaypair/claim` consumes the pair's lot into the book's receipt;
  - `/collect` mints the two titled halves.
- **The only blanks were the staff's.** `+relaypair/stock` credited 8 blanks once, and the book
  (`Fabrications`) recorded `stocked`. `MAX_PAIRS` was a literal 8 bounding the book and the
  claims. No good row or recipe made a blank.
- **The registry's bounds** (`mush-sim/src/relay.rs`): 32 nodes and 64 pairs. A private pair's
  install adds two nodes and one pair (`install_half`). The public star holds five nodes, so 27
  are free: 13 pairs.
- **The boot hazard** (`relay_foundry::boot`): a blank pool in the foundry vault with no book
  fails the boot ("Foundry stock receipt missing"). A produced blank pooled there before any
  stock would stop the server.
- **No tacsim seam refuses an industry draft**, so the limit has to sit where the foundry's own
  operations decide.

**Built:**
1. **The limit, derived** (`relay_foundry::installable`): this foundry's pairs installed now,
   plus the lesser of the registry's free nodes halved and its free pairs
   (`mush_sim::relay::MAX_NODES`, `MAX_PAIRS`). It is never past `relay_foundry::MAX_PAIRS`, now
   `MAX_NODES / 2` (16), the structural bound the book's own check uses. It reads 13 on today's
   registry. It is projected with the foundry's offer (a transaction carries no world) and
   re-checked in the prepare.
2. **What the limit bounds:** the blanks the foundry issues, `Fabrications::issued` (the staff's
   8 if issued, plus every received blank). Each issued blank is a pair to come, needing two
   nodes unless installed, so `issued ≤ installable`. The claim also refuses a pair past the
   limit, and needs an issued blank behind it.
3. **The blank recipe:** manufacture pack v2 (`relay_foundry::PACK_VERSION`, re-signed) adds
   `relay_blank`. It takes 2 t of structural alloys and 1 t of instrument boards into one blank,
   over 48 work units, requiring `relay.entanglement`, so only the foundry makes it. A delivery
   is one lot a line, so an order of N blanks delivers one lot of N.
4. **The blank's receipt:** `+relaypair/receive <work-order>=<lot>` (op `receive`). It consumes
   a completed blank order's delivered lot and credits its blanks to the pool. It records a
   `BlankReceipt` (the order, the lot, the quantity, the owner and the actor) in the book's new
   `blanks` map, skipped when empty.
   - It refuses past the limit with `BLANKS_PAST_LIMIT`: "The relay registry can host 13
     private pairs, and the foundry has issued 13 blanks for them; it takes no more."
   - `/stock` refuses the same way. A claim past it says `PAIRS_PAST_LIMIT`.
   - `+relaypair` shows the blanks received, the blanks issued and the receipts, each against
     the limit.
5. **The boot hazard closed, two ways:**
   - A produced blank stays a lot until it is received, and the boot's check reads the pool
     only, so produced lots in the vault never stop the boot.
   - A blank pool with no book, which only a hand dissolve (`@inv/dissolve`) of a produced lot
     makes, is accepted when the foundry's completed blank orders into its vault cover it.
     Those orders are industry's own durable receipts (`produced_into_vault`).
6. **The stock-once rule is kept:** `+relaypair/stock` still issues the staff's 8 once. It is
   the starting stock and counts against the limit, so production adds up to 5 more on today's
   registry. Received blanks before the stock no longer read as a lost stock receipt.
7. **The good:** goods v10 adds `relay_blank`: family electronics, trade class, 1,200 kg and
   1,500 L each, 2,000 cr, provisional. It is stowed as a kit (`stow.kit`), so no hold carries
   it and no hold's load cells list it.
8. **The census** (`port/census.rs`):
   - `relay_blank` joins the chain goods;
   - the foundry's staff stock is a mint under `stock` (8 × 1,200 kg once issued);
   - a blank order's inputs less its blanks fall under `worked`, as every work order's do;
   - a receive moves a lot into the pool, which is neither a source nor a sink.

**Sizing** (provisional; at 24:1 an IC day is a real hour):
- **A blank** is 2 t of Cabeus alloys (400 cr) and 1 t of Crisium boards (900 cr) at Luna:
  1,700 cr of inputs. The foundry's facility has no cost model, so no fee.
- **48 work units** is two IC days, two real hours a blank. The five blanks today's registry has
  room for take ten real hours of the foundry's one slot. A pair after them takes another five.

**Tests** (run on conclav.dev):
- **New stories** (mush-server `commerce_composition` `ic_slice_loop`, on r9a's harness):
  - `pl2b_blanks_are_made_up_to_the_registry_room_and_boot_again`. At the lunar foundry the
    registry's room reads 13. Then:
    1. Before any stock, an order of five blanks is bought at Luna, run and delivered: one lot
       of five, no book.
    2. A restart over that lot boots.
    3. `+relaypair/receive` takes the lot into the pool (5), and the staff's stock issues its 8
       over it (13 of 13).
    4. A sixth blank's receive is refused with `BLANKS_PAST_LIMIT`.
    5. A second restart, with the pool at 13 and the refused lot standing, boots.
    6. The census's balance is the same before the first purchase and after all of it.
  - `pl2b_a_produced_blank_pooled_by_hand_boots`: a produced blank lot dissolved into the pool by
    `@inv/dissolve` before any book; the restart boots on the completed order's receipt.
- **New unit test:** `relay_foundry::tests::pl2b_the_book_counts_and_bounds_the_blanks_it_issued`
  covers the issued count, a pair needing an issued blank, one receipt a lot, no empty receipt,
  the structural bound, and the wire skipping an empty `blanks` and reading an old book.
- **Re-pinned by record:** r9a's and r9b's drafts name `private_pair/2`, the pack's version.
  r9a's blank counts, 8 then 7 after one pair, hold as they were, since the staff's stock is
  kept.
- **Pins moved by the good:** the goods table's rows, version and digest; the catalogue counts,
  53 (`capacity::ic2_…`, the content studio's lib and mush-server tests); and the orderable goods'
  list (`orders::ic11_the_verbs_edit_the_plan`). The blank is a kit's stowage, so no hold's load
  cells change.
- **Green:**
  - the r9a, r9b and PL.2b stories (5);
  - the whole `tacsim` lib (2,417);
  - `domain_economy` and `domain_arena` (1,742);
  - mush-server `split_boundary` and `commerce_composition` (320; the two PL.2b stories green
    after the fixes above);
  - mush-bot (803).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets`, the M7
  memory-size pass, rustdoc `-D warnings --document-private-items`, and the contract guards'
  feature census, panic budget and restriction lints (24).
- **Mutations:** seven; five killed and two named survivors.
  - Killed:
    1. `installable` ignoring the registry (always the structural bound): the main story's 13.
    2. The receive command unbounded: the main story, whose sixth receive is refused by the
       prepare without the registry's sentence.
    4. Boot not counting produced blanks: the dissolve story's restart.
    5. `issued` not counting received blanks: the unit test and the main story.
    6. The stock refusing a pool that received blanks explain: the main story's stock after its
       receive.
  - Survived:
    3. **The prepare's receive bound.** The command refuses first with the same bound, from the
       same world, so no story reaches the prepare past it. It stays as the transaction's own
       check.
    7. **The claim's "an issued blank behind the pair".** `Fabrications::check` already refuses
       more pair receipts than blanks issued, so the prepare refuses the same claim at the
       book's check.

**Divergences:**
- **The blank is received, not pooled at delivery.** The ruling's "produced blanks a legitimate
  source with their own receipt" is the `receive` op and its `BlankReceipt`. A hand dissolve
  that skips it is accepted at boot on the industry order's receipt, as above.
- **The foundry's sentences are module constants**, as its existing ones are: no bot drives the
  foundry, so none joins the mush-bot sentence table.
- **No industry seam:** a blank order is not refused at draft. Production past the limit is
  refused at the receive and makes blanks the foundry cannot take.

**Not built:**
- **A refusal at draft.** It would need an industry order gate (a seam in `mush-industry`),
  which this unit does not add.
- **Remounting and destroyed pairs** keep their nodes' accounting as before. A destroyed pair's
  blanks stay issued, which is conservative: it never frees room for another.

## PL.3

**Survey** (at `d9d20b4a2`, PL.2's tip stacked on PL.7):
- **The ground rounds have no producer.** GC.7's four munitions goods are a seed at Stickney
  Yards: 4,800 AP, 400 boxes of ball, 800 HE and 120 missiles, asked at 12, 25, 150 and
  1,500 cr. A rearm draws them and nothing refills them.
- **There are no space munitions goods.** A space magazine refills at each check-out (doc 49
  Q-49-12), and a space mount is no catalog weapon (`content/space/mounts.json`), so it names
  no ammunition entry. `MAX_GOODS` is 256, so the catalogue has room.
- **Why PL.2's process row could not make seven goods at one vendor:**
  1. A mill ledger was keyed by its ore (`MillId::derive(pack, entry, org, ore)`). Seven
     processes off one alloys pool would share one ledger.
  2. A vendor took at most four mill rows (`MAX_VENDOR_MILLS`).
  3. A process took one further input. Q-49-4's bill has three goods.
- **The Yards' inputs.**
  - They bid carbon ore (lane v33), with a 20 t seed, and strategic ore.
  - They cannot bid volatiles. The Yards are Stickney's emergency org, and doc 19's rule
    (`PortDef::check_emergency`) refuses an emergency org that asks or bids any of its goods
    (provisions, volatiles, propellant). No other Stickney vendor can host the processes: the
    munitions asks are the Yards', Kasei asks propellant and the commissary asks provisions.
- **The works.** Every recipe that requires `tacsim.works` is offered at the Ceres and Stickney
  works. A quote's fee is the work units at the slot-hour rate plus the margin, about 44.10 cr a
  work unit at either.

**Built:**
1. **A process is keyed by its product** (`port/def.rs` `MillDef::key_good`,
   `port/mill.rs` `MillLedger.process`).
   - A process ledger derives its id from `into`, not `ore`. Mill and refine rows keep the ore
     key, so no existing ledger moves.
   - The warnings, refusals, attempt keys, `mill_listed` counters and the Mill line's head all
     read `key_good`. The line head is `stickney_port mill stickney_yards ammo_he_105:`.
   - `check_vendor_mills` refuses a key authored twice ("mill {key} is authored twice").
2. **A process takes up to two further inputs** (`MillDef.with`, now a list, `MAX_MILL_WITH`
   2).
   - Each is a good the vendor bids or asks: not the ore, not the product, not twice, and
     `per_ratio` 1..=100.
   - In JSON it is one object or a list (`one_or_many`); one entry serializes as an object, so
     PL.2's rows keep their bytes.
   - The ledger keeps one total per further input (`MillLedger.with_t`), and the ledger's check
     conserves each under the ratio.
   - The draw reads each input on its own. A bought input must hold its batch's worth; the first
     short own-stock input is drawn next.
   - The Mill line names them: "with 1 carbon_ore a ratio" at the Yards, and "with 1
     carbon_ore and 1 volatiles a ratio" for a row with two (the two-input story).
3. **A vendor takes up to eight mill rows** (`MAX_VENDOR_MILLS`, was 4).
4. **Goods table v9** (`content/market/goods.json`), three space munitions goods, all family
   munitions and `stow.bulk`. Masses and prices are provisional.

   | Good | Unit | Mass | Volume | Price | Loads |
   |---|---|---|---|---|---|
   | `torpedo` | round | 400 kg | 600 L | 4,000 cr | `tacsim.ammunition.torpedo`, 1 a round |
   | `spinal_round` | round | 120 kg | 150 L | 600 cr | `tacsim.ammunition.spinal_round`, 1 a round |
   | `pd_box` | box | 80 kg | 120 L | 250 cr | `tacsim.ammunition.pd_rotary`, 500 a box |

5. **Recipe pack v4** (`INDUSTRY_RECIPES_VERSION` 4), seven munitions recipes. Each takes a
   tonne each of structural alloys, carbon ore and volatiles, requires `tacsim.works`, and pools
   its output.
6. **Lane v37**, Stickney Yards (provisional figures):
   - **Seven house processes**, one per munitions good. Each takes 1 t of alloys and 1 t of its
     carbon ore a batch, at most one batch an IC day, with its seed as its floor. The outputs
     are the recipes' (table below).
   - **An alloys bid** at 460 cr: intake 10 t, refreshing 2 t an IC day up to 10. No port with
     a route to Stickney asks alloys, so the bid is filled from Kasei's ask next door (450): its
     10 cr a tonne margin is what moves a trader, at most 20 cr an IC day after the first fill.
   - **Space stock and asks:** 8 torpedoes at 4,000 cr, 48 spinal rounds at 600 and 12
     point-defence boxes at 250, all on the yard board.
7. **The lane bump.** Lane v36 → v37 and relay v27 → v28 (re-signed), and
   `support::lane_chain::strip_munitions` as the strip chain's new head (`strip_producers` calls
   it). Pins moved:
   - `LANE_VERSION`, `lane.rs`' `LANE_DIGEST`, `cost_lane.rs`' `SHIPPED_LANE_DIGEST`, and the
     version asserts in `mill.rs`, `drones.rs`, `sites.rs` and `stations.rs`;
   - `relay_registry.rs`' relay digest and kept-world versions, `boot_parity.rs`' relay version;
   - the goods table's `GOODS_DIGEST`, its rows and version; `content_studio.rs`' goods count
     (52);
   - `mill::sm12a_strip_mill_rebuilds_the_published_v13` (the shipped mill list) and
     `ports::ic3_the_readiness_report_lists_every_binding_and_flags_the_rate` (88 commerce
     lines);
   - `port::gauges`' hold lists (three new bulk rows).

**The cost arithmetic** (Q-49-5 read with Q-49-4: a produced round costs about its GC.7 price).
A recipe's cost is its bill at the lane's asks plus the quoted fee, over its rounds:
- **The bill:** alloys 450 cr (Kasei), carbon ore 60 cr (Freeport Works) and water 100 cr
  (Occator), 610 cr in all.
- **The fee:** about 44.10 cr a work unit at the Ceres works: 122 units quote 5,380 cr and 168
  quote 7,408 cr (the story's quotes).
- **The sizing:** every good but the torpedo carries 6,000 cr of rounds a batch at its price. So
  (6,000 − 610) / 44.10 ≈ 122 work units, and the output count is 6,000 cr over the price. The
  torpedo's two rounds are 8,000 cr, so 168 work units.

| Recipe | Rounds | Work units | Bill + fee | A round | Price |
|---|---|---|---|---|---|
| `ap_30` | 500 | 122 | 5,990 cr | 11.98 cr | 12 |
| `ball_762` | 240 | 122 | 5,990 cr | 24.96 cr | 25 |
| `he_105` | 40 | 122 | 5,990 cr | 149.75 cr | 150 |
| `missile` | 4 | 122 | 5,990 cr | 1,497.50 cr | 1,500 |
| `torpedo` | 2 | 168 | 8,018 cr | 4,009.00 cr | 4,000 |
| `spinal_round` | 10 | 122 | 5,990 cr | 599.00 cr | 600 |
| `pd_box` | 24 | 122 | 5,990 cr | 249.58 cr | 250 |

Each lands within 0.25% of its price, the torpedo over (+0.23%) and the rest under (−0.17%).

**The house's arithmetic:** a Yards batch is 1 t of alloys at its own bid (460 cr) and 1 t of
carbon ore at its bid (60 cr), 520 cr, into 6,000 cr of rounds at the asks (8,000 cr for
torpedoes). The house books no works time, as PL.1's refine and PL.2's processes do not.

**Rates** (provisional; at 24:1 an IC day is a real hour):
- Each process makes at most one batch an IC day, 24 a real day. Its floor binds first: a
  process only replaces rounds bought below its seed.
- The alloys bid binds next. After its first 10 t it refreshes 2 t an IC day, so across all
  seven processes at most two batches an IC day, 48 a real day.
- Carbon ore binds last. The Yards' 20 t seed runs 20 batches. Their carbon bid (60 cr, lane
  v33) equals Freeport Works' ask at Ceres, so no haul pays to refill it; it is unchanged here
  (Not built, below).
- An HE batch is 40 rounds, a cannon's 24-round load and two-thirds of another.

**Conservation** (doc 49 §7's requirement). Rounds are conserved: a round is minted only by a
seed, a mill batch or a work order, and leaves the goods only by a rearm. The terms EX.0 must
count:
- **Sources of rounds:**
  - the seed, the stock clerk's mints (`tacsim.port_stock` receipts);
  - a process batch, `product_t` on its `tacsim.port_mill` ledger, keyed by the round's good
    (`MillLedger.process`);
  - a completed work order of a munitions recipe, its pooled outputs times its quantity.
- **What a round becomes:** a rearm moves units of a munitions good into a magazine,
  `ceil(rounds / rounds_per_unit)` units (`port/bay_work/jobs.rs`' `Rearm`, `surface/tender.rs`).
  A magazine's rounds leave by firing, the only true sink. Space rearm is CC.7's.
- **The inputs consumed:**
  - a process batch's ore, `ore_t` of alloys, and its further inputs, `with_t` of carbon ore;
  - a work order's bill, alloys, carbon ore and volatiles times its quantity.
- **The census today** (`port/census.rs`) counts the chain's goods and not munitions. Its mill
  scan reads `counted(&ledger.ore)` and subtracts the product's mass whether it is counted or
  not. For a munitions process it books `milled` as the alloys' mass less the rounds' (1,000 kg
  less 840 kg for an HE batch), and it misses the carbon ore. EX.0's terms for a process ledger
  are the counted inputs' kilograms (`ore_t` and each `with_t`) less the product's only where
  the product is counted.

**Tests** (run on conclav.dev):
- **New:**
  - `domain_economy` `mill::pl3_stickney_yards_makes_he_and_lists_it` runs one full cycle on a
    lane whose Yards seed 760 HE, forty under the floor:
    1. With only its carbon ore in the storefront, nothing is made.
    2. A tonne of alloys delivered in kind, the HE process takes it and a tonne of carbon ore
       and makes 40 rounds. The HE ask is restocked to 800 and its counter is 40. No other
       process runs, and nothing pages drift. The Mill line reads "1 to 40 with 1 carbon_ore a
       ratio".
    3. Nothing moves the next IC day.
  - `mill::pl3_a_process_debits_each_of_two_further_inputs` runs the three-input bill on a lane
    variant whose Stickney has no emergency block and whose Yards bid water. The alloys alone
    make nothing; the water bought, one batch debits each of the three goods and records both
    further inputs.
  - `mill::pl3_the_munitions_recipes_cost_about_their_prices` quotes all seven recipes at the
    Ceres works and holds each within 1% of its price (the table above).
  - `lane::pl3_strip_munitions_rebuilds_the_published_v36` (v36's digest `a38bdfa0…`,
    idempotent).
  - `tacsim` lib: `port::def::tests::pl3_processes_share_an_ore_and_take_two_further_inputs`:
    two processes sharing an ore are accepted, two of one product are refused ("mill metals is
    authored twice"), two further inputs are accepted and three refused, an input named twice
    is refused, and a product that is an own-stock mill's ore is refused as a key authored
    twice.
- **Pins moved, besides the lane bump's:**
  - the goods table's rows (`domain_arena` `content::ic2_the_goods_table_is_pinned`): the three
    space rows, and every ground munitions good names a shipped ammunition while the space
    rounds name none;
  - the goods catalogue counts, 52: `capacity::ic2_the_catalog_rows_derive_from_the_goods_table`
    and the content studio's catalogue (lib and mush-server);
  - the hold's load cells, three zero cells more: `port::gauges`' lib tests,
    `coop::ic6_the_gauges_read_the_hold_pools` and mush-server's
    `ic_slice_loop::ic12_part_one_a_leg_by_hand`;
  - the stock rows minted, 22 (Stickney 12): `commerce::ic8_the_ticks_seed_every_pool_exactly_once`,
    `ic8_a_reboot_mid_seed_re_mints_nothing` and `ic8_a_world_on_lane_v2_takes_v3_by_the_shelf`;
  - the orderable goods' list, `orders::ic11_the_verbs_edit_the_plan`;
  - the web client's `sky_chart.json` (the Yards' alloys bid) and `site/site_map.json` (four
    `at_ic` stamps 42 s later), re-blessed with `TACSIM_BLESS=1`.
- **Two harness fixes the phase shift exposed.** The Yards' clerk mints three more rows, lists
  three more asks and posts one more bid before the ports read Ready, seven fires of 6 s. A rig
  that sets its rate after stocking therefore starts its rate generation 42 real seconds later,
  so every later IC instant moves by 41,958 IC seconds:
  - `claims::sm10_a_block_with_no_deposits_is_claimed_like_any_other` and
    `sm10_a_stake_reads_the_same_whatever_the_block_holds` compare what a staker hears in two
    worlds. The rig's idle return (`tv::IDLE_HOME`) and its panel refresh land a day after its
    last act; on the base the barren world's deposit opening ran 16 fires and caught it, and
    on the shifted phase it ran 10 and missed it. `claim_lines` now drops the rig's own
    reports as it drops the economy's notices: no claim line turns on them.
  - `cost_lane::sm12c_a_restart_inside_the_cohort_changes_nothing` reboots arm B at arm A's first
    quiet point. On the shifted phase that point (IC 117,456) fell on the fire where an Occator
    yard rig's mining cycle landed: its run went on (cycle 2 to 3, next due past the warm), so
    `cohort_quiet`'s projection clause, which compared only whether a run and an act exist,
    read it quiet. The rig's projection and Occator's mined tally are written the next fire,
    arm B's first warm fire. The clause now compares the projection's run view with
    `RunView::of` the record's run, as the tick writes it.
- **Green:**
  - the whole `tacsim` lib (2,394);
  - `domain_economy`, `domain_world` and `cost_lane` (886), `domain_field`, `domain_arena` and
    `harness` (1,472), mush-server `split_boundary` and `commerce_composition` (318) and
    mush-bot (791), on the tree before the last pins; every failure there is a pin or harness
    fix above, each rerun green on the final tree (the economy set touched, 128; `cost_lane`,
    12; the server's `ic_slice_loop`, 12; the arena and field reruns, 72; the chart, 7).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets`, the M7
  memory-size pass, rustdoc `-D warnings --document-private-items`, and the contract guards'
  feature census, panic budget and restriction lints (24).
- **Mutations:** eight, all killed.
  1. A process keyed by its ore (`key_good`): the def tests refuse the shipped lane.
  2. A process ledger's id by its ore: both PL.3 stories and PL.2's Titan story.
  3. No duplicate-key check: `pl3_processes_share_an_ore…` and `sm12a_each_mill_rule…`.
  4. No further input debited: the HE story, the two-input story and the Titan story.
  5. Only the first further input debited: the two-input story.
  6. Readiness reading only the first further input: the two-input story.
  7. The floor gate off: the HE story and the Titan story.
  8. Seven mill rows a vendor: `sm12a_each_mill_rule_refuses_with_its_sentence`, which pins
     the bound at eight.

**Divergences:**
- **The house bill has two inputs, not three.** The Yards' seven processes take alloys and
  carbon ore. Stickney's emergency rule (doc 19 §6, `PortDef::check_emergency`) bars the Yards,
  its emergency org, from bidding volatiles, and the starting session ruled on 2026-11-16 to
  keep that rule unchanged. Player bookings at the works keep Q-49-4's three-input bill.
  Volatiles already have industrial buyers in PL.1's propellant refine at Occator and PL.2's
  farms, so Q-49-4's "first industrial buyer" reason holds.
- **`pd_box` is 80 kg** (provisional), so a batch of 24 boxes (1,920 kg) passes the mass rule
  under the house's two tonnes of inputs.
- **The space rounds name ammunition ids with no catalog entry.** A space mount is no catalog
  weapon, so `torpedo`, `spinal_round` and `pd_box` name `tacsim.ammunition.torpedo`,
  `spinal_round` and `pd_rotary`, which CC.7's berth rearm binds. The goods table's test now
  holds that every ground munitions good names a shipped ammunition and every space round names
  none.
- **The house books no works slot.** It is a house process on the stock clerk's turn, as PL.1's
  refine and PL.2's processes are. The works' own house order (`works.rs` `house_collect`) is not
  booked for goods.

**Not built:**
- **CC.7's space rearm and magazine loading.** The space goods are stocked and asked, and nothing
  draws them yet.
- **The census terms above.** They are EX.0's.
- **Prices do not follow the supply.** The asks stay at GC.7's prices and PL.3's provisional
  ones; EX.8 and EX.9 own the market's response.
- **The Yards' carbon bid pays no haul.** At 60 cr it equals Freeport Works' ask, so once the
  20 t seed is milled the house waits on a carbon sale nobody profits from. Repricing a bid
  authored before PL.3 (lane v33) is left to EX.8's tuning.
- **The Yards have no water.** If a later ruling lets the emergency org bid, the house bill can
  take the recipes' three inputs with a lane bump, no code change.

## PL.4

**Survey** (at `14483cbdf`, PL.3's tip):
- **The houses' three conversions** are mill rows on the stock clerk's turn, house-only:
  - the Freeport Exchange's metal mill at Ceres, metal ore into metals, 5 : 2;
  - Occator's ice mill at Ceres, ice ore into water, 5 : 2 (beside PL.1's refine of water into
    propellant);
  - Kasei's refine at Stickney, metals into structural alloys, 5 : 4.
- **No works recipe takes an ore.** The pack's 25 rows make parts, fittings, feedstock, modules
  and munitions. A player with ore can only sell it into a house's bid.
- **The works** at Ceres (Freeport Works) and at Stickney offer every recipe that requires
  `tacsim.works` as a public slot. A booking runs quote, order, deliver, collect
  (`port/works.rs`). Its fee is the cost model's.
- **The cost model at the Ceres works:** 3,000,000 cr capital over 25 years (328.77 cr a day),
  2% upkeep (164.38), and 8 staff at 150 (1,200). That is 1,693.15 cr a day over 2 slots × 24
  hours, so 35.27 cr a slot-hour; the 25% margin makes 44.09 cr a work unit. A quote rounds the
  fee up to the whole credit. The booking fee (one slot-day before margin, 847 cr) is held and
  comes back at collection.

**Built:**
1. **Recipe pack v5** (`INDUSTRY_RECIPES_VERSION` 5, re-signed), three conversions at the houses'
   ratios, pooled, each requiring `tacsim.works`, so each is a public slot at both works:

   | Recipe | Bill | Output | Work units | Fee | The house's spread a batch |
   |---|---|---|---|---|---|
   | `metals` | 10 t metal ore | 4 t metals | 2 | 89 cr | 100 cr (the Exchange: 4 × 200 − 10 × 70) |
   | `volatiles` | 20 t ice ore | 8 t water | 2 | 89 cr | 100 cr (Occator: 8 × 100 − 20 × 35) |
   | `structural_alloys` | 5 t metals | 4 t alloys | 9 | 397 cr | 400 cr (Kasei: 4 × 450 − 5 × 280) |

2. **The houses' mill rows are unchanged.** The lane is not touched: no version bump, no strip
   step and no fixture.

**Sizing** (provisional; at 24:1 an IC day is a real hour):
- **The rule: the fee is about the house's own spread.** Each recipe's fee lands within one work
  unit under what the house makes on the same batch at its own port's prices: its bid for the
  input and its ask for the output. Milling your own ore at the works then costs about what
  selling the ore and buying the product back costs at that port. It pays where the house's bid
  is full (the Exchange takes 240 t of metal ore at a time) or where the product sells dearer:
  metals bid 280 at Stickney, water 190 to 200 at Stickney, Luna and Deimos.
- **Why the batches differ.** The ratios are the houses'. The ore batches are the smallest whole
  multiples whose fee stays under the spread at two or more work units: 5 t of ice makes only
  25 cr of spread, under one work unit's 44.09. A one-unit recipe would also quote "(1 work
  units)".
- **Slot times**, a work unit being one slot-hour:
  - a 100 t hold of metal ore is ten `metals` batches, 20 work units: 20 IC hours, 50 real
    minutes;
  - its 40 t of metals refine in eight `structural_alloys` batches, 72 work units: three IC days,
    three real hours;
  - the houses' own rows mill 40 t of ore an IC day each, so a slot is slower than a house mill
    for ore and about a third of Kasei's refine rate.

**Tests** (run on conclav.dev):
- **New** (`domain_economy` `pl4_works_mills`, reusing PL.2's works harness, whose helpers are
  now `pub(crate)`):
  - `pl4_a_captain_mills_metal_ore_at_the_works_then_refines_the_metals` (13 s): at Ceres,
    with 20 t of metal ore in the docked hold:
    1. `+works/quote metals 2` bills 20 t for four work units.
    2. The order is booked, the ore delivered and the works run; the captain collects 8 t of
       metals.
    3. `+works/quote structural_alloys` bills 5 t of metals for nine work units. Booked,
       delivered, run and collected, the hold holds 4 t of alloys and 3 t of metals.
    4. The Exchange's metal mill, Occator's ice mill and refine, and Kasei's refine are the
       lane's as before.
  - `pl4_each_conversion_costs_about_its_houses_spread`: the three quotes at the Ceres works
    give fees of 89, 89 and 397 cr against the houses' 100, 100 and 400.
- **Green:** the whole `domain_economy` binary (603, every works and mill story among them), the
  `tacsim` lib (2,394), and mush-server `split_boundary` and `commerce_composition` (318).
- **Gate:** clippy `-D warnings` on `tacsim` and `mush-server` with `--all-targets`, the M7
  memory-size pass, rustdoc `-D warnings --document-private-items`, and the contract guards'
  feature census, panic budget and restriction lints (24).
- **Mutations** (content, each re-signed so the pack loads):
  1. `metals` at three work units: both stories (the quote's units; a 133 cr fee over the
     house's 100).
  2. `structural_alloys` making five tonnes: the mill-and-refine story (the hold's alloys).

**Divergences:**
- **The houses' rows stay house processes.** Q-48-24's "the NPC mill rows stay as the houses'
  own processes": they book no slot and pay no fee, as before.
- **The ore recipes' batches are larger than the houses' ratios' least terms** (10 : 4 and
  20 : 8). Sizing above.

**Not built:**
- **PL.5's escrow buying** waits on EX.3 and is not started here.
- **Prices do not follow the new supply.** A player's milled metals and water sell into the same
  bids; EX.8 and EX.9 own the market's response.

## PL.5

Not built.

## PL.6

Not built.

## PL.7

Cores from the chain (Q-48-27, with doc 48b W-48-1c's correction, and Q-48-48). Built
2026-11-15 on `claude/pl7-cores-from-chain-f210f4`, cut from PL.0's branch (#624). Main was merged
in at `72d5b7150`, then at `c85a8744f` once #624, #625, #626 and #627 had merged.

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
7. **The depot's opening stock** (the lead's ruling: a launched server must not refuse
   `+core/buy`).
   - `OriginPolicy.opening` holds up to eight goods, each 1 to 100,000 units, in order. The
     origin's one `Initialize` credits it into the depot, in its own bundle.
   - It lands through a new `InventoryFamilies::credit_many`, because two new pools at one
     endpoint in one bundle would write the endpoint's pool census twice.
   - The deployment names it: `HardwareCoordinates.depot_stock`, written only when non-empty,
     so a deployment without it keeps its digest. The composition checks that each is a pooled
     good of the shipped table.
   - `deploy/instance/launch-bringup.py` writes it (`DEPOT_STOCK`), and so do
     `launch.json.example` and `deploy/README.md`'s step 7.
   - The origin's index retains it (`initialize_policy.opening`): the census's source for it.
8. **The census's build line** (`port/census.rs`, one line, outside this unit's files): the
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
  - `mush-industry` `core_production::tests::pl7_the_initialization_stocks_the_depot`. The
    `Initialize` opens two pools in one bundle, the index retains the amounts, the first die
    draws on them, and a cold start reads them. The opening stock's bounds are checked too.
  - `mush-inventory` `pool::pl0_tests::pl7_credit_many_opens_several_pools_in_one_bundle`.
    `credit` twice in one bundle refuses. `credit_many` opens both pools, counts them once, and
    credits an existing pool beside a new one. A doubled or zero row is refused and writes
    nothing.
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
    - The rig's deployment opens the depot (`depot_stock`, 200 t of each chain good), so its
      `Initialize` stocks it.
    - The shipped manufacture story checks that its Class I die took a board and a tonne.
    - The launch test checks that the composed policies carry the catalogue's chain and prices.
    - The named-launch test opens the depot through the launch deployment's `depot_stock`, as
      the bring-up does. After the Active boot the depot holds it, and the first die plans.
  - The M8 rig opens the depot the same way, through its coordinates' `depot_stock`
    (`m8_world::DEPOT_STOCK`), so CO.6's and CO.13's stories run on the bring-up's path. And
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
- **Files outside the unit's list:**
  - `mush-inventory` `pool.rs`: the new `credit_many`, and `stage_new_pools`, its census step,
    taken out of `transfer_many` unchanged so the two share it.
  - `deploy/instance/launch-bringup.py`, `launch.json.example` and `deploy/README.md`: the
    depot's opening stock, on the lead's ruling.
  - The `HardwareCoordinates` literals in `mush-server`'s two tests (`depot_stock: Vec::new()`).
  - `port/census.rs`: one line (the build line's `core_kg`). EX.0's terms are in the group's notes
    folder (`pl7-census-terms-for-ex0.md`).
  - `port/stock.rs` `part_row_of`: a core module's row is its core's definition. Without it a
    made module has no provenance, and the yard refuses it.
  - `definitions/mass.rs` `core_minor`: the core reference reads the catalogue.
  - Test support also changed: `excursions.rs` (`keep` made `pub(crate)`),
    `m8_world.rs` and `driving_rig.rs`.
- **`commission.rs` is unchanged.** The commission's consumption already consumes every lot in
  the escrow, so the module needs no new step.
- **Stacked on PL.1 (#628), in the lead's land order: #628, then PL.7, then PL.2.** PL.1's branch
  is merged in. PL.2, the lane chain's one writer, merges this branch and lays its own goods and
  recipe rows on top of this unit's, re-pinning the goods digest and counts on its side.
- **PL.0's unload test** (`surface/act/fixed.rs`) counted the shipped bulk goods exactly. It now
  counts them relative to the shipped table, with the same 47 test goods and the same weigh.
- **`CoreClass::finished_minor()` and its siblings stay**, as the v1 table's names, so the
  stories that read a price (CO.6's, CO.13's, `m8_end_user`) are unchanged. No production path
  reads them.

**What the next go-live (MW.3) gets:**
- The bring-up writes the deployment's `depot_stock`: 112 t of instrument boards and 88 t of
  strategic ore. That is one month of the full quota (64 Class I, 16 Class II, 4 Class III
  dies). It is provisional, named so in the script.
- The origin's initialize action credits it to the Basin die depot once.
- After it, nothing refills the depot. `+core/buy` refuses `CORE_NO_STOCK`, naming the bill,
  once the depot cannot cut a die. Refilling is EX.9's or the lane's.
- `scripts/launch-bringup-acceptance.sh` checks no core buy, so it is unchanged.

**Not built:**
- **No NPC hauls boards or ore to the depot.** Its supply is the lane's or EX.9's, as for every
  finite seed (§3.7).
- **The census's terms** are EX.0's to add: the `cored` sink and the depot's opening stock as
  a source, both named in the notes folder.
- **CO.13's buy-back baseline** (`hardware/sale.rs`, the market's core catalogue entries and the
  quote) still reads the v1 table. That is EX.2b's closed `core_sale` family. At the shipped
  prices the two agree.
- **Heirloom and grey dies** (staff issues) consume no chain goods. They are not made from the
  chain.
- **No vendor stocks core modules.** A module is made at a works from boards and ore.

## PL.8

Not built.

## PL.9

Facilities as titled assets (Q-48-26, Q-48-37, Q-48-45, Q-48-53). Built 2026-11-16 and 2026-11-17
on `claude/pl9-facilities-assets`, cut from main at `73d05678b`. Main was merged in at
`45b50f303`, after EX.3a (#644).

**Survey** (at `73d05678b`):
- **A facility had no title.** The lane opened its refinery, works and yard rooms as
  `FacilityRecord`s owned by the port's operator org, but issued no economy title, so nothing
  could list one. `+market/post #<object>` (EX.2) lists a titled object through the market's
  `AssetSale` seam, which tacsim implements in `port/hull_sale.rs` for hulls only.
- **The lane reverted any owner.** The port applier's preflight refused the whole entry when a
  facility's owner differed from the authored operator (`port/plant.rs`), and
  `install_authored_facility` re-termed a facility back to its authored cost and customers on
  every bump (`mush-industry/src/store.rs`).
- **`@facility/terms` was all or nothing.** It set the customers and the whole cost model
  together, for staff or whoever acts for the owner. Q-48-37 splits it.
- **The works arm charged the live fee.** Doc 48b's fix cancelled an order at submit on any
  policy-revision move, so a lowered margin or a customers change also cancelled.
- **No command listed for an organization.** `post_asset` already let an org sell when the actor
  acts for it, but `+market/post` always listed as the person typing it.
- **The market's dispatch cannot read industry's work orders.** Typed reads are owner-scoped
  (`ModuleActionStateReadError::ForeignOwner`). Inventory's families are delegated to the market;
  `industry.workorder` is not. A walk of the family from the seam answers an error, which read
  as busy for every facility.

**Built:**
1. **A facility is owner-held once sold** (`mush-industry` `facility.rs`).
   - `FacilityRecord.holding`: `Authored` (the default, written by absence, so an unsold
     record's bytes and the industry `.snap` files do not move), `Owner`, or `OwnerPublic`.
   - `sold_to(owner, public)` replaces the owner as `with_owner` does (the policy revision moves)
     and marks the record held; `public` forces Public customers (a sold yard).
   - An `OwnerPublic` record refuses owner-only customers on every path, the wire included
     (`FacilityKeptPublic`).
   - `stands_as_authored`: for a held record the lane compares the object, capabilities and slots
     only, not the owner or terms.
2. **The lane leaves a sold facility alone** (`store.rs`, `port/plant.rs`).
   - `install_authored_facility` returns early for a held record, after the identity check.
   - The port applier's preflight uses `stands_as_authored`, so a sold facility no longer refuses
     the port's entry.
   - `store::sell_facility(ctx, object, owner, public)` persists the sold record; a replay is a
     no-op.
3. **The lane titles its facilities** (`port/plant.rs` `title`). After installing a facility it
   issues an economy title to the operator org under the port registry, if the room has none.
   Without an economy it does nothing, and an issue error is logged, not fatal.
4. **`@facility/terms` is split** (`mush-industry` `commands.rs`, Q-48-37).
   - `<facility>=public|owner [margin=<permille>]` for staff and whoever acts for the owner.
   - The six-part cost spec is staff-only (`TERMS_COST_STAFF`).
   - A margin on an unpriced facility is refused (`TERMS_NO_PRICE`), and a kept-public facility
     cannot go owner-only (`TERMS_KEPT_PUBLIC`).
   - Anyone else reads `No such facility.`, as before.
5. **The facility arm of the asset sale** (`port/facility_sale.rs`, new; `port/hull_sale.rs`).
   - `hull_sale::admit`, `state_digest` and the settled-sale observer dispatch to the facility
     functions when the object is a facility. The facility code is in its own functions, so EX.4
     and EX.2b's edits to `hull_sale.rs` meet it only at those three call sites.
   - Three catalogue entries, `tacsim.facilities:facility.{refinery,works,yard}`, loaded by the
     vendor beside the hulls (`port/vendor.rs`).
   - `admit` refuses, with a sentence naming the room:
     - the relay foundry, the programme workshops and the medical rooms (`FACILITY_FOUNDRY`,
       `FACILITY_WORKSHOP`, `FACILITY_MEDICAL`), and any facility that is not a port's refinery,
       works or yard (`FACILITY_NOT_SOLD`);
     - work under way: an open works or yard contract, a commission still laying up, or an open
       work order (`FACILITY_BUSY`);
     - stores a vendor trades from (`FACILITY_STOREFRONT`);
     - goods in its stores (`FACILITY_STORES_LADEN`), or stores that cannot be read
       (`FACILITY_STORES_DARK`).
   - `state_digest` is the record's hash, so a quote goes stale when the facility changes.
   - **On settlement** (`on_sold`): the record passes to the buyer (a yard kept public), and the
     stores vault, if the seller owned it, is re-homed to the buyer (`rehome_vault`). The payee
     follows the record's owner, which the works arm already pays.
6. **Industry's open-order index by facility** (`store.rs`). `IndustryStore::open_orders_at`,
   kept by the work-order observer and rebuilt by `reconcile` beside the per-requester index
   (WS02-S4). The sale's idle check reads it, since the market's context may not read
   `industry.workorder`.
7. **The fee is pinned at booking** (`port/works.rs`, Q-48-53).
   - `WorksContract.fee_minor` is the facility's fee at booking (absent on older contracts).
   - At submit the arm cancels only if the live fee has risen past the pinned one
     (`fee_risen`): the booking fee and the bill come back, as doc 48b's fix did. A fee that held
     or fell runs. A contract with no pin and a priced facility cancels, as doc 48b's fix did.
8. **An organization lists through `+market/post … as=<org>`** (`mush-market` `commands.rs`, on
   the lead's ruling).
   - The org is named by name, alias or `#id`, with or without `org:`. The actor must act for it
     (`mush_economy::acts_for`, the check `post_asset` makes), or the command refuses
     `NOT_FOR_ORG`; an unknown or hidden org reads the same.
   - `+market/bid` refuses `as=` with its usage.
   - The sentence is in the bot's sentence table (`mush-bot` `mine/minds.rs`).
9. **A sold works buys nothing for the house** (`port/supply.rs`). The NPC house purchase runs
   only for authored facilities.
10. **Help:** `docs/help/exchange.md` gains "Listing a facility".

**Tests** (each failed before its change, or is named under the mutations):
- `mush-industry` `e7r2.rs`:
  - `pl9_the_owner_sets_customers_and_margin_but_not_the_cost_model`;
  - `pl9_a_sold_facility_is_owner_held_and_its_authored_re_term_leaves_it`.
- `mush-industry` `e7c4.rs`: `pl9_open_orders_are_indexed_by_facility`.
- `tacsim` `port/facility_sale.rs`: two unit tests (the entries, the sentences under the floor).
- `tacsim` `domain_economy/yard_in9.rs`:
  - `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it`:
    - a captain who does not act for Stickney Yards is refused `NOT_FOR_ORG`, and a bid naming
      `as=` is refused with the bid's usage;
    - the Yards' officer lists the idle yard with `as=`, and the captain buys it;
    - the title, record (owner-held, kept public, authored cost model), stores and money move;
    - the captain sets the margin but cannot close the yard or set its cost model;
    - a lane bump re-terms the Ceres works and leaves the captain's yard;
    - a booking, then a margin raise: the order is cancelled, the booking fee is not taken,
      nothing stays held, and the owner is paid nothing (the bill waits in the escrow for
      `/collect`, since the customer has no hold docked);
    - a booking with the margin lowered runs.
  - `pl9_a_busy_or_unsellable_facility_is_refused_with_its_reason`: the Ceres works (storefront),
    the yard with an order booked (busy), the Stickney works with goods in its stores (laden), and
    the Stickney workshop (untitled: `No such asset.`; titled by staff: a workshop). Nothing is
    listed.

**Runs** (conclav.dev, the branch with main at `45b50f303` merged in):
- the workspace check;
- `mush-industry` and `mush-market`: 529 passed;
- `tacsim` lib (`port::`, `content_studio::`): 283 passed;
- `domain_economy`: 615 run, 614 passed. The one failure was an assertion since removed (the
  rig installs submissions below the commit announcement, see M25); both PL.9 stories then pass
  on their own;
- `mush-server` `commerce_composition` and `split_boundary`: 318 passed;
- `mush-bot`: 803 passed;
- clippy (`-D warnings`) and the CI lint job's 64 KiB stack-frame pass on `mush-industry`,
  `tacsim` and `mush-market`, rustdoc, and `mush-contract-tests` (200 passed: the census, the
  panic budget, the restriction lints, the help-page lint and the fiat budget).

- **Mutations**, each applied alone in a clone on conclav.dev, its tests run (the loop is
  `pl9_muts.py` in the group's notes folder):

  | # | Mutation | Caught by |
  |---|---|---|
  | M1 | the lane re-terms a held facility | `pl9_a_sold_facility_is_owner_held_and_its_authored_re_term_leaves_it` |
  | M2 | a held record's identity includes its owner and terms | the same |
  | M3 | a held record's identity drops capabilities and slots | the same |
  | M4 | a sold yard keeps its customers | the same |
  | M5 | a sale leaves the record `Authored` | the same |
  | M6 | an owner may set the cost model | `pl9_the_owner_sets_customers_and_margin_but_not_the_cost_model` |
  | M7 | anyone may set a facility's terms | the same |
  | M8 | a kept-public facility may go owner-only | `pl9_a_sold_facility_is_owner_held_and_its_authored_re_term_leaves_it` |
  | M9 | a risen fee does not cancel | `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it` |
  | M10 | a contract with no pin runs at the live fee | **survives.** Only a contract booked before this unit has no pin; none exist before launch. |
  | M11 | a busy facility lists | `pl9_a_busy_or_unsellable_facility_is_refused_with_its_reason` |
  | M12 | a storefront's facility lists | the same |
  | M13 | a laden works lists | the same |
  | M14 | the workshop room check removed | **survives.** The shipped workshop also holds `bay_refit`, which refuses it first; the room check stands for a workshop without it. |
  | M15 | the stores are not re-homed | `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it` |
  | M16 | the house buys into a sold works | **survives.** No story sells a works: the clerk's purchases keep the Stickney works' stores laden. |
  | M17 | the lane titles nothing | both stories |
  | M18 | the facility's state digest is not used | `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it` |
  | M19 | `as=` checks nothing | the same (`NOT_FOR_ORG`) |
  | M20 | a sold yard is not kept public | the same |
  | M21 | a bid takes `as=` | `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it` (the bid answers `No such catalogue entry.`, not its usage) |
  | M22 | a settled sale does not pass the facility | `pl9_a_captain_buys_the_stickney_yard_prices_it_and_keeps_it` |
  | M23 | an open work order does not make a facility busy | **survives.** A yard booking also leaves its contract open, which refuses first; the index itself is M24's. |
  | M24 | reconcile does not index orders by facility | `pl9_open_orders_are_indexed_by_facility` |
  | M25 | the commit observer does not index orders by facility | **survives.** The test rigs install submissions without the commit announcement (`install_prepared_action_announcing` is the server's), so no rig reaches the observer; the per-requester index (WS02-S4) has the same gap. |

  Twenty caught, five survive (M10, M14, M16, M23, M25), each with its reason.

**Divergences:**
- **Touches EX.4's crate: `+market/post as=<org>`.** The `+market/post` arm and its usage line, a
  `NOT_FOR_ORG` sentence beside it in `seller.rs`, and `+market/bid`'s refusal of `as=`. Nothing
  else in `mush-market` changed. If EX.4 (#641) touches the same lines, whichever PR merges second
  resolves.
- **`hull_sale.rs` is shared with EX.4 and EX.2b.** The facility code is in `facility_sale.rs`;
  `hull_sale.rs` gains three dispatches and `scope_of` made `pub(crate)`. Whoever merges later
  resolves.
- **The fee pin replaces doc 48b's policy-revision cancel** in the works arm. A re-term that does
  not raise the fee no longer cancels.
- **The idle check reads an index, not the family**, because the seam runs in the market's
  context. The index lags a missed announcement until the next sweep's reconcile, as the
  per-requester index does.
- **Files outside the unit's list:**
  - `mush-market` `commands.rs` and `seller.rs` (`as=`), and `mush-bot` `mine/minds.rs` (the
    sentence), on the lead's ruling;
  - `port/vendor.rs` (the facility entries' load) and `port/supply.rs` (the house-purchase guard);
  - `mush-industry` `record.rs` (`FacilityKeptPublic`) and `lib.rs` (exports);
  - `docs/help/exchange.md`;
  - the tests above.

**Not built:**
- **No NPC decides to sell a facility.** An operator org's facility sells when someone who acts
  for the org lists it.
- **Running-cost debt** is EX.9's. No facility owes any yet, so nothing is settled from the sale.
- **`@facility/owner`** (staff) can still set a record's owner apart from its title holder. The
  sale moves both together.
- **A sold facility's stores, if the seller did not own them,** stay where they are.

## EX.9

Not built.

## EX.10

Not built.
