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
  +chronicle/archive [page]
                      Older canon than the recent view holds, a page at a time
                      (page 0 is the newest archived page). The page arrives a
                      moment later, read from the store without pausing play.

Each entry is a one-line summary of a consequence, with the characters
involved. What you see is filtered: some canon is public knowledge, some is
known only to those it concerns, and some is staff-only until the world
discovers it.

## Staff

  @canon              (staff) The full recent ledger: every canon event with
                      its id, visibility scope, and entities — retcons included.
  @canon/archive [page]
                      (staff) The same, over the archived pages older than the
                      recent ledger.
  @canon/retcon <id>=<reason>
                      (staff) Supersede canon event <id>. This does NOT erase
                      history: it appends an auditable retcon that marks the
                      target as no longer current and records why. Anything
                      derived from the old event keeps its reference, so staff
                      can see what a change affects.

## How canon is made

Domain systems record canon events themselves, as a deliberate act — not every
line spoken becomes canon. "Alice said the harbourmaster was sacked" is part of a
scene's transcript (see `+recap`); "the harbourmaster was sacked" becomes canon
only when it is actually established. Today the scene system records a scene's conclusion; the
faction, combat, and obligation systems will record their consequences as they
come online.

`+chronicle` shows the recent tail the server keeps in memory; everything older
is on disk and reads a page at a time with `+chronicle/archive`.

## See also

  +chronicle, +recap, help timestop
