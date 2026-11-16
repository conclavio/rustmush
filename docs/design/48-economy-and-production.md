# 48. Economy and production — one exchange for every trade, and a chain that closes, surveyed

Draft 0.7, 2026-11-15, docs only. Surveyed against `origin/main` at `b1b4c57f6` by four read-only
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

Anchors are file names with line numbers; line numbers drift. Draft 0.1, the same day, was the
survey.

**Draft 0.2, the same day, records the owner's rulings.** Every question from Q-48-1 to Q-48-32 is
ruled. Three are amended:
- Q-48-15: the berth fee is 5 cr an IC day.
- Q-48-18: prices step 5% an IC week.
- Q-48-30: `MAX_GOODS` rises to 256.

Every other default is accepted as written.

The reason for the first two amendments is the live clock. At 24:1 an IC day is one real hour, so a
rate per IC day is felt per real hour. Reading the other accepted figures with that in mind raised
four follow-up questions, Q-48-33 to Q-48-36 (§6.6).

**Draft 0.3, the same day, records that the owner accepted all four follow-ups at their defaults.**
Every question in Draft 0.3 is ruled.

**Draft 0.4, the same day, folds in an outside review** of the PR at `8ddf6ca9b` (Astra), triaged in
[doc 48a](48a-outside-review-triage-2026-11-15.md). All fourteen of its findings held; none was
refuted.
- Where a ruled default was wrong in its facts, it is corrected in place and marked
  **Correction (outside review)**. These are §5.1, §5.2, Q-48-5, Q-48-9, Q-48-11, Q-48-19, Q-48-23 and
  Q-48-26.
- Where a ruling needs a new choice, it gets a follow-up question: Q-48-37 to Q-48-46 (§6.7), open.
  The ruling stands until the follow-up is answered.

**Draft 0.5, the same day, records that the owner accepted all ten follow-ups at their defaults.**
Every question in this survey, Q-48-1 to Q-48-46, is ruled.

**Draft 0.6, the same day, folds in a second outside review** of the PR at `8ddf6ca9b` (Gemini). It
is triaged in [doc 48b](48b-second-review-triage-2026-11-15.md), with five verification lanes against
`b1b4c57f6`.
- Most of its findings held; about ten were refuted, and some were already addressed by Draft 0.4.
- Facts the rulings got wrong are corrected in place, marked **Correction (second review)**:
  §3.2, §4.1, §5.1, §5.2, §5.6, Q-48-2, -3, -15, -21, -27, -28, -29, -30, -35, -38, -42, -43, -44
  and -46.
- §4.3 lists the live defects the review found.
- Choices the findings need are the follow-ups Q-48-47 to Q-48-61 (§6.8).

**Draft 0.7, the same day, records that the owner accepted all fifteen follow-ups at their
defaults.** Every question in this survey, Q-48-1 to Q-48-61, is ruled. Among the owner's own calls:
- yard-built combat hulls keep a governed Class II core outside the monthly die quota (Q-48-48);
- facility staff stay at 150 cr an IC day (Q-48-60);
- a worked-out deposit re-rolls after 28 IC days (Q-48-61).

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
- Each venue has its own replay guard. **Correction (second review, doc 48b W-48-8):** V9's guard is
  `house_sale::pending`, which matches an open contract by its *name* alone, any issuer, any state
  (`port/house_sale.rs:76-85`). A player's free `@contract/draft` with that name hides a Tycho hull
  from `+salvage/forsale` and refuses `/buy` (§4.3).

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
**Correction (second review, doc 48b W-48-1):** that last clause is too strong. Three rulings also change closed
families, each a closed-family edit sized as CO.13 was, not an "opening":
- Q-48-7: core resale to a player. The core sale admits no player buyer, as a kernel image rule
  (`mush-core/src/core_sale.rs:222-227`) and a closed two-line plan with no holds
  (`mush-economy/src/registry_filing/core_sale.rs:118-131`, `:386-389`).
- Q-48-3: a managed licence on an open listing.
- Q-48-27: the core billed through the yard.

They are Q-48-47 and Q-48-48.

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

**Found by the second review (doc 48b), live in the tree, for the fix thread:**
- **The works arm charges the live fee, not the quoted one.** `works.rs:1799-1823` calls
  `money::place` and submits, skipping `@wo/submit`'s policy-revision check (`commands.rs:1294-1302`).
  `money.rs:220-229` reads the fee at that moment. A staff re-term today, or an owner's margin change
  under Q-48-37, reprices orders already quoted, and an unfunded one is cancelled with its booking fee
  forfeit.
- **A hold-backed Consume skips the capacity check.** Its entries carry no plan (`mush-market/src/lot.rs:486`)
  and its prepare does not check the hold (`:873-903`), unlike a Draw (`:1126-1150`). Two accepted
  exact-lot quotes can overfill a governed hold. The comment at `:415-416` is stale.
- **The house-sale guard matches by name** (above, §3.2).
- **The retrieval path checks neither contract cap.** `post_retrieval` and `accept_retrieval` check
  neither the 16 open contracts an issuer nor the 100 live contracts a party
  (`contract_live.rs:6555-6556`, `:6603-6667`).
- **The licence resale offer's limit is 86,400 real seconds** (`mush-market/src/mind_paid_resale/offer.rs:24`).
  That is 24 IC days, under the owner's 28-IC-day minimum for a wait on a player.
- **CO.13's integrator account starts at 0 cr** (`deploy/instance/launch-bringup.py:918-929`). The
  buy-back has no money to pay with until it is funded.

## 5. The shape proposed

### 5.1 The exchange

One protocol, many local venues.

