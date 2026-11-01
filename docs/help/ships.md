# Ships

The ship of the trade run is the **Dray**, a Stickney-built bulk hauler: slow,
efficient, unarmed. A Co-op pilot flies one on charter (`help coop`). `+ship`
says where yours is, what its hold carries and how your programme stands, from
anywhere.

## Aboard

A Dray is six rooms you walk like any others. Board it from the berth gallery
of the port where it is cradled:

  +ship/board   in the berth gallery: cycle the lock and go aboard (you are in
                the Airlock)
  in            the Airlock to the Central Passage
  up            the passage to the Bridge (and down again)
  crew          the passage to the Crew Compartment, galley and bunks
  hold          the passage to the Cargo Hold, and down again to Engineering
  passage       back to the passage from any of them
  airlock       the passage to the Airlock
  +ship/leave   in the Airlock, while the ship is docked: cycle out into the
                berth gallery

A prospector refit also has a vehicle bay. From the cargo hold, use
`+tv/bay in` to enter it and `+tv/bay out` to return to the hold. Rig boarding
and leaving use `+tv/<n> board` and `+tv/<n> leave` inside that bay.
Sailing and supplies count people currently inside the carrier, including
operators in its rigs. A plan read ashore includes the captain as well.

Every room aboard shows the ship's status line when you look: docked and where,
cleared for a port, or under way with the leg and the time it is due. The rooms
with instruments show their readings as well.

While you are aboard, the ship is where you are: the exits of Limbo (`o`, `s`)
and `home` do not take you off it, and nothing you do crosses the hull under
way: your own moves out of the hold refuse until she docks. You leave by the
Airlock, in port.

While aboard, `+ship` also estimates the one-way physical radio lag to an
Exchange endpoint. Queue time and the returning acknowledgement add to that
figure. Orders and acknowledgements in flight keep their original arrival time
even if the ship progresses or the world rate changes. If your passage cannot
be located, traffic waits; the instruments report that the lag is unavailable.
OOC channels remain immediate.

## The hold

`+ship/hold` shows the hold good by good, in tonnes and cubic metres. The cargo
hold has **two limits**, a mass rating in tonnes and a packed volume in cubic
metres, and a load can meet either one first: dense goods weigh it out, bulky
goods cube it out. The reaction-mass tank and the stores locker are apart from
the cargo hold, each with its own limit; reaction mass goes only in the tank and
provisions only in the locker, and no cargo goes in either. When you buy, the
port routes each good to its place and refuses a quantity that will not fit,
before anything is charged.

## Planning and leaving

  +ship/plan [<port>]     a dry run of the passage: the hold's room by mass and
                          by volume, the payload, the burn of reaction mass,
                          the stores in crew-days and the passage time. With no
                          port it plans every port this berth clears for. It
                          writes nothing.
  +ship/undock <port>     from aboard, docked: file the passage and take her
                          out now
  +ship/lift <port>       the same as +ship/undock: the word for lifting off a
                          pad (or out of a berth)

The burn is reaction mass. A Dray flips and burns: it accelerates to the
midpoint, turns, and decelerates, so a leg takes twice the square root of its
distance over the ship's acceleration, and the burn is what the rocket equation
charges for the speed it reaches — a farther port costs more time **and** more
reaction mass, a heavier hull accelerates less, and the sky moves, so the same
port is cheap one month and dear another. Every tonne aboard — cargo, the tank
itself, the stores — counts; the stores are crew-days for the souls aboard.
`+ship/plan` reads it all: plan it, do not assume it.

  +ship/plan <port> throttle=<permille>   fly slower and cheaper: 500 is half
                                          thrust, 41 % longer, 29 % less burn
                                          (250 to 1000)
  +ship/plan <port> policy=cheapest       keep the listed route that burns least
                                          rather than the fastest
  +ship/plan <port> via=<place>           a route that calls at that place
                                          (Kirkwood Depot, for one)

`+ship/undock` takes the same three. `+ship/plan` ends with a window line when the
sky moves: the best departure in the next thirty IC days against today's.

