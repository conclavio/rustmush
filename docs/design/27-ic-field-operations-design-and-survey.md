# 27. IC field operations — detachments, combat, and salvage

**Draft 0.2, 2026-10-28. Owner decisions recorded; implementation not yet built.**
Based on `origin/main` at `2cbe9f7c264c79ee6ce6790a3c5ad420a30bec3b` and the owner's
2026-10-28 rulings. This is the shared brief for doc 26 DT.1–DT.3, backlog H1 and H2.
Rechecked against `origin/main` at `ed245516` before publication on the same date.
The owner accepted all 39 survey defaults with the eight amendments recorded below. §7 is the
decision record; §9 contains new follow-up questions, which are not covered by that acceptance.

## 1. Shared goal and authority of this draft

A player aboard a carrier selects a saved detachment plan, assigns physically embarked units
that the participants may use, and launches a sortie. Humans and AI-controlled units occupy the
same roster. The first active player in slot order commands it, with leadership automatically
returning to an earlier slot when that player becomes active again. The group can encounter another force on
real ground, withdraw, leave disabled units behind, and return with survivors. A recovery crew
can subsequently reach a disabled unit, attach a tow, move it and load it into a carrier. The
recovered unit has precisely the damage and surviving equipment it had in the field. Physical
possession, lawful recovery rights and legal title remain separately recorded.

This is one persistent world lifecycle, divided into three workstreams:

| Stream | Owns | Delivers to the others |
|---|---|---|
| **DT.1–DT.3: detachments** | Plans/presets, live roster, accepted human assignments, command and launch coordination | Named units, authorized operators, roster revision, command principal and launch intent |
| **H1: IC field combat** | Field authority, combat entry/exit, durable physical outcomes and observer projections | Exact unit condition and position, causal outcome records, custody handoff, observation evidence |
| **H2: physical recovery and salvage** | Inspection, attachment, towing/dragging, loading, recovery custody and salvage-claim handoff | The same damaged unit in a new physical location, recovery provenance and legal claim/evidence |

The explicit post-H2 milestone, H3, owns stripping/scrapping together with restoration: repair
work, replacements, rearming, resources, facilities and their prices.
H2 must preserve enough information for H3; it does not restore a unit as a side effect of loading.
Full infantry, orbital fighting and facility sieges are not silently included in this first ground
slice. Their exclusion is a proposed delivery scope, not a claim that the simulator forbids them.

The owner's explicit rulings in §2 and accepted decisions in §7 supersede conflicting earlier
proposals. Existing accepted world rules in §3 continue except where these decisions amend them.
§§4–6 express the common design and delivery plan; exact schemas, cut sizes and implementation
briefs still need engineering review. Acceptance of the design does not claim any code has shipped.

## 2. Owner rulings recorded on 2026-10-28

| ID | Settled direction |
|---|---|
| FO-1 | A disabled unit remains on the field in the state that disabled it. If nobody retrieves it, its operator has lost it; recovery never restores its condition automatically. |
| FO-2 | A headshot biped can become usable by replacing the head and cockpit components; a destroyed engine needs replacement. Destroyed weapons are normally slag; disabled or jammed weapons may be repairable. Surviving equipment must not disappear merely because the unit is defeated. |
| FO-3 | Salvage is a separate physical operation: tow or drag the unit back to a ship and load it. |
| FO-4 | On unclaimed/unowned ground anyone capable of recovery may salvage a disabled/destroyed unit. On owned ground BOTH the unit owner and landholder are legally entitled to recover it; neither needs the other's permission. Others may physically take it unlawfully; discovery/evidence, rather than an omniscient prohibition, makes enforcement possible. Title transfer follows Q-S2/Q-S13/Q-S15. |
| FO-5 | Players can keep multiple detachment presets, choose one for a mission and change the plan before launch. |
| FO-6 | Squad size defaults to six. Tanks/bipeds occupy one slot each; future battle armor/infantry occupy a slot as one command group (for example six armor troopers or eighteen rifle infantry). Humans may occupy any slot. Leadership belongs to the first active player in slot order, automatically falling through on death/incapacitation, disconnect or unit disablement and returning when an earlier player becomes active again. With no active player there is no leader: units continue their last order or act for themselves. An order is an order to both humans and AIs. |
| FO-7 | AI assignments are dynamic. The physical units must have embarked; an AI grouping does not have to have been declared or embarked at the port. `+sortie` is an appropriate player-facing verb family. |
| FO-8 | The carrier's charter owner, detachment organizer and detachment leader need not be the same person, and the charter owner need not accompany the sortie. Any player aboard may form a detachment from units they have permission to use. |
| FO-9 | Personal ownership inherently grants use, and the owner may permit other players to use their units. Organization-owned units support organization-wide and individual-unit permissions. Granting permissions is itself permissioned and cannot convey more authority than the grantor holds. |
| FO-10 | The OOC simulators are combat-code proving grounds and consequence-free practice. Their lobby, reset, team, allocation and lifecycle conventions do not define IC world rules. |
| FO-11 | After an outage, preserve committed state and resume without unseen shooting/towing catch-up. The IC world stays globally frozen for a five-minute recovery countdown, broadcasting at five, two and one minutes and again at unfreeze. Players can reconnect and get situated before movement/combat and other world activity resume. |
| FO-12 | There is no player permadeath. The worst-case recovery route is retrieval by a third-party NPC organization to a known port such as Phobos for recuperation. The player may later return in another owned vessel, a newly acquired coop unit or passage aboard another player's vessel to recover/salvage the original unit. Crew retrieval never retrieves or repairs that unit automatically. |
| FO-13 | Any mobile unit may tow one disabled unit of up to twice its own tonnage. Speed is reduced by the tonnage differential; a twenty-ton tug dragging forty tons moves very slowly. Recovery-optimized vehicles remain useful. Towing is physical, and does not bypass terrain or loading capacity. |
| FO-14 | H3 is an explicit post-H2 milestone for component stripping, scrapping and repair/restoration together. H2 still delivers whole-hull recovery in its damaged state. |

The owner accepted the remaining defaults in §7 as written. The examples of six battle-armor
troopers and eighteen rifle infantry illustrate group representation, not universal authored
strengths. No NPC squad leader or permadeath exception is implied by any earlier wording.

## 3. Existing world decisions to retain, rather than ask again

[Doc 10](10-tacsim-foundations.md) decisions 3A, 5A, 6A, T2 and T3 already establish:

- Disablement, capture, salvage and permanent destruction are different outcomes. Physical
  recovery needs access, capacity and transport. Q-H12 further rules out player permadeath entirely.
- Frontier deployed mobile assets are exposed; core hostility needs war, sanction, an accepted
  scene or an explicit crime. Docked/vaulted assets and permanent facilities have their own
  protections; a field battle is not permission to bypass them.
- A scene must not hold or delay a scheduled journey. Arrival and its time floor must be handled
  by the time design, including arrivals during combat.
- A combat pulse does not post economic transactions. Typed causal facts cross to the economy
  through authoritative boundaries.
