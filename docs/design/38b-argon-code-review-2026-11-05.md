# 38b. Adversarial Code Review of the Post-IC Train (Docs 38, 23, 39, 40, 41, 42/42b)

**2026-11-05.** Reviewed against `origin/main` at `57f4c7e66c43f9f67060d276f8f567575432cbd2` (`2026-11-05 06:58:37 -0800`), covering the 761 commits landed in the preceding 72 hours (`2026-11-02T07:19:05-08:00` → `2026-11-05T06:58:37-08:00`).

**Authority & Scope:**
- [Doc 38](38-after-the-ic-phase-next-systems.md) (`Q-38-1..29`, §9 acceptance rules)
- [Doc 23](23-ic-sorties.md) (§17 moving-anchor custody, `D-23-1..23`, `Q-23-7..22`, `D-23-18` carried memory) and [23-cut-notes.md](23-cut-notes.md) (`S.4b-T`, `S.4b-1`, `S.4b-2`, `S.4b-3`, `D-23-18`)
- [Doc 39](39-industry-production-chain.md) (`D-39-1..14`, `Q-39-1..28`, §6 `IN.0`–`IN.11`) and [39-cut-notes.md](39-cut-notes.md) (`IN.0`–`IN.10`)
- [Doc 40](40-field-second-round.md) (`Q-40-1..25`, §5 `FR.0`–`FR.7`) and [40-cut-notes.md](40-cut-notes.md) (`FR.0`–`FR.7c`, `FR.5b`, ramp-exit boarding window `b34208d8`)
- [Doc 41](41-restoration-and-salvage-loop.md) (`Q-41-1..25`, §5 `RS.0`–`RS.10`) and [41-cut-notes.md](41-cut-notes.md) (`RS.0`–`RS.8`, `RS.6b`, `RS.6c-1`)
- [Doc 42](42-crew-custody-and-capture.md) (`Q-42-1..24`, §5 `CR.0`–`CR.7`), [42-cut-notes.md](42-cut-notes.md), and [Doc 42b](42b-surrender.md) (`Q-42b-1..21`, `SU.0`–`SU.6`)
- Owner 28-IC-day player-response floor rule (`cf162b21` / `#475`, `c558db35`)

Method: five parallel domain and cross-seam review lanes (Doc 23 / `S.4b`; Doc 39 Industry; Doc 40 Field Second Round; Doc 41 Restoration & Salvage; Doc 42 / 42b Crew Custody & Surrender), with every reported P0 and P1 verified against exact file paths, line numbers, and caller chains at `57f4c7e6`.

---

## Executive Summary Table

