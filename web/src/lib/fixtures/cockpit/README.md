# The canonical cockpit fixtures (16b cut Q.3; doc 16 §5)

One scene, read by all three renderers. Doc 16 §5 asks for renderer
parity, and three renderers each pinned to a golden of its own is regression and
not parity — so there is exactly one set here, and no copies to drift. Rust
`include_str!`s these files (the way `mush-net-ssh`'s TUI reads
`../run-allowlist.json`); vitest reads them natively.

| file | what it is | built by |
|---|---|---|
| `cockpit_frame.json` | the per-pulse cockpit keyframe, feed `arena.cockpit`; since 16b cut O.2 it carries `"ecm": null` (the scene's platform carries no jamming pod) | `tacsim::match_::cockpit::status_frame` |
| `arena_map.json` | the whole arena's terrain, feed `arena.map`, pulled and never pushed; since 16b cut V.1 it carries the ground's named `spots` (`{ letter, name, cell }`, authored order, arena cells) after `legend` | `tacsim::match_::cockpit::map_frame` |
| `view_frame.json` | the 39×20-cell window `VIEW` sends, the two composed server-side, with `cell_columns: 2`, `origin_cell` and `extent` (16b cut B.4); since 16b cut V.1 its `units` open with the spot markers (`spot: true`) and its `legend` ends with `["A", "The Ford"]`; since 16b cut O.2 it carries `"ecm": null` with the rest of the status keys | `tacsim::match_::cockpit::view_window` |
| `view_grid.txt` | the twenty map rows a line client prints of it, north-up, ONE glyph per cell (39 by 20; the line client prints each cell twice); the spot `A` is the letter at row 3 from the south, column 25, over the pond | `mush_ansi::panel::grid_lines` |
| `view_brief.txt` | the sixteen lines `VIEW BRIEF` says of the scene (16b cut AG.1): the header, the ground on eight bearings from (6, 4), the four contacts as `CONTACTS` words them, the beyond-window line, the spots the legend names in the window (`A` alone: `B The Roof` is drawn under `X3` and named in no legend, so the brief does not name it either) and the off-window line; read by `tests/cockpit.rs` alone (the brief has one renderer; neither the TUI nor the web reads it) | `tacsim::match_::cockpit::brief_lines` |
| `web_view_model.txt` | every line the web's cockpit screen prints for that frame (16b cut Q.5), composed at 78 by 20 — so its legend line is `x BA  + RB  ? X3  A The Ford` | `web/src/lib/cockpit.ts`'s `cockpitViewLines` |
| `notice_lines.json` | the notices ring's wording: an array of `{ "notice": <one notice object as the wire sends it>, "line": "<the ring line>" }` (16b cut B.1) | **hand-written**; no bless variable |
| `catchup_lines.json` | the catch-up report's wording (16b cut Y.4): an array of `{ "callsign": "RA", "catchup": <the frame's catchup object as the wire sends it>, "lines": ["<one line>", …] }` | **hand-written**; no bless variable |

`notice_lines.json` is the parity fixture for the two notices rings. Every
`notice` is shaped the way `status_frame` publishes one (its `text` is the
server's sentence, which neither ring reads), and every `line` is what the ring
prints for it. vitest drives each pair through the real decoder and
`noticeLine` (`cockpit.test.ts`); 16b cut B.3 pins the TUI's ring
(`notice_words`) against the same file. It covers every kind on
`cockpit_frame.json`, a refusal with a weapon and a cycling detail and one with
a heat detail, a block with a cause and one with `cause: null`, `unidentified`
and `classified` with `was` (and a friendly `classified`, whose mark follows the
new name), a spent hit, a friendly lock, the victim's `struck`, `struck_spent`
and `section_lost` with their section, the pod's two switch notices `emcon_on`
and `emcon_off` with the pod's radius on `detail.radius_m` and an `EMCON`
refused `no_jammer` (16b cut O.2), and a kind named `constructor` (the word
tables are `Map`s, so an inherited key is not a word). Change a line here only
together with both renderers.

`catchup_lines.json` is the parity fixture for the catch-up report a reclaim's
first frame carries (16b cut Y.4; doc 07 §1.3 step 5). Every `catchup` is
shaped the way `status_frame` publishes one, and every `lines` is the block the
server says on line mode (`tacsim::match_::cockpit::catchup_lines`, one `Sys`
line tagged `arena-notice`) and both rings print. Three readers, one file:
`tacsim`'s in-source `cockpit.rs` test and `tests/cockpit.rs` (which also puts
each object on `cockpit_frame.json`'s frame and pins that `status_frame` writes
it back exactly), the TUI's `catchup_words` (`render.rs`), and vitest through
`decodeCatchup` and `catchupLines` (`cockpit.test.ts`). Its nine cases: the
full report; the empty one (`none new, none lost`, `unchanged`, `none`, an
unchanged position); a capped contact list (`and 3 more` on the `new` list,
the one that is full); capped sections and weapons (seven of each, `and 1 more`
on both lines, one section whose armour and internals both fell); a wreck (the
eighth line); `machine: null`; `cell_now: null`; `cell_then: null` with a
`cell_now` (and a section whose internals alone fell); and both contact lists
full, where the one overflow count cannot say whose it was and is its own
clause (`3 more not named`) rather than read as contacts gained. The condition words are
the arena's own (`intact`, `destroyed`, `wreck`). `cockpit_frame.json` and
`view_frame.json` carry `"catchup": null`: the scene is an ordinary pulse. They
carry `"ecm": null` too (16b cut O.2): the scene's platform carries no jamming
pod, so neither panel's scope row grows an `EMCON` suffix.
Change a line here only together with all three renderers.

The scene's platform carries the **shipped** 600 m sight (`sight_cm: 60_000`, 60
cells at 1,000 cm), so two of its four contacts read `far` (16b cut D.1): the
band word is thirds of the lesser of the sight and the fired weapon's reach, and
on every platform the game ships the sight is the lesser.

**The scene's ground names three spots** (16b cut V.1), one for each rule a
named spot is held to:

- `A` The Ford at (25, 3) — inside the window, on the pond: a letter drawn over
  water, and named in every legend;
- `B` The Roof at (14, 7) — inside the window and under contact `X3`. The spot
  goes down first and `X3`'s `?` overwrites it, so `B` **is** in
  `view_frame.json`'s `units` (placed, then covered: the grid renderer draws in
  order and the later write wins, which is the rule) and is **not** in any
  legend — the contact owns the cell, and the three legends name only what owns
  its cell;
- `C` The Woods at (50, 25) — off the 39 by 20 window and inside the woods: on
  the map, not in the window, and the spot `VIEW`'s off-window line names. On
  the web's default 128-cell view (the whole 96 by 64 ground) it is drawn and
  named: `x BA  + RB  ? X3  ? X4  A The Ford  C The Woods`.

Re-bless the first five with `TACSIM_BLESS=1`:

```
cargo test -p tacsim   --test cockpit   # the three JSON files and view_brief.txt
cargo test -p mush-ansi --test golden   # view_grid.txt
```

`web_view_model.txt` is the web's own, and is blessed from `web/`:

```
COCKPIT_BLESS=1 npx vitest run src/lib/cockpit.test.ts
```

## What a renderer is held to

`view_grid.txt` is the target. A composition of `arena_map.json` +
`cockpit_frame.json` at **exactly 39 by 20 cells** must equal it byte for byte.
That equality, and not three separate goldens, is what makes the three renderers
one renderer.

**39 by 20 cells, the line client printing each cell twice** (16b cut B.4): a
terminal cell is about twice as tall as it is wide, so `VIEW`'s window carries
`cell_columns: 2` and the line client draws its ground square at 78 printed
columns. The file stays **one glyph per cell**, because the TUI's pane and the
web's canvas draw one cell per cell; `mush-ansi/tests/golden.rs` checks that
every printed row is each glyph of the file's row twice, and blesses the file
from the printed rows halved. The footer the line client prints names **arena**
cells (`origin_cell` and `extent`), not the window's own.

The composition rules, which `view_window` follows and every other compositor
must:

- the window is 39×20 cells centred on the platform's `cell` and **clamped** to
  the arena, so a platform near a corner draws at its own cell;
- row `0` of `rows` is the **south** edge; a terminal prints them north-up;
- markers go down in order (16b cut V.1) — the map's named `spots` **first**, in
  authored order, each as its `letter`; then the pilot's own `@`; then the
  contacts in the cockpit frame's own order — and a later marker overwrites an
  earlier one on the same cell, so a contact or the `@` standing on a spot hides
  its letter;
- markers are **named** in the reverse order: the legend lists the contacts that
  own their cell first, then the spots that own theirs (`A The Ford`). A marker
  owns its cell when no later marker landed on it; the `@` is never listed. The
  contacts lead because a narrow legend is cut from the back (the TUI's
  `fit_legend`) and a crowded canvas places labels in order (the web's
  `labelPlan`), and a pilot must keep `x BA` before the name of a field.

## One thing the fixture deliberately does not carry

`view_frame.json`'s own marker (the one with `"you": true` — since 16b cut V.1
the third, after the two spot markers) has **no `name`**. A window is a pure
function of a frame and a terrain, and the name the live feed sends there is the
tank's display name, which only `view_frame` has a world to ask for. An absent
key is honest; a callsign in its place would teach every renderer written against
these files a value the server never sends. A renderer that wants to label the
own marker should use the cockpit frame's `callsign`, or read the `you` marker's
`name` where it is present.
