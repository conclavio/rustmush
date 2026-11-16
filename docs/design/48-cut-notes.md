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
