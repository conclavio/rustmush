# The cockpit

The seventeen verbs you fly a tank with, in the order a pilot uses them. They work
while you are at the station of a tank a running match has deployed, and nowhere
else — `help simulator` is how you get there.

  STARTUP             bring the tank's power up. Idempotent; motion and fire
                      refuse until it is online.
  HEADING <0-359>     steer to a bearing.
  SPEED <kph>         make way at that speed; a negative speed reverses.
  VIEW [BRIEF]        the arena window around your platform, as a map; BRIEF
                      says the same window in sentences.
  CONTACTS            what your platform holds.
  LOCK <contact>      hold one of your contacts by the designator CONTACTS
                      prints. Bare, it answers
                      `LOCK <contact>; CONTACTS lists them.`
  RANGE [<contact>]   how far away a contact plotted; your lock if you name
                      none. A named spot of the ground works too.
  BEARING [<contact>] which way a contact plotted; your lock if you name none.
                      A named spot of the ground works too.
  SCAN [<contact>]    what is left of a contact, section by section; your lock
                      if you name none. Needs a contact you have identified and
                      had a close look at.
  FIRE [<weapon>]     shoot your lock with your machine's main gun — the cannon
                      on a tank that carries one — unless you name another. A
                      weapon of two words takes its initials: FIRE mg.
  FIRE <weapon> WHEN <cond> [AND <cond>]…
                      arm the weapon: it fires your lock whenever every
                      condition holds, and tells you each time. FIRE
                      <weapon|ALL> OFF clears it. See "Arming a weapon".
  WEAPONSTATUS        every weapon: ready or cycling, its rounds, its own chance
                      against your lock, and that lock's range band — and,
                      under an armed weapon, what it is armed to do.
  EMCON [on|off]      switch your jamming pod, if you carry one: on jams every
                      sight inside its bubble, yours among them; off clears
                      your own sight and hides you from nothing. On at
                      check-out.
  STATUS              your callsign, power, speed, heading, position, the named
                      spot nearest you, what your own scope holds, your pod's
                      switch if you carry one, and your armour and internals
                      section by section. If you chose
                      a chassis with `SIM UNIT`, the second line brackets it
                      — `Red Tank 1 [biped_mk1]: …` —
                      because a pod's tank keeps the pod's own name whatever is
                      fitted to it.
  ORDER <squad|ALL> <objective>
                      command your side's machines: hold, advance on a named
                      spot, engage, withdraw, screen a bearing or support a
                      tank — or release them to their own minds. See
                      "Commanding the machines".
  WATCH [ON|OFF]      push a short block of your own line, your guns and your
                      contacts whenever it would read differently; for line
                      mode. See "Watching from line mode".
  DRIVE <spot|x,y|HOLD|OFF>
                      drive to a named spot or a cell on the machine's own
                      route-finding; HEADING or SPEED takes the helm back. See
                      "Driving".
  MIND [PROFILE <name>|RULES]
                      your tank's sealed mind: what it is, its profiles and the
                      one it will fly when you leave the seat, and who holds
                      each channel; it is dormant while you fly. MIND PROFILE
                      <name> switches to another sealed profile, once in ten
                      seconds; MIND RULES says what rules it carries.

`EXIT` climbs out. Once the match is running that is final: see
`help simulator`.

## Driving

**Heading is 0 to 359 degrees, clockwise from north**, so 90 is east and 270 is
west. **Speed is kph, and reverse is slower than ahead** — the stock tank makes
54 kph forward and 27 astern, and `SPEED` says both when you ask for more than
it has: `That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.`
Terrain caps it too: order 54 on grass and the tank will
make about 41, and `STATUS` shows what it is really doing rather than what you
asked for. A climb costs its slope on top of that, so the same order up a bank
of one in three makes about 31 and the tank is slowest where the ground is
steepest; a descent costs nothing extra.

`STARTUP` takes a few seconds and tells you when it is over: `Power online.`
Until then `HEADING` and `SPEED` are refused with `Power is still coming up;
hold on.`, so there is nothing to do but wait for the line.

