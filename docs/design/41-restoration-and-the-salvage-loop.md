# 41. Restoration and the salvage loop, second round — surveyed

Draft 0.1, 2026-11-03, docs only. Surveyed against `origin/main` at `c83c2eff` (PR #339, SF.R15; doc
36a's lanes 1, 2, 4 and 5 merged as PRs #334, #343, #342 and #346; lane 3 open as PR #340), with
[doc 39](39-industry-production-chain.md) (PR #347, Draft 0.2 at `62218176`, being revised to Draft
0.3 after two outside reviews) stacked under it so the README row lands after doc 39's. This is the
document [doc 38](38-after-the-ic-phase-next-systems.md) §6 proposed as "Doc 41 — Restoration and
the salvage loop, second round": categories B (restoration away from port), C's tug half (heavy
recovery), D (the loop's close, forensics, crime), the Q-36-46/47 seams with space, Q-38-6's
retrieval contracts and Q-38-18's NPC salvage organization. Doc 38 §8's list of what it had to
settle is this document's table of contents. Three read-only lanes re-verified every anchor doc 38
§4 B, C and D cite at this tree; §3 carries the line numbers as they stand now and §3.10 the eight
places doc 38's reading was wrong or has moved.

The form is doc 38's: a survey that asks the owner numbered questions, each with a proposed default,
an alternative and a reason; a later draft carries the rulings and §6's cut list becomes the plan.
"Provisional" marks a number to be tuned later under the 2026-10-24 rule (take a reasonable default,
tune later, never hold a build on it). **(expensive to reverse)** keeps doc 38 Q-38-10's meaning: an
answer that would take a significant refactor of the codebase to undo — not stored data and not a
live box, since every step may wipe and redeploy. It appears once here (Q-41-7).

## 1. Why this document exists

Doc 36 built ground combat's aftermath: the tow (GC.4), salvage on a later visit (GC.5), the claim
with story (b) (GC.6), the bay's repair order (GC.8a) and its hold, provenance, strip and scrap
(GC.8b). Its "What the programme still owes" and doc 37 §5.2 left the loop open at both ends: a
vacated claim leaves the hull in the custodian's bay with no verb to return her; repairs at the
salvor's risk are never written; the recovery authorizations are hard `false`; an organization's hull
cannot be claimed; the two contest kinds the library holds have no verb; `SiteRecord.unlawful` is
written and read by nothing that acts; the bay works only docked at a port with a workshop, though
Q-36-35 ruled a carrier's bay counts for minor repairs; the tender of doc 27a §10 was never built;
every tug is `Standard` and no unit can tow a carrier wreck. Doc 38 surveyed those gaps as its
categories B, C and D, and the owner ruled on 2026-11-03 (Q-38-5, Q-38-6, Q-38-7, Q-38-16,
Q-38-18, Q-38-19, Q-38-20) what the shape must be. Doc 39 then fixed the chain this document draws
from: the part lot and its store, the facility cost model, the mass budget and the scrap ceiling,
and it named the NPC salvage organization (Q-39-24). §2 lists those rulings; they are binding and
are not re-asked.

What is left to decide is the mechanics: which unit is the tender and what it carries; what a bay
away from port draws from and who it pays; the retrieval contract's shape, fee and fallback; the
NPC organization's hulls and whether it works on its own account; how a vacated hull comes home and
what a port does with her; which evidence source each contest kind reads; what port security fines
and quarantines, and on what proof; and how the space tow rides the same path. Each gets a section
in §4 with what exists, what the rulings require and the questions it raises; §5 is the owner survey
(Q-41-1..26); §6 the plan, cuts RS.0–RS.9, ordered after doc 39's IN.2 and IN.7 where it depends on
them.

## 2. What binds this survey, and what it leaves alone

### 2.1 Binding rulings

| Ruling | What it fixes here |
|---|---|
| Q-38-5 (owner) | scrap is bounded by the hull's dry mass and worth at most about a fifth of its buyout; the mechanism is doc 39's (Q-39-16, IN.7); this document only consumes it for remains scrap on a site and for the salvage organization's own scrapping |
| Q-38-6 (owner) | repossession charges damage past a wear allowance (provisional a tenth of the hull's points) **plus** a towage fee: what the lessor would pay a PC or NPC salvage team; the retrieval is a contract players may accept, going to an NPC towing company if not accepted within 28 IC days; deposit first, the remainder a debt on the lessee's ledger; the dormancy path pays the same; "space tow truck" is an occupation, so the contract runs on Q-36-46's one claim path |
| Q-38-7 | the 28-day write-off bar follows a lease rig that came home a wreck: not returned within 28 days, title to Occator and the deposit forfeit |
| Q-38-16 | field engineering is both halves: a tender role (a support rig carrying munition and part lots) on the surface tick with `+tv/<n> unjam\|stabilize\|rearm` on an adjacent casualty through the three mush-tactics functions; and the carrier's bay working berthed on a site at the 2× factor for minor repairs, drawing lots from her hold; replace and rebuild refused away from a workshop port |
| Q-38-18 (owner) | an NPC salvage-retrieval organization at a main port owns a heavy crawler and whatever carries it, contractable by players or Occator through the retrieval contracts, the only recovery of a carrier wreck on the ground; ground tugs stay at the escort's scale with a recovery profile above `Standard` (a provisional 3× cap); the ship's tow from orbit stays a seam |
| Q-38-19 | the loop's close: a vacated hull is returned (the owner's own carrier may berth her out of any bay; a port yard holds her where no carrier of theirs is present); `documented_repairs_at_risk` written by every bay job under a Salvor right; recovery authority is a grant kind the title holder or the landholder issues (`+salvage/authorize`), read at filing; an org-titled hull with no lessee is claimed through the organization's owner read; contest kinds (defective procedure, superior right) are `+salvage/contest` switches naming the evidence source |
| Q-38-20 | a minimal crime consumer: port security reads `SiteRecord.unlawful` at docking, a fine on the actor's ledger scaled by the kind, a quarantine of the hull until paid; bounties and standing hits later |
| Q-38-29 | every new cut's story runs at 256 (Ahuna Flank) and 512 (Urvara Reach), the bot typing the player's half over SSH |
| Q-39-7, Q-39-24 | the NPC house arm runs on the port tick from the house's capital; Tycho Mutual Assurance & Salvage at Ceres is Q-38-18's organization, its recovery row in the cost model (4 M over 15 years, six staff, 2,100 a day, one slot), a retrieval fee = the job's days × 2,100 × 1.25 + the passage's propellant at the ask, quoted here |
| Q-39-9, Q-39-10, Q-39-11, Q-39-13, Q-39-16, Q-39-17 | the cost model's shape and inputs; the facility's stores are the operator's storefront and `Replace{lot: Store}` draws from it; `PartProvenance.origin`; the scrap mechanism (the capped yield at a workshop into the operator's stock, on a site into the hold); a hull's reference price |
| Q-36-14, Q-36-20, Q-36-22, Q-36-29, Q-36-33, Q-36-35, Q-36-36, Q-36-39 | anyone recovers on any duty; a tow in a field is coupled; the 28-day write-off of a rig left out; a written-off lease rig is Occator's alone; the rights table (a salvor stabilizes and repairs at risk, never strips or scraps); minor repairs in a carrier bay at 2× time; munitions are goods a carrier carries for field rearm; remains scrap where they lie |
| Q-36-24, Q-36-27, Q-36-28, Q-36-30, Q-36-46, Q-36-47 | the claim is an E4b2 contract; an uncontested claim becomes title, a contested one is adjudicated on the evidence; evidence is a witness's own records, the durable black box with bay access, and delivered reports; unlawful recovery was evidence only "in this phase"; one salvage path for ground and space **(expensive to reverse)**; one consequence table for carrier and ship damage |
| Doc 27 Q-S2, Q-S4, Q-S13, Q-S14, Q-S15, Q-R1; FO-1, FO-4 | custody is not title; enforcement needs acquired evidence, never "an automatic omniscient fine"; a contest proves unlawful recovery, defective procedure or a superior specific right, and bare title defeats nothing; the salvor's repair is at their own expense with no automatic reimbursement; the 48 available real hours; whoever leaves a unit has lost it; on owned ground owner and landholder are both entitled |

### 2.2 The fix thread's state, so the loop's close does not re-fix it

Doc 38 §2.2 left two items of this area with the fix thread. Their state at `c83c2eff`, read for
the cuts in §6:

| Item | State | What this document does |
|---|---|---|
| The vacated hull's missing return verb (doc 36 GC.6 "Not built here"; lane 2's note: "no verb returns her — a carrier of the owner's own must take her, which `+bay/stow` does not yet do") | a defect note, **unowned, no PR**; PR #340 (lane 3) does not touch it | designs the return as Q-38-19 says (§4.5) and builds it as RS.4; the note closes with that cut |
| `+tv/<n> scrap` on remains has no room check (`surface/claim.rs:1585–1680`, the prepare mints at `surface/tow.rs:1738–1813` with no capacity rule) | **unowned, no PR** | not this document's: doc 39's IN.7 rewrites that mint (the capped yield into the hold with the room check); RS.6's sweep consumes IN.7 and adds nothing to the mint |
| P0-5 `carried_rig` on a non-logistics hull; P0-8 the Irrecoverable tug's drop | PR #340, open at `e9ccb835` with main merged in | RS.6's crawler loads a carrier wreck into a bay, which is P0-5's path: RS.6 waits on #340 |
| **New, found by this survey:** `write_off_left_out` (`port/tv.rs:4772–4773`) builds `capture` from `lessor.account`; when the account does not resolve, `settle_deposit` **releases** the deposit (`:3466`) instead of capturing it, so a write-off can hand the lessee their deposit back | unowned | named for the fix thread; RS.5 touches the same function and carries a regression test if the thread has not taken it first |
| **New:** `+helm/tow` computes `filed_real_s = (ctx.w.now() / 1_000)` (`space/commands.rs:4487`) but `World::now()` is unix seconds (`mush-core/src/world.rs:198–199`), so the space claim's filing and expiry stamps are about a thousandth of what they should be | the space track's (doc 35b §8) | RS.8 replaces that path with the one claim path, which removes the stamp |

### 2.3 Left alone

The kernel half of recovery under fire — the towed casualty as a kernel unit, coupled to its tug
(Q-38-17) — is doc 40's rebless, with `Q-36-48`'s `PartyId` and the component effects; a tender or a
bay act *inside a promoted field* waits on it and is doc 40's (§4.1 says what this document does
while a field runs). The part lot's identity, mass and store, the yard, the pools, the scrap
mechanism and the facility cost model are doc 39's (its rulings bind; its cuts IN.5, IN.7 and IN.8
are referenced below "as revised in Draft 0.3", and each dependent cut in §6 states its dependency
in one line, never their mechanics). Crew aboard a hull in custody, capture and ejection are doc
42's. The service market (E8), a technician duty (Q-38-15), bounties and standing hits (Q-38-20's
"later"), facility assaults and persistent known-hostile memory stay out by ruling.

## 3. The tree at `c83c2eff`: what exists

### 3.1 The bay, and where it works

