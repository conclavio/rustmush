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

Every room aboard shows the ship's status line when you look: docked and where,
cleared for a port, or under way with the leg and the time it is due. The rooms
with instruments show their readings as well.

While you are aboard, the ship is where you are: the exits of Limbo (`o`, `s`)
and `home` do not take you off it, and nothing you do crosses the hull under
way: your own moves out of the hold refuse until she docks. You leave by the
Airlock, in port.

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

  +chart                  the sky now from the port you stand at: every body's
                          distance, longitude and latitude, the light lag, and
                          each port's passage now (this load, empty, and the
                          direct chord beside the listed route)
  +chart <port>           the window: that passage planned on every IC day of
                          the next year — the fastest, the cheapest, the worst
  +chart/body <body>      one body and the elements it moves on
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

See also: `help orders`, `help ports`, `help trade`, `help coop`.