Ground a tank cannot enter stops it, and the cockpit says so once per stop —
`Held: the ground ahead will not take the tank.` for impassable ground,
`Held: something is in the way.` for another unit, `Held: this is the edge of
the arena.` at the boundary, and `Held: the slope ahead is too steep for the
tank.` where the ground would take the tank but the bank onto it will not. That
last one is the one to drive **round**: tracks stop at 45 per cent, legs at 70,
and a slope too steep to climb is too steep to drop off, so a bank is never a
one-way door. Another unit in your way does not stop you at once: the tank holds
against it at the speed it had and goes on if the way clears; only when it has
not cleared in two seconds does the tank stop and the line arrive. `STATUS`
carries `HELD` until you order something that frees it.
Deep water is impassable to a tracked tank; the proving ground has one ford, on
the east-west road.

**`DRIVE` hands the helm to the tank's own drive-by-wire** — the same bounded
route-finding the machines use. `DRIVE The Ford` or `DRIVE a` steers for a named
spot, `DRIVE 12,34` for a cell; the tank finds a route, follows it, slows to
arrive and stops a cell short, and says `DRIVE: arrived.` A spot no route reaches
says `DRIVE: no route from here; holding.` and the tank holds where it is.
`DRIVE HOLD` stops where you are; `DRIVE OFF` lets go; and any `HEADING` or
`SPEED` you type takes the helm back at once.

## Names

Every tank in a match has a **callsign**: its side's letter and its pod, so red
pod 1 is `RA`, red pod 2 is `RB`, blue pod 3 is `BC`. `STATUS` opens with
yours, and `SIM STATUS` prints each pod's beside its number, so a callsign on
the radio is a person you can look up.

Your side shares a radio for the match, `RedNet` or `BlueNet`: you are joined
to it when the match starts and taken off it when you climb out, and your
callsign is your alias and your title on it, so `ra contact east` reaches every
red pilot as `[RedNet] RA Ada: contact east`, and the channel screen keeps it
with the net's other lines. The other
side cannot hear it or see who is on it, and the machines say nothing. While
you hold the alias, type `RANGE` and `BEARING` past their first two letters.

A contact your sensors have found but not yet told apart has no callsign to
give you, so your platform numbers it for itself: `X1`, `X2`, in the order your
own sensors found things. Those numbers are **yours** — your teammate's `X1` is
not your `X1` — and they are never reused. Three seconds of holding a return is
enough to tell it apart, and then it takes its real name:

  Contact X2 identified: BA, blue.

After that the row reads `BA`, and **both** spellings keep working: `LOCK X2`
and `LOCK BA` are the same contact. Case does not matter anywhere a designator
is typed: `lock ba` is `LOCK BA`. If you lose the contact and find it again it
is an unknown return once more, so it reads `X2` again and says `identified`
again when it resolves.

A contact you do not hold has no name you can type. `LOCK BA` before your
sensors have found `BA` answers `No such contact.` — the same line as a name
that means nothing at all, deliberately, because a refusal that told you `BA`
existed would be telling you where the other side is.

## Seeing

`CONTACTS` is your platform's own picture and nothing else — no object numbers,
no names, no pilots. One row a contact:

  BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%

the designator `LOCK` takes, what the contact has been classified as, its true
bearing and its bearing relative to your hull, a **range band**, its range
rounded to 50 m, and how old the plot is. A contact is drawn **where it was last
seen**, not where it is now, and the age is how stale that is. A teammate's
sighting reaches you only while their own sensors still hold it.

The last two columns are what the engine knows about the shot: which way the
contact was facing when it was last seen — front, side or rear — and, for a
hostile you are locked on and then the nearest other hostiles you have
identified (two rows in all), your main gun's chance against where it was
plotted. A dash is a contact the fire control is not resolving: one of your own
side (even when you are locked on it), one not yet identified, or a hostile
beyond those two rows. A tank with no gun shows the facing alone. Odds are
advice off the plot, as RANGE is: a contact that moved since it was last seen
is not where the number was computed.

The band word is **how far across your own sight the contact is**: `close` is
the nearest third of your sensor's reach, `effective` the middle third, `far` the
outer third and anything a teammate is passing you from beyond it. On a stock
tank the sight reaches 600 m, so `close` runs to 200 m and `effective` to 400 m.

It is **not** a judgement of your gun. Every gun this game ships out-reaches the
sight that finds a target for it — the stock cannon makes 3 km and the stock
sight sees 600 m — so a contact at 450 m reads `far` and is a shot the cannon
takes comfortably. What the gun makes of the distance is the `% to hit` on your
lock, and the band your lock falls in on `WEAPONSTATUS`. Past a gun's last band
the chance is 0%: the round is still fired and spent, and it cannot land.

