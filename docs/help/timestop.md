# Timestop and scenes

A **timestop** freezes the current room into a durable, turn-based scene. It is
how you run structured play — a fight, a tense negotiation, anything that wants
turn order — without the room wandering off around you.

While a timestop is running:

- **Everyone present is held in place.** You cannot walk out an exit, go home,
  or be teleported away until the scene ends (staff can override to extract
  someone). This keeps the cast together for the whole scene.
- **NPCs stop acting on their own.** A bartender caught in the timestop won't
  strike up autonomous conversation mid-fight; it acts only when a facilitator
  drives it.
- **Turns and rounds are tracked.** Participants act in order; when everyone
  has acted the round advances.
- **The scene keeps its own clock.** IC time inside the scene is the moment it
  began plus the rounds elapsed — a fight played out over several real days is
  still only seconds of in-character time, and the outside world's clock keeps
  running normally.

A scene has a durable number (`scene #N`) and **survives a server restart**: if
the game bounces mid-scene, the timestop comes back **paused**, exactly where it
was, and the facilitator resumes it deliberately — turns never advance by
themselves across a restart.

## Commands

  +timestop           Freeze this room into a turn-based scene. You become its
                      facilitator. Everyone present (players and NPCs) becomes a
                      participant, in dbref order.
  +timestop/who       Show the scene here: round, whose turn, participants.
  +timestop/next      (facilitator/staff) Advance to the next participant's
                      turn; wrapping past the last starts a new round.
  +timestop/pause     (facilitator/staff) Freeze turn advancement.
  +timestop/resume    (facilitator/staff) Resume — also how you restart a scene
                      that came back paused after a server restart.
  +timestop/end       (facilitator/staff) End the scene and release everyone.

  +scene              Show the scene you are in: its status, the round and whose
                      turn it is, and the participants in turn order (your own
                      position marked).

The facilitator is whoever started the timestop; staff (Royalty and up) can
drive or end any scene.

## Not yet

This first cut gives you the turn-based *envelope* — the freeze, the initiative
order, the durable/restart-safe record. It does not yet route dice or other
mechanical commands through the turn (an out-of-turn `+roll` is not rejected),
record an immutable transcript, or roll system initiative automatically. Those
build onto the same scene next.

## See also

  +timestop, +scene
