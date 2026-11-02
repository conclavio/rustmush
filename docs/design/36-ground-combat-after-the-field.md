# 36. Ground combat after the field — recovery, repair and the field's open edges

Draft 0.4, 2026-11-02, docs only. Surveyed against `origin/main` at `d5ba6b2c`. Draft 0.1, the
same day, was the survey: it asked the owner questions, each with a proposed default and an
alternative, in the form docs 27 §7 and 32 §4 used. **Draft 0.2 records the owner's rulings of
2026-11-02 on every question:** Q-36-8, Q-36-11, Q-36-14, Q-36-29 and Q-36-35 amended, every
other default accepted as written. §4a lists what the amendments change elsewhere. **Draft
0.3, the same day,** replaces §7's provisional order with the cut list GC.0–GC.13, planned from
the rulings. **Draft 0.4, the same day,** adds §4b: six questions a pre-implementation review
found open (Q-36-52..57). Q-36-52 is the owner's ruling; the rest are defaults taken under the
2026-10-24 rule, and the cut lines in §7 carry them.

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

## 4b. Found open before implementation (Draft 0.4)

A review of Draft 0.3 against `main` at `66725b6a` on 2026-11-02 found six things the cuts
would have had to decide on their own. Each is in the form of §4. Q-36-52 is the owner's ruling;
Q-36-53 to Q-36-57 are defaults taken under the owner's rule of 2026-10-24 (reasonable defaults,
tuned later, the expensive-to-reverse ones flagged), and the owner may amend any of them.

**Q-36-52 — A crew member still aboard a casualty that a non-friend attaches to.**
- **Decision (owner ruling, 2026-11-02):** the non-friend may attach, tow and load regardless.
  Until capture is built (Q-CF-38 rules it out of this phase), the moment the casualty's load
  into a non-friend's carrier completes, every character still aboard is treated as abandoned:
  the NPC extraction of Q-CF-39 starts for them on the spot, as if they had requested it, and
  after the configured IC delay they are at the nearest known safe port (FO-12, P-5). The hull
  stays with the carrier. A Friend's recovery-flight deferral (CF.5) does not apply to this
  extraction. While the tow is in progress the crew stay aboard the hull and may request
  extraction themselves, as today.
- **The friendly half (a default):** a completed load into a carrier of the crew's own party or
  of a Friend sets their location "aboard the carrier" (Q-CF-38) and cancels any pending
  extraction.
- **Alternative:** attach refused while anyone is aboard and not yet extracted.
- **Why:** refusing the attach would let a stranded pilot hold a hull on the ground for as long
  as they liked, which is the bay lock of Q-CF-17 in another form. Abandonment and extraction
  is the survival route that already exists, and it leaves nothing for capture to undo later.

**Q-36-53 — From the carrier's bay to the workshop.**
- **Decision (Draft 0.4 default):** a workshop is a port room authored with the bay capability
  (`bay_refit`, doc 31 UD.4b) and a provisional berth count. A hull moves from a docked
  carrier's bay slot into the workshop by a `berth` act and back by `stow` (names provisional,
  under UD.4c's `+bay`): the hull's location becomes the room, its bay slot frees, and the
  carrier may leave without it. The bay's authority check stays as built (the hull and the
  requester in the facility). Stickney Yard and the Ceres ports each get one workshop room in
  content.
- **Alternative:** the workshop reads a hull in a docked carrier's bay in place.
- **Why:** the built bay demands one location (`bay_refit/authority.rs:143`), and a hull under
  repair is physically out of its carrier. Today no port has an authored workshop room and
  nothing moves a hull out of a bay slot, so story (c) cannot start without this.

**Q-36-54 — The bay's envelope (Q-36-15).**
- **Decision (Draft 0.4 default):** the envelope is the hull's stowage volume, on the existing
  `si.volume_l` measure of the bay's compartment (doc 06 D3 measure maps, doc 18 §4.3). Each
  unit definition gets a `stowage_volume_l`, authored where known and otherwise derived from dry
  mass at a provisional class density. "Fits both" (Q-36-15) means the bay's `si.mass_kg` and
  `si.volume_l` limits, through the existing compartment owners (Q-S8). No size class.
- **Alternative:** a size class (light, medium, heavy) on hulls and bays.
- **Why:** the measures already exist on the bay side, a volume is a number doc 31's designer
  can derive from a design, and it touches no kernel hash. The expensive-to-reverse mark on
  Q-36-15 is answered by reusing a measure rather than adding one.

**Q-36-55 — The raid director's withdrawal, and the raiders' faction.**
- **Decision (Draft 0.4 default):** two parts.
  - **Withdrawal.** A raider group orders its hulls home up the ramp when it has lost a
    provisional half of the hulls it launched, when a provisional 20 wall-clock minutes have
    passed in a promoted field, or when no armed non-friend has been in its reach for the idle
    interval of Q-36-5. The barge then lifts by Q-36-6. `@field/raiders/withdraw` stays as the
    staff override.
  - **Faction.** Each raider entry in `raiders.json` names an organization, registered in
    `mush-faction` as the group's principal, so F-2 (Q-36-11) writes standing for raider fire
    and for fire at raiders. Two raider organizations in content, one per entry, provisional.
- **Alternative:** raiders are factionless and F-2 skips them; withdrawal only when the
  director's threat budget is spent.
- **Why:** today a raider's leader is a bare NPC name, so Q-36-11 has nothing to write against,
  and Q-36-6's lift needs someone to order the hulls aboard first.

**Q-36-56 — GC.2's rebless.**
- **Decision (Draft 0.4 default):** one cut, one rebless. Draft 0.3's sentence allowing a split
  into GC.2a and GC.2b is withdrawn.
- **Alternative:** two cuts and two reblesses.
- **Why:** the rule at the head of §7 exists so the practice hashes flip once in the programme.
  A larger PR is cheaper than a second rebless.

**Q-36-57 — Housekeeping in §7.**
- **Decision:** GC.1 cites Q-36-44 (the licensed-mind hull) and GC.13 cites Q-36-45 (treatment);
  Draft 0.3 had them swapped. GC.1 depends on GC.0 for the harness hook its gate flies through,
  not by files. Five cuts start at once, not four: GC.12 and GC.13 are single-file cuts outside
  the field and surface files.

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

**GC.1 — Arrival, and licensed minds left out.** Q-36-4, Q-36-44.
- **An arriving carrier joins a running field** at the next pulse, at its pad, as a unit of its
  own party, without launches. Files: `port/voyage.rs` or `port/executor.rs` (the arrival),
  `field/carrier.rs`, `field/admit.rs`.
- **A licensed-mind hull is left out** of a field as a wall object, and every other party is
  admitted. Files: `field/admit.rs`.
- **Gate:** `gc1_a_carrier_arriving_mid_field_is_admitted_at_its_pad_and_cannot_launch`, plus
  one test for the licensed-mind case.
- **Depends on:** GC.0, for the harness hook (Q-36-49) its gate flies through. No files
  overlap (Q-36-57).
- **Split (owner ruling, 2026-11-01): GC.1 and GC.1b.** The kernel has no way to add a unit to
  a running arena, and `arena/` is GC.2's while GC.2 runs. GC.1 ships the licensed-mind half
  and arrival's host side, with `arena/` untouched. **GC.1b** comes after GC.2 merges and is
  stacked on it: `ArenaMsg::Join` and `Arena::join`, factored out of `Arena::new_with_minds`,
  and the check-out of an owed arrival. It carries the gate above, plus a test pinning that the
  OOC practice hashes do not move (no OOC match ever joins). §9 has what GC.1 built.

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
- **Depends on:** nothing. Starts beside GC.0. It is the largest cut, and it stays one cut with
  one rebless (Q-36-56).

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
  touch `field/carrier.rs` and the voyage gates. Since the split (§9), it waits on GC.1b, which
  brings the arrival itself, not on GC.1.

### Lane 2: H2 recovery

**GC.4 — The tow (story a).** Q-36-12, Q-36-13, Q-36-14, Q-36-15, Q-36-20, Q-36-21,
Q-36-52 (the friendly half), Q-36-54.
- **`tow`, attach and release** as surface acts on the IC tick, open to every duty. Any mobile
  unit within the 2× tonnage cap may do them, at `coupled_drive_rates_for_tow`. The acts are
  timed by `[tacsim.recovery].inspect_s`, `attach_s` and `load_s`, and each is interruptible.
  The live acts use the library's `RecoveryBook` rules.
- **A tow in a promoted field** is admitted coupled, and a coupled tug drives at the tow rate on
  wall time.
- **Loading into one's own carrier's bay**, checked against the bay's `si.mass_kg` and
  `si.volume_l` limits as well as `maximum_items`; each unit definition gains a
  `stowage_volume_l` (Q-36-54). `BayCapacitySpec` is retired. A completed load into a carrier of
  the crew's own party or of a Friend sets the crew aboard the carrier and cancels a pending
  extraction (Q-36-52).
