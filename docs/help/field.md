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
range or had a round in flight for a while (two minutes by default), and in any
case once nobody has fired for ten minutes (`[tacsim.field].idle_demote_s`). You
hear "Field 1 on Ahuna Flank has settled: the ground there is the surface's
again." Every unit stands on the surface where the fight left it. Only a unit
that came into the field armed, or has fired in it, holds another party engaged:
an unarmed rig in a Hostile party's sights keeps nothing open. Two hostile armed
parties parked within range of each other keep the field open until those ten
quiet minutes pass, and no longer.

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
   rigs you launched from it are Friends. Two parties are also Friends when
   their leaders or owners openly share an organization: an organization
   membership, a faction membership or a faction's principalship that fellow
   members may know of. A covert or officers-only membership makes nobody a
   Friend, since a Friend's colour on a cockpit would give it away. A raider
   band is a faction of its own, so a captain who openly stands in it is its
   Friend until it fires on them.
4. **Neutral**, for a party told apart that is none of these. A contact your
   sensors have not yet classified is **unknown** until they do.

A mark beats fire, so a Friendly mark is how a leader stands down after friendly
fire, or after a fight is over. While your party holds a party Hostile and an
armed unit of yours has an armed unit of theirs in range, the field stays open,
until ten minutes pass with no shot fired. A Friendly mark on that party lets it
settle sooner.

**Standing.** Fire costs standing. When the field settles, every round that
reached a unit of another party costs the shooter's party's principals one point
of standing with each faction that party's principals openly stand in, at most
ten a field for one pair of parties. Each principal is told. A shooter whose
party fired on a standing Friend is offered the void: `+field/void` within ten
IC minutes, or before the leader's next command, drops that change. Fire on
anyone else stands.

A leader who is not connected when the field settles does not lose the offer.
It waits for them: at their next login they read "While you were away your
party's fire on a friend cost standing: 1 change(s) wait on you. +field/void
cancels them, until your next command or ten IC minutes." The ten IC minutes
run from that login. Let it pass and log in again, and it is offered again.
28 IC days after the field settled it is gone, taken or not. Standing itself
moves the same whether or not the leader was there.

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
  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL CONCENTRATE <contact>
  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL HOLD HOLD FIRE
  ORDER DT:<detachment>:<revision>:<term> <slot>|ALL WITHDRAW [HOLD FIRE]

- **ENGAGE.** Inside a field, `ENGAGE BK` names a contact in your own cockpit by
  its designator. On open ground, `ENGAGE NE` or `ENGAGE NE-near` names a flight
  on the presence picture. That order is your Hostile mark on its party and opens
  the field.
- **CONCENTRATE** is the focus-fire order. `ALL CONCENTRATE BK` names one contact
  you have identified in your own cockpit, and every hired unit that holds that
  contact itself engages it. A unit that does not hold it answers `NEGATIVE` and
  keeps the order it had. When the contact is destroyed, or a unit loses it, that
  unit goes back to the order it had before. `ALL` reaches the units that can
  take the order and is not refused for one that cannot. An NPC who leads a
  roster gives the same order while it holds (`help detachments`).
- **HOLD FIRE** is a tail on another order, never an order alone:
  `HOLD HOLD FIRE` stands the unit where it is and holds its fire, and
  `SCREEN 90 HOLD FIRE` screens without shooting. A hired pilot's doctrine stops
  firing from the moment the order lands.
- **SCREEN and SUPPORT** bind only inside a field. `SCREEN <bearing>` holds a
  screen facing that bearing, and `SUPPORT <slot>` keeps beside the roster's unit
  in that slot. Both are combat formations, so on open ground, where nobody is
  fighting, a hired pilot answers either one `NEGATIVE`. Support for a unit that
  is not in the same field and party is `NEGATIVE` inside a field too.
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

If you go away, an ENGAGE, a CONCENTRATE or an ADVANCE you gave lapses when your own unit
passes to its mind (see *If you lose your link, or quit*). The hired unit stops
where it is, holds its fire, and answers from where it stands only once it is
hit. Any other order you left stands. `help detachments` has the whole rule.

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
unit cap. A wreck blocks movement through its cell, and while it stands there it
is a wall to sight and to fire, like a crate: a unit behind it is covered from
the far side, and a shot at the wreck itself reads the ground under it. A
casualty gives no cover. On the map a casualty, a wreck or remains is `%` and a
crate is `&`.

