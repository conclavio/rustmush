# Routes and interception (mobility)

A **route** sends a plain object along an ordered path of rooms over real time.
It **never teleports**: it moves one room at a time on a background sweep, so
anyone standing on its path sees it pass — and a third party can **intercept**
it. A WoD black car, a courier, a smuggling run: calling for one creates a
journey, it does not summon or teleport anything.

Travel is measured in **seconds**. A leg's time is both how long the object
takes to reach that room and how long it lingers there — the window in which a
co-present watcher can waylay it.

## For everyone

```
+intercept <traveler>        stop a traveler passing through your room
waylay <traveler>            (alias)
```

You can only intercept what is **here** — you must be in the same room and able
to see it. Interception stops the journey where it is; staff decide whether and
when it goes on. Just after a stopped journey resumes there is a brief grace
period in which it can't be re-waylaid, so it can actually get moving.

You are never told a traveler's destination or route by intercepting it. All you
learn is that it is here, now. Following it, watching a departure, or covering a
crossing is how you learn where it goes.

## For staff (`SIM_ADMIN` or Royalty)

```
@route <traveler>=<room>[:secs] <room>[:secs] ...   plot a route (default 30s/leg)
@route/show <traveler>                              inspect a journey
@route/pause <traveler>    @route/resume <traveler>
@route/abort <traveler>                             cancel it
@route                                              list active routes
```

- `<traveler>` must be a plain object — not something placed in the simulation
  (a mech in a domain, an interior space, a mount/station host). An object is
  *either* on a route *or* placed in the simulation, never both.
- Each `<room>` must be a live room the traveler could normally enter; the check
  runs when you plot the route.
- `@route/resume` continues a paused journey from where it stopped, shifting the
  rest of its schedule forward by the time it was held.

A traveler on a route can't be walked out, `@teleport`ed, or picked up — it
moves only along its route (or `@route/abort`). Destroying the traveler cancels
its journey; a route restored from a reboot continues from where it was.

## Not yet

Player-owned routing (routing a vehicle you own), contested/rolled interception,
movement modes and speeds (the `mush-tactics` `MobilityProfile`), NPC dispatch
services (calling a car), and route knowledge as tracked, stale-able intel (the
epistemic layer) are all later cuts. This is the interceptable-passage substrate
they build on.