- **Correction (second review, doc 48b W-48-2, G-1):** a player seller does not "reuse the lot legs as they are".
  Three things are missing:
  - **Acceptance.** The market accepts for a seller by an NPC party cap fixed at composition
    (`trade.rs:827-841`), with the seller hard-coded as an org (`api.rs:920`, `:1429`).
  - **Pooled goods.** Only a port's own counter may sell pooled goods into a hull; anyone may sell an
    exact whole lot (`trade.rs:979-985`).
  - **An endpoint.** A player can make no vault or org (both admin-only), and the venue's endpoint is
    a charter hold's helm (`port/market.rs:239-247`).

  Q-48-49 and Q-48-50 settle these. Until then, "their own hold, room or vault" below means a
  governed hold the player may trade.
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
  - **Correction (outside review, doc 48a A-14):** hiding the listing is not enough. The quote and
    the acceptance each refuse every principal but the named buyer.
- **The NPCs trade on the board.** V6, V7 and V10 become quotes and acceptances. The acceptor
  whitelist becomes a role row per org in the port def.
  - **Correction (outside review, doc 48a A-3):** that does not make them one indivisible
    transaction. The market's lot path is escrow, then a staged matter leg, then a fact, then the
    economy sweep's settlement:
    - it keeps a 600 s settlement margin for a Draw (`DRAW_SETTLE_MARGIN_SECS`);
    - a hold-backed leg that cannot be staged is a logged risk (`trade.rs:874-886`).

    What the path guarantees is that goods never move without the money in escrow, and that a missed
    Draw refunds. Each kind admitted to the exchange states its own guarantee and has a test for it:
    lot (Draw and hold-backed), titled asset, service, and managed title.

### 5.2 Services on the exchange

- **A closed list of service kinds, each with typed terms and a performance fact.** A kind is added
  in code, as retrieval was: a spec, a recognizer and a `FactProvider`. Free text is never a kind.
  - Those already built move onto the exchange's listing: retrieval, passage, survey, repair at a
    workshop, works and yard orders, leases.
  - New in the first wave:
    - **haul**: these goods, from here to there, by a deadline. The fact comes from inventory's
      transfer facts. **Correction (second review, doc 48b W-48-3b):** the provider answers only whether a marker
      exists (`mush-inventory/src/facts.rs:61-81`). But a marker stores from, to, commodity and
      amount (`pool.rs:776-784`), and a derived marker id (`pool.rs:160-187`) can be named when the
      job is posted. So the haul's fact is a derived marker written by a new haul delivery verb;
      no player verb moves goods between principals today;
    - **escort**: stay with this hull through this passage or field. **Correction (outside review,
      doc 48a A-4):** `sim.station.served` cannot evidence this. It is satisfied by any seat
      interval, a zero-length one included, and it names no protected hull and no journey. An escort
      needs its own fact (Q-48-46);
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
  within a band and a step limit per IC week (Q-48-18 as ruled). The reach premium sits on top. The
  venue keeps a bounded
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
- **Not a kernel change for open kinds.** The lot action's path is the model. Closed-family changes
  are named in §7 and sized as CO.13 was (§4.1's correction).
- **Not free-text services.** Every service kind is typed, with a fact.
- **Not negotiation with NPCs.** Prices are posted. An LLM-voiced NPC may choose among legal offers
  and never posts one (D12).

## 6. Owner survey

**Ruled 2026-11-15.** Three questions are amended:
- Q-48-15: the berth fee drops to 5 cr an IC day.
- Q-48-18: the price step is 5% an IC week, not an IC day.
- Q-48-30: `MAX_GOODS` goes to 256, not 128.

Every other default, Q-48-1 to Q-48-32, is accepted as written. Each amended question carries its
decision first, with the proposed text kept under it for the record. §6.6 holds the four follow-up
questions the amendments raised. The owner accepted all four at their defaults the same day
(Draft 0.3). §6.7 holds Q-48-37 to Q-48-46, raised by the outside review (Draft 0.4); the owner accepted all
ten at their defaults (Draft 0.5). §6.8 holds Q-48-47 to Q-48-61, raised by the second review
(Draft 0.6); the owner accepted all fifteen at their defaults (Draft 0.7).

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

**Correction (second review, doc 48b G-1(3b), W-48-2):** "reuses the lot legs as they are" is wrong for a player
seller. Neither the acceptance seam nor the pooled-goods gate admits one (§5.1's correction).
Q-48-49 and Q-48-50.

**Q-48-3 — What a listing offers.**
- **Proposed default:** goods (lots, titled assets) and services, through one new representation arm
  for a service. Managed titles (cores, licences) list on the exchange and settle in their own
  families.
- **Alternative:** goods only; services stay venue fronts.
- **Why:** one listing, quote and board is what makes it one system. Routing managed titles to their
  families keeps doc 46's wall.

**Correction (second review, doc 48b W-48-1b):** a managed title cannot settle an open ask.
- The licence resale takes a named buyer at offer, by the seller's own act, for 24 real hours, with
  the licence's frozen cuts (`mush-economy/src/managed_paid_resale/subject.rs:235-236`;
  `mush-market/src/mind_paid_resale/offer.rs:22-52`).
- The core sale has no player buyer at all.

So a managed title appears only as a private offer, and pays its family's cuts, not Q-48-8's fees.
An open listing of one is a notice, which the seller answers with a private offer.

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
- **Correction (outside review, doc 48a A-3):** two private offers are two independent sales. A
  party can accept one leg and decline the other, so this is not a barter. Whether to build one
  conditional settlement for both legs is Q-48-43.

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
- **Correction (outside review, doc 48a A-3):** "settle in one contract" is not "atomic". The board's
  lot path settles by escrow, then a staged leg, then a fact, then a sweep (§5.1). The unit states and
  tests the guarantee it gives these flows: goods never move without escrowed money, and a missed leg
  refunds. Today's V10 fallback ("staff pay by hand") goes.

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
- **Correction (outside review, doc 48a A-4, A-9):**
  - The escort's fact is not `sim.station.served`; it is Q-48-46.
  - A haul carries cargo liability as well as a fee; that is Q-48-42.

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
- **Decision (owner ruling, 2026-11-15):** keep the berth fee, so the mechanism is in the system, but
  at **5 cr an IC day** to start (provisional; it can rise later). The owner's reasoning: at 24:1 an
  IC day is one real hour, so the proposed 50 cr meant a player who logged off at a port came back to
  1,200 cr owed a real day, and 8,400 cr a real week — "pretty harsh". At 5 cr that is 120 cr a real
  day and 840 cr a real week. No customs duty, as proposed.
  **Correction (second review, doc 48b W-48-8):** the fee is per hull of any mass. The "under 100 t" band in the
  text below was wrong, since every ship is 240 t or more. Who pays at a home berth is Q-48-55.
- **Proposed default (as written):** a berth fee per IC day at a port, paid to the port authority org
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
- **Decision (owner ruling, 2026-11-15):** the rule as proposed, but the step is **5% an IC week**,
  not an IC day. An IC day is a real hour, and 5% an hour moves prices too fast. The band (50% to
  200%), the reach premium and the price history stand as proposed. The history's window became
  24 IC weeks under Q-48-35.
- **Proposed default (provisional, as written):**
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
- **Correction (outside review, doc 48a A-2):** the goods named do not match what the three bidders
  hold, and the bidders are never refunded.
  - The Lunar Habitat Board bids volatiles and carbon feedstock.
  - Syrtis Bonded and Selk Townships bid structural alloys and instrument boards, so consuming
    provisions, volatiles and propellant leaves their main stores untouched.
  - Their treasuries (1.0M, 3.1M and 1.35M cr) are finite, with no recurring income.

  Q-48-40 names each consumer's goods and its funding.

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

**Correction (second review, doc 48b W-48-5a):** "facility daily cost and staff wages" counts the payroll twice. The
daily cost already includes it (`mush-industry/src/cost.rs:111-120`). The debit is the daily cost, as
Q-48-38 sets it, and all of it goes to a system sink.

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
- **Correction (outside review, doc 48a A-5):** carbon feedstock is itself a finite seed. Mining
  yields `carbon_ore`, a distinct good, so a farm fed on feedstock stops when Titan's 600 t run out.
  The default adds one process: **carbon ore to carbon feedstock** (provisional 5:4), as a works
  recipe with public slots and as a house mill row at Titan. This also gives carbon ore its first
  buyer outside the Exchange's dead-end bid.
- **Correction (outside review, doc 48a A-11):** a recipe does not run itself. Each new producer lands
  with an operating arm on the port tick, in the shape of Stickney Yards' alloy buy: it buys its
  inputs, books its slot, collects the output and lists it. Q-48-41 gates consumption on that.

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
- **Correction (outside review, doc 48a A-1):** `@facility/terms` is not only commercial terms.
  Besides the customer policy, it sets the whole cost model: capital, lifetime, upkeep, staff, wage
  and margin (`mush-industry/src/commands.rs:1042-1052`). The bounds admit a running cost of 0.01 cr
  a day (`cost.rs:67-79`). Opening it to owners, as written, would let an owner cut the costs
  Q-48-21 charges. What an owner may set is Q-48-37; selling a facility mid-order is Q-48-45.

**Q-48-27 — Cores from the chain.**
- **Proposed default:**
  - a governed core's build consumes chain goods (instrument boards and strategic ore, provisional
    bill), beside the monthly die quota;
  - the yard bills the core where a design calls for one (F-18(b));
  - `CoreClass` prices become content.