`facility_of` (`crates/tacsim/src/port/bay_work/mod.rs:203`) answers three cases in order: the hull
stands in a workshop room (`FacilityClass::Workshop`, `:205–219`); the hull itself is
`Berth::Docked` at a port with a workshop room and an operator (`:220–235`); the hull is in a bay
(`:236`, else "That hull is in no bay and no workshop."), `carrier_of_bay` finds the carrier
(`:237`, else "That hull is in no carrier's bay."), the carrier must be `Berth::Docked` (`:238`) and
`operator_of(def, map)` must answer (`:246`, which needs `def.workshop` and an org row with an
account, `:164–172`) — then `FacilityClass::CarrierBay` (`:251`). The refusal for an undocked
carrier is a bare string, twice (`:241–242`, `:247–248`): "A carrier's bay works on its hulls only
while she is docked at a port with a workshop to supply it." There is no refusal code; the
constants in the file (`NO_HULL` `:117`, `NO_RIGHT` `:123`, `HELD_OPEN` `:121`) are not used by it.
`carrier_of_bay` (`:176–188`) iterates `store.port_maps()` and each map's `hulls` (`:179`), never
`map.rigs`, `map.site` or a lane site's carriers. `Berth` (`port/ship.rs:369`) is `Docked { map,
def }`, `UnderWay` or `Elsewhere`; `berth_of` (`:387`) is spatial — `Docked` when the hull is
`Direct` on a domain some `PortMap` records — so a carrier landed on Ahuna Flank or Urvara Reach
reads `Elsewhere`.

`BayJob` (`bay_work/jobs.rs:49`) is `Armor`, `Rebuild`, `Mend`, `Unjam`, `Replace { component, lot:
Option<LotId> }` (`:72–78`; the quote builds `lot: None` `:337–339`, `jobs::work` maps `None` to
`Intact` `:491`; `Some(lot)` comes from `fit_job`, `mod.rs:788`) and `Rearm { component, rounds,
good, units }` (`:80`). `needs()` (`:96–103`) is the facility rule: Armor, Mend, Unjam and Rearm need
`CarrierBay`; Rebuild and Replace need `Workshop`; applied at `:404` (`partition(|job|
job.job.needs() <= class)`) and `mod.rs:826`. `CARRIER_TIME_FACTOR` is 2 (`:44`, applied `:199`).
The goods: `order_goods` (`mod.rs:577`) sums each Rearm's good and units and adds `industrial_parts`
(`PARTS_GOOD` `:128`); `hold_carries` (`:644`) pre-checks the hold; `source_carrier` (`:611`)
requires the carrier `Docked` at `facility.port` (`:626`); `source_hold` (`:636`) is
`ship::hold_vault` of that carrier; at completion `finish` (`:1126`) owes `OwedKind::Draw`
(`:1176–1191`), which `act.rs:701–741` performs by `inventory.debit` for bulk goods (`:713–715`) and
`consume_product` for a fitted lot (`:733`). Who is paid: the workshop's operator at the port the
carrier is docked at, "as the parts supplier" (doc 36 GC.8a), by an escrowed economy contract. The
munitions are four goods in `crates/tacsim/content/market/goods.json`, family `munitions`, stowage
`stow.bulk`: `ammo_ap_30` (`:165`, 1 kg, 12 cr a round), `ammo_ball_762` (`:179`, a 3 kg box of 100
at 25 cr), `ammo_he_105` (`:193`, 21 kg, 150 cr), `ammo_missile` (`:207`, 50 kg, 1,500 cr); no beam
charge is a good. `+bay` (`bay_work/command.rs:25`, registered `commands.rs:304–312`) has eight
switches: `quote`, `order`, `cancel`, `berth`, `stow`, `fit`, `strip`, `scrap` (`:27–29`); there is
no grant switch. Grants are `@unitgrant/issue #unit=#character <capability> [delegate] [until=…]`
(`crates/mush-org/src/unit_authority/commands.rs:17–58`) over `UnitCapability`
(`unit_authority/record.rs:37`: `OperateCrew`, `Deploy`, `CommandAssignedAi`, `RecoverLoad`,
`MaintainRefit`, `TransferAbandonTitle`); the bay reads `MaintainRefit` at `right_of`
(`mod.rs:377`, `:386`) through `unit_grant` (`:341`) and the built refit through
`bay_hull_title_stands` (`mush-industry/src/bay_refit/authority.rs:174`,
`mush-economy/src/bay_refit/authority.rs:212`). `RecoverLoad` ("unit.recover_load") is read today by
the sortie's recover command (`detachment/sortie/commands.rs:371`, `prepare.rs:969`) and nowhere
in salvage.

The units: `tv_prospector` 6 t dry (`crates/tacsim/content/units/tv_prospector.json:96`, hopper
12,000 kg of `stow.ore`, payload ceiling 12,100 kg), `tv_escort` 8 t (`tv_escort.json:331`, a 120 kg
provisions locker, payload ceiling 220 kg `:346`), the Dray and Dray-P 240 t (`dray.json:84`,
`dray_p.json:84`; the Dray-P's bay `:208`: two rigs, 20,000 kg, 70,000 L). No definition carries a
tender, support or recovery word, and no definition field names a tow profile.

### 3.2 The tender library (mush-tactics, no caller)

`FieldEngineeringBook` (`crates/mush-tactics/src/field_ops/engineering.rs:134`) holds
`tenders: BTreeMap<ObjId, FieldTenderState>` and `logs`. `FieldTenderState` (`:53`) is `tender_unit,
site_id, pose, operational_state, spare_parts_kg, ammo_supply_rounds, charge_reserve_cu,
max_service_distance_cm` — **plain numbers, not lots or goods**. `register_tender` (`:170`) asks
nothing of the unit's definition (no flag, no capacity read; only nonzero ids, a nonzero reach and
the `MAX_FIELD_TENDERS` 256 cap, `:188–194`). `validate_tender_reach` (`:778–808`) requires the
tender Operational or Unarmed and `distance_cm_to(target) <= max_service_distance_cm`
(`DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM` 1,000, `:36`; a different site is out of range);
`validate_field_environment` (`:761`) refuses a frozen clock, active combat and a negative duration;
a target coupled to an active tow is refused (doc 27a §10). The three functions take the duration
and the completion instant from the caller:

- `field_unjam_component` (`:237`): the component must read `Jammed` (Intact or Damaged is
  `ComponentNotJammed`); a stripped part, a part in a destroyed section or an irrecoverable hull is
  `RequiresWorkshopFacility`; one `SetComponentCondition` to `damaged` if the health cell is below
  its maximum, else `intact` (`:342–353`, plan tag `field_engineering.unjam`); subtracts
  `spare_parts_cost_kg` from the tender (`:366`).
- `field_stabilize_damaged_section` (`:400`): refuses a Destroyed section and an irrecoverable hull;
  needs `anatomy.internals_cell`; restores `min(requested, missing.max(1))` points to the section's
  internals and sets the section `intact` at its maximum, else `damaged` (`:503–512`); armour facets
  untouched; consumes `points × spare_parts_kg_per_point` (`:488`, `:526`).
- `field_resupply_ammo_and_charge` (`:558`): adds rounds to a mount's magazine capped at its
  capacity (`:679`), subtracts rounds and charge from the tender (`:699–704`), computes
  `billed_credits` through `UnitExpensePolicy::payer_for` (`:707–712`) and **posts no money**; the
  charge is subtracted from the tender and written to no target cell.

Every caller is inside `#[cfg(test)]` in `mush-tactics/src/field_ops/mod.rs` (`:1179`, the test
`field_engineering_tender_unjam_stabilize_resupply_and_workshop_boundary`; calls at `:1193–1331`).
tacsim re-exports the whole surface (`crates/tacsim/src/field_ops.rs:14–41`: the book, the state,
the action kind, the error, the log entry, the three defaults, the caps, the expense policy) and
uses only the error variant (`:675`). No `+tv` or field verb reaches any of it.

### 3.3 The tow, its profiles, the carrier's floor

