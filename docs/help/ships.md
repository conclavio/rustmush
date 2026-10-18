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
and `home` do not take you off it, and nothing crosses the hull under way. You
leave by the Airlock, in port.

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

The burn is reaction mass, and every tonne aboard — cargo, the tank itself, the
stores — adds to it; the stores are crew-days for the souls aboard. Passage time
depends on the load and, later, the season: plan it, do not assume it.

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