- **Alternative:** cores stay quota-only and priced by constants.
- **Why:** the core is the most valuable part of a hull and the one part outside the chain.

**Correction (second review, doc 48b W-48-1c):** the facts behind this default differ.
- Core integration takes exactly one input lot, the die: quantity 1, root lineage
  (`mush-core/src/action_state/terminal_capacity/core_integration.rs:235-254`;
  `mush-inventory/src/core_input/records.rs:217-221`).
- The designs that carry a core (the escort and the line tanks) are commissioned with their stock
  Class II core free: no die, no title, no bill line (`port/commission.rs:12`; `port/bill.rs:98-101`).
  That core is governed in `cores.json`, but outside the quota.
- `+core/fit` fits only Drays and prospectors (`hardware/counter.rs:486-493`).

So billing chain goods for a core is a closed-family question: Q-48-48.

**Q-48-28 — Buying straight into an escrow.**
- **Proposed default:** a buyer may fund a works or yard bill at the venue's asks straight into the
  order's escrow, without a docked hold. A House can then buy hulls. CO.9's staff seed stays for the
  bring-up.
- **Alternative:** keep the docked-hold rule.
- **Why:** the hold rule makes every order a haul to Stickney, and a House has no hold.

**Correction (second review, doc 48b W-48-4):** "at the venue's asks" cannot fill a bill as written.
- No vendor asks any of the thirteen part goods.
- Stickney has no ask for instrument boards or strategic ore.
- A yard order takes only Intact, unfitted, unheld lots (`port/works.rs:607-618`).

But V7 already buys a whole hull bill into escrow with no hold, taking parts from the yard
operator's stores at catalogue price (`port/pools.rs:976-999`). So the default is: generalise V7's
seller and price to any House. A House holds about 2,100 cr against a Dray of about 156,400 cr, so it
needs an income first (Q-48-55). Until then, CO.9's seed stays its hull source.

**Q-48-29 — A parts market.**
- **Proposed default:**
  - workshops list the parts they strip or scrap;
  - each part good gets an NPC bid at its reference × 600‰ (provisional) at the workshops that fit it;
  - hull kits stay untradeable (a massless intermediate).
- **Alternative:** parts trade only player to player.
- **Why:** twelve of thirteen parts have no buyer, so strip and scrap feed nothing.

