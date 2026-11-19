# 50a. Outside review of doc 50 (2026-11-19): triage

Docs only. On 2026-11-19 a design review of
[PR #676](https://github.com/conclavio/rustmush/pull/676) was posted as one comment on the PR. It
read `d4ff2da20`, [doc 50](50-law-and-the-corridor.md) Draft 0.1, and it assumed every default from
Q-50-1 to Q-50-49 was taken as written. The owner asked to "fetch the comments, analyze their
factualness and address the real concerns". He spoke of two review comments; the PR holds one.

**How it was checked.** Five read-only lanes against the tree at `d4ff2da20`, whose code is main's
at `1e9b41174`: the faction engine; the campaign engine and its seams; the market, inventory and the
picket's reading of cargo; port security, voyages and space bubbles; jobs, escorts, the Co-op and
the lane's content. Each lane was told to try to refute the claim. The lead read three of the
gravest claims in the code before the lanes reported (A-1, A-4 and C-1). Where a lane disagreed with
the review, the code decides. The review's links pointed at GitHub; here they are repo paths.

**Wording.** Combat findings are stated at the level of game mechanics (the owner's rule of
2026-11-02).

## 0. In brief

- **About 55 claim parts.** About 40 held, about a dozen held in part, and four were refuted.
- **The review was right about the three things that matter most.**
  - **The control flip does not displace.** A filled clock adds the new holder's record beside the
    old holder's, so a region ends with two de facto holders. The record's date is wall-clock time
    and nothing reads it. A resolved front cannot reopen.
  - **The campaign engine cannot frame the Isidis operation as built.** Q-50-18's "each used as
    built" was the lead's claim and it was wrong in five ways: the objective fires mid-window and
    for the wrong side, the resolution record does not say who won, organizations cannot be sides
    or defenders, nothing outside a staff command can declare, and TacSim cannot read an operation
    without a seam at the composition root.
  - **The picket cannot see ore held as a lot.** A bulk good in an exact lot is outside the pool
    the picket reads. A buyer of a personal lot receives one, so a privately sold tonne of strategic
    ore is invisible to the picket, to the transit duty and to any audit built the same way.
- **It was also right that doc 50 had the papers rule backward.** Clearance needs a license **and**
  a registered claim. Doc 50 said "or" in two places.
- **It was weakest in its examples.**
  - An escort cannot pay out at Kirkwood, because nothing docks there.
  - A manual `+helm/dock` does reach port security.
  - `port/house_arm.rs` is Tycho Mutual's retrieval arm, not doc 48's EX.10, which is not built.
  - Two hostile player hulls cannot deadlock in a bubble today, because two players' hulls cannot
    be hostile today. The deadlock is real only once Q-50-26 is built.
- **Eleven findings are live defects in the tree today**, whatever doc 50 decides. They are listed
  in doc 50 §4.3 for the fix thread.
- **The owner ruled three times on 2026-11-19** (§5). All 71 questions are ruled.
  - First: "defaults not addressed specifically by the reviews are accepted as written". Eleven
    questions are ruled that way.
  - Thirty-four defaults were then corrected in place, four stood unchanged, and twenty-two
    follow-up questions, Q-50-50 to Q-50-71, took the choices the findings need (doc 50 §6.6).
  - Second: asked whether to accept those open defaults and follow-ups apart from five held for
    his own call, he answered "Accept those". That rules 55 more.
  - Third: shown the five, Q-50-55, -57, -62, -65 and -67, with each default and its
    alternative, he answered "Take all five defaults".

## 1. Engine and seam findings (the review's 2A)

The review's second item, on the campaign seam, is split here into A-2a to A-2h, and its eighth,
on escorts, into A-8a and A-8b. Doc 50 cites "A-2" and "A-8" where it means the whole group.

| # | Finding | Check | Verdict | Change in doc 50 |
|---|---|---|---|---|
| A-1 | A filled clock's control flip leaves the prior holder's record; `since` is wall-clock time; a resolved front cannot reopen and a clock cannot be set back | `mush-faction/src/lib.rs:285-301`: `recs.retain(\|r\| !(r.faction == clock.faction && r.kind == kind))`, then a push with `since: now` (`now` is `ctx.w.now()`, wall time; the IC time on the line above goes only to the canon draft). No reader of `.since` in any crate. `Clock::advance` returns at once when completed (`sim.rs:103-113`); `@clock/set` takes no count; `clocks_watching` skips a resolved front (`sim.rs:153-162`). Control is written only by staff verbs (`@faction/control`, `/uncontrol`, `/contest`); no public function writes it | **held**, all three | §3.1 and Q-50-1, -9, -16, -31 corrected; Q-50-50 (one de facto holder, an IC date) and Q-50-51 (a front's lifecycle) |
| A-2a | The campaign tick resolves a running operation itself; a `CanonRecorded` objective fires `"success"` the moment the record exists, mid-window, and the success is the declarer's | `mush-campaign/src/tick.rs:291-355`. `WindowElapsed` is met only at close and so always succeeds; `Muster` is read at close; `CanonRecorded` is read at every fire with no time filter. With the Republic declaring, a full tank farm would resolve the operation in the Republic's favor | **held** | Q-50-18, -21 and -28 corrected; Q-50-57 |
| A-2b | The canon seam it reads is "8 records" and never refreshed | `mush-canon/src/lib.rs:263, 283-307`: eight a source, after filtering by source. The real loss is that a record leaves the seam when its chunk seals, until a reboot (`data.rs:184-219`) | **held in part**: the bound is narrower than stated; the loss is real for a long window | Q-50-57 (no objective reads canon) |
| A-2c | Neither success nor failure forfeits the bond | `settle.rs:497-499, 630-636`: only `"forfeit"`, a staff outcome, captures it, and only to an object principal | **held** | Q-50-57 |
| A-2d | One canon kind for both outcomes, the outcome only in `detail`; the event carries no detail; the faction observer matches the kind alone | `settle.rs:239-258`; `mush-core/src/module.rs:941-960` (`canon_id`, `kind`, `entities`, `source`); `mush-faction/src/lib.rs:219` discards `source` too. A clock watching the kind advances on any operation's resolution, whoever won | **held** | §3.4 corrected; Q-50-57 |
| A-2e | Organization principals are dropped from the record's entities, and a kind outside `WORLD_KINDS` advances a clock only for a member of its faction, so an empty run advances nothing | `settle.rs:220-226`; `lib.rs:236-240, 1236` (`WORLD_KINDS` is `minds.unwarranted` alone). Entities also hold committed units, which count only if they are themselves members | **held** | Q-50-47 and Q-50-51 (polity clocks move by typed owner calls) |
| A-2f | Nothing but `@op/*` can declare or activate; declarer, sides and defenders are object principals only; an organization named as defender is notified of nothing | `commands.rs:63-65, 662-682`; `render.rs:79-85`; `notice.rs:108-111`. `mint_operation` is `pub(crate)`. Public: reads through `CampaignStore`, `enter_resolving`, `abandon`, `advance_one` | **held** | Q-50-19 corrected; Q-50-57 |
| A-2g | A declared operation with a bond is paused "unfunded" before `@op/activate`, and activation never clears the pause | `tick.rs:186-235`; `commands.rs:810-813`. Any tick between `@op/terms bond=` and `@op/activate` triggers it; the operation then never runs. No test fires the tick on a bonded, declared operation | **held**: a live defect | §4.3 |
| A-2h | TacSim is pinned without a campaign or mail edge, and the campaign crate without a faction edge, so "TacSim gains no campaign edge" cannot hold | `tacsim/tests/domain_arena/composition_edges.rs:59-84`; `mush-campaign/tests/e7c9.rs:1590-1599`. Canon is one-way: it can trigger an objective and cannot declare, read or resolve | **held in part**: no direct edge is needed. `mush-composition` already wires the campaign crate through injected builders so that neither pin moves (`mush-composition/src/lib.rs:428-437`) | Q-50-18, -26 and -30 corrected; Q-50-57 |
| A-3 | Legality sits on the global catalog entry, so it cannot differ by venue | `mush-market/src/record.rs:325-326, 694-751`. It is read only by a list filter and a label | **held in part**: a seam that sees the good already exists. `ProceedsPolicy::proceeds` gets seller, venue, commodity, quantity and price and may refuse (`module.rs:324-336, 408-417`); it is asked when a player sells into a bid and when a personal lot ask is bought. `may_list` runs at posting and sees no commodity | §0, §3.6 and Q-50-33 corrected |
| A-4 | `read_pool` ignores lots for a bulk good, so ore held as a lot reads as nothing at the picket | `tacsim/src/port_tick.rs:2610-2615`; `mush-inventory` `pool.rs:2169-2171` (crystallizing debits the pool). The picket reads only the pool (`space/picket.rs:693-700`). The hold's fill check undercounts the same way (`space/mining.rs:1301-1320`) | **held**. How lots arise was loose: a buyer of a personal lot ask receives the exact lot; `@inv/crystallize` and `@inv/split` are not staff verbs; mined-only ore probably cannot be crystallized (inferred) | §3.2, §4.3 and Q-50-6, -36 corrected; Q-50-53 |
| A-5 | A seizure of cargo under a haul's hold always fails, and the ruling lapses | `space/customs.rs:468-478` takes `min(seize, available)`; a haul's reservation is not available. It fails only when all the ore is held, and only for a principal who cannot pay; then three attempts and "she is not cleared", and a new submission repeats it | **held in part**; a live defect in that case | §4.3; Q-50-25 and Q-50-44 corrected |
| A-6a | A hold audit cannot run in `security::on_docked`: the arrival hook cannot read pools | `composition/sim_routes.rs:194-195`; `port/voyage.rs:128-129` | **held** | Q-50-36 corrected (the audit runs on the port tick) |
| A-6b | A manual `+helm/dock` never calls `on_docked` | `space/commands.rs:5734` → `mush_sim::dock_carrier_at_destination` → `apply_arrival` → `hooks.arrived` → `on_docked`. Only a hull with no journey record skips it | **refuted** | none |
| A-7 | The Republic cannot post a retrieval on a hull it does not own | `mush-economy/src/contract_live.rs:6772-6790`; `surface/retrieval.rs:972-990`. A retrieval is the owner paying to have their own hull brought in | **held** | Q-50-27 corrected |
| A-8a | An escort post names one docked hull the poster holds, so an organization cannot post one, nor anyone before the hauler's hull is known | `services/escort.rs:721-757, 791, 806`. "Personally held" overstates it: the helm, the title, a live rig lease and the seat all count | **held in part** | Q-50-22 and -23 corrected; Q-50-63 |
| A-8b | Three escort defects: it ends at any port but `from`; it fails "left" if the escort is still docked a fire after the protected hull leaves; it ignores capture in space | (i) `:454-455`: true only for a passage of several hops, where it settles at the first port; the review's Kirkwood example cannot happen. (ii) `:476-486, 594, 605-607`: true; the escort has one port fire. (iii) `:440-442`: no live defect, since space has no capture of an operational hull | **held in part** (i), **held** (ii), **refuted** (iii) | §4.3 (i and ii); Q-50-63 |
| A-9 | Two hostile player hulls in a bubble deadlock | `space/tick.rs:521-524`; `commands.rs:2595-2602, 3130, 5340-5346`. No flee verb, range, timer or surrender exists in space. But every carrier joins a bubble friendly to every other (`tick.rs:5272-5277`), and no player verb sets a hostile mark | **held in part**: not today; certain once Q-50-26 makes two players' hulls hostile | Q-50-26 corrected; Q-50-62 |
| A-10a | `Faction.org` is one to one, so several lane organizations cannot bind to one polity | `data.rs:301` (a faction has one org at most); `econ.rs:89-92, 371-378` (an org backs one faction at most). The review named the wrong half as the cause | **held in part**: the conclusion holds | Q-50-2 corrected |
| A-10b | An organization's hull answers to nobody at port security | `port/owned.rs:470-481`; `surface/security.rs:314` | **held**: a live defect | §4.3; Q-50-68 |
| A-10c | A quarantine blocks departure at every port and site | `port/voyage.rs:3456-3458`: no venue test; `answers_at` is used only in `on_docked` | **held**. Whether it is intended is not stated in the module | §4.3; Q-50-38 corrected; Q-50-68 |
| A-10d | Berth and locker obligations never compare the payee | `port/berth.rs:112, 131`. Crew wages do, and re-point (`detachment/crew/wages.rs:281-306`) | **held**; latent, since no authority changes today | Q-50-29 corrected; Q-50-64 |
| A-10e | Registry seizures carry no owner; Registry mind fines are missing from `+debts`; the 35 km breach names the hull as the pilot and writes no ruling | `mush-roll` `registry_inspection/seizure.rs:133-178`; `surface/retrieval.rs:1255-1279`; `space/tick.rs:4549` against `picket.rs:1604-1608`. "0-day grace" is wrong: due at once, one period of grace | **held**, but for the grace | §4.3; Q-50-70 |

## 2. Loops and contradictions under the defaults (the review's 2B)

| # | Finding | Check | Verdict | Change in doc 50 |
|---|---|---|---|---|
| B-1 | The standing sizes do not reach the bands the penalties name | `mush-faction/src/reputation.rs:510-533`: hostile to −50, unfriendly −49 to −10, neutral −9 to 9, friendly 10 to 49, trusted from 50; standing starts at 0. From 0, −3 or −5 stays neutral. The store holds 16,384 receipts a map and never evicts (`:18, 116-118`) | **held** | Q-50-36, -39, -42 and -47 corrected; Q-50-69 |
| B-2 | "A seized Freeport-flagged hauler" can never be an Isidis supply hull, and no hull has a flag | The picket seizes only unlicensed strategic ore from one who cannot pay; the stockpile goods are lawful. A hull's ties are its title, a Co-op charter, the owned register and field sides taken from people (`port/owned.rs:78-91`, `field/parties.rs:90-125`) | **held** | Q-50-20 corrected; Q-50-61 |
| B-3a | The long way round carries no picket | The lane's one interdicted leg is `ceres_stickney` leg 0 (`ceres_phobos.json:2384-2404`). `ceres_aitken`, `aitken_voltaire`, `voltaire_stickney` and the outward `stickney_ceres` carry none. `port/house_arm.rs:128-132` calls it "the lane's own picket-free way" | **held** | Q-50-6 corrected; Q-50-55 |
| B-3b | Free-flight autopilot skips the owed leg, and writes no pending voyage to pin terms on | `space/commands.rs:3549-3562, 3766-3779`. From Ceres' halo to Deimos: nothing owed, **held**. From the halo to Stickney: the leg is owed, **refuted** | **held in part** | §4.3; Q-50-10 corrected; Q-50-55 |
| B-3c | An absent pilot's queued departure, and "EX.10's NPC haulers", cannot answer the picket | The link-loss record is written only by the presence hooks (`space/tick.rs:2841-2846, 2924`); a bubble seats a crewless hull with no thrust (`:5710-5719`). What follows was inferred, not traced. `house_arm.rs` is Tycho Mutual's retrieval arm; EX.10 is not built. Its comment does say an NPC house never answers the customs bubble | **held in part**; partly unsettled | Q-50-24 corrected; Q-50-54 and Q-50-56; §8 |
| B-4 | Selling into a bid is a second public way to sell, unchecked; a private ask pays the port its fees | `mush-market/src/trade.rs:2507-2946` runs the presence gate and the proceeds policy and charges no listing fee, cut or levy. A private ask pays the 1% listing fee and the 3% cut to the port's authority and the 1% levy to the treasury (`seller.rs:340-371`, `trade.rs:865-866`). Not raised: a private bid sold into pays no fee at all | **held** | Q-50-34 corrected; Q-50-71 |
| B-5 | A private sale leaves no bystander record, no IC time and no snapshot of the seller's papers; a reward with no cost to the buyer invites informing for a rebate | `record.rs:918-927, 1002-1122`: the settled quote holds the parties, good, venue, price, quantity and a wall-clock time. Tape rows are capped (32 a pair, 1,024 in all) and evicted. The buyer reads it with `+market/orders`; the seller has no verb | **held** | Q-50-35 and -43 corrected; Q-50-66 and Q-50-67 |
| B-6 | Raising the Co-op's share for new enrollments barely touches play, and the copy hard-codes "a quarter" | `port/coop.rs:58-59, 93-97, 286-287, 1386-1480, 1606`. One enrollment a gateway account, frozen at signing; a graduate never enrolls again. Missed: a pilot whose charter ended by dormancy or release may recharter, and signs at the current terms | **held in part** | Q-50-7 corrected |

## 3. Facts doc 50 had wrong (the review's 2C)

| # | Doc 50 said | The tree says | Verdict | Change |
|---|---|---|---|---|
| C-1 | Ore is unlicensed with "no license and no registered claim"; sold "under license or claim" | `space/picket.rs:760`: `!licensed \|\| !has_registered_claim`. Both are needed. The claim is any live claim on a strategic rock in an orbital cluster, or every mined block when the hull has mined; surface claims never count. The review's "never bought ore" is wrong: bought ore clears for a licensed holder of such a claim | **held**, but for that clause | §3.2 and Q-50-32 corrected; Q-50-65 |
| C-2 | Port security "quarantines the hull until paid"; "the shortfall an obligation" | `surface/security.rs:328-353, 462-469`: 28 IC days of grace before any quarantine, and payment is all or nothing. The review's "immediate" is its own word; doc 50 never used it | **held** in substance | §0, §3.6 and §4.2 corrected |
| C-3 | Five `faction` tags; the bring-up makes the Republic and the three Houses | Six tags (`townships`, `ceres_phobos.json:3114`), on 24 organizations. `launch-bringup.py:883-914` makes one faction, the Republic, with no backing organization; the Houses are organizations | **held** | §3.1 corrected |
| C-4 | §3.3's table of charges | It omits NPC crew wages, paid to the hiring port's authority (`detachment/crew/wages.rs:141-146`), and, among charges not paid to an authority, the license fee and port security's fines | **held** | §3.3 corrected |
| C-5 | `space/tick.rs:5894` for the friendly-carrier rule | That line is in a test module. Production is `:5272-5277` | **held** | §3.4 corrected |
| C-6 | §3.6's rows for the bay, the field and the Registry | No owner can strip or clear a hot part in TacSim (`port/bay_work/strip.rs:261-263`: dormant). The capture's standing loss is not voidable (`field/crew.rs:1014-1017`); field fire counts all fire between parties and only friendly fire is voidable. `@registry/inspect` gathers evidence; `/fine`, `/revoke`, `/seize` and `/forfeit` sanction. A capture carries no fine. `witnessed()` has no time test on custody evidence, and its one record a site is overwritten on a return visit (`security.rs:588-602`, `site_record.rs:798-830`). "Not from the black box" is too strong: the capture's test does read field fire receipts | **held**, but for that clause | §3.6 corrected; §4.3 |
| C-7 | "Mail is in character and must transit" | Letters do. An official notice does not: `mush_mail::post_notice` delivers at once (`transit.rs:483-516`) | **held** | §4.2, Q-50-30 and -40 corrected |
| C-8 | The matter census "has no term" for a seizure | It has one (`port/census.rs:38-39, 139-140`; `SpaceBurnReceipts.seized_t`). Nothing feeds it: both callers pass 0 and the customs seizure records nothing | **held** | §4.3 and Q-50-44 corrected |

## 4. What doc 50 §8 had not checked (the review's 3A)

All six answers held, and one of the review's own supporting facts needed a correction.

| # | Open item in §8 | Answer | Change |
|---|---|---|---|
| D-1 | Where a passage is priced before launch | `+ship/plan [<port>]` is a dry run that shows the waypoints, the burn and the ETA and writes nothing (`port/voyage.rs:2132-2267`). It shows no money. `+ship/undock` files clearance | Q-50-8 names it; §8 |
| D-2 | Whether a lot records its license or origin | No, by rule: "not `legal`, not `custody`" (`mush-inventory/src/record.rs:8-13`) | Q-50-66; §8 |
| D-3 | The trade tape's visibility | Private trades are kept off `+market/depth` (`projection.rs:250-257`); there is no tape verb | Q-50-66; §8 |
| D-4 | A seeded roll in the port tick | None. The precedents are a keyed HMAC (`surface/geology.rs`) and SHA-256 (`port/survey/standing.rs`) | Q-50-36 corrected; §8 |
| D-5 | `+territory` today | "No territory is mapped." (`mush-faction/src/commands.rs:1688-1691`) | §8 |
| D-6 | Doc 11 §9's questions | Still unruled. §9 question 2 asks whether Isidis has two de jure claimants; §5's table already assumes it has | Q-50-52; §8 |
| D-7 | (The review's support for Q-50-17) no Dray can land on Mars | True: 131‰ laden and 231‰ empty against a floor of 1,200‰. The gate applies to a location whose role is a site (`port/passage.rs:235, 327`), not to a `site` field | Q-50-17 notes it |

## 5. The owner's rulings

**The first ruling, 2026-11-19:** "defaults not addressed specifically by the reviews are accepted
as written". The review raised nothing against eleven:
- never cited: Q-50-3, Q-50-12, Q-50-13, Q-50-14, Q-50-41, Q-50-48 and Q-50-49;
- cited only in agreement: Q-50-5, Q-50-11, Q-50-45 and Q-50-46.

**The second ruling, the same day.** The lead set five questions aside for the owner's own call
and asked whether to accept the rest of the open defaults and follow-ups. He answered "Accept
those". Doc 50 Draft 0.3 records it. It rules 55 questions:
- **as written:** Q-50-8, Q-50-15, Q-50-16 and Q-50-17, which the review cited and the triage
  left unchanged. The review's facts either support them or are carried by a follow-up;
- **as corrected:** the other 34 from Q-50-1 to Q-50-47. Each correction is marked **Correction
  (outside review, doc 50a …)** with the row above that caused it, and it governs;
- **as proposed:** seventeen of the follow-ups in doc 50 §6.6: Q-50-50 to -54, -56, -58 to -61,
  -63, -64, -66 and -68 to -71.

**The third ruling, the same day.** The lead laid out the five with each default and its
alternative, and the owner answered "Take all five defaults". Doc 50 Draft 0.4 records it. All
five are accepted as proposed:
- **Q-50-57:** `mush-campaign` frames the operation, with five additions built as their own
  unit first. TacSim keeps no operation record of its own. It is marked expensive to reverse.
- **Q-50-65:** one papers test, a license and a registered claim, for buying, holding and
  selling. The board refuses an unlicensed buyer, and possession is an offense an audit finds.
- **Q-50-62:** a hull may break off from a fight in an operation's bubble. With it, Q-50-26 is
  built for the first operation.
- **Q-50-55:** the long way round stays free of a picket, the shortcut closes, and the outward
  run stays free.
- **Q-50-67:** a buyer who reports hands over the goods, and is repaid from the collected fine.

No question is open. Doc 50 §6 says what the third ruling settled in answers already ruled.

The 22 follow-ups cover the review's fourteen "missing contracts" as follows.

| The review's item | Verdict on the gap | Question |
|---|---|---|
| 1. An unpaid transit duty; an absent pilot | real | Q-50-54 |
| 2. The dutiable base and its exemptions | real | Q-50-53 |
| 3. The bypasses, and pinning on autopilot | real | Q-50-55; Q-50-10 corrected |
| 4. Two de jure holders; polities against one-to-one `Faction.org` | real | Q-50-52; Q-50-2 corrected |
| 5. Faction-engine changes for the first units | real | Q-50-50, Q-50-51 |
| 6. Possession against sale; the lawful buyer | real | Q-50-65 |
| 7. Selling into a bid; private-offer fees | real | Q-50-34 corrected; Q-50-71 |
| 8. Station-bound offenders; character, organization and account | real | Q-50-68 |
| 9. Standing thresholds; the witness's proof; the reward | real | Q-50-69, Q-50-66, Q-50-67 |
| 10. The scope of `+record` and `+debts` | real | Q-50-70 |
| 11. The engine contract between campaign, faction and TacSim | real | Q-50-57 |
| 12. Where the import duty falls; Isidis's routes; the stockpile on a repeat | real | Q-50-59, Q-50-58, Q-50-60 |
| 13. A hull's side; leaving a hostile bubble | real | Q-50-61, Q-50-62 |
| 14. Escorts for a haul; the Republic's jobs; NPC hulls at the picket; payees on a flip; notices | real, but for the notices, which Q-45-17 already rules | Q-50-63, Q-50-27 corrected, Q-50-56, Q-50-64 |

## 6. What changed, in one place

- **Doc 50 Draft 0.2:**
  - the owner's ruling recorded on eleven questions;
  - corrections of fact in §0, §3.1 to §3.4, §3.6 and §4.2;
  - eleven live defects added to §4.3;
  - 34 defaults corrected in place in §6.1 to §6.5;
  - follow-ups Q-50-50 to Q-50-71 in §6.6, open;
  - §7's sketch gains the engine work the first units need (KC.0a, OP.0) and a cargo-reading fix
    that gates every unit that measures a hold;
  - §8 loses the six items answered here and gains what the lanes could not settle.
- **Doc 50 Draft 0.3:**
  - the owner's second ruling recorded on 55 questions, and the five left open marked;
  - doc 50 §6 lists the ruled answers that wait on an open question, and §7 the units whose
    questions are all ruled;
  - a note in doc 48 at Q-48-15, which Q-50-6 reverses, and in doc 11 §9 at question 2, which
    Q-50-52 answers;
  - no default's text changed.
- **Doc 50 Draft 0.4:**
  - the owner's third ruling recorded on the five, so all 71 questions are ruled;
  - doc 50 §6 says what that settled in the answers that had waited, and §7 that no unit waits
    on a question;
  - no default's text changed, and no unit is cut.
- **Not changed:** the model of §5. Control in the faction engine, an authored schedule in the lane
  pack, compiled arms and a port for Isidis all survived the review, which says so.