`RANGE` and `BEARING` ask about one contact without re-reading the whole scope.
Name one, or name none and get your lock:

  RANGE BA         Range to BA: 440 m.
  BEARING          Bearing to BA: 087° (rel 357°).
  RANGE The Ford   Range to The Ford (A): 430 m.

Your **lock** reads finer than 50 m: it is a fire-control solution and the
platform is tracking that one return continuously. How much finer is what the
plot in front of you can carry — a fresh one reads to 10 m, a one-second plot to
20, and a plot a couple of seconds old reads no better than the rest of the
scope, because a tank making 60 kph has moved seventeen metres in each of those
seconds. Both verbs read the same plot every other surface reads, and both say so
when it is going stale: `Range to BA: 450 m, plotted 4 s ago.` With no lock and
no name they answer `No lock: LOCK a contact first, or name one.`

A contact your sensors had told apart can go back to being a number — it drops
out of your picture for a moment and comes back unresolved — and your panel says
so once: `BA is unidentified again: X2.` The old name stops working the instant
that line is printed, so `LOCK BA` answers `No such contact.` until the contact
has been told apart a second time; `LOCK X2` is what works meanwhile.

If something is jamming your sight, `STATUS` says so on a line of its own —
`receiver degraded: 15 of 40 through cover, 60% reach` — and your panel says it
once on the way in and once on the way out. It is your own machine's reading:
nothing tells you who is jamming you or from where.

If your own tank carries the jamming pod (`SIM UNIT line_tank_ecm`), the pod is
on at check-out and radiates from the moment power is up, and it jams **every**
sight inside its bubble: the other side's, your own side's radio links, and
your own sight too, because you are standing in the middle of it. `EMCON off`
silences it — `EMCON off: the pod is silent; your own sight clears, and nothing
hides you.` — so your receiver clears and your side's mesh closes again, and the
other side's sights near you are no longer blinded. `EMCON on` switches it back
and says how far it reaches: `EMCON on: the pod radiates; every sight inside 60
m is jammed, your own among them.` Bare `EMCON` says which it is, and `STATUS`
carries `emcon on: the pod radiates` or `emcon off: the pod is silent` on a line
of its own. Nobody else is told you switched: a sight inside the bubble finds
out only from its own receiver, at its own next look. A tank with no pod
answers `This tank carries no jamming pod.`

`SCAN` is what is left of one contact, section by section:

  SCAN BA          Scan of BA, read 2 s ago:
                     engine green, hull yellow, left_track green,
                     right_track destroyed, turret red

Four words and never a number: **green** is most of a section still there,
**yellow** is about a third to two thirds of it gone, **red** is worse, and
**destroyed** is a section that has stopped being one. It is a reading and not a
tether — `read 2 s ago` is the age of the **look**, which is older than the plot
whenever you have held the contact since without getting close.

A reading is earned twice over. The contact has to be **identified**, because a
section name says what kind of machine it is:

  SCAN X2          X2 is not identified yet; there is nothing to read.

— and your platform has to have had a **close look** at it, which is well inside
the range it can merely detect things at:

  SCAN BA          No reading on BA yet: close the range for a look.

Drive closer and look again. A teammate's reading reaches you over the mesh the
same way their plot does, and it is as old as *their* look; and a section you
watched one of your own rounds take apart reads `destroyed` from that moment,
without waiting for another look.

**A tank on the move is seen from further away than a tank standing still.** A
sight reaches its full distance only against a target that gives it something to
see, and what you give it is the ground you stand on and whether you are moving.
Against the stock sight's 600 m, with a clear line between you: on grass a
stopped tank can be seen out to 80 per cent of that (480 m) and a moving one out
to all of it; bare ground stops at 90 per cent; a road hides nothing either way;
rubble stops at 70 per cent and moves at 95; light woods stop at 65 and move at
90; heavy woods stop at 50 per cent (300 m) and move at 80. Those are ceilings: a
canopy between you and the watcher hides you closer in than any of them. So cover
helps the tank that has stopped in it most, and rolling across open ground hands
the other side a contact at a range that would have held nothing had you stopped.
A thermal sight reads your engine and not your outline, so the ground matters
little to it and your power matters a lot: against its 450 m a tank under way can
be seen out to 92 to 100 per cent whatever it stands in, a stopped tank with
power up out to about two thirds of that (heavy woods: about 257 m), and one
whose power is down out to about an eighth. Stop before you are found; against
the thermal sight, shut down.