**Correction (second review, doc 48b G-9):** a flat 600‰ bid overpays a damaged part and underpays an intact one.
Scrap credits 750‰ intact and 400‰ worn (`port/bay_work/scrap.rs:7-11`, `:32-36`), and the bid
path checks neither heat nor condition. The bid takes unfitted, unheld, cool, Intact parts only; a
damaged part goes to scrap. (Provenance is not lost: parts are indivisible and lineage is kept,
`mush-inventory/src/pool.rs:1940-1962`.)

**Q-48-30 — The catalogue cap.**
- **Decision (owner ruling, 2026-11-15):** `MAX_GOODS` from 48 to **256**, in one change with its
  re-pins, before any new good lands. The extra room costs little and saves doing this again soon.
- **Proposed default (as written):** `MAX_GOODS` from 48 to 128, in one change with its digest and
  count re-pins,
  before any new good lands.
- **Alternative:** raise it as each good needs it.
- **Why:** this document and doc 49 add at least a dozen goods. One re-pin is cheaper than six.

**Correction (second review, doc 48b W-48-8):** the bump moves no pin. `goods_digest` hashes `goods.json`
(`definitions/goods.rs:624-629`), and `MAX_GOODS` is read only at the bound (`:282`). Every goods row
added later still re-pins `ic2_the_goods_table_is_pinned`. The ruling stands for the room it gives,
not for fewer re-pins. PL.0 must also raise the per-endpoint pool cap, or the unload plan breaks
(§7, PL.0; doc 48b G-5).

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

### 6.6 Raised after the rulings: the other rates, read at 24:1

The reason behind Q-48-15's and Q-48-18's amendments applies to four other figures the owner
accepted. The live game runs at 24:1, so an IC day is one real hour and an IC week about seven real
hours. Each figure is raised here with a default in the same spirit.

**Ruled 2026-11-15: all four at their defaults** ("Yes, at defaults"). The figures that now stand:
- Crew wages: 10 cr an IC day for a rating, 20 for an officer.
- The throttle: 10% an IC week.
- Price history: 24 IC weeks.
- Consumption and the export faucet: sized by EX.9 from EX.0's census on a measured run at 24:1, to
  within 1% of the lane's capital per real day.

Each replaces the figure Q-48-14, Q-48-21, Q-48-18 or Q-48-19 and Q-48-22 accepted.

**Q-48-33 — Crew wages at 24:1.**
- **The problem:** Q-48-14 accepted 150 cr an IC day for a rating and 300 for an officer. At 24:1
  that is 3,600 and 7,200 cr a real day for each hire, owed while the employer is offline. It is the
  berth fee's problem, larger.
- **Proposed default:** 10 cr an IC day for a rating and 20 for an officer (provisional). That is
  1,680 and 3,360 cr a real week.
- **Alternative:** keep the accepted figures, and stop the wage while a hire is off duty at a port.
- **Why:** the ruled berth fee's reasoning; tune upward once play shows what a crew earns.

**Q-48-34 — The throttle's step.**
- **The problem:** Q-48-21 accepted "asks up 10% a day" for a house that cannot pay its running
  costs. That is 10% a real hour, so a house in deficit doubles its asks in about seven real hours:
  faster than a player can react, and faster than the ruled price rule moves anything else.
- **Proposed default:** 10% an IC week, on the same clock as Q-48-18's ruled step.
- **Alternative:** keep 10% an IC day.
- **Why:** one clock for every price movement.

**Q-48-35 — The price history's window.**
- **The problem:** Q-48-18 accepted 28 IC days of history. That is about 28 real hours, which spans
  only four of the ruled weekly steps.
- **Proposed default:** 24 IC weeks (168 IC days, about one real week), bounded as before. A trader
  coming back after a week away then sees the whole week.
- **Alternative:** keep 28 IC days.
- **Why:** the window should cover the time a player is likely to be away.

**Correction (second review, doc 48b G-5):** the trade tape cannot hold 24 IC weeks. It is a global ring of 1,024 rows,
32 a pair (`mush-market/src/record.rs:57`; `store.rs:61`). The price history is its own bounded
series, one row per venue, good and period (EX.8), not the tape.

**Q-48-36 — Consumption and the inflow are sized from the census.**
- **The problem:** two accepted figures were not sized against the 24:1 clock.
  - Q-48-19's consumption ("a port's seed lasts about 60 IC days with no trade") is about 2.5 real
    days.
  - Q-48-22's export faucet, 50,000 cr an IC day, is 1.2 million cr a real day: about 7% of the
    lane's boot capital every real day.

  Both are economy-scale rates and rightly kept in IC time, since the mills, works and running costs
  they balance run in IC time.
- **Proposed default:** EX.9 sets both from EX.0's census, on a measured run at 24:1. It aims to keep
  the lane's net money change within 1% of its capital per real day (provisional). The figures above
  are starting points only.
- **Alternative:** keep the accepted figures and tune after launch.
- **Why:** these two numbers decide whether the economy floods or drains, so they should be measured,
  not guessed.

### 6.7 Raised by the outside review (doc 48a)

Each question answers a finding that held.

**Ruled 2026-11-15: all ten at their defaults (Draft 0.5).** Each one's proposed text below is now
the decision.

**Q-48-37 — What a facility's owner may set** (A-1).
- **Proposed default:**
  - The owner sets the customer policy (public or owner-only) and the margin, which is the price.
    The margin keeps today's bounds.
  - The cost model (capital, lifetime, upkeep, staff and wage) is authored with the facility and
    stays staff-only.
  - An owner lowers its running cost only by mothballing (Q-48-38), which also stops the facility
    working.
- **Alternative:** owners set staffing and upkeep too, with a matching loss of capacity (fewer slots)
  and wear (a breakdown chance).
- **Why:** a running cost is a sink only if the payer cannot set it. The alternative is a richer
  management game, and it needs capacity and wear rules that do not exist yet.

