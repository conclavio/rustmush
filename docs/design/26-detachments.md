# 26. Detachments — plans, units, pilots and orders

**Draft 0.4, 2026-10-28.** Rewritten against `origin/main` at `2cbe9f7c` after the owner's
2026-10-28 rulings. Draft 0.1 remains in Git history; no DT implementation is claimed.
Rechecked against `origin/main` at `62d093c2` before this update.

Read [27 — IC field operations: shared design and owner survey](27-ic-field-operations-design-and-survey.md)
with this document. It aligns DT.1–DT.3, H1 field combat and H2 salvage around one lifecycle.
FO-1–FO-16 and all 45 survey decisions are accepted, including the human-NPC commander and
last-port NPC embarkation rulings in doc 27 §9.

## 1. Goal and IC authority

An embarked player selects one of several saved detachment presets, edits the plan for this
mission, obtains the necessary permissions and player acceptances, then launches from the
carrier. Default squad size is six command units: one tank/biped or, in future, one battle-armor/
infantry group per slot. Players may occupy any slot; the first active player by slot order
leads. With no active player, an assigned available human NPC commander may lead. The leader
issues **orders** to player, NPC and automated recipients alike: players exercise judgment,
and NPC/software controllers attempt orders through their actual capabilities and observations.

The carrier's charter holder, organizer, leader, unit owner, operator and payer may be different
people. The charter holder need not accompany the sortie. Any player aboard may form a
detachment using units they have permission to use. Physical units and real NPC crew must have
embarked: any NPC assigned a unit or eligible to lead was aboard at the ship's last-port departure.
Unit assignments, software profiles and squad groupings remain dynamic mission preparation;
the grouping itself need not have been declared at the port.

The OOC simulator is a proving ground and consequence-free practice. Its lobby claim order,
pod counts, red/blue sides, auto-dealt squads, resets, fixed timers and roster caps do not set
IC rules. DT.1 may exercise the common roster contract there; the lobby is only an adapter.

## 2. Records and unit identity

| Record / role | Meaning |
|---|---|
| Preset | Reusable named, revisioned private plan; multiple per player, explicit load/copy/save (Q-D2). |
| Mission draft | Editable instance of a preset or fresh plan; references candidate hulls, pilots and AI profiles. |
| Embarked NPC personnel | Real character identities aboard at the carrier's last-port departure; resolved before unit/command assignment, never generated from a software profile. |
| Active detachment | Resolved numbered roster of six command units; tank/biped individually, future battle armor/infantry as groups (Q-D1). |
| Player sortie | The operation's identity/history, potentially containing multiple hull launches and returns. `+sortie` is approved vocabulary. |
| Hull epoch | Doc 23's seal interval / `SortieId`, separate from the operation or roster. |
| Organizer | An embarked player managing the plan; need not become leader. |
| Leader | First active player in ascending slot order; otherwise an assigned, available, authorized human NPC commander who was aboard when the ship departed its last port. Without either, no leader (Q-D4/Q-F1). |
| Operator | The character holding actual station/control authority; membership does not confer a control lease. |

Presets do not create units, grants, reservations or consent. Activation resolves stable
unit identities and records a roster revision; changing a draft does not silently rewrite its
preset. One unit cannot be deployed into conflicting operations. The character's actual location
and station still constrain what they can do; simulator account-slot quotas are not IC policy.

Q-D1 uses the useful command scale for each unit type. A group of, for example, six battle-armor
troopers or eighteen rifle infantry functions as one unit for orders and command, while its
constituent equipment, mass and casualties remain real. These examples do not fix future content
strengths, and a tank/biped slot does not become an arbitrary group of vehicles. Neither larger
formations nor physical capacity is decided by the arena's dealer or six keyboard shortcuts.

## 3. Permission to form and use

Settled authority roots:

- Personal ownership inherently grants use, and the owner may grant access to others.
- Organization-controlled units support wildcard and individual-unit permission scopes.
- Granting permission is itself a permission and cannot exceed the grantor's authority.
- Carrying another party's unit does not grant its use. Charter ownership is not a generic
  detachment-formation or arbitrary-unit-control gate.

Use the current `mush-org` and `mush-economy` owners through a shared authorization adapter;
these systems already exist. Q-A1–Q-A4 define capability bundles, scope/delegation, revocation
and financial liability. Personal lending must not require inventing an organization, and
`port::coop::helm_of` is a programme-hull resolver, not the general unit-permission answer.

Planning cannot mint authority. Relevant rights are checked on preparation and again at
reservation, launch, new station acquisition and command admission. Player acceptance approves
the assignment and command relationship, never involuntary helm/fire control. Permission to
command does not imply permission to sell, scrap, spend someone else's money or re-grant use.
Record organizer, leader, owner and operator separately from doc 23's actual launch principal.
Automatic succession never creates asset permissions. Accepted Q-F2 requires explicit bounded
mission grants for the current-leader role, authorized by each relevant owner/grantor, for the
accepted roster and subject to revocation. This covers eligible players and the assigned human
NPC commander; preflight exposes uncovered units. Neither leadership nor a character's presence
aboard supplies a missing grant.