When the plan does not fit at the throttle you asked even with the tank and the
locker filled to their limits, `+ship/plan` prints one more line after the
stores, beginning Throttle:. It names the highest throttle that fits this load
with both full, and that plan's burn, crew-days and passage — or it says that no
throttle from 250 fits. A plan short only because you have not bunkered yet gets
no such line: top the tank and the locker up. The line is advice and changes
nothing: it appears only in `+ship/plan`, and `+ship/undock` and a sailing
plan's departure refuse a tank short of the burn or a locker short of the stores
in the same words as ever. Pass the throttle yourself (`throttle=` on either).
Titan's lanes fit the tank only throttled (`help trade`).

  +chart                  the sky now from the port you stand at: every body's
                          distance, longitude and latitude, the light lag, and
                          each port's passage now (this load, empty, and the
                          direct chord beside the listed route)
  +chart <port>           the window: that passage planned on every IC day of
                          the next year — the fastest, the cheapest, the worst
  +chart/body <body>      one body and the elements it moves on
  +chart/ground [<body>]  the Registry's survey of a body: its sites by letter,
                          each with the survey's words, its tenure, the passage
                          now and your own best reading (`help mining`)
  +chart/ground map [<body>] [<family>]
                          the survey as a map: the body flat, 56 by 28, one
                          family's wash, the port and the sites by letter
  +chart/map              the sky as a flat map: the Sun, every orbit, the bodies
                          by letter, the listed routes dotted, you at @ — what the
                          TUI's chart screen (Alt+c) and the web's chart tab draw

`+ship/undock` files a clearance: the port checks the plan, the tank and the
locker, and on its next tick the burn is committed and the ship launches. If the
plan cannot go — the tank short of the burn, the locker short of the stores, the
hold over its limits, goods you bought not yet aboard — it says why and nothing
is burned. A clearance that cannot launch is held, you are told, and staff are
told; staff can stand it down, and nothing burned is returned. To leave at a
later hour, or while you are off watch, file a sailing plan (`help orders`).

Docking is automatic: the ship arrives at the far port's berth gallery when it
is due, and every room aboard hears it. Under way, `+travel/status` aboard says
where the ship has got to.

## Settling at a site

A mining site is a Registry pad on open ground: five a short hop from the
Freeport on Ceres, and Rheasilvia Commons a passage away on Vesta (`help
travel`). A ship bound for one does not dock; it settles onto the pad, and
`+ship` and every room aboard read "settled on the pad". A hull that cannot lift
1.2 times its weight on a body cannot land there, so no Dray lands on Luna, Mars
or Titan, and a hop is never free: lifting off a pad burns at least a tonne.

