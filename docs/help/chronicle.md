# The chronicle (canon)

The **chronicle** is the world's memory of what became *true* — the durable
consequences of play, as opposed to the moment-to-moment transcript of a scene.
When a scene concludes, when territory changes hands, when a debt is sworn or a
character dies, a **canon event** is recorded. The chronicle is append-only:
history is never rewritten, only added to (see retcons, below).

## Reading the chronicle

  +chronicle          The recent canon you know of, newest first — the
                      "previously on..." of the world. You see public events
                      and those you were personally involved in. Events that
                      have since been retconned are marked.

Each entry is a one-line summary of a consequence, with the characters
involved. What you see is filtered: some canon is public knowledge, some is
known only to those it concerns, and some is staff-only until the world
discovers it.

## Staff

  @canon              (staff) The full recent ledger: every canon event with
                      its id, visibility scope, and entities — retcons included.
  @canon/retcon <id>=<reason>
                      (staff) Supersede canon event <id>. This does NOT erase
                      history: it appends an auditable retcon that marks the
                      target as no longer current and records why. Anything
                      derived from the old event keeps its reference, so staff
                      can see what a change affects.

## How canon is made

Domain systems record canon events themselves, as a deliberate act — not every
line spoken becomes canon. "Alice said the Prince is dead" is part of a scene's
transcript (see `+recap`); "the Prince is dead" becomes canon only when it is
actually established. Today the scene system records a scene's conclusion; the
faction, combat, and obligation systems will record their consequences as they
come online.

Only recent canon is shown; reading deep history is the archive, arriving with
the recap tools.

## See also

  +chronicle, +recap, help timestop
