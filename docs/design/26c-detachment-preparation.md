# DT.2b — Detachment preparation

The accepted rules remain [doc 27](27-ic-field-operations-design-and-survey.md).
This implementation brief separates durable planning from the physical and
economic facts needed to activate a mission.

**Owner correction, 2026-10-29:** current containment in the carrier is the
personnel eligibility rule, checked at preparation and launch. Embarkation
intervals, departure manifests and complete movement coverage are not prerequisites.
Docs 26g–26j are superseded and must not be extended. Their merged code stays in
place for now; the closed provider is inert on the live command path. A later
cleanup cut may delete the v2 provider if nothing adopts it.

| Slice | Deliverable | Admission boundary |
|---|---|---|
| DT.2b1 | Private presets, independent drafts, invitations and revision-bound player declarations | No reservations, employment contracts, mission-role grants or launches |
| DT.2b2a | Prepared structural movement foundation | Core object images and the canonical Sim placement patch commit together; no personnel eligibility yet |
| DT.2b2b.1 | Immutable personnel crossing capture | Same-commit physical evidence is explicitly uncertified while legacy paths remain; no intervals or eligibility |
| DT.2b2b.2 | Prepared staff interior authoring | `@spatial/space` commits its real topology and crossing capture together; no synchronous fallback |
| DT.2b2b.3a–3d | Superseded movement work in docs 26g–26j; do not extend | Retain merged code for now; no live-path adoption or DT dependency |
| DT.2b2b.3e+ | Withdrawn movement-coverage, interval and departure-manifest programme | No further coverage work required for NPC eligibility or contracts |
| DT.2b3 | Hire a real NPC at port under a contract record | Existing character, accepted terms and mission scope; later wages accrue from the contract, not aboard-time evidence |
| DT.2b4 | Preparation and activation of the six slots | Resolve who is currently in the hull; live authority, consent, licences, exclusive reservations and accepted mission-role grants |
| DT.3 | Per-unit launch and operation history | Recheck current containment, then launch the assigned people and units; durable departures, partial failure/retry, hull epochs, custody and returns |

## Current delivery sequence

1. **DT.2b3 — hire:** at port, record the accepted contract referencing a real NPC.
   Hiring does not create a synthetic pilot or teleport the character. Contract
   terms define any future wage accrual; wage settlement can follow later.
2. **Board:** `+sortie/board <npc>` checks the caller's recorded crew contract and
   reuses the existing `+passage/board` movement path: real station exits to the
   berth, the ship's airlock and available berth, then the crew compartment. The
   person accompanies the ship through existing containment. No boarding-history
   record is needed.
3. **DT.2b4 — activate:** resolve the plan's six slots against real units and NPCs
   currently contained in the selected carrier. Read current room/interior
   containment through the existing physical model. Contract scope, availability,
   permissions, consent and reservations remain separate checks. Saved plans and
   crossing records do not substitute for current presence.
4. **DT.3 — launch:** recheck containment at launch so an NPC who left after
   preparation cannot launch from a stale roster. Move the assigned people and
   units into their actual field roles; they need not remain aboard to serve in
   the launched detachment. Returning, rescued or transferred NPCs qualify on
   current containment without historical boarding evidence.

Durable hiring, activation and launch changes use the coordinator under K8.
That does not require ordinary walking or every post-move observer to migrate
to the closed movement protocol. Cleanup of unused movement infrastructure is
a separate later cut, not an activation prerequisite.
Preserve the completed movement work for reconsideration once the core surface
and mining flow is playable; it is deferred engineering, not rejected as incorrect.

## DT.2b3 hiring contract

Tacsim owns `tacsim.crew_contracts`, keyed by the existing NPC object's ID. A
contract records that NPC, the authenticated employer, the world-IC hiring stamp,
the port, agreed duties (mining, combat and/or recovery), operator/commander scope
and the accepted terms hash. The record is strictly decoded and bounded to 4 KiB.
It is an employment agreement, not an economy settlement or a personnel-location
record. No wages, escrow, account debit, embarkation evidence or unit reservation
are written by hiring. A later financial integration must use the economy owner.

