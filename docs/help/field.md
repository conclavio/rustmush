# The combat field

A mining site is quiet ground: rigs drive, scan and dig on the campaign's clock,
and another captain's flight is a row on your map. When someone opens fire, or a
party's leader declares another party hostile, the whole site becomes a
**field**: a fight at human speed, with everyone on the ground in it. This page
covers how a field opens, who fights whom, what you see, how you get out and
what the fight leaves behind.

(On the mining pages a *field* is also a 32-cell patch of ground with a name like
`K-NW`. Here it means the whole site while a fight is on.)

## How a field opens

A site turns into a field through one of two doors, both from its open surface:

- **A Hostile mark.** A party's leader marks the party of a flight on the presence
  picture Hostile with `+field/mark <bearing> [<band>]=hostile`.
- **A fire order.** The pilot of a unit on the ground orders a shot at a flight with
  `+field/fire <bearing> [<band>]`. The site promotes, and the shot is taken in the
  field's first moments, as soon as your own sensors hold the target. If they do
  not hold it within ten seconds, no shot is fired.

A flight is named by what you see: its bearing (`N`, `NE`, `E` … `NW`) and, when two
share a bearing, its band (`close`, `near`, `far` or `distant`). The flight
meant is the nearest one that matches, seen from the nearest of your own units.
`+field/mark E near=hostile` marks the near flight to the east. Nothing about who
it is is printed back.

A leader's `ENGAGE` order, delivered on open ground, opens a field the same way
(see *Orders to a hired escort* below).

**Everyone on the site is admitted where they stand.** Every unit on the ground
joins: rigs mid-drive and mid-dig, armed hulls, unarmed haulers, and the carriers
on the pad. A drive in progress stops where it has got to. A mining run stops
after the cycle it is on, and that cycle's ore is kept. A launch or recover already
on the ramp finishes first. Nobody is left out, and nobody is moved.

You hear "Field 1 is open on Ahuna Flank: the ground there is tactical until it
settles." Your seat now flies the cockpit (`help cockpit`): `CONTACTS`, `LOCK`,
`FIRE`, `STATUS` and `DRIVE` work as in the simulator. Every unit starts powered
up, and every seat starts at weapons hold, so a standing `FIRE … WHEN` rule does
not shoot until you set `FIRE FREE`. A hand `FIRE` is always yours. After a
server restart, take your seat back with the `+focus` the resume line prints.

**No launches while promoted.** `+sortie/launch` onto the site is refused until
it settles: "Field 1 is being fought on Ahuna Flank. No launch goes down onto it
until it settles: the fight runs at human speed and will be over before you
could join it." A rig's `+tv/<n> launch` is refused too. Other surface work on
the site waits as well:
"Field 1 is being fought on Ahuna Flank. Surface work there waits until it
settles."

**The field settles** once no two hostile parties have fired, held each other in
weapons range or had a round in flight for a while (two minutes by default). You
hear "Field 1 on Ahuna Flank has settled: the ground there is the surface's
again." Every unit stands on the surface where the fight left it. Two hostile
armed parties parked within range of each other keep the field open.

  +field                   the field your party stands in: its number, your
                           party's letter, how many parties are on the ground
  +field/mark <bearing> [<band>]=hostile  on open ground: mark a flight's party
                           Hostile, which opens a field
  +field/fire <bearing> [<band>]  on open ground: fire on a flight from the
                           unit you fly, which opens a field
  +field/mark <letter>=hostile|friendly|clear  inside a field: your party's
                           mark on another party

## Parties, marks and who fights whom

Each party in a field gets a letter in the order it was admitted. The letters
say nothing about who opened the field, and they skip `I`, `O` and `X`. A
detachment's accepted roster is one party (`help detachments`). Any other unit
is a party of one: its pilot leads it, or, with nobody aboard, the operator who
launched it. A carrier is in the party of the roster it launched, or else a party
of one led by its commander. `+field` tells you your own letter.

**Only a party's leader marks, and marks are private.** `+field/mark B=hostile`
makes your party hold party B Hostile; `=friendly` holds it a Friend; `=clear`
drops your mark. Only your own party's book changes. Party B is never told,
and nobody else sees your marks. Inside a field a lone `N`, `E`, `S` or `W` is a
party's letter, not a bearing.

What your party believes of another is decided in this order, and the first
that applies wins:

1. **Your leader's mark**, Friendly or Hostile.
2. **Fire.** A party that fired on you, or on a Friend of yours, in this field
   is Hostile.
3. **Standing friendship.** Two parties are Friends when one owner stands behind
   both: the same organizer for two rosters, the same leader for two parties of
   one, or a roster's organizer leading a party of one. Your carrier and the
   rigs you launched from it are Friends. Organizations and factions do not make
   friends yet.
4. **Neutral**, for a party told apart that is none of these. A contact your
   sensors have not yet classified is **unknown** until they do.

A mark beats fire, so a Friendly mark is how a leader stands down after friendly
fire, or after a fight is over. While your party holds a party Hostile and an
armed unit of yours has it in range, the field stays open. A Friendly mark on
that party lets it settle.

