# The canonical cockpit fixtures (16b cut Q.3; doc 16 §5)

One scene, read by all three renderers. Doc 16 §5 asks for renderer
parity, and three renderers each pinned to a golden of its own is regression and
not parity — so there is exactly one set here, and no copies to drift. Rust
`include_str!`s these files (the way `mush-net-ssh`'s TUI reads
`../run-allowlist.json`); vitest reads them natively.

| file | what it is | built by |
|---|---|---|
| `cockpit_frame.json` | the per-pulse cockpit keyframe, feed `arena.cockpit` | `tacsim::match_::cockpit::status_frame` |
| `arena_map.json` | the whole arena's terrain, feed `arena.map`, pulled and never pushed | `tacsim::match_::cockpit::map_frame` |
| `view_frame.json` | the 78×20 window `VIEW` sends, the two composed server-side | `tacsim::match_::cockpit::view_window` |
| `view_grid.txt` | the twenty map rows a line client prints of it, north-up | `mush_ansi::panel::grid_lines` |
| `web_view_model.txt` | every line the web's cockpit screen prints for that frame (16b cut Q.5) | `web/src/lib/cockpit.ts`'s `cockpitViewLines` |

The scene's platform carries the **shipped** 600 m sight (`sight_cm: 60_000`, 60
cells at 1,000 cm), so two of its four contacts read `far` (16b cut D.1): the
band word is thirds of the lesser of the sight and the fired weapon's reach, and
on every platform the game ships the sight is the lesser.

Re-bless the first four with `TACSIM_BLESS=1`:

```
cargo test -p tacsim   --test cockpit   # the three JSON files
cargo test -p mush-ansi --test golden   # view_grid.txt
```

`web_view_model.txt` is the web's own, and is blessed from `web/`:

```
COCKPIT_BLESS=1 npx vitest run src/lib/cockpit.test.ts
```

## What a renderer is held to

`view_grid.txt` is the target. A composition of `arena_map.json` +
`cockpit_frame.json` at **exactly 78 by 20** must equal it byte for byte. That
equality, and not three separate goldens, is what makes the three renderers one
renderer.

The composition rules, which `view_window` follows and every other compositor
must:

- the window is 78×20 cells centred on the platform's `cell` and **clamped** to
  the arena, so a platform near a corner draws at its own cell;
- row `0` of `rows` is the **south** edge; a terminal prints them north-up;
- markers go down in order — the pilot's own `@` **first**, then the contacts in
  the cockpit frame's own order — and a later marker overwrites an earlier one on
  the same cell.

## One thing the fixture deliberately does not carry

`view_frame.json`'s own marker (`units[0]`) has **no `name`**. A window is a pure
function of a frame and a terrain, and the name the live feed sends there is the
tank's display name, which only `view_frame` has a world to ask for. An absent
key is honest; a callsign in its place would teach every renderer written against
these files a value the server never sends. A renderer that wants to label the
own marker should use the cockpit frame's `callsign`, or read `units[0].name`
where it is present.
