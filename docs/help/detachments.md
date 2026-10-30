# Detachment preparation

Use `+sortie/plans` to see your private presets and editable mission drafts.
Use `+sortie/invitations` to see drafts that assign you a player slot. These
commands prepare a plan. Activate a complete draft to reserve its assignments,
then use `+sortie/launch` at a surface berth with a ramp and vehicle bay.

To hire an NPC, meet them at a port and use `+sortie/crew`. It lists the available
people in your room, their agreed duties and whether they can command a detachment.
Use the exact `+sortie/hire #<NPC>=<terms>` command printed beside their terms.
If the terms change or the NPC leaves, inspect the current list again.
`+sortie/contract #<NPC>` shows your accepted agreement, even after leaving port.
Only the employer and the hired character can inspect that contract.

Hiring does not move the person or assign them a vehicle. Use `+sortie/board <npc>`
to arrange boarding under your recorded crew contract. Give their local name or
a # followed by their numeric ID; the contract display prints the boarding command. As with passenger
boarding, you may arrange this from the ship's current port or aboard your ship,
while it is alongside and before filing departure clearance. The crew member
walks through real station exits to the berth, uses the airlock and settles into
the crew compartment. Locked routes and full berths can stop boarding; clear the
problem and invite them again. This is an explicit boarding action, not an ongoing
follow order.

Preparation and launch will check who is currently inside
the carrier. Hiring currently records employment and scope without charging or
accruing wages; future pay will follow contract terms rather than time aboard.

A plan has up to six numbered command slots. Loading a preset creates an
independent draft, so editing a mission does not rewrite your saved preset.
Several plans may refer to the same unit without reserving it.

| Command | Purpose |
|---|---|
| `+sortie/crew` | Inspect NPCs currently available for hire in this port room |
| `+sortie/hire #<NPC>=<printed terms>` | Accept the displayed employment terms |
| `+sortie/contract #<NPC>` | Inspect your recorded crew agreement |
| `+sortie/board <npc>` | Arrange boarding for an NPC you hired, using the ship's real berth |
| `+sortie/preset <name>=<plan JSON>` | Create a private preset |
| `+sortie/copy <preset>:<revision>=<name>` | Copy one of your presets |
| `+sortie/load <preset>:<revision>` | Create a draft from a preset |
| `+sortie/draft <plan JSON>` | Create a draft directly |
| `+sortie/plan <draft>` | Read your draft and current preparation report |
| `+sortie/plan <organizer>:<draft>` | Read a draft that assigns you a player slot |
| `+sortie/edit <draft>:<revision>=<plan JSON>` | Replace your draft's material selections |
| `+sortie/save <draft>:<revision>=<name>` | Save a private preset copy |
| `+sortie/discard <draft>:<revision>` | Discard your draft and its invitations |
| `+sortie/drop-preset <preset>:<revision>` | Delete your private preset |
| `+sortie/activate <draft>:<revision>=mining\|combat\|recovery` | Accept the current assignments and reserve their units and people |
| `+sortie/active <draft>` | Inspect your accepted roster and release command |
| `+sortie/active <organizer>:<draft>` | Inspect an accepted roster that assigns you a player slot |
| `+sortie/deactivate <draft>:<activation revision>` | Release the accepted assignments |
| `+sortie/preflight <draft>:<activation revision>` | Inspect current launch facts for your accepted roster while aboard its carrier |
| `+sortie/launch <draft>:<activation revision>=all` | Admit the unlaunched members of your accepted roster |
| `+sortie/launch <draft>:<activation revision>=<slot>` | Admit or retry one unlaunched member |
| `+sortie/operation <draft>:<activation revision>` | Read the operation's saved progress and flight references |
| `+sortie/recover <draft>:<activation revision>=<slot>` | Bring a unit physically beside the ramp back into the carrier |
| `+sortie/cancel-launch <draft>:<activation revision>=<slot>` | Cancel a departure that is still physically aboard |
| `+sortie/end <draft>:<activation revision>` | End coordination, cancelling only departures still aboard |
| `+sortie/history <unit>:<flight>` | Read an authorized flight's departure and return history |

`+sortie/active` prints the preflight command for the accepted activation. Preflight
uses those accepted assignments even if you subsequently edit the draft. It
checks current containment, reservations, NPC agreements, unit condition, your
Deploy right and the assigned operator's OperateCrew right. It also checks for
a bay-equipped carrier settled at a field site and a filed departure clearance.
Unit power, control, movement and weapons are reported separately; an unarmed
mining vehicle is not treated as an armed combat vehicle. This is an inspection,
not launch permission. Actual station control, crew capability and any selected
mind still require admission at launch. Preflight moves nobody and stores no
readiness flag.