**The facet you show decides what a hit costs you.** Armour is per facet and a
round lands on the face it arrives at: from within 45 degrees of your bow it
strikes the front, from within 45 degrees of your stern the rear, and from
anywhere else a side — and a side is twice the target, 7 m of flank against 3.5
m of bow. On the stock tank the hull carries 400 in front, 250 a side and 150
behind, the turret 350, 220 and 140, and a side shot lands on that side's track
— 120, and a slowed tank once it is gone — three times in ten; a round from
behind can reach the engine through the hull's rear plate. So a tank fought
nose-on spends its thickest plate, and one caught from behind spends its
thinnest. Turn into the shot.

`VIEW` draws the same picture as a map: terrain glyphs, `@` for you, `?` for a
contact nothing has classified, `+` for your side and `x` for the other. `A`,
`B`, `C` and so on are the ground's named spots, listed by name in the legend
after the contacts; a contact standing on one hides its letter, because the
contact is what you hold. It is a close-in window, 39 cells across and 20 up,
each cell printed two columns wide so the ground is square; a line under it
says how many contacts plot beyond it, and `CONTACTS` lists them. A last line
names every spot the window could not draw, with its range and bearing:
`Off the window: C North Barn, 1390 m at 045°; E East Compound, 2080 m at 075°.`

`VIEW BRIEF` says the same window in sentences, for a screen reader or a pilot
who would rather read: where the window is, then the ground on eight bearings
from your own cell, run by run —
`North: 50 m of grass, then road to 60 m, then grass to 150 m, then beyond the window.`
— then every contact as `CONTACTS` words it (or `No contacts in the window.`),
the beyond-window line, the named spots the window shows and the ones off it,
with their range and bearing, and `End of brief.` to close it, so a script
knows where it ends.
It describes the window `VIEW` draws and nothing past it: `beyond the window` is
where the map stops, `the edge of the arena` where the ground does. A diagonal
is measured along the diagonal, so five cells read 70 m. `VIEW` takes no other
word.

**Named ground.** The proving ground is surveyed, so its named places are yours
from the moment you board: `A` The Ford, `B` The Crossroads, `C` North Barn,
`E` East Compound and `F` The Bluff. `RANGE` and `BEARING` take a spot's name
or its letter — `RANGE The Ford`, `BEARING c` — whether or not it is in sight,
and `STATUS` names the one nearest you. A name is the whole name, in any case; a
word that is neither a contact you hold nor a spot's name is `No such
contact.`, as ever.
Before the panel has told you anything a spot answers `The panel has told you
nothing yet.`, because what is missing is where you are.

Contacts arrive and leave on their own: `New contact X2: bearing 087°,
effective.`, `Contact X2 identified: BA, blue.`, `Contact BA lost.`

## Shooting

`LOCK <contact>` holds a contact. When the lock resolves you are told what it is
worth: `Locked on BA, 62% to hit.` `WEAPONSTATUS` gives the same number per
weapon, because a weapon's chance is its own.

Locking one of **your own** side puts `(FRIENDLY)` after the name, on the lock,
on `WEAPONSTATUS` and on every hit: `Locked on RB (FRIENDLY), 62% to hit.`
Nothing is refused. The word is the whole of the warning, and what you do next
is yours.

`FIRE` shoots the lock with your machine's main gun — the cannon on a tank that
carries one, the autocannon on the scout car; `WEAPONSTATUS` lists what you
carry — and `FIRE mg` names another. A weapon whose id is two words can be
typed as its **initials** — `machine_gun` is `mg` — and `WEAPONSTATUS` prints
the short name in brackets beside the id where there is one. A prefix works
too: `FIRE can`. The full id always works, and a word that could mean two of
your weapons is refused saying which: ``FIRE m`` answers
``` `m` could be machine_gun or missiles; say more. ``` A round is traced from
where you fired it, so once it is away you may move; a miss on a contact you no
longer hold reads `Miss.` and no more, in the cockpit and in your debrief.

You are told what your round did and nothing more: `Hit on BA: turret.`, `Miss
on BA: the target moved.`, `BA's turret is destroyed.`, `BA destroyed.` The
**section** is all of it: what the round did to that section, and what the
target has left inside it, is the target's business and not yours. A round that
lands on a section you have already destroyed says so — `Hit on BA: turret,
already destroyed.` — and says it at most once every five seconds, because it
is the same sentence every time.