`+sortie/crew` lists standing offers from real, visible NPCs in the player's
current authored port room. The NPC's GOD-owned JSON `crew:offer` attribute is
their authored consent to those terms; a player cannot negotiate arbitrary terms
or accept on another employer's behalf. The first authoring interface is this
attribute on existing NPCs, not an automatically replenished character pool:

```json
{"schema":1,"port":"ceres","mission_scope":"subordinate_operator","duties":["mining","recovery"]}
```

The shipped `tacsim.crew.ceres` pack adds two persistent people at the Slack Line:
Ilya Mendes (subordinate operator) and Sana Okonkwo (commander-capable), both for
mining/recovery duty. Its `crew_offer` entries bind standing consent to NPC entries
in the same pack, using the passenger-role content pattern. The NPC applier owns
identity and profile; the crew role writes only the staff-owned offer attribute.
Content updates and boot preserve location, memories and actual hiring contracts.
No replacement pool, wages, permission grants or automatic boarding are created.
The new pack waits for a kept world's Ceres v4 dependency to be upgraded by staff.

Use `detachment_commander` for a commander-capable offer. The port must resolve
to the NPC's current station port, rather than a field landing site or a ship
interior. The pool read admits at most 256 room contents and 32 available offers;
overflow refuses rather than silently omitting crew. NPC movement and injury
remain ordinary character state, with deployment eligibility checked by DT.2b4.

The hire command accepts the exact displayed terms. Its ordinary coordinator
offer fixes the actual NPC and terms; a revision check re-reads current presence,
authored consent and existing contracts through final admission. A unique NPC
key prevents two employers from hiring one person. Acknowledgement installs the
contract, and restart restores it; retry uses the common coordinator receipt.
`+sortie/contract #<NPC>` discloses it only to its employer or that NPC. Reauthoring
an offer never changes an existing contract. Release, renegotiation and wage
settlement are later contract operations, not implied by leaving the vessel.

`+sortie/board <npc>` supplies the explicit boarding action, using the paying
passenger service's existing physical helper. It first checks the caller's own
crew contract, then their current ship, port presence and unfrozen boarding window.
The person must actually be at that departure station, and walks its authored
route with ordinary lock and movement-hook checks. The existing Sim airlock
placement consumes a real berth; the person then walks into the crew compartment
when reachable. The captain may arrange this while ashore at that port or aboard
the ship. Repeating it for someone already aboard is a no-op.

Boarding writes no new agreement, manifest, wage evidence or embarkation interval.
It uses normal physical persistence and does not adopt the deferred movement
provider. A failed route can leave the NPC partway to the berth, as passenger
boarding already does; retry starts from the person's actual room. This is not a
persistent follow order. DT.2b4 reads the contract and current containment when
resolving the six slots, including after boarding and restart.

## DT.2b4 current personnel resolution

The DT.2b4 personnel resolution used by `+sortie/plan` reads current physical
containment and the real contracts for the draft's NPC slots. The organizer must
be aboard the selected carrier to resolve their crew. A nominated NPC commander
must occupy an NPC slot and have commander scope in their contract. Missing or
wrong-kind people, missing/mismatched contracts and absent crew remain unresolved;
a saved preference cannot supply them. At most six slot rows plus an unresolved
unassigned-commander notice are produced. Invited players do not receive another
employer's contract terms.

Resolution is ephemeral and is recomputed after ordinary walking and restart.
It creates no aboard ledger, reservation or cached authority. Activation and
launch must call the resolver again and also admit actual unit permissions,
player consent, control, capability/health, licences and mission-role grants.
The current report is not a claim that those remaining checks are built.

## DT.2b4 accepted roster and reservations

