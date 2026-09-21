# Perception and Visibility

What you perceive is decided **per observer**, not once for the whole room.
Two people standing in the same place can be shown different scenes: whatever
hides a character removes them from *your* view — the room description, `look`,
`examine`, and even speech you would otherwise overhear — while another watcher,
keener or carrying the right sense, still sees and hears them normally. There is
no "you sense a presence" tell: to an observer who fails to notice a concealed
character, that character emits **true silence** — they are simply not there.

With nothing concealing anybody, everyone perceives everyone, exactly as always;
the projection only ever *subtracts* a subject from the view of an observer who
cannot perceive them.

## The worked example: what a tank's sensors have earned

The simulator is where this rule is easiest to watch, because a tank's picture
of the arena is nothing but per-observer perception with the arithmetic shown.
A pilot reads their own platform's picture with `CONTACTS`, and it holds exactly
what their sensors have found — no object numbers, no room, no pilot names, and
nothing at all about a unit they have not detected.

  CONTACTS         X2  unknown  bearing 087° (rel 032°)  far  450 m  1 s
                   BA  blue     bearing 154° (rel 099°)  close 100 m  0 s

A return your platform has found but not told apart has no name to give you, so
your platform numbers it for itself — `X1`, `X2`, in the order **your** sensors
found things. Those numbers are yours alone: your teammate's `X2` is not your
`X2`. Holding the return long enough to resolve it is what earns the real name,
and the panel says so once:

  Contact X2 identified: BA, blue.

Perception is not a ratchet. Lose the contact and find it again and it is an
unknown return once more, and the old name stops working the instant you are
told:

  BA is unidentified again: X2.

The refusal is the same either way. `LOCK BA` for a contact your sensors have
never held answers `No such contact.` — the identical line a designator that
means nothing at all answers — **deliberately**, because a refusal that told you
`BA` existed would be telling you where the other side is. A refusal is never an
existence oracle, here or anywhere else in the game.

`SCAN` shows the same rule one layer deeper: knowing a thing is there is not
knowing what it is made of. A reading is earned twice over. The contact has to
be identified, because a section name says what kind of machine it is —

  SCAN X2          X2 is not identified yet; there is nothing to read.

— and your platform has to have had a **close look** at it, well inside the
range at which it can merely detect something:

  SCAN BA          No reading on BA yet: close the range for a look.

Drive closer and look again. A teammate's reading reaches you over the mesh the
same way their plot does, and it is as old as *their* look, not yours.

What you are told about your **own** tank is the other way round and complete —
armour, internals, section by section — and it carries nothing whatever about
who shot you. Where a round came from is something only a sensor may tell you.
`help cockpit` is the whole of the pilot's side of this.

## Power use and other acts are perceived, not broadcast

The same director runs outside the arena. When an act is flagged as one that
onlookers must *notice* rather than simply be told about:

  * **you** always perceive your own act;
  * every other character present makes a single silent notice roll against a
    difficulty set by how subtle the act is;
  * a watcher who succeeds is shown a short in-fiction line describing what they
    sensed; a watcher who fails is shown **nothing at all** — true silence, no
    dice line, no tell.

A subtle act can be gated further, so that only an observer with the right
sense makes a roll at all; an overt one still emits a single obvious line to the
whole room, and the sharpest-eyed onlookers additionally get the richer tailored
line. NPCs are observers here too: an NPC that notices something can quietly
file it as intel for its principal (see `help intel`), while one that perceives
nothing files nothing. A plain `+roll` is unchanged — it reads to the room
exactly as before.

## Notes

  * Concealment is **volatile**: it does not survive a disconnect or a reboot.
  * Moving rooms, or the observer moving away, clears a remembered verdict; a
    fresh concealment is always a fresh roll.
  * Staff are **not** specially exempt from the perception gate — a concealed
    character is hidden from a Wizard's `look`/`examine` the same as from anyone
    without the sense to pierce it. Broader staff-visibility tooling is future
    work.

See also: `help cockpit`, `help simulator`, `help intel`, `help npc`.