The section word is only there for a contact you have **identified**, for the
reason `SCAN` needs one: a section name says what kind of machine you are
shooting at. A return you have not told apart reads `Hit on X2.`, and a target
your platform is not holding at all reads `Hit.`

What you are told about your **own** tank is the other way round and complete:

  Hit on the hull's front facet: 280 of 400 armour left.
  Hit on the hull's front facet: armour gone, 140 of 200 inside.
  The left_track is destroyed: the tank is slowed.
  Hit on the left_track, which is already destroyed.

— and nothing whatever about who shot you. Where a round came from is something
only a sensor may tell you. That last line, like the gunner's, is said at most
once every five seconds a section: a dead track takes rounds all day and there
is nothing new to say about it.

Lose one track and the tank is slowed and will not turn that way; lose both and
it neither drives nor turns, and the second line ends `the tank cannot move`.

## Arming a weapon

You decide when a weapon fires; the tank's fire control does the waiting. Arm a
weapon with conditions and it fires your lock, by itself, every time all of them
hold — and a typed `FIRE` still fires whenever you type it, armed or not. An
armed weapon fires only at your lock, and only at a lock your sensors have
identified: an unknown return is never shot by a slot.

  FIRE cannon WHEN odds >= 60 AND aspect = side,rear
                   Arming cannon: fires your lock when your chance to hit is
                   60% or better and it shows its side or rear.

A rule is up to four conditions joined by AND — there is no OR — and
these eight are the conditions there are, one example each:

  odds >= 60            your chance to hit with this weapon is 60% or better,
                        in whole per cent from 1 to 100; `odds cannon >= 60`
                        says the same, naming the weapon being armed
  aspect = side,rear    the face it shows you: front, side or rear, a comma
                        between two or three
  range < 600 m         it is nearer than 600 m; `range > 200 m` farther than
                        200 m, and the two together are a window. The range
                        is the one `RANGE` prints for your lock, so a printed
                        600 m is outside `range < 600 m`. `WEAPONSTATUS`
                        shows the last metre that fires (`under 599m`), or
                        the first (`past 201m`). A band and a range term
                        that disagree are refused: type one or the other
  band = effective      close, effective or far across your sight, compiled to
                        fixed metres from the frame you typed it on: the reply
                        says which metres, and they do not move afterwards
  rounds > 10           keep ten rounds back; `rounds > 25%` keeps a quarter
                        of a full load
  reserve               on a tank you fly, the same as `rounds > 0`
  target = identified   what every slot asks already, said out loud
  lock                  your lock, which is what every slot fires at

**The Compact floor.** Every condition carries at least the match's Compact
floor, 10% unless the match says otherwise, so `FIRE cannon WHEN odds >= 5` is
refused as you type it:
`FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less.`
A rule with no odds term at all asks for the floor: `FIRE cannon WHEN aspect =
rear` fires on a rear shot at 10% or better, and its reply says so.

**Your core's slots.** A tank has as many slots as its core carries: a line
tank four, the heavy six, a scout two. They are `S1` upward, given in the
order you arm: the first weapon you arm takes the first free one, and arming a
weapon again replaces its rule and keeps its slot. The reply names no slot,
because the slot is given when the order lands: the number shows under the
weapon's row on `WEAPONSTATUS` —

  cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit
    S1: >=60% side/rear

— on the cockpit screen's weapons pane, and in every line the slot says. Every
gun on a shipped tank has a slot of its own. Only a hull with fewer slots than
weapons runs out: there, one more weapon while every slot stands is taken as
you type it, and the tank refuses it a moment later, as a notice under the
reply:
`FIRE cannon refused. Every slot is armed; FIRE <weapon> OFF frees one.`
Free a slot first and the new rule takes it, even typed straight after
`FIRE cannon OFF`.
`FIRE cannon OFF` answers `Cleared: cannon fires only when you say.` and `FIRE
ALL OFF` answers `Cleared: your guns fire only when you say.` — both said
whether or not anything was armed.

**What a slot tells you.** Every shot a slot fires is announced once, naming
the slot, the chance and the face it read:

  Cannon fires at BA by S1 (62%, side).

and a shot a slot held is said at most once every ten seconds for a contact,
with the condition that held it:

  S1 held the cannon: 54% under your 60%.
  S1 held the cannon: BA shows its front.
  S1 held the cannon: BA at 650 m, outside 201 to 599.
  S1 held the cannon: 7 rounds, keeps 8.