**Q-48-38 — Running costs, insolvency and recovery** (A-10).
- **The problem:** Q-48-21 debits a facility's whole daily cost every IC day. Stickney Yards'
  workshop, works and yard cost about 6,183 cr an IC day against a treasury of 100,000 cr. With no
  orders, that lasts about 16 real hours.
- **Proposed default:**
  - **The cost follows the work.** A slot-hour worked costs its share of the daily cost, which the
    fee already recovers with its margin. An idle slot-hour costs a standing share (provisional 10%,
    sized by EX.9 from the census). With nothing booked, Stickney's standing cost is then about 620 cr
    an IC day: about a week of real time on its treasury.
  - **A house that cannot pay mothballs the facility.** It takes no new order. An open order runs to
    completion if its escrow covers it; otherwise it is cancelled, and the customer's booking fee and
    goods are returned. Customer goods in its stores stay the customer's and can be collected.
  - **Unpaid cost is a debt** owed to the system account; nothing is minted. The facility reopens
    when the house can pay 7 IC days ahead (provisional).
- **Alternative:** keep the whole daily cost, and let houses fail.
- **Why:** an NPC house that dies in an evening because nobody played is not a market. Tying the cost
  to the work keeps the sink in proportion to the activity it prices.

**Correction (second review, doc 48b W-48-5e,f):** two parts of the default need narrowing.
- "Which the fee already recovers with its margin" holds only for a bookable facility with a margin.
  Kasei's refinery has a margin of 0 and is never booked (`port/plant.rs:8-9`), and workshop fees
  come from the tariff (`port/bay_work/command.rs:307-316`).
- The way back from mothballing is not closed. A mothballed house takes no orders and posts no bids,
  so it never gets 7 IC days ahead.

Both are Q-48-59.

**Q-48-39 — The throttle against the price rule** (A-10).
- **The problem:** Q-48-18 limits an NPC price to 5% an IC week, and Q-48-34 raises a house's asks
  10% an IC week when it cannot pay. Neither says which wins.
- **Proposed default:** the throttle is the one exception to the step limit. While a house is
  mothballed or in debt, its asks rise 10% an IC week inside the same 50–200% band, and it posts no
  new bids. Once it is solvent, the normal rule resumes from the throttled price at 5% an IC week.
- **Alternative:** the throttle stays inside the 5% limit.
- **Why:** a house in debt has to recover faster than a healthy one drifts, and the band still
  bounds it.

**Q-48-40 — Who consumes what, and how it is funded** (A-2).
- **Proposed default:**
  - **Each consumer consumes what it buys.** The Lunar Habitat Board consumes volatiles, carbon
    feedstock and provisions (provisions are added to its bids). Syrtis Bonded and Selk Townships
    consume structural alloys and instrument boards, as off-map construction demand.
  - **The export faucet (Q-48-22) pays the consumers, not the sellers.** The consumers are the
    off-map buyers' agents, so the faucet's money reaches players and houses only through their bids.
    Each consumer's share of the daily budget is authored, and EX.9 sizes it so the consumer's
    treasury holds steady at its rate of consumption.
- **Alternative:** the faucet pays exporters directly, and staff top up the consumers.
- **Why:** money that enters through the buyers who consume keeps demand funded, and every credit in
  circulation was earned by a sale.