- Licence and compute seats stay with the casualty until its epoch closes.
- Files: `surface/` (a new `tow.rs`), `detachment/sortie/surface.rs` (the duty refusal goes),
  `definitions/logistics.rs` (the bay limits and `stowage_volume_l`), `port/voyage.rs` (the bay
  gate counts mass), `field/admit.rs` (coupled admission), `field/carrier.rs`, `field/crew.rs`
  (the friendly load), the content bays and the unit definitions.
- **Story (a), main path:** the escort's wingman drags the disabled escort to the ramp during the
  field and loads it before the carrier lifts. A restart while attached keeps the attachment; a
  restart while loading keeps the load. The bot's escort pilot learns the tow lines.
- **Depends on:** GC.0 (the site index), GC.1 (both touch `field/admit.rs`) and GC.3 (both touch
  `field/carrier.rs` and the voyage's bay gate).

**GC.5 — Salvage on a later visit.** Q-36-16, Q-36-19, Q-36-22, Q-36-23, Q-36-29, Q-36-52
(the non-friend half).
- **A scan reveals salvage** (casualties, wrecks, remains and cargo in reach) into the scanner's
  own `known` record.
- **Loading into a foreign bay:** the epoch closes `RecoveredToBase{carrier}` with `recovered_by`,
  and the carrier's operator gets custody and a pending claim, which GC.6 files. Any character
  still aboard is treated as abandoned when the load completes: the Q-CF-39 extraction starts
  for them on the spot, with no recovery-flight deferral (Q-36-52).
- **Cargo pickup** by inventory relocation, on open ground for anyone and on owned ground by
  FO-4.
- **Lease rigs.**
  - A disabled lease rig recovered to a bay can be returned.
  - One still out after 28 IC days is written off and becomes Occator's alone.
  - A written-off rig can be towed, but loading it hands it back to Occator, with no claim.
- Files: the scan in `surface/`, `surface/tow.rs`, `port/tv.rs`, `field/crew.rs` (the abandoned
  crew), and `mush-inventory`'s relocate path.
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

**GC.7 — Munitions and the tariff (content and economy).** Q-36-31, Q-36-36, Q-36-53 (the
rooms).
- **A munitions good** per ammunition family, with mass and price, sold at the Ceres ports and
  Stickney Yard. A carrier can carry munitions as cargo.
- **The workshop tariff** becomes content: per point, per section, per component, per round, in
  IC time. Operators may set a multiplier, not a free fee. `industrial_parts` are consumed per
  structural point.
- **Workshop rooms** at Stickney Yard and the Ceres ports, authored with the `bay_refit`
  capability and a provisional berth count (Q-36-53).
- Files: `content/goods.json`, the ammunition packs, the vendor content, a tariff content file,
  the port room content, and `mush-tactics/src/field_ops/workshop.rs`'s `WorkshopFacilityTariff`
  read from content.
- **Gate:** `gc7_munitions_are_bought_and_carried_and_the_tariff_quotes_a_known_repair`.
- **Depends on:** nothing. Starts beside GC.0 and GC.2.

**GC.8 — The repair actions and `+bay` (story c).** Q-36-32, Q-36-33, Q-36-35, Q-36-37,
Q-36-39, Q-36-40, Q-36-53 (the acts).
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
- **`berth` and `stow`** (names provisional): a hull moves from a docked carrier's bay slot into
  the workshop room and back; its location becomes the room, its slot frees, and the carrier may
  leave without it (Q-36-53).
- **UD.4c's `+bay`** player surface over all of it: the doll, the quote, the order, berth and
  stow.
- Files: `mush-industry/src/bay_refit/` (the authority), a new repair action beside it,
  `mush-tactics/src/field_ops/workshop.rs` (made live), `hardware/bay.rs`, the `+bay` commands,
  the berth and stow acts in `port/`, and `mush-inventory` lots.
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

**GC.11 — The raid director.** Q-36-41, Q-36-55.
- **A raid director on the surface tick** seeds, launches and marks authored raiders by a per-site
  schedule and threat budget, triggered by presence and ore value, with a cooldown. Raiders
  arrive by GC.1's arrival and leave by GC.0's lift. A raider group whose carrier is wrecked does
  not respawn.
- **Withdrawal** (Q-36-55): the group orders its hulls up the ramp after losing half of what it
  launched, after 20 wall-clock minutes in a promoted field, or after the idle interval with no
  armed non-friend in reach; all three numbers provisional.
- **Raider organizations** (Q-36-55): each entry in `raiders.json` names one, registered in
  `mush-faction`, so GC.10's standing writes have a principal on the raider side.
- Files: `field/raiders.rs`, a new `surface/raid.rs` (name provisional),
  `content/field/raiders.json`, and the raider organizations in `mush-faction`'s registry
  content.
- **Gate:** `gc11_a_scheduled_raid_arrives_declares_and_leaves_with_its_losses`. The bot's
  captain story runs once with a director raid instead of a staff one.
- **Depends on:** GC.0 and GC.1. Since the split (§9), it waits on GC.1b: a raider arrives by
  the kernel's join.

**GC.12 — The cap, measured.** Q-36-9.
- A scale harness fields 48 and then 100 IC units from mixed parties on conclav.dev, and reports
  pulse time, snapshot bytes and relation-table cost. `FIELD_MAX_UNITS` rises to the largest
  measured size within budget, and the result is recorded in this doc.
- Files: a scale test, and `site_record.rs`'s constant at the end.
- **Depends on:** nothing. It can run any time, and is better after GC.2 because the tempo rule
  changes the hot loop.

**GC.13 — Crew treatment.** Q-36-45.
- An injured or incapacitated character recovers to fit at a port after a provisional IC time,
  faster at a port with a medical room.
- Files: `field/crew.rs`, and port content.
- **Gate:** `gc13_an_extracted_incapacitated_pilot_is_fit_again_after_treatment`.
- **Depends on:** nothing.

### Lanes at a glance

| Starts at once | Then | Then | Then |
|---|---|---|---|
| GC.0 housekeeping, barge lift | GC.1 licensed minds, arrival's host side → GC.1b the join (after GC.2) | GC.11 raid director | |
| GC.2 kernel batch (one rebless) | GC.3 remains, carrier loss (after GC.1b too) | GC.4 tow (story a) → GC.5 later visit → GC.6 claim (story b) | GC.10 relations, beside GC.4 |
| GC.7 munitions, tariff | GC.8 repair, `+bay` (story c), after GC.2 | GC.9 lease charge | |
| GC.12 the cap; GC.13 treatment | | | |

At most five writers run at once: one per row, GC.12 and GC.13 beside them as single-file cuts
(Q-36-57), with the files kept apart as each cut's line names them. The three stories close the
programme. Then a short review, in the form of the 2026-10-31 one, before the next survey.

## 8. GC.0 as built (housekeeping and the barge's exit)

Built on a branch off `main` at `9877284f`, beside GC.1 and GC.2. It covers Q-36-6 (the host
half), Q-36-17, Q-36-42, Q-36-49, Q-36-50 and Q-36-51. No kernel hash moved. Where the rulings
left something open, this is what the cut decided, for the owner to object to:

- **The raider's carrier lifts** (Q-36-6, the host half; `field/raiders.rs`, `field/exit.rs`).
  Once every hull the barge launched into the field is aboard again or is a casualty or a wreck,
  the host asks the field to let the barge go with no ramp. That is doc 32 F-6's departure,
  through `field::exit` with a new continuation, `Then::RaiderLift`, asked as the raider's
  leader. Out, the barge leaves the site's ground (`raiders::lift`), and the site hears "The
  Skulk Barge lifts from Ahuna Flank with its hulls aboard, leaving what it lost where it fell."
  - *What "a casualty or a wreck" means.* It is what demotion would write for the hull now. The
    demotion's rule is factored out of `close::demote_units` (`close::condition_now`) and read
    off the last snapshot the host holds. A hull still fighting therefore holds its barge on the
    ground. An immobile or unarmed hull counts as a casualty, as it does on the site record.
  - *When it is checked* (`raiders::lift_check`): after each hull boards, and at the field's
    snapshots, paced to once each `raiders::LIFT_CHECK_MS` (10 s of arena time, a provisional
    default; `raiders::lift_check_paced`). A field holding no raider's carrier decodes nothing.
    A field holding one decodes its snapshot at that pace while the carrier stands.
  - *Why it is paced* (the fix after GC.0 merged). As first built, the check ran at every
    snapshot. The decode costs about 300 ms of the world task at 512 a side on conclav.dev, and
    the harness snapshots every 500 ms, so the world task spent most of the fight decoding.
    `s2_the_field_story_at_512` went red on main's hosted CI: its world fell behind its field,
    the escort was hit past mission-kill before the story's stand-down landed, and the field
    ended before the walker's head fell. Pinned to three cores on conclav.dev, main failed it
    three times in four, at both of CI's assert points; the paced check passed four in four.
  - *Not lifted:* a carrier that launched no hull into this field (none admitted, none named on
    the open promotion's exits), and a carrier that is itself a wreck. What a damaged drive does
    to lift is still F-3's (Q-36-8, GC.3).
  - *Coming back.* `@field/raiders/launch` lands a lifted barge at its authored cell again before
    it launches, so a raider's second visit needs no reseed. Seeding stays refused while any hull
    of the raider exists, aboard or not.
  - *Files beyond the cut's line:* `field/home.rs` and `field/mod.rs` gained one line each (the
    new continuation's arm, and the snapshot hook), and `field/close.rs` shares its condition
    rule.
- **The tempo half stays GC.2's.** The gate demotes without it. Once the barge is gone, the
  raider's party has only the walker's wreck in the field, and a wreck's contact reads dead
  (`Arena::hostile_in_weapons_range`). So nothing keeps the field engaged, and it settles on its
  quiet interval with the escort still holding the raider Hostile.
- **The site index stays current** (Q-36-17). A unit that leaves the site under its own power is
  taken off the record's unit index (`SiteRecord::take_unit`, through
  `site_record::left_under_own_power`) in the commit that starts its recover. That is the rig
  recover's start in `surface/act/providers.rs` (the surface act provider already holds the site
  records family) and the DT recover in `detachment/sortie/prepare.rs`, which writes the record
  beside its flight rows. A tow or a load is H2's (GC.4), not this cut's.
