# 38. After the IC phase — the next systems, surveyed

Draft 0.2, 2026-11-03, docs only. Surveyed against `origin/main` at `afa082a7` (doc 36a's lanes 1,
2 and 4 merged as PRs #334, #343, #342; the f6 fix as #329; lane 3 open as PR #340 at `e9ccb835`
with main merged in; lane 5 on a local branch with no PR). Four read-only lanes verified every
claim the two outside reviewers made against that tree; the anchors below are theirs. Draft 0.1,
the same day, was the survey. **Draft 0.2 records the owner's rulings of 2026-11-03 on every
question:** Q-38-5, Q-38-6, Q-38-9, Q-38-10, Q-38-12 and Q-38-18 amended, every other default
accepted as written. §4a lists what the amendments change elsewhere.

This is a survey in the form doc 36 §4 used: it asks the owner questions, each with a proposed
default, an alternative and a reason. "Provisional" marks a number to be tuned later under the
2026-10-24 rule. **(expensive to reverse)** marks an answer that would take a significant refactor
of the codebase to undo — not stored data and not a live box: there are no continuous games yet,
and every step may wipe and redeploy (the owner's ruling on Q-38-9 and Q-38-10). Draft 0.1 used the
wider meaning; Draft 0.2 keeps the marker on Q-38-12, Q-38-17 and Q-38-27 only. The implementation
plans are written from these rulings, in the documents §6 proposes.

## 1. Why this document exists

The IC phase closed on 2026-11-02 ([doc 37](37-ic-phase-closeout-2026-11-02.md)). Its §7 ordered
the next areas from the close-out's own view of the tree. The same day two outside reviews of the
ground-combat code arrived; their defects were triaged in
[doc 36a](36a-ground-combat-review-triage-2026-11-02.md) and are being fixed in their own thread.
Each reviewer also sent a list of the ten systems they would build next. The owner then flagged a
gap neither list has: **unit construction rules, and the economy engine's chain from ore to refined
goods to parts**, because that chain is what every repair and refit conversation keeps reaching for.

This document rolls those four sources into one survey:

- the decisions still waiting on the owner (doc 36a §9, doc 37 §6);
- the feature backlog docs 36 and 37 left (doc 36 "What the programme still owes", doc 37 §5, §7);
- the two reviewers' "next ten" lists, each claim checked against the tree (§3);
- the production chain, surveyed for the first time (§4 A).

It is not a bug list. §2.2 names what the fix thread holds so nothing here re-surveys it. Its
product is the set of categories in §4, the owner questions in §5, a proposal for which category
gets which design document in §6, and a provisional order in §7.

## 2. Sources, and what stays with the fix thread

### 2.1 Sources

| Source | What it gives this survey |
|---|---|
| Doc 36, "What the programme still owes (after GC.6)" with the lane 1, lane 2 and F-6 as-built notes | the ground-combat backlog as the cuts left it, struck through where a lane closed an item |
| Doc 37 §5 (owed by programme), §6 (owner decisions), §7 (next areas in order) | the phase-wide backlog and the close-out's ordering |
| Doc 36a §9 (five owner questions), §11 (the second review, routed to lanes 2, 3 and 5) | the decisions the review surfaced, and which review items are features rather than defects |
| Reviewer 1 (ten features, ranked) | S.4b, field engineering, sustained battles, component effects, diplomacy, reinforcement, recovery under fire, a parts economy, capture, the salvage loop |
| Reviewer 2 (ten systems, ranked) | seam hardening and the rebless, S.4b, field engineering, capture, coupled towing, reinforcement and DT.4, salvage adjudication and crime, stances and standing, component damage and the part market, the bot and the dual-scale suite |
| The owner, 2026-11-03 | the production chain and unit construction as a probable lead |
| The tree at `afa082a7` | four verification lanes (kernel and seams; the bay, salvage and economy; field, crew, bot and tests; the ore-to-parts chain) |

Doc 36a §11 (the second review's nine findings) is included in this PR: its commit was pushed to the
triage branch after PR #330 merged and never reached `main`.

### 2.2 Left to the fix thread

These are defects or seam fixes with an owner already. They are named once so the categories below
can refer to them, and not surveyed:

| Item | Where it is |
|---|---|
| `ArenaMsg::Join` applied before the previous pulse's think batch is awaited (`arena_task.rs:1869–1872` against `:1937`), so a join's snapshot refuses (`snapshot.rs:641`); GC.1b's refused arrival never retried (`field/carrier.rs:562–579`, cleared only when the field is gone, `:308–309`) | doc 36 "the field host"; GC.11's deferred root fix; the next rebless (§4 I) |
| `Relations::note_fire` (`arena/party.rs:340–357`): a friend of **both** parties *is* moved today; the owed fix is to skip parties that hold the shooter a Friend (doc 36 l.2785–2792). Reviewer 2 has the direction backwards | the next rebless |
| `ConditionTable` built as a scratch table and dropped at `join.rs:143/152` and `state.rs:4717/4904` | P1-9 in PR #340 (mind-controlled hulls only) |
| P0-5 `carried_rig` on a non-logistics hull; P0-8 `set_down` after an irrecoverable tug; the held-out walls lost on every rebuild | PR #340 |
| Lane 5's five (party letters, check-in healing, DT launches past the cap, admission-time leader for marks, the seated subordinate's fire) | local branch `claude/youthful-kowalevski-168ddb` at `50cdbe03`, no PR |
| `+tv/<n> scrap` on remains has no room check (`claim.rs:1585–1680`; the prepare mints at `tow.rs:1740–1800` with no capacity rule); lane 2's `strip::hold_takes` (`strip.rs:96–123`) is the check to mirror | doc 36 lane 2 note; unowned |
| `gc11_a_scheduled_raid` and `cf6_a_seeded_raider` fail under a shared module load on `main` itself (`403bdeae`, and with P1-9 reverted), and pass alone | lane 3's finding, 2026-11-03; unowned |
| The matter census pin counts `.mint_origin(` only; scrap mints through `.mint_origins(` (doc 37 §5.1) | unowned; §4 A takes it as a rule |
| 35-cut-notes has no section for SF.R9–SF.R13 (its headers end at SF.R8) | docs owed, the space track |

## 3. The two reviewers' lists, mapped

Each row is one reviewer item, the category it lands in (§4), and the one-line verdict of the lane
that checked it. CONFIRMED means the gap was traced in code; REFUTED means the tree already does it.

| Reviewer item | Category | Verdict |
|---|---|---|
| R1-1 licensed minds in ground combat (S.4b) | E | CONFIRMED: `fixture_start`/`fixture_close` are `#[cfg(test)]` (`participant.rs:499–549`); `earned_contacts.rs:4` "No producer yet" |
| R1-2 field engineering and resupply | B | CONFIRMED: `field_unjam_component`, `field_stabilize_damaged_section`, `field_resupply_ammo_and_charge` (`engineering.rs:237/400/558`) have test callers only; the bay works only `Berth::Docked` at a workshop port (`bay_work/mod.rs:203–237`) |
| R1-3 sustained battles (bounded journal, checkpoints) | I | PARTLY: a checkpoint exists (snapshots every 5 s carry `LedgerState`; `boot.rs:228` resumes with `segment += 1`), but `bytes_total` is cumulative against `max_bytes` 64 MiB (`config.rs:1626`, `ledger.rs:1033`) and no resume resets it; "18 minutes" is doc 36 l.1805's estimate from 7.4 MB in two minutes |
| R1-4 component damage with tactical consequences | I | CONFIRMED: the only in-match effect of a struck part is a jammed mount leaving the fire table (`damage.rs:782–789`); Sensor, Ecm, Eccm, Power, Relay and Core (`definitions/unit.rs:463ff`) carry none; section destruction does (`resolution.rs:153ff`) |
| R1-5 organization diplomacy | H | CONFIRMED: no org-to-org table; `mush-faction/src/data.rs:23–26` reserves "the future relationship matrix"; `field/parties.rs:13–21` makes Friends from shared ownership or membership only |
| R1-6 reinforcement and reserves | G | CONFIRMED: launches onto a promoted site refuse `FIELD_LAUNCH` (`prepare.rs:559–571`); the only `Join` senders are F-6 arrivals and director raids; the clocks are already related (`orders.rs:248–258`) |
| R1-7 recovery under fire, specialized recovery vehicles | C | CONFIRMED: the casualty rides off the kernel's ground (`tow.rs:49–51`, `admit.rs:337–340`); release in a field refuses (`tow.rs:1331–1337`); the heaviest tug reaches 16 t (`tv_escort.json:332`) against a 120 t floor for a dray wreck |
| R1-8 a replacement-parts economy | A | CONFIRMED: `tacsim.part.*` lots exist only from strip (`hardware/bay.rs:44–56`), are not in `goods.json`, have no price and no buyer; `Replace{lot: None}` fits an Intact part from nothing (`jobs.rs:467–491`) |
| R1-9 ejection, surrender, capture, rescue | F | CONFIRMED: `CrewLocation` has Unit, Carrier, Stranded, Port (`crew.rs:101`); a non-friend's load abandons the crew (`tow.rs:2314–2321`, `crew.rs:590–616`); `help/field.md:288` "There is no ejection or capture" |
| R1-10 the salvage ownership loop | D | CONFIRMED: `settle` on Vacate only retains custody and logs (`claim.rs:743–797`); `documented_repairs_at_risk` always empty (`claim.rs:622`); authorizations hard `false` (`:602–603`); `title_of` answers only through a lessee (`:239–245`) |
| R2-1 seam hardening and the second rebless | §2.2, I | CONFIRMED where it is a defect (the join race, the scratch table, `carried_rig`); the `note_fire` direction is backwards; `line_to` traces terrain only (`stock_mind.rs:634–648`) while the kernel's own traces see wrecks (`wreck_cover.rs`); `MaintainRefit` **is** honoured (`bay_work/mod.rs:370`; `bay_refit/authority.rs:174`), that sub-claim is REFUTED; the salvage-claim lock is fixed (#334 `53603c4f`), as is the Occator award (#334 `30c2808b`) |
| R2-2 licensed minds in IC fields | E | as R1-1; "blocks site promotion" is stale: the hull stands as a wall since GC.1 (`admit.rs:341/517/526`) |
| R2-3 field engineering and away-from-port bay work | B | as R1-2; the function is `facility_of`, not `facility_for` |
| R2-4 capture, prisoners, ejection | F | as R1-9 |
| R2-5 coupled towing inside fields | C | as R1-7; a Wreck or Casualty tug does set its load down at its own cell; only an Irrecoverable tug loses it (P0-8, PR #340) |
| R2-6 reinforcement, DT.4, open-ground orders | G | as R1-6; only the roster leader may issue (`roster.rs:531–534`); `npc.rs:234–241` answers NEGATIVE to SCREEN/SUPPORT on open ground, as Q-36-42 ruled |
| R2-7 salvage adjudication, forensics, crime | D | as R1-10; `record_arena_notices_to_black_box` (`tacsim/src/field_ops.rs:598`) and `mark_stripped_component_contraband_on_vacated_claim` (`mush-tactics field_ops/workshop.rs:660`) have no production caller; `SiteRecord.unlawful` has no reader but `is_empty()` and a length bound (`site_record.rs:602/1065`) |
| R2-8 stances, durable standing, the void | H | as R1-5; the fire tally and void offers are RAM (`standing.rs:30–31`, `field/mod.rs:294–295`); the "next command" close runs in the 1 Hz sweep (`standing.rs:298–320`), not at the command; an NPC leader is offered the void and has no way to issue it |
| R2-9 component effects, UD.4c, part market, heavy tug | I, A, C | component effects as R1-4; two bay paths CONFIRMED (`+bay` has no refit switch; `Act::Refit` is only a permits row, `mod.rs:85`); scrapped shells stay in the room (`strip.rs:322/395`); the part market as R1-8; the tug as R1-7 |
| R2-10 bot autonomy and the dual-scale suite | K | the 3-digit address CONFIRMED (`mine/lines.rs:343–353`, gating drive lines too, `lane.rs:320`); no bot vocabulary for `pickup`, `scrap`, `+salvage`, `+bay`, `+field/void`, `+sortie/condition`; "no story at both sizes" is REFUTED (`field_story.rs:406/414` runs one script on Ahuna and Urvara) but every GC story is single-size; `field_scale` is `#[ignore]` with 100 units measured once (p99 6.4 ms) |

Both reviewers agree that DT.4 sub-leaders, infantry, larger ground maps and mission generation stay
deferred; so do docs 32 (Q-CF-39, Q-CF-44, Q-CF-51) and 36 (§5), and this survey.

## 4. The categories

Each category says what the tree has, what the programmes left owed, what the reviewers asked for,
and the questions it raises for §5.

### A. The production chain: ore → refined goods → parts → hulls (the owner's flag)

**What exists.** The chain stops at the ore mills.

- *Ore.* Four goods, `metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore` (`content/market/goods.json`
  v4; reference 70/35/50/600 cr a tonne). A drill cycle draws the block's deposit into the rig's
  hopper and `transfer_many_marked` moves it into the Dray-P's hold (`surface/mine.rs`,
  `surface/unload.rs`); money moves only at the sale. Only Ceres buys: the Exchange bids metal,
  carbon and strategic ore, Occator bids ice (`ceres_phobos.json:2635–2646`), each bid refilling
  daily to a cap. Space mining (SF.R5) produces the same four goods (D-35-13).
- *Concentrate.* Two mills (`port/mill.rs`): the Exchange turns `metal_ore` into `metals`, Occator
  turns `ice_ore` into `volatiles`, both at 5:2, 20 t a batch, at most 40 t of ore an IC day
  (`:2639`, `:2647`). Carbon and strategic ore have no mill: doc 20 §7.3 l.1333 gives carbon "none
  in phase 1 — a stock for a later feedstock ask" and strategic ore "the Exchange's prize shelf".
  Milled ore leaves no tailings (O-3, a lead default, doc 20 l.2306). Doc 20 l.1338: "Kasei bids no
  ore: the refinery's feed is concentrate".
- *Refined goods.* No live refining. Kasei's 7,000 t of `structural_alloys` is a one-time seed
  (`:2709`); its bids for metals (4,000 t) and volatiles (9,600 t) are sinks that convert nothing.
  A recipe exists as a test fixture only: `alloy_smelt`, 4 `metals` → 1 `structural_alloys`,
  `requires: ["smelt"]`, 3 work units (`fixture/tacsim_core.json:11–23`), with two fixture
  facilities, "Phobos Yard Forge" and "Ceres Freeport Refinery" (`tacsim_corridor_v2.json:90–152`);
  the deploy template loads no packs (`mushd.toml.tmpl:255`). `industrial_parts` ("yard fittings,
  crated", 400 cr/t) is seeded only: Stickney Yards holds 5,400 t and asks 400, the Co-op at Ceres
  bids 560 (`:2660`, `:2717–2722`); bay repair consumes it at 1 t a hundred structural points
  (`workshop_tariff.json`, `workshop.rs:1613`); nothing produces it.
- *Scrap.* GC.8b mints `structural_alloys` at grade 50 from `SCRAP_KG_PER_STRUCTURAL_POINT` = 100
  (`workshop.rs:28`, `strip.rs:47–52/273`) into the carrier's hold; remains scrap on a site mints
  the same. **The mass is not bounded by the hull's:** an escort's 2,990 points are 299 t of alloys
  out of a hull that masses 8 t (`tv_escort.json:332`), repair draws 10 kg a point where scrap
  returns 100, and Freeport Works bids 520 cr/t for alloys with nothing between a title holder's
  scrap and that bid — 155,000 cr a scrapped escort, against a 90,000 cr buyout.
- *Parts.* A part is a lot of `tacsim.part.<component suffix>` (`hardware/bay.rs:44–56`; cores
  excluded; grade 9 intact, 3 worn), not in `goods.json`. `PartProvenance` (`bay_work/act.rs:85–110`,
  family `tacsim.bay_parts`) records lot, parent hull and schema, slot, definition, condition, claim,
  legality, who stripped it and when, scrapped, fitted into. Minted by `+bay/strip` and `+bay/scrap`,
  and by `bay_refit`'s Install returning the removed part with an order lineage only. Consumed by
  `+bay/fit` (a traced, clean lot of the slot's definition; a hot part refused) and `bay_refit`
  Install. `Replace{lot: None}` fits a new Intact part from nothing for
  `labor_cost_replace_component` 400 cr × the operator's multiplier and 14,400 IC s
  (`jobs.rs:325–339`, `:482`; 480 cr and 4 IC h at Ceres). No bill of materials; nobody buys a part.
- *Hulls.* Content pools laid up by the PortApplier (eight Drays at Ceres, `:2610–2612`; Occator's
  eight `tv_prospector` and two `tv_escort`; the Occator Yard house hull, `port/yard.rs`), staff
  `@unit/create` (`mush-tactics/src/module.rs:1277`), arena pods, and a recovery provider off by
  default. A player leases (`+coop/enrol`), buys out at 90,000 cr (`port/buyout.rs`), or refits a
  Dray to a Dray-P in place for no materials (`port/refit.rs`); Occator rents rigs (1,400 cr +
  5,000 deposit; 2,500 + 15,000 for an escort). Doc 18 Q-18-70: the desk answers `NO_HULL` once all
  eight are chartered. The line tank's 100,000 cr is a reference for core prices only
  (`hardware/content.rs:161–172`); nothing sells one.
- *The engine that could run the steps.* E0–E7 are built. E7 cut 4's facility, recipe and work-order
  engine exists: recipes as pack entries `kind: "recipe"` `{inputs, outputs, requires, work_units}`
  through `TacsimRecipeAuthority` (`composition/industry_recipes.rs`, wired at
  `mush-composition/src/lib.rs:400`; `PinnedRecipe` refuses a recipe with no inputs); facilities as
  `FacilityRecord` with capability and slot reservations, opened by `@facility/open` (the port's
  workshop blocks are facilities with the `bay_refit` capability); work orders `@wo/draft|submit|deliver`
  at one work unit an IC hour (`PRODUCTION_CADENCE_SECS` 3600) with E4 holds for the money. Its one
  live user is the R9a lunar relay foundry (`relay_foundry.rs`): 1 `relay_blank` + 2
  `instrument_boards` → `relay_pair`, 120 work units (`content/relays/manufacture-v1.json`). A work
  order can deliver lots only: `DeliveryTarget::Endpoint` is its one target
  (`mush-industry/src/workorder.rs:448–454`). The port tick's arms are seed minting with receipts,
  the intake drip, the mill and `restock_ask` (`port/stock.rs`, `port/mill.rs`). E8, the service
  market, is unplanned (doc 08 l.1246, l.1941).

**What is owed.** Doc 18 §5 l.2652–2655 P5: "production restock with conservation (Kasei refinery
work orders, Ceres deposits, a construction sink) before a second cohort" — only the mill half
landed; §9 risk 9 (l.3610) a recapitalization path; Q-18-60 (l.3491) Kasei drained after about
fifty round trips. Doc 20 §7.3 the carbon and strategic consumers; 20b OD-2/OD-4 unruled. Doc 22 §7
l.431: "A builder, its economy and who may field what are their own design". Doc 31 §9 l.447 what a
design costs to register and to build; Q-31-2 ruled yes (an IC hull on a registry design) and only
staff can do it; D-31-13's table (l.362) has a replacement part "from stores or bought". Doc 36
Q-36-31 (parts drawn from lots), Q-36-32 ("A labour market is a later economy cut"), Q-36-39,
GC.8a l.1868 ("a part price is a GC.8b or economy question"); doc 36a §6 P0-1, §9.1, §9.5; doc 37
§5.1 l.163 (the census), §5.2 l.199, §7.9. Doc 10 §4A l.255–262 asks for "chains ≈ three
transformations deep" with "drydock and factory time" as reserved capacities; doc 10 §5 l.94–97
sets the loop "extract → haul → refine → manufacture → deploy".

**What the reviewers ask.** R1-8 a parts economy with condition and provenance, "a part market or
pricing" (the provenance exists; the market and price do not). R2-9 a port market or tariff for
`tacsim.part.*` lots and technician labour, UD.4c's unification of the two bay paths, disposal of
scrapped shells. R2-1 a part lot or price on `Replace`. Neither reviewer surveys the supply side:
both treat parts as a market with nothing feeding it, which is the owner's point.

**Questions.** Q-38-1, Q-38-4, Q-38-5, Q-38-12 to Q-38-15.

### B. Restoration away from port: the carrier's bay and the tender in the field

**What exists.** `facility_of` (`bay_work/mod.rs:203–237`) requires `Berth::Docked` at a port with a
workshop operator outside a workshop room ("A carrier's bay works on its hulls only while she is
docked at a port with a workshop to supply it"); `carrier_of_bay` (`:174–186`) searches port maps'
hulls only. GC.8a built armor, mend, unjam and rearm in a carrier bay at `CARRIER_TIME_FACTOR` 2
(`jobs.rs:44/199`), rebuild and replace in a workshop only, and called the docked-only rule "a cheap
one to widen later" (doc 36 l.1873–1884). The mush-tactics tender functions exist with test callers
only: `field_unjam_component` (`engineering.rs:237`), `field_stabilize_damaged_section` (`:400`),
`field_resupply_ammo_and_charge` (`:558`), `register_tender`; tacsim re-exports only
`FieldEngineeringBook` (`tacsim/src/field_ops.rs:22`). No `+tv` or field verb unjams, stabilizes or
rearms on the ground.

**What is owed.** Q-36-35 (owner, 2026-11-02): "a carrier's bay counts for minor repairs at double
the workshop's time… The field-engineering boundary of doc 27a §10 stays as it is for units out on
the ground." Q-36-36: "a carrier can carry munitions as cargo for field rearm". Doc 36 l.3243 and
doc 37 §5.2: "Q-36-35/36 are narrower than ruled". Doc 37 §5.4: 27a §10 field engineering never
built; §7.5 the fifth area.

**What the reviewers ask.** R1-2 support vehicles that unjam, stabilize casualties and rearm from
carried stocks; bays beyond port workshops. R2-3 minor repairs in a carrier's bay berthed on a site
or in flight, drawing `industrial_parts` and munitions from the hold at 2×; the tender on a site.
The two agree, and both match the rulings.

**Dependency.** Rearm draws munition lots and parts draw part lots: the shape of those lots is A's.

**Questions.** Q-38-16.

### C. Recovery under fire, and heavy recovery

**What exists.** The towed casualty rides as its tug's load, off the kernel's ground, "neither seen
nor struck while carried" (`tow.rs:49–51`; `admit.rs:337–340` filters carried units out of
admission). Releasing inside a promoted field returns `FIELD_OPEN_CODE` (`tow.rs:1331–1337`, lane 1's
P1-10). A Wreck or Casualty tug sets its load down at its own cell at demotion; an Irrecoverable tug
is unplaced first (`close.rs:296–302`) and loses it (P0-8, fixed in PR #340 by falling back to the
casualty's own pose). The tow cap is twice the tug's mass (`recovery.rs:163–176`), every attach is
`TowRigProfile::Standard` (`tow.rs:686`), and a carrier's floor is `TUG_SHARE_PERMILLE` 500
(`carrier_damage.rs:116/243`, `field::carrier::refuse_tow`). Only `+tv/<n> tow` rigs tow
(`tv.rs:2393`): the prospector at 6 t and the escort at 8 t reach 12 and 16 t; a dray is 240 t dry.

**What is owed.** Q-36-20 (default accepted): "The tug may drive coupled at the tow rate, or
release. A hit on either does not break the coupling" — the build deviates (GC.4). Q-36-8 (owner):
a carrier wreck needs a tug of half its tonnage; "No such unit exists yet". Doc 36: the hopper's ore
in a casualty's laden mass; every tug Standard; a unit must already be on a site's index to be
towed. Doc 37 §7.9 a carrier-wreck tug.

**What the reviewers ask.** R1-7 damageable towed casualties, release during combat, cargo mass,
"recovery equipment capable of heavier wrecks". R2-5 the casualty inside the kernel or as a coupled
trailing entity, `DROP TOW` inside a field, the Irrecoverable tug's drop. R2-9 "a heavy recovery
vehicle (≥ 0.5× carrier dry mass)".

**An observation for the owner.** A 120 t tug cannot ride a Dray-P's 20 t bay, so the heavy
recovery vehicle both reviewers ask for cannot reach a wreck by the carrier that found it. Doc 36's
Q-36-46 already rules one salvage path for ground and space with `+helm/tow` onto the same claim.
A carrier wreck on the ground is then a ship's recovery from orbit, and ground tugs stay at the
escort's scale.

**Questions.** Q-38-17, Q-38-18. The kernel half is part of I's one rebless.

### D. The salvage loop's close: adjudication, forensics, crime

**What exists.** A vacated claim retains custody and logs `UnlawfulKind::Vacated`
(`claim.rs:743–797`); the hull does not move. Lane 2's `berth_stands` (`mod.rs:307–322`) stops the
salvor's carrier serving her ("a carrier of your own must take her") but `berth` needs the carrier
right or custody aboard (`:1466–1475`), `stow` returns her only to `row.bay_room` (`:1505–1538`), and
no verb moves her to the owner's carrier. `documented_repairs_at_risk` is `Vec::new()` at filing
(`claim.rs:622`): `record_work_order` writes an at-risk repair only when given a book
(`workshop.rs:1677–1683`) and every `jobs::work` call passes `None`. The recovery authorizations are
hard `false` (`:602–603`). `title_of` answers only through a live lessee (`:239–245`), so an
org-titled hull with no lessee cannot be claimed (an award on one is never made, #334). The black
box is written by `surface::black_box::append` (`field/provider.rs:317`, `prepare.rs:100`);
`record_arena_notices_to_black_box` (`tacsim/src/field_ops.rs:598`) has a test caller only;
`mark_stripped_component_contraband_on_vacated_claim` (`workshop.rs:660`) has none, and tacsim's own
`strip::vacate_claim` (`strip.rs:411`) is called from `bay_gc8.rs:700` alone. `SiteRecord.unlawful`
(`site_record.rs:490`, "Evidence for a later crime system, never an enforcement") is written by
`log_unlawful` (`claim.rs:420–424`) and read by nothing that acts.

**What is owed.** Doc 36: returning a vacated hull (GC.6); hot parts on a vacated claim and a
salvor's strip (GC.6, GC.8b); repairs at the custodian's risk (GC.6); recovery authority grants
(GC.6); story (b) contests on the sensor record, not the black box; no verb for a
defective-procedure or superior-right contest; `claim_window_s` not settable. Q-36-30 (default):
"unlawful recovery is recorded as evidence only… No enforcement verb in this phase." Doc 37 §7.9 "a
crime system over the evidence GC.6 records".

**What the reviewers ask.** R1-10 recovery-authority grants, org-owned hull claims, usable evidence,
documented repairs at risk, physical return after a vacate. R2-7 the same, plus the two forensics
functions wired, and a port-security consumer of `unlawful` (fines, quarantine, bounties).

**Dependency.** Hot parts and repairs at risk carry prices and lots from A.

**Questions.** Q-38-19, Q-38-20.

### E. Licensed minds in fields: S.4b and custody for moving anchors

**What exists.** `HostTransition { _private: () }` is built only inside `#[cfg(test)]` fixtures
(`participant.rs:510/545/562/584`); the module says "There is presently no producer for a genuine IC
region transition, so this module exports no constructor" (`:20–22`). Admission partitions on
`held_out` (`admit.rs:341/517`) and `with_held_walls` (`:526`) stands such a hull as ground nobody
crosses ("its licensed mind cannot be taken into a field yet"); the walls are rebuilt only at
admission (36a §11 row 1, fixed in PR #340). `earned_contacts.rs:4`: "No producer yet"; `admit()`
has a unit-test caller.

**What is owed.** Q-36-44 (default accepted): leave it out as an inert obstacle. Doc 23 l.376: "S.4b
is the claim Open/Close (the production `HostTransition`), the pool seat, the `abi` drop on resume
and the long-leg snapshots… together with a custody design for moving anchors." Doc 37 §5.5: the
blocker named by docs 32, 36 and GC.1; "its precondition 'custody for moving anchors' has no owning
doc"; §7.2.

**What the reviewers ask.** R1-1 production claim open/close, compute allocation, restart recovery,
"resolve custody for moving anchors first". R2-2 the same, the resume fallback, the testimony
producer. Both rank it first or second. It consumes nothing from A.

**Questions.** Q-38-21.

### F. Crew: capture, prisoners, ejection, treatment

**What exists.** `CrewLocation` is Unit, Carrier, Stranded or Port (`crew.rs:101`); a non-friend's
completed load runs `abandoned_loaded` (`tow.rs:2314–2321`) and `abandon` sets
`Stranded { site, unit }` with `Extraction { abandoned: true }` while the hull sits in the captor's
bay (`crew.rs:590–616`). No brig, ransom or handover code; the field's verbs are `/fire`, `/mark`,
`/void` (`field/commands.rs`); `help/field.md:288` says there is no ejection or capture. Treatment
(GC.13) recovers a character at a port on the IC clock; beds are authored but not a limit.

**What is owed.** Q-CF-38: "No treatment, no ejection, no capture in the first version." Q-36-45
(default): recovery at a port; "ejection first: a forced extraction with a chance of capture" was the
alternative. Q-36-52 (owner): abandonment "until capture is built (Q-CF-38 rules it out of this
phase)". Doc 37 §7.3 the third area; both reviewers say it needs new rulings.

**What the reviewers ask.** R1-9 separate crew survival and custody from hull ownership; evacuation;
prisoner transfer; "requires new design rulings for incapacitation, custody, and player recovery".
R2-4 `CrewLocation::Captured { carrier, captor }` with ransom, release and port-handover verbs;
`EJECT` for a seated pilot in a doomed unit.

**Questions.** Q-38-22.

### G. Command and reinforcement

**What exists.** A launch onto a promoted site refuses `FIELD_LAUNCH` (`prepare.rs:559–571`,
`site_record.rs:1108`). The only `ArenaMsg::Join` senders are F-6 arrivals (`carrier.rs:505`) and
director raids (`raiders.rs:987/1079`); neither launches from a pad. The field runs on its own wall
clock and the world keeps its rate (Q-CF-32: "a ten-minute fight is four campaign hours at rate
24"); the one conversion is `orders.rs:248–258` (`arena_deadline`); the field's tick holds while the
W-clock is frozen (`admit.rs:3`). Only the roster leader orders (`roster.rs:531–534`, `NotLeader`;
`detachment/commands.rs:53`); no sub-leader concept exists. `npc.rs:234–241` sends SCREEN and
SUPPORT to NEGATIVE on open ground.

**What is owed.** Q-36-5 (default accepted): keep Q-CF-17's refusal, bounded by GC.2's idle demote;
"Real reinforcement is better answered with F-6 arrival plus a launch rule designed for it."
Q-36-43: DT.4 stays deferred. Q-36-42: NEGATIVE on open ground stays. Doc 36: a shard_1 story for
SCREEN and SUPPORT as delivered orders. Doc 37 §7.9.

**What the reviewers ask.** R1-6 launches into an active field through capacity reservations and
deployment positions; "needs a new launch rule, including the relationship between combat time and
the accelerated world clock" (the relationship exists; the rule does not). R2-6 launches at the ramp
via `Join` with a launch-cycle cooldown under the cap; DT.4 sub-leaders issuing ORDER; SCREEN and
SUPPORT executed on the surface. On DT.4 and open-ground orders the reviewers contradict the
rulings; on reinforcement they ask for Q-36-5's rejected alternative.

**Questions.** Q-38-25, Q-38-26.

### H. Relations: stances, standing, the void

**What exists.** Friends come from a shared owner or a shared openly-visible organization, faction
or principal (`parties.rs:76–96`, `:424–446`); Hostile comes only from a leader's mark or fire
(`arena/party.rs:172–174`); `mush-faction/src/data.rs:23–26` reserves "the future relationship
matrix". GC.10's standing at demotion is durable through mush-faction receipts, but `FireTally`
(`standing.rs:68`) and `VoidOffers` (`:84`) are `reg.state` RAM; `void()` checks only the IC deadline
(`:322–333`) and the "next command" close runs in the 1 Hz sweep from `session_stat(leader).last_input`
(`:287–320`), which its own doc says two inputs in one second can slip. `live()` accepts an NPC
leader (`:141–145`), who is offered the void and has no way to issue it.

**What is owed.** Q-36-10 (default accepted): "shared membership only… There are no declared stances
yet"; the alternative was a stance table. Doc 36: the fire tally and void offers are RAM; an NPC
leader cannot void. Doc 37 §5.4 F-1's other half; §7.4 the fourth area, "small, and GC.10 left the
seams".

**What the reviewers ask.** R1-5 declared alliances, neutrality and hostility; conflicting
affiliations; when a stance change takes effect during combat. R2-8 an Allied/Hostile/Neutral matrix
in mush-faction or mush-org fed into `parties.rs`; durable tallies and offers; the void enforced
synchronously; NPC leaders auto-voiding accidental friendly fire. It consumes nothing from A.

**Questions.** Q-38-23, Q-38-24.

### I. Kernel depth: one rebless

Every change here moves the kernel's hashes, so they are planned as one batch (Q-36-56's rule:
one rebless a programme).

**What exists.** A struck part is set jammed or damaged only when `struck.penetrated && !struck.fresh`
(`damage.rs:720`, `part_struck` `:1025–1065`); the one in-match effect is a jammed mount leaving the
fire table (`:782–789`); `part_works` (`state.rs:1802–1810`) filters mounts at check-out and nothing
else reads a part's condition. Component kinds are the mount, Sensor, Ecm, Eccm, Power, Relay and
Core (`definitions/unit.rs:463ff`); a jammed sensor works in the next match; sensors go only on a wreck
(`damage.rs:1427`); heat (`heat.rs`) has no damage input. Section destruction does carry authored
consequences (`resolution.rs:153ff`: mounts removed, slowed, no turn-toward, motion disabled, the
core degraded or destroyed, the unit destroyed). `line_to` traces terrain only (`stock_mind.rs:634–648`)
while the kernel's fire and sensor traces see wrecks (`wreck_cover.rs`, `damage.rs:645`). The ledger
is cumulative: `max_bytes` 64 MiB (`config.rs:1626`) against `bytes_total` (`ledger.rs:844/1033`),
`EndReason::Timeout` at the bound (`step.rs:758–762`); chunks are 16 KiB and stream out, with
`max_unflushed_chunks` 256 voiding the match past it; snapshots every 5 s carry `LedgerState`
including `bytes_total` (`ledger.rs:873–882`, `snapshot.rs:1263`) and `boot.rs:228` resumes with
`segment += 1` and no reset. Doc 36 l.1805: 7.4 MB in two minutes of non-stop fighting, so about
eighteen engaged minutes at 100 units.

**What is owed.** Doc 36 "Kernel, with the next rebless": `note_fire` (§2.2 has the corrected
direction), the towed casualty as a kernel unit (Q-36-20); "the unit-mind API": `line_to`'s wrecks
(GC.2); "the field host": the ledger at 100 units, the join race (§2.2). Q-36-38 (default, expensive
to reverse): "a hit that does not destroy a section may jam or damage one component in it" — built;
its consequences — not. Doc 36 GC.2 l.1423: "A jammed or damaged part of any other kind changes
nothing in the match yet." Doc 37 §7.8 the second rebless.

**What the reviewers ask.** R1-3 bounded journal segments, recoverable checkpoints, durable arrivals
and departures. R1-4 damaged engines, sensors, controls affecting performance; "meaningful choices
between disabling, destroying, repairing, and withdrawing". R2-1 the rebless pair. R2-9 internal
hits damaging sensors, ECM/ECCM, cooling and actuators with immediate penalties (there are no
actuator parts; heat has no damage input).

**Dependency.** The part outcomes (what a damaged Power part costs to repair, what a stripped one is
worth) are A's; the kernel's side is not.

**Questions.** Q-38-27, Q-38-28.

### J. The space track

**What exists.** SF.R9–SF.R13 merged on 2026-11-03 (PRs #332, #333, #335, #336, #337: the authority
close-out, the tempo and persistence budget, one ledger and a rated drive, domain bubbles and real
hulls, presence and feeds); SF.R14 (#338) and SF.R15 (#339) open; SF.R16 (the client docking game)
owed. `space::tick::register` and `+helm`/`+tac`/`+mine` still register under `[tacsim].enabled`
alone (`lib.rs:1022`); `mush-config` knows `space_pulse_ms` and no space flag; docs 35 and 35b carry
no flag in their text. 35-cut-notes stops at SF.R8.

**What is owed.** Doc 37 §5.3 and §7.1; doc 36's "space's own cut" (Q-36-46 `+helm/tow` onto the
one claim, Q-36-47 `ShipSectionId` onto `CarrierPart`, Q-36-48 `SpacePartyId` to `PartyId`).
Neither reviewer surveyed space. Q-38-9 (ruled): the track's exposure on a redeploy is not a
concern — the boxes are wiped and redeployed at every step — and it gates nothing in this survey.

**Questions.** Q-38-9 (ruled); the three seams belong to the document that takes C and D, and
Q-38-6's retrieval contracts need the one claim path because a space tow truck is the same
occupation as a ground one.

### K. Acceptance: the bot over SSH, both sizes, the sweep

**What exists.** `is_addr` accepts one to three digits (`mine/lines.rs:343–353`) against addresses
to `P1024` (`surface/address.rs:21/903`), and gates drive lines too (`lane.rs:320`), so the bot
cannot drive to a four-digit pocket; it has tow, load and release (`escort.rs:821–825`) and nothing
for `pickup`, `scrap`, `+salvage/*`, `+bay/*`, `+field/void`, `+sortie/condition`, `+sortie/extract`.
`staff_hand` after lane 4: field_story 5, field_gc3 2, field_gc4 1, field_gc5 1, field_gc6 1,
field_director 4, bay_gc8 3, beside the seams doc 36a §8 allows (`set_section`, `seed_pool`,
`seat_npc`, `@econ/mint`, `seed_hold`, `@tacsim/site`). field_story runs one script on Ahuna and
Urvara (`:406/414`); every GC story runs at one size (256: gc2–gc5, gc10, director, host, carrier,
crash, crew_exit, crew_treatment, escort_lease, field_scale; 512: gc6, bay_36a, most of review_36a;
bay_gc8 at Stickney). `field_scale` is `#[ignore]` with `GC12_SIZES` 9–100 and one hand run (Engaged
p99 6.375 ms at 100 against a 40 ms budget); `FIELD_MAX_UNITS` is 100 (`site_record.rs:60`).

**What is owed.** Doc 36 §7's "stories grow" rule; Q-36-3's bot over SSH for (a), (b), (c); the
director-raid captain run (`FIELD_RAID=director`, no recorded run); the gate.yml sweep. Doc 37
§7.6, §7.7.

**What the reviewers ask.** R2-10: the four-digit parser, the vocabulary, the no-fiat suite at 256
and 512 (Ahuna, Urvara), the sweep above 100.

**Questions.** Q-38-29.

### L. Operations

The first routine redeploy of the phase (nothing of it is on dev1/dev2), T-112 backups, the ufw/ACL
closure of 6790/6793, the nightly's owner, a weekly slot for the dispatch-only stories (doc 37
§7.10). Nothing here needs a design document, and after Q-38-9 and Q-38-10 nothing gates the
redeploy: it ships `main` as it stands, and a wipe is the migration.

### Out of scope by ruling

The 10 km ground (doc 16 D-16-5, doc 32 Q-CF-51), infantry and battle armor (doc 27 FO-6, Q-CF-39),
a mission generator beyond the raid director (Q-CF-44, Q-36-41), persistent known-hostile memory
(doc 32 F-7, the Epistemic Engine), facility assaults, DT.4 (Q-36-43). Both reviewers leave the same
list deferred.

## 4a. Consequences of the rulings

What the six amendments of 2026-11-03 change beyond their own questions.

- **Retrieval contracts (Q-38-6).** A new contract kind: a title holder (Occator at a repossession,
  or anyone) posts a retrieval for a rig — the tow fee, the destination, a 28-IC-day acceptance
  window — and a player accepts it as a job; one not accepted within the window goes to an NPC
  towing company. The repossession charge adds that fee to the damage charge. "Space tow truck" is
  an occupation, so the contract runs on the one claim path of Q-36-46 (ground and space) and
  belongs to doc 41 with C and D.
- **The NPC salvage-retrieval organization (Q-38-18).** An organization at one of the main ports,
  outside the player's immediate economic zone, owning a heavy crawler and whatever carries it; the
  fallback acceptor of retrieval contracts and the only recovery of a carrier wreck on the ground.
  The player, or Occator, contracts it to retrieve anything. Its fee comes from doc 39's facility
  cost model, and it answers doc 37 §5.4's "Occator recovering a written-off wreck".
- **The facility cost model (Q-38-12).** Every facility has a capital cost, an expected lifetime and
  a staffing, from which a daily cost follows; a work's manufacturing cost is the daily cost over
  the days the facility spends on it; a registry design's bill is its slots plus that cost; the
  workshop tariff's labour rows become instances of the same model. Doc 39 owns it, and Q-38-15
  stands: the facility fee is what covers labour.
- **The scrap ceiling (Q-38-5).** The yield's mass is bounded by the hull's dry mass, and its value
  by about a fifth of the hull's buyout: the scrapping facility's fee consumes the rest, as a navy's
  scrappers do. Doc 39's mass budget carries both rules, and Q-38-14's census covers the mint.
- **The space track (Q-38-9).** Nothing in §4 J or §7 gates a redeploy on a flag; the first routine
  redeploy ships `main`. The missing SF.R9–R13 sections in 35-cut-notes stay a docs-owed item.
- **The marker (Q-38-10).** "(expensive to reverse)" now means a refactor to undo, so it stays on
  Q-38-12 (the hull's provenance record every later claim reads), Q-38-17 and Q-38-27 (the kernel's
  hashes) and comes off Q-38-5, Q-38-9, Q-38-10 and Q-38-22.

## 5. Owner survey

**Ruled 2026-11-03.** The owner amended Q-38-5 (scrap is worth at most a fifth of the buyout),
Q-38-6 (repossession also charges the retrieval, as a contract players may take), Q-38-9 (no space
flag, no concern), Q-38-10 (the rooms stay, decided now), Q-38-12 (a design's bill carries the
facility's amortized time) and Q-38-18 (an NPC salvage organization with a heavy crawler). Every
other default is accepted as written. Each amended question carries its decision first, with the
proposed text kept under it for the record.

### 5.1 Order and shape

**Q-38-1 — The lead programme.**
- **Proposed default:** the production chain (A) is surveyed and planned first, as its own document,
  with E, H and the space flag beside it (Q-38-3). B, D and I's part outcomes each consume what A
  defines: a part lot with a price, a munition lot, a scrap mass bounded by the hull's.
- **Alternative:** doc 37 §7's order: space hardening, S.4b, capture, stances, field engineering.
- **Why:** GC.8a's free part, the 100 kg scrap point and the seeded `industrial_parts` are three
  placeholders in one chain, and every restoration cut after this one would add a fourth. Both
  reviewers rank parts as a market and neither feeds it.

**Q-38-2 — One document or several.**
- **Proposed default:** §6's split: three design documents, one note, one re-plan already written.
- **Alternative:** one doc 38 plan with lanes, as doc 36 §7 did for GC.
- **Why:** the categories cut across three engines (economy and industry, the field kernel, crew
  and custody) with different rebless and persistence costs; a single cut list would serialize what
  can run beside each other.

**Q-38-3 — What runs beside the production chain.**
- **Proposed default:** E (after its custody note), H's stance table, and the space enable flag.
  None consumes a part or a price, and each unblocks something named: Q-36-44's wall, F-1's other
  half, the redeploy.
- **Alternative:** nothing in parallel.
- **Why:** the owner's rule since the wave programme is that independent work runs in parallel
  sessions.

### 5.2 Decisions carried from docs 36a and 37

**Q-38-4 — The part price (doc 36a §9.1).**
- **Proposed default:** price the part. `Replace{lot: None}` draws a `tacsim.part.*` lot of the
  slot's definition from the facility's own stores at a catalogue price; the stores are fed by strip
  (today) and by A's part recipe (later); until the recipe exists, a seeded part stock per workshop
  at a provisional reference price (the component's share of the hull's reference price), and an
  empty store refuses the order with the reason, as `NO_HULL` does. The labour fee stays.
- **Alternative:** keep the workshop an uncapped vendor at 480 cr.
- **Why:** a free part makes strip, salvage and any part market pointless; the 2026-10-24 rule says
  take a number and tune it.

**Q-38-5 — The scrap yield (doc 36a §9.5).**
- **Decision (owner ruling, 2026-11-03):** the default, with a value ceiling on top of the mass
  bound. An 8 t hull cannot produce more than 8 t of scrap; and scrapping is work that consumes most
  of the material's value — a navy pays its scrappers, it does not profit from them — so the scrap
  of a whole hull is worth on the order of **a fifth of its buyout at most**: a Dray's 90,000 cr
  buyout scraps to about 18,000 cr. Whether that is a yield tuned so the alloys' bid value lands at
  the ceiling, or a mass-bounded mint with a scrapping fee the facility takes, is doc 39's; the
  ceiling is the rule.
- **Proposed default (as written):** bound scrap by the hull's mass. A hull's scrap is its dry mass × surviving
  structural points ÷ total points × a recovery fraction (provisional 0.6), minted as
  `structural_alloys`; a workshop's scrap goes to the port's stock (the operator's), a site's
  remains scrap to the carrier's hold, where the room check applies. The same mass budget is one
  rule for mining, milling, repair and scrap in A.
- **Alternative:** keep 100 kg a point and divide by 15.
- **Why:** 299 t out of an 8 t escort is matter creation, worth 155,000 cr at Freeport Works' bid
  against a 90,000 cr buyout, and no rule stands between a title holder's `+bay/scrap` and that bid.
  Expensive to reverse because the yield is a mush-tactics constant tests pin and the first live
  box would carry its lots.

**Q-38-6 — Repossession damage charge (doc 36a §9.2, doc 37 §6.3).**
- **Decision (owner ruling, 2026-11-03):** the default, plus a towage fee. Beyond the damage past
  the wear allowance, repossession charges the lessee for the retrieval of the lessor's property:
  whatever the lessor would pay a PC or NPC salvage team to bring the rig in. The retrieval is a
  contract players may accept; one not accepted within 28 IC days goes to an NPC towing company.
  "Space tow truck" becomes a viable occupation (§4a).
- **Proposed default (as written):** at repossession and at return, the lessor charges the tariff's quote for
  repairs beyond a wear allowance (provisional: a tenth of the hull's points), the deposit first and
  the remainder as a debt on the lessee's ledger; the dormancy path pays the same.
- **Alternative:** no charge, as `repossess` (`tv.rs:4996`) takes today.
- **Why:** without it a lessee lets a charter go dormant on a damaged rig and takes the deposit whole.

**Q-38-7 — The 28-day bar on a wreck brought home (doc 36a §9.3).**
- **Proposed default:** the bar follows the rig. A lease rig that came home a wreck and is not
  returned to Occator within the 28 days is written off the same way: title to Occator, the deposit
  forfeit.
- **Alternative:** the bar is a field-only clock.
- **Why:** the bar is Occator's recourse, and that does not change with where the wreck lies.

**Q-38-8 — Delegated crew (doc 36a §9.4).**
- **Proposed default:** stays fail-closed. Boarding is a custody question for the ownership and
  permissions design (doc 27 Q-A1's delegable capabilities), not a `MaintainRefit` one.
- **Alternative:** a `MaintainRefit` grantee may board.
- **Why:** nothing in play needs it yet, and a wrong grant kind is a migration later.

**Q-38-9 — Space exposure on the next redeploy (doc 37 §6.1).**
- **Decision (owner ruling, 2026-11-03):** no flag, and the space track is not a concern of this
  survey. There are no continuous games; the boxes are wiped and redeployed at every step, so what a
  redeploy exposes costs nothing. The track's exposure stops being raised as a precondition.
- **Proposed default (as written):** `[tacsim.space].enabled`, default `false`, gating `space::tick::register`
  and the `+helm`/`+tac`/`+mine` registrations (`lib.rs:1022`), as one cut before SF.R16 and before
  the first routine redeploy.
- **Alternative:** ship the track live with SF.R9–R15.
- **Why:** doc 37 §5.3's holes 7, 10 and 11 key persisted records on fake ids; once a live box has
  written them the migration is the owner's.

**Q-38-10 — Workshop rooms (doc 37 §6.2).**
- **Decision (owner ruling, 2026-11-03):** leave them where they are, decided now, not deferred to a
  redeploy. Re-homing hulls berthed on a box that is wiped anyway costs nothing, so the question was
  never expensive: "expensive to reverse" means a significant refactor of the codebase to undo, and
  the header now says so.
- **Proposed default (as written):** leave the workshops in the existing rooms (GC.7/GC.8a).
  Nothing is berthed on a live box yet; the first routine redeploy is the last cheap moment, so the
  decision is taken then, not now.
- **Alternative:** move them to dedicated rooms before that redeploy.
- **Why:** the cost is re-homing berthed hulls, which is zero until the redeploy.

**Q-38-11 — The rest of doc 37 §6.** Unchanged and not re-asked: the W-clock restart mid-countdown
(§6.4), M8b-2's heirloom and M8h's term (§6.5), the M8 gate sign-off (§6.6), `launch_path` (§6.7),
20b OD-2/OD-4 (§6.8), the first routine redeploy and the 512 hand smoke (§6.9).

### 5.3 Questions the categories raise

**Q-38-12 — Unit construction (A).** **(expensive to reverse)**
- **Decision (owner ruling, 2026-11-03):** the default, with the bill widened. A registry design's
  bill is the sum of its slots **plus the manufacturing cost**: an amortization of the facility's
  time on the work over the facility's cost and its expected lifetime, before labour and profit. A
  shipyard costs tens to hundreds of millions of credits and lives about thirty years; at ten
  million that is about 1,000 cr a day, so a hull that takes two weeks carries about 14,000 cr of
  yard time before labour and margin. Doc 39 derives each facility's **daily cost** from its capital,
  its lifetime and its staffing, and that daily cost drives hull prices. Q-38-15 stands: the
  facility fee is what covers labour.
- **Proposed default (as written):** a hull is built by a work order (doc 08 §10, E7's engine) at a
  facility with a `yard` capability. Its inputs are the design's bill of materials:
  `structural_alloys`, `industrial_parts` and one `tacsim.part.*` lot a slot; its output is a hull
  delivered into the requester's title and the yard's berth, which needs a second `DeliveryTarget`
  beside `Endpoint` (`workorder.rs:448–454`). The catalogue's six U.6 assemblies each get a bill of
  materials in the content; a registry design's bill is the sum of its slots.
- **Alternative:** no construction; the content pools restock from the chain on a tick, so the
  desk's `NO_HULL` is answered by production rather than by a player's order.
- **Why:** Q-31-2 is ruled yes and only staff `@unit/create` does it; doc 10 §5's loop ends at
  "manufacture → deploy". Expensive because a hull's provenance (built, from which lots) is a stored
  record every later claim reads.

**Q-38-13 — The chain's depth (A).**
- **Proposed default:** three transformations (doc 10 §4A): ore → concentrate (the mills, built) →
  `structural_alloys` and `industrial_parts` (a refinery at Kasei and a works at Stickney and Ceres,
  by recipe, through `@wo` or a mill row) → parts and hulls (the yard). Carbon ore feeds the works'
  `industrial_parts`; strategic ore feeds a part recipe for sensors, ECM and cores.
- **Alternative:** two steps: a Kasei mill row metals → alloys (authorable as lane data today,
  `port/def.rs:1847–1900`, if `MillDef`'s same-family rule is relaxed), and parts straight from alloys.
- **Why:** the fixture recipe `alloy_smelt` and its two facilities already exist for tests; the live
  precedent (the relay foundry) runs one recipe through a real work order; carbon and strategic ore
  have had no consumer since doc 20.

**Q-38-14 — Conservation (A).**
- **Proposed default:** every mint in the chain is a recipe's output or a seed with a receipt; the
  matter census pin covers `.mint_origins(`; Stickney's and Kasei's seeds become stock the chain
  refills, with the intake drips retired as the recipes land.
- **Alternative:** seeds stay as the floor.
- **Why:** doc 18 P5 "production restock with conservation" is owed before a second cohort.

**Q-38-15 — Labour (A, Q-36-32).**
- **Proposed default:** still later. The facility fee covers labour; a technician duty arrives with
  the service market (E8), which this survey does not plan.
- **Alternative:** a technician crew duty now, as the scarce input the reviewers ask for.
- **Why:** nothing in the chain needs a second scarce input before the first is real.

**Q-38-16 — Field engineering's shape (B).**
- **Proposed default:** both halves of the rulings. A tender role (a support rig carrying munition
  and part lots) on the surface tick with `+tv/<n> unjam|stabilize|rearm` on an adjacent casualty,
  through the three mush-tactics functions; and the carrier's bay working berthed on a site at the
  2× factor for minor repairs, drawing lots from her hold. Away from a port with a workshop, replace
  and rebuild stay refused.
- **Alternative:** the bay only.
- **Why:** Q-36-35 and Q-36-36 rule both; the engine's half exists with no caller.

**Q-38-17 — The tow in the kernel (C, I).** **(expensive to reverse)**
- **Proposed default:** the towed casualty is admitted as a kernel unit coupled to its tug: it can be
  struck, a hit on either does not break the coupling (Q-36-20), the tug may release inside the
  field, and an Irrecoverable tug drops it at the tug's last cell. One rebless with Q-38-27.
- **Alternative:** stays off the ground, as GC.4 built.
- **Why:** Q-36-20 is already ruled this way; GC.4 deviated to stay inside one rebless.

**Q-38-18 — Heavy recovery (C, J).**
- **Decision (owner ruling, 2026-11-03):** the alternative, placed outside the player's reach. A
  larger crawler exists, outside the player's immediate economic zone, owned by an NPC
  salvage-retrieval organization at one of the main ports; the player, or Occator, contracts it to
  retrieve anything, through Q-38-6's retrieval contracts (§4a). Ground tugs at the escort's scale
  stay as proposed; the ship's tow from orbit (Q-36-46) stays a seam, not the answer to the carrier
  wreck.
- **Proposed default (as written):** a carrier wreck on the ground is recovered from orbit by a
  ship's tow onto the same claim (Q-36-46's `+helm/tow`), not by a ground tug. Ground tugs stay at
  the escort's scale, with a recovery profile authored above `Standard` (a provisional 3× cap) for
  a tracked recovery rig in the kit.
- **Alternative:** author a 120 t crawler.
- **Why:** a tug of half a dray's mass cannot ride a 20 t bay, so it could never reach the wreck by
  the carrier that found it; the space seam is ruled and unbuilt.

**Q-38-19 — The salvage loop's close (D).**
- **Proposed default:** four things. A vacated hull is returned: the owner's own carrier may berth
  her out of any bay, and a port yard holds her for the owner where no carrier of theirs is present.
  `documented_repairs_at_risk` is written by every bay job run under a Salvor right (the book
  `record_work_order` already takes). Recovery authority is a grant kind the title holder or the
  landholder issues (`+salvage/authorize`), read at filing. An org-titled hull with no lessee is
  claimed through the organization's owner read. Contest kinds (defective procedure, superior right)
  are `+salvage/contest` switches that name the evidence source.
- **Alternative:** the vacated hull's return only.
- **Why:** story (b)'s adjudication cannot resolve against an organization or a prior repair today,
  and GC.6's evidence rows exist with no reader.

**Q-38-20 — Crime (D, Q-36-30).**
- **Proposed default:** a minimal consumer. Port security reads `SiteRecord.unlawful` at docking: a
  fine on the actor's ledger scaled by the kind, and a quarantine of the hull until paid. Bounties
  and standing hits later.
- **Alternative:** a standing hit only.
- **Why:** Q-36-30 limited evidence-only to "this phase"; the two forensics functions in mush-tactics
  were written for this consumer.

**Q-38-21 — S.4b's precondition (E).**
- **Proposed default:** write the "custody for moving anchors" note as a section of doc 23 before
  S.4b's cut, from the questions doc 23 l.376 lists: the claim Open/Close over a carrier anchor that
  moves, the pool seat, the `abi` drop on resume, the long-leg snapshots.
- **Alternative:** fold the note into the cut's PR.
- **Why:** three documents name S.4b their blocker and none owns its precondition.

**Q-38-22 — Capture's scope (F).**
- **Proposed default:** minimal custody. `CrewLocation::Captured { carrier, captor }` replaces
  Q-36-52's abandonment for a completed non-friend load; a captive is handed to a port's authority at
  docking and released for a fee, or held — no interrogation, no ransom market. Ejection is a forced
  extraction from a doomed unit with a capture chance when a Hostile party holds the cell.
- **Alternative:** the full prisoner economy the reviewers sketch.
- **Why:** Q-CF-38's ruling-out needs a new ruling, both reviewers say so, and the smallest custody
  state is the one later rulings can widen.

**Q-38-23 — Stances (H).**
- **Proposed default:** an organization-to-organization table in mush-faction (`data.rs:23–26`
  reserves it): Allied, Neutral, Hostile, declared by an organization's owner, read at admission and
  at a join, never mid-field; one rung above shared membership in `parties.rs`'s order and below a
  mark and fire. Faction effects as GC.10.
- **Alternative:** the table in mush-org.
- **Why:** mush-faction already says where the matrix goes; a change taking effect mid-field would
  re-relate live parties, which no ruling allows.

**Q-38-24 — Standing persistence and the void (H).**
- **Proposed default:** the fire tally and the void offers ride the live field record (as the
  held-out walls do after PR #340); the void window closes at the leader's next command,
  synchronously in the command path, not in the sweep; a director raid's leader voids its own
  party's friendly fire automatically.
- **Alternative:** RAM, as today.
- **Why:** a restart inside a void window should not decide the standing; the sweep's one-second
  gap is documented in the code.

**Q-38-25 — Reinforcement (G).**
- **Proposed default:** Q-36-5's alternative, bounded. A carrier on its pad — there at promotion, or
  arrived by F-6 — may launch into the running field; the launch enters at its ramp at the next
  pulse, counts against the cap, and has a launch cycle (provisional 60 s wall) so a bay cannot
  pour. The clocks stay as they are.
- **Alternative:** keep the refusal.
- **Why:** F-6 arrival already joins mid-field; a carrier that arrived and cannot launch is the half
  state Q-36-5 named.

**Q-38-26 — DT.4 and open-ground orders (G).**
- **Proposed default:** both stay as ruled (Q-36-43 deferred, Q-36-42 NEGATIVE) until play at the
  12v12 target shows the need.
- **Alternative:** one level of sub-leaders now.
- **Why:** both reviewers and both rulings agree.

**Q-38-27 — Component effects in the kernel (I).** **(expensive to reverse)**
- **Proposed default:** one in-match effect a component kind, authored in the content: Sensor —
  detection and lock range; Ecm and Eccm — their modifiers off; Power — heat capacity and charge;
  Relay — orders and comms delayed; Core — as the section's `DegradeCore`. Jammed is off until
  unjammed, damaged is degraded. One rebless with Q-38-17, `line_to`'s wrecks and `note_fire`'s fix.
- **Alternative:** the mount only, as today.
- **Why:** Q-36-38's combat half built the outcome and none of its consequences; repair, strip and
  the part market in A price parts whose damage means nothing in the match.

**Q-38-28 — Sustained battles (I).**
- **Proposed default:** `bytes_total` resets a segment on resume (`boot.rs:228`'s `segment += 1`)
  with the archive holding segments, so the 64 MiB bound is a segment's and the checkpoint already
  built carries the rest. `max_bytes` unchanged.
- **Alternative:** raise `max_bytes`.
- **Why:** the bound spans the field's whole life today; a reinforcement rule (Q-38-25) makes long
  fields the norm.

**Q-38-29 — The acceptance rule for this round (K).**
- **Proposed default:** every new cut's story runs at 256 and 512, and the bot types the player's
  half over SSH; the bot's four-digit addresses and its `pickup`, `scrap`, `+salvage`, `+bay`,
  `+field/void`, `+sortie/condition` vocabulary are the first cut of whichever document starts first.
- **Alternative:** one size a story, as GC did.
- **Why:** doc 36 §7's "stories grow" rule went unmet in every GC story; the rule is cheap when it is
  the first cut and dear when it is the last.

## 6. Which category gets which document

| Document | Categories | Shape |
|---|---|---|
| **Doc 39 — Industry: the production chain and unit construction** | A | a survey in this form (the chain's depth, recipes, facilities, the facility cost model — capital, lifetime, staffing, a daily cost —, the mass budget and the scrap ceiling, the part and hull bills, prices), then a plan of cuts over E7's engine; economy-owned |
| **Doc 40 — The field, second round** | G, H, I, C's kernel half, the Q-36-48 seam | one rebless (component effects, the coupled tow, `line_to`, `note_fire`, the join await), the stance table, durable standing, reinforcement; kernel-owned |
| **Doc 41 — Restoration and the salvage loop, second round** | B, C's tug half, D, the Q-36-46/47 seams, Q-38-6's retrieval contracts and Q-38-18's NPC salvage organization | the tender and the bay away from port, the loop's close, the crime consumer, the retrieval contract and its NPC fallback, the heavy crawler, the ship's tow on the same claim; waits on doc 39's lots and its facility cost model |
| **Doc 42 — Crew: custody, capture, ejection, treatment** | F | new rulings first (the reviewers are right that none exist); touches ownership and permissions |
| **Doc 23, a note** | E | "custody for moving anchors", then S.4b's cut as doc 23 already plans it |
| **Doc 35b §8** | J | already the re-plan; no flag (Q-38-9) |
| **Docs 36 and 37, cut lists** | K, L | no design text; cuts appended to the owed lists |

## 7. A provisional order

0. **The first routine redeploy (L)** whenever it suits: Q-38-9 and Q-38-10 are ruled, nothing
   gates it, and it ships `main` as it stands.
1. **Doc 39's survey**, now; beside it the doc 23 custody note (E), the stance table (H's half that
   needs no rebless), and the bot's four-digit addresses and vocabulary (K's first cut).
2. **Doc 39's plan and cuts**: the mill rows or recipes, `industrial_parts` and alloys, the part
   lot's price and stores, the mass budget (Q-38-5), the yard. Doc 42's survey in parallel (it
   consumes nothing from 39).
3. **Doc 41** once doc 39's lots exist: the tender, the bay on a site, the loop's close, the crime
   consumer. **Doc 40's rebless** planned through 1–2 and built once, when its four items are ready.
4. **S.4b** after its note; **capture** after doc 42's rulings.
5. **K** throughout (every cut's story at both sizes, the bot's half over SSH); **L** at each
   redeploy.

The reviewers' orders differ from this one at the top: reviewer 1 leads with S.4b and field
engineering, reviewer 2 with the seam fixes and S.4b. Both put capture in the middle and the salvage
loop last. Neither saw the chain, which is why §4 A is first here: field engineering, the salvage
loop and the component economy each draw from it.

## 8. What the implementation plans need before they can be written

Draft 0.2 carries the rulings. The plans still need:

- For doc 39: the goods catalogue's mass and price rows for alloys, `industrial_parts` and each part
  definition; which stations get which capability (`smelt`, `works`, `yard`); whether `MillDef`'s
  same-family rule is relaxed or recipes replace mill rows; the facility cost model's inputs for
  each facility kind (the yard, the refinery, the works, the workshop, the salvage organization's
  crawler): capital cost, expected lifetime, staffing, from which the daily cost and a hull's
  manufacturing cost follow (Q-38-12); and the scrap ceiling's mechanism (Q-38-5).
- For doc 40: the hash-pinned harness runs that the rebless re-blesses, listed before the first
  kernel change (doc 36 §4 Q-36-56's rule).
- For doc 41: doc 39's lot shapes and facility fees; the retrieval contract's shape (who posts, the
  fee's basis, the 28-IC-day window, the NPC towing company as the fallback acceptor) and the NPC
  salvage organization's content (Q-38-6, Q-38-18); and the fix thread's state on the vacated hull
  and the remains room check (§2.2), so the loop's close does not re-fix them.
- For doc 42: a reading of doc 27's crew and custody rulings (Q-H12, FO-12) and doc 32's Q-CF-38/39
  as the base the new rulings amend.