For H2, authorization to operate the tow vehicle is distinct from physical access to a target
and legal salvage entitlement. Do not require target-owner permission on every recovery path:
the owner explicitly allows physically possible unlawful recovery, with evidence-based consequences.

## 4. Presets, preparation and embarkation

Proposed player flow (verb arguments remain for the implementation brief):

| Step | Action | Behavior |
|---|---|---|
| Choose | list/load/save/copy presets | Multiple reusable plans; missing units remain visibly unresolved |
| Form | `+sortie/form`, optionally from preset | Embarked organizer creates a draft without necessarily occupying slot 1 |
| Assign | invite/accept/decline; choose hull/NPC/profile; arrange slots | Physically embarked units and actual last-port NPC personnel, effective grants, player acceptance |
| Review | roster/plan | Distinguish planned, accepted, reserved, ready, out, disabled, recovered and unresolved |
| Prepare | activate/ready | Reserve actual units once; readiness belongs to a plan revision and includes automatic succession |
| Lead | automatic first-active-player selection; assigned NPC if no player active | Stable slots; separate leadership term; player resumes priority; NPC is a real embarked human |
| Launch | `+sortie/disembark` / `+sortie/launch` | Preflight ready units; record each actual launch; expose partial failure |
| End | return/recover or explicitly end operation | Ending the group record does not recall or repair physical units |

**D-26-3 is superseded:** hulls and real NPC people embark, not fixed AI squads. Before launch,
an eligible embarked hull may receive an available embarked NPC assignment or a different
permitted software mind/profile. Installed hardware, licence, compute admission and sealing
still apply to the in-world mind; the NPC's decision driver is a separate concept. Dynamic
planning cannot create personnel or rewrite an active epoch. Q-D7 and FO-16 define these boundaries.

The accepted Q-D3 rule freezes the physical ship manifest for departure while permitting mission
planning over already embarked hulls. Sortie launch has its own readiness/reservation boundary.
The old blanket freeze throughout the voyage is no longer an assumed requirement.

Q-D5/Q-D6 settle player readiness and partial launch: preflight the requested ready
set, then persist actual departures individually. A later failure leaves launched units out,
reports the remainder and permits retry/cancellation of only unlaunched members. Do not silently
skip pilots, substitute AI for them or roll back physical departures by teleporting units.
Accepted Q-F1 lets an authorized embarked organizer launch a prepared no-player roster without
becoming its continuing commander. A real embarked human NPC leads it in-world; the software
driving that character does not make them an automated unit mind. Keep launch principal, field
commander and unit controller separate. Player readiness accepts automatic succession but
deliberate prelaunch unit/priority edits still require renewed readiness.

## 5. One order vocabulary, different control responses

`ORDER <slot|ALL> <objective>` is an order to any recipient. Player-facing wording says “Order
from …”; its advisory effect on simulation controls does not make it a different category.

- Players may acknowledge, refuse or exercise judgment. `WILCO` / `NEGATIVE` report their
  response; no order or acknowledgement directly moves, aims or fires their unit.
- NPC characters attempt orders through their decision drivers and authorized stations;
  automated unit minds use their available sealed controllers. Both act from their own
  observations/capabilities. Orders supply neither hidden target identities nor unauthorized assets.
- Initial command belongs to the current player leader or assigned human NPC commander. Explicit attachments/subset delegation belong
  to DT.4, not an inherited practice-pod wingman rule.
- Delivery, acknowledgement, attempt and completion are different facts. Q-D8 defines expiry
  and standing orders. IC communication availability applies; no instantaneous side-channel
  or fixed simulator TTL is assumed.
- Typed commands, `F1`–`F6` and client forms share the same versioned roster. Leadership changes
  advance a command term without renumbering slots; stale-term orders cannot overwrite newer
  leader orders. Previously delivered standing orders continue until superseded.

Scan slots in ascending order for a connected, capable player controlling their non-disabled
assigned unit. Death/incapacitation, disconnect or disablement advances the scan; an earlier
player becoming active again automatically regains command. This is combat unavailability, not
permadeath. Ordinary keyboard inactivity is not disconnect. With no active players, an assigned,
available and authorized human NPC commander may lead; returning players resume priority. If
neither is available, there is no leader and units continue their last orders or available doctrine.
The NPC must be a real character embarked at the ship's last-port departure, retaining actual
location, perception and injury. A disconnect never generates a person, and a unit mind never
becomes a character principal. Doc 17 D-UM-52/53 and doc 10 T9 already make this legal distinction.
Leadership notices use available communications and do not reveal hidden casualty details.

Player order delivery/acknowledgement changes command history, not physical unit state. NPC and
unit-controller execution may affect simulation; tests must distinguish these rather than assert every order
event is outside deterministic state.

## 6. Launches, casualties and recovery

DT.3 uses production physical hosts and doc 23's epochs. A seal opens on actual departure, not
roster creation. Its log references player operation, detachment, slot, roster revision and actual
launch principal. Licensed IC minds additionally require the production claim/admission/resume
path: the existing coreless epoch book is not that path.

