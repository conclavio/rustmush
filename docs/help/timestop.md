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
- **Mechanical actions wait for your turn.** Dice (`+roll`) and other
  mechanical commands only work on the participant's own turn while the scene
  is turn-based; speech, poses, and OOC are always free. A paused scene blocks
  mechanical actions until it resumes. Each public roll is recorded in the
  scene's log and recap.
- **The scene keeps its own clock.** IC time inside the scene is the moment it
  began plus the rounds elapsed — a fight played out over several real days is
  still only seconds of in-character time, and the outside world's clock keeps
  running normally.
- **Everything is recorded.** The scene keeps an immutable event log — who
  spoke, whose turn it was, who arrived, when it paused. `+timestop/log` shows
  the timeline. (OOC, pages, and whispers are never recorded.)

A scene has a durable number (`scene #N`) and **survives a server restart**: if
the game bounces mid-scene, the timestop comes back **paused**, exactly where it
was, and the facilitator resumes it deliberately — turns never advance by
themselves across a restart.

## Commands

  +timestop           Freeze this room into a turn-based scene. You become its
                      facilitator. Everyone present (players and NPCs) becomes a
                      participant, in dbref order.
  +timestop/who       Show the scene here: round, whose turn, participants.
  +timestop/log       Show the scene's recorded timeline (speech, turns,
                      arrivals) — a plain transcript of what happened.
  +timestop/log/archive [page]
                      Read older history: the sealed pages of a long scene's
                      log, or the whole log of a scene that ended before the
                      last restart. Page 0 is the newest; the page arrives a
                      moment later, read from the store without pausing play.
  +recap              A narrative recap of the scene, told as the events you
                      witnessed (speech, arrivals, the rounds). When the scene
                      concludes it is also recorded in the world chronicle
                      (see help chronicle).
  +recap/archive [page]
                      The same recap over older history, page by page.
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

This cut gives you the turn-based *envelope* — the freeze, the initiative
order, the durable/restart-safe record, and the recorded transcript, with the
archive for anything older than the recent tail. It does not yet roll system
initiative automatically, or narrate a recap with a storyteller's voice; the
recap is the deterministic, witnessed timeline. Those build onto the same
scene next.

## See also

  +timestop, +scene
