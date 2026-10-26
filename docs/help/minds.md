# Minds

Your tank's core can fly it when you are not in the seat. SIM MIND chooses its mind; the match's start seals it.
With none chosen, a tank you leave flies Standard Doctrine on Line, the Registry's own; MIND reads it in the cockpit.
It flies from when you climb out to the end, or from a drop past the grace until the end or you board again.

`@mind/doctrine` opens the free Stickney teaching library for Standard Doctrine.
Use `@mind/doctrine Sniper` (or another preset) for its current configured card;
`help doctrine` explains these public pages.

`help simulator` is how you claim a slot and `help cockpit` is what you type in
the seat; this topic is the mind between the two. While you fly, the mind is
dormant and every channel is yours.

## Choosing one

You choose in the lobby, during selection, for the slot you hold:

  SIM MIND                    your choice, what fits your tank's core, and
                              your side's half of the centre's compute.
  SIM MIND Standard           Standard Doctrine, flying Line.
  SIM MIND Standard PROFILE Sniper, Line
                              Standard Doctrine, flying Sniper, with Line
                              sealed beside it to switch to.
  SIM MIND none               back to nothing chosen.

Every character can use Standard Doctrine with its six presets and keep a
private library of authored minds. An authored choice uses a saved revision;
working edits do not change it. For example:

  SIM MIND Patrol@2 PROFILE Line, Cautious

Without a revision, selection resolves your latest saved revision at checkout.
Use a revision number to pin it. A profile is a saved card and its sealed
rules: the first you name is the one it flies, and the rest are the ones you
may switch to once the match runs. Every answer is
said to you alone. `SIM START` closes selection, and the start of the match
seals what you chose onto your tank, where nothing you do changes it but a
switch between the profiles you sealed (staff can edit it; see Afterwards). If
you chose one, your seat reads one line naming it before `Good hunting.`:

  Sealed for this sortie: Standard Doctrine (Sniper, Line), Class I; MIND reads it.

A choice has to fit the tank, and it is refused with its reason when it does
not:

* **The class.** How many profiles a seal keeps is the class the mind thinks
  at on your tank's core: two at Class I, three at Class II, and four at
  Class III. Standard Doctrine is Class I. Authored minds declare their class;
  the core and the centre's pool must support it. A tank with no core above
  Class 0 flies the ROM, the core's own fallback, and keeps one.
* **The floor.** On a tank that carries a gun, no profile may fire at odds
  under the server's Compact floor (10% as shipped). Every preset clears 10%,
  and a hull with no gun is never asked.
* **The pool.** Your side's half of the centre's compute has to carry it, and
  a choice it cannot carry is refused with the arithmetic: how many ops a
  second this tank would think, your side's half, and what is spoken for.

Changing your unit type with `SIM UNIT` fits the choice to the new tank again.
Once the match has started, bare `SIM MIND` reads your choice and says
selection is closed.