- Taking possession does not automatically transfer title. T3's existing legal routes include
  public salvage claims with a contest window, voluntary transfer/abandonment, forfeiture and
  insurer acquisition. FO-4 settles recovery entitlement; Q-S2 asks whether to retain T3's title
  procedure for it.

[Doc 08](08-economy-organizations-ownership.md) separates title, custody, operation and maintenance.
Its organization and economy implementations already exist; doc 26's old suggestion that those
systems must first be built was stale. Personal unit grants and the required delegation semantics
still need an integration design.

[Doc 23](23-ic-sorties.md) supplies hull epochs and durable logs. A player's sortie groups hull
activity; one engagement may intersect multiple sorties and unrostered units. Neither owns the
other's lifecycle. The word `sortie` is now allowed for the player's
operation, while the technical `SortieId` continues to identify a hull's seal epoch.

## 4. Shared contracts before parallel implementation

These are conceptual records and ownership boundaries, not a mandate for new crates or duplicated
databases. Reuse the existing owning modules where their semantics fit; document any schema change.

### 4.1 The unit survives a change of activity

One stable unit/asset identity connects its physical placement, installed equipment, damage,
operator, custody, title and provenance. A preset references it; it does not clone it. An engagement
admits it; it does not instantiate a fresh stock unit. Recovery moves it; it does not convert it
into a pristine item or a money reward.

The authoritative condition includes surviving sections and facets, installed component identities
and condition, ammunition, cargo, remaining capabilities and any persistent hazards. A separate
operational assessment says what it can currently do. `disabled`, `immobile`, `unarmed`, `wreck`
and `irrecoverable` are not interchangeable legal or physical states. An engine failure need not
destroy the gun; a headshot must not erase intact limbs or weapons. Temporary combat-only state
such as target locks is explicitly classified at handoff; dropping a lock must never clear damage.

H1 must audit the existing arena defeat consequences and write-back, not merely rename `wreck`.
H3 will distinguish repairing an extant component from installing a replacement for a destroyed or
missing one. The first H1/H2 integration must round-trip this distinction even before H3 is built.

### 4.2 Plans, active rosters and principals are separate

| Concept | Agreed meaning |
|---|---|
| Preset | A reusable, revisioned player plan; no reservation or permission grant by itself |
| Active detachment | A roster resolved to real units and accepted pilots for one operation |
| Player sortie | The operation's identity and history, including multiple hull launches and recoveries |
| Hull epoch | Doc 23's seal interval; membership changes neither open nor close it |
| Engagement | An interval of tactical authority over particular field units and terrain |
| Organizer | The person editing/forming the plan; need not hold slot 1 or own the carrier |
| Leader | First active player by ascending slot number; automatically recomputed, with no NPC fallback leader (Q-D4) |
| Operator | The character actually holding a unit's station/control channels |
| Owner / custodian / payer | Separately resolved economic and physical principals; leadership grants none of these automatically |

The six-slot interpretation, activation/freeze rules and succession are settled below. A human
in any slot remains a human actor: a delivered order never moves their unit
or asserts their consent. AI reassignment on an embarked hull still respects the mind's licence,
installed core, seal, compute admission and control rules. Dynamic AI does not mean free hardware,
an unlimited licence or an in-flight reseal.

### 4.3 Permission to use is not possession or legality

Resolve an operation against the actual actor, unit and permission scope. Personal ownership is a
root of authority; an organization is another. Organization-wide grants must cover the appropriate
controlled assets, with individual-unit grants and exclusions resolved explicitly. Q-A1–Q-A4 define
the details. Grant delegation must narrow or preserve capability, asset scope, term and delegation
authority, never widen them. A rank number alone cannot establish that containment.

Planning cannot mint permission. Preview, reservation, launch, new station acquisition and new
command admission check their relevant authority. Record the principal and permission basis at
each committed act. Revocation has prospective effects defined in Q-A3; it never teleports a unit,
changes its title or repairs it. Existing cockpit leases need an explicit transition policy.

For salvage, distinguish permission to operate the **recovery vehicle**, physical ability to
secure the **target**, and legal entitlement to remove it. Missing target-owner permission must
not globally block the physically possible illegal recovery required by FO-4. Defenses, terrain,
equipment and active resistance can stop it. The law/evidence path records what makes it unlawful
without exposing hidden owners or creating automatic public knowledge.

### 4.4 One physical writer, durable handoffs

The field host, an engagement and a recovery operation may each need authority over a unit's
motion, but never simultaneously. A versioned handoff names unit, prior owner of simulation
authority, receiving owner, epoch, state version and causal operation. Reservation is not yet
transfer; transfer is not yet completion. Retry or restart consumes an operation identity once.

H1 receives exact state and places the unit where it actually enters the fight. It returns exact
state and location to the persistent field. H2 obtains towing control over a target through a
physical attachment, coordinates the tug/target pair with H1 if combat is active, then transfers
placement into a carrier only at completed loading. Neither path writes through a stale mirror.
Payload cargo, unit title and major-component title stay with their existing authorities.

There must be no IC equivalent of restoring the checkout snapshot on a voided practice match.
Acknowledged damage, spent ammunition, recovery, cargo and economic outcomes cannot be rewound
or duplicated by restart. If an incompatible version cannot safely resume, retain/quarantine the
last authoritative state for an audited migration/recovery; do not respawn the unit.

### 4.5 Recovery law and knowledge

Record physical custody separately from the ground-right holder, legal unit title and salvage
claim. Unit owner and landholder have concurrent recovery rights; lawful self-recovery restores
custody without requiring a new title claim. A non-owner's lawful recovery may support a title
claim under Q-S2/Q-S13, but possession alone never transfers it. A recovery record carries location,
relevant claim/tenure, jurisdiction, time, actors,
physical steps and resulting custody. An unlawful recovery can complete physically while its
legal claim is invalid or disputed. Ordinary asset use grants are not silently awarded to a thief;
captured-equipment commissioning remains a separate legal/physical control transition.

The authoritative record is not an omniscient notification service. Players, NPCs, claimants and
law enforcement learn through their own observations, delivered reports, inspection or recovered
records. End of combat does not publish an objective IC replay or an exact enemy inventory.
Unseen unlawful salvage must neither broadcast a thief nor reveal a wreck through a refusal.

### 4.6 Clock and activity boundary

Q-H1 is accepted: human-paced field motion/combat alongside the rate-24 campaign. Its adapter
must specify tactical motion time, campaign timestamps, arrivals, reinforcements, communication
delivery, tow work, licence/contract deadlines and interrupted mining. Entering or leaving a fight
must not yield duplicated production, free towing, expired events replayed in the past, or a
journey held until a convenient match result. Coarse simulation may sleep; its durable state and
scheduled obligations do not cease to exist.

Q-H11 creates a global outage exception: freeze campaign and tactical progression and all timed
IC work during recovery/countdown, then resume from the checkpoint without outage catch-up.
Journey durations and event ordering remain intact; a single local engagement still cannot hold
another player's journey. The five-minute countdown runs on real elapsed time after consistent
recovery is ready. The treatment of explicitly real-time legal windows is the new Q-F3 in §9.

## 5. Delivery boundaries and dependency order

