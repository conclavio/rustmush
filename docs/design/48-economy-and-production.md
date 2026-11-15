# 48. Economy and production — one exchange for every trade, and a chain that closes, surveyed

Draft 0.1, 2026-11-15, docs only. Surveyed against `origin/main` at `b1b4c57f6` by four read-only
lanes (goods, services, the engine and money, production), with the lead spot-checking the claims
§0 leans on:
- the market's `Representation` and `Supply` enums;
- the staff gate on `@market/vendor create`;
- `MAX_GOODS`;
- the port vendors' zero buy rate;
- `accept_retrieval` as the one open join;
- the NPC acceptor whitelist;
- the unpaid crew hire;
- the contract caps.

Anchors are file names with line numbers; line numbers drift.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) §5 and
[doc 44](44-minds-wired-through-the-game.md) used. It sets out what exists, then asks the owner
questions (§6), each with a proposed default, an alternative and a reason. **Provisional** marks a
number to be tuned later, under the 2026-10-24 rule that economy defaults are taken and tuned, never
waited on. **(expensive to reverse)** marks an answer that would take a significant refactor of the
codebase to undo. Stored data and live boxes do not count: there are no continuous games, and every
step may wipe and redeploy (Q-38-9, Q-38-10). §7 sketches the units of work; they are cut only after
the rulings. Its companion, [doc 49](49-combat-content.md), surveys the combat content, whose rounds
are goods this document's chain should make.

## 0. In brief

1. **The owner's question.** Is there a single extensible system for NPC↔NPC, NPC↔player and
   player↔player trade in goods and services? **Half of one.** There are three layers:
   - **An open layer** extends without kernel edits. It is the journal, E4 contracts, inventory lots
     and the neutral market. The goods market itself was added this way: its lot action needed no
     kernel change.
   - **A closed layer** of about fourteen K3 commerce protocols carries mind licences, refits, cores,
     coverage and charters. Doc 46 ruled that this layer stays closed and is not to become one engine.
   - **About thirty venue fronts**, each with its own verbs, host record, clerk tick, price source and
     recognizer.

   The open layer could carry one exchange. Today it does not, for four reasons:
   - only a staff-registered *organisation* can sell on the market;
   - the market has no service arm;
   - `@contract` cannot name a lot;
   - only one service, retrieval, can be taken by a stranger.
2. **Goods.**
   - `mush-market` already is one engine for lot goods: one `Listing` with five supply arms, one
     `Quote`, E4 contracts, one trade tape and pluggable seams. All port trade goes through it.
   - Beside it are bespoke venues: hull buyouts, Tycho's resale, cores, scrap and the NPC-to-NPC
     buys. The hull-sale contract is written by hand three times.
   - There is **no player-to-player lot trade** without staff. Of thirteen part goods, none has an
     ask on the lane and one has a bid.
3. **Services.**
   - E4's `@contract` is a generic contract engine a player can use today: escrow, outcome tables,
     attestation or fact conditions, an adjudicator. But the issuer must name the counter-party, so
     it is not a board.
   - Every service venue is a bespoke front. Most stand on E4. Retrieval is the only job a stranger
     may join, and "retrieve my own hull" is the only job a player can post.
   - Labour is unpriced: NPC crew hire has "no wage charge".
4. **Money.**
   - There is one currency. The sources are the treasury's boot capitalisation of 24 lane orgs (about
     15.7 million cr plus 1.0 million in raises), passenger purses, and staff mints for the Houses.
   - Sinks exist only in mind commerce.
   - The facility daily cost of Q-38-12 is priced and never paid.
   - There is no money census. Money flows one way, from NPC institutions to players, which doc 18
     risk 9 warned of.
5. **Prices.** Every price is authored. The only prices that move are the reach premium and the
   emergency bids, and they move with orbital geometry, not with trade. Seven price sources serve
   goods and six serve services.
