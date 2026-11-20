# 48c. Astra final implementation review of designs 48 and 49

**Request changes.** The 2026-11-19 review found 36 issues, including 13 P1 and 23 P2. A buyer can pay for a lot that never arrives, a facility can change hands with debt or customer work outstanding, and an NPC helm can retain control after title transfers. In combat, an upgrade can freeze TacSim mechanics and a refit can refill magazines for free.

The review compared the implementation at `480df36ed` on `origin/main` with the supplied base `fd3108cfe`, using the economy and combat plans as the acceptance criteria. Code links point to the reviewed revision so they remain usable as fixes land. [Reviewed changes](https://github.com/conclavio/rustmush/compare/fd3108cfe2395abc45bd498507ae55cc3b95a3c4...480df36ed28ec4068a2f1fef3238a82251977ca0) · [Design 48](48-economy-and-production.md) · [Design 49](49-combat-content.md)

All **72 focused tests passed** on that revision.

| Suite | Tests passed |
| --- | ---: |
| Market, EX.1a | 18 |
| TacSim economy, EX.9 | 5 |
| TacSim economy, other exchange and production cases | 36 |
| TacSim field, including berth rearm, cored resale, escort, and stipends | 7 |
| TacSim arena, CC.1 through CC.4 | 6 |

## Market settlement and custody

1. **[P1] Require a delivery receipt before paying for an exact lot.**

    A held-lot purchase can settle on its time window alone. If the destination fills after acceptance, inventory refuses the handover while the seller still receives the buyer’s money. Gate payment on a durable receipt from the lot move and refund escrow if the receipt never arrives. [trade.rs:843](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/trade.rs#L843)

2. **[P1] Charge the venue cut and levy on personal bid fills.**

    Selling into a personal bid applies only the proceeds policy’s cuts. The bidder pays the posting fee, but the fill omits the 3% venue cut and 1% levy charged on an equivalent personal ask. Include both fees in the settlement split alongside any Co-op share, as Q-48-8 requires. [trade.rs:2773](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/trade.rs#L2773)

3. **[P2] Quote exact lots only at their full quantity.**

    A ten-unit exact lot accepts a quote for one unit, the command default. The inventory action tries to hand over the whole lot and rejects the quantity mismatch. Default to the full listed quantity for an exact lot and reject explicit partial quantities before taking funds. [trade.rs:355–356](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/trade.rs#L355-L356) · [lot.rs:915](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/lot.rs#L915)

4. **[P2] Preserve a personal bid across spending approval.**

    `NeedsApproval` discards the pending approval ID. A retry allocates a new listing ID, escrow purpose, and plan hash, so the original approval cannot authorize the bid. Persist the pending bid and resume the approved plan. [seller.rs:774–775](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/seller.rs#L774-L775)

5. **[P2] Recheck capacity when a personal bid receives the goods.**

    Every bid delivery carries a null capacity plan. Two fills can each pass the locker-space check before either lands, then both deposit their goods and settle. Carry the destination plan into the delivery action and check it before writing the payment marker. [lot.rs:532](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/lot.rs#L532)

6. **[P2] Count exact bulk lots in occupied capacity and locker rent.**

    The capacity read counts an exact lot only when its commodity is indivisible. Ten tonnes of ore bought as a lot therefore occupy zero measured space on the next purchase. Locker rent makes the same omission. Include lot tallies alongside pool balances in both calculations. [lot.rs:1131](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/lot.rs#L1131) · [locker.rs:246](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/locker.rs#L246)

7. **[P2] Include chartered asks in the Co-op statement.**

    The settlement callback accepts only sales into bids. A personal ask transfers the Co-op share on charter cargo, but the captain’s sale count, margin, and shares-paid statement do not change. Update the counters once for either settlement direction, keyed by quote and contract. [trade.rs:3366](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-market/src/trade.rs#L3366)

## Facilities, hulls, and operating costs

8. **[P1] Settle facility arrears from the seller’s proceeds.**

    An idle, empty facility passes admission with outstanding upkeep. The debt belongs to the facility room, so the next upkeep pass charges the new owner for the seller’s arrears. Q-48-45 requires clearing that debt from the sale before title passes. [facility_sale.rs:267](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/facility_sale.rs#L267)

9. **[P1] Keep the facility idle through settlement.**

    The settlement digest covers only `FacilityRecord`. A customer can book a yard after sale acceptance without changing that digest. The title can then pass with the customer’s work and escrow open. Lock new bookings during the sale or atomically recheck outstanding work and stores at transfer. [facility_sale.rs:274](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/facility_sale.rs#L274)

10. **[P1] Verify the hull’s actual cargo at title transfer.**

    A hold containing an exact ore lot reads as empty because `read_pool` counts lots only for parts. Cargo added after acceptance also leaves the hull-state digest unchanged. Plain and cored resales can both pass unpriced cargo to the buyer. Check pool balances and live lots at the authoritative transfer in both sale families. [hull_sale.rs:172](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/hull_sale.rs#L172) · [resale.rs:809–812](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/hardware/sale/resale.rs#L809-L812)

11. **[P1] Collect berth liens when a cored hull is sold.**

    Cored hulls must use `RegistryCoreResale`. Its settlement pays the seller and transfers the hull without collecting the berth lien handled by ordinary hull sales. Include the selling port’s outstanding berth debt in the cored-hull payout, as Q-48-54 requires. [resale.rs:817](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/hardware/sale/resale.rs#L817) · [hull_sale.rs:337](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/hull_sale.rs#L337)

12. **[P1] Revoke the NPC helm’s authority when the House loses title.**

    The persisted helm-to-hull row grants helm and trading authority without checking the current title. After sale or salvage, the House can still plan voyages and trade from the new owner’s hold. Require an active House-owned title both when resolving authority and before the autonomous planner acts. [hauler.rs:159–161](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/hauler.rs#L159-L161) · [owned.rs:446](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/owned.rs#L446)

13. **[P2] Bill player-created organizations for away berths.**

    `org_account` searches authored port organizations. A player-created House can fund a hull from its root treasury, but the berth obligation is skipped when its away-port lookup returns `None`. Resolve the root treasury as the yard and upkeep paths do. [berth.rs:242](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/berth.rs#L242)

14. **[P2] Preserve worked slot-hours across a restart.**

    The current day’s worked time stays in `PortTickState` until the day closes. Restarting shortly before rollover can reduce nearly a full day of busy work to the 10% idle charge. Persist the accumulator and cursor or reconstruct them from durable work records. [upkeep.rs:567](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/upkeep.rs#L567)

15. **[P2] Advance the refinery batch cursor only after charging.**

    If the upkeep transfer fails, the loop exits and persists the advanced batch counter. The retry sees zero new batches and skips the operating charge. Commit the counter with the successful debit or retain the amount due for retry. [upkeep.rs:613](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/upkeep.rs#L613)

## Production, freight, and jobs

16. **[P1] Fund House yard orders at executable market prices.**

    The House transfers exactly the bill calculated from authored asks, then its agent buys at live EX.8 prices. An ask increase can leave the agent short halfway through an order even when the treasury has ample funds. Price against live listings and handle price changes during procurement. [pools.rs:1377](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/pools.rs#L1377)

17. **[P1] Recover the House queue when a yard contract closes early.**

    A canceled building contract returns an error on every tick. The scheduler always advances `orders[0]`, leaving the agent’s funds tied up and blocking every later hull. Recover the canceled order’s funds and materials, then rebook it or remove it from the queue. [pools.rs:912–913](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/pools.rs#L912-L913)

18. **[P1] Pay passage escorts only for the contracted destination.**

    `journey_now` treats docking anywhere outside the origin as arrival. A shorter voyage to a different port can meet the proximity threshold and release the fee. Bind the watch to the agreed passage and require arrival at its named destination before publishing success. [escort.rs:455](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/services/escort.rs#L455)

19. **[P2] Enforce destination capacity on haul delivery and return.**

    The inventory carry checks neither mass nor volume. An already-full destination locker can receive the cargo and release the fee. The expiry path can also overfill the return locker. Check capacity in the cargo move and keep the reservation when the goods cannot land. [haul.rs:1315](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/services/haul.rs#L1315)

20. **[P2] Let NPC consumers post funded haul jobs.**

    The haul-posting path takes its shipper, account, and lockers from the command actor. EX.10’s hauling House trades for itself, so consumer demand never creates the open freight jobs described in Q-48-12. Add a House posting path and a story in which a player accepts and completes the haul. [haul.rs:646](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/services/haul.rs#L646)

21. **[P2] Reuse finished dies before checking raw stock.**

    A restart between issuing the depot’s last die and offering it leaves a finished, unoffered die on hand. With the raw inputs spent, the counter rejects further requests for that class. Check for a usable finished die before requiring the inputs for another production run. [counter.rs:475](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/hardware/counter.rs#L475)

22. **[P2] Include core production in the matter census.**

    The census counts the depot’s board and ore holdings but omits its opening stock as a source and the inputs consumed at die issue as a sink. Normal production changes the reported imbalance. Include the seed and retained issue records, then verify conservation across an issue and restart. [census.rs:300](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/census.rs#L300)

23. **[P2] Provide the PL.8 scrap path for loose damaged parts.**

    The House rejects a worn part, but the scrap operation accepts a hull. A part already stripped into a hold has no direct way to realize its worn scrap value. Add a workshop operation that consumes the loose lot and credits that value once. [market.rs:411](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/market.rs#L411)

## Combat, ammunition, and recovery

24. **[P1] Migrate saved units when changing published weapon definitions.**

    CC.2 changed the coax definition under its existing version. A saved line tank can still pin the old component digest at that locator. On boot, the digest conflict freezes the whole TacSim mechanics owner. Version the changed definitions, preserve or migrate their dependants, and boot a pre-CC.2 tank snapshot in a regression test. [mg_coax.json:4–6](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/content/weapons/mg_coax.json#L4-L6) · [module.rs:2173](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-tactics/src/module.rs#L2173)

25. **[P1] Keep ammunition recoverable until the bay reload commits.**

    The draw counts as loaded before `complete` rechecks the hull. If the hull departs or changes revision in between, the order cancels after consuming stock and the magazines stay unchanged. Commit the draw with the reload or return the drawn goods idempotently on cancellation. [mod.rs:2062](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/bay_work/mod.rs#L2062)

26. **[P1] Preserve ammunition across Co-op refits.**

    A refit rebuilds the tactical unit from its spec and discards persisted space magazines. A depleted Dray can refit to a Dray-P and check out with full point-defence magazines on shared mounts without buying ammunition. Carry compatible mount balances through refit and verify them after reboot. [module.rs:3003](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-tactics/src/module.rs#L3003)

27. **[P2] Conserve the unused rounds in a ground ammunition box.**

    A coax short by ten rounds draws a 100-round box and loads ten. The other ninety disappear. Persist the opened box’s remainder or load only whole units that fit. Check conservation in workshop and tender reloads. [jobs.rs:458](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/bay_work/jobs.rs#L458)

28. **[P2] Persist the bay’s repair of offline space subsystems.**

    `jobs::work` clears the offline-subsystem capability on a temporary unit. The committed plan carries condition and magazine changes only, so the subsystem can remain offline at the next checkout after a paid repair. Include the capability removal in the authoritative repair mutation. [mod.rs:1937](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/port/bay_work/mod.rs#L1937)

29. **[P2] Fire only the laser mount selected by ID.**

    Selecting a single laser emits `FireOffensiveLasers`, which fires every eligible offensive laser on the hull. A two-laser fit spends heat and starts cooldown on both mounts. Carry the selected mount ID into the kernel command to implement CC.6’s per-mount firing contract. [applier.rs:727](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/space/applier.rs#L727)

30. **[P2] Show overheated defence lasers as unavailable.**

    Defence mode returns `Ready` before the heat check. At 900 heat, a laser costing 140 appears ready to both pilot and mind even though automatic defence refuses to fire. Apply the kernel’s heat eligibility rule before reporting readiness. [view.rs:534–535](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/space/view.rs#L534-L535)

31. **[P2] Enforce the five-second floor on authored offensive space mounts.**

    The shared validator accepts a 250 ms cycle for spinal guns, tubes, and offence-mode lasers. The shipped rows meet Q-49-19, but a content pack can violate it. Validate offensive cycles against five seconds and retain the shorter floor for automatic defence. [space_mount.rs:278](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/definitions/space_mount.rs#L278)

32. **[P2] Resolve space fits from the world’s loaded content.**

    The custody catalogue is built with an empty pack-path list. An operator-supplied fit can load into the world yet be absent at checkout, where the hull silently receives the default Dray fit. Use the world’s resolved catalogue for checkout and berth rearm. [custody.rs:394](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/src/space/custody.rs#L394)

33. **[P2] Count burst rounds when enforcing the bot’s reserve.**

    One firing acknowledgement subtracts one round, while a coax burst spends ten. After firing from 160/600 the bot believes it has 159 rounds and can fire past its 150-round reserve before the next status poll. Reconcile against player-visible ammunition or refresh the terminal reading at the reserve boundary, consistent with the bot’s SSH-only architecture. [picture.rs:1626](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-bot/src/arena/picture.rs#L1626) · [Bot architecture](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/mush-bot/AGENTS.md#L12)

## Player interfaces and acceptance evidence

34. **[P2] Keep all 24 weeks of prices outside the 18-line modal log.**

    The server returns the newest prices first. The shared log discards them before the parser reads the series. With a current price of 100.00 cr, the panel displayed 94.00 cr from six weeks earlier as its headline. Capture the complete history response and reset it for each venue and good. [TacsimPortModals.svelte:51](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/web/src/components/TacsimPortModals.svelte#L51)

35. **[P2] Let players confirm a survey acceptance from the panel.**

    Accept Survey sends the preview command. The server requires a trailing `go` to commit, and the panel offers no confirmation action. Unity sends the same preview-only command. Wire the preview to an explicit confirmation in both clients. [TacsimPortModals.svelte:441](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/web/src/components/TacsimPortModals.svelte#L441) · [TacSimEconomyPanels.cs:353](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/unity/Assets/_TacSim/Scripts/Runtime/UI/TacSimEconomyPanels.cs#L353)

36. **[P2] Measure EX.9 balance with the launch economy running.**

    The one-percent test lays up NPC haulers, advances the clock in six-IC-hour jumps, and exercises the launch stipend in a separate test. The required multi-day 24:1 run with both active remains unverified. Keep the deterministic test and record daily capital, sources, and sinks from the launch-world run on conclav.dev. [ex9_balance.rs:571](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/tests/domain_economy/ex9_balance.rs#L571) · [48-cut-notes.md:4399–4400](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/docs/design/48-cut-notes.md#L4399-L4400)

## Further limits of the acceptance evidence

- **EX.10 reserve replenishment.** The Deimos consumption test seeds the reserve’s water and carbon directly. The delivery story hauls boards or alloys to a different buyer. Neither story follows a freight delivery into the reserve that consumes the goods. [Reserve setup](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/tests/domain_economy/ex10_haulers.rs#L424-L429) · [Delivery story](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/crates/tacsim/tests/domain_economy/ex10_haulers.rs#L448-L462)
- **CC.10 bot performance.** The cut notes record that both report-only lanes, `arena_win_rate` and `arena_muster_rate`, were skipped. The focused passes above therefore leave the effect of the new reserve discipline on those match results unmeasured. [CC.10 run record](https://github.com/conclavio/rustmush/blob/480df36ed28ec4068a2f1fef3238a82251977ca0/docs/design/49-cut-notes.md#L2285-L2287)
