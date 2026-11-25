# Spatial placement (staff)

The simulation module (`sim`) gives ordinary objects a **spatial placement**: a
durable, single-parent graph over passive **domains**, **interior spaces**, and
**mounts**. An object's room (`Obj.location`) becomes a maintained *projection*
of its placement — the placement graph is the truth, the room is where the
rest of the MUSH sees it.

Placement is a foundation for the tactical layer (square windows, route dispatch,
domain ticking): those arrive in later milestones. This cut ships the records,
the invariants, the staff surface, boot recovery, and the movement gates.

## Behaviors

- `sim.domain` (Wizard, refuses `@destroy`) — the host of a simulation domain.
  Attach it, then `@domain/create`.
- `sim.interior` (Wizard, refuses `@destroy`, rooms only) — an interior space
  (a hangar, a dropship bay) that can hold placed units. Attach it, then
  `@spatial/space`.
- `sim.participant` (Builder) — anything that may be placed: a mech, a
  vehicle, a character.

A `sim.*` binding cannot be removed while records depend on it (retire the
domain, `@spatial/unspace`, or `@spatial/unplace` first).

## Placement kinds

| Parent | Meaning | Projects into |
|---|---|---|
| `Direct` in a domain | on the map at a driver-owned pose | the domain's presentation room |
| `Interior` in a space | inside a hangar/bay, optionally in a named compartment | the space's room |
| `Attached` on a host mount | mounted on a carrier | the host's location if the mount is *exposed*, else inside the host |
| `Unplaced` | a spatial entity currently nowhere | the in-transit room while its journey is under way; the recovery room when quarantined; otherwise left where it is |

The graph is acyclic and at most 8 deep. An interior space anchored to a
moving host must be placed under that host; one fixed in a domain must be
placed in that domain (or not placed).

## Movement gates

A placed object — directly on a domain, inside an interior space, or attached to
a mount — and any quarantined object cannot walk through exits or be picked up /
dropped / given: it moves only through the simulation or staff recovery
(`@spatial/unplace`, `@spatial/recover`). Ordinary visitors to an interior room,
who carry no placement, come and go freely.

A multi-room interior — a ship's six rooms — lists its member rooms, and
someone placed inside it walks freely between them by their exits (no placement
write per step), but no exit takes them to a room outside it: the boundary
holds, and the global exits and `home` refuse too. On a Co-op Dray the way in
and out is the player's own: `+ship/board` in the berth gallery places you in the
Airlock, and `+ship/leave` in the Airlock, docked, takes you back to the gallery
(see "help ships").

## The in-transit room and the recovery room

Two rooms belong to the simulation itself:

- **In Transit** holds every carrier that is under way on a journey, and
  whatever rides on its exposed mounts, from the moment it sets out until it
  arrives.
- **Recovery** holds what is quarantined: a carrier whose journey stopped,
  whatever `@spatial/unplace` set aside without `/keep`, and anything a boot
  could not validate.

The server makes each room the first time it is needed: In Transit when the
first carrier sets out, Recovery when the first object is quarantined or set
aside. It remembers each by object number in its own record. So a wiped world
has them again with no staff step, renaming one loses nothing, and a world in
which nothing has sailed yet has neither. Neither room stands in another room
and no exit leads to either, so a player cannot walk or teleport in, and a ship
in passage is never in a start room or in a newcomer's `look`. Staff can
`@teleport` to either room. `@destroy` refuses both.

To find what is in them without going there: `@journey` lists the carriers
under way and where each is bound, `@spatial/quarantine` lists the quarantined
objects, and `@spatial <obj>` shows one object's placement.

A world first booted by an earlier server kept these objects in Limbo. Its next
boot makes the room each needs and moves them in.

## Domains

```
@domain                                  list
@domain/create <host>=<driver>[,<presentation room>]
    drivers: sim.passive (the simulation's own; opens nothing) or tactics.ground
    (the TacSim ground driver: the domain IS a ten-meter window over the host
    room's tiles — [tacsim].ground_cell_cm — with the room's occupants placed;
    @domain/retire closes it)
@domain/status <id|host>
@domain/pause <id|host>    @domain/resume <id|host>
@domain/present <id|host>=<room>          re-point the presentation room
@domain/retire <id|host>                  must be unreferenced
```

A room host presents into itself by default. Retired domains keep their record
(ids are never reused) and accept nothing.

## Placement

```
@spatial <obj>                            show placement / hosting / dependents
@spatial/tree <obj>                       the subtree under an object
@spatial/place/domain <obj>=<domain>[,{json pose}]
@spatial/place/interior <obj>=<space>[,<slot>]
@spatial/place/attached <obj>=<host>,<mount>[,{json}]
@spatial/unplace[/keep] <obj>[=<reason>]  back to a plain object (moved to the recovery room unless /keep)
```

## Spaces and mounts

```
@spatial/space <room>=fixed:<domain>|moving:<host>|unanchored
@spatial/unspace <room>
@spatial/slot <space>=<slot>[,<max>]      a compartment with capacity
@spatial/mount <host>=<mount>[,<max>][,exposed]
@spatial/unmount <host>=<mount>
```

`@spatial/space` submits the change and confirms it after commit with
`Spatial update for #<room> committed.` Wait for that confirmation before
issuing dependent commands. An affected control station, active focus, custody
or journey can refuse the change until its owner can participate in the update.

Mounts default to capacity 1 and not exposed. An exposed mount keeps the
attached unit visible in the host's room (a mech on a carrier's deck); a
suppressed one puts it inside the host (a cargo bay).

## Recovery

Every structural change is validated in full before anything moves; a refused
change mutates nothing. At boot, every placement is re-validated against the
world: anything dangling (its domain, space, host, or an ancestor gone) is
**quarantined** — moved to the recovery room with a durable reason — never
silently dropped. Destroying a host through code (not `@destroy`, which the
structural gate refuses) quarantines what depended on it.

```
@spatial/quarantine                       list quarantined objects
@spatial/recover <obj>                    lift the quarantine (it stays unplaced)
@spatial/validate                         check indexes and projections
@spatial/log                              recent staff operations and recoveries
```

Staff need the `SIM_ADMIN` power or Royalty.
