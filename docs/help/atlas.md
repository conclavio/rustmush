# The atlas

The atlas is the game's map: every place has geometry — a region in metres, the
neighborhoods on it, the sites and buildings in them, and the footprint of every
room — and the map is drawn from that geometry, filtered for you. It is a map,
not a hint: a room you may not enter is not on your map unless you are standing
in it or you are staff.

  +atlas             the neighborhood you are in: a text map, north up, five to
                     ten metres a character, then its rooms and who is in them
  +atlas <place>     a neighborhood by name
  +atlas/region      the region: neighborhoods, corridors and checkpoints

Map glyphs: `#` wall, `+` a door or a way through, `@` you, a letter for each
room (listed below the map), `=` a corridor, `o` a checkpoint on it. A building
is drawn floor by floor; the ground shows its outline.

Limbo is on no map. The OOC Area `<O>` and the Staff lot `<S>` are.

Roads between neighborhoods take time to travel: see `help travel`.

## What keys on the atlas

The worldbuilding theme (`help worldgen`) hangs off places: a region, locality,
neighborhood or site may carry one, `world` is the default, and a room resolves
up through its site, neighborhood, locality and region. Weather cells key on a
neighborhood or a region (`help weather`). A faction control region may stand on
a place (`@region/atlas`), and a residence may be linked to the site it stands on
(`@property/site`, refused if any of its rooms lies outside the site). Zones keep
locks and zone exits only.

  @atlas/theme <place>=<field>:<value>   set a theme field (staff)
  @atlas/theme/show [<place>]            the effective theme there, or here
  @atlas/theme/exemplar <place>=<room>   copy a room's desc in as an exemplar

## Player-built sites

A neighborhood whose policy allows it takes sites players ask for: `+build/place`
reserves the ground, the building agent plans the site, and `+build/go` makes it
stand, pending review (`help build`). Staff review through the `BUILD` job queue
(`+job/approve` keeps it, `+job/deny` asks for changes) and:

  @atlas/cap <character>=<n>       set a character's cap on outstanding
                                   requests (and clear their strikes)
  @atlas/strike <site>=<reason>    retire a player-built site and strike its
                                   builder; three strikes zero the cap
  @atlas/standing [<character>]    a character's standing, or every request
  @atlas/propose <prose>           have the agent plan a pack for the
                                   neighborhood you stand in (or
                                   `<place>=<prose>`), to apply live with
                                   +build/go
  @atlas/propose --from <pack>     reconceptualize a pack on the shelf: the
                                   agent keeps its ways in and revises the rest
                                   as the next version

## Content packs (staff)

Places arrive as content packs: one JSON document — a manifest, entries (a
neighborhood, its sites, structures, rooms, exits, features, items, NPCs, hooks
and secrets) and a brief — with a digest over its bytes. A pack applies once; an
unchanged pack applied again does nothing; a changed one updates in place and
never touches live state (who is in a room, what was said there).

  @atlas/pack list                 every pack on the shelf and every latch
  @atlas/pack show <id>            manifest, requires, entries, brief, secrets
  @atlas/pack plan <id>            what applying would create, update or
                                   retire, the validator's findings, and a
                                   fingerprint of that plan
  @atlas/pack apply <fingerprint>  apply the plan you just read
  @atlas/pack retire <id> [why]    retire a whole pack

`plan` runs the validator over the pack and the world as it stands — schema and
references, geometry (footprints inside the neighborhood, sites and rooms not
overlapping, exits between rooms that touch or say how far), topology (every
public room reached through unlocked exits; a way into a restricted room locked),
budgets, the neighborhood's policy and the theme's banned words, secrets kept
out of every prompt field, and what a new version would retire from under
someone. Each finding is typed (`[error] room.yard: … (geometry.outside) — how
to fix it`); an error withholds the fingerprint and `apply` refuses, a warning
prints through, an info is a note. The building agent's `pack.validate` runs the
same rules. `apply` demands the fingerprint `plan` printed: a pack or a world
that changed since gives a new one and refuses. `retire` is refused, naming the reference,
while any room of the pack is occupied, holds something the pack did not put
there, or is the destination of an exit from outside the pack. A retired pack
stays retired across restarts; `plan` it again to bring it back.

The shelf holds the shipped starter packs — `region.limbo`,
`neighborhood.ooc_area`, `neighborhood.staff_lobby` — and whatever
`[atlas].pack_paths` names. The starter area applies itself at a world's first
boot when `[game].starter_area` is on; it is static content and makes no
model call. Staff only: `SIM_ADMIN` or Royalty.

## Maps

A tactical window over the atlas — a scene's rooms as a grid of cells, walked
with `+move` — is `help map`. Staff open one by hand with `@atlas/window`,
and set a feature's state with `@atlas/feature`.