**Shared policy decisions are recorded.** Pin the records above with their existing owners,
resolve the affected follow-ups in §9, then write implementation briefs for all three streams.
This documentation task does not start their code implementation.

| Cut | Concrete result | Dependency / proof |
|---|---|---|
| DT.1 | Common roster/order contract; human/AI delivery; simulator adapter for exercising it | Slot meaning, leader/succession and order-delivery policy agreed; IC semantics own the model |
| DT.2 | Multiple presets, editable resolved plans, consent, permission checks and unit reservations aboard a carrier | Shared asset authority contract; no charter-holder shortcut; DT.1 records |
| DT.3 | Preflight, per-unit launch/recover reconciliation, durable sortie membership/history and restart behavior | DT.2; existing S.1–S.3 epoch hosts; any required actual IC-mind host for AI-controlled hulls |
| H1-A | Persistent field-unit and component state, defeat/epoch classification and crash-safe handoff | Shared unit/authority contract; can develop alongside DT.1/DT.2 |
| H1-B | Entry, reinforcement, withdrawal and demotion on actual ground under the chosen clock/risk rules | H1-A; shared global freeze/resume support (Q-H11); first slice's field controls, crew outcome and observer projection; DT.3 for the group story |
| H1-C | Real sortie fight: casualties remain where they fell; survivors leave with exact spent/damaged state | H1-B and integrated DT.3; no OOC reset path |
| H2-A | Universal tow coupling, optimized recovery equipment, inspection/attachment, movement/loading capacity, durable recovery operation | Shared state/handoff/clock contracts; can develop alongside H1 using genuine damaged-state fixtures |
| H2-B | Recovery entitlement, evidence and existing title/contest workflow integration | Ground-right rule and Q-S2–Q-S4; current economy/claim owners |
| H2-C | Separate recovery sortie retrieves a casualty produced by H1; lawful and unlawful variants | H1-C, H2-A/B; exact component/cargo/provenance preservation across restart |
| H3 (post-H2) | Component stripping, scrapping and workshop repair/restoration; compatible replacement parts, resources, work time and resulting capability | H2-C and its preserved manifests/provenance; title/use authority for destructive work; minimal restoration before broad economic playtesting |

The H1/H2 sublabels are planning proposals, not historical landed cuts. S.4/S.5 remains the
first IC-mind integration path from doc 23: use that contract when a licensed mind flies IC;
do not claim coreless epoch bookkeeping already supplies it. M8's broader procurement/contraband
programme is not a blanket prerequisite for human-operated combat or towing. Conversely, a
test-only permit is not sufficient production authorization for an AI unit.

H1 coordinates the global freeze/resume integration with the existing clock and server owners;
this shared prerequisite must cover all clock readers before live H1/H2 acceptance. Pausing one
combat loop or setting a rate without auditing legacy readers is insufficient. It does not require
DT.1's roster work or H2's state/coupling work to wait.

**Three streams can proceed after shared decisions, with these integration edges:**

```text
shared unit, authority, clock and legal contracts
  +-- DT.1 -> DT.2 -> DT.3 ----------------------+
  +-- H1-A -> H1-B ---------------------------> H1-C --+
  +-- H2-A + H2-B ---------------------------------> H2-C -> H3 strip/scrap/repair
```

Do not require a new 10 km map, public replay, a two-team lobby, a fixed arena unit cap, a
particular practice mode, or pre-dealt AI squads merely because the simulator uses them. First
slice capacity is an explicit measured admission budget, and refused admission must not delete,
duplicate or expose a real unit.

## 6. Shared acceptance story

1. The carrier owner stays at port. Another embarked player loads one of two saved presets.
   It includes their own biped, a personally lent unit and organization units granted through
   both unit-specific and scoped wildcard authority. A missing/revoked grant is reported before
   launch. Changing the live plan does not silently rewrite either preset.
2. A second human accepts a later slot. The first active player leads, initially in slot 1. AI control is
   assigned to embarked hulls after arrival at the field; no AI squad had to embark at port.
   Launch records each unit once, its actual principal, active roster revision and hull epoch.
3. The leader issues the same `ORDER` to a human and an AI. The human can acknowledge or decline
   without involuntary helm/fire changes; the AI attempts it under its actual sensors and limits.
   Loss of contact produces uncertainty rather than a false completed/acknowledged state.
   Disable/disconnect the leader: the next active player's slot takes command without swapping
   units. Reactivate/reconnect the earlier player: command returns. With no active players,
   there is no leader, and units continue orders or their available autonomous behavior.
4. A lawful frontier encounter disables a biped by a head/cockpit hit and another unit by engine
   damage. One weapon is destroyed and another jammed. Surviving equipment remains identifiable.
   The rest of the force withdraws. The field keeps exact casualties and their positions; a
   winner, logout, empty map or restart does not reset them.
5. A later recovery sortie reaches a casualty, attaches a mobile tug, tows it to the
   ship and loads it within real capacity. Restart once while attached and once during loading:
   no duplicate unit, cargo, reservation, title claim or completion event results.
   Prove both a recovery-optimized vehicle and a twenty-ton ordinary unit slowly dragging a
   forty-ton casualty. The crew's NPC rescue to port neither moves nor repairs that casualty.
6. Repeat on unclaimed ground, as the unit owner on another party's land, as that landholder or
   its authorized recovery crew, and as an unrelated unpermitted party. All physically feasible operations can occur; only the
   lawful cases qualify for the agreed salvage entitlement. The unlawful case is not publicly
   exposed until a witness/report/inspection yields evidence. Claim completion follows Q-S2.
7. At the bay, inspect the same damaged head/cockpit, engine, jammed gun, destroyed gun, remaining
   ammo and cargo. Nothing healed. H3 later replaces or repairs the appropriate parts and
   recomputes capability from the resulting equipment state; H2 does not pretend H3 has shipped.
8. Interrupt a live encounter with a server outage. Preserve every committed outcome; globally
   freeze the IC world through recovery and the five-minute countdown. Broadcast at five, two,
   one minutes and resume, including current freeze status for reconnecting players. No shooting,
   towing, mining or travel completes invisibly during the freeze. Resume with one consistent clock.

These stories use ordinary commands and production composition. Supporting gates cover stale
roster/permission races, competing recovery attempts, membership changes during combat, unknown
contacts and failure at every durable authority transfer. The OOC simulator separately proves
its practice reset cannot reach the IC asset/claim paths.

## 7. Owner survey

**Answered 2026-10-28:** all 39 questions are settled. The owner amended Q-D1, Q-D4, Q-H11,
Q-H12, Q-S1, Q-S6, Q-S11 and Q-S12 and accepted every other default as written. Each decision
below records the resulting policy; alternatives remain for rationale/history, not as active
choices. New questions in §9 were generated by this review and remain unanswered.

### A. Detachment shape and mission planning

**Q-D1 — What does the six-slot squad contain?**
- **Decision (owner ruling):** six command units. A tank or biped occupies one slot. Future
  battle armor and infantry occupy a slot as one group, for example six armor troopers or
  eighteen rifle infantry, functioning as one unit for orders and other command purposes.
  Human or AI operation does not change that representation. Actual constituent mass, casualties,
  equipment and capacity still count; the examples do not fix future group strengths.