`+sortie/activate <draft>:<revision>=mining|combat|recovery` now accepts a complete
one-to-six-slot player/NPC roster. The organizer, units and people must currently
be aboard the selected carrier. NPC contracts must belong to the organizer and
cover the selected duty and any commander assignment. Players must have marked
Ready for this exact material revision and remain at their actual helm with the
real Sim control-lease witness. Automated controllers, mind selections and
financial terms refuse until their actual owner admission is integrated.

The resolved action obtains current Org guards for organizer Deploy and each
actual operator's OperateCrew right. The bridge is declared for this action;
Tacsim cannot manufacture a right or impersonate a private owner-read caller.
Org re-evaluates current grants, title, expiry and denies through final take.
Tacsim separately re-evaluates the selected draft, current containment, consent,
contracts and reservations. Leaving and returning before final take qualifies
by current presence; no continuous-presence or movement-history proof is added.

One coordinator commit writes `tacsim.detachment_active` for the organizer's
draft and an exclusive `tacsim.detachment_reservations` row for every selected
unit and person. An exact compound organizer/local-draft identity avoids hash
collisions; this first activation format admits 32-bit local draft IDs and
refuses larger ones. The accepted record keeps the material snapshot, contract
digests, duty, source revision, independent activation revision and IC stamp.
Editing or discarding the mutable draft cannot rewrite the accepted assignments.
No physical or financial records are written by activation.

`+sortie/active` exposes the accepted assignments to their organizer and assigned
players. `+sortie/deactivate <draft>:<activation revision>` closes the record and
removes precisely its original reservations in one commit, even after a draft
edit, departure or permission revocation. It does not recall, repair or move
anything. Reservations do not intercept ordinary movement. Missing acknowledgements
and retries use the common coordinator's durability and receipt rules.

This cut reserves preparation assignments. It does not seat an NPC, grant a
mission role or certify health, equipment capability or a launch. DT.3 must
recheck current containment and admit the real control and operational facts
before moving the assigned units and people onto the surface.

## DT.3a accepted-roster launch inspection

`+sortie/preflight <draft>:<activation revision>` is an organizer-only, read-only
inspection while aboard the selected carrier. It reads the accepted snapshot,
not the editable draft, and checks current unit/person reservations, carrier
site berth and ramp, clearance, current containment and accepted NPC contract
digests/duties/scope. Unit condition derives from the actual TacticalUnit;
power, control, movement and weapons remain separate observations. This does
not require weapons for mining or manufacture a crew health result.

Org checks the authenticated organizer's current Deploy right. Its advisory
CrewUse read requires that Deploy right before evaluating the selected real
operator's current OperateCrew right. It returns only a current decision, no
grant details and no transferable permit. The caller is never impersonated as
the operator. Without Deploy, preflight withholds unit-condition and operator
authority details. NPC location is checked only under the organizer's existing
contract. The report neither stores a ready flag nor certifies a complete
launch. DT.3b supplies physical deployment for operators already at real consoles
with current leases. DT.3c adds NPC seating and existing control-handoff admission.
DT.3d–DT.3h now supply field command, mission-role grants and the physical NPC
mining driver, as recorded in [the implementation brief](26a-detachment-implementation-plan.md).
Licensed autonomous-mind field admission remains separate work.

## DT.3b physical deployment and operation history

The physical DT.3 host extends preparation with operation and flight records:
`tacsim.detachment_operations` freezes the accepted roster per activation;
`tacsim.detachment_unit_heads` names a unit's current flight;
`tacsim.detachment_flights` retains every flight under its monotone unit ordinal.
`tacsim.detachment_flight_index` bounds the outstanding acts visited by the host
to 1,024 units; closed flights leave that index but retain their history. Each
operation retains up to 256 flight references, including repeat deployments.
These are records of actual deployment acts, not a second character-location
system. The closed v2 movement provider remains unused.