- **SCREEN and SUPPORT in a field** (Q-36-42). On open ground both stay NEGATIVE, as they were.
  `help field` now says they bind only once a field opens, and that support for a unit outside
  the same field and party is NEGATIVE there too. The gates are in-source in `field/orders.rs`:
  SCREEN reaches the kernel as a screen facing its bearing, and SUPPORT binds only to another
  unit of the recipient's own party in the same field. *Not built:* a shard_1 story that flies
  them as delivered orders. No tacsim test yet runs a DT roster to its field, and the hook below
  is what such a story needs. GC.1's gate is the first to fly through it.
- **The harness hook** (Q-36-49). The Org unit grant failed in tacsim's world harness because its
  `land()` prepared on the plain path, which hands a provider no owner reads. Org's
  `unitgrant_issue` reads its owner preimage, so it refused ("unit-grant request refused"). The
  composition was never the problem. `driving::land_composed` (in `tacsim/tests/shard_1/`)
  lands a submission on the world task's composed path
  (`prepare_registered_composed_resolved_authoritative_action`, then `install_composed_action`),
  as shard_4's excursion lander does, and keeps the module writes for a reboot. Object writes,
  such as a DT launch's, stay in the World. A harness story can now land the grants a DT
  activation needs. mush-server's `dt_preparation_plans` (`pair_field_fixture`) gives the command
  sequence to mirror. Its scheduled `control_handoff_apply` needs the composed scheduled lander,
  as shard_4's `m8_world` has it.
- **The two world-task items** (Q-36-50; `mush-server/src/world_task.rs`).
  - *The decision arm.* A provider's answer (`Wake::DecisionResult`) is taken only while no
    commit is pending (`decision_result_armed`), as the command-submission and arena-egress arms
    already were. Until the receipt lands, it waits in its bounded channel.
  - *Behind the lifecycle slot.* While a lifecycle message waits in the slot behind a pending
    commit, `rx` is still taken, into a bounded queue behind it (`BehindSlot`, at most
    `COMMAND_DEFER_CAP` entries; past that `rx` stops, as before). The slot runs first, then that
    queue front to back, each entry handled as a fresh message, then `rx`. A typed line that has
    waited 250 ms behind the slot hears, once: "The world is finishing a connection's business
    first; your line is waiting and runs as soon as it can." It is not refused. A shutdown or the
    loop's end refuses what is queued there, as it refuses the defer queue.
  - *Regressions:* `gc0_a_decision_result_waits_behind_a_pending_commit` tests the arm's rule as
    a unit, since no rig holds a commit while a provider answers.
    `gc0_a_line_behind_a_held_lifecycle_slot_hears_it_waits_then_runs` holds a receipt, parks a
    `ConnDown` in the slot and types a line behind it: the notice arrives, then the line runs
    after the receipt.
- **Doc 27a** (Q-36-51). It gains a status note at its head. Every row and section describing
  code that `e3933521` deleted (§3, §6, §7's quarantine and §8) is marked superseded, with a
  pointer to the commit, to `61a670ce` where the code can be recovered, and to this document.

**The gate** (`tacsim/tests/shard_1/field_story.rs`,
`gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark`) is doc 32's field
story at 256 a side, formerly `cf7_the_field_story`, with the escort pilot's Friendly stand-down
removed. The walker falls to the escort's coaxial, and the tank withdraws up the ramp and boards.
Then the barge lifts, the open promotion names its exit, and the field demotes with the raider
still marked Hostile. The barge is not on the site record. The story's second field lands the
barge again. `s2_the_field_story_at_512` runs the same script on Urvara Reach. The other gates:

- the hook: `escort_lease::gc0_the_harness_lands_the_org_unit_grant` (the loan is refused on the
  plain prepare and lands on the composed one);
- the site index: `site_record::gc0_a_unit_leaving_under_its_own_power_leaves_the_index`;
- SCREEN and SUPPORT: `field::orders::gc0_a_screen_in_a_field_holds_its_bearing` and
  `gc0_a_support_in_a_field_binds_to_its_partys_unit_only`.

**Defaults taken (provisional):**

- the snapshot cadence as the lift's check;
- the barge landing again at its authored cell on the next launch;
- the waiting notice's wording, and its 250 ms (the defer queue's bound);
- the behind-slot queue's cap (the defer queue's 64).

## As built (GC.7)

GC.7 landed on 2026-11-01 as content and economy (Q-36-31, Q-36-36, Q-36-53's rooms). It touches
no kernel hash. It does change content pins: the goods table goes to v4, the lane to v18 and the
relay seed to v9. Every number below is provisional under the owner's economy rule of 2026-10-24.
The expensive-to-reverse choices are marked.

**Content files.**
- `crates/tacsim/content/market/goods.json` (v4): four munitions goods of family `munitions`.
  Each row has an optional `munitions` link (`{ammunition, rounds_per_unit}`) naming the
  ammunition entry it loads, read by `GoodsTable::munitions_for`. The link goes from the good to
  the ammunition, so no ammunition pack or digest changes.
- `crates/tacsim/content/market/workshop_tariff.json` (v1): the tariff, loaded by
  `tacsim::definitions::workshop::workshop_tariff`.
- `crates/tacsim/content/lane/ceres_phobos.json` (v18): the munitions stock and asks, and both
  ports' `workshop` blocks.
- `crates/tacsim/content/relays/luna_ceres.json` (v9): re-signed to require lane v18. No entry
  changed.

**The goods** (stowage `stow.bulk`, so any hold carries them as ordinary cargo):

| Good | Loads | Unit | kg / L per unit | Price | Basis |
|---|---|---|---|---|---|
| `ammo_ap_30` | `ap_30` | round | 1 / 1 | 12 cr | a 240-round scout-car load is 2,880 cr |
| `ammo_ball_762` | `ball_762` | box of 100 | 3 / 3 | 25 cr | a single round weighs less than the table's 1 kg unit; the 2,000-round belt is 500 cr |
| `ammo_he_105` | `he_105` | round | 21 / 30 | 150 cr | a 40-round tank load is 6,000 cr, about 6% of the tank's 100,000 cr |
| `ammo_missile` | `missile_round` | round | 50 / 100 | 1,500 cr | a six-round rack is 9,000 cr |

Who sells them: the Co-op's chandlery at Ceres (`coop`, board `chandlery`) and Stickney Yards
(`stickney_yards`, board `yard`), at the reference prices above. Each holds 4,800 / 400 / 800 /
120 units of stock. **Beam charge stays a cell and is not a good.** The laser's magazine is
deliberately larger than any match spends, because heat is the laser's limit (16b cut X.1). A
workshop recharges beam charge at the tariff's per-round labour.

**The tariff** (`WorkshopFacilityTariff`, costs in centicredits, times in IC seconds). The basis
is one IC hour of bay labour at 100 cr, the core-fitting fee:

| Work | Cost | Time |
|---|---|---|
| per structural point | 1 cr | 36 s (100 points an IC hour) |
| `industrial_parts` | 1 t per 100 points, drawn from lots, not priced in the quote | — |
| repair a component | 200 cr | 2 IC h |
| replace a component, or a bay refit | 400 cr | 4 IC h (`BAY_WORK_UNITS`) |
| rebuild a section shell | 800 cr plus its points | 8 IC h |
| load a round | 0.25 cr, rounds brought as goods | 30 IC min a rearm |

An operator may scale costs by a multiplier between 0.5x and 2x and cannot set a free fee.
`with_multiplier_permille` rounds each cost up and never changes durations. The `Default`
impl is gone, so the engine carries no numbers of its own. The quote now also returns
`structural_points` and `industrial_parts_t`. **The bay's quote path:** `BayRefitConfig`
carries the tariff, and `offer_bay_refit`'s selector replaces `fee` with
`multiplier_permille`. The fee is the replacement labour at that multiplier, refused above the
bay's `max_fee_minor`. At 2x a refit is 800 cr, under the bay's 1,000 cr ceiling.

**The workshop rooms** (Q-36-53). Each port entry has a `workshop` block (`PortDef.workshop`,
`WorkshopDef`). The `PortApplier`'s new part j opens the room itself as an industry facility of
the operator org, holding `bay_refit`, with the berths as its slots. Part j is idempotent, and
any drift is refused in preflight. `tacsim::port::workshop::quote` gives the tariff at the
workshop's multiplier.
- **Ceres:** room C14, the Dockmaster's Office (on the transfer junction, behind FREIGHT, BERTH
  HOLDERS), operated by `occator`. 2 berths at 1.2x.
