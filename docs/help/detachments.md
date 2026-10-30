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

Renting a unit does not supply its operating rights. For Occator's actual rental,
request the separate four-right equipment agreement at the counter with
`+tv/<n>/permissions go`, then inspect its committed loans. Lend the hired NPC
OperateCrew through the printed operating loan ID using
`@unitgrant/issue #unit=#npc operate_crew parent=<loan>`. See
[unit permissions](unit-grants.md) for delegation and revocation.

From the carrier's cargo hold, use `+tv/bay in` to reach its vehicle bay.
After returning and leaving your rig, `+tv/bay out` takes you back to the hold.
This access stays inside the actual carrier and does not cross its ramp.

A player can take their own rental's physical helm with `+tv/<n> board` while
standing in the carrier's vehicle bay. This requires current OperateCrew permission
and a vacant billet. It prints the real unit ID and the `+focus` and
`@station/handoff` commands to choose and request control personally. Boarding
itself grants no control lease. Take the station before marking the draft ready.

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
| `+sortie/chat` | Show your accepted detachment's IC channel and personal alias |
| `+sortie/chat [<organizer>:]<draft>:<activation>=<message>` | Queue a chat line to that accepted detachment |
| `+sortie/active <draft>` | Inspect your accepted roster and release command |
| `+sortie/active <organizer>:<draft>` | Inspect an accepted roster that assigns you a player slot |
| `+sortie/deactivate <draft>:<activation revision>` | Release the accepted assignments |
| `+sortie/preflight <draft>:<activation revision>` | Inspect current launch facts for your accepted roster while aboard its carrier |
| `+tv/<n> board` | Take your own rental's physical helm billet from the carrier's bay |
| `+tv/<n> leave` | Release your own control, then step into the bay and give back your billet |
| `+sortie/unseat <npc>` | Invite your contracted operator out of a returned unit |
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

## Detachment chat

An accepted roster has its own private IC channel. Use `+sortie/chat` to see
its name and your personal alias, normally `dt`. If that alias already belongs
to another channel, use the alias printed for you. All six assigned people may
participate; organizing or owning the carrier alone does not join the channel.

```
+sortie/chat
+sortie/chat =Ready beside the ramp.
dt Ready beside the ramp.
dt :signals to the crew.
@channel/recall dt
```

Use the printed alias in place of `dt` where necessary. You can also select a
roster explicitly with `+sortie/chat <organizer>:<draft>:<activation>=<message>`.
Normal channel tabs and recall show only lines that have actually reached you.
"Queued for IC transit" confirms submission, not reception by another member.
Lag, jamming, outages and transport capacity apply through the ordinary
communications system. Pending lines and your received history survive restart.

Chat text is conversation. Writing `HOLD` or `WILCO` in chat does not issue or
acknowledge an order; keep using the existing fenced order and response commands.
The channel does not show remote connection or listening status.

Chat is available during preparation, deployment and recovery. Ending operation
coordination does not close it while its accepted roster remains active.
Deactivating the roster closes chat and cancels outstanding traffic; a new
activation gets a separate channel and no previous activation's history.

You may leave with `delcom <alias>`. Use `+sortie/chat` again to request rejoining
while you are still assigned. After that request commits, repeat the command to
see your alias. Rejoining starts with new traffic and does not recover missed
or previously discarded lines. Accepted rosters from before chat was installed
can use the same command to request their channel without changing assignments.

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
After a confirmed return, use `+tv/<n> leave` for yourself and
`+sortie/unseat #<NPC>` for your hired operator. If the operator still holds K6
control, the first call requests their own release through the normal handoff
state machine. Once that release settles, repeat the command to step into the
bay and give back the billet. A player may also use `@station/release #unit/pilot`
explicitly while still at the console. Nobody is moved before control release
commits. Exit cannot race an open ramp transit; cancel a departure first or wait
for return confirmation. Field units must physically recover before anyone can
step into the carrier.

The rental counter and charter repossession inspect the rig's current room
contents and refuse to take it to the shed while anyone is inside, including
people inside carried containers. NPC unseating needs the employer aboard the
actual carrier and the recorded crew contract. No boarding history is required.

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

At the Slack Line in Ceres, `+sortie/crew` lists Ilya Mendes, a mining/recovery
operator, and Sana Okonkwo, available under commander-capable mining/recovery
terms. They are permanent people. Once hired or away, they are not replaced by
another copy at boot. Use the printed acceptance command, then `+sortie/board`.
Hiring supplies the contract only; unit rights and actual control remain separate.

After a mining deployment, the physical operator of a leased stock rig can use
its individual `+tv/<n> drive`, `scan`, `sound`, `mine`, `recover`, `dump` and
`unload` controls. They must remain at the rig's real console with control and
permission to operate it. A hired mining operator uses their own readings and
the employer's mining licence and claim rights; field leadership supplies
neither. A loaded recovery unloads into the carrier after the physical return
is confirmed. Launch the next flight with `+sortie/launch` before starting more
field work. Field crews use individual orders rather than the carrier's
multi-trip `+tv/<n> plan` controls.
An idle field crew waits for orders rather than recalling the unit automatically.

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
do not themselves move a unit.

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
ORDER DT:12:1:3 2 MINE metals
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
Contracted NPC mining crews execute delivered HOLD, ADVANCE, WITHDRAW and MINE
orders through their real controls. MINE [family] is one finite trip: use their
own scan and sounding, try up to eight nearby known ore pockets, mine one cycle,
recover and unload. A failed action or no reachable known ore produces NEGATIVE. Send a fresh
order for another trip after the next DT launch. HOLD and RELEASE stop a mining
run after its current cycle; admitted movement still settles at its actual end.
Unsupported combat and support objectives receive NEGATIVE until those field
hosts exist. Players continue to control their own units.

Executable orders belong to the exact deployed flight and site. A delayed message
cannot become a command for a later launch. WILCO, attempted work and completion
remain separate: only actual source action admission records an attempt, and only
matching movement or inventory receipts record completion. The commander sees only
replies whose return messages arrived; the report keeps execution unconfirmed
rather than exposing the recipient's private work history as instant telemetry.