A side's machines carry a mind too: `SIM AI MIND` is the lead's choice for the
side's fill, out of the lead's own holdings (staff choose one of Standard
Doctrine's presets for either side). With nothing chosen they fly Standard
Doctrine on Line.

## The six presets

Line is the Registry's doctrine as this server runs it: it closes to 70% of
its reach, fires at 10% or better on any face, keeps no rounds back, breaks
off for its entry zone when its armour and internals fall under 35% and comes
back after 20 quiet seconds, drives a minute to where it last saw a contact,
and goes looking after a minute and a half with nobody in sight. Each other
preset changes the terms it names, in the card's own words, and leaves every
other term at Line's:

  Sentry        min_odds 55: fires at 55% or better. hunt_after_s 0: never
                goes looking. rally_s 0: once it breaks off, it stays home.
  Sniper        band_pct 100: stops at the edge of its reach. min_odds 65.
                reserve_pct 30: keeps 30% of its rounds back.
  Skirmisher    band_pct 85. min_odds 35. aspect flank: fires only at a
                left or right face.
  Hunter        band_pct 60: closes to 60% of its reach. min_odds 30.
                search_s 60: drives a minute to where it last saw a contact.
                hunt_after_s 45: goes looking after 45 seconds alone.
  Rearguard     break_off_pct 70: breaks off for home under 70%. rally_s 0.
                min_odds 50.

Each also names a stance — a screen for Sentry, an orbit for Skirmisher, a
hunt for Hunter and a hold for the rest — and the card carries ten more terms
like it: `opening`, `stop_unseen`, `close_m`, `orbit_sense`, `turn_into_hits`,
`turn_s`, `screen_bearing`, `screen_arc`, `reserve_odds` and `switch_m`. Those
eleven are carried inert: Standard Doctrine does not fly them yet, so a
Skirmisher does not orbit and a Sentry holds no arc. What a preset changes in
how it fights is the list above. On a hull with no gun the fire terms are
inert too.

## In the seat

`MIND` reads your tank's seal, in four lines:

  Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II core.
  Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds.
  Channels: you hold helm, sensors and fire; the mind is dormant while you fly.
  Rules: none sealed; your own armings are under WEAPONSTATUS.

`MIND PROFILE Line` switches the profile the mind will fly to another one you
sealed, at most once in ten seconds, and only from the seat; inside the ten
seconds it answers `MIND refused. One switch in ten seconds; the next in 6 s.`
The switch changes nothing while you fly: it is what the mind flies when you
leave. `MIND RULES [slot]` reads your live pilot rules, separate from the sealed
profile. `MIND RULE <slot> <rule or OFF>` replaces or removes one; `FIRE` and
`LOCK` arm native rules (`help cockpit`). `WEAPONSTATUS` also reads their terms.
`MIND CARD` reads the complete sealed card. `MIND TRACE` reads recent thinks:
operations and fees, rule decisions, emitted intents, their outcomes, faults,
and the author's bounded trace lines. A dormant mind may have no thinks yet.

The TUI and browser show the short mind reading in the weapons pane and open
the complete card, profiles, rules, and trace in a separate readout. Profile
controls prepare `MIND PROFILE <name>` in the command box; press Enter to send.
In the TUI's Flight page, Left/Right chooses a profile and Enter prepares it;
a second Enter sends it. Press t for a trace read or f to follow, enter the
hull number and optional count, and press Enter. The Trace page shows the
permitted result; t reads once and stops following. Card fields are read-only
in flight. A station the hull does not have is
shown as inert, rather than promising that its settings can act.

If the mind faults three times running, or its core is destroyed, the hull
drops to the ROM for the rest of the match: you are told, and `MIND` says so.

## When it flies

It takes the tank the moment you climb out with `EXIT`, and you cannot take it
back. It takes it too when your line has been down past the grace; the board
then reads `flown by its mind` beside your slot, and when you return, `BOARD`
takes the tank back where it stands. Either way it flies the profile you left
it on.

## Who reads it

Your mind is yours. During selection your own row on the board ends with
`, mind Standard Doctrine (Sniper, Line)` for you and for staff, and with
`, a mind` for everybody else; a slot that chose nothing reads nothing. From
`SIM START` on no row names or marks a mind, for anybody. After that only you
and staff read it: bare `SIM MIND`, the seal line, `MIND` and your report are
yours alone, staff read it on their own pages, and another pilot's report,
`SIM HISTORY` and `SIM REPLAY` never name it. A side's fill is read by its lead
and staff during selection, and after the start by the account that chose it
and staff; everyone else reads `The fills are not shown once a match starts.`

## Afterwards

`SIM REPORT` carries a `Your mind` block after its opening lines and before the
timeline of your shots, when you chose a mind or the mind flew your tank:

  Your mind: Standard Doctrine (Sniper, Line); switched to Line at 1:05.
    It flew your tank from 4:42, when you climbed out, to the end.
    Its shots: cannon 4, 2 of them hits; it destroyed nothing.

— the mind and its profiles as sealed, its switches, when it flew, the shots
it fired, and a drop to the ROM with its cause:

  It flew your tank 5 times: first from 0:42 to 1:10, last from 12:05 to the end.
  It dropped to the ROM at 1:30: its core was destroyed.

The report's own counts — `You fired …` above the block and the timeline under
it — keep the tank's every shot, as they always did. A mind flies no wreck:
what it flew ends where your tank was destroyed. Staff can change a hull's
card while a match runs; if they change yours, the block names the card as it
then read, `Sniper, edited (min_odds 70)`, as it names a switch.

## Authoring and proving

Open Minds with F10 in the browser or TUI. Read `help mindcraft` to edit and save
a mind, and `help proving` to run paired trials and inspect a replay.

## Trace after release

After releasing your live tank, read its permitted trace by unit number:

  @mind/trace #42 3
  @mind/trace #42 3 FOLLOW
  @mind/trace #42 3 ONCE

Counts are 1–64 thinks. Following sends changed trace pictures at most once
per simulation second and stops when you lose permission. Traces expose your
mind's public decisions, never another mind's source or private memory.

See also: `help mindcraft`, `help proving`, `help simulator`, `help cockpit`.