A field closes no unit's log. What it leaves is recovered from the surface once
the site has settled:

- **The tow.** A rig standing within a cell of a casualty or a wreck on the
  site's index takes it in tow with `+tv/<n> tow [<unit>] go`. The bare command
  is the inspection: the casualty's tonnage, its state and the rate the pair
  will move at. Inspecting and attaching take IC time. A tug in tow only
  drives, releases (`+tv/<n> release go`) or loads, and a casualty in tow
  starts nothing. A tow survives a server restart, and a field that opens over
  it admits the pair coupled, at the tow's rate. The tow counts the casualty
  laden: her hopper's ore, her hold's goods and the hulls in her bay. Any rig
  tows up to twice its own mass; the tender's recovery coupling tows three
  times its mass.
- **The tender.** Occator leases a support rig (`+tv/lease tender`) whose
  stores carry munitions, structural alloys and yard fittings. In its
  carrier's bay, `+tv/<n> stores load <qty> <good> go` takes goods from the
  hold, and `+tv/<n> stores open go` opens its bale, a tonne of alloys and a
  tonne of fittings that the work then uses by the kilogram (`+tv/<n> stores`
  shows both). On the ground, within a cell of a unit, it clears a jammed part
  (`+tv/<n> unjam <unit>[=<part>] go`), patches a damaged section
  (`+tv/<n> stabilize <unit>[=<section>] [<points>|all] go`) and refills a
  magazine from its stores (`+tv/<n> rearm <unit>[=<part>] go`). Each takes
  three times a workshop's time and draws the same materials. The bare
  command shows the work, the draw and the time. A destroyed part or section
  still needs a workshop. While it works the tender does nothing else. A field
  that opens over the site stops the work, and the work goes on when the
  field settles. Nothing is paid between players.
- **The load.** Beside its carrier's ramp, `+tv/<n> load go` lifts the casualty
  into the bay, where `+bay` quotes its repair (`help ships`). Loaded into your
  own party's or a Friend's carrier, it is home. Loaded into anyone else's, it
  is in that carrier's custody with a salvage claim owed, never its title, and
  anyone still aboard is taken captive (see *Crew*). The custodian files the claim
  (`+salvage/claim <hull> go`), everyone who answers for the title (a company's
  people, or its agent) is told by IC mail, and either side's evidence is read
  and contested at the registry (`+salvage/inspect`,
  `+salvage/contest <hull>=recovery|procedure|right`): an unlawful recovery on
  your own record of it, a defect in the claim's procedure, or a right of yours
  that bound the claimant. A title holder authorizes a recovery beforehand with
  `+salvage/authorize <hull>=<who> go`, a landholder with
  `+salvage/authorize <site>/<block>=<who> go`. Repairs a salvor makes are at
  their own risk and are never repaid. An uncontested claim passes the title
  when its window runs out. A vacated claim leaves her held for her owner in
  the custodian's bay: aboard a carrier of yours standing where hers does (one
  port, or one pad), `+bay/stow <hull>=<your carrier>` takes her. If she is
  still aboard when the custodian docks at a port with a workshop, the port
  berths her there for you, and you stow her from the workshop floor.
- **Port security.** The Freeport Registry acts only on proof: a recovery a
  `recovery` contest vacated, or an act a witness reports to it with a record
  of their own (`+salvage/report <site>[=<who>|#<hull>] registry`). When the
  carrier you command docks at Ceres or Stickney, each proven act of yours is
  fined (an unlawful pickup 500 cr, scrapping 2,000 cr, a recovery 5,000 cr).
  A fine your account cannot cover is a debt (`+debts`), and your carrier is
  held at the port until you pay it (`+debts/pay <id>`).
- **A carrier** is towed only by a tug of at least half its tonnage, which no
  rig is. A carrier whose drive is destroyed cannot lift. One whose hull falls
  is a wreck that never lifts and strands its crew.
- **Remains.** A hull that goes down with every section destroyed is
  irrecoverable: it leaves the site's index and the ground, and remains with its
  mass lie where it fell. The title holder's `+tv/<n> scrap <remains> go` breaks
  them up into structural alloys in the carrier's hold.

The damage a hull took is written into its own record, and `+ship` prints a
"Damage:" line for each damaged section.

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
  under its own power, or a wreck), at a port, **held** aboard another party's
  carrier (a captive), or **in processing** at a port after a captor hands you
  over.