- **Alternative:** one individual vehicle or trooper per slot for every unit type.
- **Why:** a single rifleman is not a useful peer of a tank in this command interface. A group
  supplies the useful command scale without turning a tank slot into an arbitrary vehicle squad.

**Q-D2 — What does a saved preset remember, and when does it reserve equipment?**
- **Decision (accepted default):** a named revisioned plan of preferred units, roles, pilots and AI profiles. Loading
  creates a separately editable draft, resolves availability/permissions and highlights missing
  entries. Only activation reserves actual units; saving a preset reserves nothing. Initially
  presets are private to their creator, with explicit copy/save rather than shared live editing.
- **Alternative:** presets are live shared rosters that reserve their named units continuously.
- **Why:** several useful plans can refer to the same expensive hardware without blocking each
  other or allowing an old plan to smuggle in permission. Crew consent is acquired for the mission.

**Q-D3 — How much may be edited while the carrier travels?**
- **Decision (accepted default):** plans, pilots and AI assignments may be edited aboard before sortie launch;
  departure clearance freezes the physical ship manifest, not its internal mission planning.
  Activation/launch revalidates consent, control, licences and capacity. At a settled site, a
  new plan may use units actually aboard; previously deployed units must return before joining
  a new launch roster in the first cut.
- **Alternative:** ship departure freezes the entire detachment until docking, as old doc 26 did.
- **Why:** the player can plan the mission en route without changing cargo or pretending a unit
  left at port arrived. Field reassignment between live detachments can be a later explicit action.

**Q-D4 — What if slot 1 loses its pilot, disconnects or is disabled?**
- **Decision (owner ruling):** scan slots in ascending order and select the first active player.
  Death/incapacitation, disconnect or disablement removes that player from active leadership;
  an earlier-slot player automatically resumes leadership when active again. No slot swaps or
  deputy ceremony. With no active players there is no squad leader; units continue their last
  orders or act for themselves through their available controllers/doctrine. No NPC replacement
  leader. "Death" here is combat unavailability, not player permadeath (Q-H12).
  The implementation predicate is a connected, capable player controlling their non-disabled
  assigned unit in the active detachment; keyboard inactivity alone is not disconnect. Reconnect
  does not override continuing incapacity/disablement. Version the leadership term separately
  from the stable roster; succession notices must not reveal hidden casualty facts.
- **Alternative:** explicit handover to a designated deputy, leaving command vacant otherwise.
- **Why:** slot order gives an obvious, automatic priority and handles reconnects without moving
  units. It does not grant new asset permissions: Q-F2 settles the successor's authority scope.

**Q-D5 — What consent does a launch command represent?**
- **Decision (accepted default):** each human accepts their unit assignment and marks ready for the current plan
  revision while seated; the leader launches ready units only after preflight reports omissions.
  Editing their unit or leader invalidates that readiness. An order never creates consent or
  transfers helm authority. A human seat is not silently converted to an AI slot if unready.
  Acceptance includes Q-D4's automatic succession, which does not invalidate consent each time
  availability changes. Deliberate prelaunch plan/priority edits still require renewed readiness.
  The all-AI roster's launch issuer is the new Q-F1, not an implied NPC leader.
- **Alternative:** accepting an invitation lets the leader launch that member whenever seated.
- **Why:** group launch remains one useful action, but the actual crew knows the selected plan
  and authority. It also makes skipped/unready members explicit rather than accidental stranding.

**Q-D6 — What happens when a group launch partially fails?**
- **Decision (accepted default):** reserve and preflight the requested ready roster as one operation, then persist
  per-unit launch progress. A race/failure after some launches leaves those units physically out,
  reports the remainder and allows retry/cancel of only unlaunched members. No physical rollback.
- **Alternative:** require the whole selected group to finish launching, otherwise order a
  real, fallible physical return of launched units before allowing the mission to proceed.
- **Why:** the interface can plan a group atomically without teleporting units when reality has
  changed. Restart and partial success have one honest history.

**Q-D7 — What is dynamic about AI assignment?**
- **Decision (accepted default):** choose/rearrange a licensed mind and profile on an eligible embarked hull before
  its launch; seal at actual departure. No pre-embarked AI grouping is required. Changing the
  controller to the already sealed fallback during an epoch is distinct from installing a new
  mind, which waits for a legal hangar/reseal. Any command persona is attributable, not a new
  source of equipment permissions.
- **Alternative:** permit field controller replacement through a new explicit reseal transition,
  preserving custody and licence accounting without requiring a return to a hangar.
- **Why:** gives the requested planning freedom while retaining the licence, compute and audit
  contracts already built for actual units.

**Q-D8 — How long do orders last, and how are they acknowledged?**
- **Decision (accepted default):** orders carry issuer, roster revision, objective and optional expiry in the
  chosen IC clock. Standing orders persist until replaced/cancelled or their objective ends;
  explicitly time-limited orders expire. Delivery, `WILCO`, `NEGATIVE`, attempted and completed
  are distinct observations. Human acknowledgements never directly actuate a unit. Transport
  uses real communication availability; a dedicated detachment chat can follow in DT.5.
- **Alternative:** inherit the simulator's fixed order TTL and immediate side-wide delivery.
- **Why:** useful mission orders outlive a practice-match timer, and delayed information cannot
  honestly be rendered as immediate compliance. Only the current effective leader commands
  initially; attachments that delegate a subset of command stay in DT.4. Delivered standing
  orders survive succession until superseded; delayed orders from an expired leadership term
  cannot overwrite orders from the new leader.

### B. Unit authority, delegation and money

**Q-A1 — Which permissions are separate?**
- **Decision (accepted default):** distinguish operate/crew, deploy, command assigned AI units, recover/load, maintain/
  refit, transfer/abandon title, and grant/delegate. Owning a unit supplies the owner's rights,
  but a loan can omit sale, refit and onward delegation. The carrier separately grants embark/
  bay services and departure authority; being sortie leader grants no right to depart the ship.
- **Alternative:** one `use` permission includes every operational act, with only sale separate.
- **Why:** lending a tank should not also permit selling it, changing its expensive fittings or
  giving it to someone else. These permissions must use current organization/economy owners.

**Q-A2 — How far may a grant be delegated?**
- **Decision (accepted default):** allow explicitly authorized onward delegation. Every child grant is a subset of
  the grantor's effective capabilities, unit scope, term and onward-grant authority; record its
  parent lineage and reject cycles. Set an implementation depth bound without making rank an
  authority test. Organization wildcard access follows currently controlled eligible units;
  individual exclusions override a wildcard allow. Include future units only within the
  organization's effective title/lease/delegated rights; merely being aboard its carrier is
  insufficient. Personal owners do not need to create an organization to lend their units.
- **Alternative:** keep the current organization's one-hop delegation and enumerate unit grants.
- **Why:** expresses the owner's bounded grant rule for both personal and organization units,
  while avoiding privilege amplification. Wildcard grants remain useful as fleets change.

