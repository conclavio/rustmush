# Passengers

Some people you meet need a lift. Speak to them as you would any other NPC;
their plans can come up in conversation. Leila Mensah-Kovács is at the Shift
Board, the mess and bar off Stickney's Yard Concourse on Phobos. Her calibration
job is finished and she wants to return to Ceres Freeport.

At Ceres's Slack Line, Nadia Okafor-Sato is a maintenance scheduler arranging a
trip to Stickney. `+passage Nadia` reads her current situation. Ma can introduce
you through an optional quest, but meeting, talking with or carrying Nadia does
not require it. Neither an introduction nor a quest commits you to a booking.
Both travellers are shared, persistent people; check the live service rather
than assuming they are still looking for a captain.

Conversation can produce a real fare quote. A promise in dialogue is not a
booking: the captain must accept the exact quote the service prints.

  +passage <npc>                       Check passage status.
  +passage/offer <npc>[=<credits>]     Request a fare quote or counteroffer.
  +passage/accept <npc>=<quote token>  Accept that exact quote.
  +passage/board <npc>                 Arrange boarding at the departure port.
  +passage/cancel <npc>                Cancel when the current passage permits it.

For example, ask Leila where she is headed, discuss the fare, and use the
acceptance command printed with her quote. The service checks the captain and
ship again when an action is submitted. Quoting, accepting and boarding are
separate steps; check `+passage Leila` when unsure which has happened.

The captain can arrange boarding from the departure station or aboard the booked
ship. The passenger walks through real station exits to its berth, boards through
the airlock and settles into the crew compartment when one is available. Arrange
this before filing the ship's departure clearance.

A passenger is a person aboard the real ship. Plan the passage with enough
reaction mass and voyage provisions for everyone aboard (`help trade` and
`help orders`). The fare is held under the passage agreement and paid on its
verified completion. Merely claiming to have arrived does not complete it.

After arrival, the passenger remains the same NPC, with the same memories and
relationships. Restarting the server or reapplying their content does not put
them back in the bar, create a second person or refill their travel money.

After Leila's completed, paid passage, you can meet her again at Ceres's Visiting
Berth Gallery. `+quest Leila` checks for her authored follow-up, "A second
reading". It uses the same person and relationship; see `help quests`.

## Authoring another traveller

A traveller uses an ordinary `npc` entry plus a `passenger` entry in the same
content pack. Keep their stable entry IDs when revising dialogue. Persona,
background, public lore and goals give them a reason to travel; the passenger
entry supplies the engine's route and financial limits.

An NPC can start in a room from an explicitly required pack:

```json
{
  "npc.visitor": {
    "kind": "npc",
    "def": {
      "name": "Nadia Visitor",
      "room_pack": "neighborhood.stickney_yards",
      "room": "room.p10_shift_board",
      "profile": {
        "persona": "A patient visitor who enjoys talking about repairs.",
        "goals": ["Arrange passage home to Ceres when conversation turns to travel."],
        "interfaces": ["passenger", "help"],
        "command_refs": ["+passage", "+ship"]
      }
    }
  },
  "passenger.visitor": {
    "kind": "passenger",
    "def": {
      "npc": "npc.visitor",
      "origin": "stickney",
      "destination": "ceres",
      "opening_fare_minor": 20000,
      "max_fare_minor": 35000,
      "funding_minor": 35000
    }
  }
}
```

This is an entries fragment, not a complete pack. Declare the room's owning
pack and its exact version in the manifest's `requires`; take the version and
any optional digest pin from the installed shelf. The route strings are registered
port venue IDs, such as `stickney` and `ceres`, rather than room or pack names.
Omitting `room_pack` retains ordinary same-pack placement.
The referenced room must belong to a healthy, active dependency latch.

Fare amounts are minor currency units: 100 means one credit. Require
`0 < opening_fare_minor <= max_fare_minor <= funding_minor`, with funding at
most 100,000,000 minor units. The maximum and funding are staff-only
configuration; keep them out of persona, goals, lore and fallback lines.
The engine derives the traveller identity from the pack and passenger entry IDs.
There can be only one passenger role per NPC.

Give the NPC the `passenger` interface to read its status and offer a fare to
the current asker (`passenger.status` and `passenger.offer`, whose optional
`fare_minor` argument is in minor units). These tools cannot accept on the captain's behalf. Keep
fallback lines useful with the model disabled, without asserting that an
agreement, payment or boarding has already happened.

The shipped `tacsim.passengers.stickney@2` is a separate pack. A current world
installs it at its next boot; a world with older Ceres or Stickney content waits
for staff to upgrade those dependencies. Use `@atlas/pack plan <pack>` and
`@atlas/pack apply <fingerprint>`, then apply the passenger pack or restart.
Existing station versions are never upgraded automatically.
The independent `tacsim.passengers.ceres@1` adds Nadia at the Slack Line using
the same traveller template and dependency rules. It does not require the
optional `tacsim.quests.first_steps` pack.
Worlds that already applied passenger version 1 can plan and apply version 2
through the same commands. The upgrade adds Leila's quest role to her existing
identity, including if she has already reached Ceres.

Reapplying the passenger role changes source configuration only. Live booking
state, funds, location, inventory and memories remain. Removing a live traveller
role through pack retirement is refused.
