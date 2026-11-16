# 48a. Outside review of docs 48 and 49 (Astra, 2026-11-15): triage

Docs only. On 2026-11-15 the owner passed on an outside review of
[PR #621](https://github.com/conclavio/rustmush/pull/621) at `8ddf6ca9b` (doc 48 Draft 0.3, doc 49
Draft 0.3). The review was by Astra: read-only and source-backed, with no simulations run. The owner
asked to "review and refute, update where needed".

Every claim was checked against the tree at `b1b4c57f6`. The PR is docs only, so its code is
`b1b4c57f6`'s. This document records:
- each finding in short;
- the check it got;
- the verdict;
- what changed in [doc 48](48-economy-and-production.md) (Draft 0.4) and
  [doc 49](49-combat-content.md) (Draft 0.5).

The review's links pointed at a local checkout; they are given here as repo paths.

**Score: 14 findings, 14 held, none refuted.** One (A-7) carries a timing note: the review read
Q-49-8 as open, and the owner ruled it at its default after the reviewed commit. Two of the errors
were the lead's, not a lane's: the farm input (A-5) and "no kernel cost" for round selection (A-6).
The free laser recharge (A-8) came from a survey lane's report and was not spot-checked.

What the review got right and asked nothing of is not repeated here. It covered the exchange boundary,
the closed families, typed services, the ammunition chain, the 24:1 corrections, and data-driven space
content without one kernel.

## 1. The corrections

