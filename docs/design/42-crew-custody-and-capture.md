# 42. Crew: custody, capture, ejection, treatment — survey

Draft 0.4, 2026-11-03, docs only. Surveyed against `origin/main` at `c83c2eff` (doc 38 Draft 0.2
merged as PR #345; doc 36a's lane 5 merged as PR #346, so `check_in` already keeps a prior
injury). Draft 0.1, the same day, was the survey. **Draft 0.2 records the owner's ruling of
2026-11-03: every Q-42 default is accepted as written, and §5's consequences are acknowledged.**
No question is amended, so each question's proposed default is its decision; §7's cut list is
binding from this draft and CR.0 may start now. **Draft 0.3 folds the second outside review**
(two reviewers, every claim re-verified read-only against `main` at `f146734e`): seven items,
each a "Draft 0.3 (review round 2, 42-n)" note under the question it corrects — 42-1 under Q-42-12,
42-5/42-8 under Q-42-1 and Q-42-12, 42-2 and 42-7 under Q-42-7, 42-3 under Q-42-6, 42-4 under
Q-42-5, 42-6 under Q-42-2 — plus 42-9's advice under Q-42-16. None changes an accepted ruling;
42-4 needed a choice the rulings did not make, asked as Q-42-26. **Draft 0.4 records the owner's ruling
of 2026-11-03 on Q-42-26: the proposed default is accepted as written** ("Accept all seven", across
docs 23, 40, 41 and 42), and notes doc 40's Q-40-25, accepted at its default the same day, where
CR.4 and 42-9 read it. Docs 23 (custody for
moving anchors), 39, 40 and 41 are merged beside this one; references to them point at the
merged text, and 42-7's settle path is doc 41's `+debts/pay`. This is the crew document doc 38 §6 proposes: "new rulings first (the reviewers are right
that none exist); touches ownership and permissions". It is a survey in the form doc 38 used: it
asks the owner questions, each with a proposed default, an alternative and a reason. "Provisional"
marks a number to be tuned later under the 2026-10-24 rule. **(expensive to reverse)** has doc 38
Q-38-10's meaning — an answer that would take a significant refactor of the codebase to undo, not
stored data and not a live box — and is used once (Q-42-1). The implementation plan is §7, written
from these rulings; each cut is recorded as built in a section of this document, as doc 36 did.

**The binding ruling** is doc 38 Q-38-22 (owner, 2026-11-03, the default accepted): *minimal
custody.* `CrewLocation::Captured { carrier, captor }` replaces doc 36 Q-36-52's abandonment for
a completed non-friend load; a captive is handed to a port's authority at docking and released
for a fee, or held — no interrogation, no ransom market; ejection is a forced extraction from a
doomed unit with a capture chance when a Hostile party holds the cell. Every question below
either fills a gap that ruling leaves or picks the smallest shape that fits it.

## 1. Why this document exists

The IC combat field (doc 32) and ground combat after it (doc 36) were built with a crew model
that stops short on purpose. Doc 32 Q-CF-38 ruled "no treatment, no ejection, no capture in the
first version"; doc 36 Q-36-45 built treatment (GC.13) and left "ejection first: a forced
extraction with a chance of capture" as its alternative; doc 36 Q-36-52 (the owner, 2026-11-02)
let a non-friend attach, tow and load a casualty with crew still aboard and had every character
aboard **abandoned** to the Q-CF-39 extraction "until capture is built". Doc 37 §7.3 named capture
the third area after the phase; both outside reviewers (doc 38 §3, R1-9 and R2-4) asked for it and
said it "requires new design rulings for incapacitation, custody, and player recovery". Doc 38
Q-38-22 ruled its scope. This document turns that scope into the rulings a build can run on.

It covers seven things, each with what the tree has, what the ruling requires and the questions
left: the custody model (§4 A), the port handover (§4 B), ejection (§4 C), rescue (§4 D),
treatment (§4 E), the seams with ownership, standing, crime, the bot and space (§4 F), and
acceptance (§4 G). §5 lists what the defaults change elsewhere; §6 sketches the stories at both
sizes; §7 is a provisional cut list CR.0–CR.7; §8 is what the plan still needs.

## 2. Already decided; not asked again

| Ruling | Where | What this document does with it |
|---|---|---|
| No player permadeath; the worst case is retrieval by an NPC organization to a known port; crew rescue never retrieves or repairs the unit | doc 27 FO-12 (l.89), Q-H12 (l.685) | Kept whole. Captivity is a delay on the way home, never a dead end (Q-42-8). |
| NPC extraction starts on the stranded character's request, refused only while a friendly rescue is physically under way within a published bound; no timeout extracts an unwilling pilot | doc 27 Q-F4 (l.967), P-5 (l.1091) | Kept. Ejection is the one extraction a pilot starts without the stranding (Q-42-10); "injury/capture may delay it only up to that bound" is the captivity bound (Q-42-8). |
| Two fields on the character, location and condition, set at check-in and on a control-section kill | doc 32 Q-CF-38 (l.496) | Kept; the location gains one variant (Q-42-1). "No ejection, no capture" is amended by Q-38-22. |
| A character cannot leave a hull on foot in a field; extraction is a bounded teleport-with-delay | doc 32 Q-CF-39 (l.504) | Kept. No infantry, no boarding, no walk to the enemy's ramp: a captive moves only as a load, a handover or a release moves them. |
| Recovery to fit at a port after a provisional IC time, faster with a medical room, no skill checks | doc 36 Q-36-45 (l.627), GC.13 (l.1510) | Kept; the beds question is asked (Q-42-17). |
| A non-friend may attach, tow and load; the load completes whatever is aboard | doc 36 Q-36-52 (l.715) | Kept. Only the crew's outcome changes: captured, not abandoned (Q-42-1, Q-42-16). |
| Anybody recovers, on any duty; no separate recovery sortie | doc 36 Q-36-14 (l.364) | Applied to the deferral too (Q-42-14). |
| Unlawful recovery is evidence only; a minimal port-security consumer later | doc 36 Q-36-30 (l.495); doc 38 Q-38-20 (l.706) | A capture gets its evidence row (Q-42-22); the consumer stays doc 41's. |
| Friends by owner or open organization; standing at demotion through mush-faction receipts; a leader may void friendly fire | doc 36 Q-36-10/11 (l.328, l.336), GC.10 (l.2700) | Reused for the one standing effect this document proposes (Q-42-21). |
| Every new cut's story runs at 256 and 512 and the bot types the player's half over SSH | doc 38 Q-38-29 (l.780) | The acceptance rule of §6 and §7. |
| "(expensive to reverse)" means a refactor to undo | doc 38 Q-38-10 | Used once, on the record's shape. |

