# Travel

Places are joined by roads, and a road takes time. `+travel` walks you along one
on the world clock: you set off from the room the road leaves from, you are on
the road for as long as it takes, and you arrive when it is time. Nobody but you
and staff is told where you are going.

  +travel                     where you can go from here, by what, how long it
                              takes, and the room each road leaves from
  +travel <place>[/<mode>]    set out (foot, car or transit need no vehicle)
  +travel/status              where you have got to
  +travel/cancel              turn back if you have not passed a checkpoint,
                              or stop where you are

Time on the road is game time: a road's legs are IC seconds, and real time is
IC over the campaign rate, so a campaign that runs fast arrives sooner. Two
guards hold whatever the rate: you spend at least `dwell_floor_s` real seconds
at every checkpoint (thirty by default), so someone waiting there can act, and
a whole passage never takes longer than `real_cap_s` for its mode (three real
minutes by default for foot, car, transit and the bus).

A **timed way** — a farm road, a long stair, a ferry — works the same: go through
it and you set off; a short door stays instant.

If you are stopped on a road (you cancelled past a checkpoint, or you dropped
your connection), `+travel` names the road's two ends and you set out again from
where you stopped.

## Checkpoints, stake-outs and interception

Every road has checkpoints. `+stakeout <checkpoint>` takes you to one — a patch
of ground beside the road, made when you arrive and gone when the last watcher
leaves — and anything that comes along the road is stopped in front of you: you
are both told, and you can talk. `+stakeout/end` gives it up and brings you back;
whatever you were holding goes on its way, and for a few seconds nobody can stop
it again.

  +stakeout                    the checkpoints you could watch from here
  +stakeout <checkpoint>       go and watch it
  +stakeout/end                give the watch up
  +intercept <traveler>        stop a traveler when it reaches you, or hold a
                               vehicle standing at your stop
  +intercept/release <traveler>   let it go

`+intercept` tells you nothing about where a traveler is going or which way it
is coming: you learn only that it is here, when it is.

## Vehicles

A vehicle such as the studio tour bus is a room that travels. Board it at a stop
and you ride: everyone aboard keeps their one place — the bus — for the whole
passage, the driver announces the stops over the PA, and the doors open only
where the bus is standing. `look` aboard says where it has got to. A stake-out
stops a bus like anything else, and its doors open onto the patch.

## For staff (`SIM_ADMIN` or Royalty)

  @atlas/journey list                every journey under way, and each service
  @atlas/journey show <traveler>     one journey: nodes, legs and stamps
  @atlas/journey send <object>=<place>[/<mode>]   put a plain object on the road
  @atlas/journey hold <traveler>     stop it where it is
  @atlas/journey release <traveler>  let it go on
  @atlas/journey abort <traveler> [why]   end it where it is

Roads are content: a region pack's `connection` entries name the neighborhoods
they join, their checkpoints as legs (the last leg names the connection itself,
meaning its far end), the modes they carry, and — for a vehicle that runs on its
own clock — a `service` naming the vehicle room, its dwell at each stop, and its
driver. See `help atlas` for packs.

Hulls under way on the placement graph — a TacSim carrier journey along a
corridor — are listed by `@atlas/journey list` beside the atlas's own
journeys, and an interface NPC's `travel.status` answers for a crew aboard
one ("You are aboard the …"). A ship is not flown with `+travel`: a Dray
leaves port with `+ship/undock` or its sailing plan (see "help ships" and
"help orders"), and `+travel/status` aboard says where it has got to. The
lane's passage time is computed by the planner when the ship departs — from
the load aboard and the sky at that moment — and frozen on its ticket for the
whole passage, at the throttle the departure names. When the passage will not
fit the tank or the locker at that throttle even with both full, `+ship/plan`
adds a Throttle: line naming the highest throttle that would (see "help
ships"); the departure itself refuses a short tank or locker as it always has.
A corridor the atlas draws between ports is a picture of the bodies at the
campaign epoch, not the passage's length.

## The mining sites

Six of the lane's destinations are mining sites rather than ports: Registry pads
on open ground with nothing built but the pad. Five are a short hop from Ceres
Freeport on Ceres itself — Freeport Commons, Kerwan Floor, Ahuna Flank, Juling
Shadow and Occator Rim, an hour empty and about an hour and a half out with a
full tank, two hours with a full hold — and Rheasilvia Commons is a passage of
days away on Vesta. `+ship/plan <site>` and `+ship/undock <site>`
fly to them as to any port, a ship settles on the pad instead of docking, and
`+ship/lift ceres` brings her home ("help ships"). `+chart` lists each site with
its Registry survey and its tenure. Mining the ground arrives in later cuts; for
now a site is somewhere to land and leave.