- **How.** Fit, **injured** (a control section damaged around you) or
  **incapacitated** (a control section destroyed around you). A pilot at a remote
  console aboard the carrier is always fit.

**Held.** A captive is a passenger who cannot leave and cannot operate. Exits,
boarding and leaving a hull or a rig, the bay, every unit, ship, field, bay and
salvage act, the cockpit, `WILCO`, `NEGATIVE`, `@station`, `+focus`, the
`+sortie` switches that seat, hire, launch or recover, `+sortie/chat` and
`+sortie/extract` are refused with one line: "You are held aboard the Kestrel."
Speech in the room, `look`, `+sortie/condition`, mail, pages and OOC channels
work, and so do the reads (`+ship`, `+field`, `+tv`, `+sortie/operation`) and
standing your own operation down (`+sortie/deactivate`, `+sortie/end`). A
character in processing at a port may go anywhere in the port and nowhere else:
"You are being processed at Ceres Freeport; about 1 IC h."

**Taken at a load.** A casualty loaded into a carrier that is not your party's or
a Friend's takes everyone still inside it captive, a Neutral salvor's as much as
an enemy's. You step out into that carrier's bay room, held by the carrier's
operator, and are told so. A captor who was no enemy of yours (no declared
Hostile stance between your organizations, and no fire either way in the field
your hull came out of) loses 5 standing with each faction you openly stand in. A
hired NPC flying the hull is never taken: they leave the seat, go back to the
crew room they were hired in, and their contract with you ends; until that lands
they are not on offer to anyone. Taking a person off owned ground without the
title holder's or the landholder's leave is written in the site's evidence log.

The captor reads who they hold with `+ship/captives` and lets one go with
`+ship/release <name|#n>`, for nothing: docked at a port, the captive is set down
in its arrivals; anywhere else they ride on aboard as a passenger.

**Handed over at a port.** When a captor's carrier docks at Ceres, Stickney, Luna,
Deimos or Titan, every captive aboard is handed to the port's authority (the
Freeport Registry at Ceres, Stickney Yards at Stickney, Concord Orbital Access at
Luna, the Voltaire Bond Authority at Deimos, the Titan Concession Company at
Titan). You are set down in the port's medical room, or its arrivals where it has
none, and are in processing for an IC hour (`[tacsim.field].release_s`): you may
go anywhere in the port and nowhere else. The authority charges a release fee of
1,500 cr to your own account. If you cannot cover it, it stays with you as a debt,
and you are released all the same. The captor is paid nothing. After the hour you
are free, and the port is the one you know. Treatment begins as soon as you stand
in the port.

**The limit.** A captor who never docks cannot hold you for ever. Seven IC days
after the capture (`[tacsim.field].captive_bound_s`), `+sortie/extract` works from
captivity: a recovery service negotiates your passage and sets you down at the safe
port you know after the usual delay. Before then it is refused with the time left.
A captive the captor releases anywhere but a port rides on as a passenger and may
ask for extraction the same way. If the captor's carrier is wrecked in a later
field, you are stranded in the wreck like everyone aboard it, and the captivity
ends with the hull.

**Ejecting.** A pilot whose unit cannot bring them home, or whose cockpit is
already hit, may leave the seat at once with `+sortie/eject`, from inside the
unit (a console has nothing to eject from). The seat empties, the hull is the
field's, and you wait in the cockpit for a pickup to the safe port you know after
the usual delay (`[tacsim.field].extract_s`); `+sortie/condition` shows it. An
ejection never hurts you: you keep the condition the hull last gave you. A unit
that can still bring you home refuses ("#n can still bring you home").

If a party that holds you Hostile has an armed unit within reach of your unit,
no unit of yours or a Friend's has it within its own reach, and that party has a
carrier on the site, the pickup may be theirs (a fixed chance,
`[tacsim.field].eject_capture_permille`, even by default): you are carried aboard
their carrier and held there, as above.

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

**A Friend on the site.** If your own party or a Friend still has a hull on the
site (an open detachment flight in its field on any duty, or a leased rig standing
on its ground, other than the hull you are stranded in), your extraction or your
pickup waits for them. It waits up to `[tacsim.field].rescue_bound_s` past the due
time (two IC hours by default), then lands you anyway. A pickup that is a capture
never waits.

