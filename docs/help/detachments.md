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
| `+sortie/seat <draft>:<activation revision>=<slot>` | Arrange an accepted NPC operator's physical unit console and control request |
| `+sortie/launch <draft>:<activation revision>=all` | Admit the unlaunched members of your accepted roster |
| `+sortie/launch <draft>:<activation revision>=<slot>` | Admit or retry one unlaunched member |
| `+sortie/operation <draft>:<activation revision>` | Read the operation's saved progress and flight references |
| `+sortie/field <draft>:<activation revision>` | Read the last saved field leadership identity and command term |
| `+sortie/refresh <draft>:<activation revision>` | Recheck the operation's current field leadership |
| `+sortie/orders <draft>:<activation revision>` | Read your sent field messages, arrived replies and personally received orders |
| `+sortie/command-grants <draft>:<activation revision>` | Print grants that a unit's owner or authorized lender can issue to this mission's current leader role |
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
with a current helm lease. For an accepted NPC slot, use `+sortie/seat` while
you and the hired person are aboard the selected carrier. Its unit must be in
the actual vehicle bay and have an authored helm console. You need Deploy
permission and the NPC needs OperateCrew permission; employment alone grants
neither. An occupied billet or another operator's control prevents taking over.
The NPC boards the unit and requests control through its existing station.
Preflight and launch check current facts again after that request settles.
Each departure takes the existing ramp duration;
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

Field leadership follows the accepted numbered slots. The first available player
with a real physical helm station and current control lease leads; launching or
organizing the operation confers no field priority. A nominated NPC can take over
only with the accepted command employment scope and current unit command rights.
An unavailable roster has no leader. Session grace comes from the session owner;
returning players settle briefly before reclaiming priority. A restart retains
the command term and reconnect timers, then rechecks current sessions and control.

`/field` is the saved operation record, not a live casualty or position report.
The flight tick refreshes changed leadership; `/refresh` requests the same durable
check explicitly. Neither command issues an order or moves a unit. These records
do not supply an NPC tactical decision driver.

Use `+sortie/orders` for the current `ORDER DT:<detachment>:<revision>:<term>`
prefill. Assigned players can identify the operation as
`<organizer>:<draft>:<activation revision>`. The current leader can send to a
numbered slot or `ALL`, using the same line vocabulary as practice:

```text
ORDER DT:12:1:3 ALL HOLD
ORDER DT:12:1:3 2 ADVANCE 40,50
ORDER DT:12:1:3 2 ENGAGE
ORDER DT:12:1:3 2 SCREEN 90 HOLD FIRE
ORDER DT:12:1:3 2 SUPPORT 1
ORDER DT:12:1:3 2 WITHDRAW EXPIRES 60
ORDER DT:12:1:3 2 RELEASE
```

Replace the example identity and term with the printed prefill. ADVANCE takes a
field cell within the 256 by 256 site. SUPPORT takes an accepted slot. ENGAGE may
include a contact prefix from your own observation; a hint grants its recipient
no contact knowledge or targeting permission. Orders stand until replaced,
released or completed; an optional expiry suffix limits them in IC seconds.

Sending requires current leadership, physical station control and command rights
for every selected unit. Admission checks the entire `ALL` set before sending.
For the usual mission lending arrangement, use `+sortie/command-grants` after
activation. Each printed `@unitgrant/json` command lends command permission for
one accepted unit to this exact mission's current leader. The unit's owner can
issue it directly; another authorized lender supplies its real parent grant or
organization root in the JSON. Printing a command grants nothing, and the
organizer cannot lend a unit's rights without that authority.

The mission role follows eligible human succession without creating personal
command grants. It cannot be delegated onward and conveys no crew, deployment,
recovery or title right. A changed accepted activation needs fresh grants.
Owners can revoke their role grants through the existing unit-grant commands.
Existing direct command grants continue to work.

Messages use the existing communications transport, including its routing,
latency, interference and queue limits. Arrival rechecks current assignments,
leadership term and permissions before changing the recipient's last delivered
order. A delayed obsolete or expired replacement leaves that older order intact.

An assigned player reads their own arrived directive and can use the printed
`WILCO DT:<detachment>:<revision> <sequence>` or `NEGATIVE ...` response. These
are advisory responses. They never operate the player's unit, report an attempt
or claim completion. The sender sees a response only after its return message
actually arrives. Field transport receipts are retained for 30 IC days; older
recipient history remains readable but cannot manufacture a missing reply path.
NPC execution and actual mining remain subsequent work.