**Q-A3 — What happens when use/delegation permission is revoked with a unit out?**
- **Decision (accepted default):** revoke dependent grants prospectively for new reservations, deployment, station
  acquisition and new mission orders. Do not eject a pilot or erase already executed actions.
  A current custodian retains a narrowly scoped recovery/return capability under a recorded
  policy, not fresh offensive mission authority; self-defense and emergency actions must have
  an explicit rule. Ownership disputes do not remotely disable the physical machine.
- **Alternative:** pin the full permission grant until the current hull epoch ends.
- **Why:** revocation can matter promptly without stranding a valuable unit or becoming a remote
  kill switch. The exact allowed emergency actions belong in the authority brief before code.

**Q-A4 — Who pays for ammunition, recovery and damage caused on an ordered mission?**
- **Decision (accepted default):** physical losses remain on the actual assets. Expenses draw only from explicitly
  authorized budgets or accepted mission contracts; liability/insurance reads the recorded
  agreement and outcomes. Neither the charter owner nor the current leader silently guarantees
  everyone's repairs by taking their role. The preflight shows the responsible principals.
- **Alternative:** the organizer or leader automatically pays all sortie losses and costs.
- **Why:** respects independently owned and organization-owned equipment and makes command
  possible without inventing unaccepted financial obligations.

### C. H1 — combat in a persistent world

**Q-H1 — How do human-paced combat and the rate-24 campaign clock meet?**
- **Decision (accepted default):** keep tactical motion/weapons human-paced, while campaign time, scheduled journeys
  and deadlines continue. Explicitly adopt a fixed campaign-to-tactical time mapping for the
  engagement, with all external events admitted at ordered boundaries: at rate 24, a ten-real-
  minute fight spans four campaign hours. This is deliberate campaign-time compression, not a
  claim that movement runs at 24 times its tactical speed. Mining on engaged units pauses;
  unaffected obligations continue. The adapter must specify arrival poses, reinforcement,
  message delivery, hazard/work clocks and time floors before implementation.
  Field motion, towing and physical hazards use the same declared rate before, during and after
  promotion; coarse mode changes update frequency, not travel speed. This can amend existing
  surface-action durations and must be costed explicitly. Production/mining work retains its
  separately declared campaign cadence and cannot also run on an engaged unit.
- **Alternative:** slow the entire shared campaign to 1:1 while any human combat is active.
- **Why:** preserves trade schedules and human control without a battle holding other players'
  journeys. Different tactical and campaign time scales are explicitly accepted. Local timestop
  would conflict with 6A if it delays arrival. The global outage freeze in Q-H11 is a separate
  whole-world suspension, not a local combat scheduling tool.

**Q-H2 — What starts a fight and what is the first jurisdiction?**
- **Decision (accepted default):** first delivery is deployed mobile units on one frontier site. A hostile actionable
  encounter/attack promotes the affected field into tactical authority; merely arriving with a
  different faction does not force a lobby or match. Keep 5A's core-crime, safe-hangar and
  declared-facility-operation rules, while excluding facility assault from this first delivery.
- **Alternative:** launch fights only through an agreed challenge/mission instance at first.
- **Why:** proves the real risk and recovery loop, including surprise encounters, without quietly
  expanding H1 into sieges or redefining the existing safe-storage policy.

**Q-H3 — Are sides fixed, and may units join or leave while fighting?**
- **Decision (accepted default):** host real units by position and authority, accept reinforcements through physical
  entry, and permit withdrawal through reachable exits. Detachment is a command group, not a
  universal faction/team. Relations and known hostility decide targeting; neutral recovery
  crews and a third force can exist. Cross-boundary fire/pursuit must be specified rather than
  silently lost at a map edge.
- **Alternative:** freeze two opposing rosters until a match ends.
- **Why:** makes carriers, escorts and salvage crews part of a world rather than a practice lobby.
  A first performance budget may bound simultaneous participants, not define political reality.

**Q-H4 — When does tactical simulation stop?**
- **Decision (accepted default):** demote only when pending impacts, active hostile interactions and boundary
  handoffs are settled, then preserve every unit and location in field state. Individual
  withdrawals need not wait for everyone else. Disabled units remain obstacles and future
  salvage; no victory/reset/timeout deletes them. An empty unattended field may sleep cheaply.
- **Alternative:** keep the site continuously tactical while any deployed unit remains, even
  after fighting stops. In either model all units remain physically in the field.
- **Why:** finishing a compute-intensive activity is not a physical return to a ship. The
  cooldown and measured budget are tuning choices, not permission to discard unresolved events.

**Q-H5 — Who controls a unit when a human leaves the controls or disconnects?**
- **Decision (accepted default):** deployed units keep existing physical risk and use their actually available sealed
  fallback/standing orders on link loss. Model crew status separately: compartment injury,
  ejection, rescue and capture, with no player permadeath (Q-H12 supersedes the older 3A carve-out).
  The first slice must expose a rescue/capture outcome rather than returning a body to OOC space.
- **Alternative:** retain the last physical controls or a bounded stop posture without activating
  an AI fallback, unless the human explicitly arranged a handoff before disconnecting.
- **Why:** preserves persistent stakes without equating a replaceable cockpit with deleting a
  character. Detailed injury/recovery mechanics can follow without falsifying the physical outcome.

**Q-H6 — What information becomes public after a battle?**
- **Decision (accepted default):** none merely because combat ended. Each actor retains their own observations and
  legitimately delivered reports; inspection/recovered logs may add evidence. Leaders do not
  automatically know subordinate positions, damage, ownership or compliance without a report.
- **Alternative:** publish the simulator's objective replay and complete result inventory.
- **Why:** scouting, recovery searches and being caught stealing require imperfect knowledge.
  Staff diagnostics remain distinct from in-world evidence.

**Q-H7 — What makes a unit disabled versus permanently destroyed?**
- **Decision (accepted default):** derive usable capabilities from the exact remaining sections/components. Mission
  kills such as head/cockpit loss or engine loss leave recoverable physical units. Only a
  separately defined catastrophic destruction/dismantling outcome ends the reconstructable
  hull identity, retaining any actual debris. Jammed, damaged, destroyed and missing components
  remain distinct. The damage-content review defines those terminal outcomes explicitly.
- **Alternative:** make all defeated hulls reconstructable in the first slice, postponing
  catastrophic chassis loss while still preserving destroyed components.
- **Why:** directly implements FO-1/FO-2 and stops a match-ending convenience from erasing
  salvageable weapons or manufacturing replacements for them.

**Q-H8 — Can a disabled unit be attacked again?**
- **Decision (accepted default):** yes, if it can be physically targeted; later causal damage can ruin recoverable
  components or the chassis. Whether the attack is lawful is separate. No automatic extra
  damage occurs merely because towing begins or the engagement ends.
- **Alternative:** defeated/wrecked units become invulnerable and untargetable as in the arena.
- **Why:** recovery, denial and protection become actual tactical choices. The user settled
  state at disablement, not immunity from subsequent physical events.