`+sortie/launch` admits a selected set together through the coordinator, with
current Deploy/OperateCrew guards and real Sim helm witnesses. Every operator
must be physically inside their unit at its declared console; NPC contracts and
actual billets remain distinct. The admitted ramp act uses the existing launch
duration. The bounded host applies each due placement through ordinary Sim APIs,
then confirms that unit through the coordinator. A lost acknowledgement or restart
reconciles the same intent against actual placement, without replaying a move.
Current containment and physical condition are rechecked before the move. Rights
are checked through admission's final take; later revocation does not erase an
already admitted act or a completed physical step.

Carrier-dispatched surface work cannot overlap an unrelated detachment flight.
The surface-act provider checks the shared unit reservation and flight head,
including a final owner revision guard. DT.3g admits an accepted physical mining
crew through that same host with its exact DT flight and existing epoch; it opens
no second epoch. This is action admission, not interception of ordinary walking.

The host reuses S.1's IC epoch identity, ROM seal and open/close records, links
the carrier's existing epoch, and writes the existing black box through its
owner participant. Detachment context records operation, slot, roster revision,
launcher and operator. Generic units name their actual bay compartment rather
than inventing a numbered lease berth. Optional population telemetry is bounded;
if it cannot be established, the epoch is retained with `unrecorded`, following
S.1's failure rule, rather than losing a real departure or inventing a count.

`+sortie/recover` admits only a unit already at the ramp and uses the real bay's
capacity when returning. It preserves damage and records whether its operator
was actually aboard at confirmation. `+sortie/end` cancels only departures still
aboard; it never recalls field units or closes their epochs. Returned units may
relaunch under the same operation with a new flight ordinal and epoch. History
retains unresolved field units and missing crew. DT.3c supplies NPC seating/control
acquisition; DT.3d–DT.3h add the IC command and physical mining execution paths.
Admitted durations follow continuous world IC time across rate generations.
An explicit world-time freeze suspends physical steps until time resumes.

## DT.3c contracted NPC unit stations

`+sortie/seat <draft>:<activation>=<slot>` arranges one accepted NPC assignment.
The organizer and hired person must currently be aboard the selected carrier;
the unit must occupy its actual vehicle bay. Current accepted contract scope,
duty, reservations and real Deploy/OperateCrew decisions are checked before any
movement. The unit must already have one authored physical helm console; this
command creates neither an interior nor an NPC and cannot displace a billet holder.

The NPC boards through ordinary Sim placement and receives its existing billet
through the same owner writer as `@spatial/assign`. Its request then uses the
existing K6 control-handoff action and scheduler as that NPC, without an account,
OOC proxy focus or alternate lease record. The new physical-NPC offer and scheduled
apply both guard current console/billet presence through final take. Historical
handoff jobs retain their existing behavior and wire shape. Partial physical
seating is retained if control admission refuses; a retry uses actual placement.
Launch independently rechecks employment, containment, rights and the real lease.

## DT.2b1 ownership and records

The neutral `mush-tactics::detachment::planning` model defines material plans,
presets, drafts and declarations. Tacsim owns their durable typed families and
submits changes through the common coordinator. World state holds registered
handles and bounded pending command input; it holds no authoritative planning
mirror. A missing acknowledgement cannot install a prepared change. Restart
hydrates only committed typed records and exact retries return their receipt.

Each character owns a bounded private book containing at most eight presets
and eight drafts. Both use monotone local identities, which are never reused.
There are at most six ordered command slots per plan. Independent drafts may
prefer the same unit: planning does not reserve anything. Loading a preset copies
its preferences into a new draft. Editing that draft cannot modify its source
preset or another draft. Saving a draft as a preset is another explicit copy.

Only the owner reads their presets or edits their book. A draft is visible to its
organizer and currently assigned players. A bounded invitation index provides
that lookup without a global scan. Changes to a draft and its invitation indices
commit together; capacity exhaustion refuses the whole change. An invited
player cannot edit the organizer's plan or declare consent for another player.
Declining frees that player's invitation-index entry. They may still read the
assigned draft by its known reference and explicitly accept again; a material
edit clears declarations and can issue a fresh invitation. Both operations
remain subject to index capacity, so rejection cannot leave a partial edit.
An organizer need not occupy slot one, lead the detachment, or own the carrier.