**Q-48-41 — Supply before consumption** (A-11).
- **Proposed default:**
  - A good's consumption switches on only once its producer runs on the port tick with an operating
    arm (Q-48-23's correction), and a story shows one full cycle: inputs bought, slot booked, output
    collected and listed.
  - EX.9 is gated on PL.1 and PL.2.
  - Doc 49's CC.7, the space rearm, waits for PL.3 to produce the space munitions.
  - At a port with no producer of a good in reach, that good's consumption waits for NPC haulers
    (EX.10).
- **Alternative:** switch consumption on with EX.9 and watch the census.
- **Why:** consumption without supply is a countdown to an empty port.

**Q-48-42 — Cargo liability for a haul** (A-9).
- **Proposed default:**
  - **The cargo travels as a held lot.** The shipper keeps ownership. The lot rides in the hauler's
    hold, reserved to the job, so it cannot be sold, consumed or moved except by delivery, or by being
    taken in a fight.
  - **The shipper declares the cargo's value**, and the job shows it.
  - **The hauler's bond is sized on the declared value, not the fee**: provisional 25%, posted at
    acceptance. A job whose bond the hauler cannot escrow is refused to that hauler.
  - **Non-delivery** covers cargo lost to interception or not delivered by the deadline. The bond
    goes to the shipper and the fee is refunded. Loss beyond the bond is the shipper's risk.
  - **Cargo insurance** is a later coverage product (the coverage family, today composed only in
    tests).
- **Alternative:** the hauler posts the whole declared value.
- **Why:**
  - Holding the lot to the job removes theft by the hauler.
  - The bond covers the hauler's failure.
  - A full-value bond would make every haul a capital test that only big players pass.

**Correction (second review, doc 48b W-48-3c):** "the cargo travels as a held lot" works, because a hold does withhold
from the custodian (`mush-inventory/src/pool.rs:381-393`). The default needs three things stated:
- The hold outlives the job. Today's TTL is 7 IC days unless `reserve_for` is used.
- At the deadline, held cargo still aboard stays held, and returns to the shipper at the hauler's
  next port.
- Pool goods carry no title, so "the shipper keeps ownership" means the job's reservation.

The cap on declared value, and collusive "interception", are Q-48-57.

**Q-48-43 — Barter** (A-3).
- **Proposed default:** not in this document. Two private offers are two independent sales, and the
  board says so. A both-or-neither barter would be one contract carrying both legs as conditions of
  each other; it is deferred until play asks for it.
- **Alternative:** build the conditional settlement now.
- **Why:** the lot legs settle by facts after escrow. A two-leg contract doubles that path's failure
  cases, for a trade that credits nearly cover.

**Correction (second review, doc 48b W-48-7a):** the why also holds because two offers need one leg's cash up front (a
listing's price must be above zero, `mush-market/src/api.rs:791`) and pay the fees twice.

**Q-48-44 — Capacity at exchange scale** (A-14).
- **The problem:** the market has a global cap of 64 vendors (`MAX_VENDORS`), and E4 caps 16 open
  contracts an issuer and 100 live contracts a party.
- **Proposed default:**
  - A personal seller is not a vendor row, so the 64-vendor cap does not bind players. Each seller
    holds at most 32 open listings (provisional).
  - A listing is not a contract until it is accepted, so listings do not count against contract caps.
  - Long service contracts (hauls, escorts, crew hire) count against their own per-issuer budget of
    16, apart from trades, so standing jobs never block a player's trading. The 100-a-party cap stays.
  - EX.1 measures the exchange under at least the bot cohort's load before it lands.
- **Alternative:** raise the existing caps.
- **Why:** the caps bound sweep work. Splitting them by kind keeps that work bounded and stops one kind
  starving another.

**Correction (second review, doc 48b G-5, W-48-3d):** three things to add.
- **The listing ceiling.** `MAX_LISTINGS` is 1,024 for the whole market (`mush-market/src/record.rs:44`).
  At 32 a seller, about thirty sellers fill it, so EX.1's load test covers that ceiling too.
- **Both paths.** The caps are enforced on post and on join; today's retrieval path checks neither
  (§4.3).
- **Who binds first.** Institutional parties (Occator, the Trust, the Registry, busy vendors) reach
  the 100-a-party cap first, and "a personal seller" means any non-port seller, player orgs included.

**Q-48-45 — Selling a facility** (A-14).
- **Proposed default:** a facility can be sold only when it is idle: no open work order, no customer
  goods in its stores, and no unpaid running cost. The seller's debt is settled from the sale first.
  The buyer takes the facility's authored cost model with it (Q-48-37).
- **Alternative:** sell with open orders, which pass to the buyer.
- **Why:** passing live orders moves other players' escrow and goods between owners in the middle of a
  job.

**Q-48-46 — The escort's fact** (A-4).
- **Proposed default:**
  - An escort job names the protected hull and its journey: a passage, or a field sortie.
  - The passage or field host writes the fact. It is satisfied when the escort stayed near the
    protected hull (provisional: the same bubble in space, the same field in a sortie) for 90% of the
    journey's IC time (provisional), and the protected hull arrived or withdrew.
  - A link drop within the linkdead grace does not count against the escort.
  - The fact fails if the protected hull is wrecked or captured, or the escort leaves the journey.
- **Alternative:** escort by attestation only: the employer attests.
- **Why:** an escort is paid for presence and for the outcome, and only the hosts that run passages
  and fields can see both.

**Correction (second review, doc 48b W-48-3a):** the fact is keyed by the job, so it can be named when the job is
posted. The seat fact's key cannot be named in advance (`mush-sim/src/seat.rs:1060-1068`).

### 6.8 Raised by the second review (doc 48b)

Each question answers a finding that held.

**Ruled 2026-11-15: all fifteen at their defaults (Draft 0.7).** Each one's proposed text below is
now the decision.

**Q-48-47 — Core resale to a player** (W-48-1a).
- **Proposed default:** a second closed plan beside CO.13's buy-back: a named buyer only; the buyer's
  money escrowed at acceptance; lines for the seller, the venue's cut and the levy. The kernel's image
  rule admits a player destination. One CO.13-sized unit, EX.2b.
- **Alternative:** players sell cores only to the shop, as CO.13 built.
- **Why:** the ruling (Q-48-7) stands, but it is a closed family touching the kernel, and it should be
  sized as one.

**Q-48-48 — The yard's core** (W-48-1c).
- **Proposed default:**
  - Chain goods are consumed when a core's die is issued, by industry's origin provider, so
    integration still takes one die lot and the kernel is not touched.
  - A design that carries a stock core bills one new "core module" good, consumed at commission.
  - **For the owner:** yard-built combat hulls keep a governed Class II core outside the monthly die
    quota.
- **Alternative:** combat hulls are coreless, and get their cores fitted through `+core/fit` once it
  admits them, so every core comes from the quota.
- **Why:** putting every combat hull on the die quota would throttle a hundred-unit game.

**Q-48-49 — A player's goods and endpoint at a venue** (W-48-2a, W-48-2c, G-6).
- **Proposed default:**
  - Personal sellers list exact lots, split to size with `@inv/split`. These pass the hold gate today
    and keep their provenance; pooled listings stay with the port's counters.
  - A venue opens a locker vault for a player on first use, capacity-bound and charged like a berth
    (provisional: 1 cr an IC day a tonne).
  - The endpoint is any hold the player may trade, chosen by where they are present, not only the
    lowest-numbered hull (`port/coop.rs:602-638`).
  - A station or site venue gets a location gate before it lists, since the port gate admits anyone
    elsewhere (`port/market.rs:258-263`).
- **Alternative:** a personal seller must have a hull docked, and sells only from its hold.
- **Why:** an officer or a passenger without a hull could not trade otherwise. And today's
  capacity-free port vaults (`port/market.rs:477`) would be free storage.

**Q-48-50 — Standing acceptance for player sellers and bidders** (G-1(3b), W-48-2d).
- **Proposed default:**
  - **A listing is the seller's standing acceptance within its terms:** a party cap scoped to the
    seller and bound to the listing's id, price and quantity, revoked by withdrawal.
  - **A player's bid escrows price × quantity at posting**, draws as it fills, and releases at expiry.
    The bidder accepts only against its own escrowed bid.
- **Alternative:** player listings and bids are notices, and the player accepts each sale.
- **Why:** a sale must settle while its poster is away. Under the 28-IC-day rule, accepting sale by
  sale is unworkable.

**Q-48-51 — Fees off the board** (W-48-7b).
- **Proposed default:** accept it. A trade by `@contract`, or a 1 cr offer plus a side payment, pays no
  board fee. The fees are the price of the board's reach and escrow. EX.9 sizes the faucet from the
  *measured* levy, not the nominal one.
- **Alternative:** a journal-level levy on every player-to-player title transfer and E4 settlement.
- **Why:** a levy everywhere changes E4 and every venue, and taxes gifts and House moves. A trust
  trade carries its own risk.

**Q-48-52 — Hull resale integrity** (G-1).
- **Proposed default:**
  - **The hold goes with the hull.** EX.2's one builder re-homes the hold's vault in the same
    settlement (`rehome_vault`), and needs the hold empty, as the buyout does (`port/buyout.rs:321`),
    or sells the cargo as its own leg.
  - **A fitted governed core goes with it too.** A hull with one is listed only with its core, the
    core leg settling in Q-48-47's plan.
  - **No bait-and-strip.** The quote pins a digest of the hull's state (fitted parts, core,
    condition), and settlement refuses on drift. The bay refuses strip, scrap and fit on a hull under
    a live sale contract.
