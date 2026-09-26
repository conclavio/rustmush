# The cockpit

The thirteen verbs you fly a tank with, in the order a pilot uses them. They work
while you are at the station of a tank a running match has deployed, and nowhere
else — `help simulator` is how you get there.

  STARTUP             bring the tank's power up. Idempotent; motion and fire
                      refuse until it is online.
  HEADING <0-359>     steer to a bearing.
  SPEED <kph>         make way at that speed; a negative speed reverses.
  VIEW                the arena window around your platform, as a map.
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
  FIRE [<weapon>]     shoot your lock; the cannon unless you name another.
                      A weapon of two words takes its initials: FIRE mg.
  WEAPONSTATUS        every weapon: ready or cycling, its rounds, its own chance
                      against your lock, and that lock's range band.
  STATUS              your callsign, power, speed, heading, position, the named
                      spot nearest you, what your own scope holds, and your
                      armour and internals section by section. If you chose
                      a chassis with `SIM UNIT`, the second line brackets it
                      — `Red Tank 1 [biped_mk1]: …` —
                      because a pod's tank keeps the pod's own name whatever is
                      fitted to it.
  ORDER <squad|ALL> <objective>
                      command your side's machines: hold, advance on a named
                      spot, engage, withdraw, screen a bearing or support a
                      tank — or release them to their own doctrine. See
                      "Commanding the machines".

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

## Names

Every tank in a match has a **callsign**: its side's letter and its pod, so red
pod 1 is `RA`, red pod 2 is `RB`, blue pod 3 is `BC`. `STATUS` opens with
yours, and `SIM STATUS` prints each pod's beside its number, so a callsign on
the radio is a person you can look up.

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

  BA  blue  bearing 087° (rel 032°)  far  450 m  2 s

the designator `LOCK` takes, what the contact has been classified as, its true
bearing and its bearing relative to your hull, a **range band**, its range
rounded to 50 m, and how old the plot is. A contact is drawn **where it was last
seen**, not where it is now, and the age is how stale that is. A teammate's
sighting reaches you only while their own sensors still hold it.

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

**A tank on the move is seen from further away than a tank standing still.**
Your signature is what the other side's sensors are working against, so rolling
across open ground hands them a contact at a range that would have held nothing
if you had stopped. Cover helps the tank that has stopped in it most: heavy
woods halve what a tank standing still gives away and take only a fifth off a
tank still rolling. Stop before you are found.

**The facet you show decides what a hit costs you.** Armour is per facet, and
the round lands on whichever face you are presenting — so a tank fought
nose-on spends its thickest plate and a tank caught broadside or from behind
spends its thinnest. Turn into the shot.

`VIEW` draws the same picture as a map: terrain glyphs, `@` for you, `?` for a
contact nothing has classified, `+` for your side and `x` for the other. `A`,
`B`, `C` and so on are the ground's named spots, listed by name in the legend
after the contacts; a contact standing on one hides its letter, because the
contact is what you hold. It is a close-in window, 39 cells across and 20 up,
each cell printed two columns wide so the ground is square; a line under it
says how many contacts plot beyond it, and `CONTACTS` lists them. A last line
names every spot the window could not draw, with its range and bearing:
`Off the window: C North Barn, 1390 m at 045°; E East Compound, 2080 m at 075°.`

**Named ground.** The proving ground is surveyed, so its named places are yours
from the moment you board: `A` The Ford, `B` The Crossroads, `C` North Barn and
`E` East Compound. `RANGE` and `BEARING` take a spot's name or its letter —
`RANGE The Ford`, `BEARING c` — whether or not it is in sight, and `STATUS`
names the one nearest you. A name is the whole name, in any case; a word that is
neither a contact you hold nor a spot's name is `No such contact.`, as ever.
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

`FIRE` shoots the lock with the cannon; `FIRE mg` names another. A weapon whose
id is two words can be typed as its **initials** — `machine_gun` is `mg` — and
`WEAPONSTATUS` prints the short name in brackets beside the id where there is
one. A prefix works too: `FIRE can`. The full id always works, and a word that
could mean two of your weapons is refused saying which: ``FIRE m`` answers
``` `m` could be machine_gun or missile; say more. ```

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

## Commanding the machines

When the simulator fills the empty pods with machines (`SIM AI`), they fight on
their own doctrine. `ORDER` tells them what to do instead.

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
                                 150 m of RA and fires first on what RA is
                                 fighting.
  ORDER red-1 RELEASE            Released red-1 to doctrine: it fights on its
                                 own reflexes until it is ordered again.

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

**A support keeps station on one tank and shoots what that tank shoots.** Name
any tank of your own side by its callsign — your own included. The squad keeps
within 150 m of it, closing to about 50 m when it has nothing to fight, and
fires first on whatever that tank is fighting; past 150 m it gives up the chase
and drives back. A supporter that cannot see the tank it supports holds where it
stands and fights from there. The tank named, if it is one of the squad's own,
fights on as it would have. If the tank leaves the match or its supporters see
it destroyed, the order ends for them there and then, and the machines go back
to their own doctrine.

**A release hands the squad back at once.** `ORDER red-1 RELEASE` ends your
order on it now rather than in ten minutes, the machines go back to their own
doctrine, and the squad no longer counts against the four you may hold. It is an
order in the count of twelve a minute, and nothing may follow it — not even hold
fire.

An order **stands for ten minutes or until your next order to it**, and
every answer ends by saying so. When it runs out the machines go back to their
own doctrine and your side hears `Squad red-1: mind link dropped, unit on
doctrine (its hold order has run out).` Nobody else is told you gave an order:
there is no radio line.

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
shoots — it is `HEADING` and `SPEED` that answer `Your tank cannot move.`

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
at. In the browser the map has a zoom and draws a ring around your lock, and
clicking a contact's designator — or its `scan` button — puts `lock BA` or
`scan BA` in the box ready to send; the browser's box sits under every screen but
the channel screen, which has its own, so a prefill leaves you on the cockpit with
the line ready to send, as a command even if the box was set to Say or Pose. On
both clients the notices list also carries the answer to anything you type while
you are in the tank, and each match starts it empty. Nothing on the screen is
sent until you press Enter: the reading verbs are the only ones a bare key runs,
and `STARTUP`, `HEADING`, `SPEED`, `LOCK` and `FIRE` are always typed.
`p` pulls the panel again, and the map too if it has gone dark.

See also: `help simulator`, `help tui`, `help chargen` (gunnery is the rating
the shot reads).