Returns reconcile after restart; disbanding or editing presets is never an epoch close. An early
return and relaunch may create successive hull epochs within one operation.

**Down is not automatically resolved.** A casualty may await recovery, be unaccounted for, be
left behind or later be recovered/captured. An ended operation records unresolved outcomes; it
does not erase their physical units. Q-H7/Q-H9 distinguish recoverable defeat from terminal
destruction. No arena result marks all casualties returned or closes their epochs automatically.

H1 returns exact field damage and location. H2 moves the same damaged unit into a bay. Roster
knowledge comes from observation and delivered reports, not a perfect missing-unit tracker.
H3 is the explicit next milestone for stripping, scrapping and repair/replacement of specific
parts. Carrier departure with units left behind implements the accepted D-20-7 amendment in
Q-H10. Crew have no permadeath: worst-case NPC retrieval returns the player to a known port;
the casualty stays in the field for a later physical recovery expedition.

## 7. Revised cuts

| Cut | Scope after owner decisions | Acceptance |
|---|---|---|
| DT.0 | This rewrite and doc 27 shared design/decision record | All 45 questions answered; NPC-human/embarkation qualifications explicit; old assumptions dispositioned |
| DT.1 | Common numbered roster and command authority; player/NPC/automated orders; automatic succession; client projection; OOC adapter | Player in any slot; first active player commands and resumes priority; assigned available human NPC can lead without active players; neither means no leader; stable slots; player orders do not actuate controls; NPC acts from own view; stale/unauthorized orders handled |
| DT.2 | Presets/drafts; shared unit authority and bounded mission-role grants; last-port NPC personnel; consent/readiness; reservation; aboard editing | Charter holder absent; organizer differs from leader; personal/org grants; reject invented or non-embarked NPCs; dynamic assignment of embarked personnel; no double reservation; revoked grants/stale acceptances handled |
| DT.3 | Physical group/individual launch; epoch linkage; partial progress/restart; returns and casualty history | Units launch once; no rollback teleport; recover/relaunch; disband leaves field units out; same history feeds H1/H2 |
| DT.4 | Optional attachments/subset command delegation | Bounded authority; no forced human controls |
| DT.5 | Dedicated detachment communications surface | Real availability and observer-safe delivered knowledge |

DT.1 builds a neutral command contract, not a lobby extension awaiting an IC rewrite. DT.2 needs
real unit authority, not a charter shortcut. DT.3 can prove human/coreless launch independently,
but cannot claim AI launch complete without the actual IC host. The three-stream dependencies
and common acceptance story are in doc 27 §§5–6.

## 8. Disposition of Draft 0.1

| Old ID | Current disposition |
|---|---|
| D-26-1 | Six command units; players anywhere; first active player leads with automatic reversion, or assigned available human NPC when no player is active (Q-D4/Q-F1). Tank/biped per slot; future infantry/armor group per slot (Q-D1). |
| D-26-2 | Presets/editing required; separate organizer/leader confirmed; clearance freeze withdrawn as an assumption; Q-D2–Q-D6 settle details. |
| D-26-3 | Superseded by FO-7/FO-16: physical hulls and NPC people embark; their unit assignments and AI grouping are dynamic. |
| D-26-4 | Physical hosts still open epochs; silent skipping and “all down means resolved” removed from the lifecycle. |
| D-26-5 | Orders to humans remain orders; human-control advisory behavior retained. Succession and standing/expiring orders accepted in Q-D4/Q-D8. |
| D-26-6 | Proposed epoch references retained, with roster revision and operation identity; not implemented. |
| D-26-7 | Lobby becomes an OOC adapter; claim order/auto-dealing cannot define IC authority or force structure. |
| Q-26-1 | Answered: `+sortie` approved. |
| Q-26-2 | Prelaunch editing confirmed; exact arrangement/succession is Q-D3/Q-D4. |
| Q-26-3 | Future battle armor/infantry use one group per slot; their implementation remains outside the first ground-vehicle delivery. |
| Q-26-4 | Attachments proposed for DT.4. |
| Q-26-5 | Plan over physically embarked units; first field-formation scope is Q-D3. |
| Q-26-6 | Superseded by FO-8/FO-9: any embarked player may form using actual permissions; charter title is not the gate. |

See the [complete owner survey](27-ic-field-operations-design-and-survey.md#7-owner-survey).

## 9. Ledger

- **2026-10-28:** Draft 0.2 and doc 27 drafted from the owner's directions; documentation only.
  No DT.1–DT.3, H1 or H2 gameplay implementation is claimed.
- **2026-10-28:** Draft 0.3 records all survey decisions and the owner's amended slot/leadership,
  outage, rescue, towing, concurrent salvage-right and post-H2 milestone rules. New intersections
  remain explicit in doc 27 §9; no code work is included in this change.
- **2026-10-28:** Draft 0.4 closes Q-F1–Q-F6. No-player sorties have a real human NPC commander;
  every unit-assigned/leadership-eligible NPC embarked at the carrier's last-port departure.
  Active-player priority, bounded mission grants and dynamic assignment over real personnel remain.