**Treatment.** An injured or incapacitated character who stands in a port is
under treatment there and is fit again after a stay of IC time: one IC day when
injured (`[tacsim.field].treat_injured_s`), three when incapacitated
(`treat_incapacitated_s`), and half that at a port with a medical room (Ceres's
arrivals hall and Stickney's customs hall have one). A medical room has a number
of beds (two at Ceres, four at Stickney): when every bed is taken, a new arrival
is treated at the full time, "waiting for a bed", and takes the first bed that
frees, the time left halved from then. There is no check and no
charge. You are told when treatment begins and when you are fit again. If you
leave the port first (back aboard a hull, out to a site, or out to Limbo or the
OOC area), you keep your condition and the clock starts over at the next port you
stand in. While world time is frozen the clock does not run.

  +sortie/extract          stranded or incapacitated: ask a recovery service to
                           lift you to a safe port
  +sortie/eject            leave the seat of a doomed unit for a pickup
  +sortie/condition        how you are and where, and the time left on an
                           extraction, a treatment or a processing hour

## While you were away

A field does not wait for you to be connected. If you have a unit or a carrier
on a site when a field opens there, you are told, online or not. A unit counts
as yours when you sat in it as the field opened, when it is a rig you sent out,
when it is a carrier you command, or when it belongs to a party you lead or
organized. A player with nothing on the site is told nothing.

- **The opening and the settling** are the two lines everyone on the site hears.
  Connected, you hear each once. Not connected, they wait for you, and you read
  them at your next login under "While you were off watch:".
- **What became of your units**, when the field settles, one line a fate:

      After field 1 on Ahuna Flank, lost: Dust Mule.
      After field 1 on Ahuna Flank, home: Salt Moth.

  The fates are `lost` (a wreck, or destroyed), `captured, aboard another's
  carrier`, `taken in tow`, `down on the site, a casualty`, `holding on the
  site`, `home` (aboard its own party's carrier or a Friend's, or a carrier
  still on its pad) and `gone from the site`. A line names up to four units and
  counts the rest, so one field never fills your notices however many units you
  had there.
- **A letter** comes by IC mail, under the site's name, when a unit of yours was
  lost, left a casualty, captured or taken in tow, or when the close left you
  stranded on the site. It lists every unit you had in the field (`help mail`).
- **A capture.** A character taken captive when their casualty is hauled into
  another party's bay is sent the capture's line by IC mail, and reads it at
  login if they were away (see *Crew*).

A salvage claim on a hull of yours reaches you by IC mail as before.

## If you lose your link, or quit

Your unit always has a mind: the one you launched it with, or the house default
every unit carries. While you are at the controls it does nothing. When you are
not, it flies the unit by the posture you left on it. That is RETURN unless you
set another, and RETURN takes the unit home; this section describes it. GUARD
and HOLD keep the unit where it is (see *Postures* below).

- **A lost link.** For five minutes your unit is still yours. It keeps the
  heading and speed you gave it and fires nothing. Reconnect inside the five
  minutes and you hear "You are back at your station on ...; it held your orders
  and fired nothing."
- **After five minutes** the unit passes to its mind. It breaks off and drives
  for your carrier's ramp. When it stands beside the ramp the field lets it out
  and it is put aboard your carrier, as your own recover would. If the recover
  is refused it is tried again. If no carrier of your party stands on the site,
  the unit has nowhere to go and holds where it stands.
- **A clean `QUIT`** passes the unit to its mind at once. There are no five
  minutes to wait for someone who chose to leave.
- **If you are knocked out** in the hull (its head, cockpit or bridge destroyed),
  you stop flying it at once, connected or not. You hear "You cannot fly #123:
  its mind has the controls and is taking it home. It fires only on a unit that
  fires on it." A hull that has lost its controls cannot drive to the ramp, so
  it holds. You get the controls back only when you are fit again.
- **If a field opens while you are away.** Logging off in a unit's seat on open
  ground leaves you named in that seat. A field that opens around the unit does
  not take you for its pilot: it asks whether you are connected. If you are not,
  the unit is its mind's from the moment the field takes it in, exactly as after
  a `QUIT` inside a field, and you read at your next login "A field opened
  around #123 while you were away: its mind has the controls, makes for its
  carrier's ramp and fires only on a unit that fires on it." The same holds if
  you are in the seat but knocked out or held.
- **If your link dropped just before the field opened**, the five minutes are
  still yours, counted from the moment the link dropped and not from the
  field's opening. The unit waits in the field, firing nothing, for what is left
  of them, then passes to its mind. A link lost more than five minutes before
  is not waited for.

It is the same rule each time: the unit is yours while you are connected and at
its controls, and five minutes after your link drops, or at once if you quit or
were not there to begin with, its mind has it. A connected pilot is taken into a
field as their unit's pilot, as always.

**A rig nobody sits in.** A rig you launched and work from your carrier, or left
on a fill plan, has nobody at its controls. A field that opens around it takes
it in as its mind's from the first moment, whether or not you are connected,
and you are told "A field opened around #123 with nobody in its seat: its mind
has the controls, makes for its carrier's ramp and fires only on a unit that
fires on it. A plan it was working ends once it is aboard." It flies its
posture like any unit:

- On **RETURN**, which every rig holds until you set another, it drives to your
  carrier's ramp, is let out of the field and is put aboard, into the bay slot
  it launched from. **Its fill plan ends when it comes aboard**, and you read
  "Rig 2's plan at Ahuna Flank ended: a field opened around it and it was taken
  home." with what it mined. Ore in its hopper is unloaded once the field has
  settled. A rig you sit at the console of and recover out of the field
  yourself (`+tv/<n> recover`) ends its plan the same way. File the plan again
  when the site is quiet. A rig that cannot move, or is in tow, stays where it
  is whatever its posture.
- On **HOLD** it stays on its cell. Its plan starts nothing while the field
  runs and goes on from where it stopped when the field settles. You are told
  "... its mind has the controls and flies it on HOLD. It will not fire first.
  A plan it was working runs on once the field has closed."
- A rig with nothing to fire cannot guard. Set to **GUARD** it stays, as on
  HOLD.

Set the posture from aboard the carrier before you leave (`+tv/<n> posture
hold`); see *Postures*. To fly such a rig yourself, board it before the field
opens.

**Your carrier.** Nobody flies a carrier in a field, so its mind holds it too.
A carrier on its pad is home: it stays where it is, fires only on a unit that
fires on it, and takes your rigs into its bay as their minds bring them to the
ramp. Its mind never lifts it. The only way a carrier leaves a field, and
leaves rigs behind, is your own live `+ship/lift` naming every unit you have
out (`help ships`). A carrier that is wrecked wrecks the rigs in its bay, so a
rig sent home is as safe as the carrier is.

**What the mind will and will not do while you are away.** It never opens fire
first. It fires only on a unit whose fire has arrived on your unit in this
field, hit or miss. A party you or your leader marked Hostile is not enough, nor
is one your organisation declared Hostile, and neither is fire on a friend. (A
unit left on GUARD also answers for your other units: see *Postures*.) This
binds any mind, one you wrote and filed as much as the house default: a shot it
asks for at anyone else is refused. It never mines or trades for you.

The seat's own switch (`FIRE HOLD`, where every seat in a field starts) is yours
while you fly, and it does not bind the mind. A unit whose seat you left at
`FIRE HOLD`, and a unit taken into a field while you were away, still return
fire on a unit that fires on them. What you leave standing is the posture, not
the switch. An
order to advance or attack that stood on the unit lapses. A hold-fire order
from your leader stays, and a unit under one answers nobody.

**Those you were with are told.** The leader of your party, and the leaders of
parties that stand as your friends, hear "#123 breaks off for its carrier's ramp
under its mind: its pilot's link is lost. It will not fire first." For a unit
left on GUARD or HOLD they hear "#123 passes to its mind, which flies it on
GUARD: its pilot's link is lost. It will not fire first." A leader who is
offline reads it at login.

**Coming back.** Reconnect at any time while the field runs and your unit is
still in it, and you are seated again: "You are back at the controls of #123;
its mind stands down and what it had pending is dropped. +focus #123/cockpit
takes your cockpit back." Take the focus the line names, and your next command
is what the unit does. If the mind had already taken the unit out at the ramp,
it stays out, and you hear "While you were away its mind took #123 out of the
field at its carrier's ramp."

**A server restart** does not put you back at the controls unless you are
connected. A unit its mind was flying is still its mind's. A unit you were
flying waits five minutes from the restart for you to reconnect, firing
nothing, and then passes to its mind.

The simulator is different: there a dropped pilot's unit goes to its mind and
fights on (`help simulator`).

## Postures

Every unit holds one posture: the standing word its mind reads when you are not
at its controls. There are three, and a unit holds RETURN until you set another.

- **RETURN**: go home. This is what the section above describes.
- **GUARD**: stay where it stands and defend.
- **HOLD**: stay, and hold its fire.

The posture belongs to the unit, not to the field around it. Set it on the open
surface and it is still there when a field opens, after a restart, and when the
field has settled.

**What each does while you are away.** Whichever mind holds the unit flies the
posture, the house default and one you wrote and filed alike (`help minds`). The
limits above stand under all three: it never fires first, and it never mines or
trades.

- **RETURN** takes the unit home, as the section above describes. It returns
  fire on a unit that fires on it, on the way and beside the ramp, and it
  does not stop to do it: the house default fires as it drives.
- **HOLD** keeps the unit on the cell where its mind took it. It turns to face
  a unit that has fired on it and returns that unit's fire. It answers for
  nobody else, and it is not taken home.
- **GUARD** keeps the unit there too. It also returns fire on a unit that has
  fired on another unit of yours in this field: a rig you launched and left,
  the unit you sit in, the carrier you command. A quarrel between strangers
  beside it is not its business, whoever you marked Hostile and whatever your
  organisation declared. It answers for your units that are in the field: one
  that has gone aboard is safe, and is no longer answered for. A mind of your
  own may move, and it answers for your other units only within ten cells of
  where it took the unit; past that it answers for itself alone.
- **GUARD ends** when you are back at the controls, or when the unit can no
  longer defend itself: no fire channel with a round left, or its motion
  disabled. It is then flown as RETURN and goes home if it can. The posture
  you set is kept, and `+tv` and `MIND POSTURE` go on showing it: a unit that
  can defend itself again guards again.
- **A unit with nothing to fire cannot guard.** A prospector set to GUARD stays
  where it stands and answers nobody, exactly as on HOLD, and is not taken
  home. Set RETURN to bring it in.

A hired crew's unit is not yours for this rule: a unit on GUARD does not answer
for it.

If you were away when the mind took a unit left on GUARD or HOLD, you read at
login "While you were away #123 passed to its mind, which flies it on GUARD and
will not fire first." The unit stays in the field. When the field settles it is
on the site where it stood, under the same posture for the next field.

Under the house mind a unit on GUARD or HOLD stays on its cell however badly it
is hurt. One that has nothing left to fire falls back to your carrier's ramp,
as its doctrine has it. On GUARD it is taken aboard there; on HOLD it stands
beside the ramp and is not.

**Setting it.** A posture is an order to a unit, and it is given the way an
order reaches one:

- **From its seat**, in a field: `MIND POSTURE GUARD`, from the seat that
  holds the helm. `MIND POSTURE` with no word reads it.
- **From aboard its carrier**, or from the rig's own cab on open ground:
  `+tv/2 posture hold` sets rig 2's. `+tv/2 posture` reads it.
- **A ship's, from aboard her**: `+ship/posture hold`. It is the same record
  and the same words, and it is what a mind flies her on in space when her
  crew has no link (`help ships`, *Her posture*). On her pad she is your
  carrier and stays, whatever the word.
- **Over the relay.** If you are on the site but neither aboard the carrier
  nor in that rig's cab (in another rig's cab, say), the same
  `+tv/2 posture hold` goes to rig 2 as an order over the comms relay. Jamming
  where the rig stands stops it, and so does the carrier relay of the rig's
  party when it reads jammed in a field.
  You hear "Your order does not reach Rig 2: the relay to it is jammed. It
  still holds RETURN." and nothing has changed; an order that does not arrive
  is not kept for later. From another site or a port you have no relay to the
  rig at all: give the order from aboard its carrier.
- **By your detachment's leader.** The leader of a roster you fly in may set
  your unit's posture with a roster order (`help detachments`). You hear, or
  read at your next login, "Vance, leading your detachment, ordered #123 to
  GUARD: it holds that word until you set another." The unit answers that
  order itself; you do not. Set the posture yourself, any of the ways above,
  and yours stands until the leader orders it again.
- **Never from an OOC room.** "A posture is an order to a unit in the world.
  It is not given from an OOC room." Nothing you do in the simulator sets a
  posture on a unit in the world.

**Trying one first.** In the simulator `MIND POSTURE` sets a practice posture
on the tank you fly, for that match only, and `MIND AWAY` lets go of its
controls so you can watch its mind fly it under the rules above (`help
simulator`, *Practising a posture*).

**Reading it.** `STATUS` in the seat and `+tv` aboard the carrier each show a
line "posture GUARD" for a unit that holds one other than RETURN. A unit on
RETURN shows no line.

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