The range it quotes is the one `RANGE` prints, and its window is the metres
the slot keeps, the ones `WEAPONSTATUS` shows: `range > 200 AND range < 600`
is held as outside 201 to 599. A slot with nothing to shoot at, or whose gun
is still cycling, says nothing. Nobody else is told any of
it: not your side, not the target.

**Which wins.** In four lines:

  An ORDER's HOLD FIRE beats everything.
  FIRE <weapon> OFF clears a slot; a cleared weapon fires only when you say.
  One slot a weapon: a second FIRE <weapon> WHEN replaces the first.
  Rules beat sliders.

Anything else is refused in one sentence:
`FIRE cannon refused. That condition is not one this core takes; help cockpit lists the eight it does.`
— another word, an OR, a fifth condition or a number out of its range. The
rest of the rule language comes in a later cut.

## Commanding the machines

When the simulator fills the empty pods with machines (`SIM AI`), they fight on
their own minds: Standard Doctrine, at the preset the side's lead (or staff)
chose with `SIM AI MIND`, or on Line (`help minds`). `ORDER` tells them what to
do instead.

**Who may order what.** Your side's lead — the pilot who may `SIM START` and
`SIM CONCEDE` — orders any squad of your side. Anyone else orders one tank: the
one in their pod pair (pod 1 with pod 2, 3 with 4, 5 with 6), when a machine is
flying it. Your side's machines are grouped into squads, red-1 first: up to four
machines are one squad, and more are dealt out evenly, so five make a three and a
two.
Name a squad in full, `ORDER red-1 HOLD`; by its number alone, `ORDER 1 HOLD`,
which is your own side's; or every squad you may order at once,
`ORDER ALL HOLD`.

There are six orders and a release, and the answer says what each one means:

  ORDER red-1 ENGAGE BA          Ordered red-1 to engage BA: it closes and fires
                                 on BA where it can see it, else on what it
                                 finds.
  ORDER ALL HOLD                 Ordered red-1 to hold: it stops where it stands
                                 and fires nothing.
  ORDER 1 ADVANCE The Ford       Ordered red-1 to advance on The Ford (A): it
                                 drives there and fights nothing on the way.
  ORDER red-1 WITHDRAW HOLD FIRE Ordered red-1 to withdraw, holding fire: it
                                 breaks for its entry zone and fires nothing.
  ORDER red-1 SCREEN 90          Ordered red-1 to screen 090°: it holds where it
                                 stands, faces 090° and fires on what comes
                                 inside its arc without giving chase.
  ORDER 1 SUPPORT RA             Ordered red-1 to support RA: it keeps within
                                 150 m of RA and fights what it finds there.
  ORDER red-1 RELEASE            Released red-1 to its mind: it fights on its
                                 own until it is ordered again.

Engage without a contact — `ORDER red-1 ENGAGE` — and the squad closes and fires
on what it finds. A withdrawal without hold fire still fights anything that comes
inside its bands on the way out. Hold fire, as the last two words of any order,
forbids shooting and nothing else: `ORDER red-1 ENGAGE BA HOLD FIRE` closes and
locks but fires nothing. If you are not the lead, name the squad your pod
pair's tank flies in, or `ORDER ALL`, and the answer names that one tank by its
callsign: `ORDER ALL HOLD` is answered `Ordered RB to hold: …`.

**A screen holds an arc and never chases.** The bearing is 0 to 359, as
`HEADING` takes one, and the arc is 120 degrees wide, 60 either side of it: the
squad stays where it stands, faces the bearing, and turns to fire on what comes
inside the arc and inside its range. What is behind it, or out to a flank, it
leaves alone, however close.

**A support keeps station on one tank and fights what it finds there.** Name
any tank of your own side by its callsign — your own included. The squad keeps
within 150 m of it, closing to about 50 m when it has nothing to fight, and
fights what comes within its reach there; past 150 m it gives up the chase and
drives back. It does not know what the tank it supports is shooting at: it
picks its own targets, as every machine does. A supporter that cannot see the
tank it supports holds where it stands and fights from there. The tank named,
if it is one of the squad's own, fights on as it would have. If the tank is
taken out of the match, the order ends for them there and then and the machines
go back to their own minds — a pilot climbing out does not take it out: it
stays, flown by its mind, and the order stands. A supporter that sees it
destroyed fights on its own.

**A release hands the squad back at once.** `ORDER red-1 RELEASE` ends your
order on it now rather than in ten minutes, the machines go back to their own
minds, and the squad no longer counts against the four you may hold. It is an
order in the count of twelve a minute, and nothing may follow it — not even hold
fire.