6. **The NPC economy.**
   - It runs as hand-coded arms on the port tick.
   - The NPC-to-NPC flows are narrow (Stickney Yards buys Kasei's alloys; the Trust buys a hull bill)
     and run outside the market as a direct post plus an ask shrink.
   - The budgeted NPC market maker is built and never used. `mush-npc` has no economy.
   - No NPC hauls between ports. The goods the NPC bidders buy appear to sit unconsumed.
7. **Production.**
   - Doc 39's chain now runs from ore to hulls on production paths, but the players' half of it is a
     market chain, not an ownership chain. NPC orgs own every mill, works and yard. There is one yard,
     and no player can own a facility.
   - Propellant, provisions, munitions, instrument boards and relay blanks are finite seeds that
     nothing makes. Volatiles have no industrial buyer.
   - A core is never billed in a hull and is not made from chain goods.
   - A House cannot buy a hull.
   - The goods catalogue has two slots left (`MAX_GOODS` 48).
8. **The prior rulings this must fit.**
   - Doc 08 D11: "no universal auction house or exchange rate". Markets are local providers over
     quotes, holds, contracts, title, custody and routes. Doc 08 §18 names E8 as "a generic
     quote/listing seam, local market providers", which was never built.
   - Doc 46 Q-46-3: no runtime-interpreted protocol engine.

   **The proposal (§5) is E8 as that seam:** one exchange protocol, many local venues, any principal
   selling or buying goods and services on it. The closed families stay as settlement kinds beneath
   it. The chain is closed by giving every seeded good a producer and paying the running costs.

## 1. Why this document exists

On 2026-11-15, with docs 46 and 47 nearly closed, the owner asked for a document focused on the
economy and production. The owner named the market in particular: a unified market for sales and
purchases of goods and services, NPC to NPC, NPC to player and player to player. In the owner's words:
"we have a lot of this but i'm not sure there's a single extensible system". The owner asked for a
second document on combat
content to follow it ([doc 49](49-combat-content.md)).

The economy was built in three waves:
- doc 08's E0–E7 foundation (2026-09);
- the trade run (doc 18, IC0–IC14);
- the production chain and its close-outs (docs 39, 41, 46 and 47).

Each wave added the venues it needed beside the ones before. This document is the first look across
all of them at once.

It is not a bug list. §4.3 names the defects the lanes found and leaves them to the fix thread.

## 2. What is in scope

**In:**
- every way a good or a service changes hands, for whatever parties;
- the money and price layers under them;
- the NPC economic actors;
- the production chain from extraction to hulls, and what consumes its output.

**Out:**
- the mind-commerce protocols' internals (doc 46);
- contraband and legality enforcement (a crime document; legality is recorded and unenforced);
- player housing (WoD-era E5 tenancies are staff-created);
- surface mining's own rules (doc 20);
- combat content (doc 49).

## 3. What exists

### 3.1 The foundations

**The journal** (`mush-economy`, E2–E4).
- Double-entry over `mush_core::Ledger`. Every instrument sums to zero. Arithmetic is checked, and
  `plan_hash` covers the plan (`crates/mush-economy/src/lib.rs:1-50`).
- E3 added assets, title, custody and vaults. E4 added holds, obligations and contracts.

**E4 contracts.**
- A contract goes draft → offer → accept with escrow → settle.
- Conditions include `TypedCondition::Fact`, served by `FactProvider`s.
- Deliverables are `TitleTransfer`, `DepositInto`, `Collateral`, `Obligation` and `Refund`
  (`contract.rs:176-215`, `:420-454`).
- Caps: 16 parties and 16 open contracts an issuer (`contract.rs:56`, `:92`).
- Registered fact namespaces: jobs, inventory, industry, campaign, sim, market, and TacSim's survey,
  passenger and retrieval (`crates/mush-composition/src/lib.rs:355-375`).

**Inventory** (`mush-inventory`) is the one quantity truth: endpoints, lots, and the
`ReservationBook`.

**The neutral market** (`mush-market`).
- Catalogues, vendors, listings, quotes, NPC market makers, and a public projection that takes no
  viewer.
- A `Listing` has one of five supply arms (`record.rs:505-571`): `Stock`, `Market`, `Procurement`,
  `Lot` and `Wanted`.
- A listed good has one of five representations (`record.rs:125-136`): `Instrument`, `Lot`, `Asset`,
  `Entitlement` and `Installed`. **There is no service representation.**
- Accepting a quote drafts an ordinary E4 contract with the buyer as issuer. The market "moves no
  money itself" (`trade.rs:1-30`).
- Seams: `VenueGate`, `CapacityProvider`, `ProceedsPolicy` and party caps.

**The closed K3 families.**
- Commerce that must land across several owners' state atomically uses kernel *terminal capacity*:
  "one closed, composition-checked protocol; modules cannot provide arbitrary target vectors, byte
  ceilings or verifier callbacks" (`crates/mush-core/src/action_state/terminal_capacity.rs:1-3`).
  Examples: a licence, the Roll's records, a market receipt and money in one completion.
- A new family of that kind is edited into nine closed lists by hand: the `StateFamilyRegistry`
  protocol fields (14 today), builders, the composition checks, a `terminal_capacity` submodule, the
  `EconomyRequest`/`EconomyOutput` enums (about 52 variants), the Roll's root enums, economy's
  `PlanGuard`, the engine-owned reservations, and the composition root.

**What can be added without editing the kernel.** A trade kind that needs no cross-owner terminal
capacity. That means money through E4 or `post`, plus a module-owned K3 action on families another
owner delegates. The goods market's lot action is the precedent: it calls inventory's `reserve`,
`settle_sale` and `release` on delegated families (`mush-market/src/lot.rs:1-25`). The kernel was not
touched; the composition root only names `"market"`.

### 3.2 Goods: every venue

| # | Venue (verbs or tick) | Parties | Price | Settlement | In play? |
|---|---|---|---|---|---|
| V1 | Market ask: `+market/quote`, `+market/accept` | any player buys from a registered **org** vendor | posted listing (spread if `Supply::Market`) | buyer-issued E4 contract with escrow; the lot leg is the market's lot action; an asset is `TitleTransfer` | yes |
| V2 | Sell into a bid: `+market/sell <listing>=<qty>` | player → a house's `Wanted` | authored bid, plus reach premium and emergency bids; Co-op share by `ProceedsPolicy` | house escrows; money gated on `inventory.transferred` | yes |
| V3 | Custody sale: `+market/sell <obj>=<vendor>` | player → vendor | catalogue baseline × the vendor's buy rate | `DepositInto`; no title moves | **dead**: port vendors register with buy rate 0 (`crates/tacsim/src/port/vendor.rs:620-632`) |
| V4 | Captain's orders: `+ship/order buy\|sell`, run by the port tick | as V1/V2 | as V1/V2 | the same code | yes |
| V5 | Port supply side: the stock clerk | NPC houses | lane JSON | seeded stock minted once; asks listed; bids posted and dripped; mills | yes (tick) |
| V6 | Works purchase (IN.5) | NPC → NPC: Stickney Yards buys Kasei's alloys | Kasei's ask | **direct post, then a separate move** (`port/supply.rs`) | yes (tick) |
| V7 | House pools (IN.10) | NPC → NPC: the Trust buys a hull bill | sellers' asks | direct posts, then moves (`port/pools.rs`) | yes (tick) |
| V8 | Programme buyout: `+coop/buyout` | the Trust → the pilot | frozen buyout | E4 `TitleTransfer`, a hand copy of V1's asset arm | yes |
| V9 | Tycho's resale: `+salvage/forsale`, `+salvage/buy` | Tycho → a player | design reference price | E4 `TitleTransfer`, another copy | yes |
| V10 | Scrap and bale buy-back: `+bay/scrap`, `+bay/return` | title holder → workshop operator | the scrap ceiling | matter minted to the operator, credit posted after; on failure "staff pay by hand" | yes |
| V11 | Core purchase: `+core/buy` (CO.6) | the fitting shop → a player | class constants (`core_integration/subject.rs:28-34`) | closed K3 family; title born to the buyer | yes, on a launch |
| V12 | Stored-core sale: `+core/sell` (CO.13) | player → the shop | 500‰ of finished | closed CoreSale family | only where the launch deployment names `buy_back_permille` (§4.3) |
| V13 | Works / yard orders: `+works`, `+yard` | player → facility operator | industry cost model | escrow vault, booking fee, `@wo` hold | yes (a service whose output is goods) |
| V14 | Mind licences: `+minds/buy`, `/resell <lic>=<player> price=`, `/charter`; the procurement tick | player ↔ player; House agent ← author | one deployment-wide price, or a price the parties name | closed K3 families | with the launch |
| V15 | Generic contract: `@contract/*` | any principal ↔ any principal it names | negotiated | E4, atomic settlement | yes, **titled objects only; no lots** |
| V16 | One-sided moves: `give`, `@econ/pay`, `@asset/transfer`, `@inv/move` | — | — | no exchange (a gift, or between one's own endpoints) | yes |

**Shared across V1–V16.**
- Money is always the journal, in one of two shapes:
  - an E4 contract (V1–V3, V8, V9, V15);
  - a direct E3 `post` (V6, V7, V10, Tycho's sweep).
- Custody is inventory endpoints and lots. Title is `AssetOp::TransferTitle`, behind one wall for
  managed titles (`managed_title::require_unmanaged_mutations`).

**Bespoke.**
- The asset-sale contract is written three times: `purchase_spec`, `buyout_spec` and
  `house_sale::sale_spec`. Buyout's own doc calls itself "the market's Asset arm".
- There are four listing types and seven price sources.
- Each venue has its own replay guard; V9 has none.

**Extension cost today.**
- A new **lot good** is data: a `goods.json` row and lane rows, then V1, V2 and V4 work unchanged.
- A new **titled good** needs a staff-stocked listing or its own verb, as V8, V9 and V11 each got.
- A new **managed-title good** is a new closed family, CO.13's size.
- A **player-to-player part sale** has one route today: staff create a vendor for the player's org.
  `@contract` cannot name a lot.

### 3.3 Services, contracts and licences: every venue

| # | Venue (verbs) | Provider → buyer | Price | Settlement | Non-performance | In play? |
|---|---|---|---|---|---|---|
| A | `@contract/*` | any character or org → the parties named; NPCs only if whitelisted | negotiated | escrow → outcome table; attest, fact, title, deposit or time conditions; adjudicator | Refund, Forfeit or Dispute; `/agree` or staff `/resolve` | player |
| B | Retrieval job: `+salvage/retrieve\|jobs\|accept` | any character (Tycho as fallback) → the title holder | the poster's, never under Tycho's floor | escrow; `TitleHeld` + fact `tacsim.retrieval.delivered` | 28 IC days unaccepted → Tycho; deadline refunds | player + tick |
| C | Tycho performing B | NPC salvage org → poster | as B | as B | as B | tick |
| D | Salvage claim: `+salvage/claim\|contest\|…` | the Registry's record | none | E4, no escrow; the award moves title | a contest vacates it | player |
| E | Bay repair: `+bay/order` | the workshop's operator org → holder, lessee, salvor | tariff × operator multiplier | fee escrow; the operator accepts on completion | 30-day `Before`, refund | player + tick |
| F | Core build and fit: `+core/buy\|fit` | the shop agent → player | constants; fit fee capped | closed families: hold, capture, release | retried, then given up | player + tick |
| G | Works / yard | facility operator → player or org | cost model | booking fee + `@wo` hold | cancel forfeits the booking fee and the elapsed units | player + tick |
| H | Rig leases: `+tv/lease` | Occator → player | port-def fee, deposit, term | `offer_lease` + fee + engine-owned deposit | re-debit or forfeit; damage as debt; repossession adds retrieval | player + tick |
| I | Co-op charter: `+coop/enrol\|buyout\|release` | the Trust → player | stake, buyout, margin share | `offer_lease` + stake | revoke or dormancy repossesses | player |
| J | Housing: `@property`, `@tenancy` | landlord → tenant | staff-set | lease + rent obligation | grace, then delinquent | staff creates |
| K | Licences: `+licence/buy`, `+claim/*`, `+enduser/apply` | the Registry → player | lane fee; a 2,500 cr constant | public E4 fee contract; one bespoke transaction for end-user | lapse read at the gate | player |
| L | Mind licences | author → player or House | one price; resale `price=` | closed families | expiry, refund, renewal grace | player |
| M | House procurement | listing → House agent | within a monthly budget | as L | refund | tick |
| N | Passage: `+passage/offer\|accept\|board` | player captain → NPC traveller | negotiated in authored bounds | E4; two fact outcomes | refunds | player + NPC |
| O | Survey: `+survey/accept` | player → Occator | four fixed 500 cr offers (a Rust `const`) | E4 on `tacsim.survey.logged` | reward returned | player |
| P | NPC crew hire: `+sortie/hire` | NPC (staff-written offer) → player | **none** ("These offers have no wage charge", `detachment/crew/commands.rs:106`) | a crew-contract action, no economy | unseat | player |
| Q | Proxy seats: `+seat/offer` | OOC proxy → operation lead | none | a consent grant, not a contract | 7-day TTL | player |
| R | Operations: `@op/*` | attacker → | bond | E4 hold | pauses when unfunded | player |
| S | Relay carriage: `+relay/*` | relay operator → player | content tariff | per-message hold; a recurring service obligation | grace, delinquency | player (production registration not traced) |
| T | Hull coverage | insurer → owner | product terms | closed `managed_coverage` family | — | **tests only** |

**Shared.**
- One journal; one contract engine (A, B, D, E, H, I, J, K, N, O, buyout, house sale, customs).
- Holds, obligations and the `FactProvider` seam.
- **One NPC acceptor whitelist**, `PortPartyCaps` (`crates/tacsim/src/port/market.rs:338-388`):
  passengers, the Trust, the Registry, the workshop and recovery operators, and vendor orgs.

**Bespoke.**
- Every venue has its own verb grammar, host record, clerk tick and windows. The windows live in the
  host record because the deadline is hashed into the consent.
- Each kind has its own constructor and recognizer (`offer_lease`, `file_salvage_claim`,
  `post_retrieval`/`retrieval_contract_hull`).
- **`accept_retrieval` (`contract_live.rs:6613`) is the only path in the tree where a stranger joins
  a standing offer.**
- Services take prices from six places: port-def JSON, Rust constants, the launch deployment,
  content JSON, NPC attributes and the poster.
- E8's "generic quote/listing seam … delivery contracts" was never built, and doc 08 §10.3 calls the
  service market "E8 (unplanned)".

**Extension cost today.**
- **Player → player "escort for hire" with no code:**
  1. The employer drafts a contract, names the escort, pays, sets an attest condition and offers it.
  2. The escort accepts.
  3. The employer attests to settle.

  There is no board; nothing in the game evidences the escort; 16 open contracts an issuer.
- **The first-class version copies retrieval** (RS.5 is the size reference). It needs:
  - a spec and a recognizer;
  - a generalised standing-offer join;
  - a delivery `FactProvider`;
  - a host record and clerk tick;
  - verbs, help and bot needles.
- **An NPC → player service** needs a port-def org, a `PortPartyCaps` entry, a pricing rule, and an
  arm on the `house_arm.rs` pattern for physical work.

### 3.4 Money

**Currency.** One settlement currency, `tacsim.credit`, plus one licence entitlement
(`crates/tacsim/src/economy.rs:19-36`). Ten commodity instruments are registered from
`content/resources.json`, but goods trade as lots, and no trade path was found that uses those
instruments.

**Sources.** Only System accounts may go negative, so a mint is a faucet's debit.
- **The port treasury.** `system:tacsim.treasury` capitalises each lane org at boot in one-shot
  tranches of at most 800,000 cr (`port/apply.rs`, `port/def.rs:86`). At lane v34 that is 24 orgs at
  5 ports, about 15.7 million cr base plus 1.0 million in authored raises. For example, the Exchange
  gets 4.0 million and the Co-op 3.1 million.
- **Passenger NPCs** get finite purses from the same treasury.
- **Staff `@econ/mint`.** The bring-up uses it to fund House treasuries and filing fees
  (`deploy/instance/launch-bringup.py:801-908`).
- **The market's vendor-fund faucet drip** exists and is never called.

**Transfers mistaken for sources.** The Co-op stake (25,000 cr from the Trust's capital), scrap
payouts (paid by the operator), the reach premium ("Nothing is minted", `port/price.rs:27-29`) and
the campaign bond.

**Sinks.**
- **Money** is destroyed only in mind commerce: the filing fee, the Registry levy and the venue cut,
  paid to System accounts.
- Every other fee is a transfer to an org.
- The Q-38-12 facility daily cost **only prices the fee; it is never debited**
  (`crates/mush-industry/src/cost.rs:9-15`).
- **Matter** has real sinks: departure burns spend propellant and provisions (`port/burn.rs`).

**No money census.** There is a matter census in kilograms (`@industry/census`). The nearest money
view is `@tacsim/port status`'s per-institution accounts. Doc 18 risk 9 (`18-trade-run-vertical-slice.md`,
§ risks): "Money pools in seller orgs while the alloy buyer drains; a recapitalization path …
is needed".

### 3.5 Prices

**Authored tables.**
- `content/market/goods.json`: 46 reference prices.
- Each lane vendor's asks and bids, per port.
- `market/workshop_tariff.json`, with a 500–2,000‰ operator multiplier.
- `content/relays/service-v1.json`.
- The facility cost blocks: fee = work units × slot-hour × (1 + margin).
- Lane scalars: licence 2,500, release 1,500, rig lease 1,400 plus a 5,000 deposit, the Co-op's
  stake and share.
- Launch constants: filing 100, House licence 400.
- Procurement's budget: 2,000 cr an IC month, 400‰ cap.
- Core prices: `CoreClass::finished_minor`.
- Survey: a Rust `const`.

**What moves.**
- The reach premium raises a demand-side bid when the passage from the nearest supplier is longer
  than its synodic median (`port/price.rs:12-29`).
- Emergency `Wanted` bids appear for life-support goods when reach crosses a ratio
  (`port/def.rs:1018-1038`). `price.rs:30-31`'s comment that they are not built is stale.
- **Both move with orbital geometry, not with trade.**
- The daily bid "refresh" refills intake *quantity*.
- The projection's `last_trade`, volume, band and depth are read-only folds of settled trades.
- **No price responds to trade.** Kasei's 300 → 280 in doc 18 was an authored retune.

### 3.6 The NPC economy

**Institutions.** Each lane org is a real `mush-org` org with one credit account. Five ports trade:
Ceres, Stickney, Luna, Deimos and Titan. Vendors have:
- seeded finite stock, minted once and never re-issued;
- authored asks and bids;
- mills: the Exchange mills metal ore into metals, Occator mills ice ore into volatiles, Kasei
  refines metals into alloys.

**Agents.** Every port-tick act is one closed call "as principal" (`port_tick.rs:2046-2093`).
- Occator: yard dispatcher with drones, rig lessor, mill.
- Tycho Mutual: retrieval, the ownerless-wreck sweep, resale, the fallback retrieval clerk.
- Freeport Works: the Ceres works operator.
- The Co-op Trust: keeps a floor of two free Drays and orders hulls at the yard.
- Stickney Yards: buys alloys below its floor.
- The three Houses: a procurement agent each, buying mind licences on a monthly budget. A House needs
  three titled hulls for its first licence. Treasuries are funded by staff mint.

`mush-npc` has no economy. Every NPC economic behaviour is a hand-coded arm, not a generic agent.

**NPC-to-NPC trade exists narrowly and off the board.** V6 and V7 are each a direct economy post plus
`mush_market::shrink_ask` on the seller's ask: no quote, no contract. A player hull's offline orders,
by contrast, use the real quote and accept.

**Not in use.** `MarketSource`, the budgeted NPC market maker with volume and loss budgets, is built;
no `Supply::Market` appears anywhere in TacSim. No NPC hauls between ports. The bidders that list no
asks (Habitat Board, Syrtis Bonded, Selk Townships) appear to keep what they buy, with no consumer
found.

### 3.7 Production: the chain today

```
mine (+tv mine on the ground; +mine laser and drones in space; Occator's ice drones)
  metal_ore ─sell 70─▶ Exchange mill 5:2 ─▶ metals (ask 200) ─sell 280─▶ Kasei refine 5:4 ─▶ structural_alloys (ask 450)
  ice_ore   ─sell 35─▶ Occator mill 5:2 ─▶ volatiles ──▶ NPC bids only (no recipe, no propellant)
  carbon_ore / strategic_ore ─from the player's own hold─▶ +works order   (or sell: a dead-end pool)
works (Ceres, Stickney): alloys + carbon ─▶ industrial_parts; alloys/fittings/carbon/strategic/boards ─▶ 13 part goods
yard (Stickney only): alloys + fittings ─▶ hull kit; kit + one part lot a slot ─commission─▶ titled hull + register row
consumers: repairs (bale), rearm (munitions), passages (propellant, provisions), scrap (capped at a fifth), Tycho restorations
cores: a monthly die quota ─+core/buy─▶ governed core ─+core/fit─▶ hull; +core/sell back at 500‰
```

**Extraction.**
- Four ores (`goods.json`).
- The ground drill takes 1,800 IC s a cycle (×2 on rock) and lifts 1/2/3 t by grade.
- Space: `+mine/laser` plus up to four collector drones, licence-gated.
- Players and bots mine; Occator's house drones mine ice into its own pool.

**Processing.**
- Three mill rows: two 5:2 mills and one 5:4 refine, 20 t a batch, 40 t of input a day. They run on
  the stock clerk's arm and sit outside the fee model.
- Facilities belong to NPC orgs: two workshops, a recovery facility, two works, a refinery and the
  **only yard**.
- The refinery facility is "one slot never booked" (`port/plant.rs:8-9`).
- The E7 engine runs player orders: `+works` / `+yard` with quote, order, deliver, collect and cancel.
  The bill is drawn **only from the requester's docked hold**.
- Fourteen recipes ship (`industry/recipes-v2.json`), live whenever the game is on.
- Opening a facility is staff-only (`mush-industry/src/commands.rs:916-921`). **No player route
  builds or owns a facility.**

**Parts.**
- Thirteen part goods, one indivisible lot a unit, with provenance.
- `Replace{lot: None}` no longer conjures a part: it draws the lowest-grade clean store lot, or
  refuses `NO_PART`.
- One part bid ships: the Yards' light plant at 4,000 × 4.
- Stripped and scrapped parts are never relisted.

**Hulls.**
- Built from produced goods end to end at Stickney. The bill is derived from the design (IN.8).
  `hulls-v2.json` holds 17 rows, each pinned to `design_bill`.
- Proven by `shard_6/yard_in9.rs`, IN.11's chain story and CO.10b's yard-built wrecker.
- **The core is never billed** (`DesignBill.core` read, billed nowhere; IN.9's owed F-18(b)).
- Still conjured: the Co-op's 8 Drays, Occator's lease pools, Tycho's two boot hulls, CO.9's staff
  House seed (three Drays a House at bring-up), and `@unit/create`.
- **A House cannot buy a hull**, because a yard order needs a docked hold to deliver from.

**Consumers.**
- Repairs draw alloys and fittings from the bale; mend and unjam are labour only.
- Rearm draws munitions goods.
- Burns draw propellant and provisions.
- Construction consumes the bill.
- Tycho's restorations buy at Stickney.
- `+coop/refit` re-binds a Dray ↔ Dray-P for no materials and no fee.

**Who does each step:**

| Step | Player | NPC on a tick |
|---|---|---|
| Mine | yes (ground, space) | Occator's drones (ice) |
| Mill / refine | **no** | the Exchange, Occator, Kasei |
| Works | yes, public slots | Stickney's alloy buy; the Trust's hauler plant |
| Parts | yes | the works arm commissions |
| Yard | yes (Stickney) | the Trust's Dray; Tycho and House seeds without a build |
| Repair / scrap | yes | Tycho |
| Core | yes (`+core`) | the counter tick as the shop |

**Goods with no producer.** Each of these is a finite seed:
- propellant (five rows of 5,000 t), burned on every passage;
- provisions;
- munitions (eight rows);
- instrument boards (2,080 t at Luna; an input to every sight, pod and laser mount);
- Titan's carbon feedstock;
- Freeport Works' carbon and strategic ore;
- relay blanks (at most 8 pairs, ever).

**Producers with no industrial buyer:**
- volatiles;
- carbon feedstock;
- carbon and strategic ore sold into the Exchange's bids;
- twelve of thirteen part goods;
- seventeen hull kits.

**Adding to the chain.**
- **A good:** a `goods.json` row, but `MAX_GOODS` is 48 (`definitions/goods.rs:59`) with **two slots
  left**, and the goods digest and count pins move.
- **A recipe:** a pack entry, re-signed, with the version and `INDUSTRY_RECIPES_VERSION` bumped.
- **A mill, bid or ask:** a lane row, which means a lane version bump, a strip step and new pins.

## 4. The owner's question, answered

### 4.1 Is there one extensible system?

Not one; three layers, and the bottom one is the right foundation.

**The open layer.** The journal, E4 contracts, inventory lots and the neutral market are general. A
new kind of trade can be added to them without editing the kernel; the goods market's lot action is
the proof. They already carry most of the game's trade, but with four limits:
1. **Only an org vendor can sell on the market**, and staff create vendors. A player cannot list
   anything.
2. **The market has no service arm.** Services go through E4 directly, each with its own front.
3. **`@contract` cannot name a lot**, so the general player-to-player path cannot sell goods, only
   titled objects.
4. **Only retrieval can be joined by a stranger.** Every other contract names its counter-party, so
   no job board exists.

**The closed layer.** Doc 46 ruled these stay closed and are not to become one engine. That stands
here; nothing in this proposal touches them except to let the exchange route to them.

**The venues.** These are about thirty fronts, each with its own verbs, host record, clerk tick,
windows, recognizer and price source. Some of them are the game's texture: a retrieval job is not a
passage. Others are copies: three hull-sale contract builders, four listing types, thirteen price
sources.

### 4.2 What the prior rulings already say

- **Doc 08 D11** (2026-09): "no universal auction house or exchange rate. Market providers layer
  quotes, holds, contracts, title, custody, and routes; physical delivery preserves observation and
  interception."
- **Doc 08 §14.2**: markets decide reach, listings vs NPC quotes, liquidity and spread, legality,
  fees, delivery, price visibility, and off-screen supply.
- **Doc 08 §18's E8**: "Add a generic quote/listing seam, local market providers, fees, delivery
  contracts, and bounded price history … mobility/courier services with real routes and
  interceptable delivery". E8 was never planned.
- **Doc 46 Q-46-3**: no runtime-interpreted protocol engine. Doc 46 §5.5 names when to look again: when
  a new protocol of an existing shape is wanted.

**So the owner's unified market is E8, and E8 fits both rulings** if it is one *protocol* with many
*local venues*, built on the open layer, with the closed families as settlement kinds beneath it. It
would break D11 only as a global book, and Q-46-3 only as a runtime engine. §5 proposes neither.

### 4.3 Defects and stale notes found (for the fix thread, not this plan)

- **V3 is dead in TacSim.** Every port vendor registers with buy rate 0, so a custody sale always
  answers "not buying".
- **Pay-and-get-nothing, latent.** With no delivery providers composed, a staff-made `Instrument`,
  `Entitlement` or `Installed` listing passes the pre-check, settles, and escalates "buyer paid and
  holds nothing" (`mush-market/src/module.rs:1179-1251`).
- **V6, V7 and V10 are not atomic.** Each is a post and a separate move, recovered from durable state.
  V10 falls back to "staff pay by hand".
- **CO.13's sale needs `buy_back_permille` in the launch deployment.** It composes only when the
  deployment names it (`launch/deployment.rs:96`; doc 47's cut note), and neither
  `deploy/instance/launch-bringup.py` nor the deploy templates name it. Without it, `+core/sell` is
  not composed on a launched server. MW.3's go-live should set it.
- **The industry `bay_refit` (UD.4b) has no production submitter.** Only the hardware flow rig drives
  it. Its three defects stand (47-cut-notes: a destroyed part returns worn, the lot lands in the
  operator's stores, no provenance).
- **Tycho ignores retrieval jobs bound away from its own port.**
- **Stale:**
  - `port/price.rs:30-31` says emergency bids are not built; they are.
  - `contract_live.rs:5103` names a `+run/post` caller that does not exist.

## 5. The shape proposed

### 5.1 The exchange

One protocol, many local venues.

- **A seller is any principal.** Today a `Vendor` is an org registered by staff. It generalises to a
  seller: a principal, its account, its endpoint at the venue, and the venue. A player sells from
  their own hold, room or vault; an org sells through an officer; an NPC agent sells as its org.
  Staff vendor creation stays, for seeding.
- **A listing offers goods or a service:**
  - **goods** as a lot or a titled asset;
  - managed titles (cores, licences) are listed on the exchange but **settle in their closed
    families**;
  - **a service** (§5.2) gets a new representation arm beside the five.
- **Both sides post.** An ask is a listing; a bid is a `Wanted` listing. Any principal may post
  either, and any principal the terms admit may take it.
- **Settlement is one acceptance path.**
  - Accepting a quote drafts the E4 contract: the lot leg by the market's lot action, an asset by
    `TitleTransfer`, a service by §5.2's typed terms.
  - The three hull-sale builders become one.
  - A managed title routes to its family.
- **Venues are local**, per D11: a port's market, a station, a site. Goods are delivered where they
  are. A remote purchase is a purchase plus a haul job (§5.2), so the goods physically travel and can
  be intercepted.
- **A private offer** is a listing with a named buyer. It generalises `+minds/resell <lic>=<player>
  price=` to every kind and stands in for a trade window.
- **The NPCs trade on the board.** V6, V7 and V10 become quotes and acceptances, so payment and goods
  are atomic. The acceptor whitelist becomes a role row per org in the port def.

### 5.2 Services on the exchange

- **A closed list of service kinds, each with typed terms and a performance fact.** A kind is added
  in code, as retrieval was: a spec, a recognizer and a `FactProvider`. Free text is never a kind.
  - Those already built move onto the exchange's listing: retrieval, passage, survey, repair at a
    workshop, works and yard orders, leases.
  - New in the first wave:
    - **haul**: these goods, from here to there, by a deadline. The fact comes from inventory's
      transfer facts, if they can name a destination (unverified);
    - **escort**: stay with this hull through this passage or field (from `sim.station.served`);
    - **crew hire**: a seat for a wage.
- **Open jobs.** A `Wanted` service listing any qualified principal may take. `accept_retrieval`'s
  standing-offer join is generalised to every kind, and a venue's job board is the projection of its
  open service listings.
- **Non-performance** is what E4 already offers:
  - the deadline refunds;
  - an adjudicator settles a dispute;
  - above a size, the provider posts a bond as collateral;
  - no reputation system in this document.

### 5.3 Prices and the NPC side

- **An NPC house prices by a rule.** Its ask and bid move with its stock against an authored target,
  within a band and a step limit per IC day. The reach premium sits on top. The venue keeps a bounded
  price history (E8). Players price their own listings.
- **NPC consumption.** Each port's population consumes provisions, volatiles and propellant at an
  authored daily rate, from the storefronts of the bidders who buy them. Bids then refresh because
  stock fell, not because a timer lapsed.
- **NPC haulers, later.** A hauling house runs real hulls between ports when a price gap pays.
  Delivery is physical and can be intercepted. This is what makes the five ports one economy instead
  of five.
- **`MarketSource`**, the budgeted NPC maker, is used where a venue needs depth it cannot seed.

### 5.4 Money

- **A money census**, beside the matter census: sources and sinks by category per IC day.
- **Running costs are paid.** Q-38-12's facility daily cost is debited from its operator, and wages
  are paid. A house that cannot pay throttles: it stops bidding and raises its asks. It never mints.
- **One measured inflow after boot.** The boot capitalisation stays a one-time act. The one recurring
  source is a named faucet with an authored daily budget: off-map buyers, export bids at the five
  ports. Sinks are the levies, the berth fees and running costs. The census shows whether the two
  balance.

### 5.5 The chain closes

- Every seeded good gets a producer:
  - propellant from volatiles;
  - provisions at a habitat farm;
  - instrument boards at a Luna fab;
  - munitions at the works (doc 49);
  - relay blanks.

  The seeds become starting stock.
- Players process their own ore: the mill and refine rows become works recipes with public slots.
- A facility is a titled asset that can change hands on the exchange; building a new one is a later
  document.
- A second yard at Ceres builds light hulls.
- A governed core's build consumes chain goods beside its die quota, and the yard bills the core
  where a design calls for one.
- A buyer may fund a works or yard bill straight into the escrow at the venue's asks, so a House can
  buy hulls and nobody has to haul a bill to Stickney first.
- Workshops list the parts they strip and bid for the parts they use.
- The goods catalogue grows (`MAX_GOODS`), once.

### 5.6 What the exchange is not

- **Not a global book.** No exchange rate; no order matching across venues.
- **Not a runtime protocol engine.** The closed families stay closed (Q-46-3).
- **Not a kernel change for open kinds.** The lot action's path is the model.
- **Not free-text services.** Every service kind is typed, with a fact.
- **Not negotiation with NPCs.** Prices are posted. An LLM-voiced NPC may choose among legal offers
  and never posts one (D12).

## 6. Owner survey

### 6.1 The exchange

**Q-48-1 — One exchange, local venues.**
- **Proposed default:** build doc 08's E8 as one exchange protocol on the open layer (journal, E4,
  inventory, market), with local venues. D11 stands: no global book, no exchange rate. The closed
  families stay closed beneath it. **(expensive to reverse)**
- **Alternative:** keep the venue fronts and add only player-to-player lot sales.
- **Why:** the market is already the shared engine for goods. What is missing is breadth (who may
  sell, what may be offered), not a new engine. Every service and venue added after this would
  otherwise be another front with its own price source.

**Q-48-2 — Who may sell.**
- **Proposed default:** any principal with an endpoint at the venue: a player, an org through its
  officer, an NPC agent as its org. `Vendor` generalises to a seller. Staff vendor creation stays for
  seeding.
- **Alternative:** players sell only through a player-org shop that staff register.
- **Why:** the owner named player-to-player trade, and today it needs staff. A personal seller reuses
  the lot legs as they are.

**Q-48-3 — What a listing offers.**
- **Proposed default:** goods (lots, titled assets) and services, through one new representation arm
  for a service. Managed titles (cores, licences) list on the exchange and settle in their own
  families.
- **Alternative:** goods only; services stay venue fronts.
- **Why:** one listing, quote and board is what makes it one system. Routing managed titles to their
  families keeps doc 46's wall.

**Q-48-4 — Posted prices, not a book.**
- **Proposed default:** asks and bids are posted and taken by quote. Crossing bids and asks are not
  matched automatically: a seller sells into a bid.
- **Alternative:** a continuous book per venue for commodities.
- **Why:** a text game with a few dozen traders a port has no liquidity for a book. D11 and the
  tedium guardrails (doc 08 §14.3) both argue for posted offers.

**Q-48-5 — Private offers.**
- **Proposed default:** a listing may name its buyer, for every kind. It replaces trade windows.
  `give` stays a gift.
- **Alternative:** a two-sided trade window for barter.
- **Why:** `+minds/resell <lic>=<player> price=` is already the pattern. Barter of goods for goods is
  two private offers.

**Q-48-6 — Delivery.**
- **Proposed default:** local only: the buyer takes delivery at the venue. A remote purchase is a
  purchase plus a haul job (Q-48-11).
- **Alternative:** remote delivery for a fee, with the goods moving instantly.
- **Why:** D11's "physical delivery preserves observation and interception", and a haul job is play
  for someone.

**Q-48-7 — One asset sale.**
- **Proposed default:** `purchase_spec`, `buyout_spec` and `house_sale::sale_spec` become one builder.
  Hull resale between players goes through it. A core's resale goes through CO.13's family, opened to
  a player buyer.
- **Alternative:** keep the three and add a fourth for player hull sales.
- **Why:** they are hand copies of one contract shape, and a fourth is what the alternative costs.

**Q-48-8 — Fees.**
- **Proposed default (provisional):**
  - a listing fee of 1% (at least 10 cr) and a sale cut of 3%, both paid to the venue's operator org;
  - a 1% levy to a system sink.
- **Alternative:** no fees for player listings.
- **Why:** a listing with no cost is spam on a board read in a terminal, and the levy is one of the
  sinks §5.4 needs.

**Q-48-9 — NPC trade on the board.**
- **Proposed default:** Stickney Yards' alloy buy, the Trust's bill buys and the scrap buy-back go
  through quote and accept. Payment and goods settle in one contract. The `PortPartyCaps` whitelist
  becomes a role row per org in the port def.
- **Alternative:** leave the NPC flows as posts, and fix only their atomicity.
- **Why:** an NPC that trades on the board leaves a trade on the tape and a price in the history,
  which §5.3's rule reads. Off-board trades are invisible to both.

**Q-48-10 — No haggling with NPCs.**
- **Proposed default:** NPC prices are posted. An LLM-voiced NPC may choose among legal offers (D12)
  and never posts one. Passage's authored fare bounds stay.
- **Alternative:** LLM negotiation within authored bounds for every NPC sale.
- **Why:** D12 already rules that models propose and code posts. Haggling at every counter is tedium
  for the player and spend for the LLM budget.

### 6.2 Services

**Q-48-11 — The service kinds.**
- **Proposed default:** a closed list, each kind with typed terms, a performance fact and a refund
  rule.
  - The built ones move onto the exchange: retrieval, passage, survey, workshop repair, works and
    yard orders, leases.
  - The first new wave is haul, escort and crew hire.
- **Alternative:** a free-text service contract settled by attestation or an adjudicator.
- **Why:** a typed kind lets the game evidence the work. A free-text one makes every job a dispute,
  and `@contract` already offers that for anyone who wants it.

**Q-48-12 — Open jobs.**
- **Proposed default:** a `Wanted` service listing is a job any qualified principal may take.
  `accept_retrieval`'s standing-offer join is generalised to every kind. A venue's job board is the
  projection of its open service listings. NPC houses post jobs too, for example haul jobs from the
  consumers of Q-48-19.
- **Alternative:** jobs offered only to named parties, as `@contract` does today.
- **Why:** a job board is the shape of "hire someone to do this"; retrieval proved the join.

**Q-48-13 — Non-performance.**
- **Proposed default:**
  - the deadline refunds;
  - E4's adjudicator settles a dispute;
  - above 10,000 cr (provisional) the provider posts a bond of 20% as collateral, forfeit to the buyer
    if the fact never arrives;
  - no reputation system in this document.
- **Alternative:** no bonds; refunds only.
- **Why:** a refund protects the buyer's money but not their time. A bond makes a large job's
  provider care. Reputation is an epistemic question for later.

**Q-48-14 — Labour is priced.**
- **Proposed default:**
  - NPC crew hire carries a daily wage as an obligation (provisional: 150 cr an IC day for a rating,
    300 for an officer).
  - A player may list themselves for hire as crew.
  - The technician duty doc 38 left to E8 is a crew-hire kind.
- **Alternative:** crew stay free.
- **Why:** free crew make a hull's running cost zero, and wages are one of the sinks of §5.4. Doc 38
  §5.2 left the technician to E8, and this is E8.

**Q-48-15 — Fees nobody pays today.**
- **Proposed default:** a berth fee per IC day at a port, paid to the port authority org
  (provisional: 50 cr a day for a hull under 100 t). No customs duty yet; customs stays a fine.
- **Alternative:** no berth fees.
- **Why:** a hull parked at a port for weeks costs nothing today. A small fee gives docking a cost and
  the authority an income.

**Q-48-16 — Player-run services that need a facility.**
- **Proposed default:** Q-41-7 stands (a carrier's bay away from port pays nobody), and player-run
  workshops wait for facilities as assets (Q-48-26). Players provide repair labour only as a crew
  service.
- **Alternative:** a player carrier may charge for bay work away from port.
- **Why:** Q-41-7 was ruled and marked expensive to reverse; this survey does not reopen it.

### 6.3 Prices and money

**Q-48-17 — The money census.**
- **Proposed default:** a staff `@econ/census` beside `@industry/census`. It shows sources, sinks and
  holdings by category per IC day, and the System faucets' net mint.
- **Alternative:** read the journal by hand.
- **Why:** nobody can tune an economy they cannot see, and every other question here is a tune.

**Q-48-18 — The NPC price rule.**
- **Proposed default (provisional):**
  - an NPC house's ask and bid move with its stock against an authored target, by up to 5% per IC day,
    within 50% to 200% of the authored price;
  - the reach premium stays on top;
  - the venue keeps 28 IC days of price history, shown by `+market/history`.
- **Alternative:** static prices, retuned by hand.
- **Why:** a price that never moves cannot tell a trader where to go. A bounded, slow rule cannot be
  pumped by one player.

**Q-48-19 — NPC consumption.**
- **Proposed default:** each port's population consumes provisions, volatiles and propellant at an
  authored daily rate (provisional, sized so a port's seed lasts about 60 IC days with no trade). It
  draws from the storefronts of the bidders who buy them (Habitat Board, Syrtis Bonded, Selk
  Townships).
- **Alternative:** no consumption; bids refresh by timer.
- **Why:** without a sink for goods, every bidder fills and stops. Consumption is what makes demand
  real and the haul job worth posting.

**Q-48-20 — NPC haulers.**
- **Proposed default:** after the exchange and consumption: one NPC hauling house running real hulls
  between ports when a price gap pays for the trip, physically interceptable.
- **Alternative:** inter-port movement stays the players' alone.
- **Why:** with consumption on and no NPC haulers, a port with no players nearby starves. A raider
  also needs something to raid.

**Q-48-21 — Running costs are paid.**
- **Proposed default:**
  - Q-38-12's facility daily cost and staff wages are debited daily from the operator, the wage share
    to a system sink.
  - A house earns them back in fees and sales.
  - A house that cannot pay throttles (no new bids, asks up 10% a day) and never mints.
- **Alternative:** keep running costs as a price only.
- **Why:** an operator that never pays its costs accumulates money forever. That is doc 18 risk 9's
  pooling, seen from the other side.

**Q-48-22 — The one inflow after boot.**
- **Proposed default:** the boot capitalisation stays a one-time act. The one recurring source is an
  "off-map buyer" faucet: export bids at the five ports for alloys, parts and volatiles, funded from a
  named System account with an authored daily budget (provisional: 50,000 cr an IC day across the
  lane). The census measures it against the sinks.
- **Alternative:** no recurring source; money only circulates, and staff top up by attributed mint.
- **Why:** with sinks on (levies, fees, running costs) and no inflow, the economy deflates to zero.
  One named, budgeted faucet keeps the inflow measured and tunable.

### 6.4 Production

**Q-48-23 — Every seeded good gets a producer.**
- **Proposed default (provisional bills):**
  - propellant from volatiles at Occator (5:4);
  - provisions at a habitat farm at Luna and Titan from volatiles and carbon feedstock;
  - instrument boards at a Luna fab from strategic ore and metals;
  - munitions at the works from alloys, carbon and volatiles (doc 49 Q-49-4);
  - relay blanks by recipe at the relay foundry.

  The seeds become starting stock.
- **Alternative:** keep finite seeds with a daily restock.
- **Why:** a restock from nowhere is a mint of matter. A producer gives each good a price that follows
  its inputs, and gives volatiles and carbon feedstock their first industrial buyers.

**Q-48-24 — Players process their own ore.**
- **Proposed default:** the two mills and the refine become works recipes with public slots at the
  same ratios, at the cost-model fee. The NPC mill rows stay as the houses' own processes.
- **Alternative:** the chain stays a market chain: sell ore, buy product.
- **Why:** owning a step of the chain is what makes a production game. The fee model already prices
  a slot.

**Q-48-25 — A second yard.**
- **Proposed default:** a yard at Ceres for light hulls (Dray, Dray-P, prospector, escort, tender,
  wrecker); Stickney keeps the heavy ones.
- **Alternative:** one yard.
- **Why:** the starter port cannot build a starter hull, and every House hull would have to come from
  Stickney.

**Q-48-26 — Facilities as assets.**
- **Proposed default:** a facility is a titled asset that can be sold on the exchange. Its owner sets
  its terms (`@facility/terms`, opened to the owner). Building a new facility is a later document.
- **Alternative:** facilities stay NPC-owned.
- **Why:** player ownership of production is the step after processing one's own ore. Selling an
  existing facility needs no construction rules. **(expensive to reverse)** only in that facility
  ownership touches the industry crate's authority model.

**Q-48-27 — Cores from the chain.**
- **Proposed default:**
  - a governed core's build consumes chain goods (instrument boards and strategic ore, provisional
    bill), beside the monthly die quota;
  - the yard bills the core where a design calls for one (F-18(b));
  - `CoreClass` prices become content.
- **Alternative:** cores stay quota-only and priced by constants.
- **Why:** the core is the most valuable part of a hull and the one part outside the chain.

**Q-48-28 — Buying straight into an escrow.**
- **Proposed default:** a buyer may fund a works or yard bill at the venue's asks straight into the
  order's escrow, without a docked hold. A House can then buy hulls. CO.9's staff seed stays for the
  bring-up.
- **Alternative:** keep the docked-hold rule.
- **Why:** the hold rule makes every order a haul to Stickney, and a House has no hold.

**Q-48-29 — A parts market.**
- **Proposed default:**
  - workshops list the parts they strip or scrap;
  - each part good gets an NPC bid at its reference × 600‰ (provisional) at the workshops that fit it;
  - hull kits stay untradeable (a massless intermediate).
- **Alternative:** parts trade only player to player.
- **Why:** twelve of thirteen parts have no buyer, so strip and scrap feed nothing.

**Q-48-30 — The catalogue cap.**
- **Proposed default:** `MAX_GOODS` from 48 to 128, in one change with its digest and count re-pins,
  before any new good lands.
- **Alternative:** raise it as each good needs it.
- **Why:** this document and doc 49 add at least a dozen goods. One re-pin is cheaper than six.

**Q-48-31 — Census coverage.**
- **Proposed default:** the matter census adds volatiles, propellant, provisions and munitions.
- **Alternative:** the chain's goods only, as now.
- **Why:** these are the goods Q-48-19 and Q-48-23 make and burn, and today the census cannot see
  them.

### 6.5 Order

**Q-48-32 — Order of work.**
- **Proposed default:**
  1. The money census and the catalogue cap first.
  2. Then the exchange core: any seller, one listing and quote for lots and assets, private offers,
     one asset sale, the NPC flows on the board.
  3. Then services: open jobs, haul, escort, crew wages.
  4. Then prices, consumption, running costs and the export faucet, which balance each other and land
     together.
  5. NPC haulers last.

  Production's closures run beside the exchange from the start, because they touch the lane, the
  recipes and the port tick's works arms, not the market.
- **Alternative:** production first, then the exchange.
- **Why:** the census has to exist before anything is tuned. The exchange is the owner's question.
  Consumption without running costs and an inflow would either drain or flood the NPC accounts.

## 7. Units of work (sketch, cut after the rulings)

Two chains, one writer per file set, as docs 44 to 47 ran. **EX** (the exchange and money) and **PL**
(production) are unused as unit prefixes.

| Unit | What | Gate |
|---|---|---|
| EX.0 | `@econ/census`; the matter census's new goods | Q-48-17, Q-48-31 |
| EX.1 | The seller: any principal lists from its own endpoint; private offers | Q-48-2, Q-48-5 |
| EX.2 | One asset-sale builder; player hull resale; core resale through CO.13 | EX.1, Q-48-7 |
| EX.3 | NPC flows on the board, atomic; acceptor roles as data | EX.1, Q-48-9 |
| EX.4 | Listing fee, sale cut, levy; berth fee | EX.1, Q-48-8, Q-48-15 |
| EX.5 | The service arm; the generalised open join; retrieval, passage and survey listed | EX.1, Q-48-3, Q-48-12 |
| EX.6 | Haul and escort kinds with their facts | EX.5, Q-48-11 |
| EX.7 | Crew wages; player crew listings | EX.5, Q-48-14 |
| EX.8 | The NPC price rule and price history | EX.3, Q-48-18 |
| EX.9 | NPC consumption, running costs paid, the export faucet (one unit: they balance) | EX.0, EX.8, Q-48-19, Q-48-21, Q-48-22 |
| EX.10 | The NPC hauling house | EX.6, EX.9, Q-48-20 |
| PL.0 | `MAX_GOODS` raised, one re-pin | Q-48-30 |
| PL.1 | Propellant from volatiles | PL.0, Q-48-23 |
| PL.2 | Provisions, instrument boards and relay blanks get producers | PL.0, Q-48-23 |
| PL.3 | Munitions recipes (with doc 49's CC.5) | PL.0, Q-48-23, Q-49-4 |
| PL.4 | Mills and the refine as works recipes | Q-48-24 |
| PL.5 | Buying into an escrow; a House buys a hull | Q-48-28 |
| PL.6 | The Ceres yard | Q-48-25 |
| PL.7 | Cores from the chain; the core billed in a hull | PL.0, Q-48-27 |
| PL.8 | The parts market | EX.3, Q-48-29 |
| PL.9 | Facilities as titled assets | EX.2, Q-48-26 |

## 8. What this survey did not check

- Whether dev1 and dev2 run with TacSim on.
- Whether `PortVenueGate` admits a venue that is not a port.
- Whether the shipped sky ever gives a non-zero reach premium.
- That the NPC bidders consume nothing (no consumer was found).
- That relay carriage's `service::register` is reached in production.
- Whether inventory's transfer facts can name a destination (Q-48-11's haul).
- What an NPC naming the content studio's `hire` interface does.
- Whether a yard-built escort or line tank carries a core component.