**Q-H9 — Does a recoverable mission kill close the hull's epoch?**
- **Decision (accepted default):** no. Keep the epoch open through disablement, abandonment and towing until hangar
  recovery or genuine terminal destruction. Record a disablement event instead of treating it
  as `IcCloseKind::Destroyed`. No new operating mind may seal merely because a gun went quiet.
  Confirm how held licence seats/compute admission are accounted for on an inactive casualty.
- **Alternative:** every mission kill closes its epoch while preserving the same physical hull;
  subsequent reactivation requires a defined new-epoch transition.
- **Why:** keeps the same hull/provenance and distinguishes termination of active computation
  from termination of custody. It needs an explicit amendment to arena-derived close events.

**Q-H10 — May the carrier leave units behind?**
- **Decision (accepted default):** yes after an explicit leave-behind confirmation naming the out/dead/unaccounted
  units; record the decision and notices. Departure does not recall, delete, release title or
  close their epochs. An embarked crew cannot be presumed to consent for another carrier's
  operator. Amend D-20-7 for this delivery rather than bypassing it accidentally.
- **Alternative:** keep the current refusal to depart while any associated rig is out.
- **Why:** the owner's loss/retrieval model needs voluntary withdrawal and later recovery; a
  lost unit cannot hold its ship hostage forever. The prompt should expose the consequence.

**Q-H11 — Should active fighting catch up through a server outage?**
- **Decision (owner ruling):** preserve every acknowledged damage/expenditure/placement/custody outcome and
  resume the interrupted fight human-paced after consistent recovery, with an explicit campaign
  time checkpoint. Do not fast-forward unseen shooting or towing through the outage. Quarantine
  incompatible active state for an audited recovery rather than restoring stock.
  After recovery is ready, hold the entire IC world frozen for five real minutes and broadcast
  at five minutes, two minutes, one minute and unfreeze. Freeze tactical/campaign progression,
  movement, combat, hazards, towing/loading, production, journeys and timed IC effects together;
  re-anchor from the durable checkpoint on resume. Players may reconnect, inspect and communicate
  through OOC channels during the countdown; gameplay mutations/new world actions are not
  admitted until unfreeze.
  Reconnecting players receive the current freeze/countdown status. A further outage requires
  consistent recovery and a fresh countdown. Q-F3 covers explicitly real-time legal deadlines.
- **Alternative:** simulate the elapsed outage under recorded standing orders before reconnects,
  allowing additional casualties and completed recovery work during the downtime.
- **Why:** an infrastructure outage should not manufacture an unattended defeat or finish a
  contested tow. Both choices must preserve committed results; neither permits OOC snapshot-tail
  rollback. The shared clock must enforce the freeze for every reader, not merely pause the H1
  loop; Q-H1's adapter preserves journey durations and event ordering at the resumed checkpoint.

**Q-H12 — How much crew injury/rescue behavior is in the first IC combat slice?**
- **Decision (owner ruling):** a minimal explicit outcome for occupants: injury/incapacitation, escape/ejection
  where equipment permits, a real survivor location and a physical rescue/capture route. Keep
  enough interaction for a disabled head/cockpit not to strand the player in an undefined state.
  There is **no player permadeath**, including an optional campaign-lethality exception. In the
  worst case a third-party NPC organization retrieves the player and delivers them to a known
  port such as Phobos to recover. The player may arrange a later return using another owned
  vessel, a newly acquired coop unit or passage aboard another player's vessel. The abandoned
  unit, installed equipment and cargo stay at their actual field location; crew rescue neither
  repairs/retrieves that unit nor closes its epoch. Q-F4 asks the fallback service's timing and
  destination/capture policy; the guaranteed recovery outcome itself is settled.
- **Alternative:** deliver the full injury, treatment, prisoner and ransom system with H1.
- **Why:** defeats retain material and logistical consequences while the player has an assured
  way back to play. The NPC rescue is an explicit world outcome, not an OOC unit reset.

### D. H2 — recovery, salvage rights and restoration boundary

**Q-S1 — Who may legally recover a unit on owned ground?**
- **Decision (owner ruling):** BOTH the unit owner and recognized landholder have independent
  lawful recovery entitlement; neither needs permission from the other. Authorized agents may
  act within either principal's delegated scope. On unclaimed/unowned ground any capable
  recoverer remains eligible. Unit owner means the current legal title holder, not every former
  owner after a completed transfer. Original unit title is recorded separately in provenance.
- **Alternative:** exclusive landholder entitlement, requiring the unit owner to obtain permission.
- **Why:** both interests are recognized without inventing a priority the owner did not grant.
  Q-S9 governs competing physical attachments. Self-recovery retains existing title with no new
  claim/window; a non-owner's recovery follows Q-S2/Q-S13/Q-S15. Concurrent recovery entitlement
  is not a blanket veto over another otherwise lawful title claim.

**Q-S2 — When does lawful salvage become legal title?**
- **Decision (accepted default):** physical recovery establishes custody and grounds a salvage claim, not instant
  clean title. Retain T3's public claim/notice/contest procedure, with deterministic completion
  of uncontested claims and a dispute path. No title is minted by loading or by crossing a border.
- **Alternative:** eligible first recovery transfers title immediately, superseding T3 here.
- **Why:** honors the existing accepted distinction between possession and title while letting
  anyone lawfully recover on open ground. Claim timing, eligible objections and notice recipients
  must be defined in the H2 legal brief, not deferred until after its implementation.

**Q-S3 — Which ground owner matters when claims change or are disputed?**
- **Decision (accepted default):** use the recognized effective claim/tenure at the target's position when physical
  securing begins; persist that basis. Paper claims alone do not manufacture a second owner.
  A continuous recovery keeps that recorded legal basis; a new recovery after interruption
  re-evaluates it. Later borders cannot retroactively legalize theft or make earlier lawful
  acts crimes. Existing disputes use the jurisdiction's contest/adjudication mechanism.
- **Alternative:** salvage rights permanently belong to whoever owned the ground at disablement.
- **Why:** provides an actionable current rule and an auditable history without freezing rights
  to obsolete map boundaries. Define which interruptions break continuity before coding.

**Q-S4 — What allows authorities or owners to act on unlawful salvage?**
- **Decision (accepted default):** authoritative private records plus acquired evidence: a witness, sensor report,
  inspection or recovered log tied to the actor and recovery. Reporting can be delayed or fail.
  Distinguish observed suspicious activity from proven attribution. No public theft broadcast,
  automatic omniscient fine or hidden-owner disclosure through a command refusal.
- **Alternative:** starting unauthorized recovery immediately alerts the owner and applies a fine.
- **Why:** implements “if caught/found” and gives reconnaissance, concealment and investigation
  a role. A visible warning may use the salvager's known claim information, not secret facts.

**Q-S5 — What actions make up the first recovery loop?**
- **Decision (accepted default):** approach/inspect, connect, tow or drag, then load are separate timed physical
  stages with explicit player commands in the first cut. Add a queued recovery planner later;
  every stage remains interruptible and observable. No requirement to possess the target's use
  grant merely to attach recovery equipment.
- **Alternative:** first delivery includes one planned recovery command that automatically
  executes the same timed physical stages, including navigation and loading.