An order **stands for ten minutes or until your next order to it**, and
every answer ends by saying so. When it runs out the machines go back to their
own minds and you alone are told
`Squad red-1: your hold order has run out; back on their minds.` Your side
hears the same of its own lance leaders' orders
(`Squad red-1: red lance leader 1's hold order has run out; back on their minds.`).
Nobody else is told you gave an order: nothing is said on your side's radio
unless you say it.

**You may hold orders on four squads at once**: a fifth is refused until one of
the four runs out or you release one, and a new order to a squad you already
hold is not another.
**You may give twelve orders a minute**, `ORDER ALL` counting as one, and the
thirteenth is refused until the minute has passed.

**You may point, but a tank that cannot see still cannot see.** The contact you
name is one of yours, by the designator your `CONTACTS` prints, and a machine
that holds nothing of its own on that tank takes the best target it does hold.
Your own side's tanks are never a target: name one and the order is a plain
engage, answered as one.
An advance names one of the ground's spots, by its name or its letter, as
`RANGE` takes one.

Six refusals are the order's own. `No such squad.` is the answer for any squad
you cannot order — the other side's, one that does not exist, or one of your own
side's that is not yours to command — and it is one line for all three, so it
tells you nothing about the other side. `You have no squad to order.` is `ORDER
ALL` from a pilot with nothing under them. `No tank of your side flies that callsign.`
is a support naming anything but a tank of your own side — the other side's
callsign, an `X` number, a pod nobody flies or a word that is no callsign all
read the same. Three are the limits above: with your four squads held, any order
to another is `You hold orders on 4 squad(s) already; wait for one to run out, or RELEASE one.`;
an `ORDER ALL` that would take you past four while
you still have room for one is `ORDER ALL would put 5 squads under you; your span
is 4. Order them one at a time.`; and the thirteenth order in a minute is `You
have given 12 orders in the last minute; wait.` A target or a spot your panel
does not know is `No such contact.`, as it is everywhere else.

## Refusals

Every refusal is one line. Most of them the cockpit answers itself, the moment
you type the order, in the tank's own words: `Power not online: STARTUP first.`,
`Power is still coming up; hold on.`, `No lock: LOCK a contact first.`, `Still
cycling: 4 s to go.`, `Empty: that weapon has nothing left to fire.`

Every `FIRE` refusal names the weapon, whichever half answered: `FIRE cannon
refused. Still cycling: 4 s to go.` So a pilot with two guns is never left
guessing which breech is busy.

Arming a weapon has three refusals of its own, each naming the weapon:
`FIRE cannon refused. That condition is not one this core takes; help cockpit lists the eight it does.`
for a rule outside the eight, `FIRE cannon refused. Every slot is armed; FIRE
<weapon> OFF frees one.` with every slot your core carries taken, and `FIRE
cannon refused. The Compact floor is 10%; a condition may not ask for less.`
for a chance under the floor. The full table is always the arena's, a moment
after the reply; the floor can come from it too, in the same words.

**Any** order can also be refused by the arena itself a moment after the cockpit
passed it on — you and the arena are a pulse apart, and the tank may have been
shot in between. Those lines open by naming the order, so you know which one
they answer: `HEADING refused. Your tank cannot move.`, `LOCK refused. Power not
online: STARTUP first.`, `STARTUP refused. Your tank is a wreck.`, `FIRE cannon
refused. Too many orders at once; try that again.`

## When your tank is out of it

A tank can lose its engine and its guns and still be sat in, and it can be
finished outright. Either way you are told once, and the panel says so
afterwards:

  Your tank can neither move nor shoot. The match runs on: wait it out, EXIT to
  the team room, or SIM ABORT to call it off.

  Your tank is destroyed. The match runs on without you: wait it out, EXIT to
  the team room, or SIM ABORT to call it off.

If your side is beaten but not dead, its lead can end it honestly with
`SIM CONCEDE`: the other side wins and the record says so. See `help simulator`.

After that `STATUS` carries `DESTROYED` beside the speed, and every order is
refused at the cockpit: `Your tank is destroyed and takes no orders. EXIT leaves
the pod.` A tank that has only lost its motion still sees, still locks and still
shoots — it is `HEADING` and `SPEED` that answer `Your tank cannot move.` A tank
that has lost its guns still drives, and if it was the last gun on your side the
match is over at that hit: a side that came in armed and has no gun left has
lost.