Launch requires the assigned operator inside the real unit at its helm console,
with its actual control lease. NPCs need a real helm billet as well as their
employment agreement; hiring and activation do not create a station or take
control. The carrier must be settled at a field site, with the unit in its
vehicle bay. You need Deploy and the operator needs OperateCrew. Launch uses
the existing 300 IC second ramp transit. Returned units may launch again within
the same operation, creating another flight and hull epoch.

Departure progress is saved separately for each unit. If someone leaves their
station or a physical step refuses, departed units stay out and the remainder
stays pending. Restore the prerequisites and retry, or cancel that member while
it is still aboard. Already departed members are reported and never launched
twice. Recovery requires RecoverLoad and a unit actually beside the carrier's
ramp; it does not drive or teleport a distant unit home or repair damage.
Ending an operation leaves field units and their epochs unresolved until a real
return. Saved progress describes confirmed events, not a remote casualty sensor.

`+sortie/plan` prints the current accept, ready and decline commands for your
assignment. Use the complete printed command. Accept first, then mark ready
while personally seated at your unit's helm aboard the selected carrier.
The organizer cannot accept or mark ready for you. If a command is stale, read
the plan again. Changing units, controllers, command priority, commander,
profiles or financial selections requires renewed acceptance and readiness.
Declining removes that draft from your invitation list and frees a slot. You
can still read the assigned draft by its reference and accept again. An edited
plan may invite you again.

You can keep eight presets and eight drafts. The first interface uses explicit
plan JSON. Object IDs below are examples; replace them with your carrier, unit
and player IDs. Slots must be in ascending order, from one to six. Include
optional fields as `null` when no selection is made.

```json
{
  "carrier": 100,
  "slots": [
    {
      "slot": 1,
      "unit": 101,
      "controller": { "kind": "player", "character": 42 },
      "role": "scout",
      "mind": null
    }
  ],
  "npc_commander": null,
  "financial_terms": null
}
```

A missing unit may be `null`; an unassigned controller is
`{"kind":"unassigned"}`. A real NPC preference uses
`{"kind":"npc","character":43}`; an automated preference uses
`{"kind":"automated"}`. A mind preference is
`{"licence":7,"profiles":{"role":"scout"}}`. These are selections for later
validation, not proof that a person, unit, licence or profile is available.

The report checks current physical presence and your current unit permissions.
It also identifies prerequisites that preparation cannot establish yet.
While the organizer is aboard the selected carrier, the report resolves each
NPC slot against current hull occupants and that organizer's crew contracts.
It reports missing people, missing employment and commander scope that the
agreement does not cover. Leaving the ship changes the next report immediately;
returning through ordinary movement restores current presence. Invited players
can inspect the plan without seeing the organizer's private employment terms.
These observations do not reserve a unit or establish pilot capability or health.
An accepted or ready declaration does not grant control, reserve equipment,
hire crew or authorize a launch. Permissions and control are checked again
when activation and launch are admitted.

Activation requires the organizer, each real unit and each assigned person
currently aboard the chosen carrier. Each NPC must be hired by the organizer
under a contract covering the selected duty and any commander assignment.
Players must be ready for this exact draft and remain at their actual helm
with a current control lease. The organizer needs deploy permission for each
unit; its operator needs operate/crew permission. Hiring does not grant those
unit permissions. Automated controllers, mind selections and financial terms
remain unavailable for activation in this first cut.

Activation reserves the selected units and people exclusively and saves the
accepted roster. Editing the draft does not change that roster or release its
reservations. Inspect `+sortie/active` and use its release command before
activating another revision. Release remains available if someone leaves or
permissions change; it does not move people or units. Ordinary walking remains
available while reserved. Activation establishes assignments, not launch
readiness: launch still needs current containment, control, capability/health
and the applicable mission-role grants.

Launch requires each operator inside their assigned unit at its real console
with a current helm lease. Automatic seating and control acquisition for hired
NPCs are still being integrated. Each departure takes the existing ramp duration;
units complete independently. A blocked departure remains pending and can be
retried or cancelled while still aboard. Inspect `+sortie/operation` for confirmed
progress and individual flight references. History is available to the original
organizer and assigned player participants; it is not a remote casualty sensor.

Recover a mobile unit after bringing it beside the carrier's ramp. Recovery
requires current recovery permission and preserves damage. Disabled units need
physical towing or loading. Ending an operation leaves field units and their
epochs unresolved; it does not recall them. Returned units may launch again.
Assigned operators can use `<organizer>:<draft>:<activation revision>` to identify
their operation when recovering a unit or reading its history.

OOC practice continues to use `+sortie/practice` and `+sortie/roster`, described
in [cockpit help](cockpit.md).