| ID | Sev | Doc / Cut | Commit(s) | Primary Location (`57f4c7e6`) | Summary | Cut-Note Status |
| :--- | :---: | :--- | :--- | :--- | :--- | :--- |
| **F-01** | **P0** | 23 `S.4b-3` × 40 `FR.5b` | `911897b5`, `55753484`, `870d27cb`, `0c3a6d68` | [`carrier.rs:580-602, 822-827, 945-966`](../../crates/tacsim/src/field/carrier.rs#L580-L602) | `ArenaMsg::Join` hardcodes `source: None` and omits `IcHost::resume` / `seed_carried_mind`; claimed-mind rig rejoining field after carrier loss (`boarding::tick` → `carrier::rejoin`) triggers an infinite 1-Hz checkout/refuse/checkin loop; `join_launches` skips `LaunchSeal::Claim`. | Both `23-cut-notes.md` & `40-cut-notes.md` pointed to the other PR; neither wired it |
| **F-02** | **P0** | 23 `S.4b-2` / `S.4b-3` | `911897b5`, `55753484` | [`providers.rs:2951-2956, 3329-3346`](../../crates/tacsim/src/surface/act/providers.rs#L3329-L3346) | `RefusedCode::NeverLeft` on `ActKind::Launch` clears `record.excursion = None` before `epoch_move`, leaving `record.claim = Some(ClaimCursor::Open)` with `sortie = None` forever — permanently leaking the IC claim, exclusion, custody, and Roll licence seat, and bricking the rig (`CLAIM_STILL_OPEN`). | Contradicts `23-cut-notes.md:205, 417-422` (`D-23-23` step 5) |
| **F-03** | **P0** | 23 `S.4b-2` / `S.4b-3` | `911897b5`, `b31469c5` | [`close.rs:318-324`](../../crates/tacsim/src/field/close.rs#L318-L324), [`ic_exclusion.rs:1233-1235`](../../crates/mush-sim/src/ic_exclusion.rs#L1233-L1235) | Claimed hull destroyed (`UnitOutcome::Irrecoverable`) in a field calls `mush_sim::unplace` synchronously in `on_ended` → `demote_units` while its IC claim is still open in `IcExclusions.active`; `hangar_move_admitted` refuses `unplace`, leaving a ghost hull on the site ground forever. | Undocumented bug |
| **F-04** | **P0** | 23 `S.4b-T` / `S.4b-2` | `b0279b26`, `911897b5` | [`deployed.rs:248-250, 300-302`](../../crates/mush-sim/src/deployed.rs#L248-L302) | `mush_sim::deployed::excluded(Shown::Unit)` checks `is_custodied` (`is_deployed \|\| ic_exclusion::identity_of.is_some()`), treating every surface rig with an open `AnchorImage::Hangar` mind claim as "deployed in an arena" for its entire surface excursion — hiding it from `look`, `+map`, `atlas.map`, `@unit/show`, `tactics.vitals`, and world sensor propagation even when no field is open. | Undocumented bug |
| **F-05** | **P0** | 23 `S.4b-2` × 41 `RS.2` | `911897b5` | [`tow.rs:3104, 3212-3220, 1666-1689`](../../crates/tacsim/src/surface/tow.rs#L3212-L3220) | Towing/loading a `written_off` leased rig that has an open IC claim calls `tv::hand_back` → `mush_sim::place_interior(..., shed)` (`FixedDomain`) **before** `TowOrder::Loaded` calls `owe_claim_close`; `hangar_move_admitted` refuses with `SpatialError::IcCustody`, failing the load (`"refused:it never reached the bay"`) and leaking the claim forever. | Undocumented bug |
| **F-06** | **P0** | 39 `IN.10` × `cf162b21` | `6733ca1d`, `cf162b21` | [`coop.rs:545-569, 711, 792-807, 972, 1068`](../../crates/tacsim/src/port/coop.rs#L545-L569), [`in10_pools.rs:430`](../../crates/tacsim/tests/domain_economy/in10_pools.rs#L430) | `+coop/enrol` lends yard-built Trust Drays (`tacsim.owned_hulls`) to the 9th+ captain, but `live_charter`, `helm_holder`, `check_hull_name`, and `row_of` only scan `port.map.hulls` — brick-locking the 9th captain (`helm_of == None`) and causing `in10_pools::stocked` to **panic on `origin/main`** after `cf162b21` raised `DORMANCY_SECS` to 28d. | Undocumented bug + failing test on `main` |
| **F-07** | **P0** | 39 `IN.7` / `IN.9` | `1f925b86`, `27da64e7` | [`works.rs:1161-1184, 1261-1321, 2506-2534`](../../crates/tacsim/src/port/works.rs#L1161-L1184), [`commission.rs:88-115`](../../crates/tacsim/src/port/commission.rs#L88-L115) | Complete part provenance, legality (`Hot`/`Disputed`), condition (`Damaged`/`Jammed`), and lot-hold bypass in `+yard/deliver` and `commission`: `bay_work::disputed_refusal` has zero callers outside `bay_work`; `Hot`, `Disputed`, `Damaged`, `Jammed`, and `fitted_into` parts are accepted into hull builds and laundered into brand-new `Intact` hulls without setting `fitted_into`. | Contradicts `39-cut-notes.md:143-145, 232` & Doc 39 §4.7 |
| **F-08** | **P0** | 40 `FR.3` × `FR.1` × `814f9ea1` | `22994b6a`, `0fd2430b`, `814f9ea1` | [`parties.rs:321-326, 431-447, 538-561, 601-618`](../../crates/tacsim/src/field/parties.rs#L431-L447), [`carrier.rs:413-526`](../../crates/tacsim/src/field/carrier.rs#L413-L526) | When a carrier is admitted to a field with no detachment flight yet on the ground (`PartyKey::Unit`) and then launches slot 1 into the running field (`+sortie/launch`), `carrier_entrant` switches the carrier's key to `PartyKey::Operation`: the launched detachment is **not** marked `Friends` with its own carrier, `party_sides` loses the carrier's stances, and the leader's `+field/mark` (`led_now`) fails to mark in the launched `Operation` party's book. | Undocumented bug |
| **F-09** | **P0** | 41 `RS.6b` × `RS.6c-1` | `4c99e772`, `f94fb407` | [`house_arm.rs:199-232, 333-343`](../../crates/tacsim/src/port/house_arm.rs#L199-L232) | Tycho recovery arm (`house_step`) unconditionally prioritizes `job()` over `sweep_act()` mid-sweep: if any retrieval lapses to `RetrievalStage::House` while `crawler` is deployed on a site for a sweep, `next_act` returns `HouseAct::Wait` (if another site) or refuses `HouseAct::Tow` (if same site while towing), deadlocking `lifter` and `crawler` forever. | Undocumented bug |
| **F-10** | **P0** | 41 `RS.5` × `RS.6b` | `f402556b`, `4c99e772` | [`retrieval.rs:738-833, 920-922`](../../crates/tacsim/src/surface/retrieval.rs#L738-L833), [`house_arm.rs:236-244`](../../crates/tacsim/src/port/house_arm.rs#L236-L244) | Retrieval contracts posted with `destination = stickney_port` (`+salvage/retrieve <hull>=stickney go`) lapse at 24 IC h into `RetrievalStage::House` (Tycho accepts and locks poster's escrow), but `house_arm::job` filters `&order.destination == home` (`ceres_port`) — Tycho never flies the contract, locking the hull and escrow for 28 IC days. | Undocumented bug |
| **F-11** | **P0** | 41 `RS.6c-1` | `f94fb407` | [`house_sweep.rs:132-198`](../../crates/tacsim/src/port/house_sweep.rs#L132-L198), [`bay_work/mod.rs:2183-2196`](../../crates/tacsim/src/port/bay_work/mod.rs#L2183-L2196) | Swept hulls assessed as `Disposition::Keep` transfer title to `tycho_mutual` and remain berthed in Ceres workshop (`record.berth.is_some()`, `held_for.is_none()`). With `RS.6c-2` unbuilt, `Keep` hulls permanently consume finite Ceres workshop stands until every berth is taken (`"Every berth in the workshop is taken."`). | Undocumented side-effect of `RS.6c-2` deferral |
| **F-12** | **P1** | 41 `RS.6c-1` × 39 `IN.8` | `f94fb407`, `9c66c85c` | [`house_arm.rs:418-439`](../../crates/tacsim/src/port/house_arm.rs#L418-L439), [`strip.rs:406-531`](../../crates/tacsim/src/port/bay_work/strip.rs#L406-L531) | `scrap_at_home` calls `return_surplus(ctx, (float, house), credit)` in the **same tick** that `strip::scrap` queues an async `OwedKind::Mint`: either drains `float`'s pre-existing 20,000 cr sailing float before the workshop pays, or fails with `InsufficientFunds` when `credit > available(float)` and strands the scrap payout in `float` after the mint lands on the next tick. | Undocumented bug |
| **F-13** | **P1** | 41 `RS.5` / `RS.7` × `c558db35` | `c558db35` | [`security.rs:331-351, 366-368, 407-414`](../../crates/tacsim/src/surface/security.rs#L407-L414), [`retrieval.rs:1140-1177`](../../crates/tacsim/src/surface/retrieval.rs#L1140-L1177) | `mush_economy::reinstate_debt` transitions a `Delinquent` obligation back to `ObligationStatus::Active` for the *next* periodic sweep. Because `c558db35` changed `security::owed_minor` and `retrieval::delinquent_debts` to only check `ObligationStatus::Delinquent`, docking at Ceres/Stickney (`security::on_docked`) immediately lifts port quarantine (`holds == false`), and typing `+tv/lease` twice bypasses `desk_refusal`! | Undocumented regression in `c558db35` |
| **F-14** | **P1** | 42 `CR.2` × `3837c935` / `c558db35` | `3837c935`, `c558db35`, `cf162b21` | [`crew.rs:2641-2677`](../../crates/tacsim/src/field/crew.rs#L2641-L2677) | `create_release_obligation` sets `due_ic: ctx.w.world_ic_now().at` (`0s` grace instead of `retrieval::debt_due` = `now + 28 IC days`). Because `3837c935` added `"release:"` to `DEBT_SANCTION_PREFIXES`, an insolvent released captive transitions to `Delinquent` on the very next sweep and is immediately locked out of `+tv/lease` and `+coop/enrol` with zero 28-IC-day grace. | Violates 28-IC-day floor & `Q-42-7` |
| **F-15** | **P1** | 40 `b34208d8` | `b34208d8` | [`exit.rs:444-463, 548-591, 689-718`](../../crates/tacsim/src/field/exit.rs#L689-L718), [`boarding.rs:178-278`](../../crates/tacsim/src/field/boarding.rs#L178-L278) | Player ramp-exit boarding window: (1) `exit::tick` runs `continue_recover` before `boarding::tick`, starting surface/sortie/tow recover in the same tick `go_back` rejoins the hull into the field; (2) `go_back` leaves `unit` in `OwedExits`, `SiteRecord.promoted.exits`, and active `TowAct`; (3) refused `rejoin` in `carrier::on_joined` strands the hull on the ground with pilot unseated. | Undocumented bug |
| **F-16** | **P1** | 40 `FR.6` | `eb9b7359` | [`bubble.rs:244-249`](../../crates/tacsim/src/space/bubble.rs#L244-L249), [`party.rs:273-279, 302-313`](../../crates/tacsim/src/arena/party.rs#L273-L313), [`space/tick.rs:3985-4008`](../../crates/tacsim/src/space/tick.rs#L3985-L4008) | Replacing `SpacePartyId` with `arena::party::PartyId`: `LocalSpaceBubble::set_party_relation(..., None)` on `"INTERDICTOR_STAND_DOWN_CLEARED"` calls `Relations::mark(..., None)`, which clears `book.marks` but **not** `book.fired`. If any shot was fired before stand-down, `party_relation` stays `Hostile` and the interdiction bubble never resolves. | Contradicts `40-cut-notes.md:839-891` ("No rule changes in space") |
| **F-17** | **P1** | 39 `IN.3` / `IN.9` | `cb5b2934`, `27da64e7` | [`works.rs:1323-1340, 2017-2043, 2486-2491`](../../crates/tacsim/src/port/works.rs#L2017-L2043) | `charge_storage` seizure (`Then::Seized`) on an uncollected completed yard order (`tacsim.hull_kit.*`) puts the kit lot into `WorksAct.lots`, which `WorksProvider::prepare_authoritative` rejects (`!is_part(lot.commodity())` → `"a bulk lot is pooled before it moves"`), causing an infinite 4-tick seizure retry loop and player message spam. | Undocumented bug |
| **F-18** | **P1** | 39 `IN.9` | `27da64e7`, `ec95eb48` | [`works.rs:659-715, 871-877`](../../crates/tacsim/src/port/works.rs#L659-L715), [`bill.rs:29, 67, 100`](../../crates/tacsim/src/port/bill.rs#L29-L100) | (a) `+yard/quote <design>` calls `bill_of` instead of `bill_with_parts`, omitting all component parts from the quote; (b) `bill_with_parts` never includes `DesignBill.core`, so cored hulls (`tv_escort`, `line_tank`, `dray`) are built and commissioned with free installed cores. | Violates Doc 39 §4.7 & §6 `IN.9` |
| **F-19** | **P1** | 39 `IN.7` / `IN.8` | `1f925b86`, `9c66c85c` | [`stock.rs:1263-1301`](../../crates/tacsim/src/port/stock.rs#L1263-L1301) | `part_ask_step` only probes `seed_lot_id(&id, n)` for `n in 0..stock.qty` instead of `storefront_lots`; `Manufactured`, `Scrapped`, and player-sold part lots in a vendor's storefront are never listed on market asks. | Contradicts Doc 39 §4.6 / `IN.7` |
| **F-20** | **P1** | 41 `RS.1` | `2585edba` | [`tender.rs:226-235, 273-276, 576-578, 1278-1300`](../../crates/tacsim/src/surface/tender.rs#L226-L235) | `TendWork::Stabilize` checks `budget.fittings_kg_for(points)` (`(fittings_kg * points).div_ceil(total_points)`, e.g. `143 kg`), but `run_library` passes `library_kg_per_point = budget.fittings_kg_for(1)` (`1 kg/pt`) to `field_stabilize_damaged_section` (`points * 1 = 500 kg`), so stabilization starts, runs 15 min, and fails at `finish` with `TendStep::Lost` (`InsufficientSpareParts`)! | Undocumented bug |
| **F-21** | **P1** | 41 `RS.4` / `RS.5` / `RS.6c-1` | `f402556b`, `f94fb407` | [`claim.rs:1004, 1540-1608`](../../crates/tacsim/src/surface/claim.rs#L1004), [`retrieval.rs:905-913`](../../crates/tacsim/src/surface/retrieval.rs#L905-L913), [`house_sweep.rs:248-274`](../../crates/tacsim/src/port/house_sweep.rs#L248-L274), [`tv.rs:5084-5093`](../../crates/tacsim/src/port/tv.rs#L5084-L5093) | Salvage claim & custody edge bugs: (a) `right_of_grantor` (`+salvage/contest <hull>=right`) never checks `RS.5` retrieval contracts; (b) `apply_space_claim_order` & `retrieve_cmd` reject org officers (`answers_for` / `acts_for`); (c) vacating a claim on a hull already in Ceres workshop or on a site during a Tycho sweep strands the hull in workshop (`berth.is_none()`) or strands `lifter` at the site; (d) `write_off_left_out` teleports leased rigs out of foreign salvor carrier bays in deep space (`Q-41-14`). | Violates `Q-41-14`, `Q-41-19`, `Q-41-23` |
| **F-22** | **P1** | 42 `CR.1`–`CR.6` × 39 `IN.9` | `66a88181`, `0a08d078`, `f64935c9` | [`crew.rs:413-424, 2296`](../../crates/tacsim/src/field/crew.rs#L413-L424), [`tow.rs:1010-1048`](../../crates/tacsim/src/surface/tow.rs#L1010-L1048), [`space/custody.rs:1352, 1410`](../../crates/tacsim/src/space/custody.rs#L1352-L1413), [`detachment/commands.rs:304-315`](../../crates/tacsim/src/detachment/commands.rs#L304-L315) | Crew custody & gate bugs: (a) active `ComsysState` alias (`dt <msg>`) bypasses `Gate::Chat`; (b) `mine_cmd` and `+sortie/unseat`, `accept`, `ready` bypass `crew::gate`; (c) loading/hard-docking a friendly hull into a yard-commissioned carrier (`tacsim.owned_hulls`) treats it as **hostile**, captures friendly crew, and deducts `-10` standing; (d) `may_extract` allows any DT wingman or rescued friendly casualty on a carrier (even in deep space) to `+sortie/extract` to port. | Violates `Q-42-2`, `Q-42-4`, `Q-42-8`, `Q-42-15` |
| **F-23** | **P1** | 23 `S.4b-2` / `S.4b-3` | `911897b5`, `55753484` | [`close.rs:486-517`](../../crates/tacsim/src/field/close.rs#L486-L517), [`surface/tick.rs:416-426, 658-660`](../../crates/tacsim/src/surface/tick.rs#L658-L660), [`recovery.rs:1019-1102`](../../crates/tacsim/src/surface/recovery.rs#L1019-L1102), [`tv.rs:5680-5740`](../../crates/tacsim/src/port/tv.rs#L5680-L5740) | Additional `S.4b-2`/`S.4b-3` lifecycle wedges: (a) `wreck_bay_rigs` (`GC.3`) is refused by `ic_custody::refuse_mutation` on claimed bay rigs; (b) `claim_attempt` shares one retry/backoff key across `open_ic_mind`, `submit_launch`, and `close_ic_mind`; (c) `+tv/<n> launch` is allowed onto a promoted site (`ramp_handoff`), stranding the rig on the ramp outside the running field; (d) staff `RecoveryOperation::Close`/`Home` during `Opening` or 30s `Launch` wedges `ClaimCursor`; (e) `rig_flight` ignores `record.claim` (`Opening`/`Closing`), allowing carrier liftoff (`BootSpatialViolation::CarrierInFlight` on reboot) or failed lease return. | Undocumented bugs |
| **F-24** | **P1** | 42b `SU.0`–`SU.6` | `57f4c7e6` | [`42b-surrender.md:208-215, 364-370, 458-463, 540-573`](42b-surrender.md#L208-L215) | Four spec-vs-code contradictions in `42b-surrender.md` (committed at `HEAD`) against current `exit.rs`, `close.rs`, `crew.rs`, and `tow.rs` that will break `SU.1` and `SU.3` if implemented as written. | Spec-vs-code traps in `42b-surrender.md` |

---

## Section 1: Confirmed P0 & P1 Production Bugs & Exploits

### F-01 [P0] — `ArenaMsg::Join` Hardcodes `source: None` and Omits `IcHost::resume` / `seed_carried_mind`, Causing an Infinite 1-Hz Checkout/Refuse/Checkin Loop When a Claimed-Mind Rig Rejoins After Carrier Loss (`S.4b-3` × `FR.5b` × `D-23-18`)
- **Commits**: `911897b5` (`S.4b-2`), `55753484` / `98b7cb10` (`S.4b-3`), `049fbeb3` (`FR.5`), `870d27cb` (`FR.5b`), `0c3a6d68` (`D-23-18`), `b34208d8` (ramp boarding window)
- **Design Citations**: [Doc 23](23-ic-sorties.md#L1221-L1311) (§17.4 `D-23-23` step 3, §17.5 `D-23-17`, §17.7 `D-23-18`), [Doc 40](40-field-second-round.md#L131-L163) (§4.4 `FR.3`, `Q-40-19`).
- **Locations**:
  - [`join_with`](../../crates/tacsim/src/field/carrier.rs#L580-L602), [`joining_as`](../../crates/tacsim/src/field/carrier.rs#L623-L644), [`join_launches`](../../crates/tacsim/src/field/carrier.rs#L822-L827), and [`on_joined`](../../crates/tacsim/src/field/carrier.rs#L945-L966) in [`crates/tacsim/src/field/carrier.rs`](../../crates/tacsim/src/field/carrier.rs)
  - [`raiders::join`](../../crates/tacsim/src/field/raiders.rs#L1211-L1220) in [`crates/tacsim/src/field/raiders.rs`](../../crates/tacsim/src/field/raiders.rs)
  - [`custody_unit`](../../crates/tacsim/src/field/admit.rs#L780-L799) in [`crates/tacsim/src/field/admit.rs`](../../crates/tacsim/src/field/admit.rs)
  - [`Task::join`](../../crates/tacsim/src/arena_task.rs#L2642-L2674) in [`crates/tacsim/src/arena_task.rs`](../../crates/tacsim/src/arena_task.rs)
  - [`Arena::join_in_place`](../../crates/tacsim/src/arena/join.rs#L164-L167) in [`crates/tacsim/src/arena/join.rs`](../../crates/tacsim/src/arena/join.rs)
- **Root Cause & Cross-PR Finger-Pointing**:
  - `S.4b-3` ([`23-cut-notes.md:499-502`](23-cut-notes.md#L499-L502)) stated that the host half of `ArenaMsg::Join` (`carrier.rs` / `raiders.rs` passing `source: Option<MindSource>`) would land in `FR.5` (`#410`). Conversely, `FR.5` ([`40-cut-notes.md:97`](40-cut-notes.md#L97)) stated that `S.4b-3` owned that edit. Both merged, and [`carrier::join_with`](../../crates/tacsim/src/field/carrier.rs#L597) still sends `source: None`!
  - Meanwhile, `S.4b-2`/`S.4b-3` wired [`admit::custody_unit`](../../crates/tacsim/src/field/admit.rs#L793-L799) so that any hull with an open mind claim (`entrant.claim == Some(open)`) receives `unit.seal = claim_seal` (`SealEngine::Scripted`).
- **Concrete Reproduction Path**:
  1. A rig launched under a filed mind claim (`+tv/1 launch mind=filed:1 go`) is admitted into a field at promotion via `boot::open_for` (which works because `boot::open_for` passes `ic_retained` and `carried_minds`).
  2. During the field, the rig withdraws at its carrier's ramp (`+field/exit` or stock mind withdrawal), entering the 60s ramp boarding window ([`boarding.rs`](../../crates/tacsim/src/field/boarding.rs#L212-L265)).
  3. Before the 60s boarding window completes, the carrier is wrecked or lifts off (`boarding::lost`).
  4. On the next 1s field tick, [`boarding::tick`](../../crates/tacsim/src/field/boarding.rs#L249) calls `go_back` → `carrier::rejoin` → `join_with` → `joining_as` → `admit::custody_unit`.
  5. `custody_unit` stamps `unit.seal = claim_seal` (`SealEngine::Scripted`), checks the rig out of `mush_sim`, and `join_with` sends `ArenaMsg::Join { ..., source: None }`.
  6. [`Arena::join_in_place`](../../crates/tacsim/src/arena/join.rs#L164-L167) refuses: `"a scripted seal with no retained source"`.
  7. [`carrier::on_joined`](../../crates/tacsim/src/field/carrier.rs#L945-L966) checks the rig back into `mush_sim` and removes it from `live.units`. For an AI/stock-withdrawn rig (where `boarding::lost` in `exit.rs:442` keeps the row in `Boarding`), the very next 1s tick repeats `carrier::rejoin` → checkout → refuse → checkin in an **infinite 1-Hz loop** until the field closes!
  8. Even if `source` were passed, [`Task::join`](../../crates/tacsim/src/arena_task.rs#L2656-L2667) calls `install_sealed_runner` (strict OOC installer instead of `IcHost::resume(...).install(...)` per `D-23-17`) and never calls `arena.seed_carried_mind` (losing `D-23-18` carried memory, `think_seq`, and `dropped` state).
  9. Additionally, [`carrier::join_launches`](../../crates/tacsim/src/field/carrier.rs#L824-L827) still contains the pre-`S.4b-2` early `continue` on `LaunchSeal::Claim(open)`, stranding any mid-field detachment launch flying a claimed mind outside the running field forever.

---

### F-02 [P0] — `NeverLeft` Launch After `open_ic_mind` Leaks the IC Claim, Custody, Exclusion, and Roll Licence Seat, and Permanently Bricks the Rig (`S.4b-2` / `S.4b-3`)
- **Commits**: `911897b5` (`S.4b-2`), `55753484` (`S.4b-3`)
- **Design Citations**: [Doc 23](23-ic-sorties.md#L1036-L1037) (§17.3 & §17.4 `D-23-23` step 5: *"If the launch is cancelled or fails before the rig leaves the bay (`NeverLeft`), the excursion closes `PoweredDownInBase { base: <the hangar's hull> }`"*), [`23-cut-notes.md:205, 417-422`](23-cut-notes.md#L417-L422).
- **Locations**:
  - [`finish` (`ActKind::Launch`)](../../crates/tacsim/src/surface/act/providers.rs#L3329-L3346) and [`epoch_move`](../../crates/tacsim/src/surface/act/providers.rs#L2951-L2956) in [`crates/tacsim/src/surface/act/providers.rs`](../../crates/tacsim/src/surface/act/providers.rs)
- **Concrete Reproduction Path**:
  1. Player queues `+tv/1 launch mind=filed:1 go`.
  2. Fire 1: `surface_tick` submits `open_ic_mind`, opening the `AnchorImage::Hangar` claim in `IcSorties.active`, `IcCustodies.active`, `IcExclusions.active`, and holding the Roll licence seat.
  3. Fire 2: `surface_tick` submits `tacsim_surface_act` (`claim: "launch"`, [`providers.rs:1235-1246`](../../crates/tacsim/src/surface/act/providers.rs#L1235-L1246)), which moves `record.claim` to `Some(ClaimCursor::Open { open, operator })`, clears `record.queued = None`, starts `record.act = Some(ActKind::Launch)`, and sets `record.excursion = Some(...)`.
  4. When the 30s launch timer expires, `world_step` ([`surface/tick.rs:1525-1565`](../../crates/tacsim/src/surface/tick.rs#L1525-L1565)) cannot place the rig on the site's ground (e.g., another arrival filled `site_population >= max_units` at line 1532, or the carrier is no longer berthed on a site).
  5. In [`finish`](../../crates/tacsim/src/surface/act/providers.rs#L3329-L3346), `beside` is `false` (`rig.facts` is still `Facts::Bay { hull }`), so line 3341 sets `record.excursion = None` and returns `ActResult::Refused { code: RefusedCode::NeverLeft }`.
  6. `finish` then calls [`epoch_move`](../../crates/tacsim/src/surface/act/providers.rs#L2951-L2956). Because `record.excursion` was just cleared to `None` and `rig.facts` is `Facts::Bay`, `epoch_move` returns `Ok(None)` at line 2953 without opening `record.sortie` and **without calling `record.owe_claim_close(PoweredDownInBase { base: act.hull })`**.
  7. `record.claim` remains stuck in `Some(ClaimCursor::Open)` with `record.sortie == None`, `record.excursion == None`, `record.act == None`, and `record.queued == None`. No tick or finish path ever transitions `ClaimCursor::Open -> ClaimCursor::Closing` when `record.sortie` is `None`. The IC claim and Roll licence seat are leaked forever, and the rig can never launch again (`CLAIM_STILL_OPEN`).

---

### F-03 [P0] — Claimed Hull Destroyed (`UnitOutcome::Irrecoverable`) in a Field Is Refused by `mush_sim::unplace` (`ic_exclusion::check_plan`), Leaving a Ghost Hull on the Site (`S.4b-2` / `S.4b-3`)
- **Commits**: `911897b5` (`S.4b-2`), `b31469c5` (`S.4b-3`)
- **Design Citations**: [Doc 23](23-ic-sorties.md#L1230-L1233) (§17.4 `D-23-23` step 5: when a claimed hull is destroyed in a field, its excursion and claim close `Destroyed`).
- **Locations**:
  - [`on_ended`](../../crates/tacsim/src/field/close.rs#L95-L145) and [`demote_units`](../../crates/tacsim/src/field/close.rs#L314-L341) in [`crates/tacsim/src/field/close.rs`](../../crates/tacsim/src/field/close.rs)
  - [`prepare_close`](../../crates/tacsim/src/field/provider.rs#L279-L329) in [`crates/tacsim/src/field/provider.rs`](../../crates/tacsim/src/field/provider.rs)
  - [`check_boot_spatial`](../../crates/mush-sim/src/ic_exclusion.rs#L1137-L1153) and [`hangar_move_admitted`](../../crates/mush-sim/src/ic_exclusion.rs#L1233-L1235) in [`crates/mush-sim/src/ic_exclusion.rs`](../../crates/mush-sim/src/ic_exclusion.rs)
- **Concrete Reproduction Path**:
  1. A claimed rig (`AnchorImage::Hangar`, held in `IcExclusions.active`) enters a field and suffers a catastrophic kill (`UnitOutcome::Irrecoverable`).
  2. When the field ends, [`on_ended`](../../crates/tacsim/src/field/close.rs#L95-L145) runs synchronously during `field::tick` ([`admit.rs:212`](../../crates/tacsim/src/field/admit.rs#L212)) **before** `owe_close` submits `tacsim_field op=close` ([`admit.rs:236`](../../crates/tacsim/src/field/admit.rs#L236)), which in turn only marks `rig.owe_claim_close(Destroyed)` ([`provider.rs:316`](../../crates/tacsim/src/field/provider.rs#L316)) for a later `surface_tick` to submit `close_ic_mind`.
  3. Inside `on_ended`, [`demote_units`](../../crates/tacsim/src/field/close.rs#L318-L324) handles `UnitOutcome::Irrecoverable` by calling `mush_sim::unplace(ctx, *unit, ...)`.
  4. `mush_sim::unplace` runs `ic_exclusion::check_plan` → [`hangar_move_admitted`](../../crates/mush-sim/src/ic_exclusion.rs#L1233-L1235), which enforces `if next.unplaced.is_some() { return false; }`.
  5. Because the hull's IC claim is still active in `IcExclusions.active` when `demote_units` runs, `mush_sim::unplace` fails with `SpatialError::IcCustody(unit)`, logging `"tacsim: an irrecoverable hull could not be unplaced"` and leaving the destroyed hull placed on the site's ground (`Where::Ground`) alongside its `LeftRemains` debris pile forever!

---

### F-04 [P0] — `mush_sim::deployed::excluded` Treats Every Surface Rig With an Open IC Claim as "Deployed in an Arena", Hiding It From `look`, `+map`, `atlas.map`, `@unit/show`, `tactics.vitals`, and World Sensor Propagation (`S.4b-T` / `S.4b-2`)
- **Commits**: `b0279b26` (`S.4b-T`), `911897b5` (`S.4b-2`)
- **Design Citations**: [Doc 23](23-ic-sorties.md#L891-L1250) (§17.1–17.4: a rig with an `AnchorImage::Hangar` claim is a normal world/surface participant while in the bay and while driving/mining on the surface outside a field; it is only checked out into an arena while a field is open on its site).
- **Locations**:
  - [`is_custodied`](../../crates/mush-sim/src/deployed.rs#L248-L250) and [`excluded`](../../crates/mush-sim/src/deployed.rs#L300-L302) in [`crates/mush-sim/src/deployed.rs`](../../crates/mush-sim/src/deployed.rs)
  - [`PlacementGate::perceives`](../../crates/mush-sim/src/lib.rs#L486) in [`crates/mush-sim/src/lib.rs`](../../crates/mush-sim/src/lib.rs)
  - [`participants`](../../crates/mush-sim/src/window.rs#L425-L434) in [`crates/mush-sim/src/window.rs`](../../crates/mush-sim/src/window.rs)
  - [`unit_source`](../../crates/mush-tactics/src/view.rs#L108) in [`crates/mush-tactics/src/view.rs`](../../crates/mush-tactics/src/view.rs) and [`unit_status`](../../crates/mush-tactics/src/status.rs#L180) in [`crates/mush-tactics/src/status.rs`](../../crates/mush-tactics/src/status.rs)
  - [`held_by_an_arena`](../../crates/mush-propagation/src/pass.rs#L715) in [`crates/mush-propagation/src/pass.rs`](../../crates/mush-propagation/src/pass.rs)
- **Concrete Reproduction Path**:
  1. In `S.4b-T` (`b0279b26`), [`excluded(w, Shown::Unit(unit))`](../../crates/mush-sim/src/deployed.rs#L300-L302) was changed from `is_deployed(w, unit)` to `is_custodied(w, unit)` (`is_deployed(w, unit) || ic_exclusion::identity_of(w, unit).is_some()`).
  2. For a surface rig (`AnchorImage::Hangar`), `ic_exclusion::identity_of(w, rig)` is `Some` for the **entire surface excursion** — from `open_ic_mind` (in the carrier bay before launch) through all peaceful surface driving, scanning, and mining (when `is_deployed` is `false` and no field exists) until `close_ic_mind` commits after recovery.
  3. Consequently, throughout the rig's entire surface excursion outside a field:
     - `PlacementGate::perceives` returns `Perceive::Conceal`, hiding the rig from `look` and room contents.
     - `mush_sim::participants` filters the rig out, so `+map` and `atlas.map` omit the rig from the surface map.
     - `mush_tactics::view::unit_source` returns `None`, so `@unit/show`, `tactics.unit`, and `tactics.vitals` refuse/omit the rig.
     - `mush_tactics::status::unit_status` returns `UnitStatus::deployed()` (reporting the rig as deployed in a live arena).
     - `mush_propagation::pass::held_by_an_arena` returns `true`, excluding the surface rig from world sensor propagation.

---

### F-05 [P0] — Written-Off Leased Rig With an Open IC Claim Cannot Be Loaded by Tow (`place_in_bay` → `tv::hand_back` Refused by `ic_exclusion::check_plan`), Failing the Load and Leaking the Claim (`S.4b-2` × `RS.2`)
- **Commits**: `911897b5` (`S.4b-2`)
- **Design Citations**: [Doc 23](23-ic-sorties.md#L956-L998) (§17.2 `D-23-21` & §17.4 `D-23-23` step 5).
- **Locations**:
  - [`tow::tick`](../../crates/tacsim/src/surface/tow.rs#L3104), [`place_in_bay`](../../crates/tacsim/src/surface/tow.rs#L3212-L3220), [`prepare` (`TowOrder::Loaded`)](../../crates/tacsim/src/surface/tow.rs#L1666-L1689), and [`complete_load`](../../crates/tacsim/src/surface/tow.rs#L1863-L1868) in [`crates/tacsim/src/surface/tow.rs`](../../crates/tacsim/src/surface/tow.rs)
  - [`hand_back`](../../crates/tacsim/src/port/tv.rs#L4898-L4917) in [`crates/tacsim/src/port/tv.rs`](../../crates/tacsim/src/port/tv.rs)
  - [`hangar_move_admitted`](../../crates/mush-sim/src/ic_exclusion.rs#L1261-L1267) in [`crates/mush-sim/src/ic_exclusion.rs`](../../crates/mush-sim/src/ic_exclusion.rs)
- **Concrete Reproduction Path**:
  1. A leased rig with an open IC claim (`AnchorImage::Hangar`) is disabled/abandoned on a site for 28 IC days; `write_off_left_out` ([`port/tv.rs:5055-5083`](../../crates/tacsim/src/port/tv.rs#L5055-L5083)) marks its lease `written_off: true` while leaving it on the site's ground.
  2. A tug attaches to the casualty and runs `+tv/<n> load go` at a carrier's ramp.
  3. When the 30s load completes, `tow::tick` calls [`place_in_bay`](../../crates/tacsim/src/surface/tow.rs#L3104) **before** submitting `TowOrder::Loaded` (which calls `complete_load` → `rig.owe_claim_close` at line 1865).
  4. Inside [`place_in_bay`](../../crates/tacsim/src/surface/tow.rs#L3212-L3220), because `written_off` is `true`, it skips placing the casualty in the carrier's bay and instead calls [`tv::hand_back`](../../crates/tacsim/src/port/tv.rs#L4898-L4917), which calls `mush_sim::place_interior(ctx, tv, shed, None)`.
  5. Occator's `shed` is anchored to a `FixedDomain` (not `SpaceAnchor::MovingObject`), so while the IC claim is still open in `IcExclusions.active`, [`hangar_move_admitted`](../../crates/mush-sim/src/ic_exclusion.rs#L1261-L1267) refuses `place_interior` with `SpatialError::IcCustody(casualty)`!
  6. Both `in_bay` and `handed_back` are `false`. When `TowOrder::Loaded` runs ([`tow.rs:1666-1689`](../../crates/tacsim/src/surface/tow.rs#L1666-L1689)), it sees `!in_bay && !handed_back`, records `"refused:it never reached the bay"`, and returns early **without** calling `complete_load` or `owe_claim_close`. The casualty cannot be loaded into any bay and its IC claim stays open forever.

---

### F-06 [P0] — Enrolling Onto a Yard-Built Trust Dray (`IN.10`) Brick-Locks the 9th Captain (`helm_of == None`) and Breaks `in10_pools::stocked` on `origin/main` (`IN.10` × `cf162b21`)
- **Commits**: `6733ca1d` (`IN.10`), `cf162b21` (`#475`, 28-IC-day player-response floor)
- **Design Citations**: [Doc 39](39-industry-production-chain.md#L480-L515) (§4.8 `IN.10`, `Q-39-22`).
- **Locations**:
  - [`live_charter`](../../crates/tacsim/src/port/coop.rs#L545-L569), [`helm_holder`](../../crates/tacsim/src/port/coop.rs#L699-L721), [`free_hulls`](../../crates/tacsim/src/port/coop.rs#L792-L807), [`check_hull_name`](../../crates/tacsim/src/port/coop.rs#L972), and [`row_of`](../../crates/tacsim/src/port/coop.rs#L1068) in [`crates/tacsim/src/port/coop.rs`](../../crates/tacsim/src/port/coop.rs)
  - [`stocked`](../../crates/tacsim/tests/domain_economy/in10_pools.rs#L411-L440) in [`crates/tacsim/tests/domain_economy/in10_pools.rs`](../../crates/tacsim/tests/domain_economy/in10_pools.rs)
- **Concrete Reproduction Path**:
  1. `IN.10` (`6733ca1d`) updated [`free_hulls`](../../crates/tacsim/src/port/coop.rs#L792-L807) to chain `crate::port::owned::rows(w)` after `port.map.hulls.values()`, so when all 8 pack-defined Co-op Drays are lent out and the yard completes a replacement `dray` for `ceres_merchant_coop` (`tacsim.owned_hulls`), `+coop/enrol` lends that 9th Dray to the 9th captain (charging their 25,000 cr stake and consuming their one lifetime enrolment).
  2. However, `IN.10` **forgot** to update [`live_charter`](../../crates/tacsim/src/port/coop.rs#L557-L561), [`helm_holder`](../../crates/tacsim/src/port/coop.rs#L711), [`check_hull_name`](../../crates/tacsim/src/port/coop.rs#L972), and [`row_of`](../../crates/tacsim/src/port/coop.rs#L1068), all of which still only scan `port.map.hulls` and ignore `tacsim.owned_hulls`!
  3. Consequently, for the 9th captain on the yard-built Trust Dray, `live_charter` and `helm_of` return `None`: `+ship`, `+market`, `+coop/refit`, `+coop/buyout`, and `+coop/release` all refuse (`"You do not hold a Co-op charter."`), while `+coop/enrol` refuses with `"The Co-op enrols a captain once..."` — permanently bricking the character!
  4. **Why `in10_pools::stocked` passed in `6733ca1d` and now panics on `origin/main` (`57f4c7e6`)**:
     - In `6733ca1d`, `DORMANCY_SECS` was 7 IC days. While the test ticked ~21 IC days for the yard to build the 9th Dray, `captains[2..8]` went dormant after 7 IC days and had their pack-defined Drays (`dray_3..8`) recalled, so `captains[8]` accidentally enrolled onto recalled pack-defined `dray_3` instead of the yard-built 9th Dray!
     - When commit `cf162b21` (`#475`) raised `DORMANCY_SECS` from 7d to 28d, `captains[2..8]` stayed active, so `captains[8]` actually enrolled onto the yard-built 9th Dray for the first time — and `in10_pools::stocked` now **panics** at [`in10_pools.rs:430`](../../crates/tacsim/tests/domain_economy/in10_pools.rs#L430) (`ninth captain takes the hull the yard built for the Trust`).

---

### F-07 [P0] — Complete Part Provenance, Legality (`Hot`/`Disputed`), Condition (`Damaged`/`Jammed`), and Lot-Hold Bypass in `+yard/deliver` and `commission` (`IN.7` / `IN.9`)
- **Commits**: `1f925b86` (`IN.7`), `27da64e7` (`IN.9`)
- **Design Citations**: [Doc 39](39-industry-production-chain.md#L422-L478) (§4.7 `IN.9`: *"Every part lot is a singleton (`PartProvenance`); the yard accepts a clean or cleared lot and refuses a hot or disputed one"*), [`39-cut-notes.md:143-145, 232`](39-cut-notes.md#L143-L145) (`IN.7` exported `bay_work::disputed_refusal` specifically for `IN.9`).
- **Locations**:
  - [`deliver`](../../crates/tacsim/src/port/works.rs#L1161-L1184), [`escrow_lots`](../../crates/tacsim/src/port/works.rs#L1261-L1321), and [`WorksProvider::consume`](../../crates/tacsim/src/port/works.rs#L2506-L2534) in [`crates/tacsim/src/port/works.rs`](../../crates/tacsim/src/port/works.rs)
  - [`disputed_refusal`](../../crates/tacsim/src/port/bay_work/mod.rs#L88) in [`crates/tacsim/src/port/bay_work/mod.rs`](../../crates/tacsim/src/port/bay_work/mod.rs)
  - [`commission_cmd`](../../crates/tacsim/src/port/commission.rs#L88-L115) in [`crates/tacsim/src/port/commission.rs`](../../crates/tacsim/src/port/commission.rs)
- **Concrete Reproduction Path**:
  1. `bay_work::disputed_refusal` ([`bay_work/mod.rs:88`](../../crates/tacsim/src/port/bay_work/mod.rs#L88)) has **zero callers outside `bay_work`**.
  2. When a player runs `+yard/deliver <order> go` for a hull order (`tacsim.hull_kit.*`), [`deliver`](../../crates/tacsim/src/port/works.rs#L1161-L1184) selects part lots from the player's hold via [`escrow_lots`](../../crates/tacsim/src/port/works.rs#L1261-L1321) without ever reading `tacsim.bay_parts` (`PartProvenance`) or `mush_inventory::is_lot_held`:
     - **`Hot` & `Disputed` parts accepted**: Stolen (`PartLegality::Hot`) and contested (`PartLegality::Disputed`) parts are accepted into hull builds and laundered into a clean `tacsim.hull_kit.*`.
     - **`Damaged` & `Jammed` parts laundered into `Intact` hulls**: `deliver` does not check `prov.condition == ComponentStatus::Intact`, and `commission.rs` calls `mush_tactics::create_unit` (which initializes every component to `Intact`), laundering broken `Damaged`/`Jammed` scrap parts into a pristine hull without paying workshop repair fees.
     - **Missing `fitted_into` update & `PartProvenance` check**: Neither `WorksProvider::consume` nor `commission.rs` sets `fitted_into = Some(hull)` on `tacsim.bay_parts`, and `escrow_lots` does not reject lots without a `PartProvenance` record, lots with `fitted_into.is_some()`, or lots held by `lot_hold`.

---

### F-08 [P0] — Launching a Carrier's First Detachment Flight Into a Running Field (`FR.3`) Breaks Carrier–Detachment `Friend` Relations, Stance Opening (`FR.1`), and Leader's `+field/mark` (`814f9ea1`)
- **Commits**: `22994b6a` (`FR.3`), `0fd2430b` (`FR.1`), `814f9ea1` (`+field/mark`)
- **Design Citations**: [Doc 38](38-after-the-ic-phase-next-systems.md#L455-L459) (`Q-38-25`), [Doc 40](40-field-second-round.md#L92-L163) (§4.2 `Q-40-9..11`, §4.4 `Q-40-19`), [`40-cut-notes.md:142-150`](40-cut-notes.md#L142-L150) (leader's `+field/mark` marks for all hulls under their command).
- **Locations**:
  - [`led_now`](../../crates/tacsim/src/field/parties.rs#L321-L326), [`carrier_entrant`](../../crates/tacsim/src/field/parties.rs#L431-L447), [`friends`](../../crates/tacsim/src/field/parties.rs#L538-L561), and [`party_sides`](../../crates/tacsim/src/field/parties.rs#L601-L618) in [`crates/tacsim/src/field/parties.rs`](../../crates/tacsim/src/field/parties.rs)
  - [`joining_as`](../../crates/tacsim/src/field/carrier.rs#L413-L526) in [`crates/tacsim/src/field/carrier.rs`](../../crates/tacsim/src/field/carrier.rs)
  - [`mark_in_field`](../../crates/tacsim/src/field/commands.rs#L354-L364) in [`crates/tacsim/src/field/commands.rs`](../../crates/tacsim/src/field/commands.rs)
- **Concrete Reproduction Path**:
  1. A carrier is on the site pad when a field promotes (or settles mid-field via `F-6` `carrier::join_arrivals`) **before** any detachment flight from that carrier is on the ground.
  2. [`carrier_entrant`](../../crates/tacsim/src/field/parties.rs#L431-L447) finds no open flight on the ground from `carrier` (`roster = None`), so the carrier is admitted to `live.parties` under `PartyKey::Unit { unit: carrier }` (party `B`), led by commander `P`.
  3. Commander `P` now launches slot 1 of their operation into the running field via `+sortie/launch <op>=1` (`FR.3`).
  4. When the launch lands, `carrier::join_launches` calls [`joining_as`](../../crates/tacsim/src/field/carrier.rs#L413-L526), which recomputes `parties::entrants(ctx, site, &units)` over all field units.
  5. Because slot 1 is now on the ground with an open flight from `carrier`, [`carrier_entrant`](../../crates/tacsim/src/field/parties.rs#L438-L445) now returns `key: PartyKey::Operation { operation }` for the carrier instead of its actual key in `live.parties` (`PartyKey::Unit { unit: carrier }`).
  6. `joining_as` allocates a new party `C` (`PartyKey::Operation { operation }`, also led by `P`) for the launched unit. Now in `entrants`, **zero** entrants have `key == PartyKey::Unit { unit: carrier }` (`B`), causing three failures:
     - **(a) Launched detachment is `Neutral` (not `Friend`) to its own carrier**: [`parties::friends`](../../crates/tacsim/src/field/parties.rs#L538-L561) maps entrants via `letter_of(&entrant.key)`, so party `B` never appears in `by_affiliation` and `(B, C)` is never emitted in `joining.host.friends`.
     - **(b) `party_sides` loses the carrier's affiliations**: [`party_sides`](../../crates/tacsim/src/field/parties.rs#L601-L618) leaves party `B` with an empty `BTreeSet`, so subsequent entrants fail to open declared `Hostile` / `Allied` stances with the carrier.
     - **(c) Leader's `+field/mark` fails to mark in the launched `Operation` party's book**: When commander `P` (aboard the carrier, while an NPC crewmate or stock mind flies the launched rig) runs `+field/mark X=hostile`, [`led_now`](../../crates/tacsim/src/field/parties.rs#L321-L326) uses `.find(...)` and returns only the **first** party led by `P` (`B`, the unarmed carrier), and [`mark_in_field`](../../crates/tacsim/src/field/commands.rs#L354-L364) chains `once(B)` with `seated_parties` (cockpit seats only). Party `C` (the `Operation` party `P` leads!) never receives the mark, so `P`'s detachment rigs never engage `X`!

---

### F-09 [P0] — Tycho Recovery Arm (`RS.6b` × `RS.6c-1`) Unconditional Preemption of `sweep_act` by `job` Mid-Sweep Deadlocks `lifter` and `crawler` Forever
- **Commits**: `4c99e772` (`RS.6b`), `f94fb407` (`RS.6c-1`)
- **Design Citations**: [Doc 41](41-restoration-and-salvage-loop.md#L331-L370) (§4.7 `RS.6b`, §4.8 `RS.6c`).
- **Locations**:
  - [`next_act`](../../crates/tacsim/src/port/house_arm.rs#L199-L232) and [`house_step`](../../crates/tacsim/src/port/house_arm.rs#L333-L343) in [`crates/tacsim/src/port/house_arm.rs`](../../crates/tacsim/src/port/house_arm.rs)
- **Concrete Reproduction Path**:
  1. In [`house_step`](../../crates/tacsim/src/port/house_arm.rs#L333-L340):
     ```rust
     let act = if let Some((contract, order)) = job(ctx.w, &home) {
         Some(next_act(ctx, (lifter, crawler), &order, contract))
     } else {
         sweep_act(ctx, (lifter, crawler), &home)
     };
     ```
  2. Tycho's `lifter` and `crawler` are in the middle of a multi-tick `RS.6c-1` sweep on site `S1` (e.g., `crawler` is deployed on `S1`'s ground driving or towing a swept wreck).
  3. Any player retrieval contract lapses its 24-IC-hour open window and transitions to `RetrievalStage::House`, so `job(ctx.w, &home)` returns `Some(...)` and shadows `sweep_act`:
     - **Case A (Retrieval casualty is on site `S2 != S1`)**: [`next_act`](../../crates/tacsim/src/port/house_arm.rs#L199-L205) sees `lifter_at != Some(&site)` (`S1 != S2`). Because `crawler_in_bay` is `false` (`crawler` is on `S1`'s ground), the `if crawler_in_bay` branch is skipped, and line 204 returns `HouseAct::Wait` on **every tick forever** (since `sweep_act`, which would recover `crawler`, is never called while `job()` is `Some`).
     - **Case B (Retrieval casualty is on the same site `S1` while `crawler` is already towing the swept wreck)**: [`next_act`](../../crates/tacsim/src/port/house_arm.rs#L219-L229) drives `crawler` to the retrieval casualty and returns `HouseAct::Tow { casualty }`, which refuses on every tick (`Step::Refused`) because `crawler` is already coupled to the swept wreck!

---

### F-10 [P0] — Retrieval Contracts With `destination = stickney_port` Are Accepted by Tycho Mutual on Lapse (`RetrievalStage::House`) and Never Flown (`RS.5` × `RS.6b`)
- **Commits**: `f402556b` (`RS.5`), `4c99e772` (`RS.6b`)
- **Design Citations**: [Doc 41](41-restoration-and-salvage-loop.md#L305-L350) (§4.7 `RS.5`, `RS.6b`).
- **Locations**:
  - [`retrieval::tick`](../../crates/tacsim/src/surface/retrieval.rs#L738-L745), [`to_house` / `lapse`](../../crates/tacsim/src/surface/retrieval.rs#L752-L833), and [`retrieve_cmd`](../../crates/tacsim/src/surface/retrieval.rs#L920-L922) in [`crates/tacsim/src/surface/retrieval.rs`](../../crates/tacsim/src/surface/retrieval.rs)
  - [`job`](../../crates/tacsim/src/port/house_arm.rs#L236-L244) in [`crates/tacsim/src/port/house_arm.rs`](../../crates/tacsim/src/port/house_arm.rs)
- **Concrete Reproduction Path**:
  1. [`retrieve_cmd`](../../crates/tacsim/src/surface/retrieval.rs#L920-L922) allows posting a retrieval contract to either `ceres_port` or `stickney_port` (`+salvage/retrieve <hull>=stickney go`).
  2. When no player salvor takes a `stickney_port` contract within 24 IC hours (`RETRIEVAL_OPEN_IC_S`), [`retrieval::tick`](../../crates/tacsim/src/surface/retrieval.rs#L738-L833) unconditionally lapses it to Tycho Mutual (`to_house` / `lapse`), setting `order.stage = RetrievalStage::House`.
  3. However, [`house_arm::job`](../../crates/tacsim/src/port/house_arm.rs#L236-L244) filters `order.stage == RetrievalStage::House && &order.destination == home` (where `home` is `ceres_port`).
  4. Tycho accepts the `stickney_port` contract and never flies it, locking the poster's escrow and hull for the full 28-IC-day contract deadline!

---

### F-11 [P0] — Swept Hulls Assessed as `Disposition::Keep` Permanently Consume Ceres Workshop Berths Until the Workshop Is Completely Bricked (`RS.6c-1`)
- **Commits**: `f94fb407` (`RS.6c-1`)
- **Design Citations**: [Doc 41](41-restoration-and-salvage-loop.md#L352-L370) (§4.8 `RS.6c`, `Q-41-20`), [`41-cut-notes.md:491-565`](41-cut-notes.md#L491-L565).
- **Locations**:
  - [`assess`](../../crates/tacsim/src/port/house_sweep.rs#L132-L166) and [`to_house`](../../crates/tacsim/src/port/house_sweep.rs#L175-L198) in [`crates/tacsim/src/port/house_sweep.rs`](../../crates/tacsim/src/port/house_sweep.rs)
  - [`berth`](../../crates/tacsim/src/port/bay_work/mod.rs#L2183-L2196) in [`crates/tacsim/src/port/bay_work/mod.rs`](../../crates/tacsim/src/port/bay_work/mod.rs)
- **Concrete Reproduction Path**:
  1. When Tycho's sweep arm brings an unclaimed wreck to `ceres_port` and berths it (`HouseAct::Berth`), [`assess`](../../crates/tacsim/src/port/house_sweep.rs#L132-L166) returns `Disposition::Keep` whenever its damaged sections are repairable (`<= total / 2` and no destroyed section).
  2. [`to_house`](../../crates/tacsim/src/port/house_sweep.rs#L175-L198) transfers the asset title to `tycho_mutual` and leaves the hull berthed in the Ceres workshop (`record.berth = Some(...)`, `record.held_for = None`).
  3. Because `RS.6c-2` (refit & resale) is unbuilt, nothing ever unberths or sells a `Keep` hull.
  4. In [`bay_work::berth`](../../crates/tacsim/src/port/bay_work/mod.rs#L2183-L2196), `taken` counts every hull with `row.berth == Some(workshop) && row.held_for.is_none()` against `spec.berths`. Once Tycho sweeps `spec.berths` repairable wrecks, every player `+bay/berth` and every subsequent Tycho `HouseAct::Berth` at Ceres Freeport is permanently refused with `"Every berth in the workshop is taken."`!

---

### F-12 [P1] — `scrap_at_home` (`RS.6c-1`) Calls `return_surplus` Synchronously in the Same Tick That `strip::scrap` (`IN.8`) Queues an Async Mint
- **Commits**: `f94fb407` (`RS.6c-1`), `9c66c85c` (`IN.8`)
- **Locations**:
  - [`scrap_at_home`](../../crates/tacsim/src/port/house_arm.rs#L418-L439) in [`crates/tacsim/src/port/house_arm.rs`](../../crates/tacsim/src/port/house_arm.rs)
  - [`strip::scrap`](../../crates/tacsim/src/port/bay_work/strip.rs#L406-L531) in [`crates/tacsim/src/port/bay_work/strip.rs`](../../crates/tacsim/src/port/bay_work/strip.rs)
  - [`return_surplus`](../../crates/tacsim/src/port/house_sweep.rs#L206-L242) and [`held`](../../crates/tacsim/src/port/house_sweep.rs#L248-L274) in [`crates/tacsim/src/port/house_sweep.rs`](../../crates/tacsim/src/port/house_sweep.rs)
- **Concrete Reproduction Path**:
  1. When [`strip::scrap(ctx, hull)`](../../crates/tacsim/src/port/bay_work/strip.rs#L406-L531) scraps a hull with any intact/damaged part or `>= 1 t` alloys (`!lots.is_empty()`), it does **not** transfer `credit_minor` or set `record.scrapped_ic` synchronously; instead it queues an async `OwedKind::Mint { ..., settle: Some(settle) }` (line 529) that lands on a **subsequent** port tick (`scrap::land`).
  2. Yet [`scrap_at_home`](../../crates/tacsim/src/port/house_arm.rs#L431-L439) immediately calls `return_surplus(ctx, (float, house), credit)` in the **same tick**:
     - If `credit <= available(float)` (e.g. a `tv_rig` whose scrap credit fits inside the dispatcher's 20,000 cr `FLOAT_MINOR` sailing float), `return_surplus` drains the dispatcher's pre-existing sailing float before the workshop has paid.
     - If `credit > available(float)` (e.g. a swept `line_tank`, `escort`, or `dray` whose scrap payout exceeds what remains of `FLOAT_MINOR`), `return_surplus` fails with `InsufficientFunds` (`Step::Refused`); on the next tick the mint lands, `scrap::land` pays `credit` into `float` and sets `record.scrapped_ic = Some(...)`, so `held()` returns `false` and `return_surplus` is never called again — stranding the scrap payout in the dispatcher's `float` account instead of Tycho's `house` account.

---

### F-13 [P1] — Commit `c558db35` Regression: `reinstate_debt` in `security::on_docked` and `retrieval::desk_refusal` Flips `Delinquent` Debts Back to `Active`, Bypassing Port Quarantine and Lease Desk Refusal
- **Commit**: `c558db35` ("fix(tacsim,economy): doc 41's player waits are at least 28 IC days")
- **Design Citations**: [Doc 41](41-restoration-and-salvage-loop.md#L278-L329) (`Q-41-13`, `Q-41-25`: port security holds a carrier with delinquent fines; the lease desk refuses a delinquent debtor until paid).
- **Locations**:
  - [`FINED_OWED`](../../crates/tacsim/src/surface/security.rs#L70-L72), [`owed_minor`](../../crates/tacsim/src/surface/security.rs#L331-L351), [`holds`](../../crates/tacsim/src/surface/security.rs#L366-L368), and [`fine_actor`](../../crates/tacsim/src/surface/security.rs#L407-L414) in [`crates/tacsim/src/surface/security.rs`](../../crates/tacsim/src/surface/security.rs)
  - [`delinquent_debts`](../../crates/tacsim/src/surface/retrieval.rs#L1140-L1145) and [`desk_refusal`](../../crates/tacsim/src/surface/retrieval.rs#L1150-L1177) in [`crates/tacsim/src/surface/retrieval.rs`](../../crates/tacsim/src/surface/retrieval.rs)
  - [`tv::take`](../../crates/tacsim/src/port/tv.rs#L4252-L4255) in [`crates/tacsim/src/port/tv.rs`](../../crates/tacsim/src/port/tv.rs)
  - [`reinstate_locked`](../../crates/mush-economy/src/api.rs#L2572-L2608) in [`crates/mush-economy/src/api.rs`](../../crates/mush-economy/src/api.rs)
- **Concrete Reproduction Path**:
  1. `mush_economy::reinstate_debt` / `reinstate_sanction_debt` ([`mush-economy/src/api.rs:2597-2605`](../../crates/mush-economy/src/api.rs#L2597-L2605)) transitions an obligation's status in `EconomyStore` from `ObligationStatus::Delinquent` back to **`ObligationStatus::Active`** (`next_due = clock_now`) so the *next* periodic economy sweep will try to collect it; it does **not** sweep or charge the obligation synchronously.
  2. **Bypass 1 — Port Security Quarantine (`security::holds`)**:
     - After 28 IC days unpaid, a port security fine becomes `ObligationStatus::Delinquent`.
     - When the carrier next docks at Ceres or Stickney, [`security::on_docked` → `fine_actor`](../../crates/tacsim/src/surface/security.rs#L407-L414) calls `mush_economy::reinstate_debt`, flipping the debt from `Delinquent` to `Active`.
     - Because `c558db35` changed [`security::owed_minor`](../../crates/tacsim/src/surface/security.rs#L338-L340) to only count debts in `ObligationStatus::Delinquent`, `security::holds` immediately returns `false` on docking! The quarantined carrier can undock/lift right away before the next periodic obligation sweep tick runs. (Also note: `FINED_OWED` at line 70 still falsely tells the player `"and the {name} is held until it is paid"` when first fined during the 28-day grace.)
  3. **Bypass 2 — Occator Lease Desk (`retrieval::desk_refusal`)**:
     - A debtor with a `Delinquent` debt runs `+tv/lease` (even a preview). [`tv::take`](../../crates/tacsim/src/port/tv.rs#L4252-L4255) calls [`desk_refusal`](../../crates/tacsim/src/surface/retrieval.rs#L1150-L1177) and calls `ctx.out.pin_deferred()`, committing `reinstate_debt` and flipping every `Delinquent` debt to `Active`.
     - The debtor immediately types `+tv/lease go`: `delinquent_debts` now sees `status == Active`, returns `[]`, and the desk signs the lease for the delinquent debtor!

---

### F-14 [P1] — `create_release_obligation` (`CR.2`) Sets `due_ic = now` (`0s` Grace), Immediately Locking Insolvent Released Captives Out of `+tv/lease` and `+coop/enrol` (`CR.2` × `3837c935` / `c558db35`)
- **Commits**: `66a88181` (`CR.2`), `3837c935` (`CR.7`), `c558db35` / `cf162b21` (28-IC-day floor)
- **Design Citations**: [Doc 42](42-crew-custody-and-capture.md#L120-L150) (§4.2, `Q-42-7`), Doc 27 `FO-12`, and owner rule 2026-11-04 (28-IC-day player-response floor).
- **Locations**:
  - [`create_release_obligation`](../../crates/tacsim/src/field/crew.rs#L2641-L2677) in [`crates/tacsim/src/field/crew.rs`](../../crates/tacsim/src/field/crew.rs)
  - [`DEBT_SANCTION_PREFIXES`](../../crates/tacsim/src/surface/retrieval.rs#L40) in [`crates/tacsim/src/surface/retrieval.rs`](../../crates/tacsim/src/surface/retrieval.rs)
- **Concrete Reproduction Path**:
  1. When an insolvent captive (`< 1,500 cr`) is handed over at port, [`create_release_obligation`](../../crates/tacsim/src/field/crew.rs#L2663) creates the 1,500 cr `"release:..."` sanction obligation with `due_ic: ctx.w.world_ic_now().at` (`0s` grace), whereas `c558db35` updated `tv.rs` and `security.rs` to use `retrieval::debt_due(ctx.w)` (`now + 28 IC days`).
  2. On the very next periodic economy obligation sweep (within seconds, while the captive is still in their 1-IC-hour processing room), the obligation fails collection and transitions to `ObligationStatus::Delinquent`.
  3. Because commit `3837c935` added `"release:"` to `DEBT_SANCTION_PREFIXES`, the moment the pilot finishes processing and walks to Occator or the Co-op desk, `desk_refusal` locks them out of `+tv/lease` and `+coop/enrol` with zero 28-IC-day grace period!

---

### F-15 [P1] — Player Ramp-Exit Boarding Window (`b34208d8`): Ordering Race in `exit::tick`, Uncleaned `OwedExits` / `SiteRecord.promoted.exits` / `TowAct` on `go_back`, and Stranded Hull on Refused `rejoin`
- **Commit**: `b34208d8` ("Doc 40: the ramp exit's boarding window for player hulls")
- **Design Citations**: [`40-cut-notes.md:1070-1134`](40-cut-notes.md#L1070-L1134) (2026-11-04 ruling "back in the field").
- **Locations**:
  - [`continue_recover`](../../crates/tacsim/src/field/exit.rs#L444-L463), [`submit_owed`](../../crates/tacsim/src/field/exit.rs#L548-L591), and [`exit::tick`](../../crates/tacsim/src/field/exit.rs#L689-L718) in [`crates/tacsim/src/field/exit.rs`](../../crates/tacsim/src/field/exit.rs)
  - [`boarding::tick`](../../crates/tacsim/src/field/boarding.rs#L178-L228) and [`go_back`](../../crates/tacsim/src/field/boarding.rs#L230-L278) in [`crates/tacsim/src/field/boarding.rs`](../../crates/tacsim/src/field/boarding.rs)
  - [`carrier::on_joined`](../../crates/tacsim/src/field/carrier.rs#L945-L978) in [`crates/tacsim/src/field/carrier.rs`](../../crates/tacsim/src/field/carrier.rs)
- **Concrete Reproduction Path**:
  1. **Tick-Ordering Race**: In [`exit::tick`](../../crates/tacsim/src/field/exit.rs#L689-L718), the `for (unit, asked) in ready` loop (`continue_recover`, lines 689–714) runs **before** `super::boarding::tick(ctx)` (line 717). If the carrier is destroyed on the tick `landed(unit)` becomes true, `continue_recover` submits the surface/sortie/tow recover action and prints `"Rig 1 starts up the stern ramp..."`, and then `boarding::tick` immediately calls `go_back` → `carrier::rejoin` (`BACK_IN_FIELD`). When the queued detachment recover drains at the end of the tick, [`prepare::recover`](../../crates/tacsim/src/detachment/sortie/prepare.rs#L833-L882) transitions `flight.phase` to `Phase::Returning` while the unit is back in the field (blocking `+sortie/orders` for 300 IC s).
  2. **Uncleaned `OwedExits`, `SiteRecord.promoted.exits`, and `TowAct`**: [`go_back`](../../crates/tacsim/src/field/boarding.rs#L230-L278) removes `unit` from `ExitBook` and `Boarding`, but never removes `unit` from `OwedExits`, `hosted.live.owed_exits`, or `SiteRecord.promoted.exits`, nor does it cancel an active `TowAct { stage: Load }`. Because `SiteRecord::exited(unit)` stays `true` while `unit` is back in the field, `tow::pair_out` ([`tow.rs:684`](../../crates/tacsim/src/surface/tow.rs#L684)) returns `true`, blocking `+tv/<n> load` while `TowAct` is in flight and firing a spurious `"Rig 1's work on the <casualty> was interrupted"` when `due_ic` expires.
  3. **Refused `rejoin` Strands Hull**: If `ArenaMsg::Join` is refused by the kernel (e.g. another unit moved onto the ramp cell), [`carrier::on_joined`](../../crates/tacsim/src/field/carrier.rs#L945-L978) checks `unit` back in and removes it from `live.units`, but does nothing for `rejoin == true` (and `go_back` already removed `unit` from `Boarding` and moved the pilot to `ReturningSeats`), stranding the hull on the ground with its pilot unseated.

---

### F-16 [P1] — `FR.6` (`eb9b7359`): `LocalSpaceBubble::set_party_relation(..., None)` on `"INTERDICTOR_STAND_DOWN_CLEARED"` Does Not Clear `book.fired`, Keeping the Bubble Stuck `Hostile`
- **Commit**: `eb9b7359` ("FR.6 the seam, SpacePartyId deleted for the arena's PartyId")
- **Design Citations**: [Doc 40](40-field-second-round.md#L191-L208) (§4.6 `FR.6`), [`40-cut-notes.md:839-891`](40-cut-notes.md#L839-L891) ("No rule changes in space").
- **Locations**:
  - [`set_party_relation`](../../crates/tacsim/src/space/bubble.rs#L244-L249) in [`crates/tacsim/src/space/bubble.rs`](../../crates/tacsim/src/space/bubble.rs)
  - [`Relations::relation`](../../crates/tacsim/src/arena/party.rs#L273-L279) and [`Relations::mark`](../../crates/tacsim/src/arena/party.rs#L302-L313) in [`crates/tacsim/src/arena/party.rs`](../../crates/tacsim/src/arena/party.rs)
  - [`is_carrier_clear_to_resume`](../../crates/tacsim/src/space/tick.rs#L274-L278), [`is_resolved_for_carrier`](../../crates/tacsim/src/space/tick.rs#L338-L343), and [`step_interdictor_picket`](../../crates/tacsim/src/space/tick.rs#L3985-L4008) in [`crates/tacsim/src/space/tick.rs`](../../crates/tacsim/src/space/tick.rs)
- **Concrete Reproduction Path**:
  1. Any weapon fire in `LocalSpaceBubble` (`bubble.rs:1506, 1594, 1683`) calls `self.party_relations.note_fire(shooter_party, target_party)`, inserting `shooter_party` into `target_party`'s `book.fired`.
  2. When the interdictor picket mind emits `"INTERDICTOR_STAND_DOWN_CLEARED"`, [`step_interdictor_picket`](../../crates/tacsim/src/space/tick.rs#L3985-L4000) sets `*cleared = true` and calls `bubble.set_party_relation(cparty, INTERDICTOR_PARTY, None)`.
  3. `set_party_relation(..., None)` calls `Relations::mark(..., None)`, which removes `of` from `book.marks` ([`party.rs:311`](../../crates/tacsim/src/arena/party.rs#L311)) but **never removes `of` from `book.fired`**!
  4. Because [`Relations::relation`](../../crates/tacsim/src/arena/party.rs#L276) returns `Relation::Hostile` when `book.marks.get(&other)` is `None` and `book.fired.contains(&other)` is `true`, if either ship fired a single shot before stand-down, `party_relation` remains `Relation::Hostile` and `is_carrier_clear_to_resume` / `is_resolved_for_carrier` stays `false` forever.

---

### F-17 [P1] — `charge_storage` Seizure (`Then::Seized`) on Uncollected Completed Yard Hull Orders Always Fails in `WorksProvider::prepare_authoritative`, Causing an Infinite 4-Tick Retry & Notification Loop (`IN.3` / `IN.9`)
- **Commits**: `cb5b2934` (`IN.3`), `27da64e7` (`IN.9`)
- **Locations**:
  - [`bulk_lots`](../../crates/tacsim/src/port/works.rs#L1323-L1340), [`charge_storage`](../../crates/tacsim/src/port/works.rs#L2017-L2043), and [`WorksProvider::prepare_authoritative`](../../crates/tacsim/src/port/works.rs#L2486-L2491) in [`crates/tacsim/src/port/works.rs`](../../crates/tacsim/src/port/works.rs)
- **Concrete Reproduction Path**:
  1. A player's hull work order completes (`tacsim.hull_kit.*` minted in `order.escrow`), the 28-IC-day free storage expires while the player is undocked or insolvent, and [`charge_storage`](../../crates/tacsim/src/port/works.rs#L2017-L2043) triggers `Then::Seized`.
  2. `charge_storage` calls `bulk_lots` (which excludes kits via `!is_kit(good)` at line 1331) and puts the kit lot into `WorksAct.lots`.
  3. However, [`WorksProvider::prepare_authoritative`](../../crates/tacsim/src/port/works.rs#L2486-L2491) rejects any lot in `lots` where `!is_part(lot.commodity())` with `"a bulk lot is pooled before it moves"`!
  4. The seizure fails on every attempt, retrying every `MAX_ACT_TRIES` (4 ticks) forever and spamming the requester with `"Order <id>: storage went unpaid..."` on every retry cycle.

---

### F-18 [P1] — `+yard/quote <design>` Omits All Component Parts, and `bill_with_parts` Omits `DesignBill.core` (Giving All Cored Hulls Free Installed Cores) (`IN.9`)
- **Commits**: `27da64e7` (`IN.9`), `ec95eb48`
- **Design Citations**: [Doc 39](39-industry-production-chain.md#L422-L478) (§4.7 & §6 `IN.9`: `+yard/quote` and `+yard/order` must include the hull's component parts and core).
- **Locations**:
  - [`bill_with_parts`](../../crates/tacsim/src/port/works.rs#L659-L715) and [`yard_quote_cmd`](../../crates/tacsim/src/port/works.rs#L871-L877) in [`crates/tacsim/src/port/works.rs`](../../crates/tacsim/src/port/works.rs)
  - [`DesignBill`](../../crates/tacsim/src/port/bill.rs#L29-L100) in [`crates/tacsim/src/port/bill.rs`](../../crates/tacsim/src/port/bill.rs)
- **Details**:
  1. [`yard_quote_cmd`](../../crates/tacsim/src/port/works.rs#L871-L877) calls `bill_of(&pinned, quantity)` instead of `bill_with_parts(ctx.w, &pinned, quantity)` (unlike `order` at line 993), so `+yard/quote dray` only shows `alloys` and `ship_fittings` and omits all 8 required component parts.
  2. [`bill_with_parts`](../../crates/tacsim/src/port/works.rs#L659-L715) iterates `design.weapons`, `design.sensors`, `design.mounts`, and `design.mobility`, but **never reads `design.core`** (even though [`bill.rs:67, 100`](../../crates/tacsim/src/port/bill.rs#L67-L100) and [`mass.rs:183, 263`](../../crates/tacsim/src/port/mass.rs#L183-L263) subtract `core` mass from `fittings_kg`). Every cored hull (`tv_escort`, `line_tank`, `dray`) is ordered, built, and commissioned with a free installed core!

---

### F-19 [P1] — `part_ask_step` Only Probes `seed_lot_id(&id, n)` Instead of `storefront_lots`, So `Manufactured`, `Scrapped`, and Player-Sold Part Lots Are Never Listed on Market Asks (`IN.7` / `IN.8`)
- **Commits**: `1f925b86` (`IN.7`), `9c66c85c` (`IN.8`)
- **Design Citations**: [Doc 39](39-industry-production-chain.md#L380-L420) (§4.6 `IN.7`).
- **Location**: [`part_ask_step`](../../crates/tacsim/src/port/stock.rs#L1263-L1301) in [`crates/tacsim/src/port/stock.rs`](../../crates/tacsim/src/port/stock.rs).
- **Details**: `part_ask_step` loops `for n in 0..stock.qty` and looks up `seed_lot_id(&id, n)` (`"tacsim.stock/{vendor}/{good}/{n}"`). Any part lot placed in the vendor's storefront by `IN.3`/`IN.7` manufacturing (`"tacsim.works/{order}/..."`), `IN.8` scrapping (`"tacsim.scrap/..."`), or a player selling a part into a bid has a non-seed `LotId` and is **never** offered on the market ask board.

---

### F-20 [P1] — `TendWork::Stabilize` (`RS.1`) Spare-Parts Calculation Mismatch Causes 15-Minute Field Stabilization to Fail at `finish` With `TendStep::Lost` (`InsufficientSpareParts`)
- **Commit**: `2585edba` (`RS.1`)
- **Locations**:
  - [`TendWork::need`](../../crates/tacsim/src/surface/tender.rs#L226-L235), [`library_kg_per_point`](../../crates/tacsim/src/surface/tender.rs#L273-L276), [`plan_tend`](../../crates/tacsim/src/surface/tender.rs#L576-L578), and [`run_library`](../../crates/tacsim/src/surface/tender.rs#L1278-L1300) in [`crates/tacsim/src/surface/tender.rs`](../../crates/tacsim/src/surface/tender.rs)
- **Concrete Reproduction Path**:
  1. At start (`plan_tend`), [`TendWork::need`](../../crates/tacsim/src/surface/tender.rs#L228) computes required fittings as `budget.fittings_kg_for(points)` = `(fittings_kg * points).div_ceil(total_points)` (e.g., on an escort with `fittings_kg = 515` and `total_points = 1800`, stabilizing a 500-point section checks `need.fittings_kg == 143 kg`).
  2. When the 15-minute act finishes, [`run_library`](../../crates/tacsim/src/surface/tender.rs#L1278-L1300) passes `spare_parts_kg: stores.fittings_kg` and `spare_parts_kg_per_point: need.library_kg_per_point` (`budget.fittings_kg_for(1) == 1 kg/point`) into `mush_tactics::field_stabilize_damaged_section`.
  3. `field_stabilize_damaged_section` checks `points * spare_parts_kg_per_point = 500 * 1 = 500 kg <= spare_parts_kg`!
  4. Whenever `stores.fittings_kg` is between `143 kg` and `499 kg` (or whenever a section has `> 1,000` points and the rig opened a single 1,000 kg bale), `+tv/<n> stabilize` starts, burns 15 minutes and battery charge, and then fails at `finish` with `TendStep::Lost` (`InsufficientSpareParts`)!

---

### F-21 [P1] — Salvage Claim, Retrieval, and Sweep Custody Bugs (`RS.4` / `RS.5` / `RS.6c-1`)
- **Commits**: `b292ba7d` (`RS.4`), `f402556b` (`RS.5`), `f94fb407` (`RS.6c-1`)
- **Locations & Concrete Reproduction Paths**:
  1. **`+salvage/contest <hull>=right` Ignores `RS.5` Retrieval Contracts (`Q-41-23`)**: [`claim::right_of_grantor`](../../crates/tacsim/src/surface/claim.rs#L1004-L1009) only checks `pin.owner_grant` (ground `unit.recover_load` grants) and never checks `TacsimStore.retrievals` (`RS.5`), even though [`41-cut-notes.md:191`](41-cut-notes.md#L191) promised `RS.5` would wire retrieval contracts into `right_of_grantor`. A title holder who posted a retrieval contract cannot contest a rogue claim by the contracted salvor on the `right` ground.
  2. **Organization Officers Rejected by Space `BarePriorTitle` Contest & `+salvage/retrieve`**:
     - In [`apply_space_claim_order`](../../crates/tacsim/src/surface/claim.rs#L1594-L1601), `ContestGround::BarePriorTitle` checks `file.prior_owner != actor` without checking `custody.pin.as_ref().is_some_and(|pin| pin.answers_for(actor))` (unlike surface `prepare_claim_order` at line 1419), silently dropping the space contest after `contest_cmd` printed `CONTEST_FILED`.
     - In [`retrieve_cmd`](../../crates/tacsim/src/surface/retrieval.rs#L905-L913), ownership is checked via `asset.title_owner == PrincipalId::Object(ctx.actor)` instead of `acts_for`, blocking org officers from posting retrieval contracts for org-titled hulls.
  3. **Vacating a Claim (`Settlement::Vacate`) Strands Hulls in Workshop or Strands Tycho's `lifter` at a Site (`Q-41-19`)**:
     - In [`claim::tick`](../../crates/tacsim/src/surface/claim.rs#L1239-L1258), `Settlement::Vacate` calls `bay_work::hold_for` without calling `bay_work::berth_held(ctx, row.carrier)` when the custodian's carrier is *already* docked at a workshop port, leaving the hull locked in the custodian's bay until they undock and re-dock.
     - If Tycho already berthed a swept hull in Ceres workshop (`HouseAct::Berth`) before its claim is vacated, `hold_for` sets `held_for = Some(owner)` on a record that already has `berth = Some(...)`; [`berth_held`](../../crates/tacsim/src/port/bay_work/mod.rs#L2423-L2490) filters `row.berth.is_none()`, so Occator leased rigs (`tv::hand_back`) are never returned to the shed.
     - If a claim is vacated while Tycho's `lifter` is still docked at the surface site after recovering `crawler`, [`house_sweep::held`](../../crates/tacsim/src/port/house_sweep.rs#L248-L274) returns `None` (`ours == false`), so `sweep_act` returns `None` and leaves `lifter` stranded at the surface site instead of sailing back to `ceres_port`.
  4. **`write_off_left_out` Teleports Leased Rigs Out of Foreign Salvor Carrier Bays in Deep Space (`Q-41-14`)**: [`write_off_left_out`](../../crates/tacsim/src/port/tv.rs#L5084-L5093) (and `repossess` at line 5504) only exempts `Where::Bay { hull } if hull == record.hull` (the lessee's own carrier). If a leased rig is in a **foreign salvor's carrier bay** (`hull != record.hull`) in deep space when the 28-IC-day write-off or repossession fires, `hand_back` / `place_interior` teleports the rig out of the salvor's bay in deep space back to Occator's shed.
  5. **`deliver_active_tow_at_port` Restores Pre-Checkout OOC Health Snapshot (`Q-CF-35`)**: [`deliver_active_tow_at_port`](../../crates/tacsim/src/space/tick.rs#L2811) calls `mush_tactics::custody::check_in(ctx, derelict_hull, None)` instead of passing `Some(&UnitMutationPlan::default())`, restoring full health if the derelict still carries a `DeployedUnits` marker.

---

### F-22 [P1] — Crew Custody, Radio Gate, Commissioned-Carrier Friendly Load, and `+sortie/extract` Exploits (`CR.1`–`CR.6` × `IN.9`)
- **Commits**: `66a88181`, `0a08d078`, `f64935c9`, `a5da7fd9` (`CR.1`–`CR.6`), `27da64e7` (`IN.9`)
- **Locations & Concrete Reproduction Paths**:
  1. **Active `ComsysState` Alias (`dt <msg>`) Bypasses `Gate::Chat` (`Q-42-2`)**: [`detachment/commands.rs:312`](../../crates/tacsim/src/detachment/commands.rs#L312) gates `+sortie/chat` behind `Gate::Chat`, but a pilot who ran `+sortie/chat` before capture/ejection has an active `ComsysState` alias (`dt <msg>`) registered by [`detachment/chat.rs:78-96`](../../crates/tacsim/src/detachment/chat.rs#L78-L96). Speaking via `dt <msg>` ([`mush-comsys/src/speak.rs:35`](../../crates/mush-comsys/src/speak.rs#L35)) does not check `crew::gate` or `CarrierHooks::restrains_operator`, letting captives broadcast over their detachment's IC radio channel.
  2. **Missing `crew::gate` on `+mine` and `+sortie/unseat`, `accept`, `ready` (`Q-42-8`)**:
     - [`mine_cmd`](../../crates/tacsim/src/space/commands.rs#L5668) has no `crew::gate(ctx, Gate::Operate)` check.
     - [`sortie_gate`](../../crates/tacsim/src/detachment/commands.rs#L304-L315) omits `"unseat"` (`+sortie/unseat <npc>`), `"accept"`, and `"ready"`, while gating `"contract"` (a read-only view).
     - [`tv_cmd`](../../crates/tacsim/src/port/tv.rs#L2324-L2331) mutates `RigPointer` on `+tv/<slot>` (`Act::Status`) before checking `crew::gate`.
  3. **Loading or Hard-Docking a Friendly Hull Into a Yard-Commissioned Carrier (`tacsim.owned_hulls`) Captures Friendly Crew and Deducts `-10` Standing (`CR.1`/`CR.6` × `IN.9`)**:
     - [`tow::friendly`](../../crates/tacsim/src/surface/tow.rs#L1010-L1048) and [`parties::carrier_entrant`](../../crates/tacsim/src/field/parties.rs#L431-L447) (used by [`hard_dock_friendly`](../../crates/tacsim/src/space/custody.rs#L1352-L1370)) resolve the carrier's owner via `coop::helm_holder(w, carrier)` ([`coop.rs:699-721`](../../crates/tacsim/src/port/coop.rs#L699-L721)), which returns `None` for every yard-commissioned carrier in `tacsim.owned_hulls` (`IN.9`)!
     - Consequently, when a player loads their own (or an org-mate's) disabled rig into their yard-commissioned carrier on the ground — or clamps a friendly ship in space with a yard-commissioned tug — `friendly` returns `false`, [`captured_loaded`](../../crates/tacsim/src/field/crew.rs#L995-L1050) **captures the friendly crew**, logs `UnlawfulKind::Capture`, and deducts `-10` standing!
     - Furthermore, [`capture_at_hard_dock`](../../crates/tacsim/src/space/custody.rs#L1410) hardcodes `captor: ctx.actor` instead of resolving the tug's custodian (`Q-42-4`), and [`captives_of`](../../crates/tacsim/src/field/crew.rs#L2296) uses `coop::helm_of` (returning only the single lowest-ID hull), hiding captives held on a captain's second/commissioned carrier. (Note: [`retrieval::on_docked`](../../crates/tacsim/src/surface/retrieval.rs#L642) and [`bay_work::stow`](../../crates/tacsim/src/port/bay_work/mod.rs#L2302) also fail on yard-commissioned carriers for the same `coop::helm_holder` reason.)
  4. **`may_extract` Allows Any Subordinate DT Member or Rescued Friendly Casualty Aboard a Carrier (Even in Deep Space) to `+sortie/extract` to Port (`Q-42-15`)**:
     - [`may_extract`](../../crates/tacsim/src/field/crew.rs#L413-L424) admits any `CrewLocation::Carrier { hull }` where `helm_of(w, who) != Some(*hull)` and `inside(w, who, *hull)`.
     - Because `CrewLocation::Carrier` is also written for every subordinate DT wingman (`check_in_aboard`), rescued friendly casualty (`aboard_loaded`), and multi-hull owner aboard their second hull — without checking whether the carrier is even on a surface site or in deep space — any wingman or passenger can `+sortie/extract` out of a carrier in deep space back to port!

---

### F-23 [P1] — Additional `S.4b-2` / `S.4b-3` Lifecycle Wedges & Refusals (`close.rs`, `surface/tick.rs`, `recovery.rs`, `port/tv.rs`)
- **Commits**: `911897b5` (`S.4b-2`), `55753484` (`S.4b-3`)
- **Locations & Concrete Reproduction Paths**:
  1. **Wrecked Carrier's Bay Rigs (`wreck_bay_rigs`, `GC.3`) Refuse to Wreck When Holding an IC Claim**: [`wreck_bay_rigs`](../../crates/tacsim/src/field/close.rs#L486-L517) calls `mush_tactics::module::apply_plan(ctx, rig, &plan)`, which checks [`ic_custody::refuse_mutation`](../../crates/mush-tactics/src/ic_custody.rs#L297-L305) ([`module.rs:2828`](../../crates/mush-tactics/src/module.rs#L2828)) and fails with `ModuleError::IcCustody(rig)` if a bay rig has an open or `Closing` IC claim when its carrier falls in a field — leaving the bay rig undamaged.
  2. **`claim_attempt` Key Collision in `surface/tick.rs`**: [`claim_attempt`](../../crates/tacsim/src/surface/tick.rs#L658-L660) uses `(record.tv.0, record.ordinal.saturating_add(1), CLAIM_CYCLE)`. Because `record.ordinal` does not increment on `open_ic_mind` or `claim: "refused"`, [`attempt_visit`](../../crates/tacsim/src/surface/tick.rs#L416-L426) counts a **successful** `open_ic_mind` commit on Fire 1 as a failed attempt against `submit_launch` on Fire 2, and shares the 8-attempt budget and 60-fire backoff across `open_ic_mind`, `submit_launch`, and `close_ic_mind`!
  3. **`+tv/<n> launch` Allowed Onto a Promoted Site (`ramp_handoff`), Stranding the Rig on the Ramp Outside the Running Field**: [`prepare_act`](../../crates/tacsim/src/surface/act/providers.rs#L875-L895) exempts `ActKind::Launch` (`ramp_handoff = true`) from the `site_promoted` refusal (`FIELD_OPEN_CODE`). A surface rig launched via `+tv/1 launch` while a field is running on the site is placed on the ground outside the running field (since `admit::tick` for an already-hosted site only checks DT `SortieHandles.flights`), where it can neither join the field nor drive/mine/recover (`FIELD_OPEN_CODE`).
  4. **Staff `RecoveryOperation::Close` / `Home` During `Opening` or Pre-Finish `Launch` Wedges `ClaimCursor`**: In [`execute_recovery`](../../crates/tacsim/src/surface/recovery.rs#L1019-L1102), running `Close` or `Home` while `record.claim` is `ClaimCursor::Opening` clears `record.queued = None` without transitioning `record.claim` to `Closing` (causing `submit_launch` and `claim: "refused"` to both fail in an infinite retry loop). Running `Close` during the 30s `ActKind::Launch` (`in_any_bay == true`, `record.sortie == None`) clears `act` and `excursion` without calling `owe_claim_close`, leaking the claim forever.
  5. **`rig_flight` Ignores `record.claim` (`Opening` / `Closing`)**: [`rig_flight`](../../crates/tacsim/src/port/tv.rs#L5680-L5740) checks `sortie`, `excursion`, and `act`, but ignores `record.claim`. Between Fire 1 (`open_ic_mind`) and Fire 2 (`submit_launch`), `rig_flight` returns `RigFlight::Idle`, allowing the carrier to undock/lift via [`bay_gate_leaving`](../../crates/tacsim/src/port/voyage.rs#L3082-L3137) into `JouStore` while carrying an active `AnchorImage::Hangar` claim (triggering `NeverLeft` F-02 and failing `ic_exclusion::check_boot_journeys` on server restart), or allowing `+tv/lease return go` / `repossess` to close the lease and then fail at `place_interior(..., shed)` with `SpatialError::IcCustody`.

---

### F-24 [P1] — Four Concrete Spec-vs-Code Contradictions in `docs/design/42b-surrender.md` (Committed at `57f4c7e6`)
- **Commit**: `57f4c7e6` (`docs(42b): cut plan SU.0..SU.6 for NPC surrender`)
- **Locations & Contradictions to Fix Before Implementing `SU.1`–`SU.3`**:
  1. **`SU.3` ([`42b-surrender.md:208-215, 565-573`](42b-surrender.md#L208-L215)) assumes `exit::on_withdrawn` / `close::demote_units` writes surrendered hulls to `SiteRecord.units` as `Casualty`**: In reality, [`exit::on_withdrawn`](../../crates/tacsim/src/field/exit.rs#L228-L250) discards `checked.left`, removes the unit from `hosted.live.units`, and [`close::left_condition`](../../crates/tacsim/src/field/close.rs#L366-L375) returns `None` for a mobile unit that yielded on morale (`!immobile && !wreck`). Without an explicit `SiteRecord.units` write on `Yielded`, surrendered raider hulls will not appear on `+salvage` and cannot be towed.
  2. **`SU.1` ([`42b-surrender.md:458-463, 542`](42b-surrender.md#L458-L463)) assumes `crew::check_in` reaches `leaves_alone` for captured NPCs when a carrier is wrecked**: In [`crew::check_in`](../../crates/tacsim/src/field/crew.rs#L768-L772), [`wrecked_captive`](../../crates/tacsim/src/field/crew.rs#L933-L961) runs *before* `leaves_alone` (and even when `pilot == None` on an NPC carrier) and turns every `Captured` record on the wrecked carrier into `CrewLocation::Stranded`, violating `Q-42b-4` and `Q-42b-19`.
  3. **`SU.1` ([`42b-surrender.md:541`](42b-surrender.md#L541)) widening `crew_to_capture` to NPC raiders triggers unlawful capture & `-10` standing**: [`place_in_bay`](../../crates/tacsim/src/surface/tow.rs#L3217) logs `UnlawfulKind::Capture` for every ID returned by `captured_loaded`, and [`captured_loaded`](../../crates/tacsim/src/field/crew.rs#L1035) deducts `-10` standing per captured pilot unless NPC raiders (`crew_of.is_some()`) are exempted.
  4. **`SU.1` ([`42b-surrender.md:364-370, 540`](42b-surrender.md#L364-L370)) omits `release_in_bay`**: [`release_in_bay`](../../crates/tacsim/src/field/crew.rs#L2489-L2531) writes `CrewLocation::Carrier { hull: carrier }` instead of deleting the NPC's `CrewRecord` and despawning the NPC (`Q-42b-7`).

---

## Section 2: P2 Findings (Edge Conditions, Resource Caps, & Minor Bugs)

1. **F-25 [P2] — `WorksProvider::owed_rows` Global 16-Contract Cap Starves Higher-ID Contracts & Pool Moves (`IN.3`/`IN.10`)**:
   - [`WorksProvider::owed_rows`](../../crates/tacsim/src/port/works.rs#L2202-L2242) truncates `out` at `MAX_OFFER_ROWS = 16` (`works.rs:88`) in `BTreeMap` order of `c.id` *before* calling `supply::owed_moves` or `pools::owed_moves`. If 16 lower-ID contracts are stuck (e.g., from F-17), all higher-ID player orders and house/pool replenishments are permanently starved.
2. **F-26 [P2] — Cancelled / Failed Yard Orders With Undocked Requester Get Infinite Free, Seizure-Proof Storage (`IN.3`)**:
   - When an order is cancelled or fails while the requester is undocked, [`return_bill` / `fail_back`](../../crates/tacsim/src/port/works.rs#L1891-L1938) leaves the materials in `order.escrow` awaiting `+yard/collect`, but `order_step` only runs `charge_storage` on `WorkOrderState::Completed`, violating Doc 39 §4.12.
3. **F-27 [P2] — Global `MAX_CARRIED_BYTES` Budget in `field::provider::carried_offer` Can Drop Another Site's Carried Minds (`D-23-18`)**:
   - [`carried_offer`](../../crates/tacsim/src/field/provider.rs#L378-L416) caps `CarriedRow`s across *all* sites to a single 32 KiB budget in alphabetical site order (`fit_carried_sites`), even though each `tacsim_field` action only reads its own site's entry.
4. **F-28 [P2] — Refused Mid-Field Join Cleanup (`carrier::on_joined`, `raiders::on_joined`) Leaves Dangling `stance_hostile` Entries and Empty Parties (`FR.1`/`FR.3`)**:
   - [`carrier::joining_as`](../../crates/tacsim/src/field/carrier.rs#L465-L477) pushes a new party `C` into existing parties' `row.stance_hostile`, which [`carrier::on_joined`](../../crates/tacsim/src/field/carrier.rs#L945-L960) and [`raiders::on_joined`](../../crates/tacsim/src/field/raiders.rs#L1257-L1277) never remove if the kernel refuses `ArenaMsg::Join` (and `raiders::on_joined` / `rejoin` also leave the empty party `C` in `live.parties`).
5. **F-29 [P2] — `field::boot::resume` Re-Joins Post-Snapshot `arrived` Units at `pad_pose`, Drops Coupled `tow`, and Omits `Allied` Stance Friendships (`FR.1`/`FR.3`/`FR.5b`)**:
   - [`boot::resume`](../../crates/tacsim/src/field/boot.rs#L208-L241) uses `pad_pose` and `carrier::joining` (`tow: None`) for all `row.arrived && !in_snapshot` units, and [`carrier::joining_as`](../../crates/tacsim/src/field/carrier.rs#L439) sets `allied = Vec::new()` when `row.party` is already in `live.parties`.
6. **F-30 [P2] — `+tv/<n> stores open` (`RS.1`) Has No `Where::Bay` or Busy Gate, and `house_arm::next_act` Hardcodes `(cx + 1, cy)`**:
   - [`stores_cmd`](../../crates/tacsim/src/surface/tender.rs#L1013-L1048) allows a rig in a workshop/shed to break tonnes from its carrier's hold and prints `STORES_OPENED` even when the rig is out on a site.
   - [`house_arm::next_act`](../../crates/tacsim/src/port/house_arm.rs#L230-L231) hardcodes driving to `format!("{},{}", cx + 1, cy)` instead of selecting an open adjacent cell around the casualty.
7. **F-31 [P2] — `capture_at_hard_dock` Airlock Fallback Allows Captive Movement Into Ship Interior (`CR.6`)**:
   - [`capture_at_hard_dock`](../../crates/tacsim/src/space/custody.rs#L1386-L1393) falls back to `view.airlock()` when a tug has no bay room; because `try_intercept_disembark` only intercepts `airlock_exit()`, captives placed in the airlock can walk through interior ship exits to the bridge.

---

## Section 3: Design Ruling (`Q-*`, `D-*`, 28-IC-Day Floor) & Cut-Note Audit

| Doc | Cuts Landed | Unbuilt / Deferred Cuts & Undocumented Ruling Deviations |
| :--- | :--- | :--- |
| **Doc 23** (`S.4b-*`, `D-23-18`) | `S.4b-T`, `S.4b-1`, `S.4b-2`, `S.4b-3`, `D-23-18` | **Undocumented Deviation**: Compute-seat refusal (`D-23-16` / `Q-23-12`: `pool_for` / `load_of`) was deferred from `S.4b-2` to `S.4b-3` ([`23-cut-notes.md:438`](23-cut-notes.md#L438)), but `S.4b-3` ([`dispatch.rs:384-403`](../../crates/tacsim/src/surface/dispatch.rs#L384-L403)) only wired licence seats (`seats_for` / `held_by`) and omitted compute seats without cut-note disclosure.<br>**Cross-PR Seam Dropped**: `ArenaMsg::Join.source` and `LaunchSeal::Claim` ([`carrier.rs:597, 824`](../../crates/tacsim/src/field/carrier.rs#L597)) — see **F-01**. |
| **Doc 39** (`IN.0`–`IN.11`) | `IN.0`–`IN.10` | **Unbuilt Cut**: **`IN.11`** (30-IC-day corridor soak test & rate pass, [`39-industry-production-chain.md:516-545`](39-industry-production-chain.md#L516-L545)) is completely unbuilt.<br>**Documented Deferrals**: `IN.10` Occator rig/escort pool floors (`tv_rig: 6`, `tv_escort: 2` at Ceres/Stickney) deferred in [`39-cut-notes.md:883-885`](39-cut-notes.md#L883-L885) pending a multi-output bay commissioning provider.<br>**Undocumented Deviations**: **F-06** (`coop.rs` ignores `owned_hulls`), **F-07** (`+yard/deliver` ignores `PartProvenance`/`Hot`/`Disputed`/`Damaged`), **F-18** (`+yard/quote` omits parts; `bill_with_parts` omits `core`), **F-19** (`part_ask_step` ignores non-seed lots). |
| **Doc 40** (`FR.0`–`FR.7c`) | `FR.0`–`FR.7c`, `FR.5b` | **Documented Deferrals**: `+tv/<n> launch` into a running field deferred per `Q-38-25` ("Wait on the rig ramp until Players start asking for it").<br>**Undocumented Deviations**: **F-01** (`ArenaMsg::Join.source` & `LaunchSeal::Claim`), **F-08** (first DT launch into running field breaks `Friend` / stances / `+field/mark`), **F-15** (player ramp boarding window races), **F-16** (`FR.6` `set_party_relation(..., None)` leaves `book.fired` hostile). |
| **Doc 41** (`RS.0`–`RS.10`) | `RS.0`–`RS.8` (`RS.6b`, `RS.6c-1`) | **Unbuilt Cuts**: **`RS.6c-2`** (Tycho refit & resale — causes **F-11** workshop berth exhaustion), **`RS.9`** (90-IC-day abandoned hull sweep), and **`RS.10`** (180-IC-day wreck decay) are unbuilt.<br>**Undocumented Deviations**: **F-09** (sweep preemption deadlock), **F-10** (Stickney retrieval lapse black hole), **F-12** (sync `return_surplus` before async scrap mint), **F-13** (`c558db35` `reinstate_debt` quarantine/desk bypass), **F-20** (`Stabilize` spare-parts mismatch), **F-21** (`right_of_grantor` ignores `RS.5` retrieval; `write_off_left_out` teleports from foreign salvor bays). |
| **Doc 42 & 42b** (`CR.0`–`CR.7`, `SU.0`–`SU.6`) | `CR.0`–`CR.7` landed; `SU.0`–`SU.6` design-only | **Unbuilt Cuts**: **`SU.0`–`SU.6`** ([`42b-surrender.md`](42b-surrender.md)) are design-only at `57f4c7e6` (with 4 spec-vs-code contradictions in **F-24**).<br>**Undocumented Deviations**: **F-14** (`create_release_obligation` has `0s` grace instead of 28 IC days), **F-22** (`ComsysState` alias bypasses `Gate::Chat`; commissioned carriers capture friendly crew; `may_extract` allows deep-space extraction). |
| **28-IC-Day Floor** (`cf162b21`, `c558db35`) | `#475` | 1. [`create_release_obligation`](../../crates/tacsim/src/field/crew.rs#L2663) still uses `due_ic: ctx.w.world_ic_now().at` (`0s` grace) — **F-14**.<br>2. [`RESERVATION_TTL_IC_DAYS`](../../crates/mush-inventory/src/record.rs#L33) in `mush-inventory` is still `7` IC days.<br>3. `cf162b21` broke `in10_pools::stocked` on `origin/main` — **F-06**.<br>4. `c558db35` broke `security::holds` and `retrieval::desk_refusal` via `reinstate_debt` — **F-13**. |

---

## Section 4: Test, Acceptance Story (`Q-38-29`), Dual-Scale (`256`/`512`), and Fiat Audit

1. **Broken Test on `origin/main` (`57f4c7e6`)**:
   - [`domain_economy::in10_pools::stocked`](../../crates/tacsim/tests/domain_economy/in10_pools.rs#L430) **panics on `origin/main`** (`assertion 'left == right' failed: ninth captain takes the hull the yard built for the Trust; left: None, right: Some(ObjId(225))`) due to **F-06**.
2. **Missing `mush-bot` SSH Acceptance Stories (`Q-38-29` / Doc 38 §9)**:
   - **Doc 41 (`RS.1`–`RS.8`)**: Zero `mush-bot` SSH stories were added for `RS.1` through `RS.8` ([`crates/mush-bot/src/mine/salvage.rs`](../../crates/mush-bot/src/mine/salvage.rs) has not been touched since `RS.0`, despite `41-cut-notes.md` claiming "What the bot proves" in each cut).
   - **Doc 39 (`IN.0`–`IN.10`)**: Only `IN.5` has a `mush-bot` story ([`mining_cohort.rs`](../../crates/mush-bot/src/mine/mining_cohort.rs)); `IN.1`–`IN.4` and `IN.6`–`IN.10` rely exclusively on in-process `domain_economy` tests, and `IN.11` is unbuilt.
3. **Test Fiat Masking Production Bugs**:
   - In [`crates/tacsim/tests/surface_salvage.rs`](../../crates/tacsim/tests/surface_salvage.rs) (`RS.1`–`RS.8`), tests use `@tacsim/site leave` (`to_cell_on`) and `set_section` to teleport rigs to adjacent cells and stage damage rather than driving rigs over `256` (Ahuna) and `512` (Urvara) terrain, and `RS.6c-1`'s test ([`house_sweep_rs6c.rs:212-238`](../../crates/tacsim/tests/domain_economy/house_sweep_rs6c.rs#L212-L238)) only tests a cheap `tv_rig` (`credit < FLOAT_MINOR`), masking **F-12** (`scrap_at_home` synchronous `return_surplus` failure on larger hulls) and **F-09** (mid-sweep `job()` preemption deadlock).
   - In [`crates/tacsim/tests/field_fr1_fr4.rs`](../../crates/tacsim/tests/field_fr1_fr4.rs) and [`crates/tacsim/tests/fr7_round.rs`](../../crates/tacsim/tests/fr7_round.rs), `FR.3` tests always had slot 1 already on the ground at field promotion and only launched slot 2 into the running field, masking **F-08** (first DT launch into a running field).