Relations decide what a hired pilot's doctrine targets and how your cockpit shows a
contact. They never stop a shot: you can fire on anyone you hold a lock on. A
lock on a Friend reads `(FRIENDLY)`.

**Designators.** In the cockpit a contact is named by two letters: its party's
letter and a unit letter drawn at random once per unit (`BK`, `DQ`). `LOCK BK`
locks it, and an order can name it.

## The one picture

The site map you mine by is the map you fight by.

**On open ground**, `+tv/map` (the TUI's quarry screen, the web's quarry tab)
draws the presence picture: every other flight on the ground. Within 200 m of one
of your rigs a flight shows at its cell as `o` and is listed in metres under
"Other rigs:". Farther out it is a row giving its bearing and band, which is what
`+field/mark` and `+field/fire` take. A loud sounding heard from beyond that range
marks its row "(a sounding)".

**During a field**, the quarry screen draws the field's contacts as your own
cockpit holds them, in the cockpit's glyphs, with your own unit as `@`. Your
cockpit's map draws the campaign layer under its contacts: the blocks, the
claims, the pad, the public pits and your readings. Every client draws the same
picture.

**Ground objects.** Remains and crates on the site are walls in a field. They
block movement and sight and give cover. Your sensors see one within 200 m in a
clear line. What your party saw, it remembers after the field. You are shown an
object you remember, marked "(remembered)", wherever you cannot see it now.

**Your party's readings.** While a field runs, a seat in one of your party's units
shows, on that site's map, the ore readings of every principal of the party
beside your own. The principals are its leader, the pilots who sat in its units and
its rigs' operators. A roster's escort who never sounded therefore sees the
miners' worked blocks. This is display only. Nobody's chart is written, your own
reading wins where both exist, and the shared readings leave the map when the
field closes. A Friend party's readings are not shown.

## Orders to a hired escort

A roster's leader orders hired NPC escorts with the same fenced lines as any
detachment order (`help detachments`). `+sortie/orders` prints the prefill.

  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL ENGAGE [<target>]
  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL HOLD HOLD FIRE
  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL WITHDRAW [HOLD FIRE]

- **ENGAGE.** Inside a field, `ENGAGE BK` names a contact in your own cockpit by
  its designator. On open ground, `ENGAGE NE` or `ENGAGE NE-near` names a flight
  on the presence picture. That order is your Hostile mark on its party and opens
  the field.
- **HOLD FIRE** is a tail on another order, never an order alone:
  `HOLD HOLD FIRE` stands the unit where it is and holds its fire, and
  `SCREEN 90 HOLD FIRE` screens without shooting. A hired pilot's doctrine stops
  firing from the moment the order lands.
- **WITHDRAW** breaks off and drives the unit home to its carrier's ramp, then
  takes it out of the field (see *Getting out*). With `HOLD FIRE` it fires on
  nothing on the way. Without it, it fires only when cornered.

Orders travel over the roster's own transport, with its delay. A declared
jamming level where the units stand holds orders and replies there until it
lifts. They are delayed, never lost.

A hired NPC answers `WILCO` once the order stands on its own order row.
It answers `NEGATIVE` when the contact you named is not one it holds, or for
anything a field cannot carry out: a mining run, a presence row inside a field,
a contact from another field, or support for a unit outside the field or the
party. A unit that cannot reach its ramp answers WITHDRAW by breaking off and
holding where it stands. You see a reply only once it arrives.

A hired pilot's doctrine fires on the parties its party holds Hostile, and only
on them. Your marks are its targets.

**A player who is given an order keeps control.** An order to a player's unit is
advice. The player reads it in `+sortie/orders` and may answer `WILCO` or
`NEGATIVE`, but the order never drives, locks or fires their unit.

## What a hit does

A unit in a field takes damage by section, as in the simulator. Three outcomes
matter after the fight:

- **Immobile.** A destroyed engine or drive section stops the unit moving. Its
  frame reads `immobile`, not `wreck`. It may still shoot. When the field settles
  it is left as a **casualty**.
- **Casualty.** Any unit the field leaves immobile, unarmed or disabled.
- **Wreck.** A control-section kill (a walker's head, or a cockpit or bridge
  where a design has one) or a destroyed hull. The unit is a **wreck**.

**Units stay where they fell.** A casualty or a wreck stays on the ground at the
cell where the field left it, and the site keeps it. It is still there after a
server restart, when another captain lands, and in the next field on that site,
which admits it again. Casualties and wrecks do not count against the site's
unit cap. A wreck blocks movement through its cell but not sight or fire: it
gives no cover yet. On the map a casualty, a wreck or remains is `%` and a crate
is `&`.

A field closes no unit's log, and nothing is towed home: there is no tow yet,
and a carrier is never towed. The damage a hull took is written into its own
record, and `+ship` prints a "Damage:" line for each damaged section.

## Getting out