- **Why:** creates the separate operation the owner described and supplies real moments for
  combat, competition and capacity failures without making a recovery planner another initial
  dependency. Neither option teleports the casualty into the ship.

**Q-S6 — What equipment and limits matter?**
- **Decision (owner ruling):** any mobile unit may tow one disabled unit up to and including
  twice its own tonnage, with reduced speed determined by the tonnage differential. A twenty-ton
  unit towing forty tons moves very slowly. Dedicated recovery vehicles are optimized for this
  work; specialist equipment is not a prerequisite for ordinary towing, nor an exemption from
  the two-times cap. Terrain, geometry, traction and loading constraints still apply. A towed
  hull does not occupy another commanded detachment slot, but its mass/footprint still count.
  The brief must publish one consistent mass basis (including its payload treatment) and speed curve, including the
  inclusive two-times boundary; exact coefficients are content/performance tuning.
- **Alternative:** require a dedicated recovery capability for any tow.
- **Why:** enables improvised recovery while preserving a useful role for optimized vehicles
  and a clear mass limit. Physical transport remains consequential without equipment lockout.

**Q-S7 — Can recovery occur during combat, and can it cause extra damage?**
- **Decision (accepted default):** yes, under the same field authority and physical targeting. Attachment/loading
  takes time; the tug/target can be interrupted. Dragging or a failed tow causes further damage
  only through an explicit modeled event with a previewed risk; default ordinary towing adds
  none. Combat and towing cannot both write the target's pose independently.
- **Alternative:** require the engagement to end before any recovery stage can begin.
- **Why:** allows rescue under fire while preserving FO-1's exact-state rule unless something
  actually damages the unit afterward.

**Q-S8 — How does loading use the ship's capacity?**
- **Decision (accepted default):** reserve a real destination bay/load volume and mass allowance, including wreck
  dimensions and retained cargo. Check ramp/loading equipment as appropriate. Completion
  moves the same hull to the bay once; its payload does not also get credited into the ship's
  commodity hold. A casualty that does not fit remains outside until another solution arrives.
- **Alternative:** each empty unit slot fits any wreck regardless of mass or dimensions.
- **Why:** prevents capacity exploits and makes retrieval different from actually bringing a
  unit home. Use current compartment/inventory reservation owners rather than a parallel counter.

**Q-S9 — How do two crews compete for the same wreck?**
- **Decision (accepted default):** first completed physical attachment establishes exclusive towing control, with
  a versioned operation record. Other actors can interfere through physical actions/resistance;
  a database reservation or legal claim alone does not make the wreck invulnerable or immovable.
  Restart preserves the attachment, progress and last committed position; no reset to origin.
- **Alternative:** the first claim/command locks the wreck to that player until a timer expires.
- **Why:** “show up with a tow truck” becomes the meaningful act. Authority and replay fences
  prevent duplicate loads without turning a legal right into a force field.

**Q-S10 — How long do unretrieved units remain?**
- **Decision (accepted default):** indefinitely in durable field storage, cheaply dormant when unobserved. They are
  lost from the former operator's usable roster, not erased from the world. Decay, environmental
  destruction or authorized cleanup needs a separately declared causal rule and record.
- **Alternative:** expire casualties after a fixed interval or when the last player leaves.
- **Why:** makes later discovery/recovery possible and avoids contradicting the physical loss
  model. Bounded hot memory is an implementation requirement; invisible deletion is not its policy.

**Q-S11 — Does H2 include component stripping and scrap processing?**
- **Decision (owner ruling):** H2 recovers whole hulls with their component/ammo/cargo manifest.
  Add an explicit next milestone, H3, for stripping, scrapping and repair/restoration together,
  preserving component identity, condition and provenance when parts are detached or consumed.
  Field stripping and scrap yields belong to that milestone; Q-F5/Q-F6 cover its new title issues.
- **Alternative:** build whole-hull recovery, component harvesting and scrap recipes together.
- **Why:** delivers the requested tow/load loop sooner and preserves the information later
  systems need. It avoids replacing recoverable machines with abstract commodity rewards.

**Q-S12 — How much repair must be available before broad playtesting of this loop?**
- **Decision (owner ruling):** H2 ends with the damaged unit secured in a bay; repair work belongs
  in the explicit post-H2 H3 stripping/scrapping/restoration milestone from Q-S11. Deliver its
  minimal workshop before broad economic playtesting. Actual work orders replace a head/cockpit or
  engine, repair an extant damaged/jammed weapon, and replace a slagged one using compatible
  parts, authorized resources and work time. Validate resulting capability; never reset to spec.
- **Alternative:** release H1/H2 for a limited test first, with recovered units remaining unusable
  where necessary until H3 arrives, and clearly report which repairs are not yet available.
- **Why:** the separate H2 acceptance can finish first, but a longer-running player economy
  needs a way to realize the value of its recovered machines. The component-state contract is
  required now under either schedule; detailed repair pricing and facilities belong to H3.

**Q-S13 — What can defeat an otherwise lawful salvage claim?**
- **Decision (accepted default):** a contest must establish ineligible/unlawful recovery, defective notice/procedure
  or a superior specific right under the agreed rules. Prior unit title alone does not veto
  every lawful salvage claim. On owned ground, the landholder's entitlement and any recovery
  delegation/contract remain material; the truck driver is not automatically the beneficiary.
  Q-S1 adds the unit owner's concurrent recovery entitlement without giving either party an
  automatic title-claim veto. Recovery rights alone also do not authorize attacking a competing
  tug or commandeering its carrier.
- **Alternative:** the original owner may reclaim during the contest window by paying verified
  recovery costs, even when the salvager acted lawfully.
- **Why:** lawful open-ground salvage should be capable of producing usable title. The alternative
  instead makes the first economic reward a recovery-service fee, a valid but different game loop.

**Q-S14 — What may the salvor do while title is pending, and who bears repair risk?**
- **Decision (accepted default):** custody permits stabilization and documented repair at the custodian's expense;
  deployment, sale and onward use grants wait for title or the existing title owner's permission.
  If the claim fails, no automatic reimbursement is minted: costs follow an agreed contract or
  adjudicated remedy, with repair-at-risk shown before spending. Existing component titles persist.
  Self-recovery by the existing owner does not create pending title or these provisional-use
  restrictions. Repair permission does not yet settle destructive stripping/scrapping (Q-F5).
- **Alternative:** allow provisional operation pending contest, with recorded return/compensation
  obligations if the claim later fails.
- **Why:** preserves a practical damaged-hull custody state without turning possession into a
  back door for free use, resale or delegated title. It exposes investment risk before repairs.

**Q-S15 — How long is the first salvage-title contest window?**
- **Decision (accepted default):** 48 real hours from legally sufficient public filing at the relevant registry,
  with a durable deadline and notice sent to identifiable affected principals through actual
  communication. Online acknowledgement is not required. Publication contains claim identity,
  rights asserted and the claimant's evidence, not hidden live positions or a global damage dump.
- **Alternative:** three IC days (three real hours at rate 24), allowing much faster reuse.
- **Why:** title contests should remain answerable by players who log in daily; campaign-speed
  expiry would often resolve the dispute while they sleep. Physical recovery itself never waits
  for this window. The legal notice clock is explicit and does not slow travel or combat.