- **Stickney:** room P11, the Service Counter (the drydock window and the refit queue),
  operated by `stickney_yards`. 4 berths at 1x.

**(expensive to reverse)** The rooms are existing rooms, not new atlas rooms. A new room would
mean a neighborhood version bump and its requirement chain. GC.8's `berth` puts hulls in these
rooms, so moving a workshop to a dedicated room later means re-homing berthed hulls. If a
dedicated room is wanted, add it before GC.8 ships. The authority check also needs the room
titled to the operator's scope and the bay's payment site composed over it. Both are left to
GC.8, which makes the bay live there.

**Gate:** `gc7_munitions_are_bought_and_carried_and_the_tariff_quotes_a_known_repair`
(`crates/tacsim/tests/shard_2/voyage.rs`). The test steps:
1. A chartered captain buys 40 rounds of `ammo_he_105` at the Co-op chandlery for 6,000 cr. They
   land in the Dray's hold.
2. Both workshop rooms are their operators' `bay_refit` facilities with 2 and 4 berths.
3. A line tank that is 50 points down on its hull, 40 points down on its left track and 10
   rounds short in the cannon gets two quotes. At Ceres the quote is 111.00 cr, 90 points, 1 t
   of parts and 5,040 IC s. At Stickney it is 92.50 cr in the same time.

The bay rigs (`ud4b_*`) assert the tariff-derived fee: 2,500 minor in the industry rig, and
40,000 minor in `hardware/flow/bay.rs` on the shipped tariff.

**Pins that moved with the content** (CI round 1). These were fixed as pins, not as logic:
- the goods count, now 16, in the content studio, its split-gateway round trip and the catalog
  rows;
- every hold gauge's load cells, which list the four munitions at 0;
- the goods-name refusal;
- the readiness commerce lines, now 60;
- the stock receipts, 16 instead of 8;
- the C08 board, which now shows the munitions asks;
- the older-lane upgrade plans, where Stickney's port is now an update too;
- the site sentence, which now names the workshop block;
- the site-map web fixture, re-blessed because the extra stocking fires start it 72 IC s later.

The bounds that grew with the clerk's longer queue have their reasons in the tests: the
plumbing re-post at 16 fires, and the mill batch after an apply that also adds the munitions
rows at 24 fires. The cost-lane census now credits a fire's `economy:meta` op to the economy
sweep exactly when the sweep's stamp moved in that fire. Any other write the restock adds
still fails the check. No version moved beyond goods v4, lane v18 and relay v9.

## 9. GC.1 as built (licensed minds left out; arrival's host side)

Built on GC.0's branch (`claude/gc0-housekeeping-barge-exit`, PR #295), beside GC.2 and GC.7,
and rebased onto `main` once #295 and GC.7's #294 merged. It covers Q-36-44 and the host half
of Q-36-4. No kernel file changed, and no hash moved.

**The split** (owner ruling, 2026-11-01, "Yes, GC.1b"). Q-36-4's ruling stands: an arriving
carrier joins the field at the next pulse, at its pad, as a unit of its own party. Building it
found that the kernel cannot add a unit to a running arena. `Arena::new_with_minds` is the only
constructor of a unit row (its seal and cards, sensors, squads, epochs, its party's book), and
no `ArenaMsg` admits one. That code is `arena/`, which GC.2 holds while it runs. So the cut is
split:

- **GC.1** (this section): the licensed-mind half, and arrival's host side, which spots an
  arrival and holds it owed.
- **GC.1b**, after GC.2 merges and stacked on it: `ArenaMsg::Join` with `Arena::join`, factored
  out of `new_with_minds`, and the host's check-out of each owed arrival at its pad pose
  (`carrier::pad_poses`), sent as a Join at the next pulse. The arrival also becomes a row in
  every party's presence picture. Its gate is
  `gc1_a_carrier_arriving_mid_field_is_admitted_at_its_pad_and_cannot_launch`. A second test
  pins that the OOC practice hashes do not move, since no OOC match ever joins. GC.3 and GC.11
  wait on GC.1b, not on GC.1 (§7).

**A licensed-mind hull is left out** (Q-36-44; `field/admit.rs`).
- *What counts:* a hull held by dedicated IC custody (`UnitStore::is_ic_custodied`), which is
  a licensed mind's hold until doc 23 S.4b. `custody::check_out` refuses it, and admission was
  all or nothing (Q-CF-21), so one such hull closed every field on its site.
- *What happens:* admission now sets such a hull apart before the cap, the parties and the
  check-out. Every other unit on the ground is admitted as before. The hull is neither counted
  nor lettered nor checked out.
- *How it becomes a wall:* its cell is laid on the field's ground as a one-cell object
  (`with_held_walls`), which `field::ground` reads as a wall, so nobody drives through it or
  sees past it. Only the copy of the record the ground is built from gains the object. The
  stored site record is never written, and the hull's custody and claim are untouched.
- *What the site hears:* "The {name} stands out of field {n}: its licensed mind cannot be taken
  into a field yet, so it stands as ground nobody crosses."
- *Why it is no arrival:* the same rule keeps it out of the owed arrivals below. A field never
  takes it in.

**Arrival's host side** (Q-36-4; `field/carrier.rs`, one hook in `field/admit.rs`'s tick, one
field on `FieldBook` in `field/mod.rs`).
- *What is spotted:* each second, for a site with a field running, the tick takes the carriers
  on the site's ground that are not in the field, not deployed and not held out
  (`carrier::arrivals`).
- *What is held:* it keeps them in the host's RAM book as owed (`FieldBook::arrivals`, read
  through `carrier::owed_arrivals`). Each newly seen one is logged. The set empties once the
  field is gone.
- *Not checked out here:* a check-out with no Join would leave the hull marked deployed with
  nothing to check it in, since the close checks in only the field's own rows. The check-out
  moves to GC.1b, beside the Join.