The material plan includes the carrier, ordered slots, preferred unit, human or
automated controller selection, role, selected mind/profile preferences,
explicit NPC commander and optional financial-terms digest. Missing references
remain unresolved preferences. Mind and financial selections in this slice do
not establish a licence, employment agreement, debit authority or actual host.
All durable fields are strictly decoded, with explicit nulls for optional facts,
independent encoded-size limits and bounded collections.

## Consent and readiness

A declaration binds the authenticated player, their actual selected assignment,
draft revision and canonical material-plan hash. A player accepts before marking
ready. They may decline their own assignment. Material edits clear declarations
and advance the draft revision. A no-op edit refuses without changing either.
Later automatic command succession changes the leadership term, not this stable
roster revision, and therefore does not silently invalidate consent.

Ready additionally requires the player personally at the assigned unit's actual
helm console, with the current billet and focus, and both player and unit
physically aboard the selected carrier. Remote focus and port-dispatcher
exceptions do not count as presence. The real Sim control-lease witness guards
the action through final commit; a physical preimage also revalidates placement,
focus and station changes. The record is a declaration for that plan, not a
reusable permission or proof that the crew remains seated after committing it.
Activation and launch must recheck current facts, including after restart.

## Preparation reports

Reports read current physical ancestry and the authenticated viewer's operational
permissions through the registered Org reader. They disclose no hidden object
names or another principal's authority. Other principals' permissions remain
unresolved until their own owner-attributed admission. A physical station
observation is labelled separately from the actual lease check on Ready.

Preparation reports distinguish current carrier containment from NPC employment,
applicable mind/licence admission and reservation/activation work. The old report
requirement for last-port proof and embarkation intervals is removed with DT.2b3;
current containment establishes physical eligibility. Crew contracts are inspectable
through `+sortie/contract`, and DT.2b4 must read them during activation. There is no
automatic NPC creation, teleport, fabricated contract or launch operation in a plan
or report.

Player commands are documented in [detachment preparation help](../help/detachments.md).
The retained physical transaction foundation is specified
in [26d](26d-prepared-physical-transitions.md).
Uncertified crossing capture and its compatibility boundary are specified in
[26e](26e-personnel-crossing-capture.md).
The first migrated command is specified in [26f](26f-prepared-interior-command.md).
The historical movement work is recorded in [26g](26g-locomotion-admission.md),
[26h](26h-locomotion-dispatch.md), [26i](26i-locomotion-transition.md) and
[26j](26j-movement-completion.md). All four are superseded and must not be extended.

## DT.2b1 verification

Validated locally on 2026-10-28:

- Twelve neutral-model tests cover strict codecs, independent copies, monotone
  identities, bounds, assignment-specific declarations and material revisions.
- Twelve Tacsim tests cover command fences, transactional invitation capacity,
  decline/reaccept behavior, nested physical presence, actual station facts and
  privacy-equivalent missing/inaccessible references.
- Three tests run the production server composition and ordinary coordinator:
  private planning and consent survive restart and lost acknowledgements; stale
  queued input cannot select a replacement request; readiness requires the real
  helm lease and refuses a physical departure before final commit. Live unit
  grants and revocation change the preparation report.
- The existing three unit-authority coordinator tests still pass. Production
  composition admits 35 owner-read services and 38 grants, within its 64/64 caps.
- The twelve motor regression probes reproduce the DT.1b API-6 replay fixtures
  on Windows, including the retained identity-only compatibility control.
- Clippy passes with warnings denied for the changed libraries and integration
  targets. Source-policy scans cover panic budgets and state-slot ownership.
- The complete Tacsim library suite passes: 1,917 tests passed, four ignored.
  Thirty-two server composition tests, two market privacy regressions and 26
  help/registration/typed-state/restriction contract tests also pass.