## 8. Reconciliation and implementation cautions

| Existing source | What changes or remains |
|---|---|
| Doc 26 D-26-1/2/3/5/7 and Q-26-6 | Rewritten around presets, dynamic AI assignment, human orders, separate organizer/leader and real unit grants; no port-declared AI group or charter-based formation authority |
| Doc 23 Q-23-5 and S.0b | `+sortie` and the player's sortie are permitted; retain the epoch/operation distinction rather than banning a word |
| Doc 23 destruction close | Q-H7/Q-H9 require recoverable disablement to remain distinct from terminal destruction; amend the IC close adapter accordingly |
| Doc 20 §17 | Promotion is an implementation mechanism; its 256-cell site and existing TV departure refusal are not universal IC rules; Q-H10 accepts the D-20-7 amendment for this delivery |
| Doc 16 / 16c H1–H3 | Practice resets, fixed teams, untargetable wrecks, public replay and snapshot-tail rollback cannot silently become IC policy; H2 is more than another cargo `SalvageSource`; H3 consumes real component state |
| Doc 10 3A/5A/6A/T2/T3 | World framework retained except the optional player-permadeath carve-out, superseded by Q-H12; clocks and salvage-title policy are now settled subject to §9 follow-ups |

Verified implementation seams at the review revision:

- `crates/tacsim/src/match_/cockpit.rs` (`command_of`) commands the simulator's AI rosters or
  wingman, not the proposed common human/AI roster.
- `crates/mush-org/src/authorize.rs` already resolves capabilities and scopes; existing
  delegation has one-hop restrictions. Do not claim the permission system is absent or assume
  it already implements Q-A2 for personal units.
- `crates/tacsim/src/arena/damage.rs` (`destroy_unit`, `settle_disabled`, wreck consequences)
  and `crates/mush-tactics/src/custody.rs` are mandatory H1 audit surfaces: runtime weapon/sensor
  clearing and void restoration must not erase persistent component truth or restore IC assets.
- `crates/tacsim/src/composition/salvage_source.rs` and
  `crates/mush-inventory/src/salvage.rs` expose journey/cargo salvage under custody permissions;
  they do not implement towable field hulls or lawful-versus-unlawful physical recovery.
- `crates/tacsim/src/surface/recovery.rs`, `surface/black_box.rs` and `port/epoch.rs` supply
  useful durable intent/epoch patterns. Staff `Home` is not a player tow operation.
- `crates/tacsim/src/minds/ic_sortie/participant.rs` still exposes the fixture start/close
  boundary; licensed IC controllers require the actual host integration, not a fabricated permit.

**Review status:** owner-approved design decisions with explicit unanswered follow-ups in §9.
Only documentation has changed. Test/story requirements above describe future acceptance, not
gameplay tests run by this revision.

## 9. New follow-ups raised by the answers

These six questions are **not yet accepted**. They resolve intersections introduced by the
amendments; they do not reopen the 39 decisions above. Q-F1/Q-F2 affect DT briefs, Q-F3/Q-F4
affect the shared clock/crew boundary, and Q-F5/Q-F6 can be settled for the post-H2 H3 milestone.

**Q-F1 — Who launches a detachment with no active human member?**
- **Proposed default:** an authorized embarked organizer/launch principal can launch the ready
  AI-only roster and its prepared objectives, without becoming its squad leader or acquiring
  remote field command. Record that principal separately from the effective leader.
- **Alternative:** require an active human roster member to lead every launch, even if the
  detachment later becomes leaderless.
- **Why:** Q-D5 assigned launch to the leader, while Q-D4 now deliberately gives an AI-only roster
  no leader. Separating launch coordination preserves the organizer/leader distinction and
  permits useful AI-only plans without inventing an NPC commander.

**Q-F2 — What command rights does an automatic successor receive?**
- **Proposed default:** preparation obtains explicit, bounded mission grants for the
  current-leader role from each relevant owner or authorized grantor. They follow Q-D4's
  succession only for the accepted roster/mission and remain subject to Q-A2/Q-A3 revocation.
  Preflight exposes any unit that cannot accept this role grant; nothing silently widens authority.
- **Alternative:** succession changes the leader, but each successor can issue effective AI
  orders only to units covered by their own personal grants.
- **Why:** an automatic successor may lack the previous leader's individual permissions.
  Explicit mission-role grants make succession useful while preserving the owner's authority;
  neither option skips the first active player or grants permission merely by holding a slot.

**Q-F3 — Does the global outage freeze pause the 48-real-hour salvage contest window?**
- **Proposed default:** exclude world unavailability and the restart countdown from that
  window. Persist its remaining duration, resume the deadline at unfreeze, and publish the
  adjustment. It still counts 48 available real hours, not accelerated campaign hours.
- **Alternative:** keep the original wall-clock deadline; a deadline that passes during outage
  is processed only after unfreeze, without an extra opportunity to object.
- **Why:** Q-S15 was chosen to give players time to contest; unavailable servers should not
  consume it. This would explicitly amend the accepted real-time wording. Until answered, do
  not silently shift legal deadlines or treat an H1-local pause as a complete global freeze.

**Q-F4 — When and where does guaranteed NPC crew retrieval return a player?**
- **Proposed default:** after ordinary rescue cannot resolve the casualty, record an NPC
  extraction with a published, bounded campaign-time delay to the nearest eligible safe port
  known to the character, with a guaranteed fallback such as Phobos. Injury/capture may delay
  it only up to that bound; funds or another player's refusal cannot remove the guaranteed
  return-to-play route. World freeze pauses this recovery too. The unit stays in the field.
- **Alternative:** return to the last departure port after one fixed real-time respite,
  regardless of distance; keep the guaranteed outcome and abandoned-unit state the same.
- **Why:** no-permadeath and fallback retrieval are settled, but destination, timing and capture
  otherwise risk leaving the promised route undefined or making it an instant travel shortcut.
  The exact delay is tuning after the service policy is chosen.

**Q-F5 — May a non-owner strip or scrap a unit while salvage title is pending?**
- **Proposed default:** require resolved title or explicit permission from the existing title
  owner for destructive stripping/scrapping. Q-S14's stabilization and documented repair at
  risk remain permitted; they do not authorize consuming another party's property.
- **Alternative:** allow destructive processing at risk, with preserved component/value records
  and enforceable restitution if the claim fails.
- **Why:** the new combined H3 milestone adds irreversible operations that Q-S14's repair rule
  did not decide. Waiting for title avoids making a successful contest meaningless.

**Q-F6 — Can field-recovered components establish their own salvage claims?**
- **Proposed default:** yes, where the stripping itself was authorized. Track each recovered
  part's identity, condition and parent-hull provenance, with a claim scoped to those parts.
  The remaining chassis cannot also claim or consume the same component/value.
- **Alternative:** require whole-hull recovery/title resolution before any component claim.
- **Why:** H3 field stripping should work when the chassis cannot be moved, without duplicating
  salvage or laundering components out of an unresolved parent claim. Q-F5 still governs authority.
