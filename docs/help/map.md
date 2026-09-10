# The map (`+map`, `+move`)

A **map** is a window over the atlas's ground (`help atlas`): the rooms of a
scene and the ground between them as a grid of cells, one metre each on the
street, drawn from the tiles the packs authored. It opens when a scene needs
one — a timestop opens a map over its room by itself; a scene's facilitator
opens one with `+map/open` — and everyone in those rooms stands on it, on a
free cell by the door they came through. The map is the geometry; the room is
still the audience: whoever you can hear in a room, you can hear on the map,
and the room you are in follows where you stand.

  +map                    the map you stand on: where you are, the floors,
                          who is on it — and the map itself, as text
  +map/open               open the map over your scene's room (its
                          facilitator, or staff)
  +map/close              close it, leaving everyone where they stand
  +map/floor <n>          look at floor n (0 is a building's ground floor;
                          "ground" is the lot outside)
  +move <direction> [<n>] walk n cells (one by default) north, south, east,
                          west or a diagonal (ne, nw, se, sw)
  +move <x>,<y>           walk toward a cell of the map

## Reading it

The map prints north up, one glyph per cell, with a legend: `.` paved, `,`
bare ground, `"` grass, `~` water, `_` an indoor floor, `=` road, `x` rubble,
`#` a wall, `+` a doorway, `D` a platform or dock, `@` you, `o` someone else.
A blank cell is nothing you can see: another floor, the inside of a building
you are not in, a room you may not enter. The footer names the cells shown
(`x 0..38  y 23..42`) and your own cell, which `+move <x>,<y>` walks toward.
A map wider than the screen is windowed on you; `+map` prints it again.

In the full-screen TUI (`help tui`) the unit screen's **grid** pane is the
map, and on the web the map draws itself under the scrollback; both show the
same cells the text does.

## Walking

One cell costs one, until the rules supply a table. A wall, someone standing
there, a counter, or a room that is not open to you stops the walk where it
is and says why; a doorway is the wall's own gap. Walking through a door
into a building, or out of one, changes floor for you. Stepping off the edge
of the map onto a room's ground **grows the map** to take that room in; the
map redraws and you step again. Exits still work: walking `out` of a room on
the map puts you on the far side of its door, and walking off the map's rooms
altogether leaves you where you arrive, off the map.

In a turn-based scene (`help timestop`) you walk on your turn, like `+roll`.
Anywhere else a walk is free.

## Staff

`@atlas/window` lists the open maps; `@atlas/window open [<rooms>|<x0>,<y0>,<x1>,<y1>][=<cell cm>]`
opens one by hand over rooms or an extent in metres (a map over more than
65,536 cells coarsens to the next cell size rather than refusing);
`@atlas/window close <n>` closes it. `@atlas/feature <feature>=<state>` sets a
feature's state — `intact`, `damaged`, `destroyed` — and every map over it
redraws: a destroyed wall is rubble, a destroyed door an open way. The
wreckage stays through a pack update. See `help atlas`.
