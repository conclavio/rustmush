# 48b. Second outside review of docs 48 and 49 (Gemini, 2026-11-15): triage

Docs only. On 2026-11-15 the owner passed on a second and final design review of
[PR #621](https://github.com/conclavio/rustmush/pull/621), by Gemini, and asked for it to be triaged
as the first was ([doc 48a](48a-outside-review-triage-2026-11-15.md)).

**What the review read.** It read `8ddf6ca9b`: doc 48 Draft 0.3 and doc 49 Draft 0.3, from before
48a. Its findings were checked against the code at `b1b4c57f6` and against the current drafts (doc 48
Draft 0.5, doc 49 Draft 0.6). So a finding can be **already addressed** by a later draft, or **moot**
under a later ruling. The main one is Q-49-20, where the owner unwound the "no auto-fire" rule.

**How it was checked.** Five read-only lanes, each one recomputing the review's numbers. The lanes
covered the exchange's architecture, services and money, the economy's blind spots, ground combat
and minds, and space with the §8 answers. Where a lane disagreed with the review, the code decides.
The review's links pointed at a local checkout; here they are given as repo paths.

**Wording.** Combat findings are stated at the level of game mechanics (the owner's rule of
2026-11-02).

## 0. In brief

- **About 75 claim parts.** Most held or partly held, about 10 were refuted, and about 12 were
  already addressed or moot. The rows below give each verdict.
- **The review was strongest where the drafts had promised too much.**
  - The claim that the exchange needs no closed-family edits.
  - The claim that a player seller "reuses the lot legs as they are".
  - The ruled defence-only fit for the Dray, which leaves R-47-2 with no live hull.
  - The lone-shooter balance in space.
- **It was weakest where it predicted collapse from arithmetic the code does not support.**
  - It claimed bid escrow that does not exist.
  - It claimed a ×9.85 price spiral that the 200% band stops.
  - It called the munitions recipes a contradiction.
  - It claimed a slot count, a "flank" token and a mind-version bump that the code refutes.
- **Two of its findings reverse owner rulings**, so they go back to the owner as questions, not
  corrections:
  - the Dray's lasers (Q-49-25 against Q-49-7 and Q-49-24);
  - the judgement term on player slots (Q-49-29, after Q-49-20).
- **Five findings are live defects in the tree today.** They go to the fix thread, listed in
  doc 48 §4.3 and doc 49 §4a:
  - the works arm charges the live fee, not the quoted one;
  - a hold-backed Consume skips the capacity check;
  - the house-sale guard matches a contract by name alone;
  - the yard zeroes a ground laser's charge;
  - the retrieval join checks no contract cap.

## 1. Doc 48: the exchange's architecture (W-48-1, W-48-2, W-48-7)

| # | Finding | Verdict | Evidence (`b1b4c57f6`) | Change |
|---|---|---|---|---|
| W-48-1 | Doc 48 promises no closed-family edits | **partly held** | §5.6 says "for open kinds" only. But §4.1's "nothing in this proposal touches them except to let the exchange route to them" is contradicted by Q-48-3, -7 and -27 | §4.1 and §5.6 corrected |
| W-48-1a | Core resale to a player cannot go through CO.13's family | **held** | `mush-core/src/core_sale.rs:222-227`, `:421-424` (Player source, Thing\|Room destination, seller is the actor); `mush-economy/src/registry_filing/core_sale.rs:36-42` (one composition-time vendor), `:118-131` (two lines), `:386-389` (no holds) | Q-48-7 corrected; **Q-48-47** |
| W-48-1b | A managed licence cannot settle an open ask | **held** (the review's anchors are wrong) | `paid_resale/protocol.rs:230-236` (named buyer); `managed_paid_resale/subject.rs:235-236`; `mind_paid_resale/offer.rs:22` (seller's act), `:30-52` (frozen cuts); `managed_paid_resale.rs:43` (86,400 s) | Q-48-3 corrected |
| W-48-1c | The yard cannot bill a governed core | **held**; "ungoverned" refuted | `core_integration.rs:235-254` (one lot); `core_input/records.rs:217-221` (quantity 1, root); `bill.rs:98-101`. tv_escort and line_tank commission their stock core free (`commission.rs:12`, `hull.rs:746`); `cores.json` marks `tacsim.core.line` governed Class II; `+core/fit` gate at `counter.rs:486-493` | Q-48-27 corrected; **Q-48-48** |
| W-48-2a | Pooled lots cannot go into a hull from a player | **held**, narrower | `mush-market/src/trade.rs:979-985`: only a port seller's Draw or an exact whole lot (`commands.rs:1418-1443`) | **Q-48-49** |
| W-48-2b | A Consume skips the capacity check | **held** (latent defect) | `lot.rs:486` (`plan: null`), `:873-903`; Draw re-checks `:1126-1150`; stale comment `:415-416` | §4.3 defect; EX.1 |
| W-48-2c | A player has no endpoint at a venue | **partly held** | `@vault/create` and `@org` admin-only (`mush-economy/src/commands.rs:3974`, `mush-org/src/commands.rs:104`); `endpoint_for` needs a charter helm (`port/market.rs:239-247`) | §5.1 corrected; **Q-48-49** |
| W-48-2d | Player bids cannot settle | **held** | `post_wanted` escrows nothing (`mush-market/src/api.rs:1331-1335`); `sell_into_bid` accepts as the house (`trade.rs:2412`), which needs `PartyCap::Accept`, NPC-only (`port/market.rs:340-388`) | **Q-48-50** |
| W-48-7a | Barter by two offers is cash-gated and double-taxed | **held**, mostly addressed | `api.rs:791` (price > 0); Q-48-5's correction, Q-48-43 | Q-48-43's why extended |
| W-48-7b | Fees are bypassed off the board | **partly held** | `@contract` has no fee term (`contract_live.rs:237-263`); `@econ/pay` is open to players | **Q-48-51** |

## 2. Doc 48: services, money and slips (W-48-3 to W-48-8)

| # | Finding | Verdict | Evidence | Change |
|---|---|---|---|---|
| W-48-3a | The seat fact is not an escort fact | **held, already addressed** (Q-48-46) | `mush-sim/src/seat.rs:1095` (0 s satisfies), `:1060-1068` (key not nameable), `:910-939` | Q-48-46: the fact is keyed by the job |
| W-48-3b | Inventory facts cannot name a destination | **partly held** | Existence only (`mush-inventory/src/facts.rs:61-81`), but markers store from, to, commodity and amount (`pool.rs:776-784`), and `derived_marker_id` (`pool.rs:160-187`) can be named at offer time. `@inv/move` stays within one principal | §5.2 and §8 corrected |
| W-48-3c | A 20% bond makes cargo theft profitable | **partly held**: the EV is moot under Q-48-42 | A hold does withhold from the custodian (`pool.rs:381-393`). Gaps: the hold lasts 7 IC days unless `reserve_for`; nothing said about the deadline; pool goods carry no title (`record.rs:8-13`); collusive "interception" | Q-48-42 corrected; **Q-48-57** |
| W-48-3d | `accept_retrieval` cannot generalise as it is | **held** | No bond escrow at join (`contract_live.rs:6603-6604`); player orgs and Houses refused (`:6625-6630`); deadline fixed at post (`:6511-6514`); neither contract cap checked on post or join (`:6555-6556`, `:586`, `:1680-1690`) | §4.3 defect; **Q-48-58** |
| W-48-4 | A House cannot buy a hull straight into escrow | **partly held** | No ask for any part good; V7 already buys a hull bill into escrow with no hold (`port/pools.rs:976-999`). `clean_part` refuses non-Intact lots (`works.rs:607-618`; there is no "worn" condition). Stickney has no ask for boards or strategic ore. A House holds about 2,100 cr against a Dray of about 156,400 | Q-48-28 corrected; **Q-48-55** |
| W-48-5a | Daily cost and wages double-count | **partly held** | `daily_cost_minor` includes payroll (`mush-industry/src/cost.rs:111-120`) | Q-48-21 wording |
| W-48-5b,c | 325,218 cr a real day; Stickney broke in 12 real hours | **figures partly refuted, moot** | Bids escrow nothing (`api.rs:1333-1335`), so 48a's 16 hours stands. Under Q-48-38 the idle drain is about 6.7 real days and break-even is about 29% use | — |
| W-48-5d,e | Basin and Tether at 0 cr; Kasei and Tycho bleed | **held, moot** | `launch-bringup.py:918-929`. Residue: Kasei's margin-0, never-booked refinery, and tariff-priced workshops (`bay_work/command.rs:307-316`) | Q-48-38 wording; **Q-48-59**; §4.3 note on CO.13's 0 cr integrator |
| W-48-5f | The throttle is an absorbing death spiral | **partly held** | ×9.85 refuted (the band stops at 200% after about 2.1 real days). Absorbing holds: a mothballed house takes no orders and posts no bids, so it never gets 7 IC days ahead | **Q-48-59** |
| W-48-6 | The faucet breaks the census; ore is finite | **partly held** | The census has no sink terms (`port/census.rs:100-111`, `:29-31`). "1,440 t lane-wide" counts space only (surface sites hold more), but no deposit regenerates | EX.0 corrected; **Q-48-61** |
| W-48-8.1 | V9's guard matches by name, which players can grief | **held** | `port/house_sale.rs:76-85`; `contract_live.rs:543-553`; it blocks `/forsale` and `/buy` | §3.2 corrected; §4.3 defect |
| W-48-8.2 | `MAX_GOODS` moves no pin | **held** | `definitions/goods.rs:282`, `:624-629`; each goods row re-pins `ic2_the_goods_table_is_pinned` | Q-48-30's why and PL.0 corrected |
| W-48-8.3 | Every ship is at least 240 t | **partly held** | Surface vehicles are 6–12 t; the "under 100 t" is in the superseded text | Q-48-15 decision: any mass |
| W-48-8.4 | "Eight rows" of munitions; the relay cap | **"eight rows" refuted** (4 goods × 2 ports); **relay cap held** | `relay_foundry.rs:26`, `:50`, `:383`; no `relay_blank` good | PL.2 corrected |
| — | Staff wage 150 cr an IC day against crew 10/20 (raised by lane 2) | **held** | Lane cost blocks; payroll is 62% of facility cost | **Q-48-60** |

## 3. Doc 48: what it did not cover (G-1 to G-9)

| # | Finding | Verdict | Evidence | Change |
|---|---|---|---|---|
| G-1(1) | A sold hull's hold vault stays the seller's | **held** | `port/commission.rs:572-586`; `TransferTitle` moves the title only (`mush-economy/src/engine.rs:1313-1341`); precedent `rehome_vault` (`api.rs:1204`) | **Q-48-52** |
| G-1(2) | A fitted core stays the seller's | **held** | `managed_title.rs:119-150`; no unfit verb (`hardware/counter.rs:70-81`) | **Q-48-52** |
| G-1(3a) | Bait-and-strip | **held** (cited precedent wrong) | No encumbrance on a listed object; `strip.rs:190` checks no sale | **Q-48-52** |
| G-1(3b) | A player seller has no acceptance seam | **held** | `trade.rs:827-841` (`accept_contract_as` under frozen caps); `api.rs:920`, `:1429` (seller is an org) | Q-48-2 corrected; **Q-48-50** |
| G-2(1) | A facility sale updates no state | **held**, and worse | Stores vault and foundry (`works.rs:551-575`, `relay_foundry.rs:156-174`). Worse: the lane applier refuses the whole port entry when the owner differs (`port/plant.rs:126-160`), and re-terms customers and cost back to authored on every lane bump (`mush-industry/src/store.rs:1235-1260`) | **Q-48-53** |
| G-2(2) | Live-fee bait-and-switch | **held**; also a live defect on a staff re-term | `works.rs:1799-1823` skips `@wo/submit`'s revision check (`commands.rs:1294-1302`); `money.rs:220-229` reads the live fee | §4.3 defect; **Q-48-53** |
| G-2(3) | Monopoly on the only yard | **partly held** | `may_order` (`store.rs:1211-1223`); PL.6's Ceres yard covers light hulls only | **Q-48-53** |
| G-3 | Lot listings last 7 real hours | **held** | `api.rs:902-906`; `lot.rs:1772-1775` (604,800 s / 24) | **Q-48-54** |
| G-4(1,2) | Obligations go Delinquent in 2 real hours, with no arrears | **held** | `mush-economy/src/obligation.rs:701-718`, `:309-314`, `:666-668`; `api.rs:2572-2607`. This breaks the 28-IC-day rule. The undocking hold is the fines quarantine only (`voyage.rs:3456-3459`) | **Q-48-54** |
| G-4(3) | House treasuries drain | **partly held** | Arithmetic right (5.5 real days). That treasury is a one-off procurement mint; the real gap is that Houses have no income | **Q-48-55** |
| G-5 | Caps | **mixed** | `MAX_VENDORS` already addressed (Q-48-44). `MAX_LISTINGS` 1,024 global (`mush-market/src/record.rs:44`) held, new. `MAX_POOLS_PER_ENDPOINT` held: never decrements, and the unload plan takes every accepted good (`mush-inventory/src/pool.rs:463-470`; `surface/act/fixed.rs:426-432`; `providers.rs:2101-2107`). `MAX_TRADE_ROWS` partly. `MAX_OPEN_SELLS` moot | Q-48-44 corrected; PL.0 and EX.8 corrected |
| G-6 | One hull per helm; off-hold capacity; the gate fails open | **held** (`tag=part` moot: read-only) | `port/coop.rs:602-638`; `port/market.rs:171-176`, `:240-247`, `:477`, `:258-263` | EX.1 corrected; **Q-48-49** |
| G-7 | Co-op share skipped off the bids | **held** | `port/proceeds.rs:4-6` | **Q-48-56** |
| G-8 | Offline orders drained by a griefer's ask | **refuted today; held prospectively** | `ask_at` reads authored vendors only (`port/executor.rs:614-645`); `max_major: None` by default (`port/orders.rs:212`) | **Q-48-56** |
| G-9 | A flat 600‰ part bid; provenance laundering | **bid partly held; laundering refuted** | Scrap credits parts 750‰ intact and 400‰ worn (`bay_work/scrap.rs:7-11`, `:32-36`). Parts are indivisible and lineage is kept (`pool.rs:1962`, `:1940-1943`) | Q-48-29 corrected |

## 4. Doc 49 (W-49-1 to W-49-8, G-10 to G-14)

| # | Finding | Verdict | Evidence | Change |
|---|---|---|---|---|
| W-49-1 | A defence-only Dray cannot answer fire. R-47-2 has no live hull, and the tests that assert a Dray answers break | **held, undercounted** | `space/custody.rs:204-244`; `bubble.rs:2315`, `:2797-2822`. The tests are `space_ap9.rs:514-548`, `field_ap11.rs:1428-1430`, four `tick_thinks_tests.rs` tests and six `voyage.rs` assertions. Ship cores are Dray-only (`definitions/core_replacement.rs:191-202`). The rules go vacuous rather than break: GUARD drops to HOLD (`arena/absent.rs:645-651`). `backup.rs:170-189` and `view.rs:341-348` ignore the mode | **Q-49-25**: it reverses part of two rulings |
| W-49-2 | The reserve parks two minds forever | **held**; "forever" refuted in the simulator (the 900 s cap); today's Sniper card has the same problem | `arena/conditions.rs:1055-1061`; `stock_mind.rs:434-437`, `:1947-1960`; `damage.rs:1714-1739`; `fired_on_by` recorded only in host-ended fields (`absent.rs:585-600`) | Q-49-23 completed (four sentences) |
| W-49-3a | Heat never limits the ground laser | **held, understated** | A shot crossing the ceiling is capped, not refused (`arena/heat.rs:115-127`, `:160-162`); refusal only at the ceiling (`weapons.rs:1116-1122`). Not at 5 s, and not at today's 2 s either | §3.1 corrected; **Q-49-28** |
| W-49-3b | 5 s is too slow to stop a drone | **partly held** | Drone 120 from the front, 90 from the rear, at 25 a charge; a coaxial burst already matches it. One damage figure cannot be both weak against armour and strong against drones | **Q-49-27** |
| W-49-3c | One load cannot kill an equal tank | **partly held; already addressed** | Inputs check, but the expected hits are about 7.5 and 12.5, not 8.1 and 13.5. Marginal against a line tank, well short against the heavy | CC.2 measures it (Q-49-22) |
| W-49-4(1) | A stock screen stops every torpedo of a lone shooter | **held, not covered** | `space/weapons.rs:57-68`, `bubble.rs:2236-2243`, `:2757-2778`, `:2800-2822`; the shipped leak test strips the screen (`space/mod.rs:440-480`) | **Q-49-26** |
| W-49-4(2) | The spinal gun hits all or nothing | **held on outcome**; the corvette's radius is 5 m, not 6 m | `custody.rs:418-425`; no chance roll; dodging costs Δv (`bubble.rs:2554-2582`) | **Q-49-26** |
| W-49-4(3) | The three space doctrines fail | **held, already addressed** (CC.8) | §3.4 misdescribed the interdictor and the belt raider (`interdictor_patrol.rhai:90-99`, `applier.rs:680-682`, `belt_raider.rhai:50-71`) | §3.4 corrected |
| W-49-5 | Space has no odds to show | **held** | No draw anywhere in `bubble.rs`; five bands (`:949-995`); the mind view uses constants (`view.rs:335`, `:477`) | Q-49-13 corrected |
| W-49-6 | The cockpit reads no card; aspect "all"; "flank"; bare `FIRE` | **mixed** | The Compact floor binds every slot (`conditions.rs:568-573`), so "undefined" is overstated. "aspect = front, side, rear" holds as fact (`rules.rs:321-340`). "Flank" refuted (`card.rs:151-175`). Bare `FIRE` is by design | Q-49-8 corrected; **Q-49-29** |
| W-49-7 | Round choice is engine work | **already addressed** (48a A-6, Q-49-21) | — | — |
| W-49-8a | The munitions recipes contradict GC.7's prices | **refuted** | A batch outputs many units; `mount_105` turns 4 t into 28,000 cr through output count and works time (`recipes-v2.json`) | Q-49-4 clause |
| W-49-8b | Recharge away from port is free | **held** | `bay_work/mod.rs:37`, `:179`; `command.rs:250-269` (Q-41-7) | §0 and §3.2 corrected |
| W-49-8c | "Class I gives two slots" is wrong | **refuted** | The stock mind thinks at the lesser of its own Class I and the hull's core (`arena/mind.rs:141`, `core.rs:483-496`); the hull's 4 or 6 apply to a seated pilot | §4 item 8 clarified |
| G-10(1,2) | CC.6 breaches `MAX_GOODS`; the yard asks for space parts | **partly held**, conditional on CC.6 adding part goods | `definitions/mod.rs:644-679`; `port/bill.rs:97-110` | CC.6 gated and scoped |
| G-10(3) | The yard zeroes a ground laser's charge | **held: live defect** | `port/commission.rs:735-761` at the Emptied step (`:590-593`), against Q-39-36 and the module header `:20-21` | doc 49 §4a defect |
| G-11 | The space schema; first-mount-only views | **held / partly held** | Range cap 100 km (`definitions/weapon.rs:33`); `count_per_load` ≥ 1. The mind view sees the first mount of each family only (`view.rs:341-367`), and `.max(8)` / `.max(24)` shrink Halcyon's maximum. A second tube family or spinal gun cannot fire, but no shipped hull carries one | CC.6 scoped |
| G-12 | The ground laser fires without limit | **held**; "infinite" overstated (500 charges is about 42 minutes) | W-49-3a and W-49-8b together | **Q-49-28** |
| G-13 | Mind version blast radius | **partly held** | 31 of 32 functions holds. The `STOCK_VERSION` bump is refuted (`minds/content.rs:185-213`). The ABI move only comes with a view field, and old ABIs are retained (`minds/mod.rs:221-240`) | §3.6 bullet |
| G-14 | Match length, harness window, bot windows | **partly held** | The 900 s cap and the 30 s harness window hold. The bot's `EXCHANGE_WINDOW_MS` (20 s) holds; its `HALT_MAX_MS` (60 s) does not (`mush-bot/src/arena/pilot.rs:688-694`) | CC.2 and CC.10 scoped |

## 5. The §8 items the review answered

All held or partly held. Each doc's §8 now carries its answers instead of "not checked".
- dev1 and dev2 set `TACSIM_ENABLED` and `IC_RATIO` 24 in the repo's files; the boxes themselves
  cannot be read from the tree.
- The venue gate admits anyone at a venue no port authors, but gives them no hold.
- The reach premium is non-zero under the shipped moving sky (capped at 50%) and zero at the engine's
  default rate.
- The NPC bidders consume nothing.
- Relay carriage is always composed.
- Inventory facts carry no place, though a derived marker can be keyed to one.
- The studio's "hire" is only a name.
- The stock core is carried free.
- The space cockpit shows mount counts, heat, range and EMCON, but no rounds, cooldowns, modes or
  aspect.
- The substep is derived, and is 250 ms at shipped speeds.
- Space magazines refill at check-out, end to end.
- Simulator pods start full; staff `@arena/start` takes a unit's current cells.

## 6. What changed

- **Doc 48 Draft 0.6:**
  - corrections in place, marked **Correction (second review)**;
  - follow-ups Q-48-47 to Q-48-61 (§6.8), open;
  - §4.3's defect list extended; §7 and §8 updated.
- **Doc 49 Draft 0.7:**
  - corrections in place, marked the same way;
  - Q-49-23 completed;
  - follow-ups Q-49-25 to Q-49-29 (§6.7), open;
  - a new §4a listing the defects found;
  - §7 and §8 updated.

As in 48a: a ruled default that was wrong in fact is corrected in place. A ruling that needs a new
choice, or that a finding would reverse, gets a follow-up question, and the ruling stands until that
question is answered.

## 7. The owner's rulings on the follow-ups (2026-11-15)

All twenty were accepted at their defaults ("Yes, at defaults"): doc 48 Draft 0.7 and doc 49
Draft 0.8.
- **Q-49-25** keeps the Dray's and Dray-P's lasers dual-mode, so W-49-1's finding is answered and
  R-47-2 keeps a live hull. It supersedes the "defence-only" fit in Q-49-7 and Q-49-24.
- **Q-49-29** drops the judgement term on player slots, superseding part of Q-49-8.
- The rest are at their defaults, including the owner's own calls in Q-48-48, Q-48-60 and Q-48-61.

The live defects are fixed in their own session and PR, apart from these documents.