**Out of scope by ruling.** Interrogation and a ransom market (Q-38-22); infantry and on-foot
movement (doc 27 FO-6, Q-CF-39); boarding a carrier or a ship (doc 32 §5 "carrier damage, boarding
and loss"); facility assault; a prisoner exchange market; wages for captive NPC crew (doc 27 Q-R5);
persistent known-hostile memory (doc 32 F-7). Ejection from a ship in a space bubble is the space
track's (Q-42-24).

## 3. What the tree has (checked at `c83c2eff`)

**The record** (`crates/tacsim/src/field/crew.rs`). `CrewLocation` is `Unit { unit }`,
`Carrier { hull }`, `Stranded { site, unit }`, `Port { port }` (`:101–124`), serde-tagged `at`;
`CrewCondition` is `Fit`, `Injured`, `Incapacitated`, ordered by severity (`:131–140`);
`Extraction { port, requested_ic, due_ic, abandoned }` (`:143–154`, the flag omitted when false);
`Treatment { port, started_ic, due_ic, medical }` (`:159`); `CrewRecord` adds `at_ic`,
`organizer`, `extraction`, `treatment` (`:173–196`), `CREW_FORMAT` 1, one document a character at
`tacsim:crew:<n>` on the module's lane, RAM in `CrewBook` (`:214`), read at boot by `lib.rs:1267`
(`read`, `:241–265`) and written through `put` (`:267–286`). `crew_location` prefers a running
field's carrier row (`FieldUnit::aboard`) over the record (`:226–239`). Only `ObjKind::Player`
gets a record (`check_in`, `:401–407`): hired NPC crew have none.

**Check-in** (`check_in`, `:397–481`). A soul whose extraction is `abandoned` is left alone
(`:407–413`, lane 5's row 4); the pilot of a hull that lacks power, control or motion, or reads
`Wreck`/`Irrecoverable` (`:430–439`, the live assessment through `field_ops::assess_tacsim_unit`),
is `Stranded`; a soul aboard a carrier whose hull fell is stranded in the wreck (`aboard`, `:357`,
`CarrierLoss::CrewStranded`, `carrier_damage.rs:88–95`); the condition written is the worse of the
record's and the hull's (`:457`). It runs at demotion (`close.rs:139–140`), at an exit
(`exit.rs:247–248`) and at a resumed field's boot (`boot.rs:308–314`); `on_frames` incapacitates
the pulse a control section (`head`, `cockpit`, `bridge`, `:66`) is destroyed around a pilot
physically inside (`:643–692`, called from `field/mod.rs:387`).

**The load's two halves** (`surface/tow.rs`). `friendly` (`:834–859`) reads the casualty's own
leased bay or the field's Friend rule (`field::parties::friends`); the physical step calls
`crew::aboard_loaded` for a Friend's carrier and `crew::abandoned_loaded` for anyone else's
(`:2302`, `:2316–2321`); a written-off lease rig abandons too. `aboard_loaded` moves every soul
held in the casualty to `Carrier { hull }` and cancels their extraction (`crew.rs:499–542`);
`abandoned_loaded` (`:553–607`) strands them in the hull and starts an extraction `abandoned:
true` to the safe port they know (`destination`, `:694`: the IC-return stamp's port, else Stickney
Yards, `FALLBACK_PORT` `:64`), with the line `ABANDONED` (`:545`). The foreign load's preview says
so (`LOAD_PREVIEW_FOREIGN`, `tow.rs:126`). The hull enters the carrier's custody with a claim owed
(`SalvageCustody { hull, holder, carrier, recovered_by, recovered_ic, … }`,
`surface/salvage.rs:94–125`; `holder` is the carrier's operator).

**Extraction and the tick.** `+sortie/extract` (`detachment/sortie/commands.rs:130–135` →
`crew::extract_cmd`, `crew.rs:712–758`) needs a stranded or incapacitated character still inside
the hull (`extractable`, `:199–210`; `inside`, `:288–300`), refuses a second request and a frozen
world, and writes the record from the Authoritative bracket (`legacy-handlers.toml:339`). The
host's `tick` (`:795–836`, from `admit.rs:78`) lands every due extraction unless a Friend's
recovery flight stands on the site — an open DT flight on `CrewDuty::Recovery` under the same
organizer (`rescue_on_site`, `:760–793`; `detachment/crew/mod.rs:26–30`) — within
`rescue_bound_s`, never for an abandoned one. `land` (`:991–1116`) releases the seat on every arena
channel, vacates the station, moves the character to the port's arrival room
(`ic_return::arrival_room`, `ic_return.rs:240`), stamps the port (`:216`) and writes
`Port { port }`. The knobs: `[tacsim.field].extract_s` 21,600, `rescue_bound_s` 7,200
(`mush-config/src/config.rs:111–112`, `:916–917`).

**Treatment** (GC.13). `treat` (`crew.rs:863–936`) runs in the same tick over every hurt record:
a character standing in a room of an IC port (`port_here`, `:856`; `ic_return::port_of_room`,
`ic_return.rs:181`) is under treatment there for `treatment_s` (`:838–854`): `treat_injured_s`
86,400, `treat_incapacitated_s` 259,200, scaled by `treat_medical_permille` 500 at a port with a
medical room (`config.rs:113–115`, `:918–920`; `port::medical::has_medical_room`,
`port/medical.rs:107`). One who leaves loses it and starts over. The rooms are content:
`MedicalDef { room, operator, capabilities, beds }` (`port/def.rs:214–224`), opened as an industry
facility with the beds as its slots (`medical.rs:21–40`); Ceres' C03 Arrivals, operator `coop`,
two beds, and Stickney's P03 Customs, operator `stickney_yards`, four
(`content/lane/ceres_phobos.json:2827–2828`, `:2884`). "The beds are authored but are not a limit
yet" (doc 36 l.1555). `+sortie/condition` is `status_line` (`crew.rs:948–989`;
`commands.rs:137–145`).

**The verbs.** The field's are `+field`, `/mark`, `/fire`, `/void` (`field/commands.rs:43`); the
crew's are `+sortie/extract` and `+sortie/condition` (`sortie/commands.rs:13–27`); the rig's cab
is entered and left by `+tv/<n> board|leave` (`port/tv.rs:2257–2261` → `boarding::player`,
`port/boarding.rs:151`, "Only the lessee boards themself"); a hull's airlock and bay by `+ship/board`,
`+ship/leave`, `+tv/bay in|out`. Nothing ejects, captures, hands over or releases. `help/field.md:288`
says "There is no ejection or capture" and `crew.rs:22` "No ejection or capture (Q-CF-38)".

**Parties and relations.** A party's `Entrant` carries `aboard` and `carrier` — its DT flight's
hull, its rig lease's hull or its raider's fixture carrier (`field/parties.rs:53–56`);
`principals` is the leader and, for a roster, the organizer (`:116`); `friends` and `friends_of`
apply the owner-or-open-organization rule (`:480`, `:509`). The kernel's `Relation` is `Own`,
`Friend`, `Hostile`, `Neutral`, `Unknown` (`arena/party.rs:167–180`); `Relations::relation`
(`:266`) and `hostile` (`:294`) read it; `hostile_in_weapons_range` (`arena/state.rs:5648–5663`) is
the one existing "an armed hostile inside my reach" test: a contact the viewer holds hostile, armed
at check-out or having fired, not read dead, inside the viewer's `fight_inner_cm`. There are no
declared stances (doc 38 Q-38-23 is doc 40's).

**Units and dooms.** `OperationalState` is `Operational`, `Immobile`, `Unarmed`, `Disabled`,
`Wreck`, `Irrecoverable` (`mush-tactics/src/field_ops/condition.rs:202–220`);
`FieldUnitAssessment` carries `has_power`, `has_control`, `can_move`, `can_fire`, `is_towable`
and the section conditions (`:394–416`). A unit definition has no ejection-seat flag.

**Evidence, custody of hulls, standing, grants.** `SiteRecord.unlawful` is a bounded log of
`UnlawfulAct { kind, actor, subject, landholder, at_ic }` with `UnlawfulKind` `Pickup`,
`Recovery`, `Scrap`, `Vacated` (`surface/site_record.rs:484–490`; `surface/claim.rs:389–416`,
`log_unlawful` `:420`, `MAX_UNLAWFUL` 64 `:79`), written by GC.5/GC.6 and read by nothing that acts.
Standing is written through `mush_faction::record_standing` / `record_voidable_standing` /
`void_standing` (`mush-faction/src/reputation.rs:251`, `:268`, `:332`) with `StandingEffect
{ faction, delta }` (`:27`); the field writes `STANDING_PER_ROUND` −1 bounded at
`STANDING_FIELD_CAP` −10 a pair a field, cause `field:<site tag>:<ordinal>:…`
(`field/standing.rs:43–49`, `cause_prefix` `:130`). Operational rights over a unit are
`UnitCapability` `OperateCrew`, `Deploy`, `CommandAssignedAi`, `RecoverLoad`, `MaintainRefit`,
`TransferAbandonTitle` (`mush-org/src/unit_authority/record.rs:38–45`; doc 26b); the bay's
`Right` is `Title`, `Grant`, `Lessee`, `Salvor` (`port/bay_work/record.rs:68–78`). No grant,
right or record names a person as its object. Doc 27 §4.5 (l.231) keeps physical custody apart
from title, ground right and claim.

**Ports and docking.** The IC ports are Ceres Freeport, Stickney Yards, Aitken Highport,
Voltaire Anchorage, Ligeia Terminal (`ic_return.rs:80–115`). A voyage's arrival at a port is
applied in `composition/sim_routes.rs:151–161`, where the hull's berth becomes `Berth::Docked
{ map, def }`; a site's pad is the same berth with `def.site` set (`lib.rs:1227`). A port entry
authors its organizations (`ceres_phobos.json`, `/entries/ceres_port/def/orgs`: Ceres Clearing,
the Co-op, the Co-op Trust, the Freeport Exchange, Occator, Tycho Mutual Assurance & Salvage, the
Freeport Registry, the Pallas Syndicate; `/entries/stickney_port/def/orgs`: Kasei Metals, Stickney
Yards, the Yard Commissary). The Port Watch is the Freeport's customs arm in the setting (doc 11
§3.3 l.79) and appears in Ceres' room prose and cast, not as an organization id. `PortDef`
(`port/def.rs:119`) has no `authority` field. `souls_aboard` (`port/voyage.rs:1306`) lists a
hull's occupants for a departure; `bay_gate_leaving` (`:2838`) holds a lift for units still out.
Debts exist as `mush_economy` obligations (`obligation.rs:397`); a fee is posted as an economy
plan (`api.rs:1267`), the bay's and the lease's precedents escrow or post it to the operator's
account (`bay_work/mod.rs:719`; `tv.rs:17`).

**Space.** The space track never calls `field::crew`: a ship's crew have no record. A disabled
ship is `HullStatus::DisabledDerelict` (`space/tick.rs:1031`); the only crew rule is linkdead
safety (`handle_crew_linkdead`, `:1275–1300`, D-35-7), which queues a return to the nearest port
(`nearest_return_port_for_hull`, `space/commands.rs:1662`). A docking session targets a
`PortBerth` (merging into `Berth::Docked`) or a `DisabledDerelict` hard-dock
(`space/docking.rs:258–270`, D-35-12). `all_aboard_crew_for_hull` (`commands.rs:286`) is the
occupant read.

**The bot.** `mush-bot` types tow, load and release for the escort (`mine/escort.rs:812–830`) and
the field's mark and orders for the captain (`mine/field.rs`, `in_vocabulary` `:742`), nothing for
`+sortie/extract` or `+sortie/condition` (doc 38 §4 K); each lane refuses a line outside its
vocabulary (`detachment.rs:733`, `escort.rs:799`, `lane.rs:252`).

**The stories.** `field_story.rs` runs one script on Ahuna Flank (256) and Urvara Reach (512)
through a `Ground` table (`tests/shard_9/field_story.rs:88–125`, `:406`, `:414`). The crew gates
are single-size: `gc5_a_non_friend_load_takes_custody_and_abandons_the_crew`
(`tests/shard_1/field_gc5.rs:336`), `gc5_review_the_captors_field_close_keeps_the_abandoned_extraction`
(`:511`), `gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift`
(`field_gc4.rs:107`), `crew_treatment.rs`'s two GC.13 gates and `crew_exit.rs`, all on Ahuna
Flank through `support/field/driving_rig.rs`.

### 3a. Contradictions and stale text found

- `help/field.md:288` and `crew.rs:22` say there is no ejection or capture; `crew.rs:36–38` says
  abandonment stands "until capture is built". All three are superseded by Q-38-22 and are docs
  work for the first cut (§7 CR.0).
- `rescue_on_site` (`crew.rs:760–793`) still keys the deferral on `CrewDuty::Recovery` although
  Q-36-14 (owner) made recovery anyone's on any duty and kept the duty "its one effect (a Friend's
  recovery flight defers an extraction)". A Friend's prospector dragging the casualty home does
  not defer the extraction; a recovery-duty flight that never attaches does. Q-42-14 asks.
- A pilot seated by `+tv/<n> board` is never stranded (doc 36 lane 5's owed list, l.3359–3362:
  `inside` reads a rig's cab as outside the rig), so GC.5's abandonment, and therefore any
  capture at a load, reaches only DT-seated pilots today. The fix is the fix thread's; CR.1
  inherits the gap and its gate must seat the captive through a DT flight until it lands.
- NPC crew have no crew record (`check_in`'s `ObjKind::Player` test), so a hired NPC in a captured
  hull is neither stranded nor abandoned and rides along as the hull's seated crew. Q-42-5 asks.
- Space has no crew state at all (above). Q-36-46/47/48 rule one path for ground and space; the
  crew half of that path is unbuilt on the space side. Q-42-24 asks.
- Doc 36 Q-36-45's text says "medical facility"; the build is a room with the `medical`
  capability and authored beds (`MedicalDef`), which Q-42-17 takes as the facility.

## 4. Owner survey

**Ruled 2026-11-03.** The owner accepted every default as written, Q-42-1 to Q-42-25, and
acknowledged §5. No question is amended, so the proposed text below is the decision and is kept
as written for the record; a cut cites the question it builds. Draft 0.3's review notes are
precisions of those decisions and bind the build with them. **Q-42-26** (Draft 0.3, under Q-42-5)
was ruled the same day at its default (Draft 0.4), so CR.1 is unblocked once CR.0 lands.

Each question has a proposed default, an alternative and a reason. Numbers are provisional.

### A. The custody model

**What exists.** §3: four locations; a non-friend's completed load strands and abandons; the
hull's custody is a `SalvageCustody` row naming the carrier's operator as holder; the record is
read at boot and `check_in` leaves an abandoned soul alone.

**What the ruling requires.** `CrewLocation::Captured { carrier, captor }` replacing the
abandonment for a completed non-friend load; a captive moved by the captor's docking, held, or
released; captivity created also by an ejection into a held cell (§4 C).

**Q-42-1 — The variant, and its transitions.** **(expensive to reverse)**
- **Proposed default:** `Captured { carrier: ObjId, captor: ObjId, since_ic: i64 }` under the
  existing `at` tag, `CREW_FORMAT` unchanged (a new variant decodes; old records have none).
  `carrier` is the hull the captive physically rides in; `captor` is a character (Q-42-4);
  `since_ic` is the capture's instant, the bound's origin (Q-42-8). **Into it:** (i) a non-friend's
  completed load (`tow.rs:2316–2321`'s second branch, Q-42-16); (ii) an ejection into a held cell
  (Q-42-12); (iii) a space hard-dock on a derelict with crew (Q-42-24). **Out of it:** (iv) the
  port handover and release (Q-42-6, Q-42-7) → `Port { port }`; (v) the captor's release at will
  (Q-42-9) → `Carrier { hull }` while docked or under way, `Stranded { site, unit: carrier }` on a
  site; (vi) the bound (Q-42-8) → an extraction → `Port`; (vii) the captor's carrier wrecked →
  `Stranded` in the wreck, as for anyone aboard (Q-42-15). Nothing else moves a captive.
- **Alternative:** `Captured` as a flag on `Carrier { hull }` (one variant, a `held_by` field).
- **Why:** the ruling names the variant; a flag on `Carrier` would make every reader of
  `Carrier` check it. Expensive because the record is persisted, read at boot, and matched in
  `check_in`, `crew_location`, `status_line`, `land`, `loaded_aboard`, `abandon` and `tick`:
  renaming or merging it later is a refactor across all of them.
- **Draft 0.3 (review round 2, 42-5 and 42-8): where a captive's body is.** A record that says
  `Captured { carrier }` while the character's object still sits in the casualty's seat is two
  truths. Nothing in the tree reconciles them: `aboard_loaded` and `abandoned_loaded` change only
  the record (`crew.rs:499–607`); only `land` vacates a seat, and only when it is given a unit
  (`:1015–1046`); a written-off rig goes through `tv::hand_back` → `place_interior(tv, shed)`
  (`tow.rs:2341–2344`, `port/tv.rs:4664–4683`), carrying its seated pilot to Occator's shed at the
  port. **The rule:** a capture is three steps in one pass of the load's physical step, before the
  casualty itself is placed or handed back: (1) for each soul the record holds in the casualty,
  `mush_sim::vacate_station` (`mush-sim/src/station.rs:191`) on every station they hold in it and
  `request_control_release` (`:449`) on its K6 control lease, as `land` does for an extraction; (2)
  the soul is moved into the captor's carrier's **bay room** — the room `+tv/bay in` enters — and
  unplaced from the site's domain if placed; (3) the record is written `Captured { carrier }`,
  where `carrier` is that carrier, never the casualty. The casualty then goes into the bay or back
  to the shed with nobody in it. From then on the captive's body and record agree: `inside` and
  `souls_aboard` find them in the captor's hull, `on_frames` cannot reach them (no seat), and no
  later move of the casualty moves them (42-3's note under Q-42-6). The same three steps run
  for a captured ejection's landing (Q-42-12) and, in space, for a hard-dock capture (Q-42-24).

**Q-42-2 — What a captive can and cannot do.**
- **Proposed default:** a captive is a passenger who cannot leave and cannot operate. While the
  record is `Captured`: every command that moves them (`go` and exits, `+tv/bay in|out`,
  `+ship/board|leave`, `+tv/<n> board|leave`, `+sortie/seat`), operates (`+tv/<n> …` acts,
  `+ship/*`, `+helm/*`, `+tac/*`, `+field/*`, `+bay/*`, `+salvage/*`, `+sortie/launch|recover|
  orders`, a cockpit keyword) or asks for extraction (`+sortie/extract`, until Q-42-8's bound)
  refuses with one line, "You are held aboard the {carrier}." Speech in the room, `look`,
  `+sortie/condition`, mail, pages and OOC channels work; detachment chat (`+sortie/chat`) does
  not, as the roster radio a captor would take. One gate, `crew::held(ctx) -> Option<ObjId>`, read
  at the head of each verb named.
- **Alternative:** no refusals beyond the physical ones (a captive inside a casualty in a bay has
  no exit today anyway) and a confinement room on the carrier later.
- **Why:** `+tv/<n> leave` would put a captive loose in the captor's bay room, and `+sortie/
  extract` would make captivity a six-hour inconvenience; both are one refusal each. A brig room
  is content a carrier definition does not have, and FO-12 needs no cell: the bound is the cell.
- **Draft 0.3 (review round 2, 42-6): the gate list, against the real verbs.** `+sortie/orders` is
  a read (`detachment/sortie/commands.rs:155–161` → `field_commands::report`) and comes off the
  list. Orders are the cockpit keyword `ORDER DT:…` (`match_/cockpit/orders.rs:383` →
  `detachment::commands::order`), already covered by "a cockpit keyword"; there is no `REPORT`.
  Added: the top-level responses `WILCO` and `NEGATIVE` (`detachment/commands.rs:161–189`, which
  check only `cockpit::flying`), `@station/*` (`mush-sim/src/commands.rs:3662`) and `+focus`
  (`:3642`). 42-5's vacate already makes `WILCO`/`NEGATIVE` refuse a captive in practice (no seat
  is flown); the gate states it, and covers the window before the vacate lands. Of the rest of
  `+sortie`: `/activate`, `/hire`, `/board`, `/contract`, `/seat`, `/launch`, `/recover` and
  `/cancel-launch` are gated (each commits a person or a unit to an operation); `/deactivate`,
  `/end`, `/history`, `/operation`, `/refresh`, `/orders` and `/plans` are not, so a captive can
  stand their own operation down and read its state. Draft 0.3's 42-5 and 42-2 add two more lines
  to the same gate: a pending forced extraction ("waiting for your pickup") and `Processing`
  (leaving the port only).

**Q-42-3 — Captivity across a restart and a later field.**
- **Proposed default:** the record is the authority. `Captured` is read at boot like any crew
  record; `check_in` leaves a `Captured` soul alone (the `abandoned` guard at `crew.rs:407–413`
  widened), so the captor's carrier checking in from a later field neither re-crews nor frees
  them; `crew_location` prefers a `Captured` record over the carrier row's `aboard` (`:226–239`);
  `on_frames` never reaches a captive (they sit in no seat). (Draft 0.3: they sit in no seat because the capture vacates it, 42-5 under Q-42-1.) A restart mid-capture resumes the
  bound from `since_ic`, in IC time, so a W-clock freeze pauses it.
- **Alternative:** re-derive custody from physical location at each check-in (whoever's rooms
  the character stands in holds them).
- **Why:** lane 5's row 4 already chose the record over re-derivation for the abandoned case, and
  a captive handed over at a port stands in a room nobody "holds".

**Q-42-4 — Who the captor is.**
- **Proposed default:** a character, never an organization: at a load, the carrier's operator
  (the same `holder` the hull's `SalvageCustody` names); at an ejection, the capturing party's
  leader (`parties::principals`' first), which for a fixture raider is its NPC leader; in space,
  the docking ship's commander. The captor's organizations are read from their affiliations when
  standing is written (Q-42-21).
- **Alternative:** the party's organizer, else its leader.
- **Why:** custody of the hull already names a character, the standing writes need a live subject
  (`record_standing` refuses a dead one), and a raider's leader is the only principal a fixture
  has.

**Q-42-5 — Hired NPC crew in a captured hull.**
- **Proposed default:** unchanged in this round: NPC crew keep no crew record and are never
  captured. When their hull's load into a non-friend's bay completes, or their seat's unit is
  captured at an ejection, the NPC's seat is vacated, their contract on the roster ends and the
  NPC object returns to its hire port's pool room (the `+sortie/hire` pool), told to the organizer:
  FO-12's NPC retrieval, applied to an NPC.
- **Alternative:** NPC crew get records and are captured, held and released like players.
- **Why:** doc 27 Q-R5 says NPC crew suffer injury like any character, but nothing reads an NPC's
  condition today and a captive nobody plays is a record nobody acts on; the pool return is the
  smallest thing that keeps the roster honest.
- **Draft 0.3 (review round 2, 42-4): two corrections.** (1) "or their seat's unit is captured
  at an ejection" cannot happen and is struck: a slot is one unit and one character
  (`detachment/activation/runtime.rs:343–346`), and NPC pilots never eject (Q-42-10). An NPC's exit
  is only ever at a load. (2) "Their contract ends" cannot be written where the load runs. The
  contract is K3 state in `tacsim.crew_contracts`, owned by `tacsim_crew_hire`
  (`detachment/crew/runtime.rs:14–16`); the NPC also holds a reservation in
  `tacsim.detachment_reservations` (`activation/runtime.rs:184–196`) that refuses any later
  activation with "A unit or person is already reserved." (`:393–396`), while `offers()` hides only
  contracted NPCs (`crew/mod.rs:157`) — so ending the contract alone would put a still-reserved NPC
  back on offer. The load's physical step (`place_in_bay`, `tow.rs:2296`) runs from `tow::tick`
  (`:2206`), which changes action state only by submitting an action, as it does the load's own
  finish (`TowOrder::Loaded`, `:2273–2280`). How the two records end is Q-42-26 (ruled in Draft 0.4).

**Q-42-26 — How a captured hull's NPC crew leave the roster.** (Draft 0.3, new; ruled in Draft 0.4.)
- **Decision (accepted default, 2026-11-03):** one authoritative K3 action that declares both families and, for the
  NPC, deletes the contract and clears the reservation in one bundle. The load's pass does the
  world half at once (42-5's vacate-and-release on the NPC's station, the NPC object moved to its
  hire port's pool room, the organizer told); the tow tick then submits the action as the
  organizer, by `Fire::as_principal` as the surface tick submits a plan's steps, keyed
  `tacsim.crew/<npc>/captured/<recovered_ic>`, and resubmits on each pass until the ledger shows it
  landed (the bay's owed-act precedent, `legacy-handlers.toml:53`). Until it lands the NPC stands
  in the pool room but is not on offer.
- **Alternative:** no action at the load: the contract and the reservation stand until the
  organizer deactivates or the operation closes; the NPC stands at the hire port, off the field
  roster, and is offered again only then.
- **Why:** the ruled outcome (Q-42-5) is that the contract ends and the NPC is back in the pool;
  the alternative is cheaper but leaves a hired NPC an organizer cannot use and nobody else can
  hire for as long as an operation stays open. The action's owner is whichever definition may
  declare both families under the action ledger's ownership rules; CR.1 names it.
- **Draft 0.4: the action's owner and key.** Both families are the `tacsim` module's
  (`OwnerId::new(crate::NS)`: `detachment/crew/runtime.rs:118`, `detachment/activation/runtime.rs:463`),
  and the activation definition already declares two families in one descriptor
  (`activation/runtime.rs:156`). So the action is a new definition of the `tacsim` module,
  `tacsim_crew_captured` (name provisional), registered beside `tacsim_crew_hire` in
  `detachment/crew/runtime.rs` with `"families":["tacsim.crew_contracts",
  "tacsim.detachment_reservations"]`; its idempotency key is `tacsim.crew/<npc>/captured/<recovered_ic>`,
  its principal the organizer (`Fire::as_principal`), its submitter the tow tick.

### B. The port handover

**What exists.** A voyage's arrival makes the berth `Berth::Docked` (`sim_routes.rs:151–161`);
no hook reads who is aboard then; `souls_aboard` exists for departures. Ports author
organizations, a workshop operator and a medical operator; none is "the authority". Fees post to
an operator's account; debts are obligations.

**What the ruling requires.** "A captive is handed to a port's authority at docking and released
for a fee, or held."

**Q-42-6 — The trigger and the authority.**
- **Proposed default:** the handover runs when the captor's hull docks at an IC port (the
  arrival in `sim_routes.rs:151–161` and a space `PortBerth` docking), never at a site's pad.
  Every `Captured` record whose `carrier` is that hull is handed over: the character is moved to
  the port's customs or arrivals room (the medical room's room where one exists, else the arrival
  room), told, and the record becomes `Port { port }` once released (Q-42-7). The authority is a
  new authored `PortDef.authority: CanonicalId`, one of the entry's organizations: Ceres
  `freeport_registry` (the Freeport's licensing authority; the Port Watch is its arm and not an
  org id), Stickney `stickney_yards` (the Republic's yard runs the Customs Post), Luna
  `orbital_access`, Deimos `voltaire_bond`, Titan `titan_concession`. The fee is posted to that
  organization's account.
- **Alternative:** the medical room's operator (already authored) is the authority; ports
  without one hand nobody over, and a captive rides on.
- **Why:** docking is the one moment every carrier passes through a port's hands, and the
  setting gives each port a governing body; reusing the medical operator would make the Co-op
  Ceres' gaoler.
- **Draft 0.3 (review round 2, 42-3, shared with doc 41's 41-3): custody when the hull moves.**
  [Doc 41](41-restoration-and-the-salvage-loop.md) Q-41-18/19 moves a vacated hull bay to bay (`+bay/stow <hull>=<carrier>`) and
  has a docking hook berth every `held_for` hull into the workshop; doc 36's GC.5 hands a
  written-off rig back to Occator's shed. Had a captive stayed inside the casualty, any of those
  moves would carry them off while the record still named the old carrier, whose next docking
  would then hand over someone no longer aboard. Two rules close it without a new ruling.
  (1) **A person never travels inside a hull a hook or a bay verb moves.** A capture already puts
  the captive in the captor's bay room (42-5 under Q-42-1). Every other authoritative move of a
  hull out of a bay or a site — doc 41's stow and workshop berth, a hand-back, an impound, doc 41's
  retrieval delivery — first runs 42-5's vacate-and-release on each person inside it and moves them
  into the room the hull leaves (the bay room, or the site's domain for a hull on the ground, as an
  extraction's start would leave them), and only then moves the hull. Each person's record follows
  their body: `Captured { carrier }` keeps naming the carrier they stand in, a friend's
  `Carrier { hull }` the same. (2) **At one docking, persons first.** When a hull docks, the crew
  handover (Q-42-6) runs before doc 41's `held_for` berth and any other hull hook of the same
  arrival, so no hull hook ever finds a captive aboard. Custody of a person therefore changes only
  by the transitions of Q-42-1, never as a side effect of custody of a hull. Doc 41 as merged
  agrees (its §1, round-2 item 41-3: "doc 41 moves no occupants"); what doc 41's `recovery` vacate
  does to a captive taken in that unlawful load is left to doc 42 there, and rule (1) answers it —
  the captive is already in the captor's bay room, so the vacate moves only the hull.

**Q-42-7 — The fee: who pays, how much, and what if they cannot.**
- **Proposed default:** the captive's own account pays `PortDef.release_fee_cr`, authored per
  port, provisional 1,500 cr everywhere (under the 2,500 cr prospecting licence; above a day's
  treatment is worth nothing, since treatment costs nothing). If the balance covers it the fee
  posts at the handover and the release follows after `[tacsim.field].release_s` (provisional
  3,600, one IC hour of processing); if not, the fee is written as an obligation to the authority
  (`mush_economy` obligations) and the release follows all the same, so no balance ever holds a
  character (FO-12: "funds … cannot remove the guaranteed return-to-play route"). Anyone may
  settle the obligation (their organization, their lessor, a friend). The release lands the
  character in the arrival room, stamps the port as the one they know (`ic_return::stamp`) and
  writes `Port`. The captor receives nothing from the fee.
- **Alternative:** held at the port until paid, with Q-42-8's bound as the ceiling; or a fixed
  authored bounty to the captor from the authority.
- **Why:** a payment to the captor is a ransom with a fixed price, which Q-38-22 rules out, and
  it would make capturing players pay; a fee that gates release contradicts FO-12. The obligation
  is the economy's own debt shape, and the fee is the "fee" the ruling names.
- **Draft 0.3 (review round 2, 42-2): the processing hour is a state of its own.** Draft 0.2 left
  the record `Captured { carrier, captor, since_ic }` for the `release_s` hour after the handover,
  so the `held` gate told someone standing in Arrivals "You are held aboard the {carrier}",
  `+ship/captives` still listed them, a carrier that undocked left the record naming a ship in
  space, and a second docking callback for the same hull would hand them over and charge them
  again. The build uses a fifth location: `CrewLocation::Processing { port, release_due_ic,
  charge }`, `charge` being the release obligation's id once created (`Option`, Q-42-7, 42-7's
  note). **The transition**
  `Captured { carrier: h } → Processing → Port` runs in the docking hook (`sim_routes.rs:151–161`)
  in one pass a captive: the move into the handover room, the obligation's creation and the
  record's write. It is **idempotent**: the hook selects only records that are still `Captured`
  with `carrier == h`, so a duplicate callback, a replayed arrival or a restart after the write
  finds nobody to hand over. The economy does not deduplicate a sanction's inspection key
  (`create_sanction_obligation` only bounds it, `api.rs:2193`), so the record is the only guard:
  the hook writes `Processing { charge: None }` first, then creates the obligation and fills
  `charge`; a pass that finds `Processing` with `charge: None` (a crash between the two) creates
  it then, and one that finds it filled creates nothing. The inspection key is
  `release:<captive>:<since_ic>`, one per capture, for the audit trail.
  The host's tick moves `Processing` to `Port` at `release_due_ic` (stamp, arrival room, told),
  restart-safe because the instant is on the record. `Processing` is not `Captured`: `held`
  passes it except for leaving the port ("You are being processed at {port}; about {h} IC h."),
  `+ship/captives` lists only `Captured`, `extractable` is `None`, treatment (GC.13) runs, and
  `status_line` shows the processing time left.
- **Draft 0.3 (review round 2, 42-7, kept in step with doc 41's 41-9): the fee's mechanism.**
  "Written as an obligation … anyone may settle" does not work on `main`. `create_obligation`
  (`mush-economy/src/api.rs:2045`) requires the caller to hold debit authority on the debtor's
  account (`:2094–2097`), and at docking the caller is the captor; a one-term obligation that
  misses goes `Delinquent`, which leaves the due pool (`obligation.rs:280`, `:308–314`) and is
  never attempted again; the only cure, `cure_service_obligation` (`api.rs:2302`), is for service
  obligations and also needs debit authority, so no third party can settle. The build uses:
  (1) **every** release fee, paid or not, is one `create_sanction_obligation` (`api.rs:2193`):
  one period due at the handover, debiting the captive's account under the **port authority's**
  live Org grant, not anyone's debit authority — the Registry fine's precedent (doc 17h M8d D3) —
  with `capability` a new `port.release_fee` held on the authority's scope and `levied_by` the
  authority's operator, paying the authority's levy sink. The sweep settles it at once when the
  account covers it; the record's `charge` names it either way (42-2). (2) **The settle path** is
  doc 41's, as merged (its §4.6 and RS.5, round-2 item 41-9): the engine's own cure,
  `reinstate_obligation` (`api.rs:2517`), which returns a `Delinquent` obligation to the due pool
  for one more attempt, called by doc 41's `+debts/pay <id>` (reinstate and settle in one post when
  the debtor's account covers it) and by port security's docking check. A release fee is one more
  row `+debts` lists; doc 42 adds no verb and no economy entry point. "Anyone may settle"
  (Q-42-7) is met the way doc 41 meets it for its debts: a payer moves credit into the captive's
  account and the captive's `+debts/pay` (or the next docking check) settles it; nothing debits a
  third party's account for someone else's debt. (3) The release never waits on the obligation
  (FO-12); nothing reads it but `+debts`, `+sortie/condition` and doc 41's crime consumer.

**Q-42-8 — The held case and its clock.**
- **Proposed default:** "held" is the captor's choice not to dock. It is bounded by doc 27 Q-F4's
  "cap on capture delaying return to play": `[tacsim.field].captive_bound_s`, provisional 604,800
  (seven IC days) from `since_ic`, IC time, paused by a freeze. Before the bound
  `+sortie/extract` refuses with the time left; after it, `+sortie/extract` works from captivity
  — the recovery service negotiates the release — and lands the captive at the safe port they
  know after `extract_s`, as any extraction does. Nothing is automatic (P-5: on request). The
  authority never holds a handed-over captive past Q-42-7's processing hour.
- **Alternative:** no bound; the captor holds until they dock or release.
- **Why:** Q-F4 promised the cap; seven IC days is seven real hours at the testing rate and a
  week at the real pace, long enough to matter and short enough that a captor who never docks
  cannot end a character's play.

**Q-42-9 — A captor who never docks, and the verbs.**
- **Proposed default:** two captor verbs on the hull's family: `+ship/captives` lists the
  captives aboard with their time to the bound, and `+ship/release <who>` releases one at will:
  docked, into the port's arrival room as `Port` with no fee; under way or on a site, as a
  passenger, `Carrier { hull }` on a hull not their own — and that is extractable (the `extractable`
  rule widened by one arm), so they ask for extraction or walk off at the next port. (Draft 0.3, 42-5 (C): the released character already stands in the carrier's bay room, so
  `+tv/bay out` and the hull's airlock are their way off at a port; the tick's `Carrier { .. } =>
  (None, None)` arm (`crew.rs:818–822`) is right for them, because no seat is left to vacate.) A captor who
  neither docks nor releases meets the bound (Q-42-8). A captive aboard a carrier that lifts is
  carried (no `leave=` for persons); one aboard a carrier that is wrecked is stranded in it
  (Q-42-15).
- **Alternative:** `+sortie/release`, beside `+sortie/extract`.
- **Why:** the captor acts as the hull's commander, and `+ship` is where the hull's occupants
  are already counted (`souls_aboard`, the bay gate).

### C. Ejection

**What exists.** A pilot stays in the hull until a check-in strands them or a frame incapacitates
them; `+sortie/extract` is theirs only once stranded or incapacitated; the withdraw's "under its
own power" test is read on the kernel's live cells (doc 32 §10). The kernel has an
"armed hostile inside my reach" test and the host knows each party's carrier.

**What the ruling requires.** "A forced extraction from a doomed unit with a capture chance when
a Hostile party holds the cell."

**Q-42-10 — The verb, and who may use it.**
- **Proposed default:** `+sortie/eject`, by the seated pilot of a doomed unit (Q-42-11), their own
  seat only, inside a running field or on the surface after one; refused for a hull that is not
  doomed ("#n can still bring you home"), at a remote console (nothing to eject from), while
  world time is frozen, and by an NPC pilot (doctrine never ejects in this round). Every unit
  definition may eject; a `can_eject` flag on definitions is left for the kit. The eject vacates
  the seat at once (the hull is the mind's, or nobody's, as `land` already does), writes
  `Stranded { site, unit }` with an `Extraction { forced: true }` due after `extract_s`, and no
  later frame reaches the pilot. Then Q-42-12 decides whether the extraction is a capture.
- **Alternative:** `EJECT` as a cockpit keyword beside `FIRE ALL`.
- **Why:** `+sortie/extract` and `/condition` are the crew verbs' home and the bot already types
  them; the keyword can alias later. The seat released now is what makes it an ejection rather
  than a request.

**Q-42-11 — When a unit is doomed.**
- **Proposed default:** the same test that would strand the pilot at check-in (`crew.rs:430–439`),
  read on the kernel's live cells at the verb (as the withdraw reads its own-power test): no
  power, no control, no motion, or `Wreck`/`Irrecoverable`; **or** a control section already
  `damaged` (the pilot is Injured and the next hit incapacitates). Every other state refuses.
- **Alternative:** only `Disabled`, `Wreck` and `Irrecoverable`.
- **Why:** a hull that cannot bring the pilot home loses the party nothing when its seat empties,
  and the damaged-section arm is what gives "eject now or stay" its edge.

**Q-42-12 — Holding the cell, and the chance.**
- **Proposed default:** at the eject, the host asks the field's arena task one read of its live
  state (an `ArenaMsg` query; no snapshot or hash change, the withdraw's precedent). A party P
  **holds the cell** when all four hold: (a) P's relation to the pilot's party is `Hostile`
  (`Relations::relation`, by mark or by fire); (b) a live unit of P, armed at check-out or having
  fired (`hostile_in_weapons_range`'s own armed test), has the doomed unit's cell inside its
  `fight_inner_cm`; (c) no live, armed unit of the pilot's own party or a Friend's has that cell
  inside its reach; (d) P has a carrier on the site (its entrants' `carrier` stands on the ground
  or its pad) to hold the captive. Several holders: the nearest unit's party. When held, the
  capture happens with `[tacsim.field].eject_capture_permille`, provisional 500, drawn
  deterministically (FNV-1a over the field's site tag, ordinal, the unit and the pulse, as
  `standing::cause_prefix` tags a cause), so a replay and a gate agree; a gate sets the knob to
  1,000 or 0. A captured pilot's forced extraction lands them aboard P's carrier as `Captured`
  (the "recovery service" being P's own pickup, after the same `extract_s`); an uncaptured one
  lands at the port, deferrable by a Friend on the site (Q-42-14).
  **Draft 0.3 (review round 2, 42-5 (B)): the ejectee between the eject and the landing.** The
  eject vacates the seat and releases the K6 lease at once (Q-42-10) but cannot put the pilot
  anywhere on foot (Q-CF-39), so the body stays in the cockpit room until the landing moves it.
  Three rules make that safe. (1) A record whose extraction is `forced` is skipped by every load
  rule — `loaded_aboard` (`crew.rs:530–533`) and the capture branch's filter (`:567–572`) match
  the record's `Stranded { unit }` today, so a load of the empty hull inside `extract_s` would
  otherwise carry off, or a Friend's load cancel, an ejection whose draw is already decided; the
  ejectee rides where the hull goes and lands where the extraction says. (2) The draw's outcome is
  kept on the record: `Extraction` gains `capture: Option<EjectCapture { party, carrier, captor
  }>` (omitted when `None`), written at the eject, so the six IC hours survive a restart. (3) The
  `held` gate (Q-42-2) also refuses movement and operation to a pilot whose extraction is `forced`
  ("You are waiting for your pickup."), so the body cannot walk off a hull it ejected from.
  **The landing**, at the due instant: when `capture` is set and its carrier still stands on the
  site and is not wrecked (`CarrierLoss::CrewStranded`), the three capture steps of 42-5 run into
  that carrier and the record becomes `Captured`; otherwise — the carrier lifted, left or fell —
  the extraction lands at the safe port they know, uncaptured. A director raider's carrier lifts
  once its hulls are aboard (`field/raiders.rs:800`, `lift` `:968`); `lift_check` therefore waits
  while a pending capture names its carrier, at most until that extraction's due instant, so story
  (d)'s capture lands before the lift. A player's carrier is not held: its captain may lift, and
  the ejectee then lands at a port.
- **Alternative:** certain when held, never otherwise (no draw).
- **Why:** the ruling says a chance; the kernel's no-dice rule (doc 16) is kept by a seeded draw;
  (d) answers where the captive physically goes without a brig on a tank; the reach test is the
  kernel's own, so "holds" means what the tempo already means by "engaged".
- **Draft 0.3 (review round 2, 42-1): who contests the cell.** Test (c) as written counts the
  ejecting hull itself: a hull doomed only by a damaged control section (Q-42-11) still has power,
  motion and a loaded mount, its reach is that mount's band (`fight_inner_cm`,
  `arena/weapons.rs:409`) measured from its own pose (`arena/state.rs:5670–5687`), so its own cell
  is at distance 0 and the cell is never held — story (d) would never draw. The rule the build
  uses: (c) reads **every unit of the pilot's own party or a Friend's except the ejecting unit**,
  and only those that are not `limits.wreck` (the filter `party_holds_armed_hostile_in_reach`
  already applies, `state.rs:5696`), armed at check-out or having fired, with a non-zero reach. An
  immobile or disabled friend that keeps a loaded mount and its sight still contests the cell: it
  can still fire on whoever comes for the pilot. A wreck does not. The same exclusion and filter
  apply to (b) for symmetry (a holder's wreck holds nothing). No ruling changes: this is what
  "holds the cell" meant, written down so the gate can pin it.

**Q-42-13 — The pilot's condition after.**
- **Proposed default:** unchanged by the eject: the condition the sections last gave (Fit, or
  Injured from a damaged control section) is kept; an ejection never incapacitates. The eject's
  price is the hull left to the field and the capture chance.
- **Alternative:** Injured at least, as the price of leaving.
- **Why:** doc 27 Q-H12 asks for "escape/ejection where equipment permits", not a wound; the
  treatment clocks are provisional and can carry the balance later.

### D. Rescue

**What exists.** A Friend's load is already a rescue (`aboard_loaded`); the deferral keys on the
recovery duty; a non-friend's load abandons; nothing frees a captive.

**What the ruling requires.** A captive is released by handover, by the captor, or by the bound;
Q-36-52's friendly half stands.

**Q-42-14 — A Friend's recovery of a stranded or ejected character.**
- **Proposed default:** the tow and load of the hull with the character aboard, by their own
  party or a Friend, is the rescue (as built: `Carrier { hull }`, extraction cancelled). The
  deferral in `rescue_on_site` keys on **any** open DT flight or leased rig of the character's own
  party or a Friend's standing on the site (`friends_of`), not on `CrewDuty::Recovery`; it applies
  to an asked-for and to a forced (ejected, uncaptured) extraction alike, never to a captured one.
  The recovery duty keeps its hire-terms meaning only.
- **Alternative:** keep the duty key.
- **Why:** Q-36-14 made the wingman the rescuer; an extraction that lands while the wingman is
  driving the casualty to the ramp empties the hull the wingman is saving.

**Q-42-15 — Rescue of a captive.**
- **Proposed default:** no fourth way is built: a captive is freed by the handover (Q-42-6), the
  captor's release (Q-42-9) or the bound (Q-42-8). No boarding, no raid on the captor's carrier.
  One consequence stands as is: a captor's carrier wrecked in a later field strands everyone
  aboard (Q-36-8), captives included, so they are `Stranded` in the wreck, extractable and
  towable with it like anyone else; custody ends with the hull.
- **Alternative:** at the captor's carrier's loss the capture is re-rolled under Q-42-12 for
  whoever then holds the cell.
- **Why:** Q-36-8's table is ruled and the wreck's crew are one case; a re-roll is a second
  mechanism for one event.

**Q-42-16 — Whose load captures.**
- **Proposed default:** as ruled: every completed non-friend load, Neutral or Hostile; a salvor
  who wants no captive releases them at will (Q-42-9) and pays nothing. The friendly half of
  Q-36-52 stands unchanged. A written-off lease rig's crew, abandoned today on the hand-back to
  Occator (GC.5), are captured by the loader like any other crew. (Draft 0.3, 42-8: the capture's vacate-and-move runs **before** `tv::hand_back`, so the pilot is
  in the loader's bay room and the empty rig goes to the shed.)
- **Alternative:** only a loader whose party held the crew's party `Hostile` captures; a Neutral
  loader's crew ride as passengers.
- **Why:** Q-38-22 says "a completed non-friend load"; the salvor's release makes the Neutral
  case one command long.
- **Draft 0.3 (review round 2, 42-9, advice): one crew branch for every load.** Today a load's
  physical step runs only outside a promoted field (`tow.rs:2252`, `if !promoted`), so every
  capture passes `place_in_bay`. [Doc 40](40-field-second-round.md) adds a ramp `+tv/<n> load` inside a running field with a
  trailer-follow tow; that load must call the same crew branch (`aboard_loaded` for a Friend's
  carrier, the capture of 42-5 otherwise) so this question holds inside a field too. **Draft 0.4:
  doc 40's Q-40-25 is ruled at its default (2026-11-03), which settles this:** a coupled pair
  leaves a running field by a coupled withdraw at the ramp (`+tv/<n> load [go]` on a promoted site,
  both checked in through the ordinary exit), and the host then runs the ordinary
  `TowOrder::Load`/`Loaded`. A capture inside a running field therefore passes the same
  `place_in_bay` and `captured_loaded` branch as one on open ground; doc 42 adds no field-side load. Doc 42 §5
  already rules licensed-mind hulls out of capture, so [doc 23](23-ic-sorties.md)'s foreign-load item is not this
  document's.

### E. Treatment

**What exists.** GC.13 (§3): a port room, three provisional times, a medical room's half time,
beds authored and not a limit; treatment runs only while the character stands in a port room.

**What the ruling requires.** Nothing new; this document carries doc 36 Q-36-45's open edge and
the captive's case.

**Q-42-17 — Beds as a limit.**
- **Proposed default:** beds limit the faster rate, never treatment itself: a medical room treats
  at most `beds` characters at `treat_medical_permille` at once, by `started_ic`; the rest are
  treated at the port's full time until a bed frees, when their remaining time is rescaled.
  `+sortie/condition` says "waiting for a bed".
- **Alternative:** beds stay a number with no effect.
- **Why:** the count is authored and the facility's slots already hold it (`medical.rs:40`); a
  limit that only slows is safe before players and makes the medical room scarce when a field's
  casualties come home together.

**Q-42-18 — Times and the medical room's effect.**
- **Proposed default:** GC.13's numbers stand (one IC day injured, three incapacitated, half at a
  medical room); no new tier.
- **Alternative:** an incapacitated character needs a medical room (a port without one only
  holds the condition).
- **Why:** the 2026-10-24 rule; nothing has been played at these numbers yet.

**Q-42-19 — An injured captive.**
- **Proposed default:** captivity never heals: aboard the captor's hull no treatment runs
  (GC.13's leaving rule, since the character is in no port room), and the bound's clock runs on.
  At the handover the character stands in the port's customs or arrivals room, so GC.13's tick
  begins their treatment at once under Q-42-17's beds; an incapacitated captive is handed over
  and released like any other. The captor is told a captive's condition in `+ship/captives`.
- **Alternative:** the captor's carrier counts as a treatment place at the full time (a sickbay).
- **Why:** GC.13's rule needs a port room and a carrier has none; a sickbay is doc 41's
  bay-away-from-port family and would reward holding a captive longer.

### F. The seams

**What exists.** §3: unit grants and bay rights name hulls; standing writes take a cause and
effects; the unlawful log has four kinds; the bot has no crew vocabulary; space has no crew state.

**Q-42-20 — Custody of a person as a grant kind.**
- **Proposed default:** no. Custody of a person is a fact of the crew record (`carrier`, `captor`,
  `since_ic`), never a right anyone issues or delegates; what a holder may do is the closed list
  of this document (carry, hand over, release, hold within the bound). Doc 26b's registry and
  the bay's `Right` stay about units.
- **Alternative:** a `person.custody` capability in mush-org's grant registry, delegable, so a
  captor could hand a captive to another hull.
- **Why:** doc 27 §4.5 keeps custody apart from rights; a transfer between captors is a prisoner
  market in all but name, which Q-38-22 rules out.

**Q-42-21 — Standing and faction effects.**
- **Proposed default:** one effect, written at the capture: when the captor's party did **not**
  hold the captive's party `Hostile` at the capture (a Neutral's crew carried off by a salvor, or
  an ejectee taken by a party that never marked or was fired on by them), one standing grant
  against the captor with each faction the captive openly stands in (GC.10's affiliations),
  `STANDING_CAPTURE` provisional −5, cause `capture:<site tag>:<ordinal>:<captive>`, not voidable.
  A capture of a Hostile's crew writes nothing (the fight's fire already moved standing at
  demotion); the handover and the release write nothing.
- **Alternative:** no standing effect until the stance table (doc 40, Q-38-23).
- **Why:** GC.10's machinery takes exactly this; an organization whose member is carried off by
  someone who was not their enemy should notice, and the number is provisional.

**Q-42-22 — The crime evidence row.**
- **Proposed default:** a new `UnlawfulKind::Capture` on the site record, written beside
  `Recovery` under its own condition: on owned ground, a load by neither the title holder's nor
  the landholder's authority that carries crew, and an ejection capture by any party but the
  landholder's own. The row names the captor and the captive (`subject` is the captive's object
  number). Evidence only; doc 41's consumer (Q-38-20) reads it.
- **Alternative:** no new kind; the `Recovery` row and the crew's own testimony suffice.
- **Why:** a person taken is a graver thing than a hull and the consumer will price kinds; the
  write is one line in the same bundle as the recovery's.

**Q-42-23 — The bot's vocabulary.**
- **Proposed default:** the captive's half and the captor's half are typed over SSH in the
  stories: `+sortie/condition`, `+sortie/eject`, `+sortie/extract` (refused, then allowed),
  `+ship/captives`, `+ship/release`, and the fee's line read at the handover; added to the field
  and escort lanes' vocabularies in the first cut (CR.0), beside whatever [doc 39](39-industry-production-chain.md)'s first cut did
  for the GC verbs doc 38 §4 K lists.
- **Alternative:** the harness types the player's half.
- **Why:** Q-38-29.

**Q-42-24 — Space.**
- **Proposed default:** the same record and rules, as the space track's own cut (CR.6, after
  doc 35b's SF.R9–R16): a ship's crew get a record from the space check-in
  (`check_in_space_unit`), a `DisabledDerelict`'s crew are `Stranded { site: the bubble's site,
  unit }` and extractable, a hard-dock on a derelict with crew by a non-friend (D-35-12) captures
  them as a load does, and a captive aboard a ship is handed over at the ship's `PortBerth` docking
  through the one hook of Q-42-6. Ejection in a bubble (a pod) is not designed here.
- **Alternative:** ground only; space keeps linkdead safety as its one crew rule.
- **Why:** Q-36-46/47/48 rule one path for ground and space, and today a pilot disabled in a
  bubble has no way home but the linkdead return.

### G. Acceptance

**Q-42-25 — The stories, and the one this document changes.**
- **Proposed default:** one story file, `crew_custody.rs`, grown cut by cut (doc 36 §7's
  "stories grow"), run on Ahuna Flank and Urvara Reach through `field_story.rs`'s `Ground` table,
  through production command paths, the bot typing both halves over SSH as a dispatch-only run
  (the G-4 precedent). Doc 36 story (a)'s non-friend variant,
  `gc5_a_non_friend_load_takes_custody_and_abandons_the_crew` (`field_gc5.rs:336`), and its
  review twin (`:511`) are rewritten to assert the capture and run at both sizes; GC.4's friendly
  load, GC.13's two gates and `crew_exit.rs` are unchanged.
- **Alternative:** one size a story, as GC did.
- **Why:** Q-38-29; the GC.5 gate is the only existing test that asserts the abandonment.

## 5. Consequences of the rulings

What the rulings change beyond their own questions. **Acknowledged by the owner on 2026-11-03**
with the defaults; each item is owned by the cut §7 names, CR.7 for the docs.

- **Doc 32.** Q-CF-38's "No treatment, no ejection, no capture in the first version" gets an
  "amended by doc 42" note: the location has five variants, ejection exists, capture exists.
  Q-CF-39 gains the forced extraction (an eject) and the captured landing. §10's "Not built in
  CF.5: ejection, capture and treatment" is struck through.
- **Doc 36.** Q-36-52's non-friend half is replaced (captured, not abandoned); its friendly half
  stands. Q-36-45 gains the beds rule. GC.5's "The abandoned crew" and lane 5's row 4 are noted
  as superseded by CR.1. The "What the programme still owes" entry "GC.13: beds are authored but
  not a limit" closes with CR.5.
- **Doc 27.** Q-H12's "a physical rescue/capture route" is now built; Q-F4's "cap on capture
  delaying return to play" is `captive_bound_s`.
- **Doc 38.** §4 F and Q-38-22 point here; Q-38-29's vocabulary list gains the crew verbs.
- **Help and code text.** `help/field.md:288` and the Crew section; `crew.rs:22`, `:36–38`;
  `LOAD_PREVIEW_FOREIGN` ("anyone still inside it is abandoned to extraction" → "is in your
  custody"); `ABANDONED`'s line; the `+sortie` and `+ship` notes in
  `mush-contract-tests/legacy-handlers.toml:334`, `:339` (new switches of existing verbs, the
  baseline does not move; the handover and the fee are the port tick's writes).
- **Config.** `[tacsim.field]` gains `captive_bound_s`, `release_s`, `eject_capture_permille`,
  each documented in `mush-config`'s table (`config.rs:111–115`'s rows).
- **Content.** `PortDef.authority` and `release_fee_cr` on each lane port entry; a lane version
  and a re-signed relay seed, as GC.13 moved them.
- **The field's records.** No kernel hash moves: the holding read is a query of live state, the
  eject's seat release is CF.5's `Released` control. The site record gains one `UnlawfulKind`
  (optional rows, byte-identical when empty, as GC.6's were).
- **Standing.** One new cause prefix, `capture:`, through `record_standing`.
- **Economy.** The fee is a post to the authority's account; an unpaid fee is an obligation.
  Nothing is minted.
- **Doc 23's custody note (S.4b)** is unaffected: a licensed-mind hull stands as a wall and is
  never a captor or a captive.

## 6. Acceptance sketch

Three stories, each at 256 (Ahuna Flank) and 512 (Urvara Reach), through tacsim's world harness
on the production command paths, then the bot over SSH for the parts a player types.

- **(a′) The captured crew** (doc 36 story (a)'s non-friend variant, rewritten). The captain flies
  the escort from inside it; a stranger's mark opens a field, which settles with the escort a
  casualty and the captain stranded in it. The stranger's rig tows the escort into the stranger's
  carrier. At the load the captain is `Captured { carrier, captor }`, told so, and `+sortie/
  extract` refuses with the time to the bound; the hull is in the stranger's custody with a claim
  owed, as GC.5 left it. The stranger lifts for Ceres and docks; the captain is handed to the
  Freeport Registry, stands in Arrivals, is charged the fee (paid; in a second run, unpaid and
  written as an obligation), released after the processing hour, stamped Ceres as their port,
  and under treatment in the medical room. The captor's `+ship/captives` empties.
  - A restart while captured keeps the record and the bound.
  - A second field closed by the captor's carrier with the captive aboard does not re-crew them.
- **(d) The ejection.** Doc 32's field story to the escort's fight; the escort's pilot ejects
  under the raider's guns with a damaged cockpit. With the knob at 1,000 the raider's party holds
  the cell and the pilot lands aboard the raider's carrier as `Captured`, captor the raider's
  leader; the raider withdraws and lifts (CF.6b) with the captive aboard; the bound passes
  (compressed) and `+sortie/extract` lands the pilot Injured at Stickney. With the knob at 0 the
  extraction is forced and uncaptured; the wingman's standing on the site defers it within the
  bound; the pilot lands Fit at the port. A variant with the wingman's unit inside its own reach
  of the cell: the cell is not held, no draw.
- **(e) The held captive and the wreck.** A captor who never docks: the bound passes on a site,
  `+sortie/extract` works. A captor's carrier wrecked in a later field (GC.3's loss) strands the
  captive in it; the captive extracts; custody is gone.

The bot's half: `+sortie/condition` at each stage, `+sortie/eject`, the refused and the allowed
`+sortie/extract`; on the captor's side `+ship/captives` and `+ship/release`.

## 7. Cuts CR.0–CR.7 (binding since the ruling of 2026-11-03)

Like doc 36's GC cuts: each runs through the production command paths, ships its own gate at both
sizes, and is recorded as built in a section of this document. A cut names the files it touches
and the questions it builds; two cuts that touch the same files never run at once. The order is
the owner's to change; the content of each cut is its rulings'.

**CR.0 — The record, the gate and the vocabulary.** Q-42-1, Q-42-2, Q-42-3, Q-42-23.
`CrewLocation::Captured`, `Extraction::forced`, the `check_in` and `crew_location` rules, the
`held` gate and its refusals in the verbs named, `status_line`; the bot's crew vocabulary; the
help and code text of §5; the contract census notes. Files: `field/crew.rs`, the verb files for
the gate, `help/field.md`, `mush-bot/src/mine/*.rs`, `legacy-handlers.toml`. Gate: in-source
decode and gate tests; no story yet. **Built** — see [42-cut-notes.md](42-cut-notes.md) §CR.0.

**CR.1 — Capture at the load.** Q-42-4, Q-42-5, Q-42-16, Q-42-21, Q-42-22. `tow.rs:2316–2321`'s
second branch calls `captured_loaded`; the NPC seat's return to the pool; Draft 0.3 (42-4): the NPC exit's K3 action (Q-42-26), adding `detachment/crew/runtime.rs` and
`detachment/activation/runtime.rs` to the files; Draft 0.4 (Q-42-26 ruled): the action is the
`tacsim` module's `tacsim_crew_captured` declaring both families, submitted by the tow tick as
the organizer (`Fire::as_principal`) and keyed `tacsim.crew/<npc>/captured/<recovered_ic>`, resubmitted
each pass until the ledger shows it landed; until then the NPC stands in its hire port's pool room
but is not on offer; the `Capture` evidence
row; the `capture:` standing grant; `+ship/captives` and `+ship/release`. Files: `surface/tow.rs`,
`field/crew.rs`, `surface/claim.rs`, `port/voyage.rs` or `port/ship.rs` for the verbs. Story:
(a′) to the load, at both sizes; the GC.5 gate and its twin rewritten. **Built** — see
[42-cut-notes.md](42-cut-notes.md) §CR.1.

**CR.2 — The port handover and release.** Q-42-6, Q-42-7, Q-42-19. `PortDef.authority`,
`release_fee_cr`, `release_s`; the docking hook (`sim_routes.rs:151–161`); the fee post and the
obligation; Draft 0.3 (42-7): the fee is `create_sanction_obligation` under a `port.release_fee` grant
authored for each authority, settled through doc 41 RS.5's `+debts/pay` (CR.2 waits on RS.5 or
lands after it); the release into the arrival room, the stamp, treatment picking them up. Files:
`port/def.rs`, the lane content (version bump), `composition/sim_routes.rs`, `field/crew.rs`.
Story: (a′) whole, both sizes, the bot typing the captive's half.

**CR.3 — The bound and the held case.** Q-42-8, Q-42-9, Q-42-15. `captive_bound_s`;
`+sortie/extract` from captivity after the bound; the release at will's two landings; the
captor's wreck. Files: `field/crew.rs`, `mush-config`. Story: (e), both sizes, with a restart.

**CR.4 — Ejection.** Q-42-10, Q-42-11, Q-42-12, Q-42-13, Q-42-14. `+sortie/eject`; the doomed
test on live cells; the holding query to the arena task; the deterministic draw and
`eject_capture_permille`; the forced extraction's two landings; the deferral keyed on Friends
rather than the duty. Files: `detachment/sortie/commands.rs`, `field/crew.rs`, `arena_bridge.rs`,
the arena task's message handler (a read; no hash). Draft 0.3 (42-5): also `field/raiders.rs` (`lift_check` waits on a pending capture naming its carrier). Draft 0.4 (Q-40-25 ruled): the coupled-casualty crew rule reads doc 40's coupled withdraw at the ramp, after which the ordinary `TowOrder::Load`/`Loaded` runs, so a casualty recovered from a running field reaches the same `place_in_bay`/`captured_loaded` branch as CR.1's. Story: (d), both sizes, knob at 1,000 and 0.
After CR.1 (both touch `crew.rs`).

**CR.5 — Beds as a limit.** Q-42-17, Q-42-18. `treat` counts the medical room's occupants
against `beds`. Files: `field/crew.rs`, `port/medical.rs`. Gate: GC.13's gates plus a third
casualty waiting for a bed at Ceres. After CR.4.

**CR.6 — The space seam.** Q-42-24. The space track's cut, in its tree, after SF.R9–R16: crew
records from the space check-in, the derelict's crew, the hard-dock capture, the ship's handover
through CR.2's hook. Not this train's.

**CR.7 — Docs.** §5's amendments to docs 27, 32, 36 and 38; `42-cut-notes.md`; Draft 0.2 of this
document with the rulings and the as-built sections.

**Order.** CR.0 → CR.1 → CR.2 → CR.3, one writer, since every cut touches `crew.rs`; CR.4 after
CR.1; CR.5 last; CR.6 whenever the space track reaches it; CR.7 rides each PR. Nothing here
consumes doc 39's lots or doc 41's facilities. **CR.0 may start now**; nothing else runs beside it,
because every later cut edits `crew.rs` after it.

### 7a. What the first cuts author

The names and numbers the rulings leave to the builder, fixed here so the stories can pin them.
Every number is provisional under the 2026-10-24 rule.

- **Config** (`[tacsim.field]`, documented in `mush-config`'s table beside `extract_s`):
  `captive_bound_s` 604,800 (60 to 2,592,000; Q-42-8); `release_s` 3,600 (60 to 86,400; Q-42-7);
  `eject_capture_permille` 500 (0 to 1,000; Q-42-12).
- **Content** (each lane port entry; lane version and relay seed moved as GC.13 moved them):
  `authority` — Ceres `freeport_registry`, Stickney `stickney_yards`, Luna `orbital_access`,
  Deimos `voltaire_bond`, Titan `titan_concession`; `release_fee_cr` 1,500 at every port (Q-42-6,
  Q-42-7). The handover room is the medical room where one exists, else the arrival room.
- **Constants:** `STANDING_CAPTURE` −5 with the cause prefix `capture:` (Q-42-21);
  `UnlawfulKind::Capture` (Q-42-22).
- **The record:** `CrewLocation::Captured { carrier, captor, since_ic }` under the `at` tag, and `Processing { port, release_due_ic, charge }` (Draft 0.3, 42-2),
  `Extraction::forced` omitted when false, `CREW_FORMAT` unchanged (Q-42-1); Draft 0.3 adds
  `Extraction::capture: Option<EjectCapture { party, carrier, captor }>` (42-5) and the
  `Processing` location (42-2), both CR.0's to decode and display.
- **The NPC exit's action** (Q-42-26, Draft 0.4): `tacsim_crew_captured` (name provisional), owned by
  the `tacsim` module, declaring `tacsim.crew_contracts` and `tacsim.detachment_reservations`;
  idempotency key `tacsim.crew/<npc>/captured/<recovered_ic>`; principal the organizer
  (`Fire::as_principal`); submitted by the tow tick each pass until it lands.
- **Verbs:** `+sortie/eject` (Q-42-10); `+ship/captives`, `+ship/release <who>` (Q-42-9); the
  `held` refusal on the verbs Q-42-2 lists, one line: "You are held aboard the {carrier}."
- **Lines** (the builder's wording, in the house voice): the capture at a load, the capture at an
  eject, the uncaptured eject, the handover with the fee paid or owed, the release, the bound
  reached, the captor's list.
- **Stories:** `tests/shard_1/crew_custody.rs`, grown by CR.1–CR.5, on both grounds of
  `field_story.rs`'s `Ground` table; the GC.5 gate and its review twin rewritten in CR.1.

## 8. What the cuts still wait on

The rulings are in (§4). What a builder must know before each cut starts:

- The fix thread's state on the rig cab (`inside` reading a cab as outside the rig, §3a), since
  CR.1's capture reaches a cab-seated pilot only once that lands; until then the stories seat the
  captive through a DT flight.
- Doc 39's first cut's vocabulary work, so CR.0 adds to it rather than beside it.
- Whether doc 40's one rebless is scheduled before CR.4: the holding query is a read and needs no
  hash change, but if the kernel is reblessed anyway the eject's seat release could become a
  kernel message rather than CF.5's `Released` control.
- The space track's plan for SF.R9–R16, so CR.6 has a place.
- **Future (owner, 2026-11-04):** surrender by circumstance and personality. A raider whose carrier
  is wrecked fights to the death for now (no life support to scatter or wait for a pickup, and no
  surrender yet); a later cut may let crews surrender into custody instead (doc 41 RS.1's note on
  `gc11`).

**Defaults the survey held with least confidence**, ruled as written and worth a second look
after the first play: Q-42-7 (who pays and what a fee is for when it cannot gate release; the
obligation shape is new to crew), Q-42-8 (the bound's length against an unruled play rate),
Q-42-12 (a seeded draw against a kernel that has never rolled; "holds the cell" by reach), Q-42-16
(every non-friend load captures, Neutral salvors included), Q-42-21 (a standing hit for capturing
a non-Hostile, before the stance table exists).