## Your tank's mind

Your tank carries a mind that flies it when you are not in the seat: the one
your slot chose with `SIM MIND` before the match, or Standard Doctrine on Line
if it chose none (`help minds`). It takes the tank when you `EXIT`, for the
rest of the match, or when your line has been down past the grace, until you
`BOARD` again. While you fly it is dormant. `MIND` reads it:

  Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II core.
  Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds.
  Channels: you hold helm, sensors and fire; the mind is dormant while you fly.
  Rules: none sealed; your own armings are under WEAPONSTATUS.

`MIND PROFILE Line` sets the profile it will fly to another of those sealed
on it, once in ten seconds; `MIND RULES` says it carries no rules, and
`MIND TRACE` that the core keeps no trace of its thinks yet. If the mind drops
to the ROM — three faults running, or its core destroyed — you are told, and
the first line says the hull is on the ROM for the match. After the match,
`SIM REPORT` carries a `Your mind` block saying when it flew your tank and
what it shot (`help minds`).

## Watching from line mode

The line client has no screen that redraws itself: it prints the arena's
notices as they happen and answers what you type, and your position, speed,
readiness and contact ranges reach you only when you ask (`STATUS`,
`CONTACTS`, `WEAPONSTATUS`, `RANGE`). `WATCH ON` changes that: whenever the
block below would read differently from the last one you were sent, the panel
pushes it, one to three lines, each within 120 columns —

  Watch RA: online, 24 kph, heading 090°
  Watch guns: cannon ready, machine_gun (mg) cycling (3 s); lock BA, 450 m, 46% to hit
  Watch contacts: BA blue 087° far 450 m; X3 unknown 315° effective 200 m, plotted 4 s ago

— your own line, your guns with your lock, and your contacts, in that order;
the guns line only while you carry a gun and the contacts line only while
your platform holds a contact, a plot's age named only once it has gone
stale. It is the picture `STATUS`, `WEAPONSTATUS` and `CONTACTS` print and
nothing more: no tempo, no position, and nothing about a unit your platform
has not found. A line that would run past 120 columns ends in `; +2 more`,
and `CONTACTS` lists the rest. `WATCH OFF` stops it, bare `WATCH` says which
it is, and the switch is the seat's: it is off again when you next board. On
the TUI and in the browser the cockpit screen already draws all of this, and
the block lands in your scrollback only.

## The cockpit screen

Both full-screen clients put the whole of this on **screen 0**, updating by
itself as the arena sends it — press `0` in `@tui` (`help tui`), or click the
`0 cockpit` tab in the browser. In the browser, as in `@tui`, the tab appears the
first time a match sends you a frame and goes when you leave the tank. Either way
the screen opens itself on that first frame, and after that it is yours to leave
and come back to until you climb out; if you are on it then, you are put back on
the scene screen.

It shows what the panel tells you and nothing else: the arena's ground drawn
around your own tank, with your contacts on it — `x` hostile, `+` friendly, `?`
not told apart yet — then your callsign, power, speed, heading, heat, what your
own scope holds and your lock; what is left of your tank section by section; your
weapons with their cycle, rounds and chance; and your contacts with their
bearing, band, range, plot age and the sections of any you have had a close look
at. Under your weapons are four rows of your tank's mind, the short form of
`MIND`: the mind and its class, its sealed profiles with the one it will fly
marked, that it is dormant while you fly, and the channels you hold. In the
browser the map has a zoom and draws a ring around your lock, and clicking a
contact's designator — or its `scan` button — puts `lock BA` or `scan BA` in
the box ready to send, and clicking a profile the mind is not flying puts
`MIND PROFILE <name>` there; the browser's box sits under every screen but
the channel screen, which has its own, so a prefill leaves you on the cockpit with
the line ready to send, as a command even if the box was set to Say or Pose. On
both clients the notices list also carries the answer to anything you type while
you are in the tank, and each match starts it empty. Nothing on the screen is
sent until you press Enter: the reading verbs are the only ones a bare key runs,
and `STARTUP`, `HEADING`, `SPEED`, `LOCK` and `FIRE` are always typed.
`p` pulls the panel again, and the map too if it has gone dark. Your side's
radio is not on this screen: its lines are on the channel screen with every
other channel's, and the notices list does not repeat them.

See also: `help simulator`, `help tui`, `help chargen` (gunnery is the rating
the shot reads), `help macros` (a word of your own for a line you type
often).