**Up your own carrier's ramp, under the unit's own power.** There is no edge
exit. Drive to within one cell of your carrier's ramp (`DRIVE x,y`), then recover
it as you would on the surface: `+tv/<n> recover go` for a rig, or `+sortie/recover`
for a detachment unit. You hear "#123 makes for the ramp: it leaves field 1 at
the next pulse if it stands beside the ramp and can move under its own power."
Once it is out, its recover into the bay runs, and the field carries on without
it. A recover cannot drive a unit to the ramp in a field. From farther off you
hear "That unit is not beside the ramp; drive it there first, then recover it."
An immobile or wrecked unit hears "That unit cannot come in under its own power;
it stays in the field." Its crew's way home is extraction.

A unit that leaves is checked in as it is, damage and all. If the server
restarts after the exit, run your recover again, and it passes.

**The carrier.** A carrier does not drive to a ramp. A live `+ship/lift` or
`+ship/undock` takes it out of a running field, after every check a departure
makes (see *Leaving units behind*). You hear "The Salt Moth breaks off: it leaves
field 1 at the next pulse, and its departure files once it is clear. The units
it leaves stay in the field." A queued departure from the sailing plan never
takes a carrier out of a field.

## Crew

The game keeps a record of every character a field touched: **where** they are and
**how** they are.

- **Where.** Aboard a unit, aboard a carrier (a pilot at a remote console on the
  carrier), **stranded** on the site (inside a hull that cannot bring them home
  under its own power, or a wreck), or at a port.
- **How.** Fit, **injured** (a control section damaged around you) or
  **incapacitated** (a control section destroyed around you). A pilot at a remote
  console aboard the carrier is always fit. There is no ejection, capture or
  treatment.

A control-section kill incapacitates you at once: "Your head is gone and you with
it, for now: you are incapacitated." A stranded pilot is told so when the field
checks the hull in.

**Extraction.** A stranded or incapacitated character still inside the hull asks
for it with `+sortie/extract`. Nothing extracts you on its own, so you may stay
and guard the wreck. After a delay of IC time (`[tacsim.field].extract_s`, six
IC hours by default) a recovery service sets you down at the safe port you know:
the port you last stood in, or else Stickney Yards on Phobos. Your hull, its
cargo and its log stay where they are, and your seat is left empty. While world
time is frozen the request is refused and the delay does not run.

**A Friend's recovery flight.** If a Friend has an open recovery flight on the site
(a detachment flight activated for `recovery` duty under the same organizer as
your own party), your extraction waits for it. It waits up to
`[tacsim.field].rescue_bound_s` past the due time (two IC hours by default), then
lands you anyway.

  +sortie/extract          stranded or incapacitated: ask a recovery service to
                           lift you to a safe port

## Leaving units behind

A carrier normally lifts only with every rig and detachment unit aboard. To go
without them, name them on a live departure with `leave=`:

  +ship/lift <port> leave=<slots and #units>
  +ship/undock <port> leave=<slots and #units>

Rigs are named by bay slot and detachment units by number:
`+ship/lift ceres leave=1,#345`. A departure that does not name every unit out
is refused with the unit holding her, and the refusal ends with the `leave=` list
it needs. A unit mid-act or on the ramp is never left; wait for it.

`leave=` records the units on the filed voyage, and tells you and each left
unit's operator: "Left behind by your order: #345. They stay where they stand,
still yours." It recalls nothing, releases nothing and closes nothing. Each unit
stays on the ground where it stands, still yours, its log open, and its lease
runs on. Units left in a running field stay in the fight. A queued departure
never leaves anything behind. A carrier with nothing out needs no `leave=`.

## The armed rig lease

At Occator's counter in the Exchange Hall at Ceres Freeport, with your Dray-P
docked, `+tv/lease escort` shows the terms for an armed escort line tank, and
`+tv/lease escort go` signs them: 2,500.00cr for 168 IC days and a 15,000.00cr
deposit. It is the simulator's line tank with its 105 mm gun and coaxial machine
gun, in a one-seat cab. At 8 t it rides a Dray-P's bay beside one prospector.

It needs the **escort clearance**:

- your prospecting licence in force, not in its grace;
- no lease of yours forfeit;
- one armed hull at a time;
- none written off in the last 28 IC days.

Get the operating loan the same way as for a prospector (`+tv/<n>/permissions go`).
Then go down to the bay from the hold (`+tv/bay in`) and board with `+tv/<n> board`,
taking control with the `+focus` it prints. The cab's one console carries the
helm, the sensors and the guns. From inside the cab you can launch, drive and
recover it with `+tv/<n>`.

A leased rig wrecked out of the bay is **written off** when you hand it back
(`+tv/<n> lease return go`, at the counter, wherever the wreck is). The lease
ends, Occator keeps the deposit, and the wreck stays where it fell. A damaged
hull that comes home goes back with its damage and no repair charge.

**A combat crew.** At the Slack Line in Ceres, `+sortie/crew` also offers Rook
Adeyemi, an escort driver under a combat contract only. Hire him with the printed
`+sortie/hire` command, seat him in an armed unit, and activate the roster for
combat (`+sortie/activate <draft>:<revision>=combat`). A combat roster never
mines. Lend him the unit's operating right as `help detachments` and
`help unit-grants` describe.

See also: `help mining`, `help detachments`, `help cockpit`, `help ships`.
