# Informants and Intel

Some NPC agents are **informants**: staff can set an NPC to quietly report what
it witnesses to a **principal** (its domitor, patron, or employer — a player).
The NPC files short, provenance-tagged **intel records**; the principal pulls
them on demand with `+intel`. Nothing is ever pushed at you — you debrief your
assets when you want to.

The one rule that governs everything below: **an informant can report only what
its NPC actually perceived.** Filing rides on the same perception the NPC uses to
notice anything at all (see `help perception`), so a subject your asset could not
sense — a vampire cloaked past its senses, someone who was never in the room —
produces no record. Your asset never invents intel.

## Pulling your intel (`+intel`)

  +intel              List everything your informants have filed FOR YOU,
                      newest first: the subject, what happened, where, how it
                      is known, and how long ago.
  +intel <subject>    Show only what has been filed about one subject (by name,
                      or a dbref like #42). A subject who has since left is still
                      findable by the name your asset recorded.
  +intel/clear        Clear your own intel log.

You only ever see records filed for you. Two principals running assets in the
same room keep entirely separate logs; there is no way to read another player's
intel.

A record reads roughly:

    5m ago  Vlad [concealed] in The Tap Room: arrived (though they were trying
            to stay hidden)  (witnessed directly by Bartender)

The bracketed word is the kind of event: `arrived`, `departed`, `present`,
`spoke`, or `concealed` (your asset caught someone trying to stay hidden).

## What triggers a report

An informant files an event when **all** of these hold:

- staff have set the NPC's **principal** to you;
- the NPC is **loyal** (a resentful asset reports nothing) and **enabled**;
- the NPC **perceived** the subject (the load-bearing rule above); and
- the event is **reportable** — either the subject is on the NPC's **watch**
  list, or the subject was under active concealment and the NPC **pierced** it.

A pierced concealment is *always* filed, watch list or not — catching someone who
is trying not to be seen is inherently notable. With an empty watch list and an
ordinary, unconcealed subject, nothing is filed, so the log stays signal-rich.
An informant never reports on itself or on its own principal.

Each record carries a **provenance** string — a short "how the NPC knows" note
(for now, "witnessed directly by <informant>"). This is the first seed of the
game's wider epistemic model: intel is graded by how it was learned, not treated
as omniscient fact.

## Configuring an informant (staff)

Set the informant fields on the NPC's profile with `@agent/set` (Builder+):

  @agent/set <npc>/principal=<name>   Who this NPC reports to. Resolved to a
                                      character at set-time; an empty value
                                      (`principal=`) clears it — the NPC then
                                      files nothing.
  @agent/set <npc>/loyal=<yes|no>     The willingness gate. yes (default): a
                                      loyal asset files what it perceives. no:
                                      it reports nothing, however much it sees.
  @agent/set <npc>/watch=<a>;<b>      Subjects of interest (";"-separated names
                                      or topics). An empty value watches nobody,
                                      so only pierced concealments are filed.

`@agent/show <npc>` reports an informant's principal, loyalty, watch list, and
how many records it has filed so far.

Example — a ghouled bartender who reports every notable face to Marcus and always
flags anyone trying to stay hidden:

  @agent/set Bartender/principal=Marcus
  @agent/set Bartender/watch=the Prince; the sheriff
  @agent/show Bartender

The loop is entirely opt-in and additive: an NPC with no principal set behaves
exactly as it did before, files nothing, and costs nothing.

## Not yet built

Reporting on **power use** (an NPC noticing and relaying that someone worked a
Discipline or cast a spell) is future work: it waits on the Storyteller
perception-director that will adjudicate what a witness understands of an effect.
Today an informant reports presence, movement, speech, and pierced concealment.

## See also

  +intel, help npc, help perception, @agent