The cap is inclusive twice the tug's mass (`mush-tactics/src/field_ops/recovery.rs:170–173`,
`tow_mass_ratio_permille`). `TowRigProfile` (`:149–155`) is `Standard` (default) and
`RecoveryOptimized`; **the enum carries no cap**: both profiles get the same 2×, and the profile
changes only the speed factor (`tow_speed_factor_permille`, `:197` Standard `850 − ratio × 300 /
1000`, `:198` RecoveryOptimized `950 − ratio × 225 / 1000`), read into the coupled drive by
`coupled_drive_rates_for_tow` (`tacsim/src/field_ops.rs:517`, `:527`) and `surface/act/plans.rs:604`.
Every attach is `rig_profile: TowRigProfile::Standard` (`surface/tow.rs:686`, in `attach_request`
`:679`); no production code builds the other. The carrier floor is `TUG_SHARE_PERMILLE` 500
(`crates/tacsim/src/carrier_damage.rs:116`, `tug_floor_kg` `:243–246`), applied by
`field::carrier::refuse_tow` (`field/carrier.rs:601`, refusal `TOW_CARRIER` `:41`): a 240 t Dray
wreck needs a 120 t tug, and the heaviest tug is the escort at 8 t. The consequence table is
`carrier_damage::TABLE` (`:101`): `Drive → [RefusesLift]`, `Hull → [RefusesLift, Wrecked,
CrewStranded, BayRigsWrecked]` over `CarrierPart { Hull, Drive }` (`:45`). Space has its own
`ShipSectionId { Hull, Hold, Drive }` (`space/bubble.rs:121`) and `SpaceSectionConsequence`
(`:165`: `DestroyUnit, DisableMotion, Slow { percent }`); nothing under `space/` references
`carrier_damage` or `CarrierPart`, so Q-36-47's mapping does not exist. Only `+tv/<n> tow` tows on
the ground (`port/tv.rs:2390–2394` → `tow_cmd`, `tow.rs:1884`, its one caller); the twenty `+tv`
words are `ACT_WORDS` (`tv.rs:1790–1793`), with `tow`, `release`, `load` parsed at `:2003` and
dispatched at `:2390/2395/2400`, `pickup` and `scrap` at `:2030` and `:2406/2413`. The reach is
`TOW_REACH_CM` 1,500 (`tow.rs:97`); the timings `[tacsim.recovery].inspect_s/attach_s/load_s` are
300/600/900 (`mush-config/src/config.rs:1168–1221`; tacsim's twin `tacsim/src/config.rs:144–149`).

### 3.4 Custody, the claim, the evidence

`SalvageCustody` (`surface/salvage.rs:94`: hull, holder, carrier, recovered_by, recovered_ic,
left_ic, claim, operation, pin, file, told, evidence `:128`) sits in `SiteRecord.custody`
(`surface/site_record.rs:483`), at most 32 a site. `+salvage` (`surface/claim.rs:997`, registered
`commands.rs:368/378`) has `/claim` (`:999`), `/contest <hull> [go]` (`:1003`), `/report <hull>=<who>`
(`:1007`), `/inspect` (`:1014`) and the bare list (`:1022`); `/contest` takes no kind and no source:
`contest_cmd` (`:1222`) picks the ground itself (`:1228–1243`): `evidence_of(me)` →
`UnlawfulRecovery`, else `prior_owner == me` → `BarePriorTitle`, else `NO_GROUND` (`:115–116`).
`DefectiveProcedure` and `SuperiorSpecificRight` are never constructed in tacsim. The claim is
`mush_economy::file_salvage_claim` (`crates/mush-economy/src/contract_live.rs:6009`, `SalvageClaimSpec`
`:5905`, the shape predicate `salvage_claim_hull` `:5929`), locked against every non-staff
`@contract` switch but `/draft` (`:306–324`, `SALVAGE_CLAIM_LOCKED` `:5887`, PR #334), awarded by
`award_salvage_title` which refuses **any organization-titled hull** (`:6277–6287`, `Unauthorized`).
The window is `claim_window_s` — already a tacsim config field (`tacsim/src/config.rs:154`, default
`SALVAGE_CLAIM_WINDOW_SECS` 48 × 3,600 at `:172`, `contract_live.rs:5896`; clamped `1..=` a week by
`window_secs`, `claim.rs:957–964`) that `mush-config` does not know and `mush-server/src/config_ext.rs:816–817`
hard-sets from the default ("not in the config file yet"); the stories set it on the struct
(`WINDOW = 600`, `tests/shard_1/field_gc6.rs:41/49`, `review_36a.rs:46/54`).

`open_claim` (`claim.rs:591`) writes `has_owner_authorization: false` and
`has_landholder_authorization: false` (`:602–603`; the struct is `RecoveryAuthorityContext`,
`mush-tactics/src/field_ops/salvage.rs:64`) and `documented_repairs_at_risk: Vec::new()` (`:622`;
type `Vec<AtRiskRepairEntry>`, `salvage.rs:490`; the entry `:414` is `description, cost_incurred,
performed_at_ic`). The authorizations are read only by `evaluate_recovery_entitlement`
(`salvage.rs:141`, `:152`), whose result is stored as `entitlement_at_attach` and read by `contest`
(`:559`); `adjudicate` (`:611`) reads neither the repairs nor the authorizations: a contest with
`valid_to_vacate` vacates (`:620`), else past the deadline the claimant is awarded (`:634`). Repairs
are pushed only by `record_repair_at_risk` (`:1343`), whose one caller is the private
`record_work_order` (`workshop.rs:1664`, the `salvage_book: Option<&mut SalvageClaimBook>` parameter
`:1673`, `:1686–1690`) on the **in-memory** book, not on the ground claim in `custody.file`; every
`WorkshopBook` method `jobs::work` calls passes `None` (`jobs.rs:457, :464, :471, :501, :519`).
`title_of` (`claim.rs:235–247`) answers a character's title directly and any other title only through
a live `+tv` lease. An organization has no owner field (`mush-org/src/record.rs:30`); acting for one
is capability-based, `mush_economy::acts_for` (`mush-economy/src/api.rs:835–859`, "economy.account.post")
and `principal_holders` (`notice.rs:356`) name its members and grantees; a hull's title is
`EconomicAsset.title_owner` (`record.rs:549`), programme hulls titled
`PrincipalId::Organization(OrgId(trust.org))` (`port/hull.rs:925–929`).

Evidence: the pin's witnesses are `WitnessKind::{Sensor, Known}` (`claim.rs:149`, built at
`:331–357`), turned into `SalvageEvidenceKind::SensorLog` and `DirectWitness` by `evidence_of`
(`:443–457`); `RecoveredBlackBox` is written by `+salvage/inspect` from aboard the custody carrier
(`:1324`, `aboard` `:1315`), which counts the box's open tail (`:1338`) and decodes nothing;
`CustodyInspection` (`salvage.rs:186`) is never constructed. The durable black box
(`surface/black_box.rs`, `append` `:1043`) carries two line kinds only, `MindSealed` (`:317`) and
`SortieClosed` (`:381`: close kind, `recovered_by`, `unrecorded`); the tow's load writes a
`SortieClosed { RecoveredToBase, recovered_by: tug }` (`tow.rs:1600–1610`); there are no hit, damage
or tow lines. Its readers are `+hull/box` (`surface/hull.rs:69`; a character title holder, staff, a
lessee from their lease's start; an org-held title is refused) and the inspect. The appends: `field/provider.rs:317`,
`detachment/sortie/prepare.rs:100` (doc 38 wrote `surface/prepare.rs`), `surface/tow.rs:1473`,
`surface/recovery.rs:1138`, `surface/act/providers.rs:3438`, `port/epoch.rs:1047/1075`. The claim's
notice goes by `tell` (`claim.rs:838`, private) through the `ClaimMail` hook (`:806`, installed by
`mush-server/src/boot.rs:815–826` over `mush_mail::post_notice`, `mush-mail/src/transit.rs:492`).
The forensics twins: `record_arena_notices_to_black_box` (`tacsim/src/field_ops.rs:598`) has one
`#[cfg(test)]` caller (`:1568`); `record_surface_black_box_line_to_forensics` (`:537`) has none;
`mark_stripped_component_contraband_on_vacated_claim` (`mush-tactics workshop.rs:660`) has none,
tests included; `strip::vacate_claim` (`bay_work/strip.rs:411`) is called from
`tests/shard_1/bay_gc8.rs:700` alone.

### 3.5 The vacated hull

`settle` (`claim.rs:743`) runs `record.custody.retain(|row| row.hull != hull)` **before** the
branch (`:778`, "a vacated claim's custody is void"), so a vacated claim **drops** the custody row —
doc 38 §4 D's "retains custody" is wrong — and logs `UnlawfulKind::Vacated` (`:781–792`). Nothing
moves. The hull stays in the custodian's bay with no row anyone can act on: the custodian's
`right_of` is `None` (no custody, no title), the title holder's is `Title` but `berth` (`mod.rs:1483`,
the check `:1513–1523`: "Hulls are berthed out of your own carrier's bay, or out of the bay that
holds your salvage.") needs the run of the carrier (`carrier_right` `:289`, the helm holder or the
yard operator) or custody aboard her; `stow` (`:1552`) passes `berth_stands` (`:323–337`: "Her
berth's carrier is not yours to use, and whoever berthed her holds no right over her now; a carrier
of your own must take her.") and returns her only to `row.bay_room` and `row.slot` (`:1581`). A
written-off lease rig that is loaded goes to Occator's shed instead (`tow.rs:2305–2309`,
`tv::hand_back` `tv.rs:4664`), which is the one physical return the tree has. The bay record
`tacsim:baywork:{hull}` (`BayHull`) holds the berth row and the order (GC.8a); the port map's
`HullRow` (`port/map.rs:415–426`) holds no owner, title, custody or location.

### 3.6 The lease, repossession, the write-off

`repossess` (`port/tv.rs:4996`) refuses the yard hull (`:5000–5002`), sends the rig to the shed
unless it is a wreck (`:5131–5133`), writes the record `Returned`, `repossessed: true`,
`written_off: wreck` (`:5138–5145`) and settles the deposit (`settle_deposit`, `:5096–5113`):
released within the paid term, captured whole past the term or for a wreck (`:5100`). It never
calls `restoration_charge`. It is reached by `@coop/revoke`, the dormancy recall and `return_hull`
through `recall_block_clearing` (`port/recall.rs:322–328`; `RecallCause::Dormant` `:145`). The
lessee's own return (`+tv/<n> lease return`, parsed `tv.rs:2105`, `give_back` `:4811`, `plan_return`
`:4364`) charges GC.9's `restoration_charge` (`:3654`: `quote_full_restoration` at the lessor
port's workshop multiplier, Occator's 1.2 at Ceres, lane `:2825–2826`) capped at the deposit
(`charged_minor` `:3625–3629`), the capture through `charge_deposit` (`:3556`, key
`tacsim.tv.charge/{lease}`), the rest released, nothing above the deposit; a rig past repair takes
the whole deposit (`:196–200`). **There is no wear allowance** (no `wear` or `allowance` in the
file). The deposit is the engine-owned E4 hold `tacsim.tv.deposit` (`DEPOSIT_KIND` `:1021`, placed in
the take's post `:4228–4245`, listed in `ENGINE_OWNED_RESERVATIONS` `mush-economy/src/api.rs:3120–3125`),
moved only by `settle_engine_reservation` (release `:3475`, capture `:3502`, part `:3556`). Two
28-day constants exist, both `pub const` and provisional: `WRITE_OFF_BAR_IC_DAYS` (`tv.rs:139`) is
the escort-clearance bar (`escort_check` `:1253`, `:1266–1278`, `ESCORT_WRITTEN_OFF` `:135`);
`WRITE_OFF_LEFT_IC_DAYS` (`:4642`) is the left-out write-off, `write_off_left_out` (`:4719`) on the
surface tick (`surface/tick.rs:460`), clocked by `left_out_since` (`:4687`: the site index's
`left_ic`, else the custody row's), ending the lease (`:4763`), capturing the deposit
(`:4772–4773`, the `capture: None` hole of §2.2), writing `Returned, written_off` (`:4778–4786`),
handing back to the shed unless the rig is on the ground (`:4787–4792`). A filed claim on a
written-off hull is closed by the tick (`claim.rs:1379–1394`); filing on one is refused
(`:1083–1084`, `WRITTEN_OFF` `:97–98`); `custody_of` answers `None` for one (`salvage.rs:159`). One
IC day is `DAY` 86,400 s (`tv.rs:1032`); `PLAY_RATE` is 24 (`port/readiness.rs:37`), so an IC day is
a real hour and 28 IC days are 28 real hours. The economy knows a bounded overdraft
(`CreditPolicy { allow_negative, floor }`, `mush-economy/src/record.rs:227–245`, default `forbid()`)
and a recurring `Obligation` with `Grace` and `Delinquent` states (`obligation.rs:391–413`, `:267–312`),
and no one-off debt or receivable.

### 3.7 Contracts: no kind, no open acceptance

There is no `ContractKind`: contracts are one `EconomicContract` (`contract.rs:731`) recognised by
shape (`is_lease_shaped` `contract_live.rs:4914`, `salvage_claim_hull` `:5929`) or by a
`ManagedWire` variant (`contract/managed.rs:24–60`: hull coverage, personal, the mind kinds, paid
resale). `@contract` (`:228–232`, dispatch `:289–368`) has `show, list, draft, party, pay, deliver,
split, remainder, condition, deadline, visibility, adjudicator, evidence, outcome, role, roster,
offer, accept, amend, attest, dispute, agree, cancel, resolve`; no `/post`. `acting_party`
(`:402–430`) admits only a listed party or someone acting for an org party, so **nobody can accept a
contract they were not named on**. The nearest board is Occator's survey board (`port/survey.rs`):
a static `OFFERS` list (`:72`), `+survey/accept <id> [go]` (`:29–30`, `:645`), where the pilot is
the issuer and author (`:225`, `offer_contract` `:678`, `accept_contract` `:688`), the sponsor
accepts later on the tick by `accept_contract_as` (`:826`, `:851`), and the fact condition
`tacsim.survey.logged` settles it (`:21–25`). Space pays a fixed 1,200 cr "Freeport Registry
Derelict Salvage Bounty" (`space/picket.rs:1643–1646`, offer, accept and registry acceptance in one
call `:1719–1736`, from `space/commands.rs:4571`).

### 3.8 The organizations and the lane

`crates/tacsim/content/lane/ceres_phobos.json` (the lane has grown since doc 39's `6f9dff29`; its
line numbers moved): Ceres Freeport's orgs at `:2741–2749` — `ceres_clearing` (50,000), `coop`
(3.1 M), `coop_trust` (250,000), `freeport_exchange` (4 M), `occator` (50,000 + a 250,000 raise),
**`tycho_mutual` "Tycho Mutual Assurance & Salvage"** (`:2747`, `capital_major` 50000, faction
`concord`, no other field, no vendor row, no hull), `freeport_registry` (50,000), `pallas_syndicate`
(`:2749`, 50,000, faction `syndicate`); Stickney's at `:2849–2851` (`kasei` 600,000, `stickney_yards`,
`yard_commissary`). Occator's lease rows `:2815–2822`: eight `tv_prospector` at 1,400 + 5,000 for
168 IC days, two `tv_escort` at 2,500 + 15,000; the shed "Occator Vehicle Shed" (`:2816`); the
workshop at 1.2 (`:2825–2826`). The Occator Yard (`port/yard.rs:1–16`, content `:3121–3123`): a
Dray-P house hull on Occator Rim's pad, titled to Occator, its hold a house vault, two house rigs,
`rearm_ic_secs` 3600, a DARK dispatcher operating it — the one NPC arm that drives rigs on a site.
There is no raider org in the lane (raiders are `content/field/raiders.json`). The recovery
provider doc 38 mentioned is `mush-campaign/src/recovery.rs:1–43`: a loaner hull on an ordinary
lease for a player who lost everything, off by default (`[tacsim.recovery].enabled`,
`mush-config/src/config.rs:50`); it is not a salvage or towing service.

### 3.9 Crime: the evidence, and where a port could read it

`SiteRecord.unlawful` (`site_record.rs:490`, at most `MAX_UNLAWFUL` 64, `claim.rs:79`) is written by
`log_unlawful` (`claim.rs:420–424`) from four places: `Vacated` (`claim.rs:782`), `Recovery`
(`tow.rs:1567`), `Pickup` (`:1722`), `Scrap` (`:1802`). `UnlawfulKind` (`claim.rs:389`) is `Pickup,
Recovery, Scrap, Vacated`; `UnlawfulAct` (`:405`) is `kind, actor: ObjId, subject: u32 (the hull or
site object), landholder: Option<ObjId>, at_ic`. Its production readers are `is_empty()`
(`site_record.rs:602`) and a bounds check (`:1065`). The docking commit point is mush-sim's
`apply_arrival` (`mush-sim/src/journey.rs:1783`), which calls `hooks.arrived` (`:1876`, live ticks
only) — tacsim's `TacsimCarrierHooks::arrived` (`composition/sim_routes.rs:146`) already branches
on `Berth::Docked` (`:163`); the ship epoch's close (`port/epoch.rs:324`, from `port_tick.rs:2107`)
is the later hook. Nothing named port security exists. Fines exist as precedents: `@registry/fine`
(`minds/registry_inspection/fine.rs:1`, cores only), the sanction (`mush-economy/src/api.rs:2157`),
the space customs fine (`space/picket.rs:1342`, `:1425`) and seizure (`:1525`), the core impound
(`match_/seal.rs:496`, `mush-economy/src/registry_filing/seizure.rs:422`). A departure is refused by
`DepartRefusalKind` (`port/voyage.rs:2586`: `Unwired, VoyageDark, Cleared, UnderWay, NotAboard,
NotDocked, NoRoute, … BayOver, Damaged`), which is where a quarantine would sit.

