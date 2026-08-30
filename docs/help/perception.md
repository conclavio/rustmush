# Perception and Visibility

What you perceive is decided **per observer**, not once for the whole room.
Two people standing in the same place can be shown different scenes: a power
that hides a character removes them from *your* view — the room description,
`look`, `examine`, and even speech you would otherwise overhear — while another
watcher, keener or gifted with the right sense, still sees and hears them
normally. There is no "you sense a presence" tell: to an observer who fails to
notice a concealed character, that character emits **true silence** — they are
simply not there.

With no concealment in play, everyone perceives everyone, exactly as always;
the projection only ever *subtracts* a character from the view of an observer
who cannot perceive them.

## Vampire: Obfuscate vs Auspex

The first worked example is the Kindred Discipline of concealment,
**Obfuscate**, opposed by the sight-sharpening Discipline **Auspex**.

  +obfuscate          Show whether you are currently concealed.
  +obfuscate on       Draw the shadows about yourself and fade from sight.
                      Requires Obfuscate 1+. Everyone present rolls once, then,
                      to notice you.
  +obfuscate off      Let the shadows fall away and step back into view.

When you cloak, every character in the room makes a single **piercing roll** —
Perception + Awareness, plus their Auspex — against a difficulty set by your
Obfuscate. The result is fixed for that scene: a watcher who pierces the veil
keeps seeing and hearing you until you move, they move, or you de-cloak; a
watcher who fails is shown nothing of you at all. The roll is made once and
remembered, never re-rolled per line of speech, and it is logged privately so
staff can audit it (`+roll/last` on the roller).

A **mundane sense is not enough**: without Auspex (or a comparable supernatural
sense) an observer cannot pierce Obfuscate no matter how high their Perception
and Awareness — they make no roll and simply do not perceive you. NPCs are
observers too: a ghoul bartender with Auspex may notice a cloaked vampire and
react; one without will carry on as though the room were empty of you.

While you are cloaked from an observer, that observer:

  * does not see you listed in the room or in `look`;
  * does not see you in `examine`'s contents, or in a container's;
  * never hears you `say`, `pose`, `semipose`, or `@emit`;
  * (if an NPC) is not woken by your speech, arrival, or departure.

You always perceive yourself, and you still perceive everyone else normally —
concealment hides *you from others*, it does not blind you.

## Power use is perceived, not broadcast

Using a supernatural power — a Vampire Discipline, a Mage working, a Werewolf
Gift — runs through the *same* per-observer projection. When you invoke a
power, the mechanical roll no longer floods the whole room with a raw dice
line. Instead:

  * **you** always perceive your own act;
  * every other character present makes a single silent notice roll —
    Perception + Awareness, plus **Auspex** for a numinous act — against a
    difficulty set by how subtle the power is;
  * a watcher who succeeds is shown a short in-fiction line describing what they
    sensed; a watcher who fails is shown **nothing at all** — true silence, no
    dice line, no tell.

A **subtle** power (Dominate, Presence, Obfuscate, Auspex — anything that works
on the mind or the senses) is *hard* to notice and can only be noticed at all
by an observer with the right supernatural sense: without Auspex, a mundane
onlooker makes no roll and never learns the power was used. A **vulgar** or
overtly physical act (a blur of Celerity, a fireball) is *widely* read: it
still emits a single obvious line to the whole room so no one is blind to it,
and the sharpest-eyed onlookers additionally get the richer tailored line.

NPCs are observers here too: a ghoul bartender with Auspex who notices a
vampire bend someone's will can quietly file that as intel for its domitor (see
`help intel`), while a mundane onlooker in the same room perceives nothing and
files nothing. Plain `+roll` and non-power checks are unchanged — they read to
the room exactly as before; only acts flagged as a *power* route through the
director.

## Notes

  * Concealment is **volatile**: it does not survive a disconnect or a reboot.
    Re-cloak after reconnecting.
  * De-cloaking, moving rooms, or the observer moving away clears the remembered
    verdict; a fresh cloak is always a fresh roll.
  * Staff are **not** specially exempt from the perception gate in this first
    cut — a concealed character is hidden from a Wizard's `look`/`examine` the
    same as from anyone without the sense to pierce it. Broader staff-visibility
    tooling is future work.

The same per-observer machinery drives the **silent witnesses** of an
adjudicated `+action` / `+cast`: when you throw an adjudicated check, only
onlookers who make a Perception + Awareness roll are told what you did, and
everyone else gets true silence. See `help action`.

See also: `help wod`, `help action`, `help +obfuscate`, `help +disc`.