- **Alternative:** refuse to list a cored or laden hull, and add a `+core/remove` verb.
- **Why:** otherwise the buyer gets a hull whose hold and core the seller still owns, or a hull
  stripped after the price was agreed.

**Q-48-53 — What a facility sale moves, and the fee pinned at booking** (G-2).
- **Proposed default:**
  - **What moves:** `FacilityRecord.owner`, the stores vault and the payee.
  - **An "owner-held" lane rule.** The lane applier today refuses the whole port entry when an owner
    differs from the authored operator (`port/plant.rs:126-160`), and re-terms a facility back to
    authored on every lane bump (`mush-industry/src/store.rs:1235-1260`). An owner-held facility
    skips both.
  - **What can be sold:** not the relay foundry, the programme workshops or the medical rooms. A
    sold yard stays public.
  - **The fee is pinned at booking.** If the live fee has risen when the arm submits, the order is
    cancelled with its booking fee and bill returned. This also fixes §4.3's live defect.
- **Alternative:** authored facilities are never sold; only facilities built later are.
- **Why:** without the lane rule, a sale is undone by the next content bump. Without the pin, an
  owner reprices orders already quoted.

**Q-48-54 — Listings and obligations at 24:1** (G-3, G-4).
- **Proposed default:**
  - **A listing's lot hold renews while the listing stands**, and the listing fee is charged once a
    listing. Today the hold lasts 7 IC days, which is 7 real hours (`mush-market/src/api.rs:902-906`).
  - **A daily obligation a player owes (berth, wages) gets a grace of at least 28 periods.** It
    accrues arrears as a one-term debt (`create_debt`, Q-41-13's shape), and reinstates on funding.
    Today it goes Delinquent in 2 real hours, with no arrears (`mush-economy/src/obligation.rs:701-718`).
  - **An unpaid berth is a debt** with a lien on the hull's next sale at that port. It never blocks
    undocking before 28 IC days.
- **Alternative:** keep E4's defaults, and size every daily rate to stay under a player's balance.
- **Why:** the owner's rule that any wait on a player is at least 28 IC days. Q-48-35's reasoning
  applies to sellers too.

**Q-48-55 — House income and home berths** (G-4(3), W-48-4).
- **Proposed default:**
  - A House's hulls at its home port pay no berth fee.
  - A House gets an income: a daily stipend from the export faucet (Q-48-22), as a consuming
    institution does under Q-48-40, sized by EX.9. That pays for its licences, berths and, under
    Q-48-28, its hulls.
- **Alternative:** Houses stay funded by staff mint at the bring-up only.
- **Why:** a House's 2,000 cr is a one-off procurement mint, already spent on licences. It cannot pay
  for anything else.

**Q-48-56 — The Co-op share and offline orders** (G-7, G-8).
- **Proposed default:**
  - The Co-op's proceeds policy runs on every sale from a chartered hold, not only sales into a bid
    (`port/proceeds.rs:4-6`).
  - A captain's offline buy reads house asks only, unless the order names a price ceiling.
- **Alternative:** chartered holds may not post asks; and offline buys default their ceiling to the
  reference price × 200%.
- **Why:** a player ask would otherwise skip the Co-op's share. And once players post asks, an
  unbounded offline buy (`port/orders.rs:212`) could be drained by a griefer's listing.

**Q-48-57 — Declared value and collusion** (W-48-3c).
- **Proposed default:**
  - A haul's declared value is capped at the venue's reference price × quantity, so a shipper cannot
    inflate the bond.
  - A collusive "interception" (shipper and raider together) nets the colluders the bond's 25% at
    most; that is accepted.
- **Alternative:** an uncapped declared value, with insurance later.
- **Why:** the cap stops bond inflation, and a 25% gain bounds the collusion that remains.

**Q-48-58 — Org acceptors and job deadlines** (W-48-3d).
- **Proposed default:**
  - Members holding a capability may accept a job for their org or House. Today the join refuses
    player orgs and Houses (`contract_live.rs:6625-6630`).
  - A job's deadline is per kind: "N IC days from acceptance" for hauls, escorts and crew hire, and
    absolute for retrieval, as today.
  - The join escrows the provider's bond (EX.5).
- **Alternative:** jobs are open to characters only, with a deadline fixed at posting.
- **Why:** a deadline fixed at posting starves a late taker, and an org's members are who does the
  work.

**Q-48-59 — The way back from mothballing, and facilities with no bookable work** (W-48-5e,f).
- **Proposed default:**
  - A mothballed facility owes nothing while mothballed.
  - It may still take orders whose escrow covers their worked cost, and each fee's margin repays the
    debt first.
  - A facility with no bookable work (Kasei's refinery, mill-run) is charged per batch milled, not a
    standing share.
- **Alternative:** a house's debt is written off after 28 IC days, and it reopens.
- **Why:** paid work is the only honest way out of a debt, and a facility nobody can book should not
  pay for idle slots it cannot fill.

**Q-48-60 — One wage scale?** (raised by the verification).
- **The problem:** lane facility staff are paid 150 cr an IC day each, against Q-48-33's crew wage of
  10 or 20. That is a 15× gap, and the payroll is 62% of facility cost.
- **Proposed default:** keep 150. Under Q-48-38 the payroll bites only while the facility works, and
  the fee pays it.
- **Alternative:** 15 cr, one scale for crew and staff. That cuts works and yard fees by about 62%.
- **Why it needs a ruling:** the two scales disagree by 15×, and nothing says whether that is meant.

**Q-48-61 — Ore regeneration** (W-48-6).
- **Proposed default:** a worked-out surface site or space cluster re-rolls its deposit after 28 IC
  days (provisional). The space ore lane-wide is about 1,440 t, roughly 15 real minutes of one laser.
- **Alternative:** finite ore, and the chain ends when it does.
- **Why:** at 24:1 the closed chain closes only until the ore runs out. A re-roll keeps mining a
  job, not a one-off.

## 7. Units of work (sketch, cut after the rulings)

Two chains, one writer per file set, as docs 44 to 47 ran. **EX** (the exchange and money) and **PL**
(production) are unused as unit prefixes.

| Unit | What | Gate |
|---|---|---|
| EX.0 | `@econ/census`; the matter census's new goods, with declared sink terms (consumed, burned, fired; doc 48b W-48-6) | Q-48-17, Q-48-31 |
| EX.1 | The seller: any principal lists from its own endpoint; private offers authorised at quote and accept; exact lots only, with the hold checked at prepare (§4.3); standing acceptance; any tradeable hold by presence; the exchange-scale caps and a load test | Q-48-2, Q-48-5, Q-48-44, Q-48-49, Q-48-50, Q-48-54, Q-48-56 |
| EX.2 | One asset-sale builder; player hull resale with the hold and fitted core, and no bait-and-strip | EX.1, Q-48-7, Q-48-52 |
| EX.2b | Core resale to a player: a second closed plan, a kernel image change (CO.13-sized) | EX.2, Q-48-47 |
| EX.3 | NPC flows on the board, atomic; acceptor roles as data | EX.1, Q-48-9 |
| EX.4 | Listing fee, sale cut, levy; berth fee | EX.1, Q-48-8, Q-48-15 |
| EX.5 | The service arm; the generalised open join (escrowing the provider's bond, checking both caps, org acceptors); retrieval, passage and survey listed | EX.1, Q-48-3, Q-48-12, Q-48-58 |
| EX.6 | Haul (a held lot, declared value, a bond on it) and escort (its own host-written fact) | EX.5, Q-48-11, Q-48-42, Q-48-46 |
| EX.7 | Crew wages; player crew listings | EX.5, Q-48-14, Q-48-33 |
| EX.8 | The NPC price rule (5% an IC week) and price history as its own bounded series | EX.3, Q-48-18 (ruled), Q-48-35 |
| EX.9 | NPC consumption, running costs by the work with mothballing and debts, the export faucet paying the consumers (one unit: they balance) | EX.0, EX.8, **PL.1, PL.2**, Q-48-19, Q-48-21, Q-48-22, Q-48-34, Q-48-36, Q-48-38..41 |
| EX.10 | The NPC hauling house; consumption at ports with no producer in reach | EX.6, EX.9, Q-48-20, Q-48-41 |
| PL.0 | `MAX_GOODS` 48 → 256 (no pin moves); the unload plan takes the hopper's goods and an empty pool frees its count, so `+tv/unload` survives past 64 goods (doc 48b G-5) | Q-48-30 (ruled) |
| PL.1 | Propellant from volatiles, with its operating arm | PL.0, Q-48-23 |
| PL.2 | Provisions, instrument boards, relay blanks (`MAX_PAIRS` 8 is a Rust cap to lift, and a `relay_blank` good to add) and carbon feedstock (from carbon ore) get producers, each with an operating arm | PL.0, Q-48-23 |
| PL.3 | Munitions recipes (with doc 49's CC.5) | PL.0, Q-48-23, Q-49-4 |
| PL.4 | Mills and the refine as works recipes | Q-48-24 |
| PL.5 | Buying into an escrow (V7's seller and price generalised); a House buys a hull once it has income | Q-48-28 |
| PL.6 | The Ceres yard | Q-48-25 |
| PL.7 | Cores from the chain at die issue; a core-module good for stock-cored designs | PL.0, Q-48-27, Q-48-48 |
| PL.8 | The parts market | EX.3, Q-48-29 |
| PL.9 | Facilities as titled assets; owner terms (customers, margin); sale only when idle; what a sale moves, the owner-held lane rule, the fee pinned at booking | EX.2, Q-48-26, Q-48-37, Q-48-45, Q-48-53 |

## 8. What this survey did not check, and what the second review answered

The second review answered each item; doc 48b §5 has the evidence.
- **TacSim on the dev boxes:** `deploy/dev1` and `dev2` set `TACSIM_ENABLED=true` and `IC_RATIO=24`
  in the repo. What the boxes actually run is not readable from the tree.
- **The venue gate:** `PortVenueGate` admits anyone at a venue no port authors, but gives no hold to
  trade from (`port/market.rs:238-263`).
- **The reach premium:** non-zero under the template's moving sky, capped at 50%; zero at the engine's
  default rate of 0.
- **NPC bidders:** they consume nothing (Q-48-40 is the fix).
- **Relay carriage:** always composed (`mush-propagation/src/lib.rs:255`).
- **Inventory facts:** they carry no place, but a derived marker can be keyed to one (§5.2's
  correction).
- **The content studio's `hire`:** a name only, with no provider.
- **A yard-built escort or line tank:** carries its stock Class II core free (Q-48-27's correction).