### 3.10 Space's tow, and corrections to doc 38 §4

`+helm/tow` (`space/commands.rs:71`, `:846`, `helm_tow` `:4447`) needs a prior hard-dock clamp,
repeats the 2× rule in its own code (`:4478`), and files **not** through `file_salvage_claim` (whose
one caller is `surface/claim.rs:1140`) but through `space::custody::register_derelict_salvage_in_book`
(`:4522`; `space/custody.rs:918`) on the in-memory `SalvageClaimBook` space persists, always with
tenure `Unclaimed`, `has_owner_authorization: recoverer == prior_owner` (`custody.rs:932`), the tug
hull as recoverer; the window `DERELICT_WINDOW_S` 48 × 3,600 (`space/bubble.rs:28`) is counted down
by `SPACE_TICK_PERIOD` (`space/tick.rs:1525–1543`); there is no contest verb and no adjudication in
`space/`; and the filing stamp is a thousandth of real time (§2.2). Doc 35b §8 already lists the
derelict that never moves and the tow record never cleared.

Eight places where doc 38 §4 B, C, D read the tree wrong or where it has moved since `afa082a7`:
a vacated claim **drops** custody (`claim.rs:778`), it does not retain it; tacsim re-exports the
whole engineering surface (`field_ops.rs:14–41`), not only the book; `TowRigProfile` carries no cap,
both profiles are 2× (`recovery.rs:170–173`, `:197–198`); `facility_of` returns a bare string, not a
code; the black box has only seal and close lines; `claim_window_s` is a tacsim config field
already, missing only its mush-config key; `file_salvage_claim` is at `:6009`, `carrier_damage.rs`
at the crate root, `tycho_mutual` at lane `:2747`, the lease rows at `:2815–2822`,
`berth_stands` at `mod.rs:323–337`, `berth` at `:1483`, `stow` at `:1552`, the caller of
`black_box::append` in `detachment/sortie/prepare.rs:100`; and `+helm/tow` has its own claim path on
the RAM book, so Q-36-46's "the fabricated principals are replaced" is still the whole of the seam.

## 4. The survey

### 4.1 The tender: the role, the verbs, the reach

**What the rulings require.** Q-38-16: a tender role — a support rig carrying munition and part
lots — on the surface tick, with `+tv/<n> unjam|stabilize|rearm` on an adjacent casualty through
the three mush-tactics functions. Q-36-35: the field-engineering boundary of doc 27a §10 stays for
units on the ground (unjam, stabilize, rearm; a destroyed part or section needs a facility).
Q-36-36: munitions are goods a carrier carries for field rearm. Q-36-14: anyone, on any duty. Doc
27a §10: no field engineering while either unit is in active combat or the target is coupled to a
live tow. A tender inside a promoted field is doc 40's.

**The role.** Nothing in a definition says "tender" and the library asks nothing of it (§3.2), so
the role is this document's to place. Proposed: a `tender` block in a unit definition's `logistics`
— `logistics.tender: { stores: "<stowage id>", reach_cm: 1500 }` — naming a stowage compartment
that accepts `stow.bulk` (munitions and `industrial_parts`), with `register_tender` fed from it. One
new definition carries it: **`tacsim.unit.tv_tender`**, class vehicle, the prospector's chassis
without the drill (the kit's prospector look, doc 22's sprites; a different fit, not a new model),
7 t dry (provisional), unarmed, one pilot, a `stores` compartment of 2,000 kg and 2,400 L
(provisional) of `stow.bulk`, `stowage_volume_l` 28,000, `recovery_profile: recovery_optimized`
(§4.8), so it rides the Dray-P's bay beside one escort (15 t of 20) or one prospector (13 t), and
Occator leases it on the rig lease's terms (provisional 1,800 + 7,000 deposit, pool 2; lane v21).
The alternative, the prospector's hopper doubling as stores (accepting `stow.bulk` beside
`stow.ore`), needs no definition but makes every miner a tender and mixes ore with rounds in one
compartment the unload empties. (Q-41-1.)

**What it carries, and what the library's numbers become.** The tender's reserves stop being
plain numbers: `spare_parts_kg` is the `industrial_parts` the stores hold, `ammo_supply_rounds` the
munition goods by family (the four rows of §3.1, matched to a mount the way the bay's rearm matches
them, `jobs.rs:378–399`), `charge_reserve_cu` nothing in the first cut (no shipped terrain vehicle
carries a beam; a charge transfer waits for one that does). `tacsim.part.*` lots ride in the stores
as cargo for a bay to fit later and are consumed by no field verb: a field replace is a workshop's
work by Q-36-35, and the tender's "part lots" in Q-38-16 are read as the fittings a patch draws.
The spare-parts figures are the tariff's until doc 39's IN.7 lands and the design's own
`fittings_kg_per_point` after it (the tender reads whatever the repair draws, so IN.7 changes a
constant, not the tender). (Q-41-2.)

**The verbs**, first words on `+tv` like the tow's, each a timed act on the surface tick, filed as
an op of `tacsim_surface_tow` (GC.6's precedent: `claim` and `scrap` joined GC.4's action rather
than opening a second) under one new op `tend`:

- `+tv/<n> unjam <unit>[=<mount>] [go]`: the preview names the jammed mounts in reach, the fittings
  each costs and the time; `go` files one act per mount (or the first, with no `=`).
- `+tv/<n> stabilize <unit>[=<section>] [points|all] [go]`: the damaged sections in reach, points
  missing, fittings a point, time a point; `all` to each section's maximum.
- `+tv/<n> rearm <unit>[=<mount>] [go]`: the mounts short of rounds, the good and units the stores
  hold for each, the time; a mount whose good the stores do not hold is listed as "no rounds aboard".

**Time** is the tariff's base for the same work (a component repair for unjam, a structural point
for stabilize, a round's loading for rearm; `workshop_tariff.json:3–17`) at a `TENDER_TIME_FACTOR`
of 3 (provisional): the workshop is 1, the carrier bay 2 (`CARRIER_TIME_FACTOR`), the tender 3, one
scale across the three classes. An unjam is 6 IC hours (15 real minutes at rate 24), a 50-point patch
1.5 IC hours, a 105 mm magazine of 30 rounds ten minutes. The act is interruptible as the tow's is:
due on a promoted site it lands interrupted; a field opening over the pair stops it; the tender
drives or tends, never both. (Q-41-3.)

**The money.** None between players: the tender's operator supplied the goods, and the goods are
the price. The library's `billed_credits` is computed and dropped. A tender working someone else's
hull is a gift or an arrangement outside the engine, as a wingman's tow is. (Q-41-4.)

**Reach** is the tow's: within a cell, diagonals included (`TOW_REACH_CM` 1,500), the tender
standing where a tug would; the library's `max_service_distance_cm` is set to 1,500 from the
definition's `reach_cm`. The tender and the target must both be on the site's index or the ground
(a casualty, a wreck, or an operational unit between fields; never a hull carried in tow, doc 27a
§10). Inside a promoted field every `tend` act waits, as every surface act does (Q-CF-30); doc 40's
coupled kernel unit is the only way a tender works under fire.

**What stabilize does to a casualty.** The library's rule stands: a Damaged section's internals
are restored up to their maximum (`min(requested, missing)`), the condition reads `intact` at the
maximum and `damaged` below it, armour facets are untouched, a Destroyed section is refused to the
workshop. On the shipped outcomes a casualty is usually past the tender — a hull is disabled when a
section or its drive part is destroyed, and the tender touches neither — so the tender's value is
between fields: a jammed mount cleared, a damaged drive patched before the next hit destroys it, a
magazine refilled from the carrier's stock without a lift. Where a casualty's disabling cause is a
Damaged section at its floor, the patch lifts the floor read and the hull is operational again on
the next check-in; nothing more is granted. The field unjam leaves a part `damaged` when its health
cell is down, exactly as the library writes it, so a field-unjammed mount may need the bay's mend.

### 4.2 The bay on a site

**What the rulings require.** Q-38-16 and Q-36-35: the carrier's bay works berthed on a site at the
2× factor for minor repairs, drawing lots from her hold; replace and rebuild refused away from a
workshop port. GC.8a called the docked-only rule "a cheap one to widen later".