| # | Finding | Check | Verdict | Change |
|---|---|---|---|---|
| A-1 | Q-48-26 opens `@facility/terms` to owners, but the command sets the whole cost model, so an owner could cut its own running costs to almost nothing | `crates/mush-industry/src/commands.rs:1042-1052`: staff-only today; `<capital>/<years>/<upkeep>/<staff>/<wage>/<margin>`. Bounds `cost.rs:67-79` admit capital 1, 200 years, upkeep, staff and wage 0; `daily_cost_minor` (`:111-121`) then rounds up to 1 minor unit, 0.01 cr a day | **held** | Q-48-26 corrected; owner terms split from authored costs as Q-48-37 |
| A-2 | Total money can balance while the consuming institutions go broke; and Syrtis and Selk buy alloys and boards, not what Q-48-19 consumes | Lane `crates/tacsim/content/lane/ceres_phobos.json`: Lunar Habitat Board bids volatiles 195 and carbon feedstock 380 (capital 1.0M). Syrtis Bonded bids alloys 460 and boards 1,100 (3.1M). Selk Townships bids alloys 760 and boards 1,500 (1.35M). None has recurring income, and Q-48-22's faucet paid exporters | **held** | Q-48-19 corrected; Q-48-40 names each consumer's goods and funds the consumers from the faucet |
| A-3 | Routing NPC trades through the market does not make them atomic, and "barter is two private offers" is not an atomic exchange | `crates/mush-market/src/trade.rs:75-96`: `DRAW_SETTLE_MARGIN_SECS` 600 exists because a late Draw could move goods after the contract refunds. `:874-886`: a hold-backed leg that cannot be staged is "§9 risk 3", logged. The lot path is escrow → staged leg → fact → sweep settlement, not one transaction. Two private offers are two contracts | **held** | §5.1, Q-48-5 and Q-48-9 corrected to state the guarantee the path gives; barter becomes Q-48-43 |
| A-4 | `sim.station.served` does not prove an escort | `crates/mush-sim/src/seat.rs` `SimFactProvider::resolve`: `satisfied: service.to >= service.from`, so a zero-length seat interval satisfies it. It names a seat on a unit, not a protected hull or a journey | **held** | §5.2 and Q-48-11 corrected; the escort fact is Q-48-46 |
| A-5 | The provisions farm consumes carbon feedstock, which nothing produces; mining yields `carbon_ore` | `content/market/goods.json`: `carbon_feedstock` (`:100`) and `carbon_ore` (`:139`) are distinct goods of one family. No recipe outputs feedstock (doc 48 §3.7) | **held** (the lead's error) | Q-48-23 corrected: a carbon-ore-to-feedstock process is added |
| A-6 | AP/HE selection at rearm, differing in which sections they reach, is not content-only | `crates/tacsim/src/definitions/weapon.rs:111`: a mount names one `ammunition`. `arena/damage.rs:895` `choose_section(target, band, shot_seq)` takes no round. So a selected round needs mount state, and a section effect needs the resolver | **held** (the lead's error: "no kernel cost") | Q-49-14 corrected; Q-49-21 proposes deferring the round choice (CC.11 leaves the programme) |
| A-7 | A 35% threshold does not remove automatic triggering: a slot on a steady 80% target fires at every ready moment with no new decision | Doc 49 Q-49-8 as written; `stock_mind.rs` "the armed slot owns the shot" | **held**. Timing: the owner ruled Q-49-8 at its default after `8ddf6ca9b`. The ruling stands; the text overclaimed | Q-49-8's framing corrected; the honest choice (standing or single-use authorisation) is Q-49-20 |
| A-8 | Laser recharge is not free: no good, but labour and time | `crates/tacsim/src/port/bay_work/jobs.rs:430-433`: cost = rounds × `cost_per_ammo_round` (25 minor, `content/market/workshop_tariff.json:12`), time `duration_ic_rearm_component` 1,800 IC s. A 500-charge refill is 125 cr before the operator's multiplier | **held** | Doc 49 §0, §3.2 and §4 corrected |

## 2. What the documents did not cover

| # | Finding | Check | Verdict | Change |
|---|---|---|---|---|
| A-9 | Cargo liability: a 500 cr haul can entrust 100,000 cr of cargo, and a bond on the job's price does not cover it | Q-48-13's bond is 20% of the price above 10,000 cr | **held** (gap) | Q-48-42 |
| A-10 | Insolvency and recovery; and the throttle's 10% an IC week against the price rule's 5% | Stickney Yards: capital 50,000 + a raise of 50,000. Its workshop, works and yard cost about 1,229 + 1,693 + 3,261 = 6,183 cr an IC day by `daily_cost_minor` over the lane rows: about 16 IC days, so **16 real hours**, with no orders. Q-48-34 (10% an IC week) and Q-48-18 (5% an IC week) do not say which wins | **held** | Q-48-38 (worked-hours costs, mothballing, debts, customer goods) and Q-48-39 (the throttle against the step) |
| A-11 | Production must be ready before consumption starts, and a recipe alone does not run itself | Doc 48 §7: EX.9 was gated on EX.0 and EX.8, not on any PL unit. Doc 49's CC.7 had no gate on space munitions being produced | **held** | Q-48-41; gates changed in both §7s |
| A-12 | Space lasers switch between offence and defence on one cooldown, so a mount-level exemption cannot express the rulings | `crates/tacsim/src/space/weapons.rs` `LaserMount`: one `mode` and one `cooldown_ms` / `cooldown_remaining_ms` | **held** | Q-49-24 |
| A-13 | The ruled cycles with the proposed magazines *raise* continuous-fire endurance; and a mind holding a reserve is not "dry" | Arithmetic checked: cannon 40 × 6 s = 240 s → 24 × 15 s = 360 s; coaxial 200 bursts × 1 s = 200 s → 60 × 5 s = 300 s; autocannon 80 × 3 s = 240 s → 40 × 10 s = 400 s; missiles 6 × 10 s = 60 s → 4 × 30 s = 120 s. `arena/stock_mind.rs:431-434` `is_dry`: dry only when no row holds a load | **held** | Q-49-22 (magazines sized by endurance, set by measured fights) and Q-49-23 (when a reserve is spent, and dry for withdrawal) |
| A-14 | Capacity, privacy and lifecycle at exchange scale | `crates/mush-market/src/record.rs:66` `MAX_VENDORS` 64; `crates/mush-economy/src/contract.rs:92` 16 open contracts an issuer, `:98` 100 live contracts a party. Private offers, facility sales with open orders and debts, and round conservation were unstated | **held** | Q-48-44 (caps), Q-48-45 (facility sales); private-offer authorisation stated in §5.1; round conservation a requirement in doc 49 §7 |

## 3. What changed, in one place

- **Doc 48 Draft 0.4:**
  - corrections to §5.1, §5.2, Q-48-5, Q-48-9, Q-48-11, Q-48-19, Q-48-23 and Q-48-26, each marked
    as an outside-review correction;
  - follow-ups Q-48-37 to Q-48-46 (§6.7), open;
  - §7's gates updated.
- **Doc 49 Draft 0.5:**
  - corrections to §0, §3.2, §4, Q-49-8 and Q-49-14;
  - follow-ups Q-49-20 to Q-49-24 (§6.6), open;
  - §7's gates and a round-conservation requirement.

A ruled default that the review showed was wrong in its facts is corrected in place. A ruled default
that needs a new choice gets a follow-up question; the ruling stands until that is answered.
