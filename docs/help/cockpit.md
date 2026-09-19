# The cockpit

The nine verbs you fly a tank with, in the order a pilot uses them. They work
while you are at the station of a tank a running match has deployed, and nowhere
else — `help simulator` is how you get there.

  STARTUP             bring the tank's power up. Idempotent; motion and fire
                      refuse until it is online.
  HEADING <0-359>     steer to a bearing.
  SPEED <kph>         make way at that speed; a negative speed reverses.
  VIEW                the arena window around your platform, as a map.
  CONTACTS            what your platform holds.
  LOCK <contact>      hold one of your contacts by the id CONTACTS prints.
  FIRE [<weapon>]     shoot your lock; the cannon unless you name another.
  WEAPONSTATUS        every weapon: ready or cycling, its rounds, its own chance
                      against your lock, and that lock's range band.
  STATUS              power, speed, heading, position, the arena's tempo, and
                      your armour and internals section by section.

`EXIT` climbs out. Once the match is running that is final: see
`help simulator`.

## Driving

**Heading is 0 to 359 degrees, clockwise from north**, so 90 is east and 270 is
west. **Speed is kph.** Terrain caps it: order 54 on grass and the tank will
make about 41, and `STATUS` shows what it is really doing rather than what you
asked for. A climb costs its slope on top of that, so the same order up a bank
of one in three makes about 31 and the tank is slowest where the ground is
steepest; a descent costs nothing extra.

Ground a tank cannot enter stops it, and the cockpit says so once per stop —
`Held: the ground ahead will not take the tank.` for impassable ground,
`Held: something is in the way.` for another unit, `Held: this is the edge of
the arena.` at the boundary, and `Held: the slope ahead is too steep for the
tank.` where the ground would take the tank but the bank onto it will not. That
last one is the one to drive **round**: tracks stop at 45 per cent, legs at 70,
and a slope too steep to climb is too steep to drop off, so a bank is never a
one-way door. `STATUS` carries `HELD` until you order something that frees it.
Deep water is impassable to a tracked tank; the proving ground has one ford, on
the east-west road.

## Seeing

`CONTACTS` is your platform's own picture and nothing else — no object numbers,
no names, no pilots. One row a contact:

  c4  tank  bearing 087° (rel 032°)  effective  450 m  2 s

the opaque id `LOCK` takes, what the contact has been classified as, its true
bearing and its bearing relative to your hull, the range band of the weapon a
bare `FIRE` fires, its range rounded to 50 m, and how old the plot is. A contact
is drawn **where it was last seen**, not where it is now, and the age is how
stale that is. A teammate's sighting reaches you only while their own sensors
still hold it.

`VIEW` draws the same picture as a map: terrain glyphs, `@` for you, `?` for a
contact nothing has classified, `+` for your side and `x` for the other.

Contacts arrive and leave on their own: `New contact c4: bearing 087°,
effective.`, `Contact c4 identified: tank.`, `Contact c4 lost.`

## Shooting

`LOCK <id>` holds a contact. When the lock resolves you are told what it is
worth: `Locked on c4, 62% to hit.` `WEAPONSTATUS` gives the same number per
weapon, because a weapon's chance is its own.

`FIRE` shoots the lock with the cannon; `FIRE mg` names another weapon. You are
told what your round did and nothing more: `Hit on c4.`, `Miss on c4: the target
moved.`, `c4 destroyed.` Which section you struck and what the target had left
is the target's business, not yours.

What you are told about your **own** tank is the other way round and complete:

  Hit on the hull's front facet: 280 of 400 armour left.
  Hit on the hull's front facet: armour gone, 140 of 200 inside.
  The left_track is destroyed: the tank is slowed.

— and nothing whatever about who shot you. Where a round came from is something
only a sensor may tell you.

## Refusals

Every refusal is one line. Most of them the cockpit answers itself, the moment
you type the order, in the tank's own words: `Power not online: STARTUP first.`,
`Power is still coming up; hold on.`, `No lock: LOCK a contact first.`, `Still
cycling: 4 s to go.`, `Empty: that weapon has nothing left to fire.`

`STARTUP`, `LOCK` and `FIRE` can also be refused by the arena itself a moment
after the cockpit passed them on, and those lines open by naming the order, so
you know which one they answer: `FIRE refused. Still cycling: 4 s to go.`,
`FIRE refused. Your tank is a wreck.`, `LOCK refused. Power not online: STARTUP
first.` No other order is answered that way — `HEADING` and `SPEED` are refused
at the cockpit and nowhere else.

See also: `help simulator`, `help chargen` (gunnery is the rating the shot
reads).