**`facility_of` widened.** A fourth case after the three of §3.1: the hull is in a bay, the carrier is
`Berth::Elsewhere` or `UnderWay`, and the facility is `FacilityClass::CarrierBay` with `port: None`
and the carrier's own site (when she stands on one) or none. The refusal string retires: the only
refusals left are "in no bay and no workshop" and "in no carrier's bay". `source_carrier` stops
requiring `Docked` at a port (`mod.rs:626`): the hold that serves a bay hull away from port is the
hold of the carrier whose bay she stands in, which is the GC.8b rule without the port. `berth` and
`stow` do not apply away from port (there is no workshop room to berth into); the hull is held in
its slot by the `BayHold` while an order is open, as it is docked. Proposed: the bay works wherever
she is not docked — on a site **and under way** — since the bay is inside the hull and nothing about
a voyage stops a crew mending a section; the alternative is a site only, which Q-38-16's words name.
Under a field running on her site, bay acts wait as surface acts do; her bay's rigs are not in the
field (GC.3's rule). (Q-41-5.)

**Which jobs, and at what time.** `needs()` is already the table: Armor, Mend, Unjam and Rearm in a
carrier bay at `CARRIER_TIME_FACTOR` 2, Rebuild and Replace in a workshop. Away from port the quote's
"needs a workshop" lines name where: "The Salt Moth's cannon is destroyed: a workshop must replace
it — Occator's counter at Ceres Freeport, or Stickney Yards." (the ports whose `PortDef.workshop`
exists, nearest first by the lane). The part a replace needs is doc 39's store (`Replace{lot:
Store}`, IN.5 as revised in Draft 0.3) **and a carrier has no storefront, so she never replaces
away from port**: that is the whole of IN.5's bearing on this document. What she draws from her
hold is what GC.8b already draws — munitions and `industrial_parts` for the four minor jobs, and a
traced part lot for `+bay/fit`, which stays a workshop's order unless the owner says otherwise
(Q-41-6: fit from the hold on a site is the one part change that needs no store).

**Who is paid.** Docked, the port's workshop operator is paid "as the parts supplier" (GC.8a). Away
from port there is no supplier: the parts come from the hold and the labour is the crew's. Proposed:
no fee away from port — the time is the price (2×), which Q-36-35 named — and the escrow contract
is not filed; the alternative pays the carrier's operator org, which for a sole owner moves money
from one pocket to the other and for a chartered Dray pays the Co-op for its own captain's labour.
(Q-41-7, marked **(expensive to reverse)** only because the fee contract is read by `right_of`'s
payer path and the port party-cap source, both of which would have to learn a no-fee order.) The
Occator Yard's bay (the house Dray-P on Occator Rim) works the same way for the house rigs, which
retires the yard's own `rearm_ic_secs` timer as a special case once the dispatcher orders a rearm.

### 4.3 Retrieval contracts, and the repossession charge

**What the rulings require.** Q-38-6: a retrieval is a contract players may accept, going to an NPC
towing company if not accepted within 28 IC days; the fee is what the lessor would pay a PC or NPC
salvage team; the repossession charge is damage past a wear allowance (provisional a tenth of the
hull's points) plus that fee, the deposit first and the remainder a debt; the dormancy path pays
the same; a space tow truck is the same occupation. Q-38-7: the bar follows a wreck brought home.
Q-39-24: Tycho Mutual's quote is days × 2,100 × 1.25 plus the passage's propellant at the ask,
"which doc 41 quotes on the contract". Q-36-46: one claim path.

**The contract.** No kind enum exists and the salvage claim is recognised by shape (§3.7), so a
retrieval is the same: `mush_economy::post_retrieval(ctx, RetrievalSpec)` files one `Offered`
contract whose shape `retrieval_contract_hull(c)` recognises — issuer the poster, subject the hull
(a `TitleHeld`-style condition naming it), **destination** a port (its `PortDef` id), **fee** in
minor, deadline `WorldIc` at posting + 28 IC days (`RETRIEVAL_WINDOW_IC_DAYS`, provisional 28, a
config key beside `claim_window_s`), visibility `Public` (a board must be seen; this is the owner's
own notice of their own property, not doc 27 Q-S4's theft broadcast), audience Tycho Mutual. The
fee is escrowed at posting from the poster's account (Occator's capital under the capital clause;
a player's balance). Acceptance is the one thing the engine lacks: `acting_party` admits only a
named party. Proposed: `accept_retrieval(ctx, contract, acceptor)` adds the acceptor as the second
party and accepts in one post (the survey board's `accept_contract_as` precedent, run by the
acceptor's own command rather than a sponsor's tick), guarded by the shape — the generic
`@contract/accept` stays closed to strangers. **Delivery** is a fact condition,
`tacsim.retrieval.delivered`, posted by the act that puts the hull at the destination: `+bay/berth`
into the destination port's workshop room, a `+bay/stow` into the poster's own carrier docked
there, or the shed's `hand_back` for a lease rig; one predicate for ground and space — the hull
`Direct` on the destination port's domain (`berth_of` reads it), which a docked derelict satisfies
too (§4.7). Payment on delivery: the escrow pays the acceptor and the contract settles. (Q-41-8.)

**Who posts.** `repossess` and `write_off_left_out` post one automatically when the rig is not in
the shed and not aboard a carrier (left on a site, or on a site in a custody the write-off just
closed), destination Ceres Freeport, from Occator's account; any title holder posts one by hand —
`+salvage/retrieve <hull>[=<port>] [fee=<cr>] [go]` — for a hull of theirs lying on a site index, in
someone else's custody with no claim filed, or a wreck anywhere; and the lessor of a wreck under
Q-38-7 is Occator again. `+salvage/jobs` lists the open retrievals (hull, where it lies, destination,
fee, days left); `+salvage/accept <job> [go]` accepts. The bot learns the three. (Q-41-9.)

**The fee.** One basis for both PC and NPC: Tycho's quote is the floor and the market's reference —
`days × slot_day × (1 + margin)` + the passage's propellant at the ask, with the recovery row's
slot-day 2,100 and margin 25 % (Q-39-10, Q-39-24), `days` = the voyage planner's legs from Tycho's
port to the site and on to the destination, each rounded up to whole days, plus one ground day
(provisional). A poster may add a premium above the floor and never post under it, so a PC acceptor
is never paid less than the NPC would charge for the same job. Worked: a prospector left on Ahuna
Flank or Urvara Reach (both Ceres sites, a landing from Ceres orbit inside a day) is 2 days × 2,625
= **5,250 cr** plus the lifter's landing propellant — against the prospector's 5,000 cr deposit and
1,400 cr rent. The owner should see that figure: it says a rig abandoned on the ground costs its
lessee the deposit and a debt before any damage, and it says a player with a tender and a bay earns
a week's rent for an afternoon's tow, which is what "a viable occupation" means. Until doc 39's IN.2
lands, `slot_day` is a lane constant on Tycho's org row (2,100), replaced by the cost model's figure
when it does. (Q-41-10.)

**The window and the fallback.** At the WorldIc deadline with no acceptance, the sweep offers the
job to Tycho Mutual, which accepts it (`accept_contract_as` by the house arm) and runs it as §4.4
says. A PC acceptor carries a performance deadline (provisional 7 IC days from acceptance, 7 real
hours at rate 24): not delivered by then, the acceptance lapses, the job reopens for what is left
of the window or goes to Tycho if the window has run, and the acceptor loses nothing but the job;
the alternative bonds a tenth of the fee at acceptance, forfeit on a lapse. (Q-41-11.)

**The repossession charge.** `repossess` gains GC.9's assessment with two changes and one addition,
and `give_back` and the dormancy recall take the same: (1) the **wear allowance**: structural points
down by at most a tenth of the hull's total points are charged nothing; the quote is computed on the
points beyond (components repaired, replaced or rearmed are charged whole — the allowance is wear
on the structure, not a free part); (2) the charge is no longer capped at the deposit: the deposit is
captured first (the part-capture of GC.9) and the **remainder is a debt**; (3) the **retrieval fee**
is added when a retrieval is posted for the rig — the floor at posting, since the lessee is charged
what the lessor would pay, not what a premium bought. The debt: a one-term `Obligation` (debtor the
lessee, creditor Occator, due now, grace none), which the economy's sweep collects when the account
can cover it and which reads `Delinquent` until then; Occator's desk (`+tv/lease`, `+coop/enrol`)
refuses a delinquent debtor with the amount. The alternative is an account overdraft to a floor
(`CreditPolicy.floor`), which the ledger supports today but which hides the debt in a balance.
(Q-41-12, Q-41-13.)

**Q-38-7's bar.** `write_off_left_out` gains a second clock: a live lease whose rig reads a wreck or
`Irrecoverable` **inside its own carrier's bay** for 28 IC days from the load (`recovered_ic`) is
written off on the same terms — the lease ends, the deposit is captured whole (a wreck's rule), the
record `Returned, written_off`, the lessee told. The rig is Occator's property aboard a player's
carrier; it is handed to the shed at the carrier's next docking at the lessor's port (the load's
`hand_back` path run from the docking hook), and no retrieval is posted for a hull that is aboard
a carrier. A lessee who returns the wreck before the bar pays GC.9's charge as today, now with the
allowance and the debt. (Q-41-14.)

### 4.4 The NPC salvage-retrieval organization

**What the rulings require.** Q-38-18: at one of the main ports, outside the player's immediate
economic zone, owning a heavy crawler and whatever carries it; the fallback acceptor; the only
recovery of a carrier wreck on the ground; its fee from the cost model. Q-39-24 named it: Tycho
Mutual Assurance & Salvage at Ceres, the recovery row, the formula. Q-39-7: houses run on the port
tick from their own capital. The organization is doc 39's; this document gives it hulls, an arm and
a sweep, and re-asks nothing it ruled. Ceres is inside the player's zone, which the owner's words
put it outside of; the mitigation is that nothing of Tycho's is ever leased, sold or bought by a
player: its hulls are house hulls like the Occator Yard, reached only through the contract.

**Its hulls.** Two definitions and two house hulls on a Tycho berth at Ceres Freeport (lane v21):

- `tacsim.unit.tycho_crawler`, class vehicle, **120 t dry** (provisional; Q-36-8's half-tonnage floor
  for a 240 t Dray, and the 2× cap reaches 240 t — a Dray-P wreck with two wrecked rigs in its bay
  is 256 t, so the crawler is 130 t if the owner wants the full laden case), tracked, unarmed,
  `recovery_profile: heavy` (§4.8), a crew of two.
- `tacsim.unit.tycho_lifter`, class hauler, **600 t dry** (provisional), one bay slot for the
  crawler (130,000 kg, 400,000 L) and one for the load (`maximum_items` 1, 300,000 kg, 1,200,000 L),
  no hold to speak of, the Dray's drive rated for the mass; it lands where a Dray-P lands (doc 20's
  landing law reads thrust to weight; the lifter's figure is authored to clear a Ceres site and no
  more).

The alternative is a single landing recovery ship that is itself the tug — no crawler, the ship
clamps the wreck on the ground as `+helm/tow` clamps a derelict — which is cheaper to author and
makes the carrier wreck's recovery a ship's act, the seam Q-38-18 declined. (Q-41-15.)

**How it reaches a site, and what it does there.** The house arm is the Occator Yard's dispatcher
widened (`port/yard.rs`, DARK): on the tick after Tycho accepts, it files the lifter's voyage to the
site (the planner the Dray-P flies), lands, launches the crawler at the ramp, drives it to the hull
(the drive plan the yard's rigs use), attaches (`RecoveryBook::attach`, `heavy`), tows to the ramp
at the coupled rate, loads (the bay check counts the wreck's mass and volume), recovers the crawler,
lifts, docks at the destination and berths the hull into the workshop room (or hands a lease rig to
the shed), which posts `tacsim.retrieval.delivered`. Every step is a production act the tree has;
the arm only sequences them, and a step that refuses (a promoted site, a full berth) waits a tick
and tries again, as the yard's rigs do. A site with a field running is left alone until it settles.
The crawler and lifter are in the field's count while on the ground (Q-CF-21's cap) and admitted as
units of Tycho's party if a field opens over them, which is the one place an NPC hull of a concord
faction can be shot at; its loss is a wreck like any other, and Tycho posts a retrieval for it.

**On its own account.** Proposed: Tycho **sweeps**. A wreck that has lain on a lane site's index for
56 IC days (two write-off bars, provisional) with no claim filed and no retrieval posted is
recovered by Tycho under its own salvage claim on the one claim path — lawful on open ground;
lawful on owned ground only with the landholder's authorization (§4.5), which Tycho asks for by the
registry's notice and takes as given after 7 IC days of silence; a Registry seed reads as open
ground — then scrapped at Occator's workshop at Ceres under doc 39's IN.7 as revised (the yield into
Occator's stock, Tycho credited the capped value less the fee), so a raid's wrecks return to the
chain as alloys instead of filling a site record's object cap. Remains it never touches (anyone's
to scrap). The alternative is no account of its own: wrecks lie until a player claims them, and a
long-lived site fills. The raid director's wrecks are the case this decides. (Q-41-16.)

**Capital.** 50,000 and a raise of 500,000 (provisional; Occator's raise is 250,000), so it can
escrow its own fees and absorb a lost crawler; the capital clause bounds how many jobs it holds at
once (one slot, one job). Its fee income and its scrap credit are its recapitalization. (Q-41-17.)

### 4.5 The loop's close

**What the rulings require.** Q-38-19's four things, plus `claim_window_s` from config. Q-S14: the
salvor's repair is at their own expense, shown before spending, no automatic reimbursement.
Q-S13/Q-R1: a contest proves unlawful recovery, defective procedure or a superior specific right.

**The vacated hull's return.** At vacate the custody row goes (§3.5) and nothing can move her.
Proposed: `settle`'s vacate branch writes `held_for: Some(title_holder)` on the hull's bay record
(`BayHull`, the record that already holds her berth and order), and `right_of` answers that
principal `Right::Title` for berth and stow alone while the flag stands. Then: (1) **the owner's own
carrier takes her out of any bay**: `+bay/stow <hull>=<carrier>` — a new form of stow, by the title
holder (or their `MaintainRefit` grantee) aboard a carrier of theirs that stands where the
custodian's does (both docked at one port, or both on one site), moving her bay to bay with the
bay's mass and volume check; the custodian cannot refuse it. (2) **The port yard holds her**: at the
custodian's next docking at a port with a workshop, the docking hook berths every `held_for` hull in
the bay into the workshop room under the title holder's name (`+bay` lists her "held for <name>"),
freeing the custodian's slot; the title holder's carrier later `+bay/stow`s her from there. A full
berth count (2 at Ceres, 4 at Stickney) leaves her in the bay until one frees, which blocks that
slot of the custodian's — the consequence of carrying what proved not yours. A lease rig takes the
shed as today. The alternative is a hand-over at a port counter only (no bay-to-bay), which strands
a hull vacated on a site until the custodian chooses to dock. (Q-41-18, Q-41-19.)

**Repairs at risk, written and read.** `complete` already runs the work under `custody_for` from the
order's right (P2-5: a `PendingSalvageClaim` for a salvor); under `Right::Salvor` it now pushes an
`AtRiskRepairEntry { description: the job line, cost_incurred: the fee plus the goods at their
reference, performed_at_ic }` onto the custody row's own claim (`custody.file.documented_repairs_at_risk`)
in the same write, and the `+bay` quote under a Salvor right prints the Q-S14 line first: "At your
risk: the claim on her is open, and a failed claim repays nothing." **Read at adjudication** means
reported, not paid: the vacate and award notices to both parties list the repairs and their cost as
the record an agreed remedy can cite (`+salvage/report` already hands evidence on); `adjudicate`
stays as it is. The alternative — a lien the registry charges the title holder at return — is
Q-S14's rejected alternative in another coat. The mush-tactics `record_work_order` book parameter
is the RAM book's and is left alone; the durable write is tacsim's. (Q-41-20.)

**`+salvage/authorize`.** Two grantors, two records, one verb:

- the **title holder's** authorization is the grant kind that already exists: `UnitCapability::RecoverLoad`
  ("unit.recover_load"), issued per hull by `@unitgrant` today and read by the sortie's recover
  command; `+salvage/authorize <hull>=<who> [until=<ic>] [go]` fronts `@unitgrant/issue` for it, and
  the pin's `has_owner_authorization` (`claim.rs:602`) becomes `unit_grant(recoverer, hull,
  RecoverLoad)` through the same Org owner read the bay uses. No new grant kind.
- the **landholder's** authorization is a `recovery` delegation row on the block's Registry claim
  (`port/claims.rs`; the claim record the tenure read already resolves), issued by `+salvage/authorize
  <block>=<who> [until=] [go]` by the claim's holder, read into `has_landholder_authorization` at the
  pin from the casualty's cell.

Both are read **at the attach** (where the pin is written, Q-S3's "when physical securing begins"),
which is what Q-38-19's "read at filing" means in this tree: the filing reads the pin. An
authorization dated after the attach changes nothing. (Q-41-21.)

**Org-titled claims.** `title_of` gains the organization case: who answers for an org's title is
`principal_holders(org)` filtered to those `acts_for` the org (the "economy.account.post" holders,
which is the organization's owner read this engine has), each told at filing, any of them able to
contest; `award_salvage_title`'s blanket refusal of an org title (`:6277–6287`) narrows to what
P0-3 meant — a hull under a live lease or charter follows the lease rule (Q-36-29: never awarded;
its claim closes at the write-off) — so an org-titled hull with no lessee (a house Dray lost on a
raid, Tycho's crawler, a Co-op hull after its charter lapsed) is claimable and awardable. (Q-41-22.)

**The contest kinds and their evidence.** `+salvage/contest <hull>[=recovery|procedure|right] [go]`,
the bare form keeping today's automatic choice:

| Kind | Library ground | Evidence source the switch names | Who may file |
|---|---|---|---|
| `recovery` | `UnlawfulRecovery` | the challenger's own `SensorLog` or `DirectWitness` (the pin's witnesses, delivered reports) | anyone holding such evidence |
| `procedure` | `DefectiveProcedure` | **the registry's own records**: the contract's audience against `principal_holders` of the title (someone who answers was not told), the filing registry against the site's, a pin missing or replaced (P1-4), and the hull's black box — the registry reads the `SortieClosed { RecoveredToBase, recovered_by }` line and vacates a claim whose claimant is not the recoverer the box names | whoever answers for the title |
| `right` | `SuperiorSpecificRight` | a recovery authorization (the grant or the delegation row) held by the challenger and dated before the attach, or a retrieval contract on the hull accepted by the challenger before the attach | its holder |

This resolves GC.6's contradiction: the black box is **never a player's evidence** of an unlawful
recovery (a player cannot reach it from outside the custodian's carrier, and it holds no hit or tow
lines), and story (b)'s contest stays on the sensor record; the box **is** read — by the registry,
the system principal that may read any hull's box — at a `procedure` adjudication, to check the
claim against the one fact the box records about a recovery. The `RecoveredBlackBox` evidence the
inspect writes keeps its one use, binding the inspector to the custody. (Q-41-23.)

**`claim_window_s` from config.** `TacsimRecoveryCfg` gains `claim_window_s` (default 172,800) and
`config_ext.rs:816–817` maps it; the stories keep setting the struct. A line, in RS.3.

### 4.6 Crime: the minimal consumer

**What the rulings require.** Q-38-20: port security reads `SiteRecord.unlawful` at docking, a fine
on the actor's ledger scaled by the kind, a quarantine of the hull until paid; bounties later.
Q-36-30: evidence only "in this phase". **Doc 27 Q-S4 (accepted):** enforcement needs acquired
evidence — a witness, sensor report, inspection or recovered log — never "an automatic omniscient
fine". The two rulings meet only if the port acts on proof, not on the bare row.

**What port security reads.** At `TacsimCarrierHooks::arrived`'s `Docked` branch, at a port whose
registry answers for lane sites (the Freeport Registry: the Ceres ports and Stickney; a frontier
site has no security), for every principal aboard the docking carrier (the helm holder, the
operator, the crew the carrier's row lists), every `UnlawfulAct` across the lane's site records that
names them as `actor` and is **proven and unsettled**. Proven means one of two things: a `Vacated`
row on the same hull whose vacating contest was `recovery` (the registry adjudicated it), or a row a
witness has reported to the registry — `+salvage/report <hull>=registry`, a new recipient for the
existing verb, which hands the challenger's `SensorLog`/`DirectWitness` to the clerk and marks the
row reported. A bare row nobody contested or reported stays what Q-36-30 made it, evidence. The
alternative fines every row at docking, which is Q-S4's rejected omniscient fine and makes every
unlawful pickup on a Registry seed a fine nobody saw. (Q-41-24.)

**The fine and the quarantine.** Provisional, by kind, to the Freeport Registry's account: `Pickup`
500 cr, `Scrap` 2,000 cr, `Recovery` 5,000 cr; a `Vacated` row carries no fine of its own but proves
the `Recovery` row it follows. The fine posts from the actor's account at docking when it covers it
(the customs fine's one-post shape, `picket.rs:1425`); when it does not, the shortfall is the
one-term obligation of §4.3 and the **carrier the actor docked aboard is quarantined**:
`DepartRefusalKind::Quarantined` in `prepare_departure`, the line "The Salt Moth is held at Ceres
Freeport: a fine of 5,000.00cr stands against her master; it lifts when it is paid.", released on
the sweep after the obligation settles. The row is marked `settled` with the receipt so it is never
charged twice. The alternative scales the fine by the subject hull's reference price (doc 39's
Q-39-17), which fines a Dray's unlawful tow at a fifth of a Dray and a prospector's at a trifle.
(Q-41-25.)

**Contraband on a vacated claim.** `settle`'s vacate branch calls `strip::vacate_claim(hull)`, which
flags every `tacsim.bay_parts` row whose claim is the vacated one hot (its existing behaviour), and
the mush-tactics `mark_stripped_component_contraband_on_vacated_claim` retires (deleted, with its
RAM-book semantics) so one function answers. Honestly stated: on the rights table as ruled (Q-36-33,
Q-F5: a salvor never strips) no part is ever stripped under a claim, so the flag is dormant until a
path strips under one — a title obtained by an award that a later `procedure` finding reopens is
the one such path this document can name, and it is not built. The wiring is a line and closes the
seam; it is not re-asked.

**What stays for later.** Standing hits in mush-faction (GC.10's receipts carry the shape), a bounty
the registry posts on a delinquent actor (the retrieval contract's shape with a person as subject),
a wanted read at the airlock rather than the berth, enforcement on open ground by a security hull,
and persistent known-hostile memory (the Epistemic Engine).

### 4.7 The space seams

**Q-36-46, one salvage path.** The spec for the space track (doc 35b §8 owns the cut): `+helm/tow`
files through `mush_economy::file_salvage_claim` with the same `claim_window_s`, the same audience,
the same `award_salvage_title`; its custody record is a `SalvageCustody` on a new `tacsim:space:custody`
document keyed by domain (a space site has no `SiteRecord`), with the pin's tenure
`GroundTenure::Unclaimed` and the witnesses the bubble's presence rows; its stages are the clamp
(attach, timed by `attach_s`), the tow (the coupled drive the space tick already rates) and the dock
(load, into the berth gallery, with the bay check where a hull is carried); its principals the real
ones (the actor, the derelict's title); `register_derelict_salvage_in_book`, `DERELICT_WINDOW_S` and
the 1,200 cr bounty retire. `+salvage/claim|contest|report|inspect` then work on a space custody as
on a ground one, through the same `claim.rs` paths given a custody source. **What a space tow truck
needs from §4.3:** nothing more than the delivery predicate — a derelict towed to a port and docked
is `Direct` on the port's domain, so `tacsim.retrieval.delivered` fires for a ship as for a crawler;
a retrieval's `destination` is a port either way, and `+salvage/jobs` lists space jobs beside ground
ones. (Q-41-26 asks only who builds it.)

**Q-36-47, one consequence table.** `ShipSectionId { Hull, Hold, Drive }` maps onto `CarrierPart`:
`Hull → Hull`, `Drive → Drive`, and `Hold` onto nothing in the table (its `Slow { percent }` stays a
space-only consequence read beside the table), so `carrier_damage::TABLE` is read by the space
bubble's section loss as by the ground's check-in, and a carrier that lost her drive in a bubble
refuses lift at her next port exactly as one that lost it on Urvara. (Folded into Q-41-26.)

**Q-36-48** (`SpacePartyId` to `PartyId`) is doc 40's with the rebless.

### 4.8 Recovery profiles: the heavy crawler and the escort-scale tug

**What the rulings require.** Q-38-18: ground tugs at the escort's scale with a recovery profile
authored above `Standard` (a provisional 3× cap); the crawler. Q-36-8: a carrier wreck's tug is at
least half its tonnage (the floor, separate from the cap).

**The profile carries the cap.** `TowRigProfile` gains a cap multiplier beside its speed factor:
`Standard` 2× (today's, `recovery.rs:170–173` reads the profile instead of the constant),
`RecoveryOptimized` **3×** (the ruling's provisional figure), and a new `Heavy` with the optimized
speed factor and a 2× cap (a 120 t crawler reaches 240 t; the floor, not the cap, is what sizes it
for a carrier). The profile is authored on the unit definition — `logistics.recovery_profile:
standard|recovery_optimized|heavy`, default `standard` — and `attach_request` reads it from the
spec instead of hard-coding `Standard` (`tow.rs:686`). The tender (§4.1) is the escort-scale
recovery rig doc 27a §10 named in the same breath ("Mobile Recovery Rigs & Resupply Tenders"): at 7 t
and 3× it tows 21 t, which is any shipped terrain vehicle and nothing of a carrier's. Nothing here
touches the kernel: the profile lives in mush-tactics' recovery book and the host's attach, so no
practice hash moves. The alternative puts the cap on the definition directly (`tow_cap_permille`)
and keeps the profile for speed only. (Q-41-27.)

### 4.9 Acceptance

Q-38-29 rules: every cut's story at 256 and 512 with the bot typing the player's half; doc 39 §4.9
read it as the ground half at both sizes and the port half once, which this document follows.
Doc 36's three stories are **extended, never replaced** (doc 37 §5.2's complaint that no story ran
whole at both sizes and that (c) used a fixture instead of (a)'s casualty is answered here):

- **(a) The recovery run, extended.** After GC.4's tow on Ahuna Flank (256) and the same script on
  Urvara Reach (512): the party's tender clears the escort's jammed cannon and patches its damaged
  chassis on the ground (`+tv/2 unjam`, `+tv/2 stabilize`) and refills a magazine from its stores
  (`+tv/2 rearm`); in the Dray-P's bay on the site, `+bay/order` mends the damaged section at twice
  the workshop's time from the hold's fittings, and a replace is refused with the line naming the
  nearest workshop; the carrier lifts for Stickney and the bay keeps working under way.
- **(b) The contested salvage, extended.** On Urvara Reach (512) and Ahuna Flank (256): T is now an
  organization's officer and the hulls are org-titled; O's claim tells every holder (`procedure`
  stands when one is missed); L holds a recovery authorization T issued before the raid
  (`+salvage/authorize`) and contests `right`; T contests `recovery` on the sensor record and the
  claim vacates; O's bay quote under the Salvor right read "at your risk" and the at-risk repair is
  in the vacate notice; at Ceres, T's carrier `+bay/stow`s the vacated hull out of O's bay; in the
  variant where T has no carrier docked, the port berths her "held for T" at O's docking; O's proven
  unlawful recovery is fined at that docking and O's carrier is quarantined until the obligation is
  paid; the window is compressed and paused across a freeze as before.
- **(c) The refit run, extended.** Story (a)'s casualty, not a fixture, is quoted at Stickney Yards;
  the replace draws doc 39's store (IN.5 as revised) and the destroyed part is nothing; the charter's
  half: a dormant lessee's damaged prospector on Ahuna (256) and Urvara (512) is repossessed with the
  wear allowance, the retrieval fee and a debt beyond the deposit; the retrieval is on `+salvage/jobs`,
  a second player accepts, tows and loads it, docks at Ceres, hands it to the shed and is paid; the
  variant where nobody accepts runs 28 IC days under the clock and Tycho's lifter comes.
- **(d) The carrier wreck (new).** GC.3's fought hull loss on Urvara (512) and on Ahuna (256): the
  owner posts a retrieval; no player tug can take her (the floor's line); Tycho's lifter lands, the
  crawler tows her to the pad, loads her with her wrecked bay rigs, lifts, docks at Ceres and berths
  her in C14; the fee pays; `+bay` quotes the rebuild.
- **(e) The sweep (new).** A raider wreck left 56 IC days on Ahuna is claimed by Tycho, retrieved and
  scrapped at Ceres; the matter census holds to the tonne (doc 39's IN.10 story takes the line).

Every step through the production command paths (`+tv`, `+bay`, `+salvage`, `@contract` for the
staff view, `+ship`), never a staff hand past the seams doc 36a §8 allows; the bot's vocabulary for
the player's half is RS.0's.

## 5. Owner survey

Each question: a proposed default, an alternative, the reason. Numbers are provisional unless the
line says otherwise.

### 5.1 The tender

**Q-41-1 — The tender's unit.**
- **Proposed default:** a new `tacsim.unit.tv_tender`: the prospector's chassis without the drill,
  7 t dry, unarmed, one pilot, a 2,000 kg / 2,400 L `stow.bulk` stores compartment, `recovery_profile:
  recovery_optimized`, leased by Occator at 1,800 + 7,000 for 168 IC days (pool 2); the role is
  `logistics.tender { stores, reach_cm }` on the definition.
- **Alternative:** the prospector's hopper accepts `stow.bulk` beside `stow.ore` and every miner is a
  tender.
- **Why:** Q-38-16 says "a support rig"; a stores compartment the unload never empties keeps rounds out
  of the ore run; 7 t rides beside an escort in a 20 t bay.

**Q-41-2 — What the tender carries and consumes.**
- **Proposed default:** `industrial_parts` (for unjam and stabilize, at the tariff's kg a point
  until IN.7, the design's `fittings_kg_per_point` after it) and the four munition goods (for rearm,
  matched to a mount as the bay matches them); `tacsim.part.*` lots ride as cargo and no field verb
  fits one; no charge transfer in the first cut.
- **Alternative:** a field replace of a Damaged (not Destroyed) part from a lot in the stores.
- **Why:** Q-36-35 keeps part replacement a workshop's; the library's `spare_parts_kg` is the
  fittings and nothing else; no shipped terrain vehicle carries a beam.

**Q-41-3 — Time, reach, and the field.**
- **Proposed default:** the tariff's base time for the same work at `TENDER_TIME_FACTOR` 3 (workshop
  1, carrier bay 2, tender 3); reach the tow's one cell (1,500 cm); every `tend` act waits while a
  field runs on the site and lands interrupted if one opens; a tender under fire is doc 40's.
- **Alternative:** the library's own fixed durations and 1,000 cm reach.
- **Why:** one scale across the three facility classes is a rule a player can predict; the tender
  stands where a tug stands; Q-CF-30 already holds every surface act under a field.

**Q-41-4 — The money between players.**
- **Proposed default:** none: the operator's goods are the price, `billed_credits` is dropped.
- **Alternative:** a tariff line per act paid to the tender's operator.
- **Why:** a wingman's tow is free; a tender working for hire is the service market (E8), unplanned.

### 5.2 The bay on a site

**Q-41-5 — Where the carrier's bay works.**
- **Proposed default:** wherever she is not docked — on a site and under way — for Armor, Mend,
  Unjam and Rearm at 2×, from her own hold, waiting under a field on her site.
- **Alternative:** on a site only, as Q-38-16's words name.
- **Why:** the bay is inside the hull; a voyage home is when a crew mends; the hold is aboard either
  way.

**Q-41-6 — `+bay/fit` from the hold away from port.**
- **Proposed default:** allowed: a traced, clean lot of the slot's row from the hold fits in the
  carrier's bay at 2× — the one part change that needs no store.
- **Alternative:** fit stays a workshop's order, as GC.8b built it.
- **Why:** a stripped cannon in the hold and a `Missing` slot beside it is the salvage loop's own
  reward; refusing it on a site sends the carrier home to do what it carries the part for.

**Q-41-7 — Who is paid away from port.** **(expensive to reverse)**
- **Proposed default:** nobody: no fee contract is filed, the time (2×) is the price.
- **Alternative:** the labour fee paid to the carrier's operator organization.
- **Why:** there is no parts supplier to pay; a sole owner paying themself is noise and a chartered
  captain paying the Co-op for their own hours is wrong. Expensive because the order's payer path
  and the port party-cap source both assume a fee.

### 5.3 Retrieval contracts and the charge

**Q-41-8 — The retrieval contract's shape.**
- **Proposed default:** an `EconomicContract` recognised by shape (`retrieval_contract_hull`), issuer
  the poster, subject the hull, destination a port, fee escrowed at posting, `WorldIc` deadline at
  28 IC days (`RETRIEVAL_WINDOW_IC_DAYS`, a config key), `Public`, audience Tycho; `accept_retrieval`
  adds the acceptor as a party and accepts in one post, guarded by the shape; delivery the fact
  `tacsim.retrieval.delivered` posted when the hull is `Direct` on the destination port's domain;
  payment on delivery.
- **Alternative:** a `ManagedWire` variant (a closed managed kind) with its own wire.
- **Why:** the salvage claim and the lease are shape-recognised and the lock on the claim is a
  shape check; one delivery predicate serves a crawler's berth and a ship's dock.

**Q-41-9 — Who posts, and the verbs.**
- **Proposed default:** `repossess` and `write_off_left_out` post automatically for a rig left on a
  site, destination Ceres Freeport, from Occator's account; any title holder posts with
  `+salvage/retrieve <hull>[=<port>] [fee=] [go]`; `+salvage/jobs` lists, `+salvage/accept <job> go`
  accepts; the bot learns the three.
- **Alternative:** `@contract` switches for players.
- **Why:** `@contract/draft` is a staff grammar (endpoints by number); the retrieval is the salvage
  verb's other half.

**Q-41-10 — The fee's basis.**
- **Proposed default:** Tycho's quote is the floor for PC and NPC alike: `days × 2,100 × 1.25` + the
  passage's propellant at the ask, `days` the planner's legs rounded up plus one ground day; a poster
  may add a premium, never post under the floor; until IN.2 lands the slot-day is a lane constant on
  Tycho's row. A prospector on a Ceres site is 5,250 cr plus propellant — more than its deposit.
- **Alternative:** the poster's free fee with Tycho's quote shown.
- **Why:** Q-38-6 says the charge is what the lessor would pay; one basis means a PC tug is never
  underbid by the house; the owner should look at 5,250 against 5,000 before it ships.

**Q-41-11 — A PC acceptor who fails.**
- **Proposed default:** a performance deadline of 7 IC days from acceptance; past it the acceptance
  lapses, the job reopens or goes to Tycho, no penalty.
- **Alternative:** a bond of a tenth of the fee at acceptance, forfeit on a lapse.
- **Why:** a first job board should not take money to look at; a griefer who accepts and sits costs
  the poster seven real hours, which the alternative is there to price if it happens.

**Q-41-12 — The wear allowance.**
- **Proposed default:** structural points down by at most a tenth of the hull's total points are
  free; the quote runs on the points beyond; components repaired, replaced or rearmed are charged
  whole.
- **Alternative:** a tenth off the whole quote.
- **Why:** the ruling says "a tenth of the hull's points"; wear is structure, a lost part is not wear.

**Q-41-13 — The debt beyond the deposit.**
- **Proposed default:** a one-term `Obligation` (debtor the lessee, creditor Occator, due now), collected
  by the sweep when the account covers it, `Delinquent` until then; the desk refuses a delinquent
  debtor a new lease or charter and names the amount.
- **Alternative:** an overdraft to a floor on the lessee's account (`CreditPolicy.floor`).
- **Why:** the obligation engine already knows delinquency and a creditor; an overdraft hides a debt
  inside a balance and lets it be spent.

**Q-41-14 — Q-38-7's bar on a wreck aboard.**
- **Proposed default:** 28 IC days from the load into the bay; the write-off captures the deposit
  whole; the rig goes to the shed at the carrier's next docking at the lessor's port; no retrieval is
  posted for a hull aboard a carrier.
- **Alternative:** a retrieval is posted and the lessee charged its fee as well.
- **Why:** the rig is already in a hull bound for port; charging a tow for a hull nobody tows is a
  fee for nothing.

### 5.4 Tycho Mutual

**Q-41-15 — Tycho's hulls.**
- **Proposed default:** a 120 t tracked `tycho_crawler` (`heavy`) and a 600 t `tycho_lifter` with a
  crawler slot and a 300 t / 1,200 m3 load slot, both house hulls on a Tycho berth at Ceres, never
  leased or sold.
- **Alternative:** one landing recovery ship that is itself the tug, clamping the wreck on the
  ground as `+helm/tow` clamps a derelict.
- **Why:** the owner chose the crawler over the ship's tow; a crawler needs a carrier and a Dray's
  20 t bay is not it; two authored hulls are content.

**Q-41-16 — Tycho on its own account.**
- **Proposed default:** it sweeps: a wreck 56 IC days on a lane site with no claim and no retrieval
  is claimed by Tycho on the one claim path (open ground; owned ground with the landholder's
  authorization, taken as given after 7 IC days of silence), retrieved and scrapped at Occator's
  workshop under IN.7, the alloys into the chain; remains untouched.
- **Alternative:** never on its own account; wrecks lie until a player claims them.
- **Why:** the raid director leaves wrecks every raid and a site record caps its objects; a sweeper
  that scraps returns the matter to Kasei's chain, which is Q-38-14's conservation.

**Q-41-17 — Tycho's capital.**
- **Proposed default:** 50,000 and a raise of 500,000; one job at a time (one slot).
- **Alternative:** no raise; the capital clause stalls it after a few fees.
- **Why:** it escrows its own fees as the fallback acceptor and may lose a crawler.

### 5.5 The loop's close

**Q-41-18 — The vacated hull's return.**
- **Proposed default:** `held_for` on the bay record at vacate; `+bay/stow <hull>=<carrier>` bay to
  bay by the title holder from a carrier of theirs standing where the custodian's does; the custodian
  cannot refuse.
- **Alternative:** a hand-over at a port counter only.
- **Why:** Q-38-19 says the owner's carrier may berth her out of any bay; a hull vacated on a site
  should not wait for the custodian to feel like docking.

**Q-41-19 — The port yard holding her.**
- **Proposed default:** at the custodian's next docking at a workshop port, the docking hook berths
  every `held_for` hull into the workshop room under the owner's name; a full berth count leaves her
  in the bay, blocking that slot, until one frees.
- **Alternative:** an unbounded impound room per port.
- **Why:** the workshop room and its berths exist; a blocked slot is the cost of carrying what proved
  not yours.

**Q-41-20 — Repairs at risk.**
- **Proposed default:** written on the custody row's claim by `complete` under a Salvor right, the
  Q-S14 warning first in the quote; reported in the vacate and award notices; never paid
  automatically.
- **Alternative:** a lien the registry charges the title holder at return.
- **Why:** Q-S14 ruled no automatic reimbursement and rejected the fee-for-recovery loop; the record
  is what a later agreed remedy cites.

**Q-41-21 — The authorization grant.**
- **Proposed default:** the title holder's is `UnitCapability::RecoverLoad`, issued per hull through
  `+salvage/authorize <hull>=<who>` fronting `@unitgrant`; the landholder's is a `recovery` delegation
  row on the block's Registry claim through `+salvage/authorize <block>=<who>`; both read at the
  attach into the pin.
- **Alternative:** a new grant kind for both.
- **Why:** a delegable "recover and load this unit" capability already exists and is read by the
  sortie's recover; a block's rights live on its claim record.

**Q-41-22 — Org-titled claims.**
- **Proposed default:** who answers for an organization's title is its `economy.account.post`
  holders; each is told, any may contest; the award's blanket refusal narrows to hulls under a live
  lease or charter (the lease rule).
- **Alternative:** org titles stay unclaimable.
- **Why:** an organization has no owner field and its owner read is the capability; a house hull
  lost on a raid and never recovered is FO-1's lost unit like any other.

**Q-41-23 — The contest kinds and the black box.**
- **Proposed default:** `+salvage/contest <hull>[=recovery|procedure|right]`: `recovery` on the
  challenger's sensor or witness evidence; `procedure` on the registry's own records (the audience
  against who answers, the filing registry, the pin, and the hull's box read by the registry — a
  claimant who is not the box's `recovered_by` is vacated); `right` on an authorization or a retrieval
  contract dated before the attach. The box is never a player's evidence; story (b) stays on the
  sensor record.
- **Alternative:** the title holder may read the box from outside the carrier.
- **Why:** the box holds only seal and close lines and the close line names the recoverer; that is a
  registry's check, not a witness's; reading a box through a hull is the omniscience Q-S4 forbids.

### 5.6 Crime

**Q-41-24 — What port security acts on.**
- **Proposed default:** proven rows only: a `Recovery` row proved by a `recovery` vacate, or any row a
  witness reported to the registry (`+salvage/report <hull>=registry`); bare rows stay evidence.
- **Alternative:** every row at docking, Q-38-20 read literally.
- **Why:** doc 27 Q-S4 forbids the automatic omniscient fine; a fine on a reported or adjudicated
  row is "if caught", which is the ruling's model.

**Q-41-25 — The fine and the quarantine.**
- **Proposed default:** `Pickup` 500, `Scrap` 2,000, `Recovery` 5,000 cr to the Freeport Registry,
  posted at docking from the actor's account; the shortfall an obligation and the docking carrier
  quarantined (`DepartRefusalKind::Quarantined`) until it settles; the row marked settled.
- **Alternative:** scaled by the subject hull's reference price.
- **Why:** a kind is what Q-38-20 named; a reference-scaled fine makes a prospector's theft a trifle
  and a Dray's a fifth of a Dray.

### 5.7 Space and the profiles

**Q-41-26 — The space seams' builder, and the `Hold` section.**
- **Proposed default:** the space track builds RS.8 from §4.7's spec in its own tree (`+helm/tow`
  onto `file_salvage_claim`, a `tacsim:space:custody` document, the stages, real principals, the
  retire of the RAM book path and the bounty; `ShipSectionId::Hold` onto no `CarrierPart`, its `Slow`
  kept space-side).
- **Alternative:** a doc 41 lane builds it; `CarrierPart` gains `Hold` with an empty loss list.
- **Why:** doc 35b §8 owns the space tree and the re-plan; a part with no loss in the table is a
  variant every ground reader must match for nothing.

**Q-41-27 — The cap on the profile.**
- **Proposed default:** `TowRigProfile` carries a cap: `Standard` 2×, `RecoveryOptimized` 3×, `Heavy`
  2× at the optimized speed; authored as `logistics.recovery_profile` on the definition, read by
  `attach_request`; the carrier floor stays `TUG_SHARE_PERMILLE`.
- **Alternative:** a `tow_cap_permille` on the definition, the profile for speed only.
- **Why:** the ruling names a profile above `Standard` with a 3× cap; one enum holding both figures
  is one place to tune; no kernel hash moves.

## 6. The plan: cuts RS.0–RS.9

Each cut lands as one PR, tested through the production command paths with its story at both
ground sizes (Q-38-29); each cut's note goes to `41-cut-notes.md`. Doc 39's rulings bind
throughout; where a cut consumes one of doc 39's cuts it says so in one line and names it "as
revised in Draft 0.3", never its mechanics. **Dependencies:** RS.0 ∥ RS.1 ∥ RS.2 ∥ RS.3 may start
now; RS.4 after RS.3 (they share `claim.rs`'s `settle` and the bay record); RS.5 after RS.3 (one
writer in `contract_live.rs`); RS.6 after RS.5, doc 39's IN.2 and IN.7, and PR #340; RS.7 after
RS.3 and RS.5; RS.8 after RS.3 and RS.5, in the space tree; RS.9 last. One marker: Q-41-7 in RS.2.

| Cut | Rulings | What | Depends on doc 39 | Acceptance (the story) |
|---|---|---|---|---|
| **RS.0** the vocabulary | Q-38-29 | the bot's `+tv/<n> pickup\|scrap`, `+salvage/*`, `+bay/*`, `+field/void`, `+sortie/condition` lines and the four-digit address parser (`mine/lines.rs:343–353`), unless doc 39's IN.10 or doc 40 has taken them first; this document and its README row | — | the bot types story (b)'s `+salvage/claim`, `/contest` and `+tv/1 scrap` on Urvara over SSH and the lines classify |
| **RS.1** the tender | Q-41-1..4, Q-41-27, Q-38-16, Q-36-35/36 | `logistics.tender` and `logistics.recovery_profile` on the definition; the cap on `TowRigProfile`; `tv_tender` in the kit and Occator's lease row (lane v21); the `tend` op on `tacsim_surface_tow` with `unjam`, `stabilize`, `rearm` from the stores at `TENDER_TIME_FACTOR` 3; `register_tender` fed from the compartment | IN.7 as revised (the fittings a point); the tariff's constant until it lands | (a) extended, the ground half at 256 and 512: a jammed cannon cleared, a chassis patched, a magazine refilled on the ground, each act interrupted by a field and resumed |
| **RS.2** the bay away from port | Q-41-5..7, Q-38-16, Q-36-35 | `facility_of`'s fourth case; `source_carrier` without the port; no fee contract away from port; the "needs a workshop" line naming the nearest; `+bay/fit` from the hold on a site; bay acts waiting under a field; the yard's rearm through the bay | none (replace is refused away from port, so IN.5 does not reach it) | (a) extended, the bay half: a mend at 2× on the site from the hold, a replace refused with the line, the order finishing under way |
| **RS.3** the claim's close | Q-41-20..23, Q-38-19, Q-S13, Q-S14 | `claim_window_s` into `TacsimRecoveryCfg`; `+salvage/authorize` (RecoverLoad for a hull, the delegation row for a block) read at the attach; org-titled claims through `principal_holders` and the narrowed award refusal; at-risk repairs written at `complete` under Salvor and reported; `+salvage/contest =recovery\|procedure\|right` with the registry's box read; `strip::vacate_claim` wired at settle, the mush-tactics twin retired | none | (b) extended at 512 and 256: the three contest kinds each vacating or failing on its named source; an org's holders told; the at-risk line in the quote and the notice |
| **RS.4** the vacated hull's return | Q-41-18, Q-41-19, Q-38-19 | `held_for` on the bay record; `+bay/stow <hull>=<carrier>` bay to bay; the docking hook berthing held hulls into the workshop room; the fix thread's defect note closed | none | (b) extended: T's carrier takes her out of O's bay at Ceres; without one, the port holds her "held for T" and T's next carrier stows her |
| **RS.5** the retrieval contract and the charge | Q-41-8..14, Q-38-6, Q-38-7 | `post_retrieval`, the shape, `accept_retrieval`, `+salvage/retrieve\|jobs\|accept`, `RETRIEVAL_WINDOW_IC_DAYS`, the delivery fact and payment; the wear allowance, the retrieval fee and the one-term obligation at `repossess`, `give_back` and the recall; Q-38-7's second clock; the `capture: None` hole closed with a test | IN.2 as revised (Tycho's slot-day); a lane constant until it lands | (c) extended: a dormant charter's prospector on Ahuna and on Urvara repossessed with allowance, fee and debt; a second player accepts the job, delivers to the shed and is paid; the desk refuses the delinquent lessee |
| **RS.6** Tycho Mutual | Q-41-15..17, Q-38-18, Q-39-7, Q-39-24 | the crawler and lifter definitions, Tycho's berth, pool and raise (lane v21); the house arm sequencing voyage, landing, launch, tow, load, lift, dock, berth; the fallback acceptance at the window's end; the fee from the recovery row; the 56-day sweep with the scrap at Occator's workshop | IN.2 and IN.7 as revised; PR #340 (P0-5, a wreck in a bay) | (d) at 512 and 256: the carrier wreck retrieved by the lifter and berthed at C14, the fee paid; (c)'s no-acceptor variant; (e) the sweep with the census |
| **RS.7** port security | Q-41-24, Q-41-25, Q-38-20, Q-S4 | the `arrived` hook's read of proven rows across the lane's site records; `+salvage/report <hull>=registry`; the fines by kind to the Registry; the obligation on a shortfall; `DepartRefusalKind::Quarantined` and its release; rows marked settled | none | (b) extended: O fined at docking after the `recovery` vacate, O's carrier held until the obligation settles, a bare pickup row never fined |
| **RS.8** the space seams | Q-41-26, Q-36-46, Q-36-47 | `+helm/tow` onto `file_salvage_claim` and `award_salvage_title`; `tacsim:space:custody`; the stages; real principals; the RAM path and the bounty retired; `ShipSectionId` onto `CarrierPart`; the retrieval's delivery at a dock | none | a derelict clamped, towed and docked at Ceres under the one window, contested by `+salvage/contest`, and a space retrieval job on `+salvage/jobs` delivered by a ship; built in the space tree (doc 35b §8) |
| **RS.9** the stories whole | Q-38-29 | (a)–(e) run whole at 256 and 512 with the bot typing the player's half over SSH; `41-cut-notes.md` closed | IN.10's census line for (e) | every story green on gate.yml at both sizes; the bot run recorded |

## 7. What the first cuts need, and what I could not default

Content and schema the cuts author as they land, listed once: lane v21 (the tender's lease row,
Tycho's berth, pool, hulls and raise, `RETRIEVAL_WINDOW_IC_DAYS` beside `claim_window_s` in
`[tacsim.recovery]`), the three unit definitions in the kit (`tv_tender`, `tycho_crawler`,
`tycho_lifter`) and their looks (doc 22's bake, the prospector's sprites for the tender; the crawler
and lifter need a look the kit does not have), the `tend` op's refusal codes in the surface act
table, the `tacsim.retrieval.delivered` fact, the `Quarantined` refusal line, the help passages
(`docs/help/field.md`'s recovery section gains the tender, the retrieval and the return), and the
`41-cut-notes.md` file.

**Questions I could not default with confidence**, for the owner's eye before the cuts start:
Q-41-10's figure (a retrieval on a Ceres site at 5,250 cr exceeds a prospector's deposit, so an
abandoned rig always leaves a debt; the ruling intends a real occupation, but the number was derived
from doc 39's provisional slot-day, not chosen); Q-41-16 (whether an NPC sweeper claiming wrecks
after 56 days is the owner's intent for the raid director's leavings, or whether wrecks should lie
as the field's memory); Q-41-15's hulls (a 600 t lifter is the largest hull in the game by far and
needs a look and a landing figure nobody has authored); Q-41-24 (reading Q-38-20's "reads unlawful
at docking" through Q-S4's "no omniscient fine" is my reconciliation of two rulings, and the owner
may mean the literal one); Q-41-7 (no fee away from port makes the carrier's bay cheaper than the
workshop for minor work, which the 2× time may not offset at rate 24); and Q-41-23's reading of the
black box as the registry's instrument rather than a player's, which closes GC.6's contradiction one
way when the owner may prefer the other.