- *Already refused:* a launch from the arrived carrier while the site is promoted, by the
  existing Q-CF-17 refusal (`FIELD_LAUNCH` and the surface act provider's promoted-site gate).
  No new code was needed.
- *Changed in no way:* nothing in `port/voyage.rs` or `port/executor.rs`. The voyage settles
  the hull exactly as before, and the field side notices it.

**The gates.**
- `field::admit::tests::gc1_a_licensed_mind_hull_is_left_out_of_the_field_as_a_wall`
  (in-source): the held hull's cell becomes a one-cell object on the copy, numbered after the
  record's own, and the kept record gains nothing. *Not covered:* an admission run with a held
  hull on the ground. No tacsim rig turns on the four-owner IC custody, which is the only
  source of a real `open_ic_mind` row, so the partition rests on this test and on review.
- `field_carrier::gc1_a_carrier_arriving_mid_field_is_owed_its_admission_and_cannot_launch`
  (shard_1): a second Dray-P under way when CF.3's field opens on Ahuna Flank settles mid-field.
  The host owes it its admission, it is not in the field, the field runs on, and its rig's
  launch is refused. The test helper `driving::en_route` is `on_pad_as` stopped at the launch.

**Defaults taken (provisional):**

- the held-out hull's one-cell footprint, and the site's line for it;
- the owed arrivals as RAM only, rebuilt by the tick after a restart.

## 10. GC.2 as built (the kernel batch)

Built 2026-11-01 on `main` at `9877284f`, beside GC.0 and GC.7, as the programme's one kernel
rebless (§7's first rule, Q-36-56). Four changes, one cut. What the cut had to decide where the
rulings left it open, for the owner to object to:

- **The tempo (Q-36-6, the tempo half).** `Arena::hostile_in_weapons_range`
  (`arena/state.rs`) counts a held Hostile contact only when its hull **came into the field
  armed** (`UnitState::armed_at_checkout`, a weapon row at check-out) **or has fired a shot**.
  "Never armed" is read at check-out, not now: a hull that was armed and has lost or spent its
  guns still keeps a field engaged, as it did. The test reads truth the presence picture already
  names (the hull's class), so the tempo leaks nothing a pilot could not see.
- **The idle bound (Q-36-5).** `[tacsim.field].idle_demote_s`, 600 by default, banded 10 to
  3,600 like `disengage_s` (`config.rs`, `mush-config`, `config_ext.rs`). The arena clock keeps
  the last shot's millisecond (`Clock::last_shot_ms`, `arena/clock.rs`), carried in a snapshot's
  `ClockState` (serde default, absent before the first shot), so a resumed field's bound counts
  from the shot it really saw. `Task::disengaged` (`arena_task.rs`) demotes on either interval,
  under the same "no round in flight" rule; `FieldTerms` carries both (`field/admit.rs`,
  `field/boot.rs`). Arena time is the measure, which a field runs at the wall's pace.
- **Wreck cover (Q-36-7, G-8).** A new module, `arena/wreck_cover.rs`. A wreck's cell reads as
  `TerrainClass::Wall` — the class CF.4 lays a site record's objects as — in both traces while
  the wreck stands in it. The fire trace (`Arena::trace_eye_to`, `arena/damage.rs`: the shot, the
  impact's re-trace and the cockpit's cover advice) walks the terrain through an overlay grid;
  the sensor trace (`Arena::sight`, `arena/perception.rs`) keeps the terrain's horizon index and
  then asks whether a wreck stands on the line's interior. Neither end of a line is ever its own
  cover, so a shot at a wreck reads the ground under it. A field with no wreck pays nothing. The
  trace stayed in `arena/`, not `mush-atlas`: the atlas has no notion of a unit.
- **Component outcomes (Q-36-38, combat's half).** A round that reaches a section's internals
  and leaves the section standing takes one more draw, `component:<shot_seq>` (the seventh draw
  label, pinned in `arena/rng.rs` and `mush-contract-tests`' `arena_library`). Below the unit's
  `jam_permille` it jams a part of the section, below `jam_permille + damage_permille` it damages
  one, else nothing; the same value, divided by a thousand, picks the part in component-id order
  among those the outcome can change (intact or damaged for a jam, intact for a damage). The op
  rides the hit's one plan, and the draw is the hit line's third. The chances are content:
  `UnitSpec::component_hits` (`definitions/unit.rs`), absent in every shipped unit, so no
  `DefinitionRef` moves, and `COMPONENT_HITS` — **50 and 100 per mille, provisional** — when a
  unit names none. They reach the kernel on `UnitProfile::component_hits`
  (`arena/resolution.rs`, `match_/arena_def.rs`); a hand-built profile has none, so no fixture
  jams. A round that stops in the armour touches no part.
- **What a jam does.** A jammed gun leaves the weapon table as a shot-off turret's guns do (its
  rounds kept for the check-in, its slots cleared; `Arena::drop_weapons`, factored out of
  `remove_weapons`). A jammed or damaged part of any other kind changes nothing in the match yet.
  At check-out a gun whose part the record reads jammed, destroyed or missing does not come into
  the match (`part_works`, `arena/state.rs`), so a jam a field left stays a jam until the bay
  clears it. A damaged gun still fires.
- **The check-in keeps it.** Nothing new was needed: the IC condition plan
  (`field_ops::apply_ic_combat_return`) already carried an explicit component condition, and
  `effective_component_condition` keeps a stored `jammed`. A hull whose only gun jammed reads
  `Unarmed`, which the demotion lists as a casualty on the site record.
- **`Missing` (Q-36-38).** `ComponentCondition::Missing` (`mush-tactics`'
  `field_ops/condition.rs`), after `Destroyed` in the order; `"missing"`, `"stripped"` and
  `"removed"` classify to it. It neither works nor salvages, it survives whatever its section
  suffers, and the assessment lists it apart (`FieldUnitAssessment::missing_components`). The
  workshop's own `"stripped"` word is unchanged; field engineering refuses a missing part as it
  refuses a destroyed one.

**Not done, and why.** The unit-mind API's `line_to` (`stock_mind::line_to`, the `line_to(v, id)`
kernel) still reads the terrain alone: a mind's view holds no wreck positions, and giving it
them is a mind API change with its own seal digests, which this cut's rebless did not take. A
mind may therefore believe it has a line through a wreck, fire, and find the wreck at the impact.
That is a follow-up for whoever next bumps the mind API.

**The gates.**
- `gc2_an_unarmed_barge_does_not_hold_a_field_engaged` and `gc2_an_idle_declared_field_demotes`
  (`tests/shard_1/field_gc2.rs`): through the production command paths on Ahuna Flank, a pilot's
  `+field/mark E=hostile` on a prospector (never armed) and on a second armed hull; no shot is
  fired, and each field demotes on its own interval with the Hostile still in reach.
- `gc2_a_wreck_hides_what_stands_behind_it` (`arena/wreck_cover.rs`): the sensor and fire lines
  blocked at the wreck's cell from both ends, open with the same hull alive, open to the wreck
  itself. The CF.7 story's G-8 assert flips with it (`tests/shard_1/field_story.rs` step 13: the
  escort's sensors never hold the raider behind the wreck).
- `gc2_a_survivable_hit_can_jam_a_weapon_and_the_check_in_keeps_it` (`arena/damage.rs`): a
  turret hit that leaves the turret at 100 jams the cannon; the record reads `jammed`, the gun is
  off the table with its rounds kept, the hit line carries three draws, the IC check-in keeps the
  jam (`Jammed`, degraded, not an operational weapon), and the next arena the hull enters brings
  no gun.
- Beside them: `gc2_an_unarmed_target_keeps_nobody_engaged` and
  `gc2_only_a_round_through_to_the_internals_can_touch_a_part` (the kernel halves), and
  `gc2_a_stripped_part_is_missing_and_not_destroyed` (`mush-tactics`). The CF.7 story's step 7
  lost its Friendly stand-down: the field demotes with only the unarmed barge Hostile.

**The rebless, taken once.**
What moved, each re-pinned with its reason beside it:
- **The stock pool's record** (`tests/shard_2/twin.rs`, `M4_5_POOLED_RECORD`, 21 lines): its
  hulls are content-backed, so the shipped chances jam and damage parts and wrecks give cover.
  Match ends move by up to 43 s, thinks 10,281 → 10,603, shots and wrecks a few per cent; every
  winner, the 14–1 split and `first_divergence_ms` stand, and nothing is `unlawful`. Two
  divergence counters leave nought: `line` 0 → 28 and `no_line` 0 → 31. That is the follow-up
  above, measured: the kernel's fire line finds a wreck that the mind's `line_to` cannot see.
- **Gate A's coverage pairs** (`COVERAGE`): `pooled` 10,008 / 273 → 10,295 / 308, `mixed`
  13,220 / 502 → 13,230 / 502, `lance` 6,001 / 164 → 7,157 / 130, and
  `POOLED_MATCHES_WITH_STAND_IN` 11 → 12. T1 ≡ T2 still holds on every lane, with no divergence.
- **The draw-label set**: seven kinds, `component` last (`arena/rng.rs`'s own pin and
  `mush-contract-tests`' `the_draw_label_set_is_pinned`). Adding it shifts no existing draw
  (`adding_a_label_shifts_no_existing_draw` is unchanged and green).
- **The cross-platform practice rows** (`tests/shard_2/minds_rhai.rs`,
  `M2_CROSS_PLATFORM_PINS`, D-UM-28): the twin corpus is content-backed too. Rows 0 and 1 fight
  differently, their stopping times moving 170,250 → 176,250 and 192,500 → 235,250 with every
  hash. Row 2 stops at the same 170,000 with the same state hash and substep digest, and only
  its ledger digest moved (its hit lines carry the third draw). Row 3, the float probe, is
  unmoved, with its 3,600 thinks and the 369098751 bitmask. The test is `#[ignore]`d and run by
  name only in CI's `tacsim-minds` cell, so the first local pass missed it and the PR's first CI
  run caught it (run 41387953901). Windows and Linux print the same four rows byte for byte, so
  this is a pin left behind, not a divergence between hosts. The two other by-name lanes beside
  it, the whole-match hashes between the runners and the thirty real pool tasks, were unmoved
  and green.

What did **not** move, and was checked: the shipped duel's hash and its 34 lines
(`tests/shard_3/motor.rs`), the (j) field's substep hashes and its 50 lines
(`tests/shard_2/reflex.rs`), the production encoding pins (`tests/shard_2/minds.rs`),
`STANDARD_SHA256` and the default seal digests. Those fights are hand-built fixtures with no
chances, or end before a wreck stands in a line. No snapshot format bump: the clock's new field
and the profile's new row decode from older snapshots by serde default.

Five tests changed their fixtures, not their claims. Four tempo tests (`reflex.rs`'s `t1_in_band…`,
`t1_t32…` and `e2_b45…`, and `weapons.rs`' `the_engaged_tempo_is_entered_on_a_shot_and_held`)
asserted `Engaged` against a target with no gun. That target now counts only once it has fired, so
each sets it to have fired. `extensions.rs`' `x4_a_shut_down_or_wrecked_pod_jams_nothing` moves its
wrecked pod one cell off the watch line, because a wreck there now hides the mark for a reason
that test is not about.

**Verified** on Windows: tacsim's lib (2,087), all six integration shards (shard_1's 401 with the
CF.7 story, shards 2 and 3's 800, shards 4 to 6's 786), `mush-tactics` and `mush-contract-tests`
(census, panic budget, restriction lints) green; clippy and rustdoc `-D warnings` clean on the
touched crates. As a probe, the two field gates fail with the tempo and idle rules switched off.
The broad suites run once more on Linux CI on the draft PR (#296) and the gate sweep on
conclav.dev.

## As built (GC.13)

GC.13 covers Q-36-45 (treatment). It touches no kernel hash. It changes content pins: the lane
goes to v19 and the relay seed to v10, re-signed to require it, with no entry changes. Every
number below is provisional under the owner's rule of 2026-10-24.

**The rule.** An injured or incapacitated character who stands in a room of an IC port
(`ic_return::port_of_room`) is under treatment there. They are fit again after a stay of IC
time. There is no skill check and no cost. The host's field tick drives treatment the same way
it drives extraction (`field/crew.rs`, `tick` → `treat`). The crew record carries the due
instant (`Treatment { port, started_ic, due_ic, medical }`), so no new scheduler was added.
A frozen world runs no tick, so a W-clock freeze pauses treatment as it pauses extraction.

- **Leaving (the rule chosen):** if the tick finds the character outside every port room, the
  treatment is dropped and the condition kept. That covers being aboard a hull, out on a site,
  or in Limbo or the OOC area. The clock starts over, in full, at the next port they stand in.
  A treatment begun at one port and found at another also starts over. This rule was chosen
  because it is the simplest one that cannot be gamed by stepping in and out. It needs no
  partial-progress state. Stepping out to Limbo or the OOC area also counts as leaving.
- **What counts as treated:** the character's physical room, not the crew record's location.
  A pilot who flies home and walks into the port is treated the same way as one extracted
  there.
- **Times** (`[tacsim.field]`): `treat_injured_s` = 86,400 (one IC day) and
  `treat_incapacitated_s` = 259,200 (three IC days). Each may be set from 60 to 2,592,000.
  `treat_medical_permille` = 500 (half), from 1 to 1,000, applies at a port with a medical room.
  An incapacitated character goes straight to fit, not through injured.
- **What the character hears:** a line when treatment begins (port, medical room, hours left),
  "Your treatment is done: you are fit again." when it ends, and a line when they leave first.
- **The crew status line:** `+sortie/condition` shows the condition and location, and the IC
  hours left on an extraction or a treatment. It names the medical room, or says that treatment
  begins at the next port. Note that `+sortie/crew` was already taken by the NPC hiring terms.

**The medical rooms** (port content). This follows GC.7's workshop: each port entry has a
`medical` block (`PortDef.medical`, `MedicalDef { room, operator, capabilities, beds }`).
The `PortApplier`'s new part k (`port/medical.rs`) opens the room as an industry facility of
its operator. The facility holds the `medical` capability (`port::def::MEDICAL_CAPABILITY`).
Treatment reads it back with `port::medical::has_medical_room`: a facility holding the
capability whose room is in that port. The block is refused on a site and on the workshop's
own room.
- **Ceres:** room C03, Arrivals (the customs counter under the Port Watch's crest), the Co-op's
  room, two beds.
- **Stickney:** room P03, Customs (the Republic's queue lanes), the Yard's room, four beds.

These are existing rooms, chosen because arrivals and customs are where a port screens the
people it takes in. Unlike the workshop, nothing is kept in the room, so moving it to a dedicated
clinic room later is cheap. The beds are authored but are not a limit yet: GC.13 treats every
character at the port at once.

**Pins moved:** the lane digest and `LANE_VERSION` (19), the relay digest and its v10
latches, and the strip helpers that rebuild older lanes (`support::sites::strip_workshops` and
`relay_registry`'s `strip_urvara` also remove `medical`). The site refusal sentence now names
the medical block.

**The gates** (shard_1, `crew_treatment.rs`):
- `gc13_an_extracted_incapacitated_pilot_is_fit_again_after_treatment`: B's biped has its head
  destroyed, and B is incapacitated in the field's first frames. B asks for extraction and lands
  at the port. Treatment begins at the medical room's time. Stepping out to a room off the map
  drops it and keeps the condition. Back at the port the clock starts over. At the due instant B
  is fit and told. `+sortie/condition` is checked at each stage.
- `gc13_a_port_without_a_medical_room_treats_slower`: the same path, with every medical facility
  closed before the extraction lands. The treatment takes the full time, twice the medical room's.
- `field::crew::tests::a_medical_room_treats_faster` (in-source): the times at the defaults.

**Defaults taken (provisional):** the three times; the leaving rule; the two rooms and their
operators; beds that are not yet a limit.

## 11. GC.1b as built (the kernel join)

Built 2026-11-01 on GC.2's branch (`claude/gc2-kernel-batch` at `17bd256a`, PR #296), because
GC.2 is the batch that reblessed the practice hashes, and stacked on it. It finishes Q-36-4: a
carrier that settles on a running field's pad is admitted at its pad, as a unit of its own party,
at the next pulse. No practice hash moved.

**The kernel's join** (`arena/join.rs`, `arena/state.rs`, `arena_task.rs`).
- *One path for a unit.* The per-unit build of `Arena::new_with_minds` moved, unchanged, into
  `Arena::admit_unit`: the pose and opening cell, the cost row and class, the consequences the
  record already carries, the seal verified against the frozen mind limits, the cards and
  profiles admitted, the sensors, the guns, the band cache. The check-out calls it once per unit;
  `Arena::join` calls it once. Four small pieces of the check-out came out with it for the same
  reason (`raise_seat`, `check_substep`, `alert_range_cm`, `jam_radius_cm`).
- *The message.* `ArenaMsg::Join { unit, pilot, host, spread }`. Like a withdrawal, it is buffered
  (one per unit, the first offer wins) and decided at the next pulse's head, before the step, so
  the unit lands at a pulse boundary. The answer, `ArenaOut::Joined`, carries the name it joined
  under or the refusal, and is queued behind the ledger so it is never lost. The message carries
  more than `{ unit, pilot }`: `host` is the field's standing friendships and marks (Q-CF-5,
  Q-CF-44) for a party the field has not seen yet, and `spread` is how far it may be stood from
  its pad.
- *What a join does.* The unit is inserted in unit-id order. Its party's book is opened if the
  field has none, with only those friendships and marks that name it, so every book already open
  is kept as it stands. It takes a letter its party has not given, drawn by the check-out's keyed
  mix of seed, party and id (Q-CF-11), so no other unit's name moves. The chassis table, the
  spatial index, the alert range and the jamming radius are laid down again. The substep bound is
  checked again. Its first sensor pass, and for a mind's hull its first think, fall at its next
  slot of their stagger grids. One `mind_sealed` line journals it at the join's millisecond, as
  every unit's does at check-out. The task puts it online, as `spawn_field` puts every admitted
  unit.
- *Where it stands.* At its pad pose, or, if another unit holds that cell, at the centre of the
  nearest free cell it may stand on within `spread` (`PAD_SPREAD_CELLS`, 8). The search goes
  nearest ring first, row-major within a ring: `pad_poses`' own rule, read against the arena's
  live occupancy, which only the arena knows while it runs. The ring walk moved into the kernel
  (`arena::join::ring_cells`), and `field::carrier` re-exports it.
- *All or nothing.* The join is built on a copy of the arena and swapped in whole. A refusal (a
  check-out's refusals, a unit already here, no free cell, a scripted seal, a host that is not
  the IC field's) leaves the arena exactly as it was.
- *What never joins.* A squad: squads are dealt at check-out for the OOC lobby. A scripted mind:
  an IC field carries none until doc 23 S.4b, and the task holds no source. An OOC match: the
  arena refuses unless its own end is `EndCondition::Host`, which only the IC host supplies, so an
  OOC arena refuses whatever host it is handed. The task refuses first, with no `FieldTerms` or
  with a tape, so no Join reaches an OOC arena's step or its input tape.
- *A fix on the way.* `Arena::remove_unit` (CF.5's withdraw) left the chassis table as it was,
  though it is kept in row order. After a withdrawal, every later row's frame words came from the
  unit before it. It now lays the table down again, as a join does. Frames only: the table is
  never hashed.

**The host's side** (`field/carrier.rs`, `field/admit.rs`, `field/boot.rs`, `field/record.rs`,
`arena_bridge.rs`).
- *The check-out GC.1 moved here.* Each tick, after GC.1 notes the owed arrivals, the host takes
  each one in (`carrier::join_arrivals`). It builds the hull's entrant as admission does: a
  carrier's party is the roster it launched, else its commander's party of one. The party keeps
  its letter if the field already has it, else takes the next one. The hull is checked out under
  the field's custody mark, its row (`FieldUnit::arrived`) and any new party go on the live
  record, which is written, and the Join is offered at its pad pose. The check-out row comes from
  `admit::custody_unit`, factored out of admission's check-out, so an arrival is sealed and
  described exactly as an admitted unit is. A busy inbox checks the hull straight back in, and the
  next tick tries again.
- *The answer* (`carrier::on_joined`). If the hull is taken in, the site is told
  `ARRIVED_IN_FIELD` ("The {name} has set down on the pad and is in field {n}; nothing launches
  from it while the field runs."). If it is refused, the hull is checked back in as it went out,
  its row and its own party of one leave the live record, and it is not asked again while this
  field runs (`FieldBook::join_refused`). It stays owed, so `owed_arrivals` still names it.
- *The registry.* At the offer, the hull is bound to the task with its controller and whether it
  comes in armed (`ArenaRegistry::bind_joining`), as the check-out binds every admitted unit. A
  refusal unbinds it.
- *Every party's presence picture.* A carrier admitted at promotion stays the pad on the map. One
  whose live row says it arrived is a flight in the presence book
  (`carrier::arrived_mid_field`, read by `surface::presence::publish`). So every party reads its
  arrival as a row, by bearing and band, with its class inside sight (Q-CF-15).
- *No launch.* Unchanged: the existing Q-CF-17 refusal (`FIELD_LAUNCH`) holds the arrived
  carrier's bay shut while the site is promoted.
- *Close and void.* Nothing new. The joined hull is a live row, so demotion checks it in with the
  field's own rows, and its carrier crew is checked in from the row's `aboard` like any other.

**The determinism record and its replay.** A field records no input tape (an IC field has none,
Q-CF-43), and an OOC match never joins, so no `TapeEvent` was needed and the tape format did not
move. A field's durable record is its live record plus its last snapshot, and a restart resumes
from the two (Q-CF-33). A join changes both:
- The live record gains the hull's row in the same batch as the check-out, before the message is
  sent.
- Every snapshot after the join holds the unit, its letter and its party's book (they are arena
  state, carried and hashed as any unit's).

The gap is a restart between the join and the next snapshot. The live record names the hull and
its custody mark holds it, but the snapshot predates it. The resume (`boot::resume`) closes the
gap: every `arrived` row the decoded arena does not hold is joined again, at its pad, through the
same `carrier::joining` builder and `Arena::join`, before the task reopens. A hull that can no
longer join is checked back in and leaves the record, as a refused join does. A resumed field
rewinds to its snapshot in any case, so the re-join lands at the snapshot's millisecond, and the
field runs on from there. `FieldUnit::arrived` is serde-default and omitted when false, so every
live record written before this cut decodes, and re-encodes byte for byte.

**The gates.**
- `field_carrier::gc1_a_carrier_arriving_mid_field_is_admitted_at_its_pad_and_cannot_launch`
  (shard_1, GC.1's deferred gate, replacing GC.1's interim test). A second Dray-P settles on
  Ahuna Flank's pad while CF.3's field runs. It is checked out under the field's mark with an
  `arrived` row in a party nobody held at the opening. Nothing is owed after that. The armed
  hull's sensors, which held one contact on the pad before, hold two. The presence book has the
  arrival as a flight, the field runs on, and the arrival's rig launch is refused. When the field
  demotes, its black box carries the arrival's `mind_sealed` line at a non-zero millisecond, and
  the demotion has checked it in with the field's own rows.
- `motor::gc1b_an_ooc_match_offered_a_join_keeps_its_practice_hashes` (shard_3). This is the
  no-OOC-hash pin. The shipped duel is offered a join between every pair of pulses of its minute,
  under an IC-shaped host. It refuses each one, each refusal leaves the state hash as it was, and
  the duel ends on GC.2's reblessed `DUEL_HASH_API6_OBSERVED` and, through the API 4 control, on
  `DUEL_HASH_AT_HEAD`.
- Beside them, in-source: `arena::join`'s four (a joiner in a party of its own with its books;
  one in a held cell stood beside it, or refused whole with no spread; one in a party already on
  the field taking an ungiven letter, and a duplicate refused; an OOC arena refusing and
  unchanged) and `arena_task::gc1b_only_a_field_task_takes_a_join`.

**Defaults taken (provisional).**
- The arrival's line to the site.
- A refused arrival is not retried while its field runs. It stands outside, owed, until the field
  demotes.
- A party's letter for a newcomer is the next in the alphabet, not re-sorted by key. The opening
  rule (Q-CF-11, letters by key order) cannot be kept without renaming the parties already on the
  field.

**Verified** on Windows: the gate, the no-OOC-hash pin beside `m1a1_the_shipped_duel_hashes_as_before`,
the five in-source join tests, and the CF.7/GC.0 field story at 256
(`gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark`) and at 512
(`s2_the_field_story_at_512`) are green; `mush-contract-tests`' panic budget, restriction lints,
gated-test census, tacsim shard and arena library guards are green; clippy and rustdoc `-D warnings`
are clean on tacsim. The broad suites run on Linux CI on the draft PR.

## As built (GC.12)

**The cap, measured** (Q-36-9). `FIELD_MAX_UNITS` is now **100**, raised from 24. This section
records two measurements: the first on main before GC.2, and the second on main after GC.2
merged. The constant is set from the second.

**The harness.** `field_scale::gc12_the_field_pulse_is_measured_at_scale` in
`crates/tacsim/tests/shard_1/field_scale.rs`, one `#[ignore]` test, so the ordinary shards never
run it. Both measurements below were run by hand, as `cargo test --release -p tacsim --test
shard_1 field_scale:: -- --ignored --nocapture --test-threads=1`. gate.yml's `sweep` dispatch
should also run it, with `packages` set to `tacsim`, `redis` off, and `extra` set to
`--run-ignored only --cargo-profile release --no-capture -E 'test(/^field_scale::/)'`. Nobody
has run it that way yet.
- *The field:* the field's arena on Ahuna Flank's priced ground, built as S.2's `story_arena`
  builds it (`field_story.rs`). It holds `n` units, every one a stock mind under the production
  seal terms. The cohort is mixed: line tank ×3, laser, biped, thermal, ECM and ECCM, in that
  order within each party.
- *The parties:* six parties, `A` to `F`, in blocks on a ring 18 cells out from the pad. They
  open under the IC host's terms (`EndCondition::Host`, every class, weapons hold). `A–B` and
  `C–D` are friends by roster. The marks are not symmetric: `B` holds `D` neutral while `D`
  holds `A` hostile, `E` minds only `F`, and `F` is a raider marking everyone. Fire then adds
  its own hostility.
- *The two loops:* each size runs twice, for 120 arena seconds, with a fresh world each time.
  - **lattice:** steps at `StepOutput::next_due_ms`, the field task's loop. This is the verdict
    lane.
  - **fixed100:** steps in 100 ms pulses, S.2's loop, so its figures compare with S.2's.
- *Acknowledging the ledger:* the harness acknowledges each pulse's chunks, as the field task
  does (`Ledger::mark_flushed`). Without that, a 100-unit field backs up past
  `ledger.max_unflushed_chunks` and voids itself at 22.25 s with `LedgerOverflow`. S.2's
  9-unit rig never acknowledges, and stays under the bound for its minute.
- *What it prints:* one `GC12` line per size and lane:
  - pulse median, p95, p99 and max, over all wakes and over `Engaged` wakes;
  - the worst 500 ms window;
  - the snapshot at the opening and at the end;
  - contacts held, and the parties' books encoded;
  - the timed cost of `Arena::hostile_in_weapons_range`, and of a sweep of every held contact
    through `Relations::hostile`;
  - the ledger's bytes;
  - the worst wake's traces, cell probes, share operations and path work;
  - mind faults, and the closing state hash.
- *Knobs:* `GC12_SIZES` (default `9,24,48,64,80,100`) and `GC12_ARENA_S` (120).
  `GC12_ASSERT=1` turns the budget into an assertion. The default is report-only (D-H-8).

**The budget.** `[tacsim.harness].engaged_p99_ms`, 40 ms a pulse in `Engaged` tempo, set on the
production VPS (16c `T-109`). This is the only pulse-time budget in the config: `[tacsim.field]`
holds the cap but no time, and Q-CF-45 budgets units, not milliseconds. `tempo.stall_ms` (2,000)
is the watchdog, not a budget. For reference, the OOC harness's stricter lens is 20 ms a wake
and 32 ms per 500 ms window.

**The first measurement on main.** Tree: main `9dd000a4` plus the harness (`eaa089e9`).
Machine: conclav.dev, AMD EPYC 9J45, 32 vCPU, 62 GB, Ubuntu 26.04, idle, the test on one
thread. Timings are milliseconds, from the lattice lane, `Engaged` wakes, release build. From 24
units up, the field was engaged by 3.25 s and stayed engaged: 467 of the 480 wakes. At nine units,
347 of them were.

| Units | Shots in 120 s | Standing at the end | Pulse median / p95 / p99 / max | Worst 500 ms window | Snapshot, opening → end | Contacts at peak | Worst wake: traces / cell probes / share ops |
|---|---|---|---|---|---|---|---|
| 9 | 335 | 5 | 0.045 / 0.101 / 0.150 / 0.225 | 0.28 | 137 KB → 171 KB | 46 | 42 / 144 / 2 |
| 24 | 1,287 | 9 | 0.210 / 0.520 / 0.609 / 0.697 | 1.20 | 350 KB → 410 KB | 544 | 260 / 504 / 30 |
| 48 | 1,912 | 20 | 0.479 / 1.009 / 1.396 / 2.126 | 2.87 | 684 KB → 881 KB | 2,192 | 780 / 1,509 / 112 |
| 64 | 2,981 | 29 | 0.801 / 1.840 / 2.286 / 3.847 | 4.78 | 900 KB → 1.25 MB | 3,915 | 1,333 / 2,941 / 189 |
| 80 | 3,745 | 36 | 1.337 / 3.025 / 3.542 / 7.687 | 9.04 | 1.13 MB → 1.61 MB | 6,227 | 1,863 / 3,814 / 267 |
| 100 | 3,365 | 49 | 1.249 / 4.777 / 6.375 / 12.762 | 14.63 | 1.40 MB → 1.94 MB | 9,704 | 2,896 / 6,063 / 419 |

- **Debug, same machine, lattice `Engaged` p99 / max:**
  - 9 units: 2.07 / 3.69
  - 24 units: 6.07 / 7.36
  - 48 units: 12.41 / 24.23
  - 100 units: 58.59 / 143.08, past the budget. A debug build is not production.
- **S.2's comparison, the fixed100 lane at nine units:**
  - debug: median 0.267 ms, p99 1.14;
  - release: median 0.018 ms, p99 0.084.
  - S.2 printed 0.35 and 0.8 in debug on the Windows box, and 0.03 and 0.13 in release on
    conclav.dev, over a minute of its own story's cells.
- **The fixed100 lane at 100 units (release):** median 0.613, p99 2.774, max 7.607.
- **Determinism:** the closing state hash of each size and lane is the same in release and in
  debug, and across two release runs.

**What the numbers say.**
- **100 units is within budget on main, with room to spare.** The p99 of 6.4 ms is a sixth of
  40 ms. The worst single wake, 12.8 ms, and the worst window, 14.6 ms, are inside even the OOC
  harness's 20 ms and 32 ms lens.
- **The cost grows with contacts, roughly as the square of the units.** From 48 to 100 units
  (×2.1), the p99 grows ×4.6, peak contacts ×4.4, and worst-wake traces ×3.7. Perception is
  where the time goes.
- **Traces near their OOC bound at 100 units.** The worst wake's traces, 2,896, are the
  measure nearest its OOC harness bound, 4,000 (`max_traces_per_pulse`), which fits about
  120 units. Cell probes (6,063 of 32,000) and share operations (419 of 768) have more room.
  Those bounds are the OOC harness's assertions, not throttles, and nothing in the field
  enforces them.
- **The relation table, Q-36-9's worry, is not the cost:**
  - The parties' books stay at 515–542 bytes at every size. They grow with parties and marks,
    not with units.
  - `hostile_in_weapons_range` costs 0.1–3 µs. It is an `any()`, so a field that is engaged
    answers at its first hostile pair.
  - A full sweep of every held contact through the books is 3 µs at 100 units in release.
  - What grows with parties × contacts is the contact store itself (9,704 rows at its peak),
    and that cost is perception's.
- **Snapshots grow linearly**, at about 14 KB a unit at the opening. A 100-unit field ends its
  two minutes at 1.94 MB, a quarter of the OOC harness's 7 MiB lane bound and far under
  `snapshot_max_bytes` (480 MiB).
- **The ledger at 100 units:** 7.4 MB in two minutes of non-stop fighting, against the 64 MiB
  `ledger.max_bytes`. A field engaged for 18 minutes at that rate would reach the bound and stop
  with `Timeout`. Doc 32 Q-CF-45 never sized the ledger for this many units. It is flagged here,
  not changed.
- **Caveat:** the machine is conclav.dev, not conclav.io, the host the 40 ms was set on. The
  sixfold margin is the reason a conclav.dev figure is trusted here.

**The re-measure after GC.2.** GC.2 (PR #296, merged as `c2653243`) changes the tempo rule in
the hot loop, so the harness ran again. Tree: main after GC.2 plus the harness (`34416410`, the
branch's merge of main). Same machine, conditions and lanes as above. The opening is the same
world as on main; GC.2 changes how the fight goes (more shots, more units standing), so the rows
differ from the table above.

| Units | Shots in 120 s | Standing at the end | Pulse median / p95 / p99 / max | Worst 500 ms window | Snapshot, opening → end | Contacts at peak | Worst wake: traces / cell probes / share ops |
|---|---|---|---|---|---|---|---|
| 9 | 344 | 5 | 0.047 / 0.094 / 0.115 / 0.204 | 0.28 | 138 KB → 159 KB | 46 | 42 / 144 / 2 |
| 24 | 1,633 | 12 | 0.239 / 0.489 / 0.634 / 0.678 | 1.25 | 352 KB → 460 KB | 544 | 260 / 504 / 30 |
| 48 | 2,263 | 26 | 0.519 / 1.092 / 1.916 / 2.252 | 3.99 | 687 KB → 906 KB | 2,192 | 780 / 1,509 / 112 |
| 64 | 3,262 | 38 | 0.931 / 1.828 / 2.261 / 3.808 | 4.78 | 904 KB → 1.31 MB | 3,915 | 1,333 / 2,941 / 189 |
| 80 | 4,028 | 42 | 1.496 / 2.991 / 3.554 / 7.516 | 8.87 | 1.13 MB → 1.65 MB | 6,227 | 1,863 / 3,814 / 267 |
| 100 | 3,818 | 63 | 1.612 / 4.870 / 5.986 / 12.385 | 14.32 | 1.41 MB → 1.96 MB | 9,704 | 2,896 / 6,063 / 419 |

- **Debug, lattice `Engaged` p99 / max:** 9 units 1.78 / 2.33; 24 units 6.38 / 7.33; 48 units
  12.74 / 25.29; 100 units 60.17 / 146.80, past the budget, as on main.
- **The fixed100 lane at 100 units (release):** median 0.591, p99 2.820, max 7.456.
- **The ledger at 100 units:** 7.8 MB in two minutes, as on main.
- **Determinism:** each size's closing hash is the same in release and in debug.

**The cap: 100.** The largest size measured is 100 units. Its `Engaged` p99 is 6.0 ms against
the 40 ms budget (`[tacsim.harness].engaged_p99_ms`), and its worst wake is 12.4 ms. So
`FIELD_MAX_UNITS` (`site_record.rs`) is 100. With it move the default of `[tacsim.field].max_units`,
which may still be set anywhere from 2 to 100; `MAX_EXCURSIONS_PER_SITE`, which is the cap;
and the mush-config default. Nothing past 100 was measured, so nothing past it is claimed. Past
about 120 units, the worst wake's traces would cross the OOC harness's 4,000 bound, and a cap
above 100 needs its own run. Two other things were checked at the cap:
- A site record whose open field holds 100 settled rigs and 100 exits, beside a full salvage
  memory, stays under the 64 KiB record ceiling (`gc12_a_field_at_the_cap_fits_the_record`).
- An `open`'s rig list at 100 ids is about 1.1 KB, under the 4 KiB bound on an offer's text.

The ledger still fills in about 18 engaged minutes at 100 units, as noted above; that is a
follow-up for the ledger's sizing, not a bound on the cap.
