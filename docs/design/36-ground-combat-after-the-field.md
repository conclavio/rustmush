# 36. Ground combat after the field — recovery, repair and the field's open edges

Draft 0.3, 2026-11-02, docs only. Surveyed against `origin/main` at `d5ba6b2c`. Draft 0.1, the
same day, was the survey: it asked the owner questions, each with a proposed default and an
alternative, in the form docs 27 §7 and 32 §4 used. **Draft 0.2 records the owner's rulings of
2026-11-02 on every question:** Q-36-8, Q-36-11, Q-36-14, Q-36-29 and Q-36-35 amended, every
other default accepted as written. §4a lists what the amendments change elsewhere. **Draft
0.3, the same day,** replaces §7's provisional order with the cut list GC.0–GC.13, planned from
the rulings.

## 1. Why this document exists

The IC combat field is built. Doc 32's cuts CF.0–CF.7, CF.6b, CF.6c, the bot cut G-4 and the
512-site cuts S.0–S.2 are on `main`. The doc 32 story passes at 256 and at 512 a side, the bot
flies a captain and an escort over SSH, and `help field` is in review (PR #289). Doc 27b
(PR #195), the earlier plan for this ground, was closed on 2026-11-02 because doc 32 absorbed
it. Its §7 table of what follows the first skirmish is the starting point here.

A field today ends with units left on the ground, as a `casualty` or a `wreck`, kept in the
site record across restarts and visits. Nothing can pick them up, fix them or fight over them
afterwards. Around the field itself, several edges were left open: carrier arrival, a raider's
barge that never leaves, wrecks that give no cover, a 24-unit cap that was never measured, and a
staff-run raider.

This survey collects all of that into one place, so the next design doc and its cuts start from
rulings, not from a hunt through docs 23, 26, 27, 27a, 31, 32 and 35. It covers:

- **H2: recovery** (§4 D). Towing a casualty home: who may do it, on which clock, in what stages,
  into which bay.
- **H2: entitlement** (§4 E). Salvage claims, contest windows, evidence, and title.
- **H3: restoration** (§4 F). Inspection, repair, rearm, replacement, refit, stripping and
  scrapping, through doc 31's bay.
- **The field's open edges** (§4 B, C, G). Carrier arrival (F-6), reinforcement (F-4), the
  unarmed barge, wreck cover (G-8), carrier loss (F-3), the unit cap, organization relations
  (F-1) and faction effects (F-2).
- **Opposition, orders, minds, crew** (§4 G, H). A real opposing force, SCREEN and SUPPORT on
  open ground, DT.4, licensed minds, treatment.
- **The seam with space** (§4 I). Doc 35 built a live, separate salvage path and a separate
  consequence schema; both touch this work.

**Two things are outside this survey and stay that way unless the owner says otherwise:** the
10 km ground (doc 16 D-16-5), and infantry or battle armor as units (doc 27 FO-6).

## 2. Already decided; not asked again

| Decision | Where | What it fixes here |
|---|---|---|
| A disabled unit stays as it was disabled; whoever leaves it has lost it; recovery never heals it | doc 27 FO-1 | Recovery and restoration are separate |
| Salvage is its own operation: tow or drag, then load | doc 27 FO-3, Q-S5 (inspect, connect, tow, load are separate timed, interruptible stages) | §4 D asks only the timings and the clock |
| On open ground anyone may recover; on owned ground the unit's owner and the landholder may; theft is physically possible and enforced through evidence, never a broadcast | doc 27 FO-4, Q-S4, Q-H6 | §4 E asks where the evidence comes from |
| Any mobile unit may tow one casualty of up to 2× its tonnage; speed falls with the difference; recovery rigs exist but are not required | doc 27 FO-13, Q-S6; doc 27a §2.1 (Standard `850−300r`, RecoveryOptimized `950−225r`) | The tow rate is settled; §4 D asks who drives it |
| Driving the tug, holding the casualty and the right to remove it are three separate things; one physical writer; placement moves into the carrier only when loading completes | doc 27 §4.3–4.5 | §4 D/E keep them apart |
| Possession gives custody and a claim, never title; bare prior title never defeats a lawful claim | doc 27 Q-S2, Q-S13, Q-R1 | §4 E asks the claim's home, not its meaning |
| The first completed attachment wins; casualties stay indefinitely; recovery may happen during combat without extra damage | doc 27 Q-S9, Q-S10, Q-S7 | §4 D asks what promotion does to a tow |
| Loading reserves real bay volume and mass through the existing compartment owners, not a parallel counter | doc 27 Q-S8 | §4 D asks the capacity basis |
| The contest window is 48 **available real** hours, paused by outages | doc 27 Q-S15, Q-F3 | §3a records that space took IC hours instead |
| A non-owner's bay closes the epoch `RecoveredToBase`; the brief says who reads the black box | doc 27 P-3; doc 23 D-23-1 | §4 D/E use these close reasons |
| Destructive stripping or scrapping needs resolved title or the owner's permission; a stripped part gets its own claim and keeps its identity, condition and parent hull | doc 27 Q-F5, Q-F6, Q-S11 | §4 F asks how lots carry it |
| A minimal workshop ships before broad economic playtesting: replace a head/cockpit or engine, repair a damaged or jammed weapon, replace a slagged one with a compatible part; authorized resources and work time; capability re-checked; never reset to spec | doc 27 Q-S12 | §4 F asks cost, time, labour and authority |
| While title is pending, the custodian may stabilize and do documented repair at their own risk, with no automatic reimbursement | doc 27 Q-S14 | Salvors may repair; §4 F confirms |
| Losses stay on the assets; expenses come only from authorized budgets or contracts; `maintain/refit` is a separate, delegable permission a loan may leave out | doc 27 Q-A4, Q-A1 | §4 F asks whether the bay honours it |
| Field engineering in the field: unjam, stabilize, resupply ammunition and charge; destroyed or stripped parts and wrecks need a workshop | doc 27a §10 | §4 F asks which facilities count |
| The bay is a front end over work orders, never a direct edit; it quotes first and runs on the IC clock at a facility with `unit.maintain_refit`; refit is a design change within a refit family, at most one part out and one in | doc 31 D-31-13..15 | §4 F builds on UD.4b |
| Repair is for IC hulls only; damage shows on the doll, sprites stay whole | doc 31 R4, R2 | Not re-asked |
| One kernel; parties and relations as beliefs; one field per site; everyone on the ground admitted; no launches while promoted; the carrier is attackable; 24 units a field | doc 32 Q-CF-1..51 | §4 B revisits only F-4/F-6 and the cap, as follow-ups doc 32 itself opened |
| Ground salvage comes in four kinds (casualty, wreck, remains, cargo); no decay; 64 objects with remains merged at the cap; per-observer knowledge, no global wreck map | doc 32 Q-CF-24..28 | §4 D asks who first drops remains |
| Space derelicts keep an open epoch and fall under doc 27a's 2× cap and 48 h contest | doc 35 D-35-12 | §4 I asks for one claim path, not two |
| Economy and content numbers are provisional; take defaults, tune later, flag the expensive-to-reverse ones | owner rule 2026-10-24 | Every number here is provisional unless marked |
| IC fields are inherently multi-party; never scope IC design down to two sides | owner rule 2026-10-30 | Recovery and raids are multi-party too |

## 3. What the tree has (checked at `d5ba6b2c`)

**The site record** (`crates/tacsim/src/surface/site_record.rs`). `LeftCondition{Casualty,Wreck}`
(l.149), `LeftUnit{unit, pose, condition, epoch, left_ic}` (l.180), `GroundKind::Remains` and
`Cargo` (l.197), per-observer `KnownSalvage` (l.406). `demote` (l.673) replaces the whole unit
index. `take_unit` (l.749) and `take_object` (l.804) are called only by the staff fixture `apply`.
No player path removes anything from a site.

**Field close** (`crates/tacsim/src/field/close.rs`, `demote_units`, l.173–283). It indexes an
`Irrecoverable` hull as a `Wreck` with its epoch still open (l.243). It never drops a `Remains`
object, and nothing closes an epoch `Destroyed`.

**Towing and loading.** There is no tow verb on the ground. `field::carrier::refuse_tow`
(`field/carrier.rs:239`) has only a test caller. The DT recover (`detachment/sortie/facts.rs:168`)
refuses with "arrange physical towing and loading" unless the unit has power, control and
motion, and returns a unit only to its own reserved bay slot. Bays count items only
(`BaySlotDef.maximum_items` 1–4, `MAX_BAYS` 2, `definitions/logistics.rs:391–420`).

**`CrewDuty::Recovery`** is a hire and activation word. Its one effect is a Friend's recovery
flight holding back an extraction (`field/crew.rs:529–560`). The surface refuses it any act
(`detachment/sortie/surface.rs:55`, "recovery stays its own duty's"), so a recovery flight
cannot even drive.

**The field-ops library after PR #264.** `mush_tactics::field_ops::{condition, recovery,
salvage, workshop, engineering, crew}` remain. With no production callers:
- recovery: `RecoveryBook::{attach, advance_tow, detach, load_into_bay}` and `BayCapacitySpec`;
- salvage: `evaluate_recovery_entitlement`, the evidence gate, `submit_contest`,
  `adjudicate_claim`;
- workshop: `WorkshopBook` with `strip_component`, `scrap_hull`, `repair_section`,
  `reconstruct_section`, `repair_component`, `replace_component_in_slot`, `rearm_component`,
  `quote_full_restoration`, and a `WorkshopFacilityTariff` of defaults. Every method takes its
  cost and time from the caller, mutates at once and moves no money; the book is never stored;
- engineering: `field_resupply_ammo_and_charge`, which computes a bill and posts nothing;
- `tacsim::field_ops`: `coupled_drive_rates_for_tow`, `ground_tenure_at_site_pose` and others,
  test-only.

The live callers are `assess_tacsim_unit`, the field's check-in (`apply_ic_combat_return`,
`close.rs:205`) and space (below).

**The condition vocabulary.** `ComponentCondition` is Intact, Damaged, Jammed or Destroyed
(`field_ops/condition.rs:46–55`). There is no Missing: a stripped slot is the bare string
`"stripped"`, which classifies as Destroyed. In combat a component's condition follows from its
section's (`condition.rs:739–758`). The arena never jams or damages a component on its own, and
sets a section `destroyed` only on its "fresh" transition (`arena/damage.rs`).

**Ammunition.** The `ammo` cell is filled at unit creation from `count_per_load`. There is no
restock. Ammunition packs have no price, mass or commodity, and `goods.json` has no munitions
row.

**The refit bay (doc 31 UD.4b).** It is built as an industry order, an economy fee hold, inventory
part lots and a tactics `component_refit` participant, at one work unit per IC hour:
- **The fee** is whatever the supplier types (`bay_refit/provider.rs:295`), capped by
  `BAY_MAX_FEE_MINOR`, not the tariff.
- **Authority** requires the requester to hold the hull's title personally
  (`bay_refit/authority.rs`); `UnitCapability::MaintainRefit` is never read.
- **A returned part's lot** carries an order lineage and a grade (9 intact, 3 otherwise), not its
  parent hull.
- **No player verb**, and repair or rearm without a part change is not built (UD.4c is the
  player surface).

**The armed lease (CF.6c, `port/tv.rs`).** A rig wrecked outside its bay is written off: Occator
keeps the deposit, takes title where it fell, and bars that captain from armed leases for 28 IC
days. A damaged rig returned home gives back the full deposit with no damage check. A rig
disabled in the field cannot be returned at all (`RIG_OUT`).

**The field's own edges:**
- **Arrival (F-6).** Departure is built. Nothing in `port/voyage.rs` or `port/executor.rs` reads
  an open field on arrival, so a hull that settles during a field lands outside it. That breaks
  Q-CF-16, "nobody on the ground is outside the field".
- **Reinforcement (F-4).** Launches are refused while a field runs (`FIELD_LAUNCH`,
  `site_record.rs:999`, applied in `sortie/prepare.rs:547`).
- **The unarmed barge.** Any armed viewer holding a live Hostile contact in reach keeps the tempo
  engaged (`Arena::hostile_in_weapons_range`, `arena/state.rs:5554–5568`), unarmed targets
  included. The raider's barge never leaves (doc 32 §13).
- **Wreck cover (G-8).** Fire and sensor traces read the terrain alone
  (`mush_atlas::trace::trace_cells`). The pinned assert is in `tests/shard_1/field_story.rs`.
- **Carrier loss (F-3).** A hit is section damage only (`field/carrier.rs:8`), and a damaged
  drive does not hold a carrier on its pad. Space resolves hits through its own
  `SpaceShipState`, `ShipSectionId` and `SpacePartyId` (`space/bubble.rs`).
- **The cap.** `FIELD_MAX_UNITS` is 24 (`site_record.rs:57`), counting carriers but not casualties
  or wrecks. S.2 measured a 9-unit field, with a pulse median of 0.35 ms in debug. No IC field
  above 24 units has been measured.
- **Organization relations (F-1).** `friends()` groups parties by a shared owner only
  (`field/parties.rs`). No field code reads `mush_org` or `mush_faction`.
- **Faction effects (F-2).** Nothing writes reputation from a field.

**Orders.**
- **On open ground**, the NPC executor answers ENGAGE, SCREEN and SUPPORT with NEGATIVE
  (`sortie/npc.rs:234–241`).
- **Inside a field**, all three pass through: `field/orders.rs` maps SCREEN and SUPPORT, the
  adapter validates them, and the house doctrine handles them. No field test covers SCREEN or
  SUPPORT.
- **DT.4** (sub-leaders) is deferred; field orders come from the roster's leader only.

**The opposition.** `@field/raiders` is staff-only: seed, launch, withdraw
(`field/raiders.rs:308–354`). The raider's declaring mark has no production verb, and a test or
a staff member types it. There are two raiders in `content/field/raiders.json`. No doc plans a
generator.

**Licensed minds.** A hull under a licensed mind fails `custody::check_out`, and admission is
all-or-nothing (`field/admit.rs:350–361`, Q-CF-21). So one such hull on a site blocks every field
there until doc 23 S.4b.

**Crew.** Condition reads only the `head`, `cockpit` and `bridge` sections (`field/crew.rs`).
Extraction is a timed return to a known port (`extract_s` 21,600). There is no treatment, so an
injured or incapacitated character stays that way.

**Space salvage is live, and it is the only live salvage path.** `SpaceBook` holds a
`SalvageClaimBook` (`space/commands.rs:175`). `+helm/tow` records the attachment, the bay load
and custody all at once (`space/custody.rs`). The book is not serialized. The prior owner,
the base and the tenure are placeholders. The contest window is
`SALVAGE_CONTEST_WINDOW_IC_SECS = 48 × 3600` IC seconds (`space/docking.rs:43`), which is about
2 real hours at rate 24.

### 3a. Contradictions and stale text found

These go to the owner as findings, not questions. Each is folded into a question below where it
changes a default.

- **Space's contest window is IC hours**, the alternative doc 27 Q-S15 rejected. It also skips
  the tow stages (Q-S5) and the capacity check (Q-S8), and uses fabricated principals. (Q-36-46)
- **Where a salvage claim lives.** `mush-inventory/src/salvage.rs` says a claim is an E4b2
  contract with a contest deadline. Doc 27a's cut notes say it is never an `EconomicContract`.
  (Q-36-24)
- **Doc 27a is stale.** Its rows for live coupled towing, the W-clock quarantine (§7) and §8
  describe code that e3933521 deleted. Doc 27a and doc 31 call H3 "built", but `WorkshopBook` is
  an unstored, test-only library.
- **`BayCapacitySpec` is the parallel counter** Q-S8 forbids. (Q-36-15)
- **Who first drops remains.** Doc 32 §7 and `close.rs` say remains wait for "27a H3". Doc 27a
  §1.2 puts close-destroyed in H1-A. Q-CF-25(c) says an irrecoverable epoch closes destroyed,
  but the code keeps it open as a wreck. (Q-36-18)
- **Two black boxes.** One is the durable surface ledger (`surface/black_box.rs`); the other is
  the RAM-only tape in `SalvageClaimBook`, which is what the P-3 forensics read. (Q-36-28)
- **A written-off lease rig's title** goes to Occator (`tv.rs:140`). Whether Q-S10 and Q-R1 still
  let a third party salvage it is unruled. (Q-36-29)
- **Missing as its own state.** Doc 27 Q-H7 and §4.1 require jammed, damaged, destroyed and
  missing to stay distinct. The code has no Missing, and combat produces neither Jammed nor
  Damaged on its own. (Q-36-38)
- **`maintain/refit` is delegable** under Q-A1, but the built bay demands personal title.
  (Q-36-33)
- **Doc 32 §3 says the field-ops library has one live caller.** Space is a second.

## 4. Owner survey

**Ruled 2026-11-02.** The owner amended Q-36-8 (the carrier's tug), Q-36-11 (a leader may void
friendly-fire standing changes), Q-36-14 (anyone may recover, with no separate recovery step),
Q-36-29 (a written-off lease rig is Occator's alone) and Q-36-35 (minor repairs in a carrier bay
at double time). Every other default is accepted as written. Each amended question carries its
decision first, with the proposed text kept under it for the record.

Each question has a proposed default, an alternative and a reason. "Provisional" marks a number
to be tuned later. Questions marked **(expensive to reverse)** are ones whose answer shapes stored
data or the kernel's hashes.

### A. Order and acceptance

**Q-36-1 — Which player story comes first?**
- **Proposed default:** **the recovery run.** A captain sails back to a site after a field, finds
  the casualty, tows it with a second unit, loads it into the carrier and brings home the same
  damaged hull. A rival tries to take it, and the claim decides who keeps it. Then **the refit
  run**: the captain pays a workshop to replace the broken engine, rearms, and fields the hull
  again.
- **Alternative:** the refit run first. It needs no tow, because a damaged rig that came home
  under its own power is already in the bay.
- **Why:** doc 27b §7 and doc 27 order it H2 before H3. A recovered casualty is the case that
  makes H3 matter. A rig that limped home with section damage is a thinner first case. The refit
  run can still start early as a parallel lane (§7).

**Q-36-2 — Does the field's open edge work (§4 B) run before, beside or after H2?**
- **Proposed default:** beside H2, as small cuts. The barge exit (Q-36-6) and arrival (Q-36-3)
  come first, because a recovery run sails into a site where a field may be running.
- **Alternative:** after H2 and H3, as a batch.
- **Why:** a recovery captain arriving during a raid is the first thing the recovery story meets.
  Arrival is already a hole in Q-CF-16.

**Q-36-3 — The acceptance stories.**
- **Proposed default:** three stories, in the doc 32 form (production command paths, tacsim's
  world harness, then the bot over SSH):
  - **(a) the recovery run** above, including a restart while attached and again while loading;
  - **(b) the contested salvage**: three parties, namely the owner's recovery, a landholder and an
    opportunist, with the claim settled in the real 48-hour window, which a test compresses;
  - **(c) the refit run.**
- **Alternative:** one long story covering all three.
- **Why:** doc 32's 13-step story took several cuts to finish. Three shorter stories can each
  land with their own cut.

### B. The field's open edges

**Q-36-4 — F-6: a carrier arriving by voyage while its destination site has a field running.**
- **Proposed default:** it joins the field. At the next pulse it is admitted at its pad, as a unit
  of its own party (as `carrier::pad_poses` places carriers today). It may not launch while the
  field runs (Q-CF-17). Its arrival is a row in every party's presence picture.
- **Alternative:** a voyage cannot settle on a promoted site, and holds in orbit until the field
  demotes.
- **Why:** Q-CF-16 says everyone on the ground is in the field, and today an arrival breaks that.
  Holding in orbit lets one Hostile mark close a site to every arriving ship, which is the bay
  lock Q-CF-17's rejected reason already warned about.

**Q-36-5 — F-4: reinforcement.**
- **Proposed default:** keep Q-CF-17's refusal, but bound it. A field that has gone a provisional
  10 wall-clock minutes with no shot fired demotes, even with live Hostile contacts, so a declared
  but idle raider cannot hold bays shut.
- **Alternative:** launches from a carrier already on its pad enter at its ramp at the next pulse,
  counted against the cap.
- **Why:** Q-CF-17 was a deliberate owner amendment: the field runs at human speed and the world
  at 24×. A time bound answers the bay-lock objection without reopening it. Real reinforcement is
  better answered with F-6 arrival plus a launch rule designed for it.

**Q-36-6 — The unarmed barge keeping a field engaged.**
- **Proposed default:** two changes.
  - A target that has never been armed and is not firing does not keep `Engaged`. Its presence
    alone lets the quiet interval run.
  - A raider carrier whose hulls are all aboard lifts (CF.7's F-6 departure, applied to the NPC
    carrier).
- **Alternative:** only the barge lifting. The tempo rule stays.
- **Why:** today a player has to mark the raider Friendly to end a field, which is backwards. The
  tempo change is a kernel change with a rebless. The lift is a host change with none.
  **(expensive to reverse: the tempo half)**

**Q-36-7 — G-8: wreck cover.**
- **Proposed default:** a wreck's cell takes its class's wall cover and opacity in the fire and
  sensor traces, while the wreck stands in it. This is one kernel rule and one rebless.
- **Alternative:** lay a wreck as a site-record wall at demotion only, so it gives cover in the
  next field but not this one. No kernel change.
- **Why:** doc 32's story asks for it ("a second field uses the wreck as cover"). The alternative
  is cheap but strange: the wreck gives cover only after the field that made it has ended.
  **(expensive to reverse)**

**Q-36-8 — F-3: the carrier's loss story.**
- **Decision (owner ruling, 2026-11-02):** the default, with one correction. Since a unit tows
  up to twice its own tonnage (FO-13), a carrier wreck needs a tug of at least **half** its
  tonnage, not twice. No such unit exists yet, so in practice a carrier wreck still cannot be
  towed.
- **Proposed default (as written):** one consequence table, keyed on section state and shared with space.
  - A destroyed drive refuses lift.
  - A destroyed hull wrecks the carrier. Its crew are stranded and use extraction (Q-CF-39), and
    the rigs in its bay become wrecks where it stands.
  - A carrier wreck is a salvage object like any other, but needs a tug of 2× its tonnage, which
    in practice means none exists yet.
- **Alternative:** a carrier on the ground cannot be destroyed (its sections floor at disabled),
  and carrier loss is a space-only story.
- **Why:** Q-CF-19 made the carrier attackable and the owner reversed that default knowingly. A
  carrier that takes damage but never suffers the consequences is the half-built state today.
  **(expensive to reverse)**

**Q-36-9 — The unit cap.**
- **Proposed default:** measure an IC field at 48 and then at 100 units, from mixed parties, on
  the conclav.dev runner. Raise `FIELD_MAX_UNITS` to the largest measured size that keeps the
  pulse under its budget. Until then, 24 stays.
- **Alternative:** raise it to 100 now, on doc 16's 12v12 OOC data.
- **Why:** the target is 100+ units. The OOC lane is two parties; `hostile_in_weapons_range` and
  the relation table grow with parties × contacts, which the OOC data never exercised.

### C. Relations

**Q-36-10 — F-1: organization relations.**
- **Proposed default:** shared membership only. Two parties whose leaders share an organization
  are Friends, below a mark and fire-derived hostility in the order. There are no declared
  stances yet.
- **Alternative:** an organization-to-organization stance table (Allied, Hostile) as its own rung.
- **Why:** `mush-faction` says stances are "the future relationship matrix". Membership is
  already stored and needs no new authoring surface.

**Q-36-11 — F-2: faction effects of hostility.**
- **Decision (owner ruling, 2026-11-02):** the default, plus a void. A detachment's leader may
  void any standing change that fire by their own party caused, for friendly fire. The void is
  offered at demotion, alongside the change it cancels, and is recorded with it.
- **Proposed default (as written):** at demotion, each shot a party observed being fired at it adjusts the
  victim's faction standing of the shooter's principals by a bounded amount. A mark alone never
  does.
- **Alternative:** staff-only adjustments, read from the black box.
- **Why:** Q-CF-8 keeps hostility temporal to the field. Standing is the place a lasting
  consequence belongs. Fire is evidence; a private mark is not.

### D. H2: physical recovery

**Q-36-12 — The tow: a surface act or a field act?**
- **Proposed default:** a new surface act, `tow`, on the IC tick (doc 27 Q-R3), at the drive's
  cost scaled by `coupled_drive_rates_for_tow`. If the site promotes, the tug and its casualty are
  admitted coupled, and the tow runs on the kernel's wall time.
- **Alternative:** towing only inside a promoted field.
- **Why:** most recovery happens after the shooting stops, when the site is not promoted. Doc 27
  Q-R3 already put towing on the IC tick.

**Q-36-13 — Stage timings.**
- **Proposed default:** inspect, attach and load are timed acts (provisional knobs
  `[tacsim.recovery].inspect_s`, `attach_s`, `load_s`). Each is interruptible by promotion or
  release, and the tow is a drive.
- **Alternative:** attach is instant, and only loading takes time.
- **Why:** doc 27 Q-S5 rules separate timed stages. The knobs let a test compress them.

**Q-36-14 — Who may recover.**
- **Decision (owner ruling, 2026-11-02):** anybody, on any duty. On an ordinary operation, when a
  unit is disabled, someone drags it home as part of that operation; there is no separate second
  recovery sortie. Any mobile unit within the 2× tonnage cap may attach, tow and load, whatever
  its crew's duty. `surface.rs:55` stops refusing the recovery duty, and the tow, attach and load
  acts are open to mining and combat duties too. The recovery duty keeps its one effect (a
  Friend's recovery flight defers an extraction, CF.5) and its hire-terms meaning.
- **Proposed default (as written):** any DT unit on a `Recovery`-duty activation, or a rig its owner drives
  (the prospector included, within the 2× tonnage cap). `surface.rs:55` stops refusing the
  recovery duty for drive, attach and load. Recovery duty cannot mine or fire.
- **Alternative:** any operator on any duty.
- **Why:** the duty exists and today does almost nothing. Keeping recovery as a duty preserves the
  hire terms' meaning, and `CrewDuty::Recovery` is already in the crew offers.

**Q-36-15 — The bay's capacity basis (Q-S8).** **(expensive to reverse)**
- **Proposed default:** keep `maximum_items`, and add mass and envelope limits to the bay
  definition. Retire `BayCapacitySpec` as the forbidden parallel counter. A casualty loads only
  if it fits both.
- **Alternative:** item count only for the first cut, with the 2× tow cap as the only mass check.
- **Why:** a light carrier loading a heavy walker is exactly what Q-S8 exists to refuse. The
  definition is content, which is cheap now (no players, no migration).

**Q-36-16 — Loading into a bay the unit never launched from.**
- **Proposed default:** allowed into any free slot of the recovering carrier. The epoch closes
  `RecoveredToBase{carrier}` with `recovered_by`, and that carrier's operator gets custody and a
  claim (Q-S2), never title.
- **Alternative:** a unit may only return to its own bay until the entitlement work (§4 E) lands.
- **Why:** salvage of someone else's casualty is the point of H2. Closing the epoch at the bay is
  P-3's rule.

**Q-36-17 — Keeping the site index current.**
- **Proposed default:** a completed load, or a unit leaving the site under its own power, takes
  it out of the site index in the same commit (`take_unit`).
- **Alternative:** rebuild the index lazily at the next promotion.
- **Why:** the site record is per-observer knowledge's anchor. A recovered unit still listed on
  the site would mislead a second visitor.

**Q-36-18 — Irrecoverable hulls and remains.**
- **Proposed default:** H2 builds close-destroyed. An `Irrecoverable` hull's epoch closes
  `Destroyed` at demotion, and a `Remains` object with its mass replaces it in the site record
  (doc 27a §1.2 puts this in H1-A; doc 32 Q-CF-25(c) rules the close).
- **Alternative:** irrecoverable hulls stay indexed as open-epoch wrecks until H3.
- **Why:** the code contradicts Q-CF-25 today. A wreck and remains differ in what can be towed
  (a wreck can, remains cannot), so the recovery story needs the distinction.

**Q-36-19 — Crates and dropped cargo.**
- **Proposed default:** anyone may pick up cargo on open ground. On owned ground FO-4 applies.
  The manifest moves by inventory relocation (no new goods are made), into a hopper or a bay.
- **Alternative:** cargo salvage waits for H3.
- **Why:** nothing drops cargo yet except a staff fixture. If cargo is to drop, picking it up is
  the cheapest half of H2.

**Q-36-20 — What promotion does to a tow in progress (Q-S7).**
- **Proposed default:** the tug and casualty are admitted coupled. The tug may drive coupled at
  the tow rate, or release. A hit on either does not break the coupling; a tug wrecked or
  immobilized ends it.
- **Alternative:** promotion breaks the tow at its pose.
- **Why:** dragging a casualty out under fire is a real story, and doc 27 Q-S7 allows recovery in
  combat.

**Q-36-21 — Licence and compute seats on an inactive casualty (doc 27 Q-H9 leftover).**
- **Proposed default:** held until the epoch closes.
- **Alternative:** released when the hull is disabled.
- **Why:** releasing them makes a recovered hull return without the licence it left with.

**Q-36-22 — The leased rig left disabled in the field.**
- **Proposed default:** it can be returned once recovered to a bay. If it is still out after a
  provisional 28 IC days, it is written off on the same terms as a wreck.
- **Alternative:** it blocks the return forever, as `RIG_OUT` does today.
- **Why:** a lessee who cannot return a rig and cannot write it off is stuck with no way out.

**Q-36-23 — Scanning for salvage.**
- **Proposed default:** a surface scan reveals casualties, wrecks, remains and cargo within its
  reach, each written into the scanner's own `known` record (Q-CF-28).
- **Alternative:** salvage is known only from having been in the field, or from a delivered
  report.
- **Why:** a second visitor who never saw the field needs a way to find what lies there, and the
  scan already exists.

### E. H2: entitlement and evidence

**Q-36-24 — Where a salvage claim lives.** **(expensive to reverse)**
- **Proposed default:** an E4b2 economy contract with a `RealUtc` contest deadline. The contract
  engine already pauses for outages through `contract_live`, which covers Q-F3. Both ground and
  space use it (Q-36-46).
- **Alternative:** make `SalvageClaimBook` durable, with its own W-clock subscription.
- **Why:** one claim path for both tracks, on a store that already persists and already knows
  real time. The cut note saying "never an `EconomicContract`" predates E4b2.

**Q-36-25 — Who files and who is told.**
- **Proposed default:** the claim is filed at the site's port registry (the Freeport Registry
  for frontier sites). The title holder is told by IC mail at filing, and the landholder too on
  owned ground.
- **Alternative:** nobody is told; contesters must find out.
- **Why:** doc 27 Q-S4 forbids broadcasts, but a title holder being told about a claim on their
  own hull is notice, not a broadcast.

**Q-36-26 — Tenure continuity (Q-S3).**
- **Proposed default:** tenure survives a detach and reattach by the same tug, a tow interrupted
  by combat, and a restart with the attachment intact. A release, or a different tug, starts it
  again.
- **Alternative:** any detach starts it again.
- **Why:** doc 27 Q-S3 asks for this before coding. The default follows the physical hold.

**Q-36-27 — Contest outcome.**
- **Proposed default:** at the end of the window, an uncontested claim becomes title. A contested
  one goes to the registry's adjudication on the evidence filed (Q-36-28), using
  `adjudicate_claim`'s existing rules.
- **Alternative:** a contest always goes to staff.
- **Why:** the library already holds the adjudication rules, and staff time does not scale.

**Q-36-28 — Evidence sources.**
- **Proposed default:** a witness's own `known` and sensor records, the durable surface black box
  read with bay access (P-3), and delivered reports. The RAM-only tape in `SalvageClaimBook` is
  retired in favour of the durable black box.
- **Alternative:** the black box only.
- **Why:** two black boxes will disagree. The durable one is the one that survives a restart.

**Q-36-29 — A written-off lease rig.**
- **Decision (owner ruling, 2026-11-02):** the alternative. A written-off lease rig is Occator's
  alone and cannot be claimed by anyone else. Lease units are a special case of FO-1 and Q-R1:
  otherwise players could cooperate to destroy leased rigs and claim them. Anyone may still tow
  one physically (Q-36-14), for example to clear a ramp, but it gains no claim, and loading it
  into a bay returns it to Occator, not to the carrier's operator.
- **Proposed default (as written):** Occator holds title. A third party may still salvage it under Q-R1 and
  claim it. Occator may contest like any title holder, and sends no recovery of its own in this
  phase.
- **Alternative:** a written-off rig is Occator's alone and cannot be claimed.
- **Why:** FO-1 says whoever leaves a unit has lost it. A company that writes off a rig and never
  recovers it has left it.

**Q-36-30 — Crime.**
- **Proposed default:** unlawful recovery is recorded as evidence only. The claim fails at
  contest, and the black box and witnesses can be used in a later crime system. No enforcement
  verb in this phase.
- **Alternative:** a minimal bounty or standing hit for a proven unlawful recovery.
- **Why:** there is no crime system in code. Doc 27 Q-H2's core-crime rules are policy only.

### F. H3: restoration

**Q-36-31 — The cost basis.**
- **Proposed default:** quotes from `WorkshopFacilityTariff`, with `industrial_parts` consumed per
  structural point and parts drawn from inventory lots. The facility's operator may set a
  multiplier, but not a free fee.
- **Alternative:** the operator sets the fee freely, as UD.4b does now.
- **Why:** D-31-13 says the bay quotes. A tariff gives players a price they can predict and tests
  a price they can assert.

**Q-36-32 — Time and labour.**
- **Proposed default:** work units on the IC clock, which pause under a W-clock freeze. The hull
  is held in the bay for the duration. The facility fee covers labour; there is no technician
  NPC yet.
- **Alternative:** a hired technician (a new crew duty) as a scarce input.
- **Why:** UD.4b already runs one work unit per IC hour. A labour market is a later economy cut.

**Q-36-33 — Who may repair, refit, strip and scrap.**
- **Proposed default:**
  - the title holder, or anyone granted `MaintainRefit`, may repair, rearm and refit;
  - a lessee may repair and rearm a leased hull, but not refit, strip or scrap it;
  - a salvor with custody may stabilize and repair at their own risk (Q-S14), but not strip or
    scrap (Q-F5).
  
  The bay's authority check reads the grant.
- **Alternative:** the title holder only, as the bay does now.
- **Why:** doc 27 Q-A1 made `maintain/refit` delegable. Org members and lessees cannot use the bay
  today.

**Q-36-34 — Damaged leased hulls on return.**
- **Proposed default:** at return, assess the damage and take the tariff's restoration cost from
  the deposit, up to the deposit. Occator repairs the rig before it is leased again.
- **Alternative:** no charge, and the next lessee gets the damage, as today.
- **Why:** a free return makes the lease a free repair, and the next lessee gets a broken rig.

**Q-36-35 — Field repair and which facilities count.**
- **Decision (owner ruling, 2026-11-02):** a carrier's bay counts for **minor** repairs at double
  the workshop's time: replacing armor, and repairing a damaged section or component that is not
  destroyed. A destroyed component or section, or any significant repair (reconstruction, part
  replacement, refit), needs a dedicated facility. The field-engineering boundary of doc 27a §10
  stays as it is for units out on the ground.
- **Proposed default (as written):** keep doc 27a §10's boundary. In the field, a tender may unjam, stabilize
  and rearm. Destroyed parts need a facility, meaning a port workshop: Stickney Yard and the
  Ceres ports' workshops, authored as rooms with the bay capability. A carrier's bay is not a
  facility in this phase.
- **Alternative:** a carrier bay counts as a facility at a provisional 2× time.
- **Why:** a carrier that can fully repair in the field removes the reason to come home.

**Q-36-36 — Ammunition.**
- **Proposed default:** a munitions good per ammunition family, with a mass and price, sold at
  port vendors. Rearm draws lots, and a carrier can carry munitions as cargo for field rearm.
- **Alternative:** credits per round (the tariff's 10).
- **Why:** doc 15 and the weapons direction make ammunition precious. Precious things have to be
  goods, otherwise there is nothing to carry, run short of or steal.

**Q-36-37 — Stripped-part provenance.**
- **Proposed default:** a stripped part's lot carries its parent hull, its condition and its claim
  reference (Q-F6). Parts stripped without authority are flagged hot, and legitimate workshops
  refuse to install them (doc 27a §9).
- **Alternative:** lots with a grade only, as today.
- **Why:** Q-F6 is ruled. The bay's returned lot already carries a lineage; this widens it.

**Q-36-38 — Jammed, damaged and missing.** **(expensive to reverse)**
- **Proposed default:**
  - Add `Missing` as a component condition, distinct from Destroyed, for stripped and removed
    parts.
  - Combat gains a component-level outcome: a hit that does not destroy a section may jam or
    damage one component in it, at a provisional chance in the content.
- **Alternative:** H3 handles only destroyed sections and spent ammunition, and combat stays as
  it is.
- **Why:** FO-2 and doc 31's bay assume jammed and damaged parts exist. Without them, the minimal
  workshop of Q-S12 has half its orders and nothing to apply them to. The combat half reaches the
  kernel and its hashes.

**Q-36-39 — Scrap and remains.**
- **Proposed default:** scrapping turns structural mass into `structural_alloys` and salvageable
  parts into lots. Remains of an irrecoverable hull (Q-36-18) can be scrapped where they lie by
  their titled owner or a claimant who has won the contest. Nothing can be stripped from remains.
- **Alternative:** remains are inert scenery forever.
- **Why:** Q-CF-25 left remains holding "a manifest of nothing" until H3 ruled. Scrap value gives
  an opportunist a lawful reason to visit.

**Q-36-40 — The repair verb.**
- **Proposed default:** a new authoritative action for changes that touch only cells and
  conditions (repair, rearm), beside `bay_refit` for part changes. Both go through UD.4c's player
  surface (`+bay`).
- **Alternative:** extend `component_refit` to cover repair.
- **Why:** UD.4b's cut notes already say repair and rearm without a part change "get their own
  action".

### G. Opposition, orders and command

**Q-36-41 — A real opposing force.**
- **Proposed default:** a raid director on the surface tick. It seeds, launches and marks
  authored raiders by a per-site schedule and threat budget, triggered by presence and ore
  value, with a cooldown. Raiders arrive by F-6 arrival and leave by Q-36-6's lift. Their losses
  are real: their wrecks stay, and a raider group whose carrier is wrecked does not respawn.
- **Alternative:** staff-run and Storyteller-run raids only, with a production
  `@field/raiders/mark` verb.
- **Why:** the multi-party field needs someone to be hostile without staff present. The pieces it
  needs (arrival, departure, doctrine) are being built in this phase anyway.

**Q-36-42 — SCREEN and SUPPORT on open ground.**
- **Proposed default:** keep NEGATIVE on open ground, and document that both bind once a field
  opens. Add the missing field gate tests for SCREEN and SUPPORT.
- **Alternative:** on the surface, SUPPORT follows its charge's rig as a drive leg and SCREEN
  holds facing a bearing. Neither fires.
- **Why:** they are combat formations. On the surface nobody is fighting, and the doctrine that
  executes them runs only in a field.

**Q-36-43 — DT.4, sub-leaders.**
- **Proposed default:** stays deferred. Larger fights are multiple rosters made Friends by
  Q-36-10's shared membership.
- **Alternative:** one level of attachments per roster, for the 12v12 target.
- **Why:** doc 26 deferred it until play shows the need. The cap is still 24.

### H. Minds and crew

**Q-36-44 — A licensed-mind hull on a site before doc 23 S.4b.**
- **Proposed default:** leave it out of the field as an inert obstacle (an object in the site's
  walls), with its claim untouched. The rest of the site's parties are admitted.
- **Alternative:** refuse launching licensed-mind hulls onto frontier sites at all.
- **Why:** today one such hull blocks every field on its site, because admission is all or
  nothing. Leaving it out is the smallest fix that lets the field run.

**Q-36-45 — Crew treatment.**
- **Proposed default:** an injured or incapacitated character recovers to fit at a port after
  a provisional IC time, faster at a port with a medical facility. No skill checks.
- **Alternative:** ejection first: a forced extraction with a chance of capture by a Hostile
  party.
- **Why:** an incapacitated character has no way back to fit today. Capture reopens Q-CF-38's
  ruling-out.

### I. The seam with space (doc 35)

**Q-36-46 — One salvage path for ground and space.** **(expensive to reverse)**
- **Proposed default:** space's `+helm/tow` moves onto the same claim (Q-36-24), the same
  48-real-hour window, the same stages (attach, tow, load as separate acts) and the same capacity
  check. The fabricated principals are replaced with real ones. The space track's owner does it,
  in their tree, after Q-36-24 lands.
- **Alternative:** two paths, and space keeps its instant tow and IC-hour window.
- **Why:** a hull lost in space and one lost on the ground are the same legal object. Doc 35
  D-35-12 already says space follows doc 27a's 48 h; the code just does not.

**Q-36-47 — One consequence table for carrier and ship damage.**
- **Proposed default:** Q-36-8's table lives in one module both tracks read. Space's
  `ShipSectionId` maps onto it.
- **Alternative:** each track keeps its own.
- **Why:** a carrier is the same hull in a ground field and in a space bubble. Doc 35 Q-35-1 asked
  for kernel reuse.

**Q-36-48 — Party identity across the seam.**
- **Proposed default:** `SpacePartyId` becomes `PartyId`, so F-1 and F-2 (Q-36-10, Q-36-11) feed
  both tracks from one rule.
- **Alternative:** a mapping at the seam.
- **Why:** two party types will grow two relation rules.

### J. Housekeeping found on the way

**Q-36-49 — The field story's stand-in.**
- **Proposed default:** a tacsim harness hook for the org unit grant, so the doc 32 story (and
  this survey's stories) fly delivered orders end to end.
- **Alternative:** move the stories to the split-boundary harness.
- **Why:** today the escort's orders are proven in a separate harness from the story they belong
  to.

**Q-36-50 — The two latent world-task items (doc 32 §17).**
- **Proposed default:**
  - Gate the `DecisionResult` arm on no pending commit (`mush-server/src/world_task.rs`, around
    l.3729).
  - Give lines queued behind a held lifecycle slot a "waiting" notice after 250 ms.
  - Each change gets its own regression test.
- **Alternative:** leave both as documented invariants.
- **Why:** neither has been seen to fail. Both are cheap now and expensive to diagnose later.

**Q-36-51 — Doc 27a.**
- **Proposed default:** mark the rows describing deleted code as superseded, with a pointer to
  e3933521. The next design doc (§7) becomes the source for H2 and H3.
- **Alternative:** rewrite 27a.
- **Why:** 27a is a catalogue of a library, and half of that library is now gone.

## 4a. Consequences of the amended rulings

- **Q-36-14 changes the recovery story.** The recovery run in §6 becomes part of an ordinary
  operation: the escort's own party drags its casualty to the ramp and loads it before leaving,
  under fire or after. A return visit to a site to recover what was left behind is the
  second-visit variant, not the main path. Q-36-16 (loading into a foreign bay) and the
  contested salvage (§6 b) are unchanged, because they turn on who holds the casualty, not on
  duty.
- **Q-36-14 and Q-36-20 together:** a tow in progress when a site promotes is admitted coupled.
  An ordinary escort can therefore drag its wingman to the ramp during the fight.
- **Q-36-29 also settles Q-36-22's write-off.** A leased rig left disabled for 28 IC days is
  written off on the wreck's terms, which now means Occator's alone. A lessee who recovers their
  own disabled lease rig to a bay before then can return it (Q-36-22) and pays the damage charge
  (Q-36-34).
- **Q-36-29 and the claim contract (Q-36-24).** Filing a claim on a hull whose title is
  Occator's write-off is refused at filing. This is a lease rule, not a title rule: other
  companies' titles are claimable as Q-R1 says.
- **Q-36-35 splits the repair actions (Q-36-40) by facility class.** The quote names the class
  each action needs: a carrier bay or a workshop for armor and damaged parts, a workshop alone
  for destroyed parts, reconstruction and refit. A carrier bay's quote doubles the time, not the
  cost.
- **Q-36-11's void needs a record.** The standing change and its void are written together at
  demotion. The default's bounded amount is provisional, and so is the window to void: until
  the leader's next command after demotion, or a provisional 10 IC minutes.

## 5. Out of scope for this survey

- **The 10 km ground** (doc 16 D-16-5); its own programme.
- **Infantry and battle armor** as units (doc 27 FO-6).
- **Prisoners, interrogation and capture** (Q-CF-38).
- **A mission or contract generator** beyond the raid director (Q-36-41).
- **Persistent known-hostile memory** (doc 32 F-7), which needs the Epistemic Engine.
- **Facility assaults and territorial warfare.**
- **Space-only work**, except the three seams in §4 I.

## 6. Acceptance sketch

The three stories of Q-36-3, in the form doc 32 used: production command paths through tacsim's
world harness, then the bot over SSH for the parts a player types.

- **(a) The recovery run** (as amended by Q-36-14). During and after a field like doc 32's, the
  escort's wingman drags the disabled escort to the carrier's ramp at the tow rate and loads it
  before the carrier lifts. A second variant: a casualty left behind is found on a later visit
  by a scan, attached, towed and loaded by any unit of the returning party.
  - A restart while attached keeps the attachment.
  - A restart while loading keeps the load.
  - The hull comes home with every section as it was. The site record no longer lists it.
- **(b) The contested salvage.** Three parties on Urvara Reach:
  - an opportunist recovers someone else's casualty and files a claim;
  - the title holder is told and contests with evidence from their black box;
  - the landholder appears as the third party.
  
  The 48-real-hour window is compressed for the test and paused across a W-clock outage.
  Adjudication gives the hull to the right party.
- **(c) The refit run.** The recovered casualty is inspected in Stickney Yard's workshop. Its
  captain is quoted, pays from the tariff, waits the IC time, and gets the engine replaced, a
  jammed weapon unjammed and the magazines rearmed from bought munitions. The removed engine
  becomes a lot with its parent hull's provenance. The captain then fields the hull again.

## 7. Cuts (GC.0–GC.13)

Planned 2026-11-02 from the rulings. Like doc 32's CF cuts, each one runs through the production
command paths, ships its own gate, and is recorded as built in a section of its own after this
one. A cut's line must name the files it touches, and two cuts that touch the same files never
run at the same time. Cut names are GC ("ground combat"); everything else in this section is
provisional and the owner's to reorder.

**Two rules for every cut:**
- **One kernel rebless.** Every change to the kernel's hashes in this programme lands in GC.2,
  together, so the practice hashes are reblessed once (as Q-CF-3 did).
- **Stories grow.** Each story in §6 is one test file. The cut that first makes a step pass adds
  that step, and the story's last cut runs it whole, at 256 and at 512 (Ahuna Flank and Urvara
  Reach).

### Lane 1: the field's edges

**GC.0 — Housekeeping and the barge's exit.** Q-36-6 (the host half), Q-36-17, Q-36-42, Q-36-49,
Q-36-50, Q-36-51.
- **The raider's carrier lifts** once every hull it launched is aboard or is a casualty or wreck
  (CF.7's F-6 departure, run by the host for the NPC carrier). Files: `field/raiders.rs`,
  `field/exit.rs`.
- **The site index stays current.** A unit that leaves the site under its own power, through a
  DT recover or a rig's recover, is taken out of the index (`take_unit`) in the same commit.
  Files: `surface/site_record.rs`, `detachment/sortie/prepare.rs`.
- **SCREEN and SUPPORT gates** in a field, and `help field` saying they bind only in a field.
  Files: `tests/shard_1/`, `docs/help/field.md`.
- **The harness hook** for the org unit grant, so a tacsim world-harness story can land a DT
  activation and fly delivered orders end to end. Files: tacsim's world harness.
- **The two world-task items:** the `DecisionResult` arm gated on no pending commit, and a waiting
  notice for lines queued behind a held lifecycle slot after 250 ms, each with its regression.
  Files: `mush-server/src/world_task.rs`.
- **Doc 27a** rows for deleted code marked superseded.
- **Gate:** `gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark`, which is
  doc 32's story with the stand-down mark removed.
- **Depends on:** nothing. Starts first.

**GC.1 — Arrival, and licensed minds left out.** Q-36-4, Q-36-45.
- **An arriving carrier joins a running field** at the next pulse, at its pad, as a unit of its
  own party, without launches. Files: `port/voyage.rs` or `port/executor.rs` (the arrival),
  `field/carrier.rs`, `field/admit.rs`.
- **A licensed-mind hull is left out** of a field as a wall object, and every other party is
  admitted. Files: `field/admit.rs`.
- **Gate:** `gc1_a_carrier_arriving_mid_field_is_admitted_at_its_pad_and_cannot_launch`, plus
  one test for the licensed-mind case.
- **Depends on:** GC.0, because both touch the voyage's field hooks.

**GC.2 — The kernel batch (the one rebless).** Q-36-5, Q-36-6 (the tempo half), Q-36-7,
Q-36-38 (combat's half).
- **The tempo:** a target that has never been armed and is not firing does not keep `Engaged`.
  A field with no shot for a provisional 10 wall-clock minutes demotes regardless
  (`[tacsim.field].idle_demote_s`).
- **Wreck cover (G-8):** a wreck's cell takes its class's wall cover and opacity in the fire and
  sensor traces while the wreck stands in it. The CF.7 story's G-8 assert flips.
- **Component outcomes:** a hit that does not destroy a section may jam or damage one component in
  it, by provisional content chances. `ComponentCondition` gains `Missing`, distinct from
  `Destroyed`, and `"stripped"` maps to it.
- Files: `arena/state.rs`, `arena/clock.rs`, `arena/damage.rs`, the trace in `mush-atlas` or
  `arena/`, `mush-tactics/src/field_ops/condition.rs`, and the snapshot and hash pins. Reblessed
  once.
- **Gates:** `gc2_an_unarmed_barge_does_not_hold_a_field_engaged`,
  `gc2_a_wreck_hides_what_stands_behind_it`,
  `gc2_a_survivable_hit_can_jam_a_weapon_and_the_check_in_keeps_it`, and
  `gc2_an_idle_declared_field_demotes`.
- **Depends on:** nothing. Starts beside GC.0. It is the largest cut; split it into GC.2a (tempo
  and cover) and GC.2b (components) if the rebless allows two.

**GC.3 — Remains, and the carrier's loss.** Q-36-8, Q-36-18, Q-36-47.
- **Close-destroyed:** an irrecoverable hull's epoch closes `Destroyed` at demotion, and a
  `Remains` object with its mass replaces it in the site record. Files: `field/close.rs`,
  `surface/site_record.rs`.
- **One consequence table** for carrier and ship damage, in one module that both tracks read:
  - a destroyed drive refuses lift;
  - a destroyed hull wrecks the carrier, its crew are stranded, and its bay rigs become wrecks;
  - a carrier wreck needs a tug of at least half its tonnage.
  
  Files: a new `tacsim::carrier_damage` module (name provisional), `field/carrier.rs`, and the
  lift and voyage gates. Space reads the table in its own tree, as Q-36-47's follow-up.
- **Gates:** `gc3_an_irrecoverable_hull_leaves_remains_and_closes_destroyed` and
  `gc3_a_carrier_with_its_drive_destroyed_cannot_lift`.
- **Depends on:** GC.2, because the component outcomes feed the table, and GC.1, because both
  touch `field/carrier.rs` and the voyage gates.

### Lane 2: H2 recovery

**GC.4 — The tow (story a).** Q-36-12, Q-36-13, Q-36-14, Q-36-15, Q-36-20, Q-36-21.
- **`tow`, attach and release** as surface acts on the IC tick, open to every duty. Any mobile
  unit within the 2× tonnage cap may do them, at `coupled_drive_rates_for_tow`. The acts are
  timed by `[tacsim.recovery].inspect_s`, `attach_s` and `load_s`, and each is interruptible.
  The live acts use the library's `RecoveryBook` rules.
- **A tow in a promoted field** is admitted coupled, and a coupled tug drives at the tow rate on
  wall time.
- **Loading into one's own carrier's bay**, checked against the bay's new mass and envelope limits
  as well as `maximum_items`. `BayCapacitySpec` is retired.
- Licence and compute seats stay with the casualty until its epoch closes.
- Files: `surface/` (a new `tow.rs`), `detachment/sortie/surface.rs` (the duty refusal goes),
  `definitions/logistics.rs` (the bay limits), `port/voyage.rs` (the bay gate counts mass),
  `field/admit.rs` (coupled admission), `field/carrier.rs`, and the content bays.
- **Story (a), main path:** the escort's wingman drags the disabled escort to the ramp during the
  field and loads it before the carrier lifts. A restart while attached keeps the attachment; a
  restart while loading keeps the load. The bot's escort pilot learns the tow lines.
- **Depends on:** GC.0 (the site index), GC.1 (both touch `field/admit.rs`) and GC.3 (both touch
  `field/carrier.rs` and the voyage's bay gate).

**GC.5 — Salvage on a later visit.** Q-36-16, Q-36-19, Q-36-22, Q-36-23, Q-36-29.
- **A scan reveals salvage** (casualties, wrecks, remains and cargo in reach) into the scanner's
  own `known` record.
- **Loading into a foreign bay:** the epoch closes `RecoveredToBase{carrier}` with `recovered_by`,
  and the carrier's operator gets custody and a pending claim, which GC.6 files.
- **Cargo pickup** by inventory relocation, on open ground for anyone and on owned ground by
  FO-4.
- **Lease rigs.**
  - A disabled lease rig recovered to a bay can be returned.
  - One still out after 28 IC days is written off and becomes Occator's alone.
  - A written-off rig can be towed, but loading it hands it back to Occator, with no claim.
- Files: the scan in `surface/`, `surface/tow.rs`, `port/tv.rs`, and `mush-inventory`'s relocate
  path.
- **Story (a), second variant:** a casualty left behind is found by a scan on a later visit and
  brought home by the returning party.
- **Depends on:** GC.4.

**GC.6 — The claim (story b).** Q-36-24 to Q-36-28, Q-36-30.
- **A salvage claim is an E4b2 economy contract** with a `RealUtc` 48-hour contest deadline,
  paused by outages through `contract_live`. It is filed at the site's port registry (the
  Freeport Registry for frontier sites), and refused at filing for an Occator write-off.
- **Notice** goes by IC mail to the title holder, and to the landholder on owned ground.
- **Tenure** is kept across the same tug's detach and reattach, an interruption by combat, and a
  restart with the attachment intact.
- **Evidence:** a party's own `known` and sensor records, the durable surface black box read
  with bay access, and delivered reports. The RAM tape in `SalvageClaimBook` is retired for the
  ground.
- **Adjudication** at the deadline by `adjudicate_claim`'s rules: an uncontested claim becomes
  title. Unlawful recovery stays evidence only.
- Files: `mush-economy` (the contract kind), `mush-tactics/src/field_ops/salvage.rs` (made live
  and durable), a new `tacsim::salvage` host (name provisional), the mail and registry hooks.
- **Story (b):** three parties on Urvara Reach. The window is compressed by a test knob and paused
  across a W-clock outage.
- **Depends on:** GC.5.
- **Afterwards, in the space track's tree:** `+helm/tow` moves onto this claim, its window, its
  stages and the bay check, and `SpacePartyId` becomes `PartyId` (Q-36-46, Q-36-48). This is the
  space track's cut, not this programme's, and it is listed here so the two tracks agree on the
  order.

### Lane 3: H3 restoration

**GC.7 — Munitions and the tariff (content and economy).** Q-36-31, Q-36-36.
- **A munitions good** per ammunition family, with mass and price, sold at the Ceres ports and
  Stickney Yard. A carrier can carry munitions as cargo.
- **The workshop tariff** becomes content: per point, per section, per component, per round, in
  IC time. Operators may set a multiplier, not a free fee. `industrial_parts` are consumed per
  structural point.
- Files: `content/goods.json`, the ammunition packs, the vendor content, a tariff content file, and
  `mush-tactics/src/field_ops/workshop.rs`'s `WorkshopFacilityTariff` read from content.
- **Gate:** `gc7_munitions_are_bought_and_carried_and_the_tariff_quotes_a_known_repair`.
- **Depends on:** nothing. Starts beside GC.0 and GC.2.

**GC.8 — The repair actions and `+bay` (story c).** Q-36-32, Q-36-33, Q-36-35, Q-36-37,
Q-36-39, Q-36-40.
- **A new authoritative action** for changes that touch only cells and conditions (repair a
  section, repair a component, unjam, rearm from munitions lots), beside `bay_refit` for part
  changes. Work units run on the IC clock and pause under a W-clock freeze, and the hull is held.
- **Facility classes:**
  - a carrier bay does minor repair (armor, and damaged parts that are not destroyed) at 2× time;
  - a workshop does everything, including destroyed parts, reconstruction, replacement and refit;
  - out on the ground, field engineering stays as doc 27a §10 has it.
- **Authority:** the title holder or a holder of a `MaintainRefit` grant may repair, rearm and
  refit. A lessee may repair and rearm. A salvor with custody may stabilize and repair at their
  own risk. The bay's authority check reads the grant.
- **Provenance:** a stripped or removed part's lot carries its parent hull, its condition and its
  claim reference. Hot parts are refused.
- **Scrap:** structural mass becomes `structural_alloys`, and salvageable parts become lots.
  Remains can be scrapped where they lie by their title holder or a winning claimant.
- **UD.4c's `+bay`** player surface over all of it: the doll, the quote, the order.
- Files: `mush-industry/src/bay_refit/` (the authority), a new repair action beside it,
  `mush-tactics/src/field_ops/workshop.rs` (made live), `hardware/bay.rs`, the `+bay` commands,
  and `mush-inventory` lots.
- **Story (c):** the casualty from story (a), inspected at Stickney Yard. It gets a new engine, a
  jammed weapon unjammed and its magazines rearmed from bought munitions, then fields again. A
  second unit is armor-patched in its carrier's bay at double time.
- **Depends on:** GC.2 (for `Missing` and jammed parts) and GC.7 (for munitions and the tariff).
  It does not need GC.4: a damaged rig that drove home is enough to start.

**GC.9 — The lease's damage charge.** Q-36-34.
- At return, the damage is assessed, and the tariff's restoration cost is taken from the deposit,
  up to the deposit. Occator restores the rig before it is leased again.
- Files: `port/tv.rs`.
- **Gate:** `gc9_a_damaged_lease_rig_returned_pays_its_restoration_from_the_deposit`.
- **Depends on:** GC.7 (the tariff). GC.5 also touches `port/tv.rs`, so the two do not run at
  once.

### Lane 4: around the field

**GC.10 — Relations (F-1, F-2).** Q-36-10, Q-36-11.
- **Shared organization membership** makes two parties Friends, below a mark and fire in the
  order.
- **At demotion,** observed fire adjusts the victim faction's standing of the shooter's
  principals by a bounded, provisional amount. The detachment's leader may void the changes their
  own party caused, for friendly fire, until their next command or a provisional 10 IC minutes.
  The change and the void are recorded together.
- Files: `field/parties.rs`, `field/close.rs` (at demotion), and `mush-faction`'s reputation write
  path.
- **Gates:** `gc10_two_rosters_in_one_organization_are_friends` and
  `gc10_a_leader_voids_friendly_fire_standing`.
- **Depends on:** GC.3, because both touch `field/close.rs`.

**GC.11 — The raid director.** Q-36-41.
- **A raid director on the surface tick** seeds, launches and marks authored raiders by a per-site
  schedule and threat budget, triggered by presence and ore value, with a cooldown. Raiders
  arrive by GC.1's arrival and leave by GC.0's lift. A raider group whose carrier is wrecked does
  not respawn.
- Files: `field/raiders.rs`, a new `surface/raid.rs` (name provisional), and
  `content/field/raiders.json`.
- **Gate:** `gc11_a_scheduled_raid_arrives_declares_and_leaves_with_its_losses`. The bot's
  captain story runs once with a director raid instead of a staff one.
- **Depends on:** GC.0 and GC.1.

**GC.12 — The cap, measured.** Q-36-9.
- A scale harness fields 48 and then 100 IC units from mixed parties on conclav.dev, and reports
  pulse time, snapshot bytes and relation-table cost. `FIELD_MAX_UNITS` rises to the largest
  measured size within budget, and the result is recorded in this doc.
- Files: a scale test, and `site_record.rs`'s constant at the end.
- **Depends on:** nothing. It can run any time, and is better after GC.2 because the tempo rule
  changes the hot loop.

**GC.13 — Crew treatment.** Q-36-44.
- An injured or incapacitated character recovers to fit at a port after a provisional IC time,
  faster at a port with a medical room.
- Files: `field/crew.rs`, and port content.
- **Gate:** `gc13_an_extracted_incapacitated_pilot_is_fit_again_after_treatment`.
- **Depends on:** nothing.

### Lanes at a glance

| Starts at once | Then | Then | Then |
|---|---|---|---|
| GC.0 housekeeping, barge lift | GC.1 arrival | GC.11 raid director | |
| GC.2 kernel batch (one rebless) | GC.3 remains, carrier loss (after GC.1 too) | GC.4 tow (story a) → GC.5 later visit → GC.6 claim (story b) | GC.10 relations, beside GC.4 |
| GC.7 munitions, tariff | GC.8 repair, `+bay` (story c), after GC.2 | GC.9 lease charge | |
| GC.12 the cap; GC.13 treatment | | | |

At most four writers run at once: one per row, with the files kept apart as each cut's line
names them. The three stories close the programme. Then a short review, in the form of the
2026-10-31 one, before the next survey.
