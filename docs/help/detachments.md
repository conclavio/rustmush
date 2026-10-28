# Detachment preparation

Use `+sortie/plans` to see your private presets and editable mission drafts.
Use `+sortie/invitations` to see drafts that assign you a player slot. These
commands prepare a plan; activation and physical launch are later steps.

A plan has up to six numbered command slots. Loading a preset creates an
independent draft, so editing a mission does not rewrite your saved preset.
Several plans may refer to the same unit without reserving it.

| Command | Purpose |
|---|---|
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
An accepted or ready declaration does not grant control, reserve equipment,
hire crew or authorize a launch. Permissions and control are checked again
when the later activation and launch steps are admitted.

OOC practice continues to use `+sortie/practice` and `+sortie/roster`, described
in [cockpit help](cockpit.md).