Nothing sells reaction mass or provisions on a pad, so a departure for a site
holds back the leg home: `+ship/undock` (and a sailing plan's departure) refuses
unless the tank and the locker cover the passage out and the worst passage home
in the next thirty IC days, and `+ship/plan` prints that leg home with the
throttle it is held at — the least that lifts her. For a hull whose hold takes
ore it prints a second leg home, with the hold full of ore: the departure does
not hold that one back, but it is what to bunker for if you mean to come home
full (from Rheasilvia a full Dray-P burns twelve tonnes more on the way home).
For a site the Throttle: line
counts that leg home too: it names the highest throttle from the site's landing
floor at which the passage out and the leg home together fit the tank, or, when
none does today, the first day in the next IC year (the 365 days `+chart
<port>` scans) on which the floor does (Rheasilvia fits only throttled, and
only in its season); a refused
`+ship/undock` for a site ends with the same line, and the sailing plan
`+ship/plan` reads from the queue names a closed or short site's refusal as the
departure would. On a pad the Throttle: line
reads the tank and the locker as they stand, since nothing can be bought there:
when the leg home overruns the tank at the throttle you asked, it names the
highest that fits. On the pad the stores line counts down: one
crew-day a soul for each IC day, charged when she lifts, and never below the
stores for the leg home; once it reaches them the crew are on short rations.
Nobody steps out onto a pad: the lock stays sealed until she docks, and
`+ship/lift <port>` takes her home.

## The Dray-P and Occator's rigs

`+coop/refit prospector` turns a chartered or owned Dray into a Dray-P
(`help coop`): a two-slot vehicle bay aft of the hold, a stern ramp, and a hold
of 86 t and 103.2 m³ that takes bulk and ore. Her rail, tank, locker and drive
are a Dray's, and a Dray-P with two empty rigs plans exactly as a Dray carrying
the same weight.

Occator Water & Propellant leases terrain vehicles into the bay. At Occator's
counter in the Exchange Hall at Ceres Freeport, with your Dray-P docked,
`+tv/lease` shows the terms and signs nothing; `+tv/lease go` signs them:
1400.00cr for a term of 168 IC days, paid now, and a 5000.00cr deposit held on
your account while the rig is aboard. The rig rolls up the ramp into the lowest free
slot; `+tv/2/lease go` names slot 2. Two leases are two rigs. A lease needs a
prospecting licence (`+licence/buy`; `help coop`), in force or in its grace.
After buyout, the same terms apply to the current title holder.

`+tv` reads each slot anywhere: the rig, how long its lease has left and what
its hopper holds; `+tv/1` reads rig 1 alone. A term that runs out is re-debited
at your next `+tv/lease` act, even one your lapsed licence refuses; if your
account cannot pay it, Occator keeps the deposit and the rig must go back.

`+tv/1 lease return go` at the same counter hands rig 1 back: the rig must be in
the bay and its hopper empty, and your deposit is released. Without go it says
what would happen. Ore leaves a hopper only by the rig's own unload into the hold
or a dump onto the spoil; a hopper never sells.

With your Dray-P settled on a site's pad and you aboard, you work the rigs from
the bridge. `+tv/1 launch` says what sending rig 1 down the stern ramp would do
and commits nothing; `+tv/1 launch go` sends it: 300 IC s later it stands on the
ground at the foot of the ramp. `+tv/1 recover` brings it back up when it stands
beside the ramp, on the ramp's own cell or the next (300 IC s); a rig further off
drives to the ramp first, in the same act. A rig that comes home with ore in its
hopper then unloads it into the hold on its own (600 IC s); `+tv/1 unload go` does
the same by hand, for a rig in the bay. `+tv/1 dump go` empties its hopper onto the
site's spoil heap, for good. Without a number, `+tv/launch` addresses the rig you
named last, or your only rig. `+tv` shows where each rig is, its charge and any
act it is in the middle of. The Dray-P lifts only with every rig in her bay,
none working a fill plan, and every hopper empty: `+ship/lift`, `+ship/plan` and
a queued depart all say which rig holds her (`+tv/1 plan clear` ends a plan), and
on a site's pad name the unload and the dump; at a port, where neither works, only
staff can empty a hopper. Scanning, sounding and the rest of the mining loop are in
`help mining`.

## The box

Every hull keeps a black box: one line when it leaves a hangar for a trip and one
when it comes in, with the mind sealed for the trip, its class, whether it ran
under the governor, the state of its licence and the first eight characters of
its seal. `+hull/box #<hull>` reads the last eight lines, oldest first;
`+hull/box #<hull> 20` reads the last twenty (thirty-two at most). When older
lines are sealed away, the last line says how many. You can read the box of a
hull whose title you hold, or the lines since your lease began on a hull you
lease; staff read any. A hull you cannot read answers that it has no box you can
read.

## Outer space, local bubbles and bow docking

Beyond the inner ports, the ephemeris tracks the outer worlds and their moons —
Saturn, Uranus, Neptune, Triton, and the Galilean satellites Io, Europa,
Ganymede and Callisto — alongside four orbital asteroid clusters: the Ceres
Halo, the 3:1 Kirkwood Gap, the Pallas Inclined Belt, and the outer edge of
Saturn's B-Ring. Long-haul hulls fitted with external drop tanks (`+helm/droptanks`)
can decouple from a filed transit leg (`+helm/decouple`) into continuous
three-dimensional free flight, execute directed delta-v burns (`+helm/burn`,
`+helm/expedition`), or re-solve a two-burn flip-and-burn passage back onto the
port network (`+helm/autopilot`) from open space.

When an interdiction, a waypoint picket, or a contested asteroid cluster pulls a
hull into a local space bubble (`+helm/enter`), simulation runs on 1:1
wall-clock time (`+tac/step`). Ships manoeuvre with thrust vectors (`+helm/burn`),
attitude trim (`+helm/roll`), reaction-mass budgets, emissions posture (`+tac/emcon`),
light-lagged thermal/radar sensor scans (`+tac/scan`), active ECM decoy canisters
(`+tac/decoy`), unguided railgun slugs (`+tac/rail`), high-G guided torpedoes (`+tac/torp`),
point-defense bursts (`+tac/pdc`), and distance-attenuated lasers (`+tac/laser`,
`+tac/beam`); multicrew bridges bind individual station seats (`+helm/station`),
flush open-cycle radiator coolant (`+helm/coolant`), repair subsystem critical hits
(`+helm/repair`), and cut main thrust to ballistic drift while keeping point-defense
armed if all crew go linkdead (`+helm/linkdead`). A Dray-P matched within 5 m/s of
an orbital rock (`+helm/match`) can ping deposits (`+mine/ping`), stake or contest
Freeport claims (`+mine/licence`, `+mine/stake`, `+mine/contest`), launch up to four
sealed-mind collector drones (`+mine/drones`), or cut ore directly into its ore bay
(`+mine/laser`) while watching its infrared signature across the cluster.

On the bridge, the picture pane (`Alt+v` in the terminal client, or the web view
tab) switches automatically between the three-dimensional holo-tank plot, the
first-person bridge canopy, and the 6-DOF bow docking camera (`+helm/dock`). Port
approaches offer an optional manual docking run that awards a clean-trim bonus
inside ±0.25 m lateral, ±1.5 deg attitude, and under 0.80 m/s closing speed;
clamping onto an uncooperative derelict hull within 500 m requires manual
alignment before a salvage tow line (`+helm/tow`) can be rigged.

See also: `help orders`, `help ports`, `help trade`, `help coop`, `help mining`.
